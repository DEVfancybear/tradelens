//! MetaTrader 5 itself, for a machine without it: MetaQuotes' own installer, downloaded from their
//! site and run unattended (`mt5setup.exe /auto`). A terminal of MetaQuotes logs in to any broker's
//! server, so no broker's own installer is needed.
//!
//! The installer changes with every build of the terminal, so it cannot be pinned to a hash like
//! the private Python ([`crate::runtime`]); it is run only when Windows says it is signed by
//! MetaQuotes. It asks for administrator rights itself (Windows shows its prompt), installs into
//! `Program Files\MetaTrader 5` whatever `/path:` says (build 6327 ignores it), and opens the
//! terminal once done; that window is closed again, since the app runs hidden copies of its own
//! ([`crate::install::portable_copy`]).

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use crate::{
    install::{self, Install},
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

/// How the install is watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Watch {
    /// How long between two looks at the machine.
    every: Duration,
    /// How long the install may take in all.
    limit: Duration,
    /// How long the installer may be gone with no terminal found before the install is taken for
    /// given up: between the first installer and the one Windows starts with administrator rights,
    /// neither may be listed for a moment.
    settle: Duration,
}

impl Watch {
    const REAL: Self =
        Self { every: Duration::from_secs(2), limit: Duration::from_mins(15), settle: Duration::from_secs(10) };
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

/// Runs the installer `download` saved, waits until it has installed a terminal, closes the window
/// it opens and removes the installer; the terminals installed then. Blocks for as long as the
/// install takes (a minute or two, plus the time the trader takes to allow it): call it off the
/// UI thread.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with a Vietnamese message when the installer does not start, stops without
/// installing a terminal (administrator rights refused) or takes too long.
pub fn run(installer: &Path) -> Result<Vec<Install>, Mt5Error> {
    let outcome = install_with(&mut Windows, installer, Watch::REAL);
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
    /// Starts `installer` unattended.
    fn launch(&mut self, installer: &Path) -> Result<(), Mt5Error>;
    /// Whether the installer, or the Windows prompt asking to allow it, is still there.
    fn installer_running(&mut self) -> bool;
    /// The terminals installed on the machine ([`install::find_installs`]).
    fn installs(&mut self) -> Vec<Install>;
    /// Closes the window of `terminal` the installer opened, if it did.
    fn close_opened(&mut self, terminal: &Path);
    /// Waits `time` before the next look.
    fn pause(&mut self, time: Duration);
}

/// Starts the installer unless a terminal is installed already, then waits until it is gone with
/// a terminal installed.
fn install_with(machine: &mut impl Machine, installer: &Path, watch: Watch) -> Result<Vec<Install>, Mt5Error> {
    let found = machine.installs();
    if !found.is_empty() {
        return Ok(found);
    }
    machine.launch(installer)?;
    let mut waited = Duration::ZERO;
    let mut gone_for = Duration::ZERO;
    loop {
        machine.pause(watch.every);
        waited += watch.every;
        if machine.installer_running() {
            gone_for = Duration::ZERO;
        } else {
            let found = machine.installs();
            if !found.is_empty() {
                for install in &found {
                    machine.close_opened(&install.terminal);
                }
                return Ok(found);
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

/// The machine as it is.
#[derive(Debug)]
struct Windows;

impl Machine for Windows {
    fn launch(&mut self, installer: &Path) -> Result<(), Mt5Error> {
        let mut run = std::process::Command::new(installer);
        run.arg("/auto").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(folder) = installer.parent() {
            run.current_dir(folder);
        }
        // The first installer only starts a second one with administrator rights and exits: it
        // is not waited for here, the watch looks for the installer by name.
        run.spawn()
            .map(drop)
            .map_err(|error| Mt5Error::Setup(format!("Không chạy được trình cài MetaTrader 5: {error}")))
    }

    fn installer_running(&mut self) -> bool {
        let mut list = python::command("tasklist");
        list.args(["/NH", "/FO", "CSV"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        // When the list cannot be read, the installer is taken to be running: the overall limit
        // still ends the wait.
        python::output_within(list, Duration::from_secs(15))
            .map_or(true, |output| lists_installer(&String::from_utf8_lossy(&output.stdout)))
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

    fn pause(&mut self, time: Duration) {
        thread::sleep(time);
    }
}

/// Whether `tasklist /FO CSV` lists the installer or the Windows prompt that asks to allow it.
fn lists_installer(listed: &str) -> bool {
    listed.lines().filter_map(|line| line.split(',').next()).any(|name| {
        let name = name.trim().trim_matches('"');
        name.eq_ignore_ascii_case(INSTALLER_FILE) || name.eq_ignore_ascii_case("consent.exe")
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

    /// A machine whose installer runs for `runs` looks, then leaves `after` installed.
    #[derive(Debug, Default)]
    struct Fake {
        before: Vec<Install>,
        after: Vec<Install>,
        runs: u32,
        launch_fails: bool,
        launched: bool,
        looks: u32,
        closed: Vec<PathBuf>,
    }

    impl Machine for Fake {
        fn launch(&mut self, _installer: &Path) -> Result<(), Mt5Error> {
            if self.launch_fails {
                return Err(Mt5Error::Setup("không chạy được".to_owned()));
            }
            self.launched = true;
            Ok(())
        }

        fn installer_running(&mut self) -> bool {
            self.looks += 1;
            self.looks <= self.runs
        }

        fn installs(&mut self) -> Vec<Install> {
            if self.launched && self.looks > self.runs { self.after.clone() } else { self.before.clone() }
        }

        fn close_opened(&mut self, terminal: &Path) {
            self.closed.push(terminal.to_path_buf());
        }

        fn pause(&mut self, _time: Duration) {}
    }

    const WATCH: Watch =
        Watch { every: Duration::from_secs(1), limit: Duration::from_secs(60), settle: Duration::from_secs(3) };

    fn terminal() -> Install {
        Install::at(PathBuf::from(r"C:\Program Files\MetaTrader 5\terminal64.exe"))
    }

    #[test]
    fn a_terminal_installed_already_is_not_installed_again() {
        let mut machine = Fake { before: vec![terminal()], ..Fake::default() };
        let found = install_with(&mut machine, Path::new("mt5setup.exe"), WATCH).expect("nothing to do");
        assert_eq!(found, vec![terminal()]);
        assert!(!machine.launched, "the installer is not run");
    }

    #[test]
    fn the_install_ends_once_the_installer_is_gone_and_its_window_is_closed() {
        let mut machine = Fake { after: vec![terminal()], runs: 5, ..Fake::default() };
        let found = install_with(&mut machine, Path::new("mt5setup.exe"), WATCH).expect("installed");
        assert_eq!(found, vec![terminal()]);
        assert_eq!(machine.looks, 6, "watched while the installer ran, then once more");
        assert_eq!(machine.closed, vec![terminal().terminal], "the terminal the installer opened is closed");
    }

    #[test]
    fn an_installer_gone_without_a_terminal_is_a_refusal_said_after_a_moment() {
        let mut machine = Fake { runs: 2, ..Fake::default() };
        let error = install_with(&mut machine, Path::new("mt5setup.exe"), WATCH).expect_err("nothing installed");
        assert!(error.message().contains("Có"), "{error}");
        assert_eq!(machine.looks, 5, "two looks while it ran, then three seconds without it");
        assert_eq!(machine.closed, Vec::<PathBuf>::new(), "nothing to close");
    }

    #[test]
    fn an_install_that_never_ends_is_given_up() {
        let mut machine = Fake { runs: u32::MAX, after: vec![terminal()], ..Fake::default() };
        let error = install_with(&mut machine, Path::new("mt5setup.exe"), WATCH).expect_err("too long");
        assert!(error.message().contains("1 phút"), "{error}");
        assert_eq!(machine.looks, 60);
    }

    #[test]
    fn an_installer_that_does_not_start_says_so() {
        let mut machine = Fake { launch_fails: true, ..Fake::default() };
        let error = install_with(&mut machine, Path::new("mt5setup.exe"), WATCH).expect_err("not started");
        assert_eq!(error, Mt5Error::Setup("không chạy được".to_owned()));
        assert_eq!(machine.looks, 0);
    }

    #[test]
    fn the_installer_and_the_windows_prompt_are_found_in_the_task_list() {
        let listed = "\"System\",\"4\",\"Services\",\"0\",\"144 K\"\r\n\"MT5SETUP.EXE\",\"2108\",\"Console\",\"1\",\"9,120 K\"\r\n";
        assert!(lists_installer(listed));
        assert!(lists_installer("\"consent.exe\",\"77\",\"Console\",\"1\",\"1 K\""));
        assert!(!lists_installer("\"terminal64.exe\",\"9\",\"Console\",\"1\",\"1 K\""));
        assert!(!lists_installer("INFO: No tasks are running which match the specified criteria."));
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
