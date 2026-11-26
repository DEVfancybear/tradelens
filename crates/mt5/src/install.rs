//! The MetaTrader 5 terminals installed on the machine, and the portable copy each account runs in.
//!
//! One installed terminal runs one account at a time; a copy of its folder started with
//! `/portable` keeps its own data (logins, settings, history) in that folder, so the app gives each
//! account a copy of its own under the app's data folder and they run side by side.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use crate::protocol::AccountRef;

/// The terminal's program in an MT5 folder.
pub const TERMINAL_EXE: &str = "terminal64.exe";

/// An MT5 terminal installed on the machine.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Install {
    /// Its `terminal64.exe`.
    pub terminal: PathBuf,
    /// What the trader calls it: the folder's name (`MetaTrader 5`, `ICMarkets - MetaTrader 5`).
    pub name: String,
}

impl Install {
    /// The terminal at `terminal`, named after its folder.
    pub fn at(terminal: PathBuf) -> Self {
        let name =
            terminal.parent().and_then(Path::file_name).and_then(OsStr::to_str).unwrap_or("MetaTrader 5").to_owned();
        Self { terminal, name }
    }
}

/// The terminals installed on the machine: those the terminals' data folders point back to
/// (`%APPDATA%\MetaQuotes\Terminal\*\origin.txt`), and those in a folder of `Program Files`.
pub fn find_installs() -> Vec<Install> {
    let mut found = BTreeSet::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        found.extend(from_data_folders(&PathBuf::from(appdata).join("MetaQuotes").join("Terminal")));
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(root) = std::env::var_os(variable) {
            found.extend(in_folders_of(&PathBuf::from(root)));
        }
    }
    let mut seen = BTreeSet::new();
    found.into_iter().filter(|install| seen.insert(normal(&install.terminal))).collect()
}

/// A path compared without case or the kind of slash, as Windows compares them.
fn normal(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// The terminals named by the `origin.txt` of each data folder under `root`.
fn from_data_folders(root: &Path) -> Vec<Install> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("origin.txt")).ok())
        .filter_map(|bytes| decode_origin(&bytes))
        .map(|folder| PathBuf::from(folder.trim()).join(TERMINAL_EXE))
        .filter(|terminal| terminal.is_file())
        .map(Install::at)
        .collect()
}

/// The text of an `origin.txt`: UTF-16 with a byte-order mark as the terminal writes it, or UTF-8.
fn decode_origin(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (pairs, _) = rest.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        return String::from_utf16(&units).ok();
    }
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned();
    (!text.trim().is_empty()).then_some(text)
}

/// The terminals one folder below `root`.
fn in_folders_of(root: &Path) -> Vec<Install> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(TERMINAL_EXE))
        .filter(|terminal| terminal.is_file())
        .map(Install::at)
        .collect()
}

/// The folder of `account`'s portable copy under `root`.
pub fn copy_folder(root: &Path, account: &AccountRef) -> PathBuf {
    root.join(account.folder_name())
}

/// `account`'s portable terminal under `root`, copied from the installed `terminal` the first
/// time. The copy is made beside its place and renamed into it, so a copy cut short is never taken
/// for a finished one. Blocks while copying (a terminal's folder is a few hundred megabytes).
///
/// # Errors
///
/// When the installed terminal is missing or the copy cannot be written.
pub fn portable_copy(terminal: &Path, root: &Path, account: &AccountRef) -> io::Result<PathBuf> {
    let target = copy_folder(root, account);
    let copied = target.join(TERMINAL_EXE);
    if copied.is_file() {
        return Ok(copied);
    }
    let source = terminal
        .parent()
        .filter(|_| terminal.is_file())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("không thấy {}", terminal.display())))?;
    fs::create_dir_all(root)?;
    let partial = root.join(format!("{}.copying", account.folder_name()));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    copy_dir(source, &partial)?;
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(&partial, &target)?;
    Ok(copied)
}

/// Copies the folder `from` into `to`, recursively.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

/// Removes `account`'s portable copy (when the trader forgets the account).
///
/// # Errors
///
/// When the folder cannot be removed (the terminal still runs).
pub fn remove_copy(root: &Path, account: &AccountRef) -> io::Result<()> {
    let folder = copy_folder(root, account);
    if folder.exists() { fs::remove_dir_all(folder) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_install(root: &Path, name: &str) -> PathBuf {
        let folder = root.join(name);
        fs::create_dir_all(folder.join("Sounds")).expect("the folder is made");
        fs::write(folder.join(TERMINAL_EXE), b"MZ").expect("the program is written");
        fs::write(folder.join("Sounds").join("ok.wav"), b"RIFF").expect("a file is written");
        folder.join(TERMINAL_EXE)
    }

    #[test]
    fn an_origin_file_is_read_in_utf16_and_in_utf8() {
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend("C:\\Program Files\\MetaTrader 5".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_origin(&utf16).as_deref(), Some("C:\\Program Files\\MetaTrader 5"));
        assert_eq!(decode_origin(b"D:\\MT5\r\n").as_deref(), Some("D:\\MT5\r\n"));
        assert_eq!(decode_origin(b"  "), None);
    }

    #[test]
    fn terminals_are_found_from_data_folders_and_program_folders() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let terminal = fake_install(dir.path(), "Broker - MetaTrader 5");
        let data = dir.path().join("Terminal").join("ABC123");
        fs::create_dir_all(&data).expect("the data folder is made");
        let folder = terminal.parent().expect("the terminal has a folder").display().to_string();
        fs::write(data.join("origin.txt"), folder).expect("origin.txt is written");
        let named = from_data_folders(&dir.path().join("Terminal"));
        assert_eq!(named, vec![Install { terminal, name: "Broker - MetaTrader 5".to_owned() }]);
        assert_eq!(in_folders_of(dir.path()), named);
    }

    #[test]
    fn each_account_gets_a_whole_copy_made_once() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let terminal = fake_install(dir.path(), "MetaTrader 5");
        let root = dir.path().join("mt5");
        let account = AccountRef::new("123", "Demo");
        let copied = portable_copy(&terminal, &root, &account).expect("the copy is made");
        assert_eq!(copied, root.join("123@Demo").join(TERMINAL_EXE));
        assert!(root.join("123@Demo").join("Sounds").join("ok.wav").is_file(), "folders are copied too");
        fs::write(root.join("123@Demo").join("Config.ini"), b"kept").expect("the copy is changed");
        portable_copy(&terminal, &root, &account).expect("the copy is reused");
        assert!(root.join("123@Demo").join("Config.ini").is_file(), "a second call keeps the account's own data");
        remove_copy(&root, &account).expect("the copy is removed");
        assert!(!root.join("123@Demo").exists(), "nothing of the account is left");
    }

    #[test]
    fn a_missing_terminal_is_not_copied() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let error = portable_copy(&dir.path().join("none").join(TERMINAL_EXE), dir.path(), &AccountRef::new("1", "S"))
            .expect_err("nothing to copy");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
