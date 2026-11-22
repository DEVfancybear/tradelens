"""Tests for tradelens_sync.py with a fake MetaTrader5 module (no terminal needed).

    python -m unittest            # from the python/ folder
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace as NS
from typing import Any
from unittest import mock

import tradelens_sync as sync

T0 = 1_763_650_000_000  # time_msc of the entry deal

# The fields of the Rust `Deal` the app deserializes (crates/core/src/model.rs).
DEAL_KEYS = {
    "ticket", "order", "position_id", "time_msc", "symbol", "type", "entry", "volume", "price",
    "commission", "swap", "fee", "profit", "magic", "comment", "sl", "tp", "risk", "reason",
}  # fmt: skip
ACCOUNT_KEYS = {"login", "server", "broker", "name", "currency", "balance", "equity", "server_offset_min"}
KINDS = {"buy", "sell", "balance", "credit", "charge", "correction", "bonus", "commission", "other"}
ENTRIES = {"in", "out", "inout", "out_by", ""}


def deal(ticket: int, **fields: Any) -> NS:
    base = {
        "ticket": ticket, "order": 0, "position_id": 0, "time_msc": T0, "symbol": "", "type": 0, "entry": 0,
        "volume": 0.0, "price": 0.0, "commission": 0.0, "swap": 0.0, "fee": 0.0, "profit": 0.0,
        "magic": 0, "comment": "", "reason": 3,
    }  # fmt: skip
    return NS(**{**base, **fields})


def order(ticket: int, sl: float = 0.0, tp: float = 0.0) -> NS:
    return NS(ticket=ticket, sl=sl, tp=tp)


class FakeMt5:
    """Just the parts of the MetaTrader5 package the script uses."""

    def __init__(self, tick_offset_sec: int | None = 7200) -> None:
        self.deals = [
            deal(1, type=2, entry=0, time_msc=T0 - 5000, profit=10_000.0, comment='Nạp "demo"\n'),
            deal(10, order=100, position_id=100, time_msc=T0, symbol="XAUUSD", type=0, entry=0,
                 volume=0.1, price=2650.25, commission=-0.70000000001),
            deal(11, order=101, position_id=100, time_msc=T0 + 3_600_000, symbol="XAUUSD", type=1, entry=1,
                 volume=0.1, price=2660.25, profit=100.0, reason=5),
        ]  # fmt: skip
        self.orders = {100: order(100, sl=2640.0, tp=2670.0), 101: order(101)}
        self.tick_offset_sec = tick_offset_sec
        self.history_calls: list[tuple[Any, ...]] = []

    def account_info(self) -> NS:
        return NS(login=52_012_345, server="ICMarketsSC-Demo", company="Raw Trading Ltd", name="Demo",
                  currency="USD", balance=10_250.4, equity=10_310.0)  # fmt: skip

    def symbols_get(self) -> list[NS]:
        return [NS(name="XAUUSD", visible=True), NS(name="HIDDEN", visible=False)]

    def symbol_info_tick(self, name: str) -> NS | None:
        if name != "XAUUSD" or self.tick_offset_sec is None:
            return None
        return NS(time=int(datetime.now(timezone.utc).timestamp()) + self.tick_offset_sec + 3)

    def symbol_info(self, name: str) -> NS | None:
        if name == "XAUUSD":
            return NS(trade_tick_size=0.01, trade_tick_value=1.0, trade_tick_value_loss=1.0)
        return None

    def history_deals_get(self, start: datetime, end: datetime) -> list[NS]:
        self.history_calls.append((start, end))
        return list(self.deals)

    def history_orders_get(self, *window: datetime, position: int | None = None) -> list[NS]:
        if position is not None:
            return [o for o in self.orders.values() if o.ticket == position]
        return list(self.orders.values())

    def positions_get(self) -> list[NS]:
        return []

    def last_error(self) -> tuple[int, str]:
        return (1, "Success")

    def initialize(self, *_: Any) -> bool:
        return True

    def shutdown(self) -> None:
        return None


class AccountTests(unittest.TestCase):
    def test_account_shape(self) -> None:
        account = sync.account_info(FakeMt5())
        self.assertEqual(set(account), ACCOUNT_KEYS)
        self.assertEqual(account["login"], "52012345")  # text, as AccountInfo.login is a String
        self.assertEqual(account["broker"], "Raw Trading Ltd")
        self.assertEqual(account["balance"], 10_250.4)
        self.assertEqual(account["server_offset_min"], 120)
        json.dumps(account)

    def test_offset_unknown_without_ticks(self) -> None:
        self.assertIsNone(sync.account_info(FakeMt5(tick_offset_sec=None))["server_offset_min"])

    def test_stale_tick_is_not_an_offset(self) -> None:
        # a tick from 40 hours ago must not be read as a timezone
        self.assertIsNone(sync.account_info(FakeMt5(tick_offset_sec=-40 * 3600 - 421))["server_offset_min"])

    def test_explicit_offset_wins(self) -> None:
        self.assertEqual(sync.account_info(FakeMt5(), offset_min=-300)["server_offset_min"], -300)


class DealJsonTests(unittest.TestCase):
    def setUp(self) -> None:
        self.mt5 = FakeMt5()
        self.levels = sync.Levels(self.mt5, sync.history_window(90))

    def test_entry_deal_carries_levels_and_risk(self) -> None:
        entry = sync.deal_json(self.mt5.deals[1], self.levels)
        self.assertEqual(set(entry), DEAL_KEYS)
        self.assertEqual((entry["type"], entry["entry"], entry["reason"]), ("buy", "in", "expert"))
        self.assertEqual((entry["sl"], entry["tp"]), (2640.0, 2670.0))
        self.assertAlmostEqual(entry["risk"], 102.5, places=6)  # |2650.25 - 2640| / 0.01 * 1 * 0.1
        self.assertEqual(entry["commission"], -0.7)  # float noise rounded away
        self.assertIsInstance(entry["ticket"], int)
        self.assertIsInstance(entry["time_msc"], int)

    def test_exit_deal_takes_levels_of_the_opening_order(self) -> None:
        out = sync.deal_json(self.mt5.deals[2], self.levels)
        self.assertEqual((out["type"], out["entry"], out["reason"]), ("sell", "out", "tp"))
        self.assertEqual((out["sl"], out["tp"], out["risk"]), (2640.0, 2670.0, 0.0))

    def test_cash_deal_has_no_levels(self) -> None:
        cash = sync.deal_json(self.mt5.deals[0], self.levels)
        self.assertEqual((cash["type"], cash["symbol"], cash["sl"], cash["risk"]), ("balance", "", 0.0, 0.0))
        self.assertEqual(cash["comment"], 'Nạp "demo"\n')

    def test_enums_stay_inside_the_contract(self) -> None:
        for number in range(-1, 20):
            fake = deal(5, type=number, entry=number, reason=number)
            payload = sync.deal_json(fake)
            self.assertIn(payload["type"], KINDS)
            self.assertIn(payload["entry"], ENTRIES)
        self.assertEqual(sync.deal_json(deal(5, type=9))["type"], "commission")

    def test_payload_is_valid_json_text(self) -> None:
        text = json.dumps({"account": sync.account_info(self.mt5), "deals": [sync.deal_json(d, self.levels) for d in self.mt5.deals]})
        parsed = json.loads(text)
        self.assertEqual(len(parsed["deals"]), 3)


class OutFileTests(unittest.TestCase):
    def test_out_file_is_a_deal_batch(self) -> None:
        with tempfile.TemporaryDirectory() as folder, mock.patch.dict(sys.modules, {"MetaTrader5": FakeMt5()}):
            target = Path(folder) / "deals.json"
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(sync.main(["--out", str(target)]), 0)
            batch = json.loads(target.read_text(encoding="utf-8"))
        self.assertEqual(set(batch), {"account", "deals"})
        self.assertEqual(set(batch["account"]), ACCOUNT_KEYS)
        self.assertEqual(len(batch["deals"]), 3)
        for item in batch["deals"]:
            self.assertEqual(set(item), DEAL_KEYS)

    def test_out_is_required(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            sync.main([])
        self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
