//! The app updating itself, from the app's side: the settings it reads, the check and the download
//! that run on blocking threads, the state the settings page shows, and the swap as the app quits.
//!
//! The rules are NSH's (`nsh-updater`, through `tradelens-update`): a packaged app (`nsh package`)
//! reads where its releases are from the `nsh-updater.json` it ships, checks the signed `latest.json`
//! there, downloads and checks the archive for this machine, unpacks it beside itself and swaps it
//! in as it quits, so the next start runs the new version. A run from the code (`cargo run`, the
//! tests, any debug build) has no installation to replace: it checks GitHub Releases (or the source
//! the settings name) and sends the trader to the release page.
//!
//! Nothing here touches a signal off the UI thread: each step `spawn`s a task that waits for a
//! `blocking` call and then sets the state, under the root's owner so that a rebuilt card cannot
//! cancel it.

use std::{path::PathBuf, time::Duration};

use chrono::{Local, TimeZone, Utc};
use nsh_core::{element::desktop::on_app_quit, prelude::*};
use tradelens_core::{Db, DbError};
use tradelens_update::{ManifestError, Origin, Source, Staged, UpdateError};

use crate::store::{Store, Writer, store, writer};

/// `0` when the app does not look for new versions by itself; anything else, or nothing, means on.
pub(crate) const AUTO_SETTING: &str = "update.auto";
/// Another manifest for a run from the code (a test server): its URL. The variable
/// `TRADELENS_UPDATE_URL` does the same. A packaged app ignores it: its source is the package's.
const URL_SETTING: &str = "update.url";
/// The public key the manifest of [`URL_SETTING`] is signed with (`TRADELENS_UPDATE_KEY`).
const KEY_SETTING: &str = "update.public_key";

/// How long after the app opens the first automatic check waits, so that it never slows the start.
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// How often the app looks again while it stays open.
const CHECK_EVERY: Duration = Duration::from_hours(6);

/// The version that runs.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The repository whose releases a run from the code checks when `nsh.json` names none.
const DEFAULT_REPO: &str = "DEVfancybear/tradelens";

/// The folder downloads are kept in, inside the app's data folder (NSH keeps its own `updates`).
const DOWNLOADS: &str = "update-downloads";

/// The settings the updater needs, read once per refresh so that background threads never ask the
/// database for them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Settings {
    auto: Option<String>,
    url: Option<String>,
    public_key: Option<String>,
}

impl Settings {
    fn read(db: &Db) -> Result<Self, DbError> {
        Ok(Self {
            auto: db.setting(AUTO_SETTING)?,
            url: db.setting(URL_SETTING)?,
            public_key: db.setting(KEY_SETTING)?,
        })
    }

    /// Whether the app looks for and downloads new versions by itself (on unless turned off).
    pub(crate) fn auto(&self) -> bool {
        self.auto.as_deref().map(str::trim) != Some("0")
    }

    /// The manifest a run from the code checks instead of GitHub's, when one is named.
    fn url(&self) -> Option<String> {
        filled(self.url.clone()).or_else(|| filled(std::env::var("TRADELENS_UPDATE_URL").ok()))
    }

    fn public_key(&self) -> Option<String> {
        filled(self.public_key.clone()).or_else(|| filled(std::env::var("TRADELENS_UPDATE_KEY").ok()))
    }
}

fn filled(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The automatic updates, on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Auto {
    On,
    Off,
}

/// What runs: a package that can replace itself, one in a folder it cannot write, or the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Install {
    /// Not looked at yet.
    #[default]
    Unknown,
    Packaged,
    ReadOnly,
    FromSource,
}

/// Why a newer version is only offered on its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manual {
    /// A run from the code: nothing to replace.
    FromSource,
    /// The package is in a folder the trader cannot write.
    ReadOnly,
    /// The release has no archive for this machine, only a page.
    Page,
}

/// Where the updater stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    /// Downloaded, checked and unpacked: it replaces the app when the app quits.
    Ready(String),
    /// A newer version the trader gets from its page.
    Available(String, Manual),
    /// What went wrong.
    Failed(Failure),
}

/// Which job of the updater a stopped working thread had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Job {
    Check,
    Download,
    Install,
}

/// What went wrong with an update. It is made on the working threads, which cannot read the app's
/// text; [`Failure::text`] makes the words on the UI thread, in the language chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No answer from the update server.
    Network(String),
    /// The server has no `latest.json`: nothing published yet.
    NoRelease,
    /// The server answered with an error.
    Status { status: u16, url: String },
    /// The manifest, its signature or its content was refused.
    Manifest(String),
    /// The downloaded file does not match the release.
    Download(String),
    /// The new version could not be installed.
    Install(String),
    /// The updater's settings are wrong.
    Config(String),
    /// The update's files could not be written.
    Files(String),
    /// The release has no archive this app can install itself.
    NotInstallable,
    /// A working thread stopped before it answered.
    Stopped(Job, String),
    /// Words that were already made (the store's, when the settings could not be read).
    Words(String),
}

impl From<&UpdateError> for Failure {
    fn from(error: &UpdateError) -> Self {
        match error {
            UpdateError::Network(detail) => Self::Network(detail.clone()),
            UpdateError::Status { status: 404, .. } => Self::NoRelease,
            UpdateError::Status { status, url } => Self::Status { status: *status, url: url.clone() },
            UpdateError::Refused(ManifestError::Manifest(detail)) => Self::Manifest(detail.clone()),
            UpdateError::Refused(ManifestError::Download(detail)) => Self::Download(detail.clone()),
            UpdateError::Refused(ManifestError::Install(detail)) => Self::Install(detail.clone()),
            UpdateError::Refused(ManifestError::Config(detail)) => Self::Config(detail.clone()),
            UpdateError::Refused(ManifestError::Io(detail)) | UpdateError::Io(detail) => {
                Self::Files(detail.to_string())
            }
            UpdateError::NotInstallable => Self::NotInstallable,
        }
    }
}

impl Failure {
    /// The words for the trader; call it on the UI thread.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Network(detail) => t!("update.failed.network", detail = detail),
            Self::NoRelease => t!("update.failed.no_release"),
            Self::Status { status, url } => t!("update.failed.status", status = status, url = url),
            Self::Manifest(detail) => t!("update.failed.manifest", detail = detail),
            Self::Download(detail) => t!("update.failed.download", detail = detail),
            Self::Install(detail) => t!("update.failed.install", detail = detail),
            Self::Config(detail) => t!("update.failed.config", detail = detail),
            Self::Files(detail) => t!("update.failed.files", detail = detail),
            Self::NotInstallable => t!("update.failed.not_installable"),
            Self::Words(words) => words.clone(),
            Self::Stopped(Job::Check, error) => t!("update.failed.check_stopped", error = error),
            Self::Stopped(Job::Download, error) => t!("update.failed.download_stopped", error = error),
            Self::Stopped(Job::Install, error) => t!("update.failed.install_stopped", error = error),
        }
    }
}

/// Everything the settings page shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct State {
    pub(crate) settings: Settings,
    /// Whether the settings were read yet.
    pub(crate) ready: bool,
    pub(crate) install: Install,
    pub(crate) phase: Phase,
    /// Unix milliseconds of the last check that finished.
    pub(crate) checked_at: Option<i64>,
    /// The page of the newest release found.
    pub(crate) page: Option<String>,
}

impl State {
    pub(crate) fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Downloading(_))
    }

    /// Whether the app checks by itself: switched on, and either packaged or pointed at a source by
    /// the settings. A run from the code never calls GitHub unless the trader presses the button.
    fn automatic(&self) -> bool {
        self.ready
            && self.settings.auto()
            && (matches!(self.install, Install::Packaged | Install::ReadOnly) || self.settings.url().is_some())
    }

    /// The line under the version on the settings page.
    pub(crate) fn status(&self) -> String {
        match &self.phase {
            Phase::Idle if self.automatic() => t!("update.status.auto"),
            Phase::Idle => t!("update.status.never"),
            Phase::Checking => t!("update.status.checking"),
            Phase::UpToDate => match self.checked_at.and_then(clock) {
                Some(at) => t!("update.status.latest_at", at = at),
                None => t!("update.status.latest"),
            },
            Phase::Downloading(version) => t!("update.status.downloading", version = version),
            Phase::Ready(version) => t!("update.status.ready", version = version),
            Phase::Available(version, Manual::FromSource) => t!("update.status.available_source", version = version),
            Phase::Available(version, Manual::ReadOnly) => t!("update.status.available_read_only", version = version),
            Phase::Available(version, Manual::Page) => t!("update.status.available_page", version = version),
            Phase::Failed(failure) => failure.text(),
        }
    }

    /// The page "Ghi chú phát hành" opens: the newest release found, else the list of releases.
    pub(crate) fn notes_page(&self) -> String {
        self.page.clone().unwrap_or_else(|| format!("{}/releases", repository_url()))
    }
}

/// `HH:MM` on this machine's clock.
fn clock(unix_ms: i64) -> Option<String> {
    Local.timestamp_millis_opt(unix_ms).single().map(|at| at.format("%H:%M").to_string())
}

/// The repository `nsh.json`'s `"updater"` names (`owner/repo`).
fn repository() -> String {
    shipped_updater()["github"].as_str().map_or_else(|| DEFAULT_REPO.to_owned(), str::to_owned)
}

/// The repository's web page.
pub(crate) fn repository_url() -> String {
    format!("https://github.com/{}", repository())
}

/// `nsh.json`'s `"updater"`, the source `nsh package` writes into the package.
fn shipped_updater() -> serde_json::Value {
    let manifest: serde_json::Value = serde_json::from_str(crate::MANIFEST).unwrap_or_default();
    manifest["updater"].clone()
}

/// Where updates come from. A packaged release build takes its package's source and nothing else;
/// a run from the code checks the manifest the settings name, else the GitHub Releases of `nsh.json`.
/// A debug build never takes the package's, so it never replaces an executable.
fn source(settings: &Settings) -> Result<Source, Failure> {
    if !cfg!(debug_assertions)
        && let Some(source) = Source::packaged()
    {
        return Ok(source);
    }
    let made = if let Some(url) = settings.url() {
        Source::new(Origin::Url(url), crate::APP_ID, VERSION, settings.public_key().as_deref())
    } else {
        let key = shipped_updater()["publicKey"].as_str().map(str::to_owned);
        Source::new(Origin::Github(repository()), crate::APP_ID, VERSION, key.as_deref())
    };
    made.map_err(|error| Failure::from(&error))
}

fn install_of(source: &Source) -> Install {
    match (source.is_packaged(), source.installable()) {
        (true, true) => Install::Packaged,
        (true, false) => Install::ReadOnly,
        (false, _) => Install::FromSource,
    }
}

/// What a check found, read on a blocking thread.
struct Found {
    source: Source,
    available: Option<tradelens_update::Available>,
}

/// The handle to the updater, made once by [`provide`].
#[derive(Clone, Copy)]
pub(crate) struct Updater {
    pub(crate) state: Signal<State>,
    /// The root's owner: what the updater starts outlives the card that asked for it.
    owner: Owner,
    store: Store,
    write: Writer,
    /// The app's data folder; downloads go to [`DOWNLOADS`] in it.
    data_dir: Stored<Option<PathBuf>>,
    /// The update unpacked beside the app, swapped in as the app quits.
    staged: Stored<Option<Staged>>,
    /// Whether the first automatic check is due ([`FIRST_CHECK`] has passed).
    due: Stored<bool>,
}

impl std::fmt::Debug for Updater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updater").finish_non_exhaustive()
    }
}

/// Makes the updater for everything under the caller, reads its settings, starts the automatic
/// checks and installs a downloaded update as the app quits. Call it once, in the root.
pub(crate) fn provide() {
    let updater = Updater {
        state: signal(State::default()),
        owner: Owner::current(),
        store: store(),
        write: writer(),
        data_dir: stored(data_dir()),
        staged: stored(None),
        due: stored(false),
    };
    provide_context(updater);
    effect(move || {
        if updater.store.is_ready() {
            untracked(|| updater.start());
        }
    });
    set_timeout(FIRST_CHECK, move || {
        updater.due.replace(true);
        updater.check_if_due();
    });
    set_interval(CHECK_EVERY, move || updater.check_if_due());
    on_app_quit(move || {
        updater.install_now();
        std::future::ready(())
    });
}

/// The updater handle; read while building, never in a listener.
pub(crate) fn updater() -> Updater {
    use_context::<Updater>().expect("the root provides the updater")
}

impl Updater {
    /// Reads the settings, finds out what runs, and clears what an earlier update left.
    fn start(self) {
        self.owner.run(move || {
            self.store.read(Settings::read, move |read| match read {
                Ok(settings) => {
                    let downloads = self.downloads();
                    let probe = settings.clone();
                    spawn(async move {
                        let install = blocking(move || {
                            let source = source(&probe).ok();
                            if let Some(source) = &source {
                                source.remove_leftovers();
                            }
                            // An update staged by an earlier run is gone with it: so is its archive.
                            if let Some(dir) = downloads {
                                drop(std::fs::remove_dir_all(dir));
                            }
                            source.as_ref().map_or(Install::FromSource, install_of)
                        })
                        .await
                        .unwrap_or(Install::FromSource);
                        self.state.update(|s| {
                            s.settings = settings;
                            s.install = install;
                            s.ready = true;
                        });
                        self.check_if_due();
                    });
                }
                Err(message) => self.state.update(|s| s.phase = Phase::Failed(Failure::Words(message))),
            });
        });
    }

    /// Reads the settings again (after the switch was stored).
    fn reread(self) {
        self.owner.run(move || {
            self.store.read(Settings::read, move |read| {
                if let Ok(settings) = read {
                    self.state.update(|s| s.settings = settings);
                }
            });
        });
    }

    fn downloads(self) -> Option<PathBuf> {
        self.data_dir.get().map(|dir| dir.join(DOWNLOADS))
    }

    /// Checks by itself when it is time and the trader lets it.
    fn check_if_due(self) {
        let state = self.state.get_untracked();
        if self.due.get() && state.automatic() && !state.busy() && !matches!(state.phase, Phase::Ready(_)) {
            self.check();
        }
    }

    /// Turns the automatic updates on or off; kept on this device.
    pub(crate) fn set_auto(self, to: Auto) {
        self.owner.run(move || {
            let value = if to == Auto::On { "1" } else { "0" };
            self.state.update(|s| s.settings.auto = Some(value.to_owned()));
            self.write.run(move |db| db.set_setting(AUTO_SETTING, value), move |_| self.reread());
        });
    }

    /// Looks for a newer version now; when the app can install it, downloads it and readies it for
    /// the next start.
    pub(crate) fn check(self) {
        self.owner.run(move || self.check_now());
    }

    fn check_now(self) {
        let state = self.state.get_untracked();
        if state.busy() || !state.ready {
            return;
        }
        self.state.update(|s| s.phase = Phase::Checking);
        let settings = state.settings;
        spawn(async move {
            let found = blocking(move || -> Result<Found, Failure> {
                let source = source(&settings)?;
                let available = source.check().map_err(|error| Failure::from(&error))?;
                Ok(Found { source, available })
            })
            .await
            .unwrap_or_else(|error| Err(Failure::Stopped(Job::Check, error.to_string())));
            let at = Utc::now().timestamp_millis();
            match found {
                Err(message) => self.state.update(|s| {
                    s.phase = Phase::Failed(message);
                    s.checked_at = Some(at);
                }),
                Ok(Found { source, available: None }) => self.state.update(|s| {
                    s.phase = Phase::UpToDate;
                    s.install = install_of(&source);
                    s.checked_at = Some(at);
                }),
                Ok(Found { source, available: Some(available) }) => {
                    let page = source.release_page(&available.version);
                    self.state.update(|s| {
                        s.install = install_of(&source);
                        s.checked_at = Some(at);
                        s.page = page;
                    });
                    self.offer(source, available);
                }
            }
        });
    }

    /// Downloads and stages `available` when the app can install it, else sends the trader to its page.
    fn offer(self, source: Source, available: tradelens_update::Available) {
        let version = available.version.clone();
        let manual = match (install_of(&source), available.sha256.is_some()) {
            (Install::Packaged, true) => None,
            (Install::ReadOnly, _) => Some(Manual::ReadOnly),
            (Install::Packaged, false) => Some(Manual::Page),
            (Install::FromSource | Install::Unknown, _) => Some(Manual::FromSource),
        };
        let downloads = self.downloads();
        let (None, Some(dir)) = (manual, downloads) else {
            let manual = manual.unwrap_or(Manual::Page);
            return self.state.update(|s| s.phase = Phase::Available(version, manual));
        };
        // A version staged before goes first: a new one is unpacked in the same place.
        let earlier = self.staged.replace(None);
        self.state.update(|s| s.phase = Phase::Downloading(version.clone()));
        spawn(async move {
            let staged = blocking(move || -> Result<Staged, Failure> {
                drop(earlier);
                let archive = source.download(&available, &dir).map_err(|error| Failure::from(&error))?;
                source.stage(&archive).map_err(|error| Failure::from(&error))
            })
            .await
            .unwrap_or_else(|error| Err(Failure::Stopped(Job::Download, error.to_string())));
            match staged {
                Ok(staged) => {
                    self.staged.replace(Some(staged));
                    log::info!("update: version {version} installs when the app quits");
                    self.state.update(|s| s.phase = Phase::Ready(version));
                }
                Err(message) => self.state.update(|s| s.phase = Phase::Failed(message)),
            }
        });
    }

    /// Swaps the staged update in, as the app quits. Renames only: it fits NSH's wait at quit.
    fn install_now(self) {
        let Some(staged) = self.staged.replace(None) else { return };
        match staged.finish() {
            Ok(program) => log::info!("update: installed, the next start runs {}", program.display()),
            Err(error) => log::error!("update: could not install as the app quit: {error}"),
        }
    }

    /// Installs the staged update and starts the app again in it ("Khởi động lại để cập nhật").
    pub(crate) fn restart(self) {
        self.owner.run(move || {
            let Some(staged) = self.staged.replace(None) else { return };
            // The restart starts the program at the path the trader knows, even once it was swapped.
            nsh_core::restart::remember_program();
            spawn(async move {
                let finished =
                    blocking(move || staged.finish().map_err(|error| Failure::from(&UpdateError::from(error))))
                        .await
                        .unwrap_or_else(|error| Err(Failure::Stopped(Job::Install, error.to_string())));
                match finished {
                    Ok(_) => nsh_core::element::desktop::restart(),
                    Err(message) => self.state.update(|s| s.phase = Phase::Failed(message)),
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(phase: Phase) -> State {
        State { ready: true, install: Install::Packaged, phase, ..State::default() }
    }

    #[test]
    fn automatic_updates_are_on_until_turned_off() {
        assert!(Settings::default().auto());
        assert!(Settings { auto: Some("1".into()), ..Settings::default() }.auto());
        assert!(!Settings { auto: Some("0".into()), ..Settings::default() }.auto());
    }

    #[test]
    fn a_run_from_the_code_checks_by_itself_only_when_a_source_is_named() {
        let from_source = State { install: Install::FromSource, ..state(Phase::Idle) };
        if std::env::var("TRADELENS_UPDATE_URL").is_err() {
            assert!(!from_source.automatic(), "no call to GitHub that nobody asked for");
        }
        let named = State {
            settings: Settings { url: Some("http://127.0.0.1:9/latest.json".into()), ..Settings::default() },
            ..from_source
        };
        assert!(named.automatic());
        assert!(state(Phase::Idle).automatic(), "a package checks by itself");
        let off = State { settings: Settings { auto: Some("0".into()), ..Settings::default() }, ..state(Phase::Idle) };
        assert!(!off.automatic());
    }

    #[test]
    fn the_status_line_says_where_the_update_stands() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            assert_eq!(state(Phase::Checking).status(), "Đang kiểm tra bản mới…");
            assert_eq!(state(Phase::Downloading("0.4.1".into())).status(), "Đang tải bản 0.4.1…");
            assert_eq!(state(Phase::Ready("0.4.1".into())).status(), "Bản 0.4.1 đã tải, sẽ cài khi khởi động lại");
            assert!(state(Phase::Available("0.4.1".into(), Manual::FromSource)).status().starts_with("Có bản 0.4.1"));
            let checked = State { checked_at: Some(Utc::now().timestamp_millis()), ..state(Phase::UpToDate) };
            assert!(checked.status().starts_with("Bản mới nhất · kiểm tra lúc "), "{}", checked.status());
        });
    }

    #[test]
    fn the_release_notes_link_goes_to_the_release_found_or_the_list() {
        assert_eq!(state(Phase::Idle).notes_page(), "https://github.com/DEVfancybear/tradelens/releases");
        let found = State { page: Some("https://example.com/v0.4.1".into()), ..state(Phase::Idle) };
        assert_eq!(found.notes_page(), "https://example.com/v0.4.1");
    }

    #[test]
    fn a_missing_release_reads_as_none_published_yet() {
        let error = UpdateError::Status { status: 404, url: "https://x/latest.json".into() };
        let said = crate::i18n::with_text(crate::i18n::Lang::Vi, move || Failure::from(&error).text());
        assert!(said.starts_with("Chưa có bản phát hành"), "{said}");
    }
}
