"""The worker TradeLens runs for one MT5 account: it starts that account's portable terminal
hidden, logs in and answers the app over stdin/stdout, one JSON object per line.

The app (`crates/mt5/src/python.rs`) writes requests and reads replies; nothing else is written
to stdout (diagnostics go to stderr). Requests:

    {"id": 1, "op": "connect", "terminal": "C:/.../terminal64.exe", "login": 123,
     "password": "...", "server": "Broker-Demo", "timeout_ms": 60000, "hide": true}
    {"id": 2, "op": "poll", "since_msc": 0, "days": 365}
    {"id": 3, "op": "close", "ticket": 456, "volume": 0.05}          # volume null: all of it
    {"id": 4, "op": "modify", "ticket": 456, "sl": 2640.0, "tp": 0}  # 0 removes a level
    {"id": 5, "op": "quit"}

Replies: {"id": n, "ok": true, "data": ...} or {"id": n, "ok": false, "code": c, "error": "..."}.
A `poll` answers a DealBatch (`crates/core/src/model.rs`) with the open positions beside it.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from typing import Any

# The app's own Python (python.org's embeddable package) runs isolated: its ._pth leaves the
# script's folder out of sys.path, where tradelens_sync is.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import tradelens_sync as sync  # noqa: E402 - after the path above

# MetaTrader5's error code for a refused login (`RES_E_AUTH_FAILED`): the app stops retrying.
AUTH_FAILED = -6
# What a refused request reports when the package gives no code.
GENERIC = -1
# `TRADE_RETCODE_*` that mean the request went through.
DONE_CODES = {10008, 10009, 10010}
# Slippage allowed when closing at market, in points.
DEVIATION = 20
# `SYMBOL_FILLING_FOK`, `SYMBOL_FILLING_IOC` bits of `symbol_info().filling_mode`.
FILLING_FOK, FILLING_IOC = 1, 2
# How far back a poll after the first one looks again: MT5 times are server times passed through
# as if they were UTC, so a day either side covers any broker's offset.
SLACK_MS = 24 * 3600 * 1000
# `TRADE_RETCODE_CLIENT_DISABLES_AT`: the terminal's Algo Trading is off, so it refuses every order
# the package sends.
ALGO_TRADING_OFF = 10027
# The start configuration (`terminal64.exe /config:<file>`) written beside the account's portable
# terminal: Algo Trading on, and kept on when the login changes the account (`Account=0`), which
# the terminal otherwise turns it off for. Without it closing or moving a level is refused 10027.
START_CONFIG_NAME = "tradelens-start.ini"
START_CONFIG = """[Experts]
AllowLiveTrading=1
AllowDllImport=0
Enabled=1
Account=0
Profile=0
"""


class Failure(Exception):
    """A request that could not be done, with MetaTrader5's code when it gave one."""

    def __init__(self, message: str, code: int = GENERIC) -> None:
        super().__init__(message)
        self.code = code


def last_error(mt5: Any) -> tuple[int, str]:
    error = mt5.last_error()
    if isinstance(error, tuple) and len(error) == 2:  # noqa: PLR2004 - (code, text)
        return int(error[0]), str(error[1])
    return GENERIC, str(error)


# --- the terminal's windows --------------------------------------------------------------------


class Leash:
    """The account's terminal kept hidden and tied to this worker (Windows only; nothing elsewhere).

    The terminal runs inside a Job Object that ends it when the job's last handle closes, which is
    when this process ends, even when the app kills it: no terminal is left running unseen. Its
    windows are found by the program's path, not by the pid of the process this worker started: a
    terminal already running in that folder takes over a second start and the started one exits.
    """

    def __init__(self, terminal: str) -> None:
        # Started by its path as given: `initialize` finds a running terminal by the path it was
        # started with, letter case included. Started lowercased (`normcase`), the terminal was
        # not found, `initialize` started a second one that handed over to it and exited, and the
        # login failed with "IPC timeout" or "IPC send failed".
        self.program = os.path.abspath(terminal)
        # For comparing with the programs of running processes (`image_of`), which are normcased.
        self.terminal = os.path.normcase(self.program)
        self.tied: set[int] = set()
        self.job: Any = None
        if sys.platform == "win32":
            try:
                self.job = kill_on_close_job()
            except OSError as err:
                print(f"tradelens worker: no job object: {err}", file=sys.stderr)

    def start(self) -> None:
        """Starts the terminal in portable mode with its window hidden and Algo Trading on, so that
        `initialize` attaches to it instead of showing a new one."""
        if sys.platform != "win32":
            return
        info = subprocess.STARTUPINFO()  # type: ignore[attr-defined]
        info.dwFlags |= subprocess.STARTF_USESHOWWINDOW  # type: ignore[attr-defined]
        info.wShowWindow = SW_HIDE
        try:
            command = start_command(self.program, write_start_config(self.program))
            process = subprocess.Popen(command, startupinfo=info)  # noqa: S603 - the app's own copy
        except OSError as err:
            msg = f"không chạy được terminal MT5 ({self.program}): {err}"
            raise Failure(msg) from err
        self.tie(process.pid)

    def tie(self, pid: int) -> None:
        if self.job is None or pid in self.tied:
            return
        try:
            assign_to_job(self.job, pid)
            self.tied.add(pid)
        except OSError as err:
            print(f"tradelens worker: terminal {pid} not tied: {err}", file=sys.stderr)

    def tend(self) -> int:
        """Hides the terminal's visible windows (it shows its main window and dialogs by itself) and
        ties each of its processes to the job. Returns how many windows it hid; never fails a request."""
        if sys.platform != "win32":
            return 0
        try:
            hidden = 0
            for hwnd, pid in visible_windows_of(self.terminal):
                hide_window(hwnd)
                self.tie(pid)
                hidden += 1
        except OSError as err:
            print(f"tradelens worker: windows not hidden: {err}", file=sys.stderr)
            return 0
        return hidden


def write_start_config(terminal: str) -> str:
    """Writes `START_CONFIG` into the folder of the account's portable `terminal` (the app's own
    copy) and returns its path."""
    path = os.path.join(os.path.dirname(os.path.abspath(terminal)), START_CONFIG_NAME)
    try:
        with open(path, "w", encoding="ascii", newline="\r\n") as file:
            file.write(START_CONFIG)
    except OSError as err:
        msg = f"không ghi được cấu hình khởi động MT5 ({path}): {err}"
        raise Failure(msg) from err
    return path


def start_command(terminal: str, config: str) -> list[str]:
    """The command line of the account's terminal: portable, with the start configuration."""
    return [terminal, "/portable", f"/config:{config}"]


# Win32 constants (winnt.h, winbase.h, winuser.h).
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS = 9
PROCESS_TERMINATE = 0x0001
PROCESS_SET_QUOTA = 0x0100
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SW_HIDE = 0


def win32() -> Any:
    """`ctypes`, `wintypes` and the kernel32 / user32 functions the leash calls, typed for 64-bit
    handles."""
    import ctypes  # noqa: PLC0415 - Windows only
    from ctypes import wintypes  # noqa: PLC0415

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)  # type: ignore[attr-defined]
    user32 = ctypes.WinDLL("user32", use_last_error=True)  # type: ignore[attr-defined]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    kernel32.QueryFullProcessImageNameW.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPWSTR,
        ctypes.POINTER(wintypes.DWORD),
    ]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
    return ctypes, wintypes, kernel32, user32


def kill_on_close_job() -> Any:
    """A Job Object whose processes end when its last handle closes."""
    ctypes, wintypes, kernel32, _ = win32()

    class BasicLimits(ctypes.Structure):
        _fields_ = [  # noqa: RUF012 - ctypes layout
            ("PerProcessUserTimeLimit", ctypes.c_int64),
            ("PerJobUserTimeLimit", ctypes.c_int64),
            ("LimitFlags", wintypes.DWORD),
            ("MinimumWorkingSetSize", ctypes.c_size_t),
            ("MaximumWorkingSetSize", ctypes.c_size_t),
            ("ActiveProcessLimit", wintypes.DWORD),
            ("Affinity", ctypes.c_size_t),
            ("PriorityClass", wintypes.DWORD),
            ("SchedulingClass", wintypes.DWORD),
        ]

    class IoCounters(ctypes.Structure):
        _fields_ = [  # noqa: RUF012 - ctypes layout
            ("ReadOperationCount", ctypes.c_uint64),
            ("WriteOperationCount", ctypes.c_uint64),
            ("OtherOperationCount", ctypes.c_uint64),
            ("ReadTransferCount", ctypes.c_uint64),
            ("WriteTransferCount", ctypes.c_uint64),
            ("OtherTransferCount", ctypes.c_uint64),
        ]

    class ExtendedLimits(ctypes.Structure):
        _fields_ = [  # noqa: RUF012 - ctypes layout
            ("BasicLimitInformation", BasicLimits),
            ("IoInfo", IoCounters),
            ("ProcessMemoryLimit", ctypes.c_size_t),
            ("JobMemoryLimit", ctypes.c_size_t),
            ("PeakProcessMemoryUsed", ctypes.c_size_t),
            ("PeakJobMemoryUsed", ctypes.c_size_t),
        ]

    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedLimits()
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(
        job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS, ctypes.byref(limits), ctypes.sizeof(limits)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    return job


def assign_to_job(job: Any, pid: int) -> None:
    ctypes, _, kernel32, _ = win32()
    process = kernel32.OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, False, pid)  # noqa: FBT003 - Win32 BOOL
    if not process:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        if not kernel32.AssignProcessToJobObject(job, process):
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        kernel32.CloseHandle(process)


def image_of(pid: int) -> str | None:
    """The program a process runs, as a normalised path; `None` when it cannot be read."""
    ctypes, wintypes, kernel32, _ = win32()
    process = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)  # noqa: FBT003 - Win32 BOOL
    if not process:
        return None
    try:
        size = wintypes.DWORD(32_768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not kernel32.QueryFullProcessImageNameW(process, 0, buffer, ctypes.byref(size)):
            return None
        return os.path.normcase(os.path.abspath(buffer.value))
    finally:
        kernel32.CloseHandle(process)


def visible_windows_of(program: str) -> list[tuple[int, int]]:
    """The visible top-level windows of the processes running `program`: (window, pid) pairs."""
    ctypes, wintypes, _, user32 = win32()
    found: list[tuple[int, int]] = []
    images: dict[int, str | None] = {}

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)  # type: ignore[attr-defined]
    def each(hwnd: int, _param: int) -> bool:
        if not user32.IsWindowVisible(hwnd):
            return True
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        pid = int(owner.value)
        if pid not in images:
            images[pid] = image_of(pid)
        if images[pid] == program:
            found.append((hwnd, pid))
        return True

    user32.EnumWindows(each, 0)
    return found


def hide_window(hwnd: int) -> None:
    _, _, _, user32 = win32()
    user32.ShowWindow(hwnd, SW_HIDE)


# --- positions and orders ----------------------------------------------------------------------


def position_json(position: Any) -> dict[str, Any]:
    """An open position as the app's `Position` reads it."""
    return {
        "ticket": int(position.ticket),
        "time_msc": int(getattr(position, "time_msc", 0) or int(position.time) * 1000),
        "symbol": position.symbol,
        "side": "buy" if int(position.type) == sync.DEAL_BUY else "sell",
        "volume": sync.num(position.volume),
        "open_price": sync.num(position.price_open),
        "current_price": sync.num(position.price_current),
        "sl": sync.num(position.sl),
        "tp": sync.num(position.tp),
        "profit": sync.num(position.profit),
        "swap": sync.num(position.swap),
        "magic": int(position.magic),
        "comment": position.comment,
    }


def filling(mt5: Any, symbol: str) -> int:
    """The filling mode the symbol accepts for a market order."""
    info = mt5.symbol_info(symbol)
    mode = int(getattr(info, "filling_mode", 0) or 0) if info is not None else 0
    if mode & FILLING_FOK:
        return int(mt5.ORDER_FILLING_FOK)
    if mode & FILLING_IOC:
        return int(mt5.ORDER_FILLING_IOC)
    return int(mt5.ORDER_FILLING_RETURN)


def require_algo_trading(mt5: Any) -> None:
    """Refuses before sending when the terminal's Algo Trading is off: the order would only come
    back as 10027, a code the trader cannot act on."""
    info = mt5.terminal_info()
    if info is not None and not bool(getattr(info, "trade_allowed", True)):
        msg = (
            "Terminal MT5 của tài khoản đang tắt Algo Trading nên không đóng hay sửa lệnh được. "
            "Đóng TradeLens rồi mở lại để terminal khởi động lại với Algo Trading bật."
        )
        raise Failure(msg, ALGO_TRADING_OFF)


def sent(mt5: Any, request: dict[str, Any]) -> dict[str, Any]:
    """Sends `request`; the result's order and deal, or the refusal as a `Failure`."""
    require_algo_trading(mt5)
    result = mt5.order_send(request)
    if result is None:
        code, text = last_error(mt5)
        msg = f"MT5 không nhận lệnh: ({code}) {text}"
        raise Failure(msg, code)
    if int(result.retcode) not in DONE_CODES:
        msg = f"MT5 từ chối lệnh: {result.retcode} {getattr(result, 'comment', '')}".strip()
        raise Failure(msg, int(result.retcode))
    return {"order": int(getattr(result, "order", 0)), "deal": int(getattr(result, "deal", 0))}


def open_position(mt5: Any, ticket: int) -> Any:
    found = mt5.positions_get(ticket=ticket) or ()
    if not found:
        msg = f"không còn lệnh {ticket} đang mở"
        raise Failure(msg)
    return found[0]


def close(mt5: Any, ticket: int, volume: float | None) -> dict[str, Any]:
    """Closes `volume` lots of position `ticket` at market (all of it when `None`)."""
    position = open_position(mt5, ticket)
    lots = float(position.volume) if volume is None else float(volume)
    if not 0 < lots <= float(position.volume) + 1e-9:
        msg = f"khối lượng đóng {lots} không hợp lệ (lệnh đang có {position.volume})"
        raise Failure(msg)
    tick = mt5.symbol_info_tick(position.symbol)
    if tick is None:
        code, text = last_error(mt5)
        msg = f"không có giá của {position.symbol}: ({code}) {text}"
        raise Failure(msg, code)
    buy = int(position.type) == sync.DEAL_BUY
    return sent(
        mt5,
        {
            "action": mt5.TRADE_ACTION_DEAL,
            "position": ticket,
            "symbol": position.symbol,
            "volume": lots,
            "type": mt5.ORDER_TYPE_SELL if buy else mt5.ORDER_TYPE_BUY,
            "price": tick.bid if buy else tick.ask,
            "deviation": DEVIATION,
            "type_filling": filling(mt5, position.symbol),
            "type_time": mt5.ORDER_TIME_GTC,
            "comment": "TradeLens",
        },
    )


def modify(mt5: Any, ticket: int, sl: float, tp: float) -> dict[str, Any]:
    """Moves the stop and target of position `ticket` (0 removes one)."""
    position = open_position(mt5, ticket)
    return sent(
        mt5,
        {"action": mt5.TRADE_ACTION_SLTP, "position": ticket, "symbol": position.symbol, "sl": float(sl), "tp": float(tp)},
    )


# --- the session -------------------------------------------------------------------------------


class Session:
    """One account's terminal, from `connect` to `quit`."""

    def __init__(self, mt5: Any) -> None:
        self.mt5 = mt5
        self.leash: Leash | None = None
        self.offset_min: int | None = None
        self.connected = False
        self.hide = True

    def connect(self, request: dict[str, Any]) -> dict[str, Any]:
        terminal = str(request["terminal"])
        self.hide = bool(request.get("hide", True))
        if self.hide and self.leash is None:
            self.leash = Leash(terminal)
            self.leash.start()
        ok = self.mt5.initialize(
            terminal,
            login=int(request["login"]),
            password=str(request["password"]),
            server=str(request["server"]),
            timeout=int(request.get("timeout_ms", 60_000)),
            portable=True,
        )
        if not ok:
            code, text = last_error(self.mt5)
            msg = f"không đăng nhập được MT5: ({code}) {text}"
            raise Failure(msg, code)
        self.connected = True
        self.hide_terminal()
        self.offset_min = sync.server_offset_min(self.mt5)
        return sync.account_info(self.mt5, self.offset_min)

    def hide_terminal(self) -> None:
        if self.hide and self.leash is not None:
            self.leash.tend()

    def poll(self, request: dict[str, Any]) -> dict[str, Any]:
        self.require_connected()
        self.hide_terminal()
        since = int(request.get("since_msc") or 0)
        if since > 0:
            start = datetime.fromtimestamp((since - SLACK_MS) / 1000, timezone.utc)
            window = (start, datetime.now(timezone.utc) + timedelta(days=1))
        else:
            window = sync.history_window(int(request.get("days", 365)))
        if self.offset_min is None:
            self.offset_min = sync.server_offset_min(self.mt5)
        deals = sync.read_deals(self.mt5, window)
        levels = sync.Levels(self.mt5, window)
        return {
            "account": sync.account_info(self.mt5, self.offset_min),
            "deals": [sync.deal_json(d, levels) for d in deals],
            "positions": [position_json(p) for p in self.open_positions()],
        }

    def open_positions(self) -> tuple[Any, ...]:
        """The open positions; a failed read is an error, never "none open" (the app would take every
        position for a new one at the next poll)."""
        positions = self.mt5.positions_get()
        if positions is None:
            code, text = last_error(self.mt5)
            msg = f"không đọc được lệnh đang mở: ({code}) {text}"
            raise Failure(msg, code)
        return tuple(positions)

    def require_connected(self) -> None:
        if not self.connected:
            msg = "chưa đăng nhập MT5"
            raise Failure(msg)

    def quit(self) -> None:
        if self.connected:
            self.mt5.shutdown()
            self.connected = False
        # The leash's job ends the terminal once this process lets go of it.
        self.leash = None

    def handle(self, request: dict[str, Any]) -> Any:
        op = request.get("op")
        if op == "connect":
            return self.connect(request)
        if op == "poll":
            return self.poll(request)
        if op == "close":
            self.require_connected()
            volume = request.get("volume")
            return close(self.mt5, int(request["ticket"]), None if volume is None else float(volume))
        if op == "modify":
            self.require_connected()
            return modify(self.mt5, int(request["ticket"]), float(request.get("sl") or 0), float(request.get("tp") or 0))
        msg = f"không hiểu yêu cầu {op!r}"
        raise Failure(msg)


def reply(out: Any, message: dict[str, Any]) -> None:
    out.write(json.dumps(message, ensure_ascii=False) + "\n")
    out.flush()


def serve(mt5: Any, lines: Any, out: Any) -> int:
    """Answers each request line until `quit` or the end of input (the app closed)."""
    session = Session(mt5)
    try:
        for line in lines:
            if not line.strip():
                continue
            try:
                request = json.loads(line)
            except json.JSONDecodeError as err:
                reply(out, {"id": None, "ok": False, "code": GENERIC, "error": f"yêu cầu không phải JSON: {err}"})
                continue
            if request.get("op") == "quit":
                reply(out, {"id": request.get("id"), "ok": True, "data": None})
                break
            try:
                reply(out, {"id": request.get("id"), "ok": True, "data": session.handle(request)})
            except Failure as err:
                reply(out, {"id": request.get("id"), "ok": False, "code": err.code, "error": str(err)})
            except Exception as err:  # noqa: BLE001 - one bad request must not stop the worker
                reply(out, {"id": request.get("id"), "ok": False, "code": GENERIC, "error": str(err)})
    finally:
        session.quit()
    return 0


def main() -> int:
    # The app reads and writes UTF-8; an isolated Python ignores PYTHONIOENCODING and would use the
    # console's code page on a pipe.
    for stream in (sys.stdin, sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    try:
        import MetaTrader5 as mt5  # noqa: PLC0415 - Windows-only
    except ImportError as err:
        reply(sys.stdout, {"id": None, "ok": False, "code": GENERIC, "error": f"chưa cài gói MetaTrader5: {err}"})
        return 1
    start = time.monotonic()
    print(f"tradelens worker: MetaTrader5 {getattr(mt5, '__version__', '?')}", file=sys.stderr)
    code = serve(mt5, sys.stdin, sys.stdout)
    print(f"tradelens worker: done after {time.monotonic() - start:.0f}s", file=sys.stderr)
    return code


if __name__ == "__main__":
    sys.exit(main())
