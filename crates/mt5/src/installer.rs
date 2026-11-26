//! MetaTrader 5 itself, for a machine without it: MetaQuotes' own installer, downloaded from their
//! site and run unattended (`mt5setup.exe /auto`), then copied into the app's data folder. A
//! terminal of MetaQuotes logs in to any broker's server, so no broker's own installer is needed.
//!
//! The installer changes with every build of the terminal, so it cannot be pinned to a hash like
//! the private Python ([`crate::runtime`]); it is run only when Windows says it is signed by
//! MetaQuotes. It cannot install anywhere but `Program Files\MetaTrader 5` (build 6327 ignores
//! `/path:`), asks for administrator rights itself (Windows shows its prompt) and opens the terminal
//! once done. So the app closes that window, copies the installed folder into its own data folder
//! ([`own`]), and removes the copy in `Program Files` with MetaQuotes' uninstaller (a second
//! prompt). The app's copy is never run itself: each account runs a portable copy of it
//! ([`crate::install::portable_copy`]).

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use crate::{
    install::{self, Install, TERMINAL_EXE},
    protocol::Mt5Error,
    python, runtime,
};

/// MetaQuotes' installer for 64-bit Windows, as their download page links it.
pub const INSTALLER_URL: &str = "https://download.mql5.com/cdn/web/metaquotes.ltd/mt5/mt5setup.exe";
/// About how big the installer is (build 6327: 23,455,320 bytes), for the line that says it is
/// being downloaded.
pub const INSTALLER_SIZE: u64 = 23_455_320;
/// The largest installer accepted: far above any build so far, so a server sending something else
/// endlessly is cut off.
const INSTALLER_LIMIT: u64 = 200 * 1024 * 1024;
/// The name of the certificate the installer must be signed with.
const SIGNER: &str = "MetaQuotes Ltd.";
/// What the installer is saved as.
const INSTALLER_FILE: &str = "mt5setup.exe";
/// MetaQuotes' uninstaller, in the installed folder; left out of the app's copy.
const UNINSTALLER_FILE: &str = "uninstall.exe";
/// The app's copy of MetaTrader 5, under the connection's folder.
const OWN_FOLDER: &str = "metatrader";
/// What the terminal picker calls the app's copy.
const OWN_NAME: &str = "MetaTrader 5 (TradeLens)";

/// How a program of MetaQuotes is watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Watch {
    /// How long between two looks at the machine.
    every: Duration,
    /// How long the program may take in all.
    limit: Duration,
    /// How long the program may be gone without having done its work before it is taken for given
    /// up: between the first program and the one Windows starts with administrator rights, neither
    /// may be listed for a moment.
    settle: Duration,
}

impl Watch {
    const INSTALL: Self =
        Self { every: Duration::from_secs(2), limit: Duration::from_mins(15), settle: Duration::from_secs(10) };
    const REMOVE: Self =
        Self { every: Duration::from_secs(2), limit: Duration::from_mins(5), settle: Duration::from_secs(10) };
}

/// What installing left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The app's own copy of MetaTrader 5 ([`own`]).
    pub install: Install,
    /// Why MetaQuotes' copy in `Program Files` is still there, in Vietnamese, when it could not be
    /// removed. The app's copy works either way.
    pub left_behind: Option<String>,
}

/// The app's own copy of MetaTrader 5 under the connection's folder `mt5`, when it is there.
pub fn own(mt5: &Path) -> Option<Install> {
    let terminal = mt5.join(OWN_FOLDER).join(TERMINAL_EXE);
    terminal.is_file().then(|| Install { terminal, name: OWN_NAME.to_owned() })
}

/// Downloads MetaQuotes' installer into `folder` and checks that it is signed by MetaQuotes; the
/// installer's path. Nothing is left in `folder` when this fails. Blocks: call it off the UI thread.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with a Vietnamese message when the download fails, when the file is not
/// signed by MetaQuotes, or when it cannot be written.
pub fn download(folder: &Path) -> Result<PathBuf, Mt5Error> {
    let write_error = |path: &Path, error: &dyn std::fmt::Display| {
        Mt5Error::Setup(format!("Không ghi được {}: {error}", path.display()))
    };
    fs::create_dir_all(folder).map_err(|e| write_error(folder, &e))?;
    let partial = folder.join(format!("{INSTALLER_FILE}.part"));
    let installer = folder.join(INSTALLER_FILE);
    let downloaded = runtime::download(INSTALLER_URL, &partial, INSTALLER_LIMIT).and_then(|_| {
        if installer.exists() {
            fs::remove_file(&installer).map_err(|e| write_error(&installer, &e))?;
        }
        runtime::rename_patiently(&partial, &installer).map_err(|e| write_error(&installer, &e))
    });
    let checked = downloaded.and_then(|()| check_signature(&installer));
    if let Err(error) = checked {
        drop(fs::remove_file(&partial));
        drop(fs::remove_file(&installer));
        return Err(error);
    }
    Ok(installer)
}

/// Runs the installer `download` saved and waits until it has installed MetaTrader 5, closes the
/// window it opens, copies the installed folder into the connection's folder `mt5` ([`own`]),
/// removes MetaQuotes' copy with its uninstaller, then removes the installer. Blocks for as long as
/// all that takes (a few minutes, plus the time the trader takes to allow each step): call it off
/// the UI thread.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with a Vietnamese message when the installer does not start, stops without
/// installing a terminal (administrator rights refused) or takes too long, or when the installed
/// folder cannot be copied. A copy in `Program Files` that cannot be removed is not an error: it is
/// said in [`Installed::left_behind`].
pub fn run(installer: &Path, mt5: &Path) -> Result<Installed, Mt5Error> {
    let outcome = install_with(&mut Windows, installer, &mt5.join(OWN_FOLDER));
    // The installer that started a second copy of itself with administrator rights may hold its
    // file for a moment after that copy is gone.
    for _ in 0..10 {
        if fs::remove_file(installer).is_ok() || !installer.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    outcome
}

/// What installing asks of the machine; [`Windows`] is the real one, the tests have their own.
trait Machine {
    /// Starts a program of MetaQuotes unattended (`/auto`).
    fn launch(&mut self, program: &Path) -> io::Result<()>;
    /// Whether a program named `file` (`mt5setup.exe`), or the Windows prompt asking to allow it,
    /// is running.
    fn running(&mut self, file: &str) -> bool;
    /// The terminals installed on the machine ([`install::find_installs`]).
    fn installs(&mut self) -> Vec<Install>;
    /// Closes the window of `terminal` the installer opened, if it did.
    fn close_opened(&mut self, terminal: &Path);
    /// Copies the installed folder `from` to `to` ([`copy_installed`]).
    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    /// Whether the file `path` is there.
    fn exists(&mut self, path: &Path) -> bool;
    /// Waits `time` before the next look.
    fn pause(&mut self, time: Duration);
}

/// Installs MetaTrader 5 with `installer` (unless a terminal is installed already) and copies it to
/// `own`; MetaQuotes' copy is removed when the installer made it.
fn install_with(machine: &mut impl Machine, installer: &Path, own: &Path) -> Result<Installed, Mt5Error> {
    let (system, made_here) = match machine.installs().into_iter().next() {
        Some(found) => (found, false),
        None => (wait_for_install(machine, installer, Watch::INSTALL)?, true),
    };
    let folder = system.terminal.parent().filter(|folder| !folder.as_os_str().is_empty()).ok_or_else(|| {
        Mt5Error::Setup(format!("Không biết thư mục của {} để chép MetaTrader 5.", system.terminal.display()))
    })?;
    machine.copy(folder, own).map_err(|error| {
        Mt5Error::Setup(format!(
            "MetaTrader 5 đã được cài vào {} nhưng không chép được vào thư mục của TradeLens ({}): {error}",
            folder.display(),
            own.display()
        ))
    })?;
    let left_behind = made_here.then(|| remove_installed(machine, &system, Watch::REMOVE).err()).flatten();
    Ok(Installed { install: Install { terminal: own.join(TERMINAL_EXE), name: OWN_NAME.to_owned() }, left_behind })
}

/// Starts the installer, then waits until it is gone with a terminal installed, whose window is
/// closed; that terminal.
fn wait_for_install(machine: &mut impl Machine, installer: &Path, watch: Watch) -> Result<Install, Mt5Error> {
    machine
        .launch(installer)
        .map_err(|error| Mt5Error::Setup(format!("Không chạy được trình cài MetaTrader 5: {error}")))?;
    let mut waited = Duration::ZERO;
    let mut gone_for = Duration::ZERO;
    loop {
        machine.pause(watch.every);
        waited += watch.every;
        if machine.running(INSTALLER_FILE) {
            gone_for = Duration::ZERO;
        } else {
            let found = machine.installs();
            if let Some(first) = found.first() {
                for install in &found {
                    machine.close_opened(&install.terminal);
                }
                return Ok(first.clone());
            }
            gone_for += watch.every;
            if gone_for >= watch.settle {
                return Err(Mt5Error::Setup(
                    "Trình cài MetaTrader 5 đã dừng mà chưa cài xong: có thể Windows chưa được cho phép cài (hãy bấm \"Có\" khi Windows hỏi), hoặc trình cài đã bị đóng. Hãy thử lại.".to_owned(),
                ));
            }
        }
        if waited >= watch.limit {
            return Err(Mt5Error::Setup(format!(
                "Cài MetaTrader 5 quá {} phút mà chưa xong: hãy xem cửa sổ trình cài của MetaQuotes, rồi bấm \"Kiểm tra lại\".",
                watch.limit.as_secs() / 60
            )));
        }
    }
}

/// Removes MetaQuotes' copy `installed` with its uninstaller and waits until its terminal is gone;
/// why it is still there otherwise, in Vietnamese.
fn remove_installed(machine: &mut impl Machine, installed: &Install, watch: Watch) -> Result<(), String> {
    let folder = installed.terminal.parent().unwrap_or(Path::new(""));
    let left = |why: &str| {
        format!(
            "Đã chép MetaTrader 5 vào thư mục của TradeLens, nhưng bản MetaQuotes cài trong {} chưa gỡ được ({why}). TradeLens không cần bản đó nữa: có thể gỡ \"MetaTrader 5\" trong Cài đặt Windows › Ứng dụng.",
            folder.display()
        )
    };
    let uninstaller = folder.join(UNINSTALLER_FILE);
    if !machine.exists(&uninstaller) {
        return Err(left("không thấy uninstall.exe"));
    }
    machine.launch(&uninstaller).map_err(|error| left(&format!("không chạy được trình gỡ: {error}")))?;
    let mut waited = Duration::ZERO;
    let mut gone_for = Duration::ZERO;
    loop {
        machine.pause(watch.every);
        waited += watch.every;
        if !machine.exists(&installed.terminal) {
            return Ok(());
        }
        if machine.running(UNINSTALLER_FILE) {
            gone_for = Duration::ZERO;
        } else {
            gone_for += watch.every;
            if gone_for >= watch.settle {
                return Err(left("trình gỡ đã dừng, có thể Windows chưa được cho phép gỡ"));
            }
        }
        if waited >= watch.limit {
            return Err(left("trình gỡ chạy quá lâu"));
        }
    }
}

/// Copies the installed folder `from` to `to`, without MetaQuotes' uninstaller (which would remove
/// files of the app). The copy is made beside `to` and renamed into it, so a copy cut short is
/// never taken for one; an older copy at `to` is replaced.
fn copy_installed(from: &Path, to: &Path) -> io::Result<()> {
    let name = to.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let partial = to.with_file_name(format!("{name}.copying"));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    let copied = copy_tree(from, &partial).and_then(|()| {
        if !partial.join(TERMINAL_EXE).is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("không có {TERMINAL_EXE}")));
        }
        if to.exists() {
            fs::remove_dir_all(to)?;
        }
        runtime::rename_patiently(&partial, to)
    });
    if copied.is_err() {
        drop(fs::remove_dir_all(&partial));
    }
    copied
}

/// Copies the folder `from` into `to`, recursively, leaving out the uninstaller at its top.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name().eq_ignore_ascii_case(UNINSTALLER_FILE) {
            continue;
        }
        copy_all(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return fs::copy(from, to).map(drop);
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        copy_all(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// The machine as it is.
#[derive(Debug)]
struct Windows;

impl Machine for Windows {
    fn launch(&mut self, program: &Path) -> io::Result<()> {
        let mut run = std::process::Command::new(program);
        run.arg("/auto").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(folder) = program.parent() {
            run.current_dir(folder);
        }
        // The first program only starts a second one with administrator rights and exits: it is
        // not waited for here, the watch looks for the program by name.
        run.spawn().map(drop)
    }

    fn running(&mut self, file: &str) -> bool {
        let mut list = python::command("tasklist");
        list.args(["/NH", "/FO", "CSV"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        // When the list cannot be read, the program is taken to be running: the overall limit
        // still ends the wait.
        python::output_within(list, Duration::from_secs(15))
            .map_or(true, |output| lists_running(&String::from_utf8_lossy(&output.stdout), file))
    }

    fn installs(&mut self) -> Vec<Install> {
        install::find_installs()
    }

    fn close_opened(&mut self, terminal: &Path) {
        // Waits a little for the window to open, asks it to close, and ends it if it does not.
        const SCRIPT: &str = "$deadline = (Get-Date).AddSeconds(15)\n\
            do { $p = @(Get-Process terminal64 -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:TRADELENS_TERMINAL })\n\
                 if ($p.Count -gt 0) { break }; Start-Sleep -Milliseconds 500 } while ((Get-Date) -lt $deadline)\n\
            foreach ($x in $p) { [void]$x.CloseMainWindow(); if (-not $x.WaitForExit(10000)) { $x.Kill() } }";
        let mut close = powershell(SCRIPT);
        close.env("TRADELENS_TERMINAL", terminal);
        if let Err(error) = python::output_within(close, Duration::from_secs(40)) {
            log::warn!("mt5: the terminal the installer opened could not be closed: {error}");
        }
    }

    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        copy_installed(from, to)
    }

    fn exists(&mut self, path: &Path) -> bool {
        path.is_file()
    }

    fn pause(&mut self, time: Duration) {
        thread::sleep(time);
    }
}

/// Whether `tasklist /FO CSV` lists a program named `file` or the Windows prompt that asks to
/// allow one.
fn lists_running(listed: &str, file: &str) -> bool {
    listed.lines().filter_map(|line| line.split(',').next()).any(|name| {
        let name = name.trim().trim_matches('"');
        name.eq_ignore_ascii_case(file) || name.eq_ignore_ascii_case("consent.exe")
    })
}

/// Windows PowerShell running `script`, without a window or the user's profile.
fn powershell(script: &str) -> std::process::Command {
    let program = std::env::var_os("SystemRoot").map_or_else(
        || OsString::from("powershell.exe"),
        |root| PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe").into_os_string(),
    );
    let mut run = python::command(program);
    run.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run
}

/// Asks Windows whether `file` carries a valid signature of MetaQuotes.
fn check_signature(file: &Path) -> Result<(), Mt5Error> {
    const SCRIPT: &str = "$s = Get-AuthenticodeSignature -LiteralPath $env:TRADELENS_FILE\n\
        Write-Output (\"{0}|{1}\" -f $s.Status, $s.SignerCertificate.Subject)";
    let mut ask = powershell(SCRIPT);
    ask.env("TRADELENS_FILE", file);
    let output = python::output_within(ask, Duration::from_mins(1))
        .map_err(|error| Mt5Error::Setup(format!("Không kiểm tra được chữ ký của trình cài MetaTrader 5: {error}")))?;
    signed_by_metaquotes(&String::from_utf8_lossy(&output.stdout))
}

/// Reads what [`check_signature`]'s script prints (`Valid|CN=MetaQuotes Ltd., O=…`).
fn signed_by_metaquotes(printed: &str) -> Result<(), Mt5Error> {
    let line = printed.lines().map(str::trim).find(|line| line.contains('|')).unwrap_or_default();
    let (status, subject) = line.split_once('|').unwrap_or((line, ""));
    let named = subject.split(',').map(str::trim).any(|part| part == format!("CN={SIGNER}"));
    if status == "Valid" && named {
        return Ok(());
    }
    let shown = if line.is_empty() { "không đọc được chữ ký" } else { line };
    Err(Mt5Error::Setup(format!(
        "Trình cài MetaTrader 5 tải về không mang chữ ký hợp lệ của {SIGNER} ({shown}): TradeLens không chạy nó."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = r"C:\Program Files\MetaTrader 5";
    const OWN: &str = r"D:\data\mt5\metatrader";

    fn terminal() -> Install {
        Install::at(Path::new(INSTALLED).join(TERMINAL_EXE))
    }

    /// The program of MetaQuotes a [`Fake`] machine runs.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Running {
        Nothing,
        Installer,
        Uninstaller,
    }

    /// A machine whose installer runs for `install_looks` looks and then leaves [`terminal`]
    /// installed (when `installer_installs`), and whose uninstaller runs for `remove_looks` looks
    /// and then removes it (when `uninstaller_removes`). What it is asked is written in `done`.
    #[derive(Debug)]
    struct Fake {
        installed: bool,
        install_looks: u32,
        installer_installs: bool,
        remove_looks: u32,
        uninstaller_removes: bool,
        copy_error: Option<&'static str>,
        running: Running,
        looks: u32,
        done: Vec<String>,
    }

    impl Fake {
        /// A machine without MetaTrader 5 whose installer and uninstaller do their work.
        fn new(install_looks: u32, remove_looks: u32) -> Self {
            Self {
                installed: false,
                install_looks,
                installer_installs: true,
                remove_looks,
                uninstaller_removes: true,
                copy_error: None,
                running: Running::Nothing,
                looks: 0,
                done: Vec::new(),
            }
        }
    }

    impl Machine for Fake {
        fn launch(&mut self, program: &Path) -> io::Result<()> {
            let file = program.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
            self.done.push(format!("launch {file}"));
            if file == "broken.exe" {
                return Err(io::Error::other("không chạy được"));
            }
            self.running = if file == UNINSTALLER_FILE { Running::Uninstaller } else { Running::Installer };
            self.looks = 0;
            Ok(())
        }

        fn running(&mut self, _file: &str) -> bool {
            self.looks += 1;
            let runs = match self.running {
                Running::Nothing => 0,
                Running::Installer => self.install_looks,
                Running::Uninstaller => self.remove_looks,
            };
            if self.looks <= runs {
                return true;
            }
            match self.running {
                Running::Installer if self.installer_installs => self.installed = true,
                Running::Uninstaller if self.uninstaller_removes => self.installed = false,
                _ => {}
            }
            self.running = Running::Nothing;
            false
        }

        fn installs(&mut self) -> Vec<Install> {
            if self.installed { vec![terminal()] } else { Vec::new() }
        }

        fn close_opened(&mut self, terminal: &Path) {
            self.done.push(format!("close {}", terminal.display()));
        }

        fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            self.done.push(format!("copy {} -> {}", from.display(), to.display()));
            self.copy_error.map_or(Ok(()), |error| Err(io::Error::other(error)))
        }

        fn exists(&mut self, path: &Path) -> bool {
            path.ends_with(UNINSTALLER_FILE) || self.installed
        }

        fn pause(&mut self, _time: Duration) {}
    }

    fn install(machine: &mut Fake, installer: &str) -> Result<Installed, Mt5Error> {
        install_with(machine, Path::new(installer), Path::new(OWN))
    }

    #[test]
    fn metatrader_is_installed_copied_into_the_app_and_removed_from_program_files() {
        let mut machine = Fake::new(5, 3);
        let installed = install(&mut machine, "mt5setup.exe").expect("installed");
        assert_eq!(installed.install, Install { terminal: Path::new(OWN).join(TERMINAL_EXE), name: OWN_NAME.into() });
        assert_eq!(installed.left_behind, None);
        let terminal = terminal().terminal;
        assert_eq!(
            machine.done,
            [
                "launch mt5setup.exe".to_owned(),
                format!("close {}", terminal.display()),
                format!("copy {INSTALLED} -> {OWN}"),
                "launch uninstall.exe".to_owned(),
            ]
        );
        assert!(!machine.installed, "the copy in Program Files is gone");
    }

    #[test]
    fn a_terminal_installed_already_is_copied_but_never_removed() {
        let mut machine = Fake { installed: true, ..Fake::new(0, 0) };
        let installed = install(&mut machine, "mt5setup.exe").expect("copied");
        assert_eq!(installed.left_behind, None);
        assert_eq!(machine.done, [format!("copy {INSTALLED} -> {OWN}")], "neither installed nor removed");
    }

    #[test]
    fn a_copy_in_program_files_that_stays_is_said_but_the_apps_copy_is_kept() {
        let mut machine = Fake { uninstaller_removes: false, ..Fake::new(1, 2) };
        let installed = install(&mut machine, "mt5setup.exe").expect("the app's copy is there");
        let left = installed.left_behind.expect("the copy in Program Files is said");
        assert!(left.contains(INSTALLED) && left.contains("Cài đặt Windows"), "{left}");
        assert_eq!(machine.looks, 7, "two looks while the uninstaller ran, then five without it");
    }

    #[test]
    fn a_folder_that_cannot_be_copied_is_an_error_and_nothing_is_removed() {
        let mut machine = Fake { copy_error: Some("đĩa đầy"), ..Fake::new(1, 0) };
        let error = install(&mut machine, "mt5setup.exe").expect_err("no copy");
        assert!(error.message().contains("đĩa đầy") && error.message().contains(INSTALLED), "{error}");
        assert!(!machine.done.iter().any(|step| step.contains(UNINSTALLER_FILE)), "{:?}", machine.done);
    }

    #[test]
    fn an_installer_gone_without_a_terminal_is_a_refusal_said_after_a_moment() {
        let mut machine = Fake { installer_installs: false, ..Fake::new(2, 0) };
        let error = install(&mut machine, "mt5setup.exe").expect_err("nothing installed");
        assert!(error.message().contains("Có"), "{error}");
        assert_eq!(machine.looks, 7, "two looks while it ran, then five without it");
        assert_eq!(machine.done, ["launch mt5setup.exe"], "nothing to close, copy or remove");
    }

    #[test]
    fn an_install_that_never_ends_is_given_up() {
        let mut machine = Fake::new(u32::MAX, 0);
        let error = install(&mut machine, "mt5setup.exe").expect_err("too long");
        assert!(error.message().contains("15 phút"), "{error}");
        assert_eq!(machine.looks, 450, "fifteen minutes of looks every two seconds");
    }

    #[test]
    fn an_installer_that_does_not_start_says_so() {
        let mut machine = Fake::new(0, 0);
        let error = install(&mut machine, "broken.exe").expect_err("not started");
        assert!(error.message().contains("không chạy được"), "{error}");
        assert_eq!(machine.looks, 0);
    }

    #[test]
    fn the_apps_copy_is_whole_without_the_uninstaller_and_replaces_an_older_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let installed = dir.path().join("MetaTrader 5");
        fs::create_dir_all(installed.join("Sounds")).expect("the folder is made");
        fs::write(installed.join(TERMINAL_EXE), b"MZ").expect("the terminal is written");
        fs::write(installed.join(UNINSTALLER_FILE), b"MZ").expect("the uninstaller is written");
        fs::write(installed.join("Sounds").join("ok.wav"), b"RIFF").expect("a sound is written");
        let mt5 = dir.path().join("data").join("mt5");
        let own_folder = mt5.join(OWN_FOLDER);
        fs::create_dir_all(&own_folder).expect("an older copy");
        fs::write(own_folder.join("old.txt"), b"old").expect("a file of the older copy");

        copy_installed(&installed, &own_folder).expect("copied");
        assert!(own_folder.join("Sounds").join("ok.wav").is_file(), "folders are copied");
        assert!(!own_folder.join(UNINSTALLER_FILE).exists(), "the uninstaller is left out");
        assert!(!own_folder.join("old.txt").exists(), "the older copy is replaced");
        assert!(!mt5.join(format!("{OWN_FOLDER}.copying")).exists(), "nothing is left beside it");
        assert_eq!(own(&mt5), Some(Install { terminal: own_folder.join(TERMINAL_EXE), name: OWN_NAME.into() }));

        fs::remove_file(installed.join(TERMINAL_EXE)).expect("the terminal is removed");
        copy_installed(&installed, &own_folder).expect_err("a folder without the terminal is not copied");
        assert!(own_folder.join(TERMINAL_EXE).is_file(), "the copy made before is kept");
        assert!(!mt5.join(format!("{OWN_FOLDER}.copying")).exists(), "the cut-short copy is removed");
    }

    #[test]
    fn programs_and_the_windows_prompt_are_found_in_the_task_list() {
        let listed = "\"System\",\"4\",\"Services\",\"0\",\"144 K\"\r\n\"MT5SETUP.EXE\",\"2108\",\"Console\",\"1\",\"9,120 K\"\r\n";
        assert!(lists_running(listed, INSTALLER_FILE));
        assert!(!lists_running(listed, UNINSTALLER_FILE));
        assert!(lists_running("\"uninstall.exe\",\"6\",\"Console\",\"1\",\"1 K\"", UNINSTALLER_FILE));
        assert!(lists_running("\"consent.exe\",\"77\",\"Console\",\"1\",\"1 K\"", UNINSTALLER_FILE));
        assert!(!lists_running("\"terminal64.exe\",\"9\",\"Console\",\"1\",\"1 K\"", INSTALLER_FILE));
        assert!(!lists_running("INFO: No tasks are running which match the specified criteria.", INSTALLER_FILE));
    }

    #[test]
    fn only_a_valid_signature_of_metaquotes_lets_the_installer_run() {
        assert_eq!(signed_by_metaquotes("Valid|CN=MetaQuotes Ltd., O=MetaQuotes Ltd., S=Lemesos, C=CY\r\n"), Ok(()));
        for printed in [
            "HashMismatch|CN=MetaQuotes Ltd., O=MetaQuotes Ltd.",
            "Valid|CN=MetaQuotes Ltd. Fake, O=Other",
            "NotSigned|",
            "",
        ] {
            let error = signed_by_metaquotes(printed).expect_err(printed);
            assert!(error.message().contains("không chạy"), "{printed}: {error}");
        }
    }
}
