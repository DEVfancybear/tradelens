//! TradeLens's self-update, without UI: NSH's own updater ([`nsh_updater`]) over HTTPS.
//!
//! A release publishes the archives `nsh package` makes, the manifest `latest.json` that
//! `nsh update-manifest` writes for them and its signature `latest.json.sig`. [`Source`] knows
//! where that manifest is (the packaged app's `nsh-updater.json`, or a source made for a run from
//! the code) and fetches it; `nsh-updater` decides whether it offers a newer version, checks the
//! signature and the downloaded archive, unpacks the new app beside the installed one
//! ([`Source::stage`]) and swaps it in ([`Staged::finish`]), which the app does as it quits.
//!
//! Everything here blocks: call it off the UI thread.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub use nsh_updater::{
    Available, Config, Error as ManifestError, install::Installation, install::Staged, platform, sha256, sign,
};

/// The `User-Agent` every request sends: the app's name and nothing about the user.
const USER_AGENT: &str = "TradeLens-updater";
/// How long to wait for a connection, and for any single read.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest manifest or signature read: a manifest lists a few archives, so a bigger answer is
/// not one.
const MAX_MANIFEST: u64 = 1 << 20;
/// What a download is called while it arrives, after the archive's own name.
const PART: &str = "part";

/// Why an update could not be checked, downloaded or staged.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// No answer from the update server.
    #[error("update server unreachable: {0}")]
    Network(String),
    /// The server answered, but not with the file.
    #[error("update server answered {status} for {url}")]
    Status { status: u16, url: String },
    /// The source, the manifest, its signature, the archive or the new app was refused.
    #[error(transparent)]
    Refused(#[from] ManifestError),
    /// The file system refused.
    #[error("update files: {0}")]
    Io(#[from] io::Error),
    /// The release has no archive this app can install itself: no installation to replace, or the
    /// manifest names a page rather than an archive.
    #[error("this release cannot be installed by the app")]
    NotInstallable,
}

/// Where the manifest is: the GitHub Releases of a repository, or a URL of the app's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `owner/repo`.
    Github(String),
    /// The manifest's URL; it needs a public key.
    Url(String),
}

/// Where updates come from, what runs now, and the installed app they replace (if any).
#[derive(Debug, Clone)]
pub struct Source {
    config: Config,
    installation: Option<Installation>,
    agent: ureq::Agent,
}

impl Source {
    /// The source of the packaged app this process runs from: the `nsh-updater.json` that `nsh
    /// package` left among its assets, and the installation it describes. `None` when this
    /// executable is not such a package (`cargo run`, the tests) or its file cannot be read.
    pub fn packaged() -> Option<Self> {
        let executable = std::env::current_exe().ok()?;
        let bytes = fs::read(shipped_config(&executable)?).ok()?;
        let config = match Config::parse(&bytes) {
            Ok(config) => config,
            Err(error) => {
                log::warn!("update: {}: {error}", nsh_updater::CONFIG_ASSET);
                return None;
            }
        };
        let installation = Installation::current(&config.id, &bytes)?;
        Some(Self::of(config, Some(installation)))
    }

    /// A source for app `id` running `version`, with no installation to replace: a run from the
    /// code checks what a release offers, and installs nothing.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Refused`] when the source is not one `nsh-updater` accepts: a version that is
    /// not semver, an `owner/repo` that is not one, a URL that is neither HTTPS nor this machine, a
    /// URL without `public_key`, or a key that is not 32 bytes of base64.
    pub fn new(origin: Origin, id: &str, version: &str, public_key: Option<&str>) -> Result<Self, UpdateError> {
        let mut config = Config::default();
        id.clone_into(&mut config.id);
        version.clone_into(&mut config.version);
        match origin {
            Origin::Github(repo) => config.github = Some(repo),
            Origin::Url(url) => config.url = Some(url),
        }
        config.public_key = public_key.map(str::to_owned);
        config.validate()?;
        Ok(Self::of(config, None))
    }

    /// This source, replacing `installation` (a package laid out as `nsh package` does).
    #[must_use]
    pub fn with_installation(self, installation: Installation) -> Self {
        Self { installation: Some(installation), ..self }
    }

    fn of(config: Config, installation: Option<Installation>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_per_read(Some(READ_TIMEOUT))
            .build()
            .into();
        Self { config, installation, agent }
    }

    /// The version that runs now.
    pub fn version(&self) -> &str {
        &self.config.version
    }

    /// Whether the app can replace itself: it runs from a package, in a directory the user can write.
    pub fn installable(&self) -> bool {
        self.installation.as_ref().is_some_and(Installation::can_install)
    }

    /// Whether this source has an installation to replace (it may still be read-only).
    pub fn is_packaged(&self) -> bool {
        self.installation.is_some()
    }

    /// The web page of release `version`, for the trader to read or download by hand: its GitHub
    /// release; `None` for a source of the app's own.
    pub fn release_page(&self, version: &str) -> Option<String> {
        self.config.github.as_ref().map(|repo| format!("https://github.com/{repo}/releases/tag/v{version}"))
    }

    /// What the manifest offers this machine, when it is newer than the running version.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Network`] or [`UpdateError::Status`] when the manifest cannot be fetched (a
    /// repository without a release answers 404), and [`UpdateError::Refused`] for a manifest that is
    /// unsigned while the source has a key, signed with another key, malformed, or naming a URL that
    /// is not HTTPS.
    pub fn check(&self) -> Result<Option<Available>, UpdateError> {
        let manifest = self.fetch(&self.config.manifest_url())?;
        let signature = match self.config.signature_url() {
            Some(url) => match self.fetch(&url) {
                Ok(bytes) => Some(bytes),
                // A missing signature is refused by the check below, with its own words.
                Err(UpdateError::Status { status: 404, .. }) => None,
                Err(error) => return Err(error),
            },
            None => None,
        };
        Ok(self.config.check(&manifest, signature.as_deref(), &platform())?)
    }

    /// Downloads the archive `available` names into `dir`, checking its size and SHA-256 as it
    /// arrives, and returns its path. An archive already there with the right hash is not fetched
    /// again. What arrives goes to `<name>.part` first, removed when it is refused.
    ///
    /// # Errors
    ///
    /// [`UpdateError::NotInstallable`] when the manifest gives no hash (its URL is a page),
    /// [`UpdateError::Refused`] when what arrived is not what the manifest says, and the network and
    /// file errors.
    pub fn download(&self, available: &Available, dir: &Path) -> Result<PathBuf, UpdateError> {
        let Some(sha256) = available.sha256.as_deref() else { return Err(UpdateError::NotInstallable) };
        fs::create_dir_all(dir)?;
        let target = dir.join(archive_name(&available.url));
        if fs::read(&target).is_ok_and(|bytes| nsh_updater::sha256(&bytes) == sha256.to_ascii_lowercase()) {
            return Ok(target);
        }
        let part = target.with_extension(format!("{}.{PART}", extension_of(&target)));
        let fetched = self.fetch_into(&available.url, &part, nsh_updater::Download::new(sha256, available.size));
        if let Err(error) = fetched {
            drop(fs::remove_file(&part));
            return Err(error);
        }
        fs::rename(&part, &target)?;
        Ok(target)
    }

    /// Unpacks `archive` beside the installed app and checks the new app, without touching the
    /// installed one: [`Staged::finish`] swaps it in (as the app quits), dropping it removes it.
    ///
    /// # Errors
    ///
    /// [`UpdateError::NotInstallable`] without an installation, and [`UpdateError::Refused`] when the
    /// archive holds no runnable app.
    pub fn stage(&self, archive: &Path) -> Result<Staged, UpdateError> {
        let installation = self.installation.as_ref().ok_or(UpdateError::NotInstallable)?;
        Ok(installation.stage(archive)?)
    }

    /// Removes what an earlier update left beside the app: the old executable Windows could not
    /// delete while it ran, an update staged by a run that crashed.
    pub fn remove_leftovers(&self) {
        if let Some(installation) = &self.installation {
            installation.remove_leftovers();
        }
    }

    /// The body of `url`, at most [`MAX_MANIFEST`] bytes.
    fn fetch(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        let mut response = self.get(url)?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_MANIFEST)
            .read_to_vec()
            .map_err(|error| UpdateError::Network(error.to_string()))
    }

    /// Streams the body of `url` into the file `path` through `check`, then asks `check` whether it
    /// was the whole archive.
    fn fetch_into(&self, url: &str, path: &Path, mut check: nsh_updater::Download) -> Result<(), UpdateError> {
        let mut response = self.get(url)?;
        let mut body = response.body_mut().as_reader();
        let mut file = io::BufWriter::new(fs::File::create(path)?);
        let mut chunk = vec![0; 64 * 1024];
        loop {
            let read = body.read(&mut chunk).map_err(|error| UpdateError::Network(error.to_string()))?;
            let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else { break };
            check.update(bytes)?;
            file.write_all(bytes)?;
        }
        file.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
        Ok(check.finish()?)
    }

    fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, UpdateError> {
        let response = self.agent.get(url).call().map_err(|error| UpdateError::Network(error.to_string()))?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            Ok(response)
        } else {
            Err(UpdateError::Status { status, url: url.to_owned() })
        }
    }
}

/// Where `nsh package` puts the app's `nsh-updater.json`, for the executable at `executable`: in
/// `nsh/assets` beside it, or in the `.app`'s `Contents/Resources/nsh/assets` on macOS.
fn shipped_config(executable: &Path) -> Option<PathBuf> {
    let beside = executable.parent()?;
    let resources =
        if cfg!(target_os = "macos") { beside.parent()?.join("Resources").join("nsh") } else { beside.join("nsh") };
    Some(resources.join("assets").join(nsh_updater::CONFIG_ASSET))
}

/// The file name a download is kept under: the last segment of its URL, reduced to characters
/// that are safe in a file name.
fn archive_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let name: String =
        last.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    if name.trim_matches('.').is_empty() { "update.archive".to_owned() } else { name }
}

fn extension_of(path: &Path) -> String {
    path.extension().map_or_else(String::new, |e| e.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_named_after_the_last_segment_of_its_url() {
        assert_eq!(
            archive_name("https://github.com/o/r/releases/download/v1.0.0/TradeLens-1.0.0-windows-x86_64.zip?x=1"),
            "TradeLens-1.0.0-windows-x86_64.zip"
        );
        assert_eq!(archive_name("http://127.0.0.1:9/a b/..%2F..%2Fx.zip"), ".._2F.._2Fx.zip");
        assert_eq!(archive_name("https://host/"), "update.archive");
        assert_eq!(archive_name("https://host/.."), "update.archive");
    }

    #[test]
    fn the_shipped_configuration_is_in_the_package_resources() {
        let exe = Path::new("/apps/TradeLens/TradeLens.exe");
        let path = shipped_config(exe).expect("an executable has a parent");
        if cfg!(target_os = "macos") {
            assert!(path.ends_with("Resources/nsh/assets/nsh-updater.json"), "{}", path.display());
        } else {
            assert_eq!(path, Path::new("/apps/TradeLens/nsh/assets/nsh-updater.json"));
        }
    }

    #[test]
    fn a_source_outside_github_needs_a_key() {
        let url = Origin::Url("https://dl.example.com/latest.json".into());
        assert!(matches!(Source::new(url, "vn.dat.tradelens", "0.1.0", None), Err(UpdateError::Refused(_))));
        let github = Source::new(Origin::Github("DEVfancybear/tradelens".into()), "vn.dat.tradelens", "0.1.0", None)
            .expect("GitHub needs no key");
        assert!(!github.is_packaged(), "a source made by hand replaces nothing");
        assert!(!github.installable());
        assert_eq!(
            github.release_page("0.4.1").as_deref(),
            Some("https://github.com/DEVfancybear/tradelens/releases/tag/v0.4.1")
        );
    }
}
