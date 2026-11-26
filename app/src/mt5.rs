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
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use nsh_core::prelude::*;
use serde::{Deserialize, Serialize};
use tradelens_core::{AccountInfo, Db, DbError, TradeKey, journal::EntryPlan};
use tradelens_mt5::{
    AccountRef, Event, Levels, Link, LinkSettings, LinkState, Login, Mt5Error, Opener, Password, Position,
    install::{self, Install},
    memory,
    python::{self, Python},
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

/// What the deals a link writes are stored as (`deals.source`): an MT5 feed, so they are not editable.
const SOURCE: &str = "mt5";

/// How often the timer takes what the links' threads said.
const DRAIN_EVERY: Duration = Duration::from_millis(250);

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
            None => Some(
                "Chưa thấy Python 3 trên máy. Cài Python từ python.org (tick \"Add python.exe to PATH\") rồi bấm \"Kiểm tra lại\"."
                    .to_owned(),
            ),
            Some(python) if !python.has_package => {
                Some(format!("Python {} chưa có gói MetaTrader5: bấm \"Cài gói MetaTrader5\".", python.version))
            }
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
    pub(crate) status: Status,
    pub(crate) info: Option<AccountInfo>,
    /// The account's id in the database, once a poll has been stored.
    pub(crate) account_id: Option<i64>,
    pub(crate) positions: Vec<Position>,
    /// When the last poll arrived, milliseconds since 1970 on the machine's clock.
    pub(crate) polled_at: Option<i64>,
}

impl Live {
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
    pub(crate) busy: Option<&'static str>,
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

/// What a link's thread tells the UI thread.
#[derive(Debug)]
enum Heard {
    State(AccountRef, LinkState),
    Polled(Polled),
}

/// A poll as the UI thread hears it: the account's id once its deals are stored, and what the card
/// shows. `first` is the first poll since the login: its positions were open before.
#[derive(Debug)]
struct Polled {
    account: AccountRef,
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

/// Whether a position that opens puts the plan prompt on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanPrompt {
    Ask,
    Skip,
}

type Links = Arc<Mutex<BTreeMap<AccountRef, Arc<Link>>>>;

/// The connection's handle; `Copy`, provided once by the root.
#[derive(Clone, Copy)]
pub(crate) struct Mt5 {
    pub(crate) state: Signal<State>,
    links: Stored<Links>,
    sender: Stored<Sender<Heard>>,
    receiver: Stored<Receiver<Heard>>,
    /// The password of a sign-in that has not logged in yet, kept once it has.
    pending: Stored<BTreeMap<AccountRef, Password>>,
    store: Store,
    write: Writer,
    owner: Owner,
}

impl std::fmt::Debug for Mt5 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mt5").finish_non_exhaustive()
    }
}

/// Provides the connection to what the root makes after it, and connects the saved accounts once
/// the database is open. Call after the store and the root owner.
pub(crate) fn provide() {
    let (sender, receiver) = mpsc::channel();
    let mt5 = Mt5 {
        state: signal(State { prompt_plans: true, ..State::default() }),
        links: stored(Links::default()),
        sender: stored(sender),
        receiver: stored(receiver),
        pending: stored(BTreeMap::new()),
        store: store(),
        write: writer(),
        owner: root_owner(),
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

fn read_config(db: &Db) -> Result<Config, DbError> {
    let saved = db.setting(ACCOUNTS_SETTING)?.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
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

/// The folder the connection keeps its files in (the worker's scripts, the portable terminals),
/// next to the database; `None` when the app keeps its data in memory.
fn folder() -> Option<PathBuf> {
    Store::database_path().and_then(|path| path.parent().map(|dir| dir.join("mt5")))
}

/// Looks at the machine: the Python to use and the terminals installed. Blocks for seconds.
fn look(mode: Mode, python: Option<&Path>) -> Setup {
    if mode == Mode::Demo {
        return Setup { checked: true, demo: true, python: None, installs: Vec::new() };
    }
    if !cfg!(windows) {
        return Setup { checked: true, ..Setup::default() };
    }
    Setup { checked: true, demo: false, python: python::find(python), installs: install::find_installs() }
}

/// What it takes to start a link: the terminals to open and the login.
struct Prepared {
    open: Opener,
    login: Login,
}

/// Makes the account's portable terminal and the worker's scripts (or nothing, in the demo).
/// Blocks: the first copy of a terminal takes a while.
fn prepare(setup: &Setup, account: &AccountRef, terminal: &Path, password: Password) -> Result<Prepared, String> {
    if setup.demo {
        let login = Login { account: account.clone(), password, terminal: terminal.to_path_buf() };
        return Ok(Prepared { open: memory::opener(), login });
    }
    if let Some(why) = setup.missing() {
        return Err(why);
    }
    let python = setup.python.clone().ok_or_else(|| "Chưa thấy Python 3 trên máy.".to_owned())?;
    let folder = folder().ok_or_else(|| "Ứng dụng không có thư mục dữ liệu để chạy MT5.".to_owned())?;
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
fn store_poll(db: &SharedDb, poll: &tradelens_mt5::Poll) -> Option<i64> {
    let mut db = db.lock().expect("database lock poisoned");
    match db.upsert_batch(&poll.batch(), SOURCE) {
        Ok(report) => {
            if report.inserted + report.updated > 0 {
                store::data_changed();
            }
            Some(report.account_id)
        }
        Err(error) => {
            log::error!("mt5 {}: the deals could not be stored: {error}", poll.account.login);
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
                        .map(|saved| Live {
                            account: saved.account(),
                            terminal: saved.terminal.clone(),
                            saved: true,
                            removing: false,
                            status: Status::Preparing,
                            info: None,
                            account_id: None,
                            positions: Vec::new(),
                            polled_at: None,
                        })
                        .collect();
                });
                self.check_then(move || self.connect_saved(saved));
            }
            Err(error) => self.state.update(|state| state.error = Some(error)),
        });
    }

    /// Looks at the machine again (after the trader installed something).
    pub(crate) fn check_again(self) {
        self.check_then(|| {});
    }

    fn check_then(self, then: impl FnOnce() + 'static) {
        let (mode, python) = self.state.with_untracked(|state| {
            (if state.setup.demo { Mode::Demo } else { Mode::Terminal }, state.python_choice.clone())
        });
        self.state.update(|state| state.busy = Some("Đang kiểm tra Python và MetaTrader 5…"));
        self.owner.run(move || {
            spawn(async move {
                let setup = blocking(move || look(mode, python.as_deref())).await.unwrap_or_default();
                self.state.update(|state| {
                    state.setup = setup;
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
                        Ok(Some(password)) => self.start(saved.account(), saved.terminal, Password::new(password)),
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
    /// The account is kept (with its password) once its first login succeeds.
    pub(crate) fn sign_in(self, account: AccountRef, terminal: PathBuf, password: &str) {
        if account.login.is_empty() || !account.login.chars().all(|c| c.is_ascii_digit()) {
            return self.state.update(|state| state.error = Some("Số tài khoản MT5 chỉ gồm chữ số.".to_owned()));
        }
        if account.server.is_empty() || password.is_empty() {
            return self
                .state
                .update(|state| state.error = Some("Hãy nhập đủ số tài khoản, mật khẩu và server.".to_owned()));
        }
        self.state.update(|state| {
            state.error = None;
            match state.live_mut(&account) {
                Some(live) => {
                    live.terminal.clone_from(&terminal);
                    live.status = Status::Preparing;
                    live.removing = false;
                }
                None => state.accounts.push(Live {
                    account: account.clone(),
                    terminal: terminal.clone(),
                    saved: false,
                    removing: false,
                    status: Status::Preparing,
                    info: None,
                    account_id: None,
                    positions: Vec::new(),
                    polled_at: None,
                }),
            }
        });
        // Kept once it logs in: for a new account, and for a new password of one already kept.
        self.pending.with(|pending| pending.insert(account.clone(), Password::new(password)));
        self.start(account, terminal, Password::new(password));
    }

    /// Makes the account's terminal ready, then starts its link.
    fn start(self, account: AccountRef, terminal: PathBuf, password: Password) {
        self.stop_link(&account);
        self.set_status(&account, Status::Preparing);
        let setup = self.state.with_untracked(|state| state.setup.clone());
        let Some(db) = self.store.database() else {
            return self.set_status(&account, Status::Retrying("Chưa mở được cơ sở dữ liệu.".to_owned()));
        };
        let sender = self.sender.with(|sender| sender.clone());
        self.owner.run(move || {
            spawn(async move {
                let wanted = account.clone();
                let prepared = blocking(move || prepare(&setup, &wanted, &terminal, password))
                    .await
                    .unwrap_or_else(|error| Err(format!("Luồng chuẩn bị MT5 bị dừng: {error}")));
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(why) => {
                        // A new sign-in that cannot start is not listed: the form says why.
                        self.pending.with(|pending| pending.remove(&account));
                        return self.state.update(|state| {
                            state.accounts.retain(|live| live.saved || live.account != account);
                            if let Some(live) = state.live_mut(&account) {
                                live.status = Status::Retrying(why.clone());
                            }
                            state.error = Some(why);
                        });
                    }
                };
                // The trader may have forgotten the account while its terminal was being copied.
                if self.state.with_untracked(|state| state.live(&account).is_none_or(|live| live.removing)) {
                    return;
                }
                let listen = Arc::new(move |event: Event| {
                    let heard = match event {
                        Event::State(account, state) => Heard::State(account, state),
                        Event::Polled { account, poll, first } => {
                            let account_id = store_poll(&db, &poll);
                            Heard::Polled(Polled {
                                account,
                                account_id,
                                info: poll.account,
                                positions: poll.positions,
                                first,
                            })
                        }
                    };
                    // The app may be closing: nobody hears it then.
                    drop(sender.send(heard));
                });
                let link = Link::start(prepared.login, prepared.open, LinkSettings::default(), listen);
                self.links.with(|links| links.lock().expect("lock poisoned").insert(account, Arc::new(link)));
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
                let mut saved: Vec<Saved> =
                    db.setting(ACCOUNTS_SETTING)?.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
                saved.retain(|kept| kept.account() != wanted);
                db.set_setting(ACCOUNTS_SETTING, &serde_json::to_string(&saved).unwrap_or_default())?;
                Ok(env.map_or(Ok(()), |env| env.clear_secret(&secret_name(&wanted))))
            },
            move |result| {
                if let Err(error) = result.and_then(|cleared| cleared) {
                    self.state.update(|state| state.error = Some(error));
                }
            },
        );
    }

    /// Keeps `account` in the settings and its password in the credential store, after its first login.
    fn keep(self, account: &AccountRef, password: Password) {
        let Some(live) = self.state.with_untracked(|state| state.live(account).cloned()) else { return };
        let env = self.state.with_untracked(|state| state.env.clone());
        let kept = Saved { login: live.account.login, server: live.account.server, terminal: live.terminal };
        let wanted = account.clone();
        self.write.run(
            move |db: &mut Db| {
                let mut saved: Vec<Saved> =
                    db.setting(ACCOUNTS_SETTING)?.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
                saved.retain(|other| other.account() != kept.account());
                saved.push(kept);
                db.set_setting(ACCOUNTS_SETTING, &serde_json::to_string(&saved).unwrap_or_default())?;
                Ok(env.map_or(Ok(()), |env| env.save_secret(&secret_name(&wanted), password.expose())))
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
        let heard: Vec<Heard> = self.receiver.with(|receiver| receiver.try_iter().collect());
        for heard in heard {
            match heard {
                Heard::State(account, state) => self.on_state(&account, state),
                Heard::Polled(polled) => self.on_poll(polled),
            }
        }
    }

    fn on_state(self, account: &AccountRef, state: LinkState) {
        let Some(live) = self.state.with_untracked(|s| s.live(account).cloned()) else { return };
        match state {
            LinkState::Connecting => self.set_status(account, Status::Connecting),
            LinkState::Connected(info) => {
                self.state.update(|s| {
                    if let Some(live) = s.live_mut(account) {
                        live.status = Status::Connected;
                        live.info = Some(info);
                    }
                });
                if let Some(password) = self.pending_password(account) {
                    self.state.update(|s| {
                        if let Some(live) = s.live_mut(account) {
                            live.saved = true;
                        }
                    });
                    self.keep(account, password);
                }
            }
            LinkState::Retrying(error) => self.set_status(account, Status::Retrying(describe(&error))),
            LinkState::Refused(error) => {
                drop(self.pending_password(account));
                if live.saved {
                    // An account kept before stays listed (with the password it had), saying why.
                    self.set_status(account, Status::Refused(describe(&error)));
                    self.state.update(|s| s.error = Some(describe(&error)));
                } else {
                    // A first sign-in that failed: nothing is kept, the form says why.
                    self.stop_link(account);
                    self.state.update(|s| {
                        s.accounts.retain(|live| live.account != *account);
                        s.error = Some(describe(&error));
                    });
                }
            }
            LinkState::Stopped => {
                if live.removing {
                    self.state.update(|s| s.accounts.retain(|live| live.account != *account));
                    self.remove_copy(account);
                } else if self.link(account).is_none() {
                    self.set_status(account, Status::Stopped);
                }
            }
        }
    }

    /// The password a link was started with, for keeping it once the login succeeded.
    fn pending_password(self, account: &AccountRef) -> Option<Password> {
        self.pending.with(|pending| pending.remove(account))
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

    fn on_poll(self, polled: Polled) {
        let Polled { account, account_id, info, positions, first } = polled;
        let account = &account;
        let Some(live) = self.state.with_untracked(|s| s.live(account).cloned()) else { return };
        let known: HashSet<i64> = live.positions.iter().map(|p| p.ticket).collect();
        let opened: Vec<Position> =
            if first { Vec::new() } else { positions.iter().filter(|p| !known.contains(&p.ticket)).cloned().collect() };
        let now = chrono::Utc::now().timestamp_millis();
        self.state.update(|s| {
            if let Some(live) = s.live_mut(account) {
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

    /// Installs the `MetaTrader5` package into the Python found, then looks at the machine again.
    pub(crate) fn install_package(self) {
        let Some(python) = self.state.with_untracked(|state| state.setup.python.clone()) else { return };
        self.state.update(|state| {
            state.busy = Some("Đang cài gói MetaTrader5 (pip)…");
            state.error = None;
        });
        self.owner.run(move || {
            spawn(async move {
                let installed = blocking(move || python::install_package(&python))
                    .await
                    .unwrap_or_else(|error| Err(Mt5Error::Setup(format!("Luồng cài gói bị dừng: {error}"))));
                self.state.update(|state| {
                    state.busy = None;
                    state.error = installed.err().map(|error| describe(&error));
                });
                self.check_again();
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
        let Some(link) = self.link(account) else {
            return done(Err("Tài khoản MT5 này chưa kết nối.".to_owned()));
        };
        self.owner.run(move || {
            spawn(async move {
                let result = blocking(move || ask(&link))
                    .await
                    .unwrap_or_else(|error| Err(Mt5Error::Unreachable(format!("luồng gửi lệnh bị dừng: {error}"))));
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
            assert!(setup.missing().is_some_and(|why| why.contains("Python 3")));
            setup.python =
                Some(Python { program: "python".into(), args: Vec::new(), version: "3.12".into(), has_package: false });
            assert!(setup.missing().is_some_and(|why| why.contains("MetaTrader5")));
            setup.python.as_mut().expect("set above").has_package = true;
            assert_eq!(setup.missing(), None);
        } else {
            assert!(setup.missing().is_some_and(|why| why.contains("Windows")));
        }
        assert_eq!(Setup { demo: true, ..Setup::default() }.missing(), None, "the demo needs nothing");
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
