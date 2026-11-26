"""Tests for tradelens_worker.py with a fake MetaTrader5 module (no terminal needed).

    python -m unittest            # from the python/ folder
"""

from __future__ import annotations

import io
import json
import unittest
from types import SimpleNamespace as NS
from typing import Any

import tradelens_worker as worker
from test_tradelens_sync import FakeMt5


class TradingMt5(FakeMt5):
    """The fake of test_tradelens_sync with logins, positions and orders."""

    TRADE_ACTION_DEAL = 1
    TRADE_ACTION_SLTP = 6
    ORDER_TYPE_BUY = 0
    ORDER_TYPE_SELL = 1
    ORDER_FILLING_FOK = 0
    ORDER_FILLING_IOC = 1
    ORDER_FILLING_RETURN = 2
    ORDER_TIME_GTC = 0

    def __init__(self, password: str = "pass", retcode: int = 10009) -> None:
        super().__init__()
        self.password = password
        self.retcode = retcode
        self.sent: list[dict[str, Any]] = []
        self.open_positions = [
            NS(ticket=100, time=1_763_650_000, time_msc=1_763_650_000_123, symbol="XAUUSD", type=0, volume=0.2,
               price_open=2650.25, price_current=2655.0, sl=2640.0, tp=2670.0, profit=95.0, swap=0.0, magic=0,
               comment=""),
        ]  # fmt: skip
        self.error = (1, "Success")
        self.logged_in = False

    def initialize(self, path: str, **kwargs: Any) -> bool:
        if kwargs.get("password") != self.password:
            self.error = (-6, "Terminal: Authorization failed")
            return False
        self.logged_in = True
        return True

    def shutdown(self) -> None:
        self.logged_in = False

    def last_error(self) -> tuple[int, str]:
        return self.error

    def positions_get(self, ticket: int | None = None) -> tuple[Any, ...]:
        if ticket is None:
            return tuple(self.open_positions)
        return tuple(p for p in self.open_positions if p.ticket == ticket)

    def symbol_info(self, name: str) -> Any:
        info = super().symbol_info(name)
        return NS(**{**vars(info), "filling_mode": 2}) if info is not None else NS(filling_mode=2)

    def symbol_info_tick(self, name: str) -> Any:
        base = super().symbol_info_tick(name)
        return NS(time=base.time if base is not None else 0, bid=2654.9, ask=2655.1)

    def order_send(self, request: dict[str, Any]) -> Any:
        self.sent.append(request)
        return NS(retcode=self.retcode, order=555, deal=556, comment="done" if self.retcode == 10009 else "Market closed")


def run(mt5: Any, *requests: dict[str, Any]) -> list[dict[str, Any]]:
    """The replies of the worker to `requests`, one per line."""
    lines = [json.dumps(request) for request in requests]
    out = io.StringIO()
    worker.serve(mt5, iter(lines), out)
    return [json.loads(line) for line in out.getvalue().splitlines()]


CONNECT = {"id": 1, "op": "connect", "terminal": "terminal64.exe", "login": 52012345, "password": "pass",
           "server": "Demo", "hide": False}  # fmt: skip


class WorkerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.mt5 = TradingMt5()

    def test_a_poll_after_connecting_answers_the_batch_and_the_open_positions(self) -> None:
        connected, polled = run(self.mt5, CONNECT, {"id": 2, "op": "poll", "days": 30})
        self.assertTrue(connected["ok"], connected)
        self.assertEqual(connected["data"]["server_offset_min"], 120)
        self.assertTrue(polled["ok"], polled)
        data = polled["data"]
        self.assertEqual({"account", "deals", "positions"}, set(data))
        self.assertEqual(len(data["deals"]), 3)
        position = data["positions"][0]
        self.assertEqual(position["ticket"], 100)
        self.assertEqual(position["side"], "buy")
        self.assertEqual(position["time_msc"], 1_763_650_000_123)
        self.assertFalse(self.mt5.logged_in, "the end of input logs out")

    def test_a_wrong_password_answers_the_auth_code(self) -> None:
        (refused,) = run(self.mt5, {**CONNECT, "password": "wrong"})
        self.assertFalse(refused["ok"])
        self.assertEqual(refused["code"], worker.AUTH_FAILED)
        self.assertEqual(refused["id"], 1)

    def test_nothing_is_read_before_the_login(self) -> None:
        (refused,) = run(self.mt5, {"id": 5, "op": "poll"})
        self.assertFalse(refused["ok"])
        self.assertIn("chưa đăng nhập", refused["error"])

    def test_closing_part_of_a_position_sells_at_the_bid_for_a_buy(self) -> None:
        _, closed = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": 0.05})
        self.assertTrue(closed["ok"], closed)
        request = self.mt5.sent[-1]
        self.assertEqual(request["type"], TradingMt5.ORDER_TYPE_SELL)
        self.assertEqual(request["price"], 2654.9)
        self.assertEqual(request["volume"], 0.05)
        self.assertEqual(request["position"], 100)
        self.assertEqual(request["type_filling"], TradingMt5.ORDER_FILLING_IOC)

    def test_closing_more_than_is_open_is_refused_without_an_order(self) -> None:
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": 5})
        self.assertFalse(refused["ok"])
        self.assertEqual(self.mt5.sent, [])

    def test_a_refused_order_says_the_brokers_code(self) -> None:
        self.mt5.retcode = 10018
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertFalse(refused["ok"])
        self.assertEqual(refused["code"], 10018)

    def test_moving_the_levels_sends_an_sltp_request(self) -> None:
        _, moved = run(self.mt5, CONNECT, {"id": 2, "op": "modify", "ticket": 100, "sl": 2645.0, "tp": 0})
        self.assertTrue(moved["ok"], moved)
        request = self.mt5.sent[-1]
        self.assertEqual(request["action"], TradingMt5.TRADE_ACTION_SLTP)
        self.assertEqual((request["sl"], request["tp"]), (2645.0, 0.0))

    def test_quit_answers_and_stops_reading(self) -> None:
        replies = run(self.mt5, CONNECT, {"id": 9, "op": "quit"}, {"id": 10, "op": "poll"})
        self.assertEqual([reply["id"] for reply in replies], [1, 9])

    def test_a_line_that_is_not_json_is_answered_and_the_next_one_still_read(self) -> None:
        out = io.StringIO()
        worker.serve(self.mt5, iter(["not json", json.dumps({"id": 3, "op": "nope"})]), out)
        replies = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual(len(replies), 2)
        self.assertFalse(replies[0]["ok"])
        self.assertEqual(replies[1]["id"], 3)


if __name__ == "__main__":
    unittest.main()
