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
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from typing import Any

import tradelens_sync as sync

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


def start_hidden(terminal: str) -> subprocess.Popen[bytes] | None:
    """Starts `terminal` in portable mode with its window hidden, so that `initialize` attaches to
    it instead of showing a new one. `None` off Windows (tests)."""
    if sys.platform != "win32":
        return None
    info = subprocess.STARTUPINFO()  # type: ignore[attr-defined]
    info.dwFlags |= subprocess.STARTF_USESHOWWINDOW  # type: ignore[attr-defined]
    info.wShowWindow = 0  # SW_HIDE
    return subprocess.Popen([terminal, "/portable"], startupinfo=info)  # noqa: S603 - the app's own copy


def hide_windows(pid: int) -> int:
    """Hides every visible top-level window of process `pid` (the terminal shows its main window
    and dialogs on its own once started). Returns how many it hid; 0 off Windows."""
    if sys.platform != "win32" or pid <= 0:
        return 0
    import ctypes  # noqa: PLC0415 - Windows only
    from ctypes import wintypes  # noqa: PLC0415

    user32 = ctypes.windll.user32  # type: ignore[attr-defined]
    hidden = 0

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)  # type: ignore[attr-defined]
    def each(hwnd: int, _param: int) -> bool:
        nonlocal hidden
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            user32.ShowWindow(hwnd, 0)  # SW_HIDE
            hidden += 1
        return True

    user32.EnumWindows(each, 0)
    return hidden


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


def sent(mt5: Any, request: dict[str, Any]) -> dict[str, Any]:
    """Sends `request`; the result's order and deal, or the refusal as a `Failure`."""
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
        self.process: subprocess.Popen[bytes] | None = None
        self.offset_min: int | None = None
        self.connected = False
        self.hide = True

    def connect(self, request: dict[str, Any]) -> dict[str, Any]:
        terminal = str(request["terminal"])
        self.hide = bool(request.get("hide", True))
        if self.hide and self.process is None:
            self.process = start_hidden(terminal)
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
        if self.hide and self.process is not None:
            hide_windows(self.process.pid)

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
            "positions": [position_json(p) for p in self.mt5.positions_get() or ()],
        }

    def require_connected(self) -> None:
        if not self.connected:
            msg = "chưa đăng nhập MT5"
            raise Failure(msg)

    def quit(self) -> None:
        if self.connected:
            self.mt5.shutdown()
            self.connected = False
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
        self.process = None

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
            except (RuntimeError, KeyError, ValueError, TypeError) as err:
                reply(out, {"id": request.get("id"), "ok": False, "code": GENERIC, "error": str(err)})
    finally:
        session.quit()
    return 0


def main() -> int:
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
