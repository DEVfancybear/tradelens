//! Sign-in and sync, from the app's side: what is read from the settings, the work that runs on
//! blocking threads (sign in, sync, restore), and the state the screen and the sidebar show.
//!
//! Nothing here touches a signal off the UI thread. Each action `spawn`s a task that waits for a
//! `blocking` call (its own `Db` connection, the network) and then sets the state on the UI thread;
//! a sync that stored something then calls [`crate::data_changed`], so the pages read again.
//!
//! Design: `docs/cloud-sync.md`. The demo mode (`TRADELENS_CLOUD=memory`, or the setting
//! `cloud.mode = memory`) keeps the "drive" in memory so the screen can be tried without an account.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use nsh_core::{
    element::desktop::{current_window, quit},
    prelude::*,
};
use tradelens_cloud::{
    CloudError, Provider,
    auth::{ClientConfig, Identity, SignInOptions},
    drive::CloudDrive,
    google::GoogleDrive,
    memory::MemoryDrive,
    notion::NotionClient,
    onedrive::OneDrive,
    sync::{DeviceInfo, SyncEngine, SyncReport},
    token_store::{KeyringStore, MemoryStore, TokenSource, TokenStore},
};
use tradelens_core::{Db, DbError};

use crate::{
    store::{Store, Writer, data_changed, store, writer},
    ui::controls::confirm_then,
};

/// `memory` selects the demo: the drive is in memory and signing in needs no account.
pub(crate) const MODE_SETTING: &str = "cloud.mode";
/// The name of the in-memory drive the demo uses; devices with the same name share it.
pub(crate) const DEMO_DRIVE_SETTING: &str = "cloud.demo_drive";
/// Which provider the demo is signed in to.
const DEMO_USER_SETTING: &str = "cloud.demo_user";
/// The provider that sync uses (`google` / `microsoft`), the one signed in last.
const ACTIVE_SETTING: &str = "cloud.provider";
/// `1` when the app syncs by itself.
pub(crate) const AUTO_SYNC_SETTING: &str = "cloud.auto_sync";
/// What this device is called on the other devices' lists.
const DEVICE_NAME_SETTING: &str = "cloud.device_name";
/// Another address for the Notion API (a test server); the variable `TRADELENS_NOTION_URL` does the same.
const NOTION_URL_SETTING: &str = "cloud.notion.base_url";
/// `memory` keeps sessions and secrets in memory instead of the operating system's credential store,
/// so the screen tests neither see nor change the sign-ins of whoever runs them.
pub const CREDENTIALS_SETTING: &str = "cloud.credentials";

/// The demo's drive name when the setting names none.
const DEFAULT_DEMO_DRIVE: &str = "mặc định";

/// How often the app syncs by itself when it is switched on.
pub(crate) const AUTO_SYNC_EVERY: Duration = Duration::from_mins(15);

/// The only account of the demo.
const DEMO_EMAIL: &str = "demo@tradelens.local";

/// The file the data is copied to before a restore replaces it, next to the database.
const BACKUP_FILE: &str = "tradelens.before-restore.db";

/// The settings the screen needs, read once per refresh so that background threads never ask
/// the database for them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Env {
    settings: BTreeMap<String, String>,
}

impl Env {
    fn keys() -> Vec<String> {
        let mut keys: Vec<String> = [
            MODE_SETTING,
            DEMO_DRIVE_SETTING,
            DEMO_USER_SETTING,
            ACTIVE_SETTING,
            AUTO_SYNC_SETTING,
            DEVICE_NAME_SETTING,
            NOTION_URL_SETTING,
            CREDENTIALS_SETTING,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for provider in Provider::ALL {
            keys.push(provider.client_id_setting());
            keys.push(provider.client_secret_setting());
            keys.push(format!("cloud.{}.redirect_host", provider.key()));
        }
        keys
    }

    pub(crate) fn read(db: &Db) -> Result<Self, DbError> {
        let mut settings = BTreeMap::new();
        for key in Self::keys() {
            if let Some(value) = db.setting(&key)? {
                settings.insert(key, value);
            }
        }
        Ok(Self { settings })
    }

    fn get(&self, key: &str) -> Option<String> {
        self.settings.get(key).filter(|value| !value.trim().is_empty()).cloned()
    }

    /// The name of the demo's drive when the demo is on.
    fn demo_drive(&self) -> Option<String> {
        let on = self.get(MODE_SETTING).as_deref() == Some("memory")
            || std::env::var("TRADELENS_CLOUD").is_ok_and(|v| v.trim().eq_ignore_ascii_case("memory"));
        on.then(|| self.get(DEMO_DRIVE_SETTING).unwrap_or_else(|| DEFAULT_DEMO_DRIVE.to_owned()))
    }

    pub(crate) fn is_demo(&self) -> bool {
        self.demo_drive().is_some()
    }

    fn active(&self) -> Option<Provider> {
        let key = self.get(ACTIVE_SETTING)?;
        Provider::ALL.into_iter().find(|p| p.key() == key)
    }

    pub(crate) fn auto_sync(&self) -> bool {
        self.get(AUTO_SYNC_SETTING).as_deref() == Some("1")
    }

    fn device_name(&self) -> String {
        self.get(DEVICE_NAME_SETTING)
            .or_else(|| std::env::var("COMPUTERNAME").ok())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Máy này".to_owned())
    }

    /// What is missing to sign in with `provider`, in Vietnamese; `None` when it can be done (the
    /// demo needs nothing).
    pub(crate) fn missing_client(&self, provider: Provider) -> Option<String> {
        if self.is_demo() {
            return None;
        }
        match ClientConfig::resolve(provider, |key| self.get(key)) {
            Ok(_) => None,
            Err(error) => Some(describe(&error)),
        }
    }
}

// --- secrets and the Notion API -------------------------------------------------------------------------

/// The secrets of the demo (a Notion token), by drive name and secret name: kept in memory, so that
/// trying the app never writes to the operating system's credential store.
fn demo_secrets() -> &'static Mutex<HashMap<String, String>> {
    static SECRETS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    SECRETS.get_or_init(Mutex::default)
}

/// The sessions kept in memory when [`CREDENTIALS_SETTING`] says so, shared by the whole run.
fn memory_tokens() -> Arc<MemoryStore> {
    static TOKENS: OnceLock<Arc<MemoryStore>> = OnceLock::new();
    Arc::clone(TOKENS.get_or_init(Arc::default))
}

impl Env {
    /// Whether the credentials stay in memory: in the demo, or when the setting asks for it.
    fn credentials_in_memory(&self) -> bool {
        self.is_demo() || self.get(CREDENTIALS_SETTING).as_deref() == Some("memory")
    }

    /// Where the providers' sessions are kept.
    fn token_store(&self) -> Arc<dyn TokenStore> {
        if self.credentials_in_memory() { memory_tokens() } else { Arc::new(KeyringStore::new()) }
    }

    /// The key of a secret kept in memory, `None` when it goes to the operating system's store.
    fn secret_key(&self, name: &str) -> Option<String> {
        match self.demo_drive() {
            Some(drive) => Some(format!("{drive}/{name}")),
            None => self.credentials_in_memory().then(|| format!("{CREDENTIALS_SETTING}/{name}")),
        }
    }

    /// The secret `name` (a Notion token), `None` when there is none. Blocks: call it off the UI thread.
    ///
    /// # Errors
    ///
    /// The credential store's failure, in Vietnamese.
    pub(crate) fn load_secret(&self, name: &str) -> Result<Option<String>, String> {
        match self.secret_key(name) {
            Some(key) => Ok(demo_secrets().lock().expect("lock poisoned").get(&key).cloned()),
            None => KeyringStore::new().load_secret(name).map_err(|e| describe(&e)),
        }
    }

    /// Keeps the secret `name` where the operating system keeps credentials. Blocks.
    ///
    /// # Errors
    ///
    /// The credential store's failure, in Vietnamese.
    pub(crate) fn save_secret(&self, name: &str, secret: &str) -> Result<(), String> {
        match self.secret_key(name) {
            Some(key) => {
                demo_secrets().lock().expect("lock poisoned").insert(key, secret.to_owned());
                Ok(())
            }
            None => KeyringStore::new().save_secret(name, secret).map_err(|e| describe(&e)),
        }
    }

    /// Forgets the secret `name`. Blocks.
    ///
    /// # Errors
    ///
    /// The credential store's failure, in Vietnamese.
    pub(crate) fn clear_secret(&self, name: &str) -> Result<(), String> {
        match self.secret_key(name) {
            Some(key) => {
                demo_secrets().lock().expect("lock poisoned").remove(&key);
                Ok(())
            }
            None => KeyringStore::new().clear_secret(name).map_err(|e| describe(&e)),
        }
    }

    /// A client of the Notion API for `token`, on the address a setting or `TRADELENS_NOTION_URL` gives
    /// when one is set (a test server).
    pub(crate) fn notion_client(&self, token: &str) -> NotionClient {
        let client = NotionClient::new(token);
        match self.get(NOTION_URL_SETTING).or_else(|| std::env::var("TRADELENS_NOTION_URL").ok()) {
            Some(url) if !url.trim().is_empty() => client.with_base_url(url.trim()),
            _ => client,
        }
    }
}

/// What a session that is gone says; sync tells it apart from other failures by this text, so both use the constant.
const SIGNED_OUT: &str = "Phiên đăng nhập đã hết hạn hoặc bị thu hồi: hãy đăng nhập lại.";

/// An error of the cloud crate, in words for the trader.
pub(crate) fn describe(error: &CloudError) -> String {
    match error {
        CloudError::Config(message) => {
            format!(
                "Chưa cấu hình đăng nhập: {message}. Mục «Client id đọc ở đâu» trong docs/cloud-sync.md chỉ cách đăng ký."
            )
        }
        CloudError::SignedOut => SIGNED_OUT.to_owned(),
        CloudError::NoSnapshot => {
            "Trên Drive chưa có bản sao lưu nào để khôi phục: hãy sync từ máy cũ trước.".to_owned()
        }
        other => other.to_string(),
    }
}

/// What the demo keeps for a drive name: the files, shared by every device that names it.
fn demo_world(name: &str) -> MemoryDrive {
    static WORLDS: OnceLock<Mutex<HashMap<String, MemoryDrive>>> = OnceLock::new();
    let worlds = WORLDS.get_or_init(Mutex::default);
    worlds.lock().expect("lock poisoned").entry(name.to_owned()).or_default().clone()
}

/// The sessions of the providers that are signed in, kept for as long as the app runs so that an
/// access token is refreshed once and not on every sync.
fn session(provider: Provider, env: &Env) -> Result<Arc<TokenSource>, CloudError> {
    static SOURCES: OnceLock<Mutex<HashMap<Provider, Arc<TokenSource>>>> = OnceLock::new();
    let sources = SOURCES.get_or_init(Mutex::default);
    let mut sources = sources.lock().expect("lock poisoned");
    if let Some(known) = sources.get(&provider) {
        return Ok(Arc::clone(known));
    }
    let client = ClientConfig::resolve(provider, |key| env.get(key))?;
    let source = Arc::new(TokenSource::new(client, env.token_store()));
    sources.insert(provider, Arc::clone(&source));
    Ok(source)
}

/// A drive to sync with, and the name the sync's bookkeeping is kept under.
struct Connection {
    drive: Box<dyn CloudDrive>,
    name: &'static str,
}

fn connect(provider: Provider, env: &Env) -> Result<Connection, CloudError> {
    if let Some(world) = env.demo_drive() {
        return Ok(Connection { drive: Box::new(demo_world(&world)), name: "demo" });
    }
    let source = session(provider, env)?;
    let drive: Box<dyn CloudDrive> = match provider {
        Provider::Google => Box::new(GoogleDrive::new(source)),
        Provider::Microsoft => Box::new(OneDrive::new(source)),
    };
    Ok(Connection { drive, name: provider.key() })
}

/// Who is signed in with each provider, read from where the sessions are kept.
fn identities(env: &Env) -> Vec<Identity> {
    if env.demo_drive().is_some() {
        return Provider::ALL
            .into_iter()
            .filter(|p| env.get(DEMO_USER_SETTING).as_deref() == Some(p.key()))
            .map(|provider| Identity {
                provider, email: DEMO_EMAIL.to_owned(), name: "Người dùng thử".to_owned()
            })
            .collect();
    }
    let store = env.token_store();
    Provider::ALL
        .into_iter()
        .filter_map(|provider| match store.load_identity(provider) {
            Ok(found) => found,
            Err(error) => {
                log::warn!("cloud: không đọc được phiên đăng nhập của {}: {error}", provider.key());
                None
            }
        })
        .collect()
}

/// Opens `url` in the system's browser. The sign-in asks for it from a blocking thread, where the
/// UI's own `open_url` cannot be used.
fn open_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let started = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let started = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let started = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = started {
        log::warn!("cloud: không mở được trình duyệt: {error}");
    }
}

/// The automatic sync, on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoSync {
    On,
    Off,
}

/// What finished: a sync, or a restore (which also syncs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sync,
    Restore,
}

/// Why a sync is run: a button, the timer, or the window closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    Pressed,
    Timer,
    Closing,
}

/// What the app is busy with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Phase {
    #[default]
    Idle,
    SigningIn(Provider),
    Syncing,
    Restoring,
}

/// How the last sync went.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    /// Unix milliseconds.
    pub(crate) at: i64,
    pub(crate) result: Result<SyncReport, String>,
    /// Set for a restore.
    pub(crate) restored: bool,
}

/// Everything the screen and the sidebar show.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct State {
    pub(crate) env: Env,
    /// Whether the settings and sessions were read yet.
    pub(crate) ready: bool,
    pub(crate) accounts: Vec<Identity>,
    pub(crate) phase: Phase,
    pub(crate) last: Option<Outcome>,
    /// Unix milliseconds of the last sync that finished, from the database.
    pub(crate) last_sync: Option<i64>,
    pub(crate) devices: Vec<DeviceInfo>,
    /// The id this device's changes are filed under, to mark it in the list of devices.
    pub(crate) device_id: String,
    /// The last thing that went wrong when signing in or out, in Vietnamese.
    pub(crate) notice: Option<String>,
}

impl State {
    /// The provider sync goes to: the active one when it is signed in, else the one signed in.
    pub(crate) fn active(&self) -> Option<Provider> {
        let signed = |p: Provider| self.accounts.iter().any(|a| a.provider == p);
        self.env.active().filter(|p| signed(*p)).or_else(|| self.accounts.first().map(|a| a.provider))
    }

    pub(crate) fn identity_of(&self, provider: Provider) -> Option<&Identity> {
        self.accounts.iter().find(|a| a.provider == provider)
    }

    pub(crate) fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }
}

/// The handle to the cloud, made once by [`provide`]: a `Copy` bundle of what the actions need.
#[derive(Clone, Copy)]
pub(crate) struct Cloud {
    pub(crate) state: Signal<State>,
    /// The owner everything the cloud starts (a task, a write) lives under: the root's, so that a button
    /// rebuilt when the state changes, or a page that is left, does not cancel a sync that is under way.
    owner: Owner,
    store: Store,
    write: Writer,
    /// The flag that stops a sign-in that is waiting for the browser.
    cancel: Stored<Option<Arc<AtomicBool>>>,
}

impl std::fmt::Debug for Cloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cloud").finish_non_exhaustive()
    }
}

/// Makes the cloud handle for everything under the caller, reads the settings and starts the
/// timer of the automatic sync and the sync when the window closes. Call it once, in the root.
pub(crate) fn provide() {
    let cloud = Cloud {
        state: signal(State::default()),
        owner: Owner::current(),
        store: store(),
        write: writer(),
        cancel: stored(None),
    };
    provide_context(cloud);
    // The settings can be read once the database is open.
    effect(move || {
        if cloud.store.is_ready() {
            untracked(|| cloud.refresh());
        }
    });
    set_interval(AUTO_SYNC_EVERY, move || {
        let state = cloud.state.get_untracked();
        if state.ready && state.env.auto_sync() && state.active().is_some() {
            cloud.sync(Why::Timer);
        }
    });
    cloud.sync_on_close();
}

/// The cloud handle; read while building, never in a listener.
pub(crate) fn cloud() -> Cloud {
    use_context::<Cloud>().expect("the root provides the cloud")
}

impl Cloud {
    /// Reads the settings, the sessions and the progress of the sync again.
    pub(crate) fn refresh(self) {
        self.owner.run(move || self.refresh_now());
    }

    fn refresh_now(self) {
        self.store.read(
            |db| {
                let env = Env::read(db)?;
                let device_id = db.device_id()?;
                let provider = env.active().unwrap_or(Provider::Google);
                let name = if env.is_demo() { "demo" } else { provider.key() };
                let last_sync = db.sync_state(name)?.last_sync;
                Ok((env, device_id, last_sync))
            },
            move |read| match read {
                Ok((env, device_id, last_sync)) => {
                    let env_for_threads = env.clone();
                    spawn(async move {
                        let accounts = blocking(move || identities(&env_for_threads)).await.unwrap_or_default();
                        self.state.update(|s| {
                            s.env = env;
                            s.accounts = accounts;
                            s.device_id = device_id;
                            s.last_sync = last_sync;
                            s.ready = true;
                        });
                    });
                }
                Err(message) => self.state.update(|s| s.notice = Some(message)),
            },
        );
    }

    /// Signs in with `provider`: in the browser, or at once in the demo. The state follows.
    pub(crate) fn sign_in(self, provider: Provider) {
        self.owner.run(move || self.sign_in_now(provider));
    }

    fn sign_in_now(self, provider: Provider) {
        let state = self.state.get_untracked();
        if state.busy() {
            return;
        }
        if let Some(missing) = state.env.missing_client(provider) {
            return self.state.update(|s| s.notice = Some(missing));
        }
        let env = state.env;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel.replace(Some(Arc::clone(&cancel)));
        self.state.update(|s| {
            s.phase = Phase::SigningIn(provider);
            s.notice = None;
        });
        spawn(async move {
            let attempt = blocking(move || -> Result<(), String> {
                if env.is_demo() {
                    return Ok(());
                }
                let source = session(provider, &env).map_err(|e| describe(&e))?;
                let options = SignInOptions { cancel, ..SignInOptions::default() };
                source.sign_in(&options, open_in_browser).map(drop).map_err(|e| describe(&e))
            })
            .await
            .unwrap_or_else(|error| Err(format!("Luồng đăng nhập bị dừng: {error}")));
            self.cancel.replace(None);
            match attempt {
                Ok(()) => {
                    self.state.update(|s| s.phase = Phase::Idle);
                    self.remember_signed_in(provider);
                }
                Err(message) if message == CloudError::Cancelled.to_string() => {
                    self.state.update(|s| s.phase = Phase::Idle);
                }
                Err(message) => self.state.update(|s| {
                    s.phase = Phase::Idle;
                    s.notice = Some(message);
                }),
            }
        });
    }

    /// Notes which provider sync goes to now, then reads the sessions again.
    fn remember_signed_in(self, provider: Provider) {
        let demo = self.state.get_untracked().env.is_demo();
        self.write.run(
            move |db| {
                db.set_setting(ACTIVE_SETTING, provider.key())?;
                // Another account may be the one signed in now: what the drive holds is asked again.
                db.forget_uploaded(provider.key())?;
                if demo {
                    db.set_setting(DEMO_USER_SETTING, provider.key())?;
                }
                Ok(())
            },
            move |_| self.refresh(),
        );
    }

    /// Stops a sign-in that is waiting for the browser.
    pub(crate) fn cancel_sign_in(self) {
        if let Some(flag) = self.cancel.get() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Signs out of `provider`; what is on the drive stays.
    pub(crate) fn sign_out(self, provider: Provider) {
        self.owner.run(move || self.sign_out_now(provider));
    }

    fn sign_out_now(self, provider: Provider) {
        let env = self.state.get_untracked().env;
        spawn(async move {
            let demo = env.is_demo();
            let result = blocking(move || -> Result<(), String> {
                if demo {
                    return Ok(());
                }
                match session(provider, &env) {
                    Ok(source) => source.sign_out().map_err(|e| describe(&e)),
                    // Nothing to revoke without a registration: forget what is stored.
                    Err(_) => env.token_store().clear(provider).map_err(|e| describe(&e)),
                }
            })
            .await
            .unwrap_or_else(|error| Err(format!("Luồng đăng xuất bị dừng: {error}")));
            match result {
                Ok(()) => self.write.run(
                    move |db| {
                        // The next account to sign in may not be the one whose drive these images went to.
                        db.forget_uploaded(provider.key())?;
                        if demo {
                            db.set_setting(DEMO_USER_SETTING, "")?;
                            db.forget_uploaded("demo")?;
                        }
                        Ok(())
                    },
                    move |_| self.refresh(),
                ),
                Err(message) => self.state.update(|s| s.notice = Some(message)),
            }
        });
    }

    /// Turns the automatic sync on or off; kept on this device only.
    pub(crate) fn set_auto_sync(self, to: AutoSync) {
        self.owner.run(move || self.set_auto_sync_now(to));
    }

    fn set_auto_sync_now(self, to: AutoSync) {
        self.write.run(
            move |db| db.set_setting(AUTO_SYNC_SETTING, if to == AutoSync::On { "1" } else { "0" }),
            move |_| self.refresh(),
        );
    }

    /// Syncs now: uploads this device's changes, takes the others', and tells the pages to read again.
    pub(crate) fn sync(self, why: Why) {
        self.sync_then(why, || {});
    }

    fn sync_then(self, why: Why, then: impl FnOnce() + 'static) {
        self.owner.run(move || self.sync_then_now(why, then));
    }

    fn sync_then_now(self, why: Why, then: impl FnOnce() + 'static) {
        let state = self.state.get_untracked();
        let (Some(provider), Some(path), false) = (state.active(), Store::database_path(), state.busy()) else {
            if why == Why::Pressed && !state.busy() {
                let notice = if state.active().is_none() {
                    "Chưa đăng nhập: bấm Đăng nhập với Google hoặc Microsoft trước."
                } else {
                    "Ứng dụng không có thư mục dữ liệu nên chưa sync được."
                };
                self.state.update(|s| s.notice = Some(notice.to_owned()));
            }
            return then();
        };
        let env = state.env;
        self.state.update(|s| {
            s.phase = Phase::Syncing;
            s.notice = None;
        });
        spawn(async move {
            let done = blocking(move || run_sync(&env, provider, &path)).await;
            let result = done.unwrap_or_else(|error| Err(format!("Luồng sync bị dừng: {error}")));
            self.finish(result, Kind::Sync);
            then();
        });
    }

    /// Records how a sync or restore went and wakes the pages.
    fn finish(self, result: Result<Synced, String>, kind: Kind) {
        let restored = kind == Kind::Restore;
        let at = Utc::now().timestamp_millis();
        match result {
            Ok(done) => {
                self.state.update(|s| {
                    s.phase = Phase::Idle;
                    s.last = Some(Outcome { at, result: Ok(done.report), restored });
                    s.last_sync = Some(at);
                    s.devices = done.devices;
                    s.device_id = done.device_id;
                });
                data_changed();
            }
            Err(message) => {
                let signed_out = message == SIGNED_OUT;
                self.state.update(|s| {
                    s.phase = Phase::Idle;
                    s.last = Some(Outcome { at, result: Err(message), restored });
                });
                if signed_out {
                    self.refresh();
                }
            }
        }
    }

    /// Asks whether to sign out of `provider` (called `name` on the screen) and signs out when the
    /// trader says yes. The question is a task of the root, not of the button: the provider's card is
    /// built again whenever the cloud state changes, which would cancel a task of the button before
    /// the alert shows.
    pub(crate) fn confirm_sign_out(self, provider: Provider, name: &'static str) {
        confirm_then(
            self.owner,
            format!("Đăng xuất khỏi {name}?"),
            "Dữ liệu trên Drive vẫn còn; TradeLens chỉ ngừng sync trên máy này cho tới khi bạn đăng nhập lại.",
            "Đăng xuất",
            move || self.sign_out(provider),
        );
    }

    /// Asks whether to replace this device's data with the drive's, and restores when the trader says
    /// yes. A task of the root, like [`Cloud::confirm_sign_out`].
    pub(crate) fn confirm_restore(self) {
        confirm_then(
            self.owner,
            "Thay toàn bộ dữ liệu trên máy này?",
            "Dữ liệu đang có trên máy này sẽ bị thay bằng bản sao lưu gần nhất trên Drive, cộng các thay đổi sau đó. \
             Một bản của dữ liệu cũ được giữ cạnh file dữ liệu (tradelens.before-restore.db). \
             Chỉ làm khi đây là máy mới. Không hoàn tác được.",
            "Thay dữ liệu",
            move || self.restore(),
        );
    }

    /// Replaces this device's data with the newest snapshot on the drive and syncs. The old data
    /// is copied next to the database first.
    pub(crate) fn restore(self) {
        self.owner.run(move || self.restore_now());
    }

    fn restore_now(self) {
        let state = self.state.get_untracked();
        let (Some(provider), Some(path), false) = (state.active(), Store::database_path(), state.busy()) else {
            self.state.update(|s| {
                s.notice = Some("Cần đăng nhập (và có thư mục dữ liệu) trước khi khôi phục.".to_owned());
            });
            return;
        };
        let env = state.env;
        self.state.update(|s| {
            s.phase = Phase::Restoring;
            s.notice = None;
        });
        let backup = path.with_file_name(BACKUP_FILE);
        // A backup of an earlier restore is replaced.
        drop(std::fs::remove_file(&backup));
        let attachments = attachments_folder(&path);
        self.store.replace_database(
            backup,
            move |target| {
                // The device-local settings (which drive, which account) belong to this machine, not to
                // the snapshot: they are written back after it.
                let keep = local_settings(target);
                let connection = connect(provider, &env).map_err(|e| describe(&e))?;
                let engine = SyncEngine::new(connection.drive.as_ref(), connection.name, env.device_name())
                    .with_attachments(attachments);
                let (mut db, report) = engine.restore(target).map_err(|e| describe(&e))?;
                for (key, value) in keep {
                    db.set_setting(&key, &value).map_err(|e| format!("Không ghi lại được cài đặt: {e}"))?;
                }
                let devices = engine.devices().map_err(|e| describe(&e))?;
                let device_id = db.device_id().map_err(|e| e.to_string())?;
                Ok((db, Synced { report, devices, device_id }))
            },
            move |result| self.finish(result, Kind::Restore),
        );
    }

    /// When the window is asked to close, syncs first if the automatic sync is on, then quits the
    /// app (a window's `close` no longer ends it). A second request while the sync runs quits at once. NSH waits only 200 ms for the work done
    /// as the app quits (`on_app_quit`), far too little for the network, so the close button is
    /// held back instead (`close_on_press: false`) until the sync is over.
    fn sync_on_close(self) {
        let window = current_window();
        let mut attributes = nsh_core::desktop::WindowAttributes::default();
        attributes.close_on_press = Some(false);
        window.set(attributes);
        window.on_close_press(move |_| {
            let state = self.state.get_untracked();
            if state.busy() || !(state.ready && state.env.auto_sync() && state.active().is_some()) {
                return quit();
            }
            self.sync_then(Why::Closing, quit);
        });
    }
}

/// The folder the journal's images are kept in, next to the database.
fn attachments_folder(database: &Path) -> PathBuf {
    database.with_file_name("attachments")
}

/// The settings that belong to this device, read from the database about to be replaced.
fn local_settings(database: &Path) -> Vec<(String, String)> {
    let Ok(db) = Db::open(database) else { return Vec::new() };
    let mut keys = Env::keys();
    keys.push("appearance".to_owned());
    // The MT5 accounts run in terminals of this machine: they stay with it, not with the restored data.
    keys.extend(crate::mt5::LOCAL_SETTINGS.iter().map(|key| (*key).to_owned()));
    keys.into_iter().filter_map(|key| db.setting(&key).ok().flatten().map(|value| (key, value))).collect()
}

/// What a sync or a restore did, for the screen.
#[derive(Debug)]
struct Synced {
    report: SyncReport,
    devices: Vec<DeviceInfo>,
    device_id: String,
}

fn run_sync(env: &Env, provider: Provider, database: &Path) -> Result<Synced, String> {
    let connection = connect(provider, env).map_err(|e| describe(&e))?;
    let engine = SyncEngine::new(connection.drive.as_ref(), connection.name, env.device_name())
        .with_attachments(attachments_folder(database));
    let mut db = Db::open(database).map_err(|e| format!("Không mở được dữ liệu để sync: {e}"))?;
    let report = engine.sync(&mut db).map_err(|e| describe(&e))?;
    let devices = engine.devices().map_err(|e| describe(&e))?;
    let device_id = db.device_id().map_err(|e| e.to_string())?;
    Ok(Synced { report, devices, device_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env { settings: pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect() }
    }

    #[test]
    fn the_demo_is_chosen_by_the_setting_and_named_by_another() {
        assert!(!env(&[]).is_demo() || std::env::var("TRADELENS_CLOUD").is_ok());
        let demo = env(&[(MODE_SETTING, "memory"), (DEMO_DRIVE_SETTING, "một")]);
        assert_eq!(demo.demo_drive().as_deref(), Some("một"));
        assert_eq!(demo.missing_client(Provider::Google), None, "the demo needs no registration");
    }

    #[test]
    fn the_mt5_accounts_of_this_machine_are_kept_through_a_restore() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("tradelens.db");
        let mut db = Db::open(&path).expect("the database opens");
        db.set_setting(crate::mt5::ACCOUNTS_SETTING, r#"[{"login":"1","server":"S"}]"#).expect("stored");
        db.set_setting("mt5.plan_prompt", "0").expect("stored");
        drop(db);
        let kept = local_settings(&path);
        for key in [crate::mt5::ACCOUNTS_SETTING, "mt5.plan_prompt"] {
            assert!(kept.iter().any(|(k, _)| k == key), "{key} stays with this machine: {kept:?}");
        }
    }

    #[test]
    fn credentials_kept_in_memory_never_reach_the_operating_system_store() {
        let env = env(&[(CREDENTIALS_SETTING, "memory")]);
        let identity =
            Identity { provider: Provider::Microsoft, email: "thu@example.com".into(), name: "Thử".into() };
        env.token_store().save_identity(&identity).expect("kept in memory");
        assert_eq!(identities(&env), vec![identity], "the session is read back from memory");
        env.save_secret("thu-nghiem", "bí mật").expect("kept in memory");
        assert_eq!(env.load_secret("thu-nghiem").expect("read"), Some("bí mật".to_owned()));
        let in_os_store = KeyringStore::new().load_secret("thu-nghiem");
        if cfg!(target_os = "linux") {
            // A CI runner has no Secret Service, so there is no store at all: that holds nothing
            // either. Only an entry found there means the secret leaked.
            assert!(matches!(in_os_store, Err(_) | Ok(None)), "the secret did not go to the operating system's store");
        } else {
            // Windows and macOS have a store: it must answer, and have nothing.
            assert_eq!(
                in_os_store.expect("the OS store is read"),
                None,
                "the secret did not go to the operating system's store"
            );
        }
        env.clear_secret("thu-nghiem").expect("forgotten");
        env.token_store().clear(Provider::Microsoft).expect("forgotten");
        assert_eq!(identities(&env), Vec::new(), "signed out");
    }

    #[test]
    fn a_missing_client_id_is_explained_with_the_setting_the_variable_and_the_doc() {
        let message = env(&[]).missing_client(Provider::Google);
        if option_env!("TRADELENS_GOOGLE_CLIENT_ID").is_none() && std::env::var("TRADELENS_GOOGLE_CLIENT_ID").is_err() {
            let message = message.expect("no client id is configured");
            assert!(message.contains("cloud.google.client_id"), "{message}");
            assert!(message.contains("TRADELENS_GOOGLE_CLIENT_ID"), "{message}");
            assert!(message.contains("docs/cloud-sync.md"), "{message}");
        }
        let configured = env(&[("cloud.microsoft.client_id", "abc")]);
        assert_eq!(configured.missing_client(Provider::Microsoft), None);
    }

    #[test]
    fn the_active_provider_must_be_signed_in() {
        let me = Identity { provider: Provider::Microsoft, email: "a@b.c".into(), name: String::new() };
        let mut state = State { env: env(&[(ACTIVE_SETTING, "google")]), accounts: vec![me], ..State::default() };
        assert_eq!(state.active(), Some(Provider::Microsoft), "google is chosen but only microsoft is signed in");
        state.accounts.clear();
        assert_eq!(state.active(), None);
    }

    #[test]
    fn two_demo_devices_with_one_drive_name_share_the_files() {
        let one = demo_world("thử nghiệm chung");
        let two = demo_world("thử nghiệm chung");
        let other = demo_world("khác");
        one.upload("x", b"1").expect("uploads");
        assert_eq!(two.names(), ["x"]);
        assert_eq!(other.names(), Vec::<String>::new(), "another name is another drive");
    }
}
