//! The MT5 connection as the app runs it: the accounts the trader signed in to, each kept connected
//! by a [`Link`] on a thread of its own, and what the screens show of them.
//!
//! A link's thread writes each poll's deals into the database itself and bumps the revision when
//! something changed ([`store::data_changed`]), so the pages read again; what the screens show of
//! the connection (its state, the open positions) comes back to the UI thread through a channel
//! that a short timer empties. A position that opens while connected puts the plan prompt on screen.
//!
//! The accounts are kept in the `mt5.accounts` setting, their passwords in the operating system's
//! credential store (`Env::save_secret`, in memory for tests and the demo).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use tradelens_core::{AccountInfo, Db, DbError, DealBatch, TradeKey, journal::EntryPlan};
use tradelens_mt5::{
    AccountRef, Event, Levels, Link, LinkSettings, LinkState, Login, Mt5Error, Opener, Password, Position,
    install::{self, Install},
    installer, memory,
    python::{self, Python},
    runtime,
};

use crate::{
    cloud::Env,
    pages::entry_plan::{self, PlanRequest},
    store::{self, SharedDb, Store, Writer, store, writer},
    ui::controls::root_owner,
};

/// The accounts signed in to, as JSON (`[{login, server, terminal}]`).
pub(crate) const ACCOUNTS_SETTING: &str = "mt5.accounts";
/// `memory`: terminals are the broker in memory (`tradelens_mt5::memory`), for tests and the demo.
pub(crate) const MODE_SETTING: &str = "mt5.mode";
/// The Python to run the worker with, when the one found is not the one wanted.
pub(crate) const PYTHON_SETTING: &str = "mt5.python";
/// `0`: a position that opens does not put the plan prompt on screen.
pub(crate) const PROMPT_SETTING: &str = "mt5.plan_prompt";

/// The connection's settings that belong to this machine: a restore from another one keeps them.
pub(crate) const LOCAL_SETTINGS: [&str; 4] = [ACCOUNTS_SETTING, MODE_SETTING, PYTHON_SETTING, PROMPT_SETTING];

/// What the deals a link writes are stored as (`deals.source`): an MT5 feed, so they are not editable.
const SOURCE: &str = "mt5";

/// How often the timer takes what the links' threads said.
const DRAIN_EVERY: Duration = Duration::from_millis(250);

/// How often a connected account's equity is written to the database. It moves with every tick
/// while a position is open, and each write is a change the cloud sync uploads; the card shows the
/// live figure anyway.
const EQUITY_EVERY: Duration = Duration::from_mins(5);

/// The name of an account's password in the credential store.
fn secret_name(account: &AccountRef) -> String {
    format!("mt5:{account}")
}

/// An account signed in to, as the setting keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    login: String,
    server: String,
    /// The installed `terminal64.exe` the account's portable copy was made from.
    #[serde(default)]
    terminal: PathBuf,
}

impl Saved {
    fn account(&self) -> AccountRef {
        AccountRef::new(&self.login, &self.server)
    }
}

/// What the machine has for the connection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Setup {
    /// The machine has been looked at (Python, terminals).
    pub(crate) checked: bool,
    /// Terminals are the broker in memory: nothing else is needed.
    pub(crate) demo: bool,
    pub(crate) python: Option<Python>,
    pub(crate) installs: Vec<Install>,
}

impl Setup {
    /// Whether only Python (or its `MetaTrader5` package) is missing, which the app installs itself.
    pub(crate) fn needs_python(&self) -> bool {
        !self.demo && cfg!(windows) && self.checked && self.python.as_ref().is_none_or(|python| !python.has_package)
    }

    /// Whether no MetaTrader 5 terminal was found, which the app installs with MetaQuotes'
    /// installer ([`installer`]).
    pub(crate) fn needs_terminal(&self) -> bool {
        !self.demo && cfg!(windows) && self.checked && self.installs.is_empty()
    }

    /// Why an account cannot be connected yet, in Vietnamese; `None` when it can.
    pub(crate) fn missing(&self) -> Option<String> {
        if self.demo {
            return None;
        }
        if !cfg!(windows) {
            return Some("Kết nối MT5 chỉ chạy trên Windows (gói Python MetaTrader5 chỉ có cho Windows).".to_owned());
        }
        if !self.checked {
            return Some("Đang kiểm tra Python và MetaTrader 5 trên máy…".to_owned());
        }
        match &self.python {
            None => Some(format!(
                "Máy chưa có Python: bấm \"Cài Python tự động\" để TradeLens cài một bản Python {} riêng cho app (khoảng 40 MB, không ảnh hưởng phần còn lại của máy), rồi đăng nhập.",
                runtime::VERSION
            )),
            Some(python) if !python.has_package => Some(format!(
                "Python {} chưa có gói MetaTrader5: bấm \"Cài gói MetaTrader5\" rồi đăng nhập.",
                python.version
            )),
            Some(_) => None,
        }
    }
}

/// Where an account's connection stands, for the screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    /// Making the account's portable terminal (the first time) and starting it.
    Preparing,
    Connecting,
    Connected,
    /// The terminal could not be reached; the link tries again by itself.
    Retrying(String),
    /// The server refused the login.
    Refused(String),
    Stopped,
}

/// An account of the connection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Live {
    pub(crate) account: AccountRef,
    /// The installed terminal its portable copy is made from.
    terminal: PathBuf,
    /// Kept in the settings and the credential store (after its first login).
    saved: bool,
    /// The trader forgot it: it goes once its link has stopped.
    removing: bool,
    /// The link this account listens to: what an older link of the account still says is ignored.
    link_id: Option<u64>,
    /// Its terminal is being made ready (or an older link is being stopped): a second start waits.
    starting: bool,
    pub(crate) status: Status,
    pub(crate) info: Option<AccountInfo>,
    /// The account's id in the database, once a poll has been stored.
    pub(crate) account_id: Option<i64>,
    pub(crate) positions: Vec<Position>,
    /// The positions the link has already seen (each puts the plan prompt on screen once): a
    /// position missing from one poll does not count as new when it shows again.
    seen: BTreeSet<i64>,
    /// When the last poll arrived, milliseconds since 1970 on the machine's clock.
    pub(crate) polled_at: Option<i64>,
}

impl Live {
    /// An account not kept yet: it is once its first login succeeds.
    fn new(account: AccountRef, terminal: PathBuf) -> Self {
        Self {
            account,
            terminal,
            saved: false,
            removing: false,
            link_id: None,
            starting: false,
            status: Status::Preparing,
            info: None,
            account_id: None,
            positions: Vec::new(),
            seen: BTreeSet::new(),
            polled_at: None,
        }
    }

    /// `Exness Demo (52012345)`, or the login and the server before the first login.
    pub(crate) fn title(&self) -> String {
        match self.info.as_ref().map(|info| info.name.trim()).filter(|name| !name.is_empty()) {
            Some(name) => format!("{name} ({})", self.account.login),
            None => format!("{} · {}", self.account.login, self.account.server),
        }
    }
}

/// The connection as the screens see it.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct State {
    pub(crate) setup: Setup,
    pub(crate) accounts: Vec<Live>,
    /// A position that opens puts the plan prompt on screen.
    pub(crate) prompt_plans: bool,
    /// What the connection is busy with (installing the package, checking the machine).
    pub(crate) busy: Option<String>,
    /// Why the last sign-in or setting up failed.
    pub(crate) error: Option<String>,
    /// The settings were read: the saved accounts are being connected.
    pub(crate) loaded: bool,
    env: Option<Env>,
    python_choice: Option<PathBuf>,
}

impl State {
    pub(crate) fn live(&self, account: &AccountRef) -> Option<&Live> {
        self.accounts.iter().find(|live| live.account == *account)
    }

    fn live_mut(&mut self, account: &AccountRef) -> Option<&mut Live> {
        self.accounts.iter_mut().find(|live| live.account == *account)
    }

    /// The account whose current link is `id`.
    fn of_link(&self, id: u64) -> Option<&Live> {
        self.accounts.iter().find(|live| live.link_id == Some(id))
    }

    fn of_link_mut(&mut self, id: u64) -> Option<&mut Live> {
        self.accounts.iter_mut().find(|live| live.link_id == Some(id))
    }

    /// The connected account the database knows as `account_id`, and its position `ticket`.
    pub(crate) fn position(&self, account_id: i64, ticket: i64) -> Option<(&Live, &Position)> {
        self.accounts
            .iter()
            .filter(|live| live.account_id == Some(account_id) && live.status == Status::Connected)
            .find_map(|live| live.positions.iter().find(|p| p.ticket == ticket).map(|p| (live, p)))
    }

    /// How many accounts are connected.
    pub(crate) fn connected(&self) -> usize {
        self.accounts.iter().filter(|live| live.status == Status::Connected).count()
    }
}

/// What a link's thread tells the UI thread, tagged with the link it comes from.
#[derive(Debug)]
enum Heard {
    State(u64, LinkState),
    Polled(Polled),
}

/// A poll as the UI thread hears it: the account's id once its deals are stored, and what the card
/// shows. `first` is the first poll since the login: its positions were open before.
#[derive(Debug)]
struct Polled {
    link: u64,
    account_id: Option<i64>,
    info: AccountInfo,
    positions: Vec<Position>,
    first: bool,
}

/// Whether terminals are MetaTrader or the broker in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Terminal,
    Demo,
}

/// How Python is made ready: the package into a Python of the machine (for the user), or the app's
/// own Python (installed when missing) with the pinned packages.
#[derive(Debug)]
enum Plan {
    Package(Python),
    Private,
}

/// A size as the busy line says it: `12,6 MB`.
fn megabytes(bytes: u64) -> String {
    let tenths = bytes.saturating_add(50_000) / 100_000;
    format!("{},{} MB", tenths / 10, tenths % 10)
}

/// Takes the app's copy of MetaTrader 5 into `setup`, first among the terminals; what to tell the
/// trader, in Vietnamese: why it did not work, or that MetaQuotes' copy in `Program Files` is still
/// there.
fn take_installed(setup: &mut Setup, installed: Result<installer::Installed, Mt5Error>) -> Option<String> {
    match installed {
        Ok(installed) => {
            setup.installs.retain(|install| install.terminal != installed.install.terminal);
            setup.installs.insert(0, installed.install);
            installed.left_behind
        }
        Err(error) => Some(describe(&error)),
    }
}

/// Whether a position that opens puts the plan prompt on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanPrompt {
    Ask,
    Skip,
}

type Links = Arc<Mutex<BTreeMap<AccountRef, Arc<Link>>>>;

/// Downloads an installer into a folder: [`installer::download`].
type Download = Arc<dyn Fn(&Path) -> Result<PathBuf, Mt5Error> + Send + Sync>;
/// Runs a downloaded installer and copies MetaTrader 5 into the connection's folder (the second
/// path): [`installer::run`].
type RunInstaller = Arc<dyn Fn(&Path, &Path) -> Result<installer::Installed, Mt5Error> + Send + Sync>;

/// How MetaTrader 5 is put on a machine without it: MetaQuotes' installer, downloaded then run.
/// The tests give steps that touch neither the network nor the machine.
#[derive(Clone)]
pub(crate) struct TerminalSetup {
    download: Download,
    run: RunInstaller,
}

impl TerminalSetup {
    fn official() -> Self {
        Self { download: Arc::new(installer::download), run: Arc::new(installer::run) }
    }
}

impl std::fmt::Debug for TerminalSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSetup").finish_non_exhaustive()
    }
}

/// The connection's handle; `Copy`, provided once by the root.
#[derive(Clone, Copy)]
pub(crate) struct Mt5 {
    pub(crate) state: Signal<State>,
    links: Stored<Links>,
    /// What the links' threads tell the UI thread: the sending half is given to each link.
    heard: Stored<(Sender<Heard>, Receiver<Heard>)>,
    /// The password of a sign-in that has not logged in yet, by link: kept once that link has.
    pending: Stored<BTreeMap<u64, Password>>,
    /// The id the next link gets.
    next_link: Stored<u64>,
    store: Store,
    write: Writer,
    owner: Owner,
    terminal_setup: Stored<TerminalSetup>,
}

impl std::fmt::Debug for Mt5 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mt5").finish_non_exhaustive()
    }
}

/// Provides the connection to what the root makes after it, and connects the saved accounts once
/// the database is open. Call after the store and the root owner.
pub(crate) fn provide() {
    provide_with(TerminalSetup::official());
}

/// [`provide`], with `terminal_setup` installing MetaTrader 5.
fn provide_with(terminal_setup: TerminalSetup) {
    let (sender, receiver) = mpsc::channel();
    let mt5 = Mt5 {
        state: signal(State { prompt_plans: true, ..State::default() }),
        links: stored(Links::default()),
        heard: stored((sender, receiver)),
        pending: stored(BTreeMap::new()),
        next_link: stored(1),
        store: store(),
        write: writer(),
        owner: root_owner(),
        terminal_setup: stored(terminal_setup),
    };
    provide_context(mt5);
    set_interval(DRAIN_EVERY, move || mt5.drain());
    let started = stored(false);
    effect(move || {
        if mt5.store.is_ready() && !started.get() {
            started.replace(true);
            untracked(|| mt5.load());
        }
    });
    // A restore replaces the database: each link read only from its newest deal, which the restored
    // data may not have yet, so every kept account logs in again and reads its whole history.
    let restores = stored(mt5.store.replaced());
    effect(move || {
        let now = mt5.store.replaced();
        if now != restores.get() {
            restores.replace(now);
            untracked(|| mt5.read_everything_again());
        }
    });
}

/// The connection; read while building, never in a listener.
pub(crate) fn mt5() -> Mt5 {
    use_context::<Mt5>().expect("the root provides the MT5 connection")
}

/// What the settings hold for the connection.
#[derive(Debug)]
struct Config {
    env: Env,
    saved: Vec<Saved>,
    demo: bool,
    python: Option<PathBuf>,
    prompt: bool,
}

/// The accounts the setting keeps; the inner error (in Vietnamese) when the setting cannot be read,
/// so that nothing writes over it.
fn saved_accounts(db: &Db) -> Result<Result<Vec<Saved>, String>, DbError> {
    Ok(match db.setting(ACCOUNTS_SETTING)? {
        None => Ok(Vec::new()),
        Some(json) if json.trim().is_empty() => Ok(Vec::new()),
        Some(json) => serde_json::from_str(&json)
            .map_err(|error| format!("Cài đặt {ACCOUNTS_SETTING} bị hỏng ({error}): không ghi đè lên nó.")),
    })
}

fn read_config(db: &Db) -> Result<Config, DbError> {
    let saved = saved_accounts(db)?.unwrap_or_else(|error| {
        log::error!("mt5: {error}");
        Vec::new()
    });
    let demo = db.setting(MODE_SETTING)?.as_deref() == Some("memory")
        || std::env::var("TRADELENS_MT5").is_ok_and(|v| v.trim().eq_ignore_ascii_case("memory"));
    Ok(Config {
        env: Env::read(db)?,
        saved,
        demo,
        python: db.setting(PYTHON_SETTING)?.filter(|p| !p.trim().is_empty()).map(PathBuf::from),
        prompt: db.setting(PROMPT_SETTING)?.as_deref() != Some("0"),
    })
}

/// Rewrites the kept accounts with `change`; refuses (inner error) when the setting cannot be read.
fn change_saved(db: &mut Db, change: impl FnOnce(&mut Vec<Saved>)) -> Result<Result<(), String>, DbError> {
    let mut saved = match saved_accounts(db)? {
        Ok(saved) => saved,
        Err(error) => return Ok(Err(error)),
    };
    change(&mut saved);
    let json = serde_json::to_string(&saved).map_err(|error| DbError::Invalid(error.to_string()))?;
    db.set_setting(ACCOUNTS_SETTING, &json)?;
    Ok(Ok(()))
}

/// The folder the connection keeps its files in (the worker's scripts, the portable terminals),
/// next to the database; `None` when the app keeps its data in memory. Call it on the UI thread:
/// NSH tells the data folder only there (on a blocking thread it panics: no reactive runtime).
fn folder() -> Option<PathBuf> {
    Store::database_path().and_then(|path| path.parent().map(|dir| dir.join("mt5")))
}

/// Where the app's own Python goes ([`runtime`]) under the connection's folder `files`.
fn runtime_in(files: &Path) -> PathBuf {
    files.join("runtime")
}

/// Looks at the machine: the Python to use and the terminals installed, the app's own first. `files`
/// is the connection's [`folder`] (the app's own Python and MetaTrader are under it), found on the
/// UI thread: the data folder is the engine's to tell. Blocks for seconds.
fn look(mode: Mode, python: Option<&Path>, files: Option<&Path>) -> Setup {
    if mode == Mode::Demo {
        return Setup { checked: true, demo: true, python: None, installs: Vec::new() };
    }
    if !cfg!(windows) {
        return Setup { checked: true, ..Setup::default() };
    }
    let private = files.map(|files| runtime::executable(&runtime_in(files)));
    let installs = files.and_then(installer::own).into_iter().chain(install::find_installs()).collect();
    Setup { checked: true, demo: false, python: python::find(python, private.as_deref()), installs }
}

/// What it takes to start a link: the terminals to open and the login.
struct Prepared {
    open: Opener,
    login: Login,
}

/// Makes the account's portable terminal and the worker's scripts (or nothing, in the demo).
/// `folder` is the connection's ([`folder`]), found on the UI thread. Blocks: the first copy of a
/// terminal takes a while.
fn prepare(
    setup: &Setup,
    folder: Option<&Path>,
    account: &AccountRef,
    terminal: &Path,
    password: Password,
) -> Result<Prepared, String> {
    if setup.demo {
        // Any login works in the demo: the first sign-in opens a sample account with that password.
        memory::Broker::named(&account.server).open_demo_account(
            &account.login,
            password.expose(),
            chrono::Utc::now().timestamp_millis(),
        );
        let login = Login { account: account.clone(), password, terminal: terminal.to_path_buf() };
        return Ok(Prepared { open: memory::opener(), login });
    }
    if let Some(why) = setup.missing() {
        return Err(why);
    }
    let python = setup.python.clone().ok_or_else(|| "Chưa thấy Python 3 trên máy.".to_owned())?;
    let folder = folder.ok_or_else(|| "Ứng dụng không có thư mục dữ liệu để chạy MT5.".to_owned())?;
    let script = python::install_scripts(&folder.join("scripts"))
        .map_err(|error| format!("Không ghi được script kết nối MT5: {error}"))?;
    let copy = install::portable_copy(terminal, &folder.join("terminals"), account)
        .map_err(|error| format!("Không chép được terminal MT5 cho tài khoản này: {error}"))?;
    Ok(Prepared {
        open: python::opener(python, script),
        login: Login { account: account.clone(), password, terminal: copy },
    })
}

/// Stores a poll's deals; bumps the revision when something changed. The account's id, `None`
/// when the database refused (logged: the next poll tries again).
fn store_poll(db: &SharedDb, batch: &DealBatch) -> Option<i64> {
    let Ok(mut db) = db.lock() else {
        log::error!("mt5 {}: the database lock is poisoned, the deals are not stored", batch.account.login);
        return None;
    };
    match db.upsert_batch(batch, SOURCE) {
        Ok(report) => {
            if report.inserted + report.updated > 0 {
                store::data_changed();
            }
            Some(report.account_id)
        }
        Err(error) => {
            log::error!("mt5 {}: the deals could not be stored: {error}", batch.account.login);
            None
        }
    }
}

/// An error of the connection, in words for the trader.
pub(crate) fn describe(error: &Mt5Error) -> String {
    match error {
        Mt5Error::LoginRefused(message) => format!("MT5 từ chối đăng nhập: {message}"),
        Mt5Error::Refused { message, .. } => format!("MT5 từ chối: {message}"),
        Mt5Error::Unreachable(message) => format!("Không kết nối được MT5: {message}"),
        Mt5Error::Uncertain(message) => format!("Không rõ kết quả: {message}"),
        Mt5Error::Setup(message) => message.clone(),
    }
}

/// The plan prompt of a position that just opened.
fn plan_request(live: &Live, account_id: i64, position: &Position) -> PlanRequest {
    let level = |v: f64| (v > 0.0).then_some(v);
    PlanRequest {
        target: TradeKey { account_id, position_id: position.ticket, segment: 0 },
        symbol: position.symbol.clone(),
        side: position.side.side(),
        volume: Some(position.volume),
        origin: format!("{} · lệnh {} vừa mở", live.title(), position.ticket),
        seed: EntryPlan {
            entry: level(position.open_price),
            stop_loss: level(position.sl),
            take_profit: level(position.tp),
            ..EntryPlan::default()
        },
        playbook_id: None,
    }
}

/// The listener of link `id`: stores each poll's deals on the link's thread (equity at most every
/// [`EQUITY_EVERY`]) and passes what the screens show to the UI thread.
fn listener(id: u64, db: SharedDb, sender: Sender<Heard>) -> tradelens_mt5::Listener {
    let equity_written = Mutex::new(None::<Instant>);
    Arc::new(move |event: Event| {
        let heard = match event {
            Event::State(_, state) => Heard::State(id, state),
            Event::Polled { poll, first, .. } => {
                let mut batch = poll.batch();
                if let Ok(mut written) = equity_written.lock() {
                    if written.is_some_and(|at| at.elapsed() < EQUITY_EVERY) {
                        batch.account.equity = None;
                    } else {
                        *written = Some(Instant::now());
                    }
                }
                let account_id = store_poll(&db, &batch);
                Heard::Polled(Polled { link: id, account_id, info: poll.account, positions: poll.positions, first })
            }
        };
        // The app may be closing: nobody hears it then.
        drop(sender.send(heard));
    })
}

impl Mt5 {
    /// Reads the settings, looks at the machine and connects the saved accounts.
    fn load(self) {
        self.store.read(read_config, move |config| match config {
            Ok(config) => {
                let saved = config.saved.clone();
                self.state.update(|state| {
                    state.env = Some(config.env);
                    state.prompt_plans = config.prompt;
                    state.python_choice.clone_from(&config.python);
                    state.setup.demo = config.demo;
                    state.loaded = true;
                    state.accounts = saved
                        .iter()
                        .map(|saved| Live { saved: true, ..Live::new(saved.account(), saved.terminal.clone()) })
                        .collect();
                });
                // Looking at the machine starts programs (each Python candidate): only when an account
                // is kept, else when the connection's card is first shown ([`Mt5::look_once`]).
                if !saved.is_empty() {
                    // Python first (installed again if the app's own is gone), then the logins.
                    self.check_then(move || {
                        let accounts: Vec<AccountRef> = saved.iter().map(Saved::account).collect();
                        self.ensure_python(
                            move || self.connect_saved(saved),
                            move |why| {
                                for account in &accounts {
                                    self.set_status(account, Status::Retrying(why.clone()));
                                }
                            },
                        );
                    });
                }
            }
            Err(error) => self.state.update(|state| state.error = Some(error)),
        });
    }

    /// Looks at the machine again (after the trader installed something).
    pub(crate) fn check_again(self) {
        self.check_then(|| {});
    }

    /// Looks at the machine unless it was looked at (or is being) already: the card calls it when
    /// shown, so that an app without MT5 accounts never starts Python to find out.
    pub(crate) fn look_once(self) {
        let (looked, busy, loaded) =
            self.state.with_untracked(|state| (state.setup.checked, state.busy.is_some(), state.loaded));
        if loaded && !looked && !busy {
            self.check_again();
        }
    }

    fn check_then(self, then: impl FnOnce() + 'static) {
        let (mode, python) = self.state.with_untracked(|state| {
            (if state.setup.demo { Mode::Demo } else { Mode::Terminal }, state.python_choice.clone())
        });
        let files = folder();
        self.state.update(|state| state.busy = Some("Đang kiểm tra Python và MetaTrader 5…".to_owned()));
        self.owner.run(move || {
            spawn(async move {
                let looked = blocking(move || look(mode, python.as_deref(), files.as_deref())).await;
                self.state.update(|state| {
                    match looked {
                        Ok(setup) => state.setup = setup,
                        Err(error) => {
                            state.error = Some(format!("Không kiểm tra được Python và MetaTrader 5: {error}"));
                        }
                    }
                    state.busy = None;
                });
                then();
            });
        });
    }

    /// Connects each saved account with the password the credential store keeps for it.
    fn connect_saved(self, saved: Vec<Saved>) {
        let Some(env) = self.state.with_untracked(|state| state.env.clone()) else { return };
        self.owner.run(move || {
            spawn(async move {
                let passwords = blocking(move || {
                    saved
                        .into_iter()
                        .map(|saved| (env.load_secret(&secret_name(&saved.account())), saved))
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                for (password, saved) in passwords {
                    match password {
                        Ok(Some(password)) => {
                            self.start(saved.account(), saved.terminal, Password::new(password), None);
                        }
                        Ok(None) => self.set_status(
                            &saved.account(),
                            Status::Refused("Chưa có mật khẩu của tài khoản này: hãy đăng nhập lại.".to_owned()),
                        ),
                        Err(error) => self.set_status(&saved.account(), Status::Refused(error)),
                    }
                }
            });
        });
    }

    fn set_status(self, account: &AccountRef, status: Status) {
        self.state.update(|state| {
            if let Some(live) = state.live_mut(account) {
                live.status = status;
            }
        });
    }

    /// Signs in to `account` with `password`, running it in a copy of the installed `terminal`.
    /// The account is kept (with its password) once this login succeeds.
    pub(crate) fn sign_in(self, account: AccountRef, terminal: PathBuf, password: &str) {
        let refuse = |why: &str| self.state.update(|state| state.error = Some(why.to_owned()));
        if !self.state.with_untracked(|state| state.loaded) {
            return refuse("Đang đọc cài đặt, hãy thử lại sau giây lát.");
        }
        if account.login.is_empty() || !account.login.chars().all(|c| c.is_ascii_digit()) {
            return refuse("Số tài khoản MT5 chỉ gồm chữ số.");
        }
        if account.server.is_empty() || password.is_empty() {
            return refuse("Hãy nhập đủ số tài khoản, mật khẩu và server.");
        }
        if self.state.with_untracked(|state| state.live(&account).is_some_and(|live| live.starting)) {
            return refuse("Tài khoản này đang được kết nối, hãy chờ một chút.");
        }
        if self.state.with_untracked(|state| state.busy.is_some()) {
            return refuse("TradeLens đang cài đặt hoặc kiểm tra máy, hãy chờ xong rồi đăng nhập.");
        }
        if self.state.with_untracked(|state| !state.setup.demo && !state.setup.checked) {
            return refuse("Đang kiểm tra Python và MetaTrader 5 trên máy, hãy thử lại sau giây lát.");
        }
        self.state.update(|state| {
            state.error = None;
            match state.live_mut(&account) {
                Some(live) => {
                    live.terminal.clone_from(&terminal);
                    live.removing = false;
                }
                None => state.accounts.push(Live::new(account.clone(), terminal.clone())),
            }
        });
        // Python first (installed by the app when the machine has none), then the login. The
        // password is kept once it logs in: for a new account, and as the new one of an account kept.
        let password = Password::new(password);
        let wanted = account.clone();
        self.ensure_python(
            move || self.start(account, terminal, password.clone(), Some(password)),
            move |why| {
                self.state.update(|state| {
                    state.accounts.retain(|live| live.saved || live.account != wanted);
                    state.error = Some(why);
                });
            },
        );
    }

    /// Makes the account's terminal ready, then starts a new link for it once the old one (if any)
    /// has stopped: two workers never drive one portable terminal. `keep` is the password to keep
    /// once this link logs in. A start while another is making the terminal ready is dropped.
    fn start(self, account: AccountRef, terminal: PathBuf, password: Password, keep: Option<Password>) {
        if self.state.with_untracked(|state| state.live(&account).is_none_or(|live| live.starting)) {
            return;
        }
        let Some(db) = self.store.database() else {
            return self.set_status(&account, Status::Retrying("Chưa mở được cơ sở dữ liệu.".to_owned()));
        };
        let id = self.next_link.with(|next| {
            *next += 1;
            *next - 1
        });
        let old = self.links.with(|links| links.lock().expect("lock poisoned").remove(&account));
        self.state.update(|state| {
            if let Some(live) = state.live_mut(&account) {
                live.link_id = Some(id);
                live.starting = true;
                live.status = Status::Preparing;
                live.seen.clear();
            }
        });
        if let Some(keep) = keep {
            self.pending.with(|pending| pending.insert(id, keep));
        }
        let setup = self.state.with_untracked(|state| state.setup.clone());
        let sender = self.heard.with(|(sender, _)| sender.clone());
        let files = folder();
        self.owner.run(move || {
            spawn(async move {
                let wanted = account.clone();
                let prepared = blocking(move || {
                    if let Some(old) = old {
                        old.stop_and_wait();
                    }
                    prepare(&setup, files.as_deref(), &wanted, &terminal, password)
                })
                .await
                .unwrap_or_else(|error| Err(format!("Luồng chuẩn bị MT5 bị dừng: {error}")));
                // The trader may have forgotten the account, or started it again, meanwhile.
                let current = self.state.with_untracked(|state| state.of_link(id).is_some_and(|live| !live.removing));
                if !current {
                    self.pending.with(|pending| pending.remove(&id));
                    return;
                }
                match prepared {
                    Ok(prepared) => {
                        let link = Link::start(
                            prepared.login,
                            prepared.open,
                            LinkSettings::default(),
                            listener(id, db, sender),
                        );
                        self.links.with(|links| links.lock().expect("lock poisoned").insert(account, Arc::new(link)));
                        self.state.update(|state| {
                            if let Some(live) = state.of_link_mut(id) {
                                live.starting = false;
                            }
                        });
                    }
                    Err(why) => {
                        self.pending.with(|pending| pending.remove(&id));
                        // A new sign-in that cannot start is not listed: the form says why.
                        self.state.update(|state| {
                            state.accounts.retain(|live| live.saved || live.link_id != Some(id));
                            if let Some(live) = state.of_link_mut(id) {
                                live.starting = false;
                                live.status = Status::Retrying(why.clone());
                            }
                            state.error = Some(why);
                        });
                    }
                }
            });
        });
    }

    fn stop_link(self, account: &AccountRef) {
        let link = self.links.with(|links| links.lock().expect("lock poisoned").remove(account));
        if let Some(link) = link {
            link.stop();
        }
    }

    fn link(self, account: &AccountRef) -> Option<Arc<Link>> {
        self.links.with(|links| links.lock().expect("lock poisoned").get(account).cloned())
    }

    /// Logs in again every kept account that is not being started, so that each new link reads the
    /// account's whole history into the database (after a restore replaced it).
    fn read_everything_again(self) {
        let kept: Vec<Saved> = self.state.with_untracked(|state| {
            state
                .accounts
                .iter()
                .filter(|live| live.saved && !live.starting && !live.removing)
                .map(|live| Saved {
                    login: live.account.login.clone(),
                    server: live.account.server.clone(),
                    terminal: live.terminal.clone(),
                })
                .collect()
        });
        if !kept.is_empty() {
            log::info!("mt5: the database was replaced, {} account(s) read again", kept.len());
            self.connect_saved(kept);
        }
    }

    /// Logs in to `account` again with the password kept for it.
    pub(crate) fn reconnect(self, account: &AccountRef) {
        let saved = self.state.with_untracked(|state| {
            state.live(account).map(|live| Saved {
                login: live.account.login.clone(),
                server: live.account.server.clone(),
                terminal: live.terminal.clone(),
            })
        });
        if let Some(saved) = saved {
            self.connect_saved(vec![saved]);
        }
    }

    /// Forgets `account`: stops it, removes it from the settings and its password from the
    /// credential store; its portable terminal is removed once the link has stopped.
    pub(crate) fn forget(self, account: &AccountRef) {
        let had_link = self.link(account).is_some();
        self.state.update(|state| {
            if let Some(live) = state.live_mut(account) {
                live.removing = true;
            }
            if !had_link {
                state.accounts.retain(|live| live.account != *account);
            }
        });
        self.stop_link(account);
        let wanted = account.clone();
        let env = self.state.with_untracked(|state| state.env.clone());
        self.write.run(
            move |db: &mut Db| {
                if let Err(error) = change_saved(db, |saved| saved.retain(|kept| kept.account() != wanted))? {
                    return Ok(Err(error));
                }
                Ok(env.map_or(Ok(()), |env| env.clear_secret(&secret_name(&wanted))))
            },
            move |result| {
                if let Err(error) = result.and_then(|cleared| cleared) {
                    self.state.update(|state| state.error = Some(error));
                }
            },
        );
    }

    /// Keeps `account` in the settings and its password in the credential store, after a login.
    fn keep(self, account: &AccountRef, password: Password) {
        let Some(live) = self.state.with_untracked(|state| state.live(account).cloned()) else { return };
        let Some(env) = self.state.with_untracked(|state| state.env.clone()) else { return };
        let kept = Saved { login: live.account.login, server: live.account.server, terminal: live.terminal };
        let wanted = account.clone();
        self.write.run(
            move |db: &mut Db| {
                let account = kept.account();
                if let Err(error) = change_saved(db, |saved| {
                    saved.retain(|other| other.account() != account);
                    saved.push(kept);
                })? {
                    return Ok(Err(error));
                }
                Ok(env.save_secret(&secret_name(&wanted), password.expose()))
            },
            move |result| {
                if let Err(error) = result.and_then(|saved| saved) {
                    self.state.update(|state| state.error = Some(format!("Không lưu được tài khoản MT5: {error}")));
                }
            },
        );
    }

    /// Takes what the links' threads said and shows it.
    fn drain(self) {
        let heard: Vec<Heard> = self.heard.with(|(_, receiver)| receiver.try_iter().collect());
        for heard in heard {
            match heard {
                Heard::State(id, state) => self.on_state(id, state),
                Heard::Polled(polled) => self.on_poll(polled),
            }
        }
    }

    /// What link `id` says of itself; nothing when it is not the account's current link.
    fn on_state(self, id: u64, state: LinkState) {
        let Some(live) = self.state.with_untracked(|s| s.of_link(id).cloned()) else { return };
        let account = &live.account;
        let set = |status: Status| {
            self.state.update(|s| {
                if let Some(live) = s.of_link_mut(id) {
                    live.status = status;
                }
            });
        };
        match state {
            LinkState::Connecting => set(Status::Connecting),
            LinkState::Connected(info) => {
                let keep = self.pending.with(|pending| pending.remove(&id));
                self.state.update(|s| {
                    if let Some(live) = s.of_link_mut(id) {
                        live.status = Status::Connected;
                        live.info = Some(info);
                        live.saved |= keep.is_some();
                    }
                });
                if let Some(password) = keep {
                    self.keep(account, password);
                }
            }
            LinkState::Retrying(error) => set(Status::Retrying(describe(&error))),
            LinkState::Refused(error) => {
                self.pending.with(|pending| pending.remove(&id));
                if live.saved {
                    // An account kept before stays listed (with the password it had), saying why.
                    set(Status::Refused(describe(&error)));
                    self.state.update(|s| s.error = Some(describe(&error)));
                } else {
                    // A first sign-in that failed: nothing is kept, the form says why.
                    self.stop_link(account);
                    self.state.update(|s| {
                        s.accounts.retain(|live| live.link_id != Some(id));
                        s.error = Some(describe(&error));
                    });
                }
            }
            LinkState::Stopped => {
                if live.removing {
                    self.state.update(|s| s.accounts.retain(|live| live.link_id != Some(id)));
                    self.remove_copy(account);
                } else {
                    set(Status::Stopped);
                }
            }
        }
    }

    fn remove_copy(self, account: &AccountRef) {
        let Some(folder) = folder() else { return };
        let account = account.clone();
        self.owner.run(move || {
            spawn(async move {
                let removed = blocking(move || install::remove_copy(&folder.join("terminals"), &account)).await;
                if let Ok(Err(error)) = removed {
                    log::warn!("mt5: the portable terminal could not be removed: {error}");
                }
            });
        });
    }

    /// What a poll of link `id` read; a position the link has not seen before puts the plan prompt
    /// on screen, unless it was already open when the link logged in.
    fn on_poll(self, polled: Polled) {
        let Polled { link, account_id, info, positions, first } = polled;
        let Some(live) = self.state.with_untracked(|s| s.of_link(link).cloned()) else { return };
        if live.removing {
            return;
        }
        let opened: Vec<Position> = if first {
            Vec::new()
        } else {
            positions.iter().filter(|p| !live.seen.contains(&p.ticket)).cloned().collect()
        };
        let now = chrono::Utc::now().timestamp_millis();
        self.state.update(|s| {
            if let Some(live) = s.of_link_mut(link) {
                live.seen.extend(positions.iter().map(|p| p.ticket));
                live.info = Some(info);
                live.positions = positions;
                live.polled_at = Some(now);
                if account_id.is_some() {
                    live.account_id = account_id;
                }
            }
        });
        let prompt = self.state.with_untracked(|s| s.prompt_plans);
        if let (true, Some(account_id)) = (prompt, account_id.or(live.account_id)) {
            for position in &opened {
                entry_plan::enqueue(plan_request(&live, account_id, position));
            }
        }
    }

    /// Turns the plan prompt for new positions on or off, and keeps the choice.
    pub(crate) fn set_prompt_plans(self, prompt: PlanPrompt) {
        self.state.update(|state| state.prompt_plans = prompt == PlanPrompt::Ask);
        self.write.run(
            move |db: &mut Db| db.set_setting(PROMPT_SETTING, if prompt == PlanPrompt::Ask { "1" } else { "0" }),
            move |result| {
                if let Err(error) = result {
                    self.state.update(|state| state.error = Some(error));
                }
            },
        );
    }

    /// Makes Python ready: the button "Cài Python tự động" (or "Cài gói `MetaTrader5`").
    pub(crate) fn install_python(self) {
        self.state.update(|state| state.error = None);
        self.ensure_python(|| {}, move |why| self.state.update(|state| state.error = Some(why)));
    }

    /// Installs MetaTrader 5 with MetaQuotes' installer: the button "Cài MetaTrader 5 tự động".
    /// Downloads the installer into the connection's folder, runs it, copies what it installed into
    /// the connection's folder and removes MetaQuotes' copy (Windows asks the trader to allow both),
    /// then takes the app's copy, so that the form can sign in at once. Nothing is
    /// done in the demo, when a terminal was found already, or while something else installs. The
    /// connection is marked busy before this returns, so a second press while it runs is refused.
    pub(crate) fn install_terminal(self) {
        if !self.state.with_untracked(|state| state.setup.needs_terminal()) {
            return;
        }
        if self.state.with_untracked(|state| state.busy.is_some()) {
            return self.state.update(|state| {
                state.error = Some("TradeLens đang cài đặt hoặc kiểm tra máy, hãy chờ xong rồi thử lại.".to_owned());
            });
        }
        let Some(files) = folder() else {
            return self.state.update(|state| {
                state.error = Some("Ứng dụng không có thư mục dữ liệu để tải trình cài MetaTrader 5.".to_owned());
            });
        };
        let downloads = files.join("installer");
        let TerminalSetup { download, run } = self.terminal_setup.with(|setup| setup.clone());
        let state = self.state;
        let busy = move |text: String| state.update(|s| s.busy = Some(text));
        state.update(|s| s.error = None);
        busy(format!(
            "Đang tải trình cài MetaTrader 5 của MetaQuotes (khoảng {})…",
            megabytes(installer::INSTALLER_SIZE)
        ));
        self.owner.run(move || {
            spawn(async move {
                fn stopped<T>(error: &dyn std::fmt::Display) -> Result<T, Mt5Error> {
                    Err(Mt5Error::Setup(format!("Luồng cài đặt bị dừng: {error}")))
                }
                let downloaded = blocking(move || download(&downloads)).await.unwrap_or_else(|e| stopped(&e));
                let installed = match downloaded {
                    Ok(installer) => {
                        busy("Đang cài MetaTrader 5 rồi chép vào thư mục của TradeLens: Windows sẽ hỏi hai lần có cho phép MetaQuotes cài rồi gỡ bản trong Program Files không, hãy bấm \"Có\" cả hai. Thường mất vài phút…".to_owned());
                        blocking(move || run(&installer, &files)).await.unwrap_or_else(|e| stopped(&e))
                    }
                    Err(error) => Err(error),
                };
                state.update(|s| {
                    s.busy = None;
                    // A press refused while installing said so meanwhile: the outcome replaces it.
                    s.error = take_installed(&mut s.setup, installed);
                });
            });
        });
    }

    /// Makes Python ready for the connection, then runs `then`: on a machine without Python the app
    /// installs its own ([`runtime`]), and its own gets the pinned packages; a Python of the machine
    /// without the package gets it for the user. `failed` hears why it could not, in Vietnamese.
    /// Nothing is installed in the demo or when Python is ready already. The connection is marked
    /// busy before this returns, so a second press while it runs is refused.
    fn ensure_python(self, then: impl FnOnce() + 'static, failed: impl FnOnce(String) + 'static) {
        let setup = self.state.with_untracked(|state| state.setup.clone());
        if !setup.needs_python() {
            return then();
        }
        if self.state.with_untracked(|state| state.busy.is_some()) {
            return failed("TradeLens đang cài đặt, hãy chờ xong rồi thử lại.".to_owned());
        }
        let Some(files) = folder() else {
            return failed("Ứng dụng không có thư mục dữ liệu để cài Python.".to_owned());
        };
        let root = runtime_in(&files);
        let plan = match setup.python {
            Some(python) if !runtime::is_private(&python, &root) => Plan::Package(python),
            _ => Plan::Private,
        };
        let source = runtime::Source::official();
        let state = self.state;
        let busy = move |text: String| state.update(|s| s.busy = Some(text));
        busy(match &plan {
            Plan::Package(_) => "Đang cài gói MetaTrader5 (pip), có thể mất vài phút…".to_owned(),
            Plan::Private => format!("Đang tải Python cho TradeLens ({})…", megabytes(source.python.size)),
        });
        let choice = self.state.with_untracked(|state| state.python_choice.clone());
        self.owner.run(move || {
            spawn(async move {
                let stopped =
                    |error: &dyn std::fmt::Display| Err(Mt5Error::Setup(format!("Luồng cài đặt bị dừng: {error}")));
                let installed = match plan {
                    Plan::Package(python) => {
                        blocking(move || python::install_package(&python)).await.unwrap_or_else(|e| stopped(&e))
                    }
                    Plan::Private => {
                        let (python_root, python_source) = (root.clone(), source.clone());
                        let python = blocking(move || runtime::install_python(&python_root, &python_source).map(drop))
                            .await
                            .unwrap_or_else(|e| stopped(&e));
                        match python {
                            Ok(()) => {
                                busy(
                                    "Đang cài gói MetaTrader5 vào Python của TradeLens, có thể mất vài phút…"
                                        .to_owned(),
                                );
                                blocking(move || runtime::install_packages(&root, &source))
                                    .await
                                    .unwrap_or_else(|e| stopped(&e))
                            }
                            Err(error) => Err(error),
                        }
                    }
                };
                busy("Đang kiểm tra lại Python…".to_owned());
                // A look that could not run is not taken for a machine ready to connect.
                let looked = blocking(move || look(Mode::Terminal, choice.as_deref(), Some(&files))).await.ok();
                let ready = looked.as_ref().is_some_and(|setup| setup.checked && !setup.needs_python());
                self.state.update(|state| {
                    if let Some(setup) = looked {
                        state.setup = setup;
                    }
                    state.busy = None;
                });
                match (installed, ready) {
                    (_, true) => then(),
                    (Err(error), false) => failed(describe(&error)),
                    (Ok(()), false) => {
                        failed("Đã cài xong nhưng chưa dùng được Python với gói MetaTrader5.".to_owned());
                    }
                }
            });
        });
    }

    /// Closes `volume` lots of position `ticket` of `account` (all of it when `None`); `done` hears
    /// how it went, in Vietnamese.
    pub(crate) fn close(
        self,
        account: &AccountRef,
        ticket: i64,
        volume: Option<f64>,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        self.order(account, move |link| link.close(ticket, volume), done);
    }

    /// Puts `levels` on position `ticket` of `account`.
    pub(crate) fn modify(
        self,
        account: &AccountRef,
        ticket: i64,
        levels: Levels,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        self.order(account, move |link| link.modify(ticket, levels), done);
    }

    fn order(
        self,
        account: &AccountRef,
        ask: impl FnOnce(&Link) -> Result<(), Mt5Error> + Send + 'static,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        let connected =
            self.state.with_untracked(|state| state.live(account).is_some_and(|l| l.status == Status::Connected));
        let link = self.link(account).filter(|_| connected);
        let Some(link) = link else {
            return done(Err("Tài khoản MT5 này chưa kết nối: lệnh chưa được gửi.".to_owned()));
        };
        self.owner.run(move || {
            spawn(async move {
                let result = blocking(move || ask(&link))
                    .await
                    .unwrap_or_else(|error| Err(Mt5Error::Uncertain(format!("luồng gửi lệnh bị dừng: {error}"))));
                done(result.map_err(|error| describe(&error)));
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_missing_is_said_in_order() {
        let mut setup = Setup { checked: true, ..Setup::default() };
        if cfg!(windows) {
            assert!(setup.missing().is_some_and(|why| why.contains("Cài Python tự động")), "{:?}", setup.missing());
            assert!(setup.needs_python(), "without Python the app installs its own");
            setup.python =
                Some(Python { program: "python".into(), args: Vec::new(), version: "3.12".into(), has_package: false });
            assert!(setup.missing().is_some_and(|why| why.contains("MetaTrader5")));
            assert!(setup.needs_python(), "a Python without the package gets it");
            setup.python.as_mut().expect("set above").has_package = true;
            assert_eq!(setup.missing(), None);
            assert!(!setup.needs_python());
        } else {
            assert!(setup.missing().is_some_and(|why| why.contains("Windows")));
        }
        assert_eq!(Setup { demo: true, ..Setup::default() }.missing(), None, "the demo needs nothing");
    }

    #[test]
    fn metatrader_is_offered_only_when_no_terminal_was_found() {
        let mut setup = Setup { checked: true, ..Setup::default() };
        assert_eq!(setup.needs_terminal(), cfg!(windows), "a machine looked at without a terminal, on Windows");
        setup.installs = vec![Install::at(PathBuf::from("C:/MT5/terminal64.exe"))];
        assert!(!setup.needs_terminal(), "a terminal found is enough");
        assert!(!Setup { checked: true, demo: true, ..Setup::default() }.needs_terminal(), "the demo needs none");
        assert!(!Setup::default().needs_terminal(), "not before the machine is looked at");
    }

    #[test]
    fn what_the_installer_left_is_taken_or_said() {
        let picked = Install::at(PathBuf::from("D:/Broker MT5/terminal64.exe"));
        let mut setup = Setup { checked: true, installs: vec![picked.clone()], ..Setup::default() };
        let own = Install {
            terminal: "C:/data/mt5/metatrader/terminal64.exe".into(),
            name: "MetaTrader 5 (TradeLens)".into(),
        };
        let installed = installer::Installed { install: own.clone(), left_behind: None };
        assert_eq!(take_installed(&mut setup, Ok(installed.clone())), None, "nothing to say");
        assert_eq!(setup.installs, vec![own.clone(), picked.clone()], "the app's copy comes first");
        let left =
            installer::Installed { left_behind: Some("bản trong Program Files còn đó".into()), ..installed };
        assert_eq!(take_installed(&mut setup, Ok(left)).as_deref(), Some("bản trong Program Files còn đó"));
        assert_eq!(setup.installs, vec![own.clone(), picked.clone()], "taken once");
        let refused = take_installed(&mut setup, Err(Mt5Error::Setup("Trình cài đã dừng".to_owned())));
        assert_eq!(refused.as_deref(), Some("Trình cài đã dừng"));
        assert_eq!(setup.installs, vec![own, picked], "a failure keeps what was found");
    }

    /// `install_terminal` on the real engine, with steps that touch neither the network nor the
    /// machine: each waits until the test lets it go, so the busy lines can be read in turn.
    #[cfg(windows)]
    mod installing_metatrader {
        use std::{
            sync::atomic::{AtomicU8, Ordering},
            time::Duration,
        };

        use nsh_core::Size;
        use nsh_engine::{EngineConfig, Root};
        use nsh_testing::Headless;

        use super::*;
        use crate::ui::controls::{Variant, action, provide_root_owner};

        const WAIT: Duration = Duration::from_secs(30);

        /// How far the gated steps may go: 1 lets the download end, 2 the install.
        static ALLOWED: AtomicU8 = AtomicU8::new(0);

        fn wait_for_step(step: u8) {
            for _ in 0..6000 {
                if ALLOWED.load(Ordering::SeqCst) >= step {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn terminal() -> Install {
            Install::at(PathBuf::from(r"C:\Program Files\MetaTrader 5\terminal64.exe"))
        }

        /// The connection's state in words, and the button.
        fn shown(mt5: Mt5) -> impl IntoElement {
            mt5.state.update(|state| {
                state.setup = Setup {
                    checked: true,
                    python: Some(Python {
                        program: "python".into(),
                        args: Vec::new(),
                        version: "3.14".into(),
                        has_package: true,
                    }),
                    ..Setup::default()
                };
            });
            view()
                .gap_2()
                .child(text(move || {
                    mt5.state.with(|state| {
                        let installs: Vec<&str> = state.setup.installs.iter().map(|i| i.name.as_str()).collect();
                        format!(
                            "busy: {} | error: {} | installs: {}",
                            state.busy.as_deref().unwrap_or("-"),
                            state.error.as_deref().unwrap_or("-"),
                            installs.join(",")
                        )
                    })
                }))
                .child(action("Cài MT5", Variant::Primary, move || mt5.install_terminal()))
        }

        #[component]
        fn Installs() -> impl IntoElement {
            crate::ui::theme::install();
            store::provide();
            provide_root_owner();
            provide_with(TerminalSetup {
                download: Arc::new(|folder| {
                    wait_for_step(1);
                    Ok(folder.join("mt5setup.exe"))
                }),
                run: Arc::new(|installer, files| {
                    assert!(installer.ends_with("mt5\\installer\\mt5setup.exe"), "{}", installer.display());
                    assert!(files.ends_with("mt5"), "{}", files.display());
                    wait_for_step(2);
                    Ok(installer::Installed { install: terminal(), left_behind: None })
                }),
            });
            shown(mt5())
        }

        #[component]
        fn Refused() -> impl IntoElement {
            crate::ui::theme::install();
            store::provide();
            provide_root_owner();
            provide_with(TerminalSetup {
                download: Arc::new(|folder| Ok(folder.join("mt5setup.exe"))),
                run: Arc::new(|_, _| {
                    Err(Mt5Error::Setup("Trình cài MetaTrader 5 đã dừng mà chưa cài xong".to_owned()))
                }),
            });
            shown(mt5())
        }

        fn start(root: Root) -> (tempfile::TempDir, Headless) {
            let dir = tempfile::tempdir().expect("a temp dir");
            let config = EngineConfig {
                viewport: Size::new(px(900.0), px(400.0)),
                data_dir: Some(dir.path().to_path_buf()),
                ..EngineConfig::new(root)
            };
            (dir, Headless::start(config).expect("the engine starts"))
        }

        #[test]
        fn the_card_says_each_step_then_takes_the_terminal_installed() {
            let (_dir, mut app) = start(Root::of::<Installs>());
            app.wait_for_within("text:/installs: $/", WAIT).expect("no terminal at first");
            app.press("role:button[Cài MT5]").expect("the install button");
            app.wait_for_within("text:/busy: Đang tải trình cài MetaTrader 5/", WAIT).expect("the download is said");
            app.press("role:button[Cài MT5]").expect("the button is pressed again");
            app.wait_for_within("text:/error: TradeLens đang cài đặt/", WAIT).expect("a second press is refused");
            ALLOWED.store(1, Ordering::SeqCst);
            app.wait_for_within(
                "text:/busy: Đang cài MetaTrader 5 rồi chép vào thư mục của TradeLens: Windows sẽ hỏi hai lần/",
                WAIT,
            )
            .expect("the install is said, with what Windows will ask");
            ALLOWED.store(2, Ordering::SeqCst);
            app.wait_for_within("text:busy: - | error: - | installs: MetaTrader 5", WAIT)
                .expect("the terminal installed is taken, nothing is busy and the refusal is gone");
            app.press("role:button[Cài MT5]").expect("the button once installed");
            app.wait_for_within("text:/installs: MetaTrader 5$/", WAIT).expect("nothing more is done");
            assert!(!app.screen().shows("busy: Đang"), "a terminal found is not installed again");
        }

        #[test]
        fn an_install_that_failed_says_why_and_leaves_the_button() {
            let (_dir, mut app) = start(Root::of::<Refused>());
            app.wait_for_within("text:/installs: $/", WAIT).expect("no terminal at first");
            app.press("role:button[Cài MT5]").expect("the install button");
            app.wait_for_within("text:/busy: - \\| error: Trình cài MetaTrader 5 đã dừng/", WAIT)
                .expect("the failure is said once nothing is busy");
            assert!(!app.screen().shows("installs: MetaTrader"), "nothing was taken");
        }
    }

    #[test]
    fn a_saved_account_reads_back_and_its_secret_is_named_after_it() {
        let saved = Saved { login: "123".into(), server: "Demo".into(), terminal: "C:/MT5/terminal64.exe".into() };
        let json = serde_json::to_string(&vec![saved.clone()]).expect("the list is written");
        let back: Vec<Saved> = serde_json::from_str(&json).expect("the list reads back");
        assert_eq!(back, vec![saved.clone()]);
        assert_eq!(secret_name(&saved.account()), "mt5:123@Demo");
    }
}
