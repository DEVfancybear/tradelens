//! The real terminal: a Python process running `tradelens_worker.py`, which starts the account's
//! portable MT5 hidden and drives it through the `MetaTrader5` package. Requests and replies are
//! JSON lines over the process's stdin and stdout.
//!
//! Also: finding a Python on the machine, and installing the package into it.

use std::{
    ffi::OsStr,
    fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tradelens_core::AccountInfo;

use crate::{
    link::Opener,
    protocol::{Levels, Login, Mt5Error, Poll, Terminal},
};

/// The worker, written next to [`SYNC_SCRIPT`] in the folder the app gives.
pub const WORKER_SCRIPT: &str = include_str!("../../../python/tradelens_worker.py");
/// The history reader the worker imports (the same file the trader can run by hand).
pub const SYNC_SCRIPT: &str = include_str!("../../../python/tradelens_sync.py");

/// What the worker's `MetaTrader5` package calls a refused login.
const AUTH_FAILED: i64 = -6;
/// What the package says when the terminal never tells it it is ready (`RES_E_INTERNAL_FAIL_TIMEOUT`):
/// seen when the terminal does not know the account's server and so never connects.
const IPC_TIMEOUT: i64 = -10005;
/// `TRADE_RETCODE_PLACED` and `TRADE_RETCODE_DONE_PARTIAL`, which the worker answers as a failure:
/// the order went through but did not (yet) do all it asked, so its outcome is not a clear yes or no.
const UNSETTLED: [i64; 2] = [10_008, 10_010];
/// How long a login may take: a terminal's first start in a new folder downloads its updates.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a poll, an order or a probe may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the worker is given to log out and stop the terminal once told to quit.
const QUIT_GRACE: Duration = Duration::from_secs(5);
/// What `pip` installs.
const PACKAGE: &str = "MetaTrader5";

/// `CREATE_NO_WINDOW`: Python and pip run without a console window flashing up.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command for `program`; on Windows it runs without a console window.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

// --- finding Python ------------------------------------------------------------------------------

/// A Python interpreter found on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Python {
    /// The program to run, with the arguments before the script's (`py -3`).
    pub program: PathBuf,
    pub args: Vec<String>,
    /// `3.12`.
    pub version: String,
    /// Whether `import MetaTrader5` works.
    pub has_package: bool,
}

impl Python {
    fn command(&self) -> Command {
        let mut command = command(&self.program);
        command.args(&self.args);
        command
    }

    /// The interpreter as the trader would type it: `py -3`, `C:\…\python.exe`.
    pub fn display(&self) -> String {
        let mut shown = self.program.display().to_string();
        for arg in &self.args {
            shown.push(' ');
            shown.push_str(arg);
        }
        shown
    }
}

/// Where Python may be, in order: `chosen` (a setting) first, then the app's own (`private`,
/// [`crate::runtime`]), the launcher, `python` on the path and the usual install folders of python.org.
fn candidates(chosen: Option<&Path>, private: Option<&Path>) -> Vec<(PathBuf, Vec<String>)> {
    let mut found: Vec<(PathBuf, Vec<String>)> = Vec::new();
    if let Some(chosen) = chosen {
        found.push((chosen.to_path_buf(), Vec::new()));
    }
    if let Some(private) = private.filter(|program| program.is_file()) {
        found.push((private.to_path_buf(), Vec::new()));
    }
    found.push((PathBuf::from("py"), vec!["-3".to_owned()]));
    found.push((PathBuf::from("python"), Vec::new()));
    found.push((PathBuf::from("python3"), Vec::new()));
    let roots = [
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Programs").join("Python")),
        Some(PathBuf::from("C:\\")),
    ];
    for root in roots.into_iter().flatten() {
        let Ok(entries) = fs::read_dir(&root) else { continue };
        let mut versions: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|dir| dir.file_name().and_then(OsStr::to_str).is_some_and(|name| name.starts_with("Python3")))
            .map(|dir| dir.join("python.exe"))
            .filter(|exe| exe.is_file())
            .collect();
        // Newest first: `Python313` before `Python39` once the digits are read as a number.
        versions.sort_by_key(|exe| std::cmp::Reverse(minor_of(exe)));
        found.extend(versions.into_iter().map(|exe| (exe, Vec::new())));
    }
    found
}

/// The minor version a `Python3NN` folder names.
fn minor_of(exe: &Path) -> u32 {
    exe.parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_prefix("Python3"))
        .and_then(|digits| digits.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

/// What the probe prints: the version, then whether the package imports.
const PROBE: &str = "import sys, importlib.util as u\nprint('%d.%d' % sys.version_info[:2])\nprint(u.find_spec('MetaTrader5') is not None)";

/// Runs `program args` and reads what it prints, `None` when it does not run, fails or is not
/// Python 3 (the Microsoft Store's `python.exe` stand-in prints a hint and fails).
fn probe(program: &Path, args: &[String]) -> Option<Python> {
    let mut run = command(program);
    run.args(args).args(["-c", PROBE]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let output = output_within(run, Duration::from_secs(15)).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim);
    let version = lines.next()?.to_owned();
    if !version.starts_with("3.") {
        return None;
    }
    let has_package = lines.next() == Some("True");
    Some(Python { program: program.to_path_buf(), args: args.to_vec(), version, has_package })
}

/// Python 3 at `program`, `None` when it does not run there.
pub(crate) fn probe_program(program: &Path) -> Option<Python> {
    probe(program, &[])
}

/// The Python to run the worker with: the first one found that has the package, else the first
/// one found at all. `chosen` is the trader's own choice (a setting), tried first, then `private`,
/// the one the app installed for itself ([`crate::runtime::executable`]).
pub fn find(chosen: Option<&Path>, private: Option<&Path>) -> Option<Python> {
    let mut first = None;
    for (program, args) in candidates(chosen, private) {
        if let Some(python) = probe(&program, &args) {
            if python.has_package {
                return Some(python);
            }
            first.get_or_insert(python);
        }
    }
    first
}

/// Installs the `MetaTrader5` package for the user (`pip install --user`). Blocks for as long as
/// pip takes: call it off the UI thread.
///
/// # Errors
///
/// [`Mt5Error::Setup`] with the end of pip's output when it fails.
pub fn install_package(python: &Python) -> Result<(), Mt5Error> {
    let mut run = python.command();
    run.args(["-m", "pip", "install", "--user", "--upgrade", "--disable-pip-version-check", PACKAGE])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = output_within(run, Duration::from_mins(10))
        .map_err(|error| Mt5Error::Setup(format!("không chạy được pip: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(Mt5Error::Setup(format!("pip không cài được {PACKAGE}: {}", tail(&output.stderr))))
}

/// The last lines of what a program printed, on one line: what pip says when it fails.
pub(crate) fn tail(printed: &[u8]) -> String {
    let text = String::from_utf8_lossy(printed);
    let last: Vec<&str> = text.lines().rev().take(4).collect();
    last.into_iter().rev().collect::<Vec<_>>().join(" ")
}

/// Reads all of `pipe` on a thread of its own, so that a full pipe never stalls the process.
fn read_all<R: io::Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            drop(pipe.read_to_end(&mut bytes));
        }
        bytes
    })
}

/// Runs `command` to the end, killing it when it takes longer than `limit`.
pub(crate) fn output_within(mut command: Command, limit: Duration) -> io::Result<std::process::Output> {
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out = read_all(stdout);
    let err = read_all(stderr);
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            drop(child.kill());
            drop(child.wait());
            return Err(io::Error::new(io::ErrorKind::TimedOut, "quá thời gian chờ"));
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(std::process::Output { status, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

// --- the worker ----------------------------------------------------------------------------------

/// Writes the worker and the script it imports into `folder` (when they differ from what is
/// there), so the running app always uses the scripts it was built with.
///
/// # Errors
///
/// When the folder cannot be written.
pub fn install_scripts(folder: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    for (name, text) in [("tradelens_worker.py", WORKER_SCRIPT), ("tradelens_sync.py", SYNC_SCRIPT)] {
        let path = folder.join(name);
        if fs::read_to_string(&path).ok().as_deref() != Some(text) {
            fs::write(&path, text)?;
        }
    }
    Ok(folder.join("tradelens_worker.py"))
}

/// A running worker: one Python process for one account.
#[derive(Debug)]
pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<String>,
    next_id: u64,
}

impl Worker {
    /// Starts `script` with `python`.
    ///
    /// # Errors
    ///
    /// [`Mt5Error::Setup`] when the process does not start.
    pub fn spawn(python: &Python, script: &Path) -> Result<Self, Mt5Error> {
        let mut run = python.command();
        // Not `-I`: isolated mode leaves the user's site-packages (where `pip install --user` puts
        // MetaTrader5) out of `sys.path`. The app's own Python is isolated anyway by its `._pth`
        // (no script folder on `sys.path`, `PYTHONIOENCODING` ignored), which the worker handles
        // itself; `-X utf8` makes its text UTF-8 in either case, and `-u` makes every reply line
        // reach the app at once.
        run.args(["-X", "utf8", "-u"])
            .arg(script)
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(folder) = script.parent() {
            run.current_dir(folder);
        }
        let mut child = run.spawn().map_err(|error| Mt5Error::Setup(format!("không chạy được Python: {error}")))?;
        let stdin = child.stdin.take().ok_or_else(|| Mt5Error::Setup("Python không có stdin".to_owned()))?;
        let stdout = child.stdout.take().ok_or_else(|| Mt5Error::Setup("Python không có stdout".to_owned()))?;
        let (lines, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    log::debug!("mt5 worker: {line}");
                }
            });
        }
        Ok(Self { child, stdin, replies, next_id: 1 })
    }

    /// Sends one request and waits up to `limit` for its reply's `data`.
    fn call(&mut self, mut request: Value, limit: Duration) -> Result<Value, Mt5Error> {
        let id = self.next_id;
        self.next_id += 1;
        request["id"] = json!(id);
        let mut line = request.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|error| Mt5Error::Unreachable(format!("worker MT5 đã dừng: {error}")))?;
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self.replies.recv_timeout(left).map_err(|error| match error {
                RecvTimeoutError::Timeout => Mt5Error::Unreachable("MT5 không trả lời kịp".to_owned()),
                RecvTimeoutError::Disconnected => Mt5Error::Unreachable(self.exit_reason()),
            })?;
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                log::debug!("mt5 worker printed: {line}");
                continue;
            };
            // A reply without an id is the worker failing before it could read requests.
            let for_us = reply["id"].as_u64() == Some(id) || reply["id"].is_null();
            if !for_us {
                continue;
            }
            return reply_data(&reply);
        }
    }

    fn exit_reason(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!("worker MT5 đã thoát ({status})"),
            _ => "worker MT5 đã đóng kết nối".to_owned(),
        }
    }
}

/// The `data` of a reply, or the worker's refusal.
fn reply_data(reply: &Value) -> Result<Value, Mt5Error> {
    if reply["ok"].as_bool() == Some(true) {
        return Ok(reply["data"].clone());
    }
    let message = reply["error"].as_str().unwrap_or("MT5 báo lỗi không rõ").to_owned();
    let code = reply["code"].as_i64().unwrap_or(-1);
    Err(if code == AUTH_FAILED {
        Mt5Error::LoginRefused(message)
    } else if reply["id"].is_null() {
        Mt5Error::Setup(message)
    } else if UNSETTLED.contains(&code) {
        Mt5Error::Uncertain(format!("{message}; hãy xem lại lệnh trên MT5 trước khi gửi lần nữa"))
    } else {
        Mt5Error::Refused { code, message }
    })
}

impl Terminal for Worker {
    fn connect(&mut self, login: &Login) -> Result<AccountInfo, Mt5Error> {
        let number: i64 = login
            .account
            .login
            .parse()
            .map_err(|_| Mt5Error::LoginRefused("số tài khoản MT5 chỉ gồm chữ số".to_owned()))?;
        let timeout_ms = u64::try_from(CONNECT_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
        let request = json!({
            "op": "connect",
            "terminal": login.terminal,
            "login": number,
            "password": login.password.expose(),
            "server": login.account.server,
            "timeout_ms": timeout_ms,
            "hide": true,
        });
        let data = self
            .call(request, CONNECT_TIMEOUT + Duration::from_secs(15))
            .map_err(|error| explain_connect_failure(error, &login.account.server))?;
        serde_json::from_value(data)
            .map_err(|error| Mt5Error::Unreachable(format!("worker trả về tài khoản lạ: {error}")))
    }

    fn poll(&mut self, since_msc: Option<i64>, days: u32) -> Result<Poll, Mt5Error> {
        let data = self.call(json!({"op": "poll", "since_msc": since_msc, "days": days}), CALL_TIMEOUT)?;
        serde_json::from_value(data)
            .map_err(|error| Mt5Error::Unreachable(format!("worker trả về dữ liệu lạ: {error}")))
    }

    fn close(&mut self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.call(json!({"op": "close", "ticket": ticket, "volume": volume}), CALL_TIMEOUT)
            .map(drop)
            .map_err(sent_order)
    }

    fn modify(&mut self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        let request =
            json!({"op": "modify", "ticket": ticket, "sl": levels.sl.unwrap_or(0.0), "tp": levels.tp.unwrap_or(0.0)});
        self.call(request, CALL_TIMEOUT).map(drop).map_err(sent_order)
    }
}

/// A failed login in words the trader can act on. An "IPC timeout" there means the terminal never
/// got ready, which happens when it does not know `server` and opens no connection at all; it is
/// worth trying again (another sign-in may bring the server list), so it is not a refusal.
fn explain_connect_failure(error: Mt5Error, server: &str) -> Mt5Error {
    match error {
        Mt5Error::Refused { code: IPC_TIMEOUT, .. } => Mt5Error::Unreachable(format!(
            "terminal không nối được tới server {server} (IPC timeout). Thường là terminal chưa có server \
             này trong danh sách: hãy mở MetaTrader 5 bạn vẫn dùng cho tài khoản này, đăng nhập một lần để \
             nó tải danh sách server, rồi bấm \"Kết nối lại\""
        )),
        other => other,
    }
}

/// The failure of an order already written to the worker: when the worker did not answer, the
/// terminal may still have carried it out.
fn sent_order(error: Mt5Error) -> Mt5Error {
    match error {
        Mt5Error::Unreachable(why) => Mt5Error::Uncertain(format!(
            "{why}; lệnh có thể đã được thực hiện, hãy xem lại trên MT5 trước khi gửi lần nữa"
        )),
        other => other,
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Asks the worker to log out and stop the terminal, then makes sure the process is gone.
        let quit = format!("{}\n", json!({"id": 0, "op": "quit"}));
        drop(self.stdin.write_all(quit.as_bytes()).and_then(|()| self.stdin.flush()));
        let deadline = Instant::now() + QUIT_GRACE;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

/// Opens a [`Worker`] per login with `python`, running the worker script at `script`.
pub fn opener(python: Python, script: PathBuf) -> Opener {
    let python = Arc::new(python);
    Arc::new(move |_login: &Login| -> Result<Box<dyn Terminal>, Mt5Error> {
        Ok(Box::new(Worker::spawn(&python, &script)?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_login_is_told_apart_from_other_refusals() {
        let refused = reply_data(&json!({"id": 1, "ok": false, "code": -6, "error": "sai mật khẩu"}));
        assert_eq!(refused, Err(Mt5Error::LoginRefused("sai mật khẩu".to_owned())));
        let other = reply_data(&json!({"id": 2, "ok": false, "code": 10018, "error": "thị trường đóng cửa"}));
        assert_eq!(other, Err(Mt5Error::Refused { code: 10018, message: "thị trường đóng cửa".to_owned() }));
        let early = reply_data(&json!({"id": null, "ok": false, "code": -1, "error": "chưa cài gói MetaTrader5"}));
        assert_eq!(early, Err(Mt5Error::Setup("chưa cài gói MetaTrader5".to_owned())));
        assert_eq!(reply_data(&json!({"id": 3, "ok": true, "data": 7})), Ok(json!(7)));
    }

    #[test]
    fn an_order_filled_only_in_part_is_neither_done_nor_refused() {
        for code in UNSETTLED {
            let reply =
                reply_data(&json!({"id": 4, "ok": false, "code": code, "error": "MT5 chỉ khớp 0.05 trên 0.2 lot"}));
            let Err(Mt5Error::Uncertain(message)) = &reply else { panic!("code {code}: {reply:?}") };
            assert!(message.starts_with("MT5 chỉ khớp 0.05 trên 0.2 lot"), "{message}");
        }
    }

    #[test]
    fn a_terminal_that_never_gets_ready_is_explained_and_tried_again() {
        let timeout = Mt5Error::Refused { code: IPC_TIMEOUT, message: "(-10005) IPC timeout".to_owned() };
        let explained = explain_connect_failure(timeout, "Exness-MT5Trial7");
        let Mt5Error::Unreachable(message) = &explained else {
            panic!("a terminal that is not ready is worth trying again: {explained:?}");
        };
        assert!(message.contains("Exness-MT5Trial7") && message.contains("Kết nối lại"), "{message}");
        let refused = Mt5Error::LoginRefused("sai mật khẩu".to_owned());
        assert_eq!(explain_connect_failure(refused.clone(), "S"), refused, "other failures are kept");
    }

    #[test]
    fn newer_python_folders_come_first() {
        let exe = |name: &str| PathBuf::from(format!("C:/Python/{name}/python.exe"));
        assert!(minor_of(&exe("Python313")) > minor_of(&exe("Python39")));
        assert_eq!(minor_of(&exe("Python312-32")), 12);
        assert_eq!(minor_of(&exe("Other")), 0);
    }

    #[test]
    fn the_scripts_are_written_once_and_kept_up_to_date() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let worker = install_scripts(dir.path()).expect("the scripts are written");
        assert_eq!(fs::read_to_string(&worker).expect("the worker is there"), WORKER_SCRIPT);
        fs::write(dir.path().join("tradelens_sync.py"), "old").expect("the file is written");
        install_scripts(dir.path()).expect("the scripts are written again");
        assert_eq!(fs::read_to_string(dir.path().join("tradelens_sync.py")).expect("the script is there"), SYNC_SCRIPT);
    }

    #[test]
    fn a_missing_python_is_not_found() {
        assert_eq!(probe(Path::new("surely-not-a-python-here"), &[]), None);
    }
}
