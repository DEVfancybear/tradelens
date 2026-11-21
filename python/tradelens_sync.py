"""Send MT5 deal history to TradeLens (or to a JSON file to import).

Uses the official MetaTrader5 package, so it runs on Windows with the terminal open.

    pip install -r requirements.txt
    python tradelens_sync.py --token <token> --days 365
    python tradelens_sync.py --token <token> --watch 30   # keep syncing every 30 seconds
    python tradelens_sync.py --out deals.json             # then import the file in the app

Only deals the app has not acknowledged yet are sent: a cursor (the newest acknowledged
`time_msc` plus the tickets of the last hour) is kept per account in
`~/.tradelens/cursor-<login>.json`. A failed request leaves the cursor where it was, so the
next run resends exactly what is missing; the app keys deals by (account, ticket), so
resending is harmless.

Payload: docs/sync-api.md (DealBatch).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Iterable, Sequence
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

DEFAULT_URL = "http://127.0.0.1:7878"
TOKEN_HEADER = "X-TradeLens-Token"

BATCH_SIZE = 200
OVERLAP_SEC = 3600  # re-read this far behind the cursor to catch deals that reach history late
MAX_BACKOFF_SEC = 60

DEAL_TYPES = {
    0: "buy",
    1: "sell",
    2: "balance",
    3: "credit",
    4: "charge",
    5: "correction",
    6: "bonus",
    7: "commission",
    8: "commission",
    9: "commission",
    10: "commission",
    11: "commission",
}
DEAL_ENTRIES = {0: "in", 1: "out", 2: "inout", 3: "out_by"}
DEAL_REASONS = {
    0: "client",
    1: "mobile",
    2: "web",
    3: "expert",
    4: "sl",
    5: "tp",
    6: "so",
    7: "rollover",
    8: "vmargin",
    9: "split",
}
DEAL_BUY, DEAL_SELL = 0, 1
ENTRY_IN = 0


class BridgeError(Exception):
    """The bridge did not accept a request. `status` is None when it could not be reached."""

    def __init__(self, message: str, status: int | None = None, *, retryable: bool = True) -> None:
        super().__init__(message)
        self.status = status
        self.retryable = retryable


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", default=DEFAULT_URL, help="bridge address (default %(default)s)")
    parser.add_argument("--token", help="token from the app's Data sources screen")
    parser.add_argument("--days", type=int, default=90, help="how far back to read the first time (default %(default)s)")
    parser.add_argument("--out", help="write the batch to this JSON file instead of posting it")
    parser.add_argument("--terminal", help="path to terminal64.exe when several terminals are installed")
    parser.add_argument("--watch", type=int, metavar="SECONDS", help="keep running and sync every SECONDS")
    parser.add_argument("--full", action="store_true", help="ignore the cursor and resend the last --days days")
    parser.add_argument("--retries", type=int, default=3, help="retries per request when the app is unreachable")
    parser.add_argument("--state-dir", help="where cursors are kept (default ~/.tradelens)")
    parser.add_argument("--server-offset-min", type=int, help="broker server offset from UTC in minutes, if auto-detect fails")
    return parser.parse_args(argv)


def num(value: Any) -> float:
    """A float for JSON: MT5 doubles carry binary noise past 8 decimals."""
    return round(float(value), 8)


# --- account ---------------------------------------------------------------------------------


def server_offset_min(mt5: Any, now: datetime | None = None) -> int | None:
    """Broker server offset from UTC in minutes, from the newest tick of the Market Watch.

    Needs a live market (a weekend tick is hours old): the result must be a multiple of 15
    minutes within 2 minutes of the raw difference, else `None`.
    """
    now = now or datetime.now(timezone.utc)
    newest = 0
    for symbol in list(mt5.symbols_get() or ())[:200]:
        if not getattr(symbol, "visible", False):
            continue
        tick = mt5.symbol_info_tick(symbol.name)
        if tick is not None and tick.time > newest:
            newest = tick.time
    if not newest:
        return None
    diff = newest - now.timestamp()
    minutes = round(diff / 900) * 15
    if abs(diff - minutes * 60) > 120 or not -12 * 60 <= minutes <= 14 * 60:
        return None
    return minutes


def account_info(mt5: Any, offset_min: int | None = None) -> dict[str, Any]:
    """The `account` part of the batch."""
    info = mt5.account_info()
    if info is None:
        msg = f"không đọc được tài khoản MT5: {mt5.last_error()}"
        raise RuntimeError(msg)
    return {
        "login": str(info.login),  # the app keeps logins as text
        "server": info.server,
        "broker": info.company,
        "name": info.name,
        "currency": info.currency,
        "balance": num(info.balance),
        "equity": num(info.equity),
        "server_offset_min": offset_min if offset_min is not None else server_offset_min(mt5),
    }


# --- deals -----------------------------------------------------------------------------------


class Levels:
    """Stops, targets and tick values, looked up lazily once per run."""

    def __init__(self, mt5: Any, window: tuple[datetime, datetime]) -> None:
        self.mt5 = mt5
        self.window = window
        self.orders: dict[int, Any] = {}
        self.positions: dict[int, Any] | None = None
        self.fetched_positions: set[int] = set()
        self.symbols: dict[str, Any] = {}
        for order in mt5.history_orders_get(*window) or ():
            self.orders[order.ticket] = order

    def order(self, ticket: int) -> Any | None:
        """A history order by ticket; positions opened before the window are fetched by position."""
        return self.orders.get(ticket)

    def fetch_position_orders(self, position_id: int) -> None:
        if position_id in self.fetched_positions or position_id == 0:
            return
        self.fetched_positions.add(position_id)
        for order in self.mt5.history_orders_get(position=position_id) or ():
            self.orders.setdefault(order.ticket, order)

    def open_position(self, position_id: int) -> Any | None:
        if self.positions is None:
            self.positions = {p.ticket: p for p in self.mt5.positions_get() or ()}
        return self.positions.get(position_id)

    def symbol(self, name: str) -> Any | None:
        if name not in self.symbols:
            self.symbols[name] = self.mt5.symbol_info(name) if name else None
        return self.symbols[name]

    def stops(self, deal: Any) -> tuple[float, float]:
        """SL/TP of a deal: its order, then the open position's current levels, then (for
        exits) the order that opened the position (its ticket is the position id)."""
        self.fetch_position_orders(deal.position_id)
        sl = tp = 0.0
        own = self.order(deal.order)
        if own is not None:
            sl, tp = float(own.sl), float(own.tp)
        if (sl <= 0 or tp <= 0) and (position := self.open_position(deal.position_id)) is not None:
            sl = sl if sl > 0 else float(position.sl)
            tp = tp if tp > 0 else float(position.tp)
        if deal.entry != ENTRY_IN and (sl <= 0 or tp <= 0) and (opening := self.order(deal.position_id)) is not None:
            sl = sl if sl > 0 else float(opening.sl)
            tp = tp if tp > 0 else float(opening.tp)
        return sl, tp

    def risk(self, deal: Any, sl: float) -> float:
        """Money at risk at the stop: |price - sl| / tick size * tick value * volume."""
        info = self.symbol(deal.symbol)
        if sl <= 0 or info is None or info.trade_tick_size <= 0:
            return 0.0
        tick_value = getattr(info, "trade_tick_value_loss", 0) or info.trade_tick_value
        return abs(deal.price - sl) / info.trade_tick_size * tick_value * deal.volume


def deal_json(deal: Any, levels: Levels | None = None) -> dict[str, Any]:
    """One TradeDeal as the bridge expects it."""
    sl = tp = risk = 0.0
    if levels is not None and deal.type in (DEAL_BUY, DEAL_SELL):
        sl, tp = levels.stops(deal)
        if deal.entry == ENTRY_IN:
            risk = levels.risk(deal, sl)
    return {
        "ticket": int(deal.ticket),
        "order": int(deal.order),
        "position_id": int(deal.position_id),
        "time_msc": int(deal.time_msc),  # server time, as MT5 reports it
        "symbol": deal.symbol,
        "type": DEAL_TYPES.get(deal.type, "other"),
        "entry": DEAL_ENTRIES.get(deal.entry, ""),
        "volume": num(deal.volume),
        "price": num(deal.price),
        "commission": num(deal.commission),
        "swap": num(deal.swap),
        "fee": num(deal.fee),
        "profit": num(deal.profit),
        "magic": int(deal.magic),
        "comment": deal.comment,
        "sl": num(sl),
        "tp": num(tp),
        "risk": num(risk),
        "reason": DEAL_REASONS.get(getattr(deal, "reason", -1), ""),
    }


def history_window(days: int, cursor_msc: int = 0, now: datetime | None = None) -> tuple[datetime, datetime]:
    """The `history_deals_get` range. MT5 times are server times that the package passes through
    as if they were UTC, so the cursor is turned back into a datetime the same way."""
    end = (now or datetime.now(timezone.utc)) + timedelta(days=1)
    if cursor_msc > 0:
        start = datetime.fromtimestamp(cursor_msc / 1000 - OVERLAP_SEC, tz=timezone.utc)
    else:
        start = end - timedelta(days=max(days, 0) + 1)
    return start, end


def read_deals(mt5: Any, window: tuple[datetime, datetime]) -> list[Any]:
    deals = mt5.history_deals_get(*window)
    if deals is None:
        code, text = mt5.last_error()
        if code != 1:
            msg = f"không đọc được lịch sử MT5: ({code}) {text}"
            raise RuntimeError(msg)
        deals = ()
    return sorted(deals, key=lambda d: (d.time_msc, d.ticket))


def read_batch(mt5: Any, days: int, offset_min: int | None = None) -> dict[str, Any]:
    """Everything of the last `days` days, ignoring the cursor (the `--out` file)."""
    window = history_window(days)
    deals = read_deals(mt5, window)
    levels = Levels(mt5, window)
    return {"account": account_info(mt5, offset_min), "deals": [deal_json(d, levels) for d in deals]}


# --- cursor ----------------------------------------------------------------------------------


class Cursor:
    """What the app has acknowledged for one account, kept in a small JSON file."""

    def __init__(self, path: Path | None) -> None:
        self.path = path
        self.msc = 0
        self.sent: dict[int, int] = {}  # ticket -> time_msc, only the last OVERLAP_SEC
        self.offset_min: int | None = None  # last offset that was measured on a live market
        if path is not None and path.exists():
            try:
                data = json.loads(path.read_text(encoding="utf-8"))
                self.msc = int(data.get("cursor_msc", 0))
                self.sent = {int(k): int(v) for k, v in data.get("sent", {}).items()}
                self.offset_min = data.get("server_offset_min")
            except (OSError, ValueError, AttributeError):
                self.msc, self.sent = 0, {}  # unreadable state: send everything again, harmlessly

    def is_new(self, ticket: int, msc: int) -> bool:
        if msc > self.msc:
            return True
        if msc < self.msc - OVERLAP_SEC * 1000:
            return False
        return ticket not in self.sent

    def ack(self, deals: Iterable[Any]) -> None:
        for deal in deals:
            self.sent[int(deal.ticket)] = int(deal.time_msc)
            self.msc = max(self.msc, int(deal.time_msc))
        limit = self.msc - OVERLAP_SEC * 1000
        self.sent = {t: m for t, m in self.sent.items() if m >= limit}
        self.save()

    def save(self) -> None:
        if self.path is None:
            return
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.path.with_suffix(".tmp")
        payload = {"cursor_msc": self.msc, "sent": self.sent, "server_offset_min": self.offset_min}
        temporary.write_text(json.dumps(payload), encoding="utf-8")
        os.replace(temporary, self.path)  # atomic: a crash never leaves half a cursor


def cursor_path(login: str, state_dir: str | None) -> Path:
    base = Path(state_dir) if state_dir else Path.home() / ".tradelens"
    safe = "".join(c for c in login if c.isalnum())
    return base / f"cursor-{safe}.json"


# --- HTTP ------------------------------------------------------------------------------------

PostFn = Callable[[str, dict[str, Any]], dict[str, Any]]


def post(url: str, token: str, batch: dict[str, Any], path: str = "/api/v1/deals") -> dict[str, Any]:
    """POST a JSON body; raises BridgeError (never urllib's errors) so callers decide on retries."""
    request = urllib.request.Request(
        f"{url}{path}",
        data=json.dumps(batch, ensure_ascii=False).encode("utf-8"),
        headers={"Content-Type": "application/json", TOKEN_HEADER: token},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as err:
        detail = err.read().decode("utf-8", "replace")
        if err.code == 401:
            msg = "app từ chối token (401): --token phải trùng token ở màn Nguồn dữ liệu của app"
            raise BridgeError(msg, 401, retryable=False) from err
        raise BridgeError(f"app trả lỗi {err.code}: {detail}", err.code, retryable=err.code >= 500) from err
    except (urllib.error.URLError, TimeoutError, OSError, ValueError) as err:
        msg = f"không kết nối được app tại {url} ({err}) — app đã mở và cổng đúng chưa?"
        raise BridgeError(msg) from err


def with_retries(call: Callable[[], Any], retries: int, sleep: Callable[[float], None] = time.sleep) -> Any:
    """Run `call`, retrying retryable BridgeErrors with 2, 4, 8... seconds between attempts."""
    wait = 2.0
    for attempt in range(retries + 1):
        try:
            return call()
        except BridgeError as err:
            if not err.retryable or attempt == retries:
                raise
            print(f"TradeLens: {err}; thử lại sau {wait:.0f} giây", file=sys.stderr)
            sleep(wait)
            wait = min(wait * 2, MAX_BACKOFF_SEC)
    msg = "unreachable"
    raise AssertionError(msg)


def deliver(send: PostFn, account: dict[str, Any], chunk: Sequence[Any], levels: Levels) -> None:
    """Send one chunk of deals; if the app answers 400, send them one by one and skip the bad ones."""
    payloads = [deal_json(d, levels) for d in chunk]
    try:
        send("/api/v1/deals", {"account": account, "deals": payloads})
    except BridgeError as err:
        if err.status != 400:
            raise
        for deal, payload in zip(chunk, payloads, strict=True):
            try:
                send("/api/v1/deals", {"account": account, "deals": [payload]})
            except BridgeError as single:
                if single.status != 400:
                    raise
                print(f"TradeLens: app từ chối deal {deal.ticket} (JSON sai), bỏ qua: {single}", file=sys.stderr)


def sync_once(mt5: Any, send: PostFn, cursor: Cursor, days: int, *, full: bool = False, offset_min: int | None = None) -> int:
    """Send the deals the app has not acknowledged; returns how many were sent.

    The cursor moves after each acknowledged batch, so an error halfway loses nothing.
    """
    if full:
        cursor.msc, cursor.sent = 0, {}
    window = history_window(days, cursor.msc)
    fresh = [d for d in read_deals(mt5, window) if cursor.is_new(int(d.ticket), int(d.time_msc))]
    measured = offset_min if offset_min is not None else server_offset_min(mt5)
    if measured is not None:
        cursor.offset_min = measured
    account = account_info(mt5, measured if measured is not None else cursor.offset_min)
    if not fresh:
        send("/api/v1/account", {"account": account})  # balance, equity and "still alive"
        return 0
    levels = Levels(mt5, window)
    for start in range(0, len(fresh), BATCH_SIZE):
        chunk = fresh[start : start + BATCH_SIZE]
        deliver(send, account, chunk, levels)
        cursor.ack(chunk)
    return len(fresh)


def run_sync(mt5: Any, args: argparse.Namespace) -> int:
    info = mt5.account_info()
    login = str(info.login) if info is not None else "unknown"
    cursor = Cursor(cursor_path(login, args.state_dir))
    full = args.full

    def send(path: str, body: dict[str, Any]) -> dict[str, Any]:
        return with_retries(lambda: post(args.url, args.token, body, path), args.retries)

    while True:
        try:
            sent = sync_once(mt5, send, cursor, args.days, full=full, offset_min=args.server_offset_min)
            full = False
            print(f"TradeLens: đã gửi {sent} deal mới")
        except BridgeError as err:
            print(f"TradeLens: {err}", file=sys.stderr)
            if not args.watch or not err.retryable:
                return 1
        if not args.watch:
            return 0
        time.sleep(args.watch)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if not args.out and not args.token:
        print("cần --token (gửi vào app) hoặc --out (ghi file)", file=sys.stderr)
        return 2
    import MetaTrader5 as mt5  # noqa: PLC0415 - Windows-only, imported after argument checks

    if not (mt5.initialize(args.terminal) if args.terminal else mt5.initialize()):
        print(f"không kết nối được MT5: {mt5.last_error()}", file=sys.stderr)
        return 1
    try:
        if args.out:
            batch = read_batch(mt5, args.days, args.server_offset_min)
            with open(args.out, "w", encoding="utf-8") as file:
                json.dump(batch, file, ensure_ascii=False, indent=2)
            print(f"đã ghi {len(batch['deals'])} deal vào {args.out}")
            return 0
        return run_sync(mt5, args)
    except RuntimeError as err:
        print(f"TradeLens: {err}", file=sys.stderr)
        return 1
    finally:
        mt5.shutdown()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
