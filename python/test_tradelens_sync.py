"""Tests for tradelens_sync.py with a fake MetaTrader5 module (no terminal needed).

    python -m unittest            # from the python/ folder
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import threading
import unittest
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, HTTPServer
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


class Recorder:
    """A stand-in for the HTTP post: records requests, optionally fails some of them."""

    def __init__(self, fail: Any = None) -> None:
        self.requests: list[tuple[str, dict[str, Any]]] = []
        self.fail = fail

    def __call__(self, path: str, body: dict[str, Any]) -> dict[str, Any]:
        if self.fail is not None:
            error = self.fail(path, body)
            if error is not None:
                raise error
        self.requests.append((path, json.loads(json.dumps(body))))  # as the bridge would see it
        return {"account_id": 1, "inserted": 0, "updated": 0}

    def deals(self) -> list[dict[str, Any]]:
        return [d for path, body in self.requests if path.endswith("/deals") for d in body["deals"]]


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


class SyncTests(unittest.TestCase):
    def setUp(self) -> None:
        self.mt5 = FakeMt5()
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = Path(self.dir.name) / "cursor-52012345.json"

    def test_first_run_sends_everything_and_second_run_nothing(self) -> None:
        recorder = Recorder()
        cursor = sync.Cursor(self.path)
        self.assertEqual(sync.sync_once(self.mt5, recorder, cursor, 90), 3)
        path, body = recorder.requests[0]
        self.assertEqual(path, "/api/v1/deals")
        self.assertEqual(set(body), {"account", "deals"})
        self.assertEqual([d["ticket"] for d in body["deals"]], [1, 10, 11])  # oldest first
        self.assertEqual(cursor.msc, T0 + 3_600_000)

        again = Recorder()
        self.assertEqual(sync.sync_once(self.mt5, again, sync.Cursor(self.path), 90), 0)  # state read from disk
        self.assertEqual([p for p, _ in again.requests], ["/api/v1/account"])  # heartbeat only
        self.assertEqual(set(again.requests[0][1]["account"]), ACCOUNT_KEYS)

    def test_only_new_deals_are_sent(self) -> None:
        sync.sync_once(self.mt5, Recorder(), sync.Cursor(self.path), 90)
        self.mt5.deals.append(deal(12, order=102, position_id=102, time_msc=T0 + 7_200_000, symbol="XAUUSD", entry=0, volume=0.2, price=2655.0))
        recorder = Recorder()
        self.assertEqual(sync.sync_once(self.mt5, recorder, sync.Cursor(self.path), 90), 1)
        self.assertEqual([d["ticket"] for d in recorder.deals()], [12])

    def test_late_deal_inside_the_overlap_is_still_sent(self) -> None:
        sync.sync_once(self.mt5, Recorder(), sync.Cursor(self.path), 90)
        self.mt5.deals.append(deal(13, time_msc=T0 + 3_000_000, type=2, profit=5.0))  # older than the cursor
        recorder = Recorder()
        sync.sync_once(self.mt5, recorder, sync.Cursor(self.path), 90)
        self.assertEqual([d["ticket"] for d in recorder.deals()], [13])

    def test_failure_keeps_the_cursor_at_the_last_acknowledged_batch(self) -> None:
        calls = {"deals": 0}

        def fail_second_batch(path: str, _body: dict[str, Any]) -> sync.BridgeError | None:
            if path.endswith("/deals"):
                calls["deals"] += 1
                if calls["deals"] == 2:
                    return sync.BridgeError("app chưa mở")
            return None

        with mock.patch.object(sync, "BATCH_SIZE", 1):
            cursor = sync.Cursor(self.path)
            with self.assertRaises(sync.BridgeError):
                sync.sync_once(self.mt5, Recorder(fail_second_batch), cursor, 90)
            self.assertEqual(sync.Cursor(self.path).msc, T0 - 5000)  # only deal 1 was acknowledged

            recorder = Recorder()
            self.assertEqual(sync.sync_once(self.mt5, recorder, sync.Cursor(self.path), 90), 2)
        self.assertEqual([d["ticket"] for d in recorder.deals()], [10, 11])

    def test_rejected_deal_is_skipped_without_blocking_the_rest(self) -> None:
        def reject_ticket_10(_path: str, body: dict[str, Any]) -> sync.BridgeError | None:
            if any(d["ticket"] == 10 for d in body["deals"]):
                return sync.BridgeError("JSON sai", 400, retryable=False)
            return None

        recorder = Recorder(reject_ticket_10)
        cursor = sync.Cursor(self.path)
        with contextlib.redirect_stderr(io.StringIO()) as log:
            sync.sync_once(self.mt5, recorder, cursor, 90)
        self.assertEqual([d["ticket"] for d in recorder.deals()], [1, 11])
        self.assertIn("deal 10", log.getvalue())
        self.assertEqual(cursor.msc, T0 + 3_600_000)

    def test_full_ignores_the_cursor(self) -> None:
        sync.sync_once(self.mt5, Recorder(), sync.Cursor(self.path), 90)
        recorder = Recorder()
        self.assertEqual(sync.sync_once(self.mt5, recorder, sync.Cursor(self.path), 90, full=True), 3)

    def test_last_known_offset_is_kept_when_the_market_is_closed(self) -> None:
        sync.sync_once(self.mt5, Recorder(), sync.Cursor(self.path), 90)
        closed = FakeMt5(tick_offset_sec=None)
        recorder = Recorder()
        sync.sync_once(closed, recorder, sync.Cursor(self.path), 90)
        self.assertEqual(recorder.requests[0][1]["account"]["server_offset_min"], 120)

    def test_unreadable_cursor_file_means_resend_not_crash(self) -> None:
        self.path.write_text("{ not json", encoding="utf-8")
        self.assertEqual(sync.sync_once(self.mt5, Recorder(), sync.Cursor(self.path), 90), 3)


class RetryTests(unittest.TestCase):
    def test_retries_then_succeeds(self) -> None:
        attempts: list[int] = []
        waits: list[float] = []

        def flaky() -> str:
            attempts.append(1)
            if len(attempts) < 3:
                raise sync.BridgeError("down")
            return "ok"

        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(sync.with_retries(flaky, 3, waits.append), "ok")
        self.assertEqual(waits, [2.0, 4.0])

    def test_wrong_token_is_not_retried(self) -> None:
        def denied() -> None:
            raise sync.BridgeError("401", 401, retryable=False)

        waits: list[float] = []
        with self.assertRaises(sync.BridgeError):
            sync.with_retries(denied, 3, waits.append)
        self.assertEqual(waits, [])


class Handler(BaseHTTPRequestHandler):
    seen: list[tuple[str, str | None, dict[str, Any]]] = []

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        token = self.headers.get("X-TradeLens-Token")
        Handler.seen.append((self.path, token, body))
        status, answer = (200, {"account_id": 1, "inserted": 1, "updated": 0}) if token == "secret" else (401, {"error": "token"})
        data = json.dumps(answer).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_: Any) -> None:
        return None


class HttpTests(unittest.TestCase):
    def setUp(self) -> None:
        Handler.seen = []
        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)

    def test_token_header_and_utf8_body(self) -> None:
        batch = {"account": {"login": "1"}, "deals": [{"ticket": 1, "comment": "Nạp tiền"}]}
        result = sync.post(self.url, "secret", batch)
        self.assertEqual(result["inserted"], 1)
        path, token, body = Handler.seen[0]
        self.assertEqual((path, token), ("/api/v1/deals", "secret"))
        self.assertEqual(body["deals"][0]["comment"], "Nạp tiền")

    def test_wrong_token_raises_non_retryable(self) -> None:
        with self.assertRaises(sync.BridgeError) as caught:
            sync.post(self.url, "wrong", {"account": {"login": "1"}, "deals": []})
        self.assertEqual(caught.exception.status, 401)
        self.assertFalse(caught.exception.retryable)

    def test_unreachable_app_is_retryable(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        with self.assertRaises(sync.BridgeError) as caught:
            sync.post(self.url, "secret", {})
        self.assertIsNone(caught.exception.status)
        self.assertTrue(caught.exception.retryable)

    def test_main_posts_through_the_fake_terminal(self) -> None:
        with tempfile.TemporaryDirectory() as state, mock.patch.dict(sys.modules, {"MetaTrader5": FakeMt5()}):
            argv = ["--url", self.url, "--token", "secret", "--state-dir", state]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(sync.main(argv), 0)
            self.assertTrue((Path(state) / "cursor-52012345.json").exists())
        deals = [d for path, _, body in Handler.seen if path.endswith("/deals") for d in body["deals"]]
        self.assertEqual([d["ticket"] for d in deals], [1, 10, 11])

    def test_main_with_wrong_token_fails(self) -> None:
        with tempfile.TemporaryDirectory() as state, mock.patch.dict(sys.modules, {"MetaTrader5": FakeMt5()}):
            argv = ["--url", self.url, "--token", "wrong", "--state-dir", state]
            with contextlib.redirect_stderr(io.StringIO()) as log:
                self.assertEqual(sync.main(argv), 1)
            self.assertIn("401", log.getvalue())
            self.assertFalse((Path(state) / "cursor-52012345.json").exists())  # nothing acknowledged


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

    def test_token_or_out_is_required(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(sync.main([]), 2)


if __name__ == "__main__":
    unittest.main()
