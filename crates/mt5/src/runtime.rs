//! A Python of TradeLens's own, for a machine without one: the official embeddable package of
//! python.org unpacked in the app's data folder, with pip and the `MetaTrader5` package. It needs
//! no administrator rights, changes no `PATH` and touches no other Python; removing the folder
//! removes it.
//!
//! Everything is pinned: the interpreter and pip's wheel are checked against the size and SHA-256
//! written here before they are used, and pip installs `MetaTrader5` and numpy only as the wheels
//! whose SHA-256 is written here (`--require-hashes`). The values come from python.org's and the
//! Python Package Index's own listings.

use std::{
    fmt::Write as _,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use sha2::{Digest, Sha256};

use crate::{
    protocol::Mt5Error,
    python::{self, Python},
};

/// A file to download: where it is, and what it must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub url: String,
    /// The SHA-256 of the file, lowercase hex.
    pub sha256: String,
    pub size: u64,
}

/// A package pip installs, as one exact wheel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub name: &'static str,
    pub version: &'static str,
    /// The SHA-256 of its wheel for this Python on 64-bit Windows.
    pub sha256: &'static str,
}

/// What the private Python is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The embeddable package of python.org (a zip).
    pub python: Asset,
    /// pip's wheel, from the Python Package Index.
    pub pip: Asset,
    /// What pip installs into it, each pinned to one wheel.
    pub packages: Vec<Requirement>,
}

/// The version of the private Python: the newest one `MetaTrader5` has wheels for.
pub const VERSION: &str = "3.14.10";

impl Source {
    /// Python 3.14.10 (64-bit), pip 26.3, `MetaTrader5` 5.0.6231 and numpy 2.5.5, as python.org and
    /// the Python Package Index list them.
    pub fn official() -> Self {
        Self {
            python: Asset {
                url: "https://www.python.org/ftp/python/3.14.10/python-3.14.10-embed-amd64.zip".to_owned(),
                sha256: "90905e64a840557d8c1bc4f5b74db3d7ee85e3ffb217611cac1c7813acea4159".to_owned(),
                size: 12_619_664,
            },
            pip: Asset {
                url: "https://files.pythonhosted.org/packages/07/25/ee8c38ae72d66b844ce8eb9711ba905dba21ca1a8a0c960057c41cd23337/pip-26.3-py3-none-any.whl".to_owned(),
                sha256: "f3b60b20ffaadd4415ed7957212a38ac63fa4c5fa375bd0f4a797777940fad13".to_owned(),
                size: 1_950_951,
            },
            packages: vec![
                Requirement {
                    name: "MetaTrader5",
                    version: "5.0.6231",
                    sha256: "9a63f026072479298be0d0cbc1a878d4f659aff2e18280a4128843dbb425841b",
                },
                Requirement {
                    name: "numpy",
                    version: "2.5.5",
                    sha256: "20145b11776d2f7570cd61351b156a51d3a341822282095baa819d87272bc710",
                },
            ],
        }
    }

    /// The pip requirements file: each package at its version, with the hash of its wheel.
    fn requirements(&self) -> String {
        self.packages.iter().fold(String::new(), |mut text, package| {
            writeln!(text, "{}=={} --hash=sha256:{}", package.name, package.version, package.sha256)
                .expect("writing to a String does not fail");
            text
        })
    }
}

/// The longest a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// The longest a download may stall between two reads.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest a whole download may take: a trickling connection gives up instead of holding the
/// install (and every sign-in) forever.
const DOWNLOAD_LIMIT: Duration = Duration::from_mins(10);
/// The longest pip may take to install the packages.
const PIP_LIMIT: Duration = Duration::from_mins(15);
/// The file written last into an installed Python: its content is [`VERSION`].
const PYTHON_MARKER: &str = ".tradelens-python";
/// The file written into `site-packages` once pip is completely there.
const PIP_MARKER: &str = ".tradelens-pip";

/// The folder the private Python lives in, under `root`.
pub fn folder(root: &Path) -> PathBuf {
    root.join("python")
}

/// The private Python's program under `root` (it may not be installed).
pub fn executable(root: &Path) -> PathBuf {
    folder(root).join("python.exe")
}

fn site_packages(root: &Path) -> PathBuf {
    folder(root).join("Lib").join("site-packages")
}

/// Whether the private Python under `root` is complete, of [`VERSION`], and runs.
fn python_ready(root: &Path) -> bool {
    let program = executable(root);
    program.is_file()
        && fs::read_to_string(folder(root).join(PYTHON_MARKER)).is_ok_and(|version| version.trim() == VERSION)
        && python::probe_program(&program).is_some()
}

/// Downloads and unpacks the embeddable package under `root`, with `site` switched on so that
/// packages installed into it are found. Nothing is done when a complete Python of [`VERSION`]
/// that runs is there already; one that is incomplete, of another version or broken is replaced.
/// The package is unpacked beside its place and renamed into it, so a cut-short install is never
/// taken for one, and nothing is left beside it when this fails. Blocks: call it off the UI thread.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with a Vietnamese message when the download fails or is not the file
/// expected, or when the files cannot be written.
pub fn install_python(root: &Path, source: &Source) -> Result<PathBuf, Mt5Error> {
    let program = executable(root);
    if python_ready(root) {
        return Ok(program);
    }
    fs::create_dir_all(root).map_err(|e| write_error(root, &e))?;
    let archive = root.join("python-embed.zip.part");
    fetch(&source.python, &archive)?;
    let partial = root.join("python.partial");
    let made = remove_dir(&partial)
        .and_then(|()| unpack(&archive, &partial))
        .and_then(|()| enable_site(&partial))
        .and_then(|()| fs::write(partial.join(PYTHON_MARKER), VERSION));
    drop(fs::remove_file(&archive));
    if let Err(error) = made {
        drop(fs::remove_dir_all(&partial));
        return Err(write_error(&partial, &error));
    }
    let target = folder(root);
    let moved = remove_dir(&target).and_then(|()| rename_patiently(&partial, &target));
    if let Err(error) = moved {
        drop(fs::remove_dir_all(&partial));
        return Err(write_error(&target, &error));
    }
    Ok(program)
}

/// Puts pip into the private Python (its wheel unpacked beside `site-packages`, then moved in), then
/// installs the pinned packages with it, each only as the wheel whose hash [`Source`] names. Blocks
/// for as long as pip takes: call it off the UI thread.
///
/// # Errors
///
/// As [`install_python`], and when pip fails.
pub fn install_packages(root: &Path, source: &Source) -> Result<(), Mt5Error> {
    let site_packages = site_packages(root);
    if !site_packages.join(PIP_MARKER).is_file() {
        put_pip(root, source, &site_packages)?;
    }
    let requirements = root.join("requirements.txt");
    fs::write(&requirements, source.requirements()).map_err(|e| write_error(&requirements, &e))?;
    let mut run = python::command(executable(root));
    run.args(["-m", "pip", "install", "--no-warn-script-location", "--disable-pip-version-check", "--no-cache-dir"])
        .args(["--no-deps", "--only-binary=:all:", "--require-hashes", "-r"])
        .arg(&requirements)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = python::output_within(run, PIP_LIMIT)
        .map_err(|error| Mt5Error::Setup(crate::tr!("mt5.runtime.pip_launch_failed", error = error)))?;
    drop(fs::remove_file(&requirements));
    if output.status.success() {
        return Ok(());
    }
    Err(Mt5Error::Setup(crate::tr!("mt5.runtime.pip_failed", tail = python::tail(&output.stderr))))
}

/// Unpacks pip's wheel beside `site-packages` and moves its folders in, then writes the marker:
/// a cut-short unpack is never taken for a complete pip.
fn put_pip(root: &Path, source: &Source, site_packages: &Path) -> Result<(), Mt5Error> {
    let wheel = root.join("pip.whl.part");
    fetch(&source.pip, &wheel)?;
    let partial = root.join("pip.partial");
    let unpacked = remove_dir(&partial).and_then(|()| unpack(&wheel, &partial));
    drop(fs::remove_file(&wheel));
    let moved = unpacked.and_then(|()| {
        fs::create_dir_all(site_packages)?;
        for entry in fs::read_dir(&partial)? {
            let entry = entry?;
            let target = site_packages.join(entry.file_name());
            remove_dir(&target)?;
            rename_patiently(&entry.path(), &target)?;
        }
        fs::write(site_packages.join(PIP_MARKER), &source.pip.sha256)
    });
    drop(fs::remove_dir_all(&partial));
    moved.map_err(|e| write_error(site_packages, &e))
}

fn write_error(path: &Path, error: &io::Error) -> Mt5Error {
    Mt5Error::Setup(crate::tr!("mt5.write_failed", path = path.display(), error = error))
}

/// Removes the folder `dir` when it is there.
fn remove_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() { fs::remove_dir_all(dir) } else { Ok(()) }
}

/// Renames `from` to `to`, trying again for a few seconds: on Windows an antivirus scanning a file
/// just written holds it, and the rename is refused meanwhile.
pub(crate) fn rename_patiently(from: &Path, to: &Path) -> io::Result<()> {
    const TRIES: u32 = 20;
    let mut tries = 1;
    loop {
        match fs::rename(from, to) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied && tries < TRIES => {
                tries += 1;
                thread::sleep(Duration::from_millis(250));
            }
            done => return done,
        }
    }
}

/// Downloads `asset` to `path`, checking its size and SHA-256 as it arrives; nothing is left at
/// `path` when the download fails or is not the file expected.
fn fetch(asset: &Asset, path: &Path) -> Result<(), Mt5Error> {
    let fetched = fetch_unchecked(asset, path);
    if fetched.is_err() {
        drop(fs::remove_file(path));
    }
    fetched
}

fn fetch_unchecked(asset: &Asset, path: &Path) -> Result<(), Mt5Error> {
    let (size, sha256) = download(&asset.url, path, asset.size.saturating_add(1))?;
    if size != asset.size || sha256 != asset.sha256 {
        return Err(Mt5Error::Setup(crate::tr!(
            "mt5.runtime.wrong_download",
            url = asset.url,
            size = size,
            sha256 = sha256,
            want_size = asset.size,
            want_sha256 = asset.sha256
        )));
    }
    Ok(())
}

/// Downloads `url` to `path`, refusing a body of more than `limit` bytes, and tells the size and
/// the SHA-256 (lowercase hex) of what arrived. What was written stays at `path` when this fails:
/// the caller removes it.
pub(crate) fn download(url: &str, path: &Path, limit: u64) -> Result<(u64, String), Mt5Error> {
    let network = |error: &dyn std::fmt::Display| {
        Mt5Error::Setup(crate::tr!("mt5.runtime.download_failed", url = url, error = error))
    };
    // The machine's own certificate store: an antivirus or a company proxy that inspects HTTPS adds
    // its certificate there, and the download must work where the browser does.
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent("TradeLens")
        .tls_config(tls)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_TIMEOUT))
        .timeout_global(Some(DOWNLOAD_LIMIT))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| network(&e))?;
    let mut body = response.body_mut().with_config().limit(limit).reader();
    let mut file = io::BufWriter::new(fs::File::create(path).map_err(|e| write_error(path, &e))?);
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let read = body.read(&mut chunk).map_err(|e| network(&e))?;
        let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else { break };
        digest.update(bytes);
        size += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        file.write_all(bytes).map_err(|e| write_error(path, &e))?;
    }
    file.flush().map_err(|e| write_error(path, &e))?;
    let sha256 = digest.finalize().iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("writing to a String does not fail");
        hex
    });
    Ok((size, sha256))
}

/// Unpacks the zip `archive` into `into`; an entry whose path would leave `into` is refused.
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
    fs::create_dir_all(into)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, crate::tr!("mt5.runtime.odd_path", index = index))
        })?;
        let path = into.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut fs::File::create(&path)?)?;
    }
    Ok(())
}

/// Switches `site` on in the embeddable package's `python3*._pth` and adds `Lib\site-packages` to
/// it, so that `python -m pip` and the packages it installs are found (the package ships with both
/// off).
fn enable_site(dir: &Path) -> io::Result<()> {
    let pth = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "_pth"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, crate::tr!("mt5.runtime.no_pth")))?;
    let text = fs::read_to_string(&pth)?;
    fs::write(&pth, site_enabled(&text))
}

/// The text of a `._pth` with `Lib\site-packages` listed and `import site` on.
fn site_enabled(text: &str) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| {
            !line.is_empty() && *line != "import site" && line.trim_start_matches('#').trim() != "import site"
        })
        .map(str::to_owned)
        .collect();
    if !lines.iter().any(|line| line.eq_ignore_ascii_case(r"Lib\site-packages")) {
        lines.push(r"Lib\site-packages".to_owned());
    }
    lines.push("import site".to_owned());
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

/// Whether `python` is the app's own under `root`: the package goes into it with the pinned
/// installer ([`install_packages`]), never with `pip install --user`, which an isolated Python does
/// not read from.
pub fn is_private(python: &Python, root: &Path) -> bool {
    python.args.is_empty() && python.program == executable(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pth_file_lists_site_packages_and_imports_site() {
        let shipped = "python314.zip\r\n.\r\n\r\n# Uncomment to run site.main() automatically\r\n#import site\r\n";
        let fixed = site_enabled(shipped);
        assert_eq!(
            fixed,
            "python314.zip\r\n.\r\n# Uncomment to run site.main() automatically\r\nLib\\site-packages\r\nimport site\r\n"
        );
        assert_eq!(site_enabled(&fixed), fixed, "enabling twice changes nothing");
    }

    #[test]
    fn pip_installs_each_package_only_as_the_wheel_pinned() {
        let requirements = Source::official().requirements();
        assert_eq!(requirements.lines().count(), 2, "MetaTrader5 and numpy, its only dependency: {requirements}");
        for line in requirements.lines() {
            let (spec, hash) = line.split_once(" --hash=sha256:").expect("every line has a hash");
            assert!(spec.contains("=="), "an exact version: {line}");
            assert_eq!(hash.len(), 64, "{line}");
        }
    }

    #[test]
    fn only_the_apps_own_python_is_private() {
        let root = Path::new("C:/data/mt5/runtime");
        let own = Python { program: executable(root), args: Vec::new(), version: "3.14".into(), has_package: false };
        assert!(is_private(&own, root));
        let system = Python { program: "python".into(), ..own };
        assert!(!is_private(&system, root));
    }
}
