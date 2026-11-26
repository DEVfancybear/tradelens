//! A Python of TradeLens's own, for a machine without one: the official embeddable package of
//! python.org unpacked in the app's data folder, with pip and the `MetaTrader5` package. It needs
//! no administrator rights, changes no `PATH` and touches no other Python; removing the folder
//! removes it.
//!
//! Every file is checked against the size and SHA-256 written here (from python.org's and the Python Package Index's
//! own listings) before it is used; the packages pip then installs come from the Python Package Index over HTTPS.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use std::fmt::Write as _;

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

/// What the private Python is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The embeddable package of python.org (a zip).
    pub python: Asset,
    /// pip's wheel from the Python Package Index.
    pub pip: Asset,
}

/// The version of the private Python: the newest one `MetaTrader5` has wheels for.
pub const VERSION: &str = "3.14.10";

impl Source {
    /// Python 3.14.10 (64-bit) and pip 26.3, as python.org and the Python Package Index list them.
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
        }
    }
}

/// The package pip installs.
const PACKAGE: &str = "MetaTrader5";
/// The longest a download may stall between two reads.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest pip may take to install the package and numpy.
const PIP_LIMIT: Duration = Duration::from_mins(15);

/// The folder the private Python lives in, under `root`.
pub fn folder(root: &Path) -> PathBuf {
    root.join("python")
}

/// The private Python's program under `root` (it may not be installed).
pub fn executable(root: &Path) -> PathBuf {
    folder(root).join("python.exe")
}

/// Installs the private Python under `root` (unless it is there already), then pip and the
/// `MetaTrader5` package into it. Blocks for as long as the downloads take: call it off the UI
/// thread. `step` hears when the second part (the package) starts.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with a Vietnamese message when a download fails or is not what was
/// expected, when the files cannot be written, or when pip fails.
pub fn install(root: &Path, source: &Source, step: impl FnOnce()) -> Result<Python, Mt5Error> {
    install_python(root, source)?;
    step();
    install_package(root, source)?;
    let program = executable(root);
    python::probe_program(&program)
        .filter(|python| python.has_package)
        .ok_or_else(|| Mt5Error::Setup("Đã cài Python nhưng chưa nhập được gói MetaTrader5.".to_owned()))
}

/// Downloads and unpacks the embeddable package under `root`, with `site` switched on so that
/// packages installed into it are found. Nothing is done when it is there already. The package is
/// unpacked beside its place and renamed into it, so a cut-short install is never taken for one.
///
/// # Errors
///
/// As [`install`].
pub fn install_python(root: &Path, source: &Source) -> Result<PathBuf, Mt5Error> {
    let program = executable(root);
    if program.is_file() {
        return Ok(program);
    }
    fs::create_dir_all(root).map_err(|e| write_error(root, &e))?;
    let archive = root.join("python-embed.zip.part");
    fetch(&source.python, &archive)?;
    let partial = root.join("python.partial");
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(|e| write_error(&partial, &e))?;
    }
    let unpacked = unpack(&archive, &partial).and_then(|()| enable_site(&partial));
    drop(fs::remove_file(&archive));
    unpacked.map_err(|e| write_error(&partial, &e))?;
    let target = folder(root);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| write_error(&target, &e))?;
    }
    fs::rename(&partial, &target).map_err(|e| write_error(&target, &e))?;
    Ok(program)
}

/// Puts pip into the private Python (its wheel unpacked into `site-packages`), then installs the
/// `MetaTrader5` package with it.
///
/// # Errors
///
/// As [`install`].
pub fn install_package(root: &Path, source: &Source) -> Result<(), Mt5Error> {
    let site_packages = folder(root).join("Lib").join("site-packages");
    if !site_packages.join("pip").is_dir() {
        let wheel = root.join("pip.whl.part");
        fetch(&source.pip, &wheel)?;
        let unpacked = unpack(&wheel, &site_packages);
        drop(fs::remove_file(&wheel));
        unpacked.map_err(|e| write_error(&site_packages, &e))?;
    }
    let mut run = python::command(executable(root));
    run.args([
        "-m",
        "pip",
        "install",
        "--no-warn-script-location",
        "--disable-pip-version-check",
        "--no-cache-dir",
        PACKAGE,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let output = python::output_within(run, PIP_LIMIT)
        .map_err(|error| Mt5Error::Setup(format!("không chạy được pip của Python riêng: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(Mt5Error::Setup(format!("pip không cài được {PACKAGE}: {}", python::tail(&output.stderr))))
}

fn write_error(path: &Path, error: &io::Error) -> Mt5Error {
    Mt5Error::Setup(format!("Không ghi được {}: {error}", path.display()))
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
    let network = |error: &dyn std::fmt::Display| Mt5Error::Setup(format!("Không tải được {}: {error}", asset.url));
    let agent: ureq::Agent =
        ureq::Agent::config_builder().user_agent("TradeLens").timeout_per_read(Some(READ_TIMEOUT)).build().into();
    let mut response = agent.get(&asset.url).call().map_err(|e| network(&e))?;
    let mut body = response.body_mut().with_config().limit(asset.size.saturating_add(1)).reader();
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
    if size != asset.size || sha256 != asset.sha256 {
        return Err(Mt5Error::Setup(format!(
            "File tải về không đúng ({}): {size} byte, SHA-256 {sha256}; cần {} byte, {}.",
            asset.url, asset.size, asset.sha256
        )));
    }
    Ok(())
}

/// Unpacks the zip `archive` into `into`; an entry whose path would leave `into` is refused.
fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
    fs::create_dir_all(into)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, format!("đường dẫn lạ trong gói: mục {index}"))
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
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "gói Python không có file ._pth"))?;
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
    fn the_official_files_are_pinned() {
        let source = Source::official();
        assert!(source.python.url.contains(VERSION), "the package is of the version said");
        for asset in [&source.python, &source.pip] {
            assert!(asset.url.starts_with("https://"), "{}", asset.url);
            assert_eq!(asset.sha256.len(), 64, "{}", asset.url);
            assert!(asset.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }
}
