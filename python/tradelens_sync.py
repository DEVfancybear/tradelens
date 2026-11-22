"""Write the MT5 deal history to a JSON file that TradeLens imports (Nguồn dữ liệu → Import).

Uses the official MetaTrader5 package, so it runs on Windows with the terminal open.

    pip install -r requirements.txt
    python tradelens_sync.py --days 365 --out deals.json

The file is a DealBatch (`crates/core/src/model.rs`); importing it twice is harmless, since the
app keys deals by (account, ticket).
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timedelta, timezone
from typing import Any

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


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, help="the JSON file to write")
    parser.add_argument("--days", type=int, default=90, help="how far back to read (default %(default)s)")
    parser.add_argument("--terminal", help="path to terminal64.exe when several terminals are installed")
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


def history_window(days: int, now: datetime | None = None) -> tuple[datetime, datetime]:
    """The `history_deals_get` range: the last `days` days, with a day of slack on both ends
    because MT5 times are server times that the package passes through as if they were UTC."""
    end = (now or datetime.now(timezone.utc)) + timedelta(days=1)
    return end - timedelta(days=max(days, 0) + 1), end


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
    """Everything of the last `days` days, as the `--out` file holds it."""
    window = history_window(days)
    deals = read_deals(mt5, window)
    levels = Levels(mt5, window)
    return {"account": account_info(mt5, offset_min), "deals": [deal_json(d, levels) for d in deals]}


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    import MetaTrader5 as mt5  # noqa: PLC0415 - Windows-only, imported after argument checks

    if not (mt5.initialize(args.terminal) if args.terminal else mt5.initialize()):
        print(f"không kết nối được MT5: {mt5.last_error()}", file=sys.stderr)
        return 1
    try:
        batch = read_batch(mt5, args.days, args.server_offset_min)
        with open(args.out, "w", encoding="utf-8") as file:
            json.dump(batch, file, ensure_ascii=False, indent=2)
        print(f"đã ghi {len(batch['deals'])} deal vào {args.out}")
        return 0
    except RuntimeError as err:
        print(f"TradeLens: {err}", file=sys.stderr)
        return 1
    finally:
        mt5.shutdown()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
