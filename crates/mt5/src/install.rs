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
/// The folder of a terminal's settings: beside the program in a portable copy, inside the data
/// folder otherwise (Windows does not tell `Config` from `config`).
const CONFIG_DIR: &str = "Config";
/// The terminal's list of brokers' servers and their access points. Without the account's server
/// in it the terminal never opens a connection, so `initialize` of the Python package waits for it
/// until "IPC timeout".
const SERVERS_FILE: &str = "servers.dat";

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

/// The account the terminal `terminal` logged in to last, as (login, server): `[Common] Login=` and
/// `Server=` of its `common.ini`, in the data folder that names the program's folder in its
/// `origin.txt` (under `%APPDATA%\MetaQuotes\Terminal`) for an installed terminal, else beside the
/// program (a portable copy). `None` when no terminal there has logged in. Reads the `origin.txt`
/// of every data folder: call it off the UI thread.
pub fn last_login(terminal: &Path) -> Option<(String, String)> {
    let data_folders =
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("MetaQuotes").join("Terminal"));
    last_login_in(terminal, data_folders.as_deref())
}

fn last_login_in(terminal: &Path, data_folders: Option<&Path>) -> Option<(String, String)> {
    let folder = terminal.parent()?;
    let own = folder.join(CONFIG_DIR).join(COMMON_INI);
    let installed = data_folders
        .and_then(|root| fs::read_dir(root).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|data| {
            fs::read(data.join("origin.txt"))
                .ok()
                .and_then(|bytes| decode_origin(&bytes))
                .is_some_and(|origin| normal(Path::new(origin.trim())) == normal(folder))
        })
        .map(|data| data.join(CONFIG_DIR).join(COMMON_INI));
    // An installed terminal's live settings are in its data folder; a `Config\common.ini` beside
    // its program is left from a portable start, or shipped by a broker's installer.
    installed
        .chain(std::iter::once(own))
        .find_map(|ini| fs::read(ini).ok().and_then(|bytes| login_in_ini(&bytes)))
        .or_else(|| login_in_folder_name(folder))
}

/// The account an app's portable copy was made for, from its folder's name
/// (`AccountRef::folder_name`, `434572089@Exness-MT5Trial7`): a copy whose login never went
/// through has no login in its `common.ini`. The server is left out when the name had to replace
/// characters of it (`_`), since the real name is then unknown.
fn login_in_folder_name(folder: &Path) -> Option<(String, String)> {
    let name = folder.file_name()?.to_str()?;
    let (login, server) = name.split_once('@')?;
    if login.is_empty() || !login.chars().all(|c| c.is_ascii_digit()) || server.is_empty() {
        return None;
    }
    let server = if server.contains('_') { String::new() } else { server.to_owned() };
    Some((login.to_owned(), server))
}

/// The terminal's settings file that names the last account.
const COMMON_INI: &str = "common.ini";

/// `Login` and `Server` of the `[Common]` section of a terminal's `common.ini` (UTF-16 with a
/// byte-order mark as the terminal writes it, or UTF-8); `None` without a login (or `Login=0`).
fn login_in_ini(bytes: &[u8]) -> Option<(String, String)> {
    let text = decode_origin(bytes)?;
    let mut in_common = false;
    let (mut login, mut server) = (None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_common = line.eq_ignore_ascii_case("[Common]");
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_common) else { continue };
        let value = value.trim();
        match key.trim() {
            "Login" if !value.is_empty() && value != "0" => login = Some(value.to_owned()),
            "Server" if !value.is_empty() => server = Some(value.to_owned()),
            _ => {}
        }
    }
    Some((login?, server.unwrap_or_default()))
}

/// The folder of `account`'s portable copy under `root`.
pub fn copy_folder(root: &Path, account: &AccountRef) -> PathBuf {
    root.join(account.folder_name())
}

/// `account`'s portable terminal under `root`, copied from the installed `terminal` the first
/// time, with the list of servers that best knows the account's server ([`give_server_list`]). The copy is made
/// beside its place and renamed into it, so a copy cut short is never taken for a finished one.
/// Blocks while copying (a terminal's folder is a few hundred megabytes).
///
/// # Errors
///
/// When the installed terminal is missing or the copy cannot be written. A server list that
/// cannot be written is only logged: the terminal may still know the server.
pub fn portable_copy(terminal: &Path, root: &Path, account: &AccountRef) -> io::Result<PathBuf> {
    let copied = copy_once(terminal, root, account)?;
    refresh_server_list(terminal, root, account);
    Ok(copied)
}

/// Gives `account`'s portable copy under `root` a better list of servers when the machine has one
/// now ([`give_server_list`]): the trader may have signed in to the server in their own terminal
/// since the copy was made, as a failed login asks them to. Called before every login, from the
/// link's thread; a list that cannot be written is only logged.
pub fn refresh_server_list(terminal: &Path, root: &Path, account: &AccountRef) {
    let data_folders =
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("MetaQuotes").join("Terminal"));
    let candidates = server_lists(terminal, data_folders.as_deref());
    let folder = copy_folder(root, account);
    match give_server_list(&folder, &candidates, &account.server) {
        Ok(true) => log::info!("mt5 {account}: the copy takes a server list that knows {}", account.server),
        Ok(false) => {}
        Err(error) => log::warn!("mt5 {account}: the server list of {} is not updated: {error}", folder.display()),
    }
}

/// A terminal's list of servers, with the server that terminal last logged in to (so the list
/// surely knows it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerList {
    path: PathBuf,
    /// `Server=` of the terminal's `common.ini`; empty when it never logged in.
    logged_in_to: String,
}

impl ServerList {
    /// The list in the settings folder `config` of a terminal, when there is one.
    fn in_settings(config: &Path) -> Option<Self> {
        let path = config.join(SERVERS_FILE);
        path.is_file().then(|| Self { logged_in_to: server_in_settings(config).unwrap_or_default(), path })
    }

    fn knows(&self, server: &str) -> bool {
        !server.is_empty() && self.logged_in_to.eq_ignore_ascii_case(server.trim())
    }
}

/// The server the terminal whose settings are in `config` last logged in to.
fn server_in_settings(config: &Path) -> Option<String> {
    let (_, server) = login_in_ini(&fs::read(config.join(COMMON_INI)).ok()?)?;
    (!server.is_empty()).then_some(server)
}

/// The server lists the copy of `terminal` may take: the one in the program's own folder, and
/// those of every terminal's data folder under `data_folders` (`%APPDATA%\MetaQuotes\Terminal`).
/// An installed terminal keeps the list it downloaded in its data folder, not beside the program,
/// so a copy of the program folder alone may not know the account's server.
fn server_lists(terminal: &Path, data_folders: Option<&Path>) -> Vec<ServerList> {
    let own = terminal.parent().map(|folder| folder.join(CONFIG_DIR));
    let installed = data_folders
        .and_then(|root| fs::read_dir(root).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(CONFIG_DIR));
    own.into_iter().chain(installed).filter_map(|config| ServerList::in_settings(&config)).collect()
}

/// Gives the portable copy at `folder` the list of `candidates` that best knows `server`, and tells
/// whether its list was replaced. A list never comes from a smaller file merely for being there:
/// - a copy that has logged in to `server` keeps its own list;
/// - else the largest list of a terminal that logged in to `server` (it surely holds the server),
///   when it is not the copy's already;
/// - else the largest list, when larger than the copy's: a list only grows as a terminal learns
///   servers, so the largest knows the most.
///
/// Written beside its place and renamed in, so a terminal never reads half a list.
///
/// # Errors
///
/// When the list cannot be read or written (a terminal still running in `folder` holds it).
fn give_server_list(folder: &Path, candidates: &[ServerList], server: &str) -> io::Result<bool> {
    let config = folder.join(CONFIG_DIR);
    let target = config.join(SERVERS_FILE);
    let own = ServerList::in_settings(&config);
    if own.as_ref().is_some_and(|list| list.knows(server)) {
        return Ok(false);
    }
    let current = own.as_ref().map_or(0, |list| file_size(&list.path));
    let best = match largest(candidates.iter().filter(|list| list.knows(server))) {
        Some((len, path)) => (len != current).then_some(path),
        None => largest(candidates.iter()).filter(|(len, _)| *len > current).map(|(_, path)| path),
    };
    let Some(best) = best else {
        return Ok(false);
    };
    fs::create_dir_all(&config)?;
    let partial = config.join(format!("{SERVERS_FILE}.copying"));
    fs::copy(best, &partial)?;
    crate::runtime::rename_patiently(&partial, &target).inspect_err(|_| drop(fs::remove_file(&partial)))?;
    Ok(true)
}

/// The size of the file at `path`, 0 when it cannot be read.
fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |meta| meta.len())
}

/// The largest of `lists`, with its size.
fn largest<'a>(lists: impl Iterator<Item = &'a ServerList>) -> Option<(u64, &'a Path)> {
    lists.map(|list| (file_size(&list.path), list.path.as_path())).max()
}

/// `account`'s portable terminal under `root`, copied from `terminal` when it is not there yet.
fn copy_once(terminal: &Path, root: &Path, account: &AccountRef) -> io::Result<PathBuf> {
    let target = copy_folder(root, account);
    let copied = target.join(TERMINAL_EXE);
    if copied.is_file() {
        return Ok(copied);
    }
    let source = terminal.parent().filter(|_| terminal.is_file()).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, crate::tr!("mt5.install.not_found", path = terminal.display()))
    })?;
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

    /// Writes a `servers.dat` of `len` bytes into the settings of the terminal folder `folder`.
    fn server_list(folder: &Path, len: usize) -> PathBuf {
        let config = folder.join(CONFIG_DIR);
        fs::create_dir_all(&config).expect("the settings folder is made");
        let path = config.join(SERVERS_FILE);
        fs::write(&path, vec![b's'; len]).expect("the server list is written");
        path
    }

    #[test]
    fn a_copy_takes_the_server_list_an_installed_terminal_downloaded() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let terminal = fake_install(dir.path(), "MetaTrader 5");
        let program = terminal.parent().expect("the terminal has a folder");
        server_list(program, 32);
        let data_folders = dir.path().join("Terminal");
        let downloaded = server_list(&data_folders.join("D0E8"), 666);
        server_list(&data_folders.join("ABCD"), 100);
        let candidates = server_lists(&terminal, Some(&data_folders));
        assert_eq!(candidates.len(), 3, "the program's own list and both data folders': {candidates:?}");

        let root = dir.path().join("mt5");
        let account = AccountRef::new("123", "Exness-MT5Trial7");
        copy_once(&terminal, &root, &account).expect("the copy is made");
        let copy = copy_folder(&root, &account);
        let server = &account.server;
        assert!(give_server_list(&copy, &candidates, server).expect("the list is written"), "the larger list is taken");
        let taken = fs::read(copy.join(CONFIG_DIR).join(SERVERS_FILE)).expect("the copy has a list");
        assert_eq!(taken, fs::read(&downloaded).expect("the downloaded list"), "the largest list is taken");
        assert!(!copy.join(CONFIG_DIR).join(format!("{SERVERS_FILE}.copying")).exists(), "nothing is left beside it");
        assert!(
            !give_server_list(&copy, &candidates, server).expect("nothing to do"),
            "the same list is not copied again"
        );
    }

    #[test]
    fn a_copy_that_learned_more_servers_keeps_its_own_list() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let copy = dir.path().join("123@Demo");
        let learned = server_list(&copy, 900);
        let data = dir.path().join("Terminal").join("D0E8");
        server_list(&data, 666);
        let candidates = server_lists(&dir.path().join("none").join(TERMINAL_EXE), data.parent());
        assert!(!give_server_list(&copy, &candidates, "Demo").expect("nothing to do"));
        assert_eq!(fs::metadata(learned).expect("the list is there").len(), 900);
    }

    /// Says in the settings of the terminal folder `folder` that it logged in to `server`.
    fn logged_in(folder: &Path, server: &str) {
        let config = folder.join(CONFIG_DIR);
        fs::create_dir_all(&config).expect("the settings folder is made");
        fs::write(config.join(COMMON_INI), utf16(&format!("[Common]\r\nLogin=1\r\nServer={server}\r\n")))
            .expect("common.ini written");
    }

    #[test]
    fn the_list_of_a_terminal_that_logged_in_to_the_server_wins_over_a_larger_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let data_folders = dir.path().join("Terminal");
        // MetaQuotes' own terminal knows many brokers but not this one; the broker's terminal does.
        server_list(&data_folders.join("MQ"), 666);
        logged_in(&data_folders.join("MQ"), "MetaQuotes-Demo");
        let broker = server_list(&data_folders.join("EX"), 200);
        logged_in(&data_folders.join("EX"), "Exness-MT5Trial7");
        let candidates = server_lists(&dir.path().join("none").join(TERMINAL_EXE), Some(&data_folders));

        let copy = dir.path().join("123@Exness-MT5Trial7");
        server_list(&copy, 666);
        assert!(give_server_list(&copy, &candidates, "exness-mt5trial7").expect("written"), "even over a larger list");
        let taken = fs::read(copy.join(CONFIG_DIR).join(SERVERS_FILE)).expect("the copy has a list");
        assert_eq!(taken, fs::read(&broker).expect("the broker's list"));
        assert!(!give_server_list(&copy, &candidates, "Exness-MT5Trial7").expect("nothing to do"), "taken once");

        // Once the copy has logged in to its server, its own list stays.
        server_list(&copy, 900);
        logged_in(&copy, "Exness-MT5Trial7");
        assert!(!give_server_list(&copy, &candidates, "Exness-MT5Trial7").expect("nothing to do"));
        assert_eq!(fs::metadata(copy.join(CONFIG_DIR).join(SERVERS_FILE)).expect("its list").len(), 900);
    }

    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn the_last_login_of_a_terminal_is_read_from_its_common_ini() {
        let dir = tempfile::tempdir().expect("a temp dir");
        // A portable copy keeps it beside the program.
        let copy = fake_install(dir.path(), "434572098@Exness-MT5Trial7");
        let config = dir.path().join("434572098@Exness-MT5Trial7").join(CONFIG_DIR);
        fs::create_dir_all(&config).expect("settings folder");
        let ini =
            "\r\n[Common]\r\nLogin=434572098\r\nServer=Exness-MT5Trial7\r\nProxyEnable=0\r\n[Charts]\r\nLogin=1\r\n";
        fs::write(config.join(COMMON_INI), utf16(ini)).expect("written");
        let found = last_login_in(&copy, None);
        assert_eq!(found, Some(("434572098".to_owned(), "Exness-MT5Trial7".to_owned())));

        // An installed terminal keeps it in the data folder that names it.
        let installed = fake_install(dir.path(), "MetaTrader 5");
        let data = dir.path().join("Terminal").join("D0E8");
        fs::create_dir_all(data.join(CONFIG_DIR)).expect("data folder");
        let folder = installed.parent().expect("a folder").display().to_string();
        fs::write(data.join("origin.txt"), utf16(&folder.to_uppercase())).expect("origin written");
        fs::write(data.join(CONFIG_DIR).join(COMMON_INI), "[Common]\nLogin=52012345\nServer=ICMarketsSC-Demo\n")
            .expect("written");
        let found = last_login_in(&installed, Some(&dir.path().join("Terminal")));
        assert_eq!(found, Some(("52012345".to_owned(), "ICMarketsSC-Demo".to_owned())), "origin matched without case");
        // A `Config\common.ini` beside an installed terminal's program is old: the data folder wins.
        logged_in(installed.parent().expect("a folder"), "Old-Server");
        let found = last_login_in(&installed, Some(&dir.path().join("Terminal")));
        assert_eq!(found.map(|(_, server)| server).as_deref(), Some("ICMarketsSC-Demo"), "the live settings win");

        // A copy of the app whose login never went through is named after its account.
        let failed = fake_install(dir.path(), "434572089@Exness-MT5Trial7");
        fs::create_dir_all(dir.path().join("434572089@Exness-MT5Trial7").join(CONFIG_DIR)).expect("settings");
        fs::write(dir.path().join("434572089@Exness-MT5Trial7").join(CONFIG_DIR).join(COMMON_INI), "[Common]\n")
            .expect("written");
        let found = last_login_in(&failed, None);
        assert_eq!(found, Some(("434572089".to_owned(), "Exness-MT5Trial7".to_owned())));
        let odd = fake_install(dir.path(), "77@Broker_Live");
        assert_eq!(last_login_in(&odd, None), Some(("77".to_owned(), String::new())), "a changed server is left out");

        // A terminal that never logged in tells nothing.
        assert_eq!(login_in_ini(b"[Common]\nLogin=0\nServer=\n"), None);
        assert_eq!(last_login_in(&dir.path().join("none").join(TERMINAL_EXE), None), None);
    }

    #[test]
    fn a_missing_terminal_is_not_copied() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let error = portable_copy(&dir.path().join("none").join(TERMINAL_EXE), dir.path(), &AccountRef::new("1", "S"))
            .expect_err("nothing to copy");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
