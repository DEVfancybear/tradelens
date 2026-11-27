"""Tests for tradelens_worker.py with a fake MetaTrader5 module (no terminal needed).

    python -m unittest            # from the python/ folder
"""

from __future__ import annotations

import io
import json
import os
import tempfile
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
        self.positions_fail = False
        self.trade_allowed = True
        # What `symbol_info` says of every symbol; `filling_mode` 2 is IOC only.
        self.filling_mode = 2
        self.volume_step = 0.01
        self.volume_min = 0.01
        # Retcodes for the next orders, before `retcode`; and the lots a partial fill reports.
        self.retcodes: list[int] = []
        self.filled: float | None = None

    def terminal_info(self) -> Any:
        return NS(trade_allowed=self.trade_allowed, connected=self.logged_in)

    def initialize(self, path: str, **kwargs: Any) -> bool:
        self.initialized_with = path
        if kwargs.get("password") != self.password:
            self.error = (-6, "Terminal: Authorization failed")
            return False
        self.logged_in = True
        return True

    def shutdown(self) -> None:
        self.logged_in = False

    def last_error(self) -> tuple[int, str]:
        return self.error

    def positions_get(self, ticket: int | None = None) -> tuple[Any, ...] | None:
        if self.positions_fail:
            self.error = (-10004, "No IPC connection")
            return None
        if ticket is None:
            return tuple(self.open_positions)
        return tuple(p for p in self.open_positions if p.ticket == ticket)

    def symbol_info(self, name: str) -> Any:
        info = super().symbol_info(name)
        trading = {"filling_mode": self.filling_mode, "volume_step": self.volume_step, "volume_min": self.volume_min}
        return NS(**{**(vars(info) if info is not None else {}), **trading})

    def symbol_info_tick(self, name: str) -> Any:
        base = super().symbol_info_tick(name)
        return NS(time=base.time if base is not None else 0, bid=2654.9, ask=2655.1)

    def order_send(self, request: dict[str, Any]) -> Any:
        self.sent.append(request)
        retcode = self.retcodes.pop(0) if self.retcodes else self.retcode
        filled = request.get("volume") if self.filled is None else self.filled
        comment = "done" if retcode == 10009 else "Market closed"
        return NS(retcode=retcode, order=555, deal=556, volume=filled, comment=comment)


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

    def test_a_part_close_off_the_lot_step_is_refused_not_rounded(self) -> None:
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": 0.015})
        self.assertFalse(refused["ok"], refused)
        self.assertIn("bội số", refused["error"])
        self.assertEqual(self.mt5.sent, [], "the trader confirmed 0.015: nothing else is sent")

    def test_a_part_close_that_leaves_less_than_the_smallest_lot_is_refused(self) -> None:
        self.mt5.volume_min = 0.1
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": 0.15})
        self.assertFalse(refused["ok"], refused)
        self.assertIn("còn 0.05 lot", refused["error"])
        self.assertEqual(self.mt5.sent, [])
        _, closed = run(self.mt5, CONNECT, {"id": 3, "op": "close", "ticket": 100, "volume": 0.1})
        self.assertTrue(closed["ok"], closed)

    def test_a_volume_on_the_step_is_sent_without_float_noise(self) -> None:
        _, closed = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": 0.07})
        self.assertTrue(closed["ok"], closed)
        self.assertEqual(self.mt5.sent[-1]["volume"], 0.07)

    def test_a_refused_filling_mode_is_tried_again_with_the_next_one(self) -> None:
        self.mt5.filling_mode = 3  # FOK and IOC
        self.mt5.retcodes = [worker.INVALID_FILL]
        _, closed = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertTrue(closed["ok"], closed)
        modes = [request["type_filling"] for request in self.mt5.sent]
        self.assertEqual(modes, [TradingMt5.ORDER_FILLING_FOK, TradingMt5.ORDER_FILLING_IOC])

    def test_another_refusal_is_not_sent_again(self) -> None:
        self.mt5.filling_mode = 3
        self.mt5.retcode = 10018
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertFalse(refused["ok"])
        self.assertEqual(len(self.mt5.sent), 1, "only a refused filling mode is tried again")

    def test_every_filling_mode_refused_says_the_last_refusal(self) -> None:
        self.mt5.retcode = worker.INVALID_FILL
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertFalse(refused["ok"])
        self.assertEqual(refused["code"], worker.INVALID_FILL)
        modes = [request["type_filling"] for request in self.mt5.sent]
        self.assertEqual(modes, [TradingMt5.ORDER_FILLING_IOC, TradingMt5.ORDER_FILLING_RETURN])

    def test_a_partial_fill_is_not_reported_as_done(self) -> None:
        self.mt5.retcode = worker.DONE_PARTIAL
        self.mt5.filled = 0.05
        _, partial = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertFalse(partial["ok"], partial)
        self.assertEqual(partial["code"], worker.DONE_PARTIAL)
        self.assertIn("0.05 trên 0.2 lot", partial["error"])

    def test_a_refused_order_says_the_brokers_code(self) -> None:
        self.mt5.retcode = 10018
        _, refused = run(self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None})
        self.assertFalse(refused["ok"])
        self.assertEqual(refused["code"], 10018)

    def test_algo_trading_off_is_said_before_any_order_is_sent(self) -> None:
        self.mt5.trade_allowed = False
        _, closed, moved = run(
            self.mt5,
            CONNECT,
            {"id": 2, "op": "close", "ticket": 100, "volume": None},
            {"id": 3, "op": "modify", "ticket": 100, "sl": 2645.0, "tp": 0},
        )
        for refused in (closed, moved):
            self.assertFalse(refused["ok"], refused)
            self.assertEqual(refused["code"], worker.ALGO_TRADING_OFF)
            self.assertIn("Algo Trading", refused["error"])
        self.assertEqual(self.mt5.sent, [], "nothing reaches the broker")

    def test_the_terminal_starts_portable_with_algo_trading_on(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            terminal = os.path.join(folder, "Nguyễn Văn A", "terminal64.exe")
            os.makedirs(os.path.dirname(terminal))
            self.assertTrue(worker.write_start_config(terminal))
            config = os.path.join(os.path.dirname(terminal), worker.START_CONFIG_NAME)
            with open(config, encoding="ascii") as file:
                lines = [line.strip() for line in file]
            self.assertEqual(lines[0], "[Experts]")
            for setting in ("AllowLiveTrading=1", "Enabled=1", "Account=0"):
                self.assertIn(setting, lines)
            self.assertEqual(
                worker.start_command(terminal, configured=True),
                [terminal, "/portable", "/config:tradelens-start.ini"],
                "relative to the terminal's folder, so no quoting for spaces or Vietnamese letters",
            )

    def test_a_folder_that_cannot_be_written_starts_the_terminal_without_the_config(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            terminal = os.path.join(folder, "missing", "terminal64.exe")
            self.assertFalse(worker.write_start_config(terminal), "no folder to write in")
            self.assertEqual(worker.start_command(terminal, configured=False), [terminal, "/portable"])

    def test_the_login_uses_the_same_absolute_path_as_the_start(self) -> None:
        relative = os.path.join("data", "..", "terminals", "terminal64.exe")
        (connected,) = run(self.mt5, {**CONNECT, "terminal": relative})
        self.assertTrue(connected["ok"], connected)
        self.assertEqual(self.mt5.initialized_with, os.path.abspath(relative))

    def test_the_terminal_is_started_by_its_path_as_given_letter_case_included(self) -> None:
        terminal = os.path.abspath(os.path.join("Terminals", "434572098@Exness-MT5Trial7", "terminal64.exe"))
        leash = worker.Leash(terminal)
        self.assertEqual(leash.program, terminal, "initialize finds the running terminal by this exact path")
        self.assertEqual(leash.terminal, os.path.normcase(terminal), "windows are matched case-insensitively")

    def test_a_failed_login_still_ties_the_terminal_to_the_worker(self) -> None:
        tended: list[str] = []

        class FakeLeash:
            def __init__(self, terminal: str) -> None:
                self.terminal = terminal

            def start(self) -> None:
                pass

            def tend(self) -> int:
                tended.append(self.terminal)
                return 0

            def watch(self, seconds: float) -> None:
                watched.append(seconds)

            @staticmethod
            def end_watch(helper: None) -> None:
                ended.append(helper)

        watched: list[float] = []
        ended: list[None] = []

        original = worker.Leash
        worker.Leash = FakeLeash  # type: ignore[misc, assignment]
        try:
            (refused,) = run(self.mt5, {**CONNECT, "password": "wrong", "hide": True})
        finally:
            worker.Leash = original  # type: ignore[misc]
        self.assertFalse(refused["ok"], refused)
        self.assertEqual(tended, [os.path.abspath("terminal64.exe")], "tied before the failure is answered")
        self.assertEqual(len(watched), 1, "the windows are watched while initialize logs in")
        self.assertEqual(len(ended), 1, "and the watch ends with the login, failed or not")

    @unittest.skipUnless(os.name == "nt", "windows are Windows only")
    def test_the_window_watch_runs_as_this_script_and_ends_on_time(self) -> None:
        program = os.path.normcase(os.path.abspath("no-such-terminal64.exe"))
        start = worker.time.monotonic()
        self.assertEqual(worker.hide_windows_for(program, 0.3), 0)
        self.assertGreaterEqual(worker.time.monotonic() - start, 0.25)
        leash = worker.Leash(os.path.abspath("no-such-terminal64.exe"))
        helper = leash.watch(5)
        self.assertIsNotNone(helper)
        assert helper is not None
        self.assertIsNone(helper.poll(), "the watch runs until it is ended")
        worker.Leash.end_watch(helper)
        self.assertIsNotNone(helper.poll(), "ended")

    @unittest.skipUnless(os.name == "nt", "process snapshots are Windows only")
    def test_the_processes_of_a_program_are_found_by_its_path(self) -> None:
        program = os.path.normcase(os.path.abspath(worker.sys.executable))
        self.assertIn(os.getpid(), worker.processes_of(program))
        self.assertEqual(worker.processes_of(os.path.normcase(os.path.abspath("no-such-program.exe"))), [])

    def test_moving_the_levels_sends_an_sltp_request(self) -> None:
        _, moved = run(self.mt5, CONNECT, {"id": 2, "op": "modify", "ticket": 100, "sl": 2645.0, "tp": 0})
        self.assertTrue(moved["ok"], moved)
        request = self.mt5.sent[-1]
        self.assertEqual(request["action"], TradingMt5.TRADE_ACTION_SLTP)
        self.assertEqual((request["sl"], request["tp"]), (2645.0, 0.0))

    def test_positions_that_cannot_be_read_fail_the_poll_instead_of_reading_as_none_open(self) -> None:
        self.mt5.positions_fail = True
        _, polled = run(self.mt5, CONNECT, {"id": 2, "op": "poll", "days": 30})
        self.assertFalse(polled["ok"], polled)
        self.assertEqual(polled["code"], -10004)

    def test_an_unexpected_exception_is_answered_and_the_worker_goes_on(self) -> None:
        def broken(_request: dict[str, Any]) -> Any:
            msg = "no such attribute"
            raise AttributeError(msg)

        self.mt5.order_send = broken  # type: ignore[method-assign]
        _, failed, polled = run(
            self.mt5, CONNECT, {"id": 2, "op": "close", "ticket": 100, "volume": None}, {"id": 3, "op": "poll"}
        )
        self.assertFalse(failed["ok"])
        self.assertIn("no such attribute", failed["error"])
        self.assertTrue(polled["ok"], "the next request is still answered")

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
