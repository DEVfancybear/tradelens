"""Send MT5 deal history to TradeLens (or to a JSON file to import).

Uses the official MetaTrader5 package, so it runs on Windows with the terminal open.

    pip install -r requirements.txt
    python tradelens_sync.py --token <token> --days 365
    python tradelens_sync.py --out deals.json        # then import the file in the app

Payload: docs/sync-api.md (DealBatch).
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from datetime import datetime, timedelta, timezone
from typing import Any

DEFAULT_URL = "http://127.0.0.1:7878"
TOKEN_HEADER = "X-TradeLens-Token"

DEAL_TYPES = {0: "buy", 1: "sell", 2: "balance", 3: "credit", 4: "charge", 5: "correction", 6: "bonus"}
DEAL_ENTRIES = {0: "in", 1: "out", 2: "inout", 3: "out_by"}


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", default=DEFAULT_URL, help="bridge address (default %(default)s)")
    parser.add_argument("--token", help="token from the app's Data sources screen")
    parser.add_argument("--days", type=int, default=90, help="how far back to read (default %(default)s)")
    parser.add_argument("--out", help="write the batch to this JSON file instead of posting it")
    parser.add_argument("--terminal", help="path to terminal64.exe when several terminals are installed")
    return parser.parse_args(argv)


def account_info(mt5: Any) -> dict[str, Any]:
    """The `account` part of the batch."""
    # TODO(v0.1): mt5.account_info() → login, server, company, name, currency, balance, equity;
    # server offset from mt5.symbol_info_tick(<any symbol>).time vs UTC now.
    raise NotImplementedError("account_info")


def deal_json(deal: Any) -> dict[str, Any]:
    """One TradeDeal as the bridge expects it."""
    return {
        "ticket": deal.ticket,
        "order": deal.order,
        "position_id": deal.position_id,
        "time_msc": deal.time_msc,
        "symbol": deal.symbol,
        "type": DEAL_TYPES.get(deal.type, "other"),
        "entry": DEAL_ENTRIES.get(deal.entry, ""),
        "volume": deal.volume,
        "price": deal.price,
        "commission": deal.commission,
        "swap": deal.swap,
        "fee": deal.fee,
        "profit": deal.profit,
        "magic": deal.magic,
        "comment": deal.comment,
    }


def read_batch(mt5: Any, days: int) -> dict[str, Any]:
    end = datetime.now(timezone.utc) + timedelta(days=1)
    start = end - timedelta(days=days + 1)
    deals = mt5.history_deals_get(start, end) or ()
    return {"account": account_info(mt5), "deals": [deal_json(d) for d in deals]}


def post(url: str, token: str, batch: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        f"{url}/api/v1/deals",
        data=json.dumps(batch).encode("utf-8"),
        headers={"Content-Type": "application/json", TOKEN_HEADER: token},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


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
        batch = read_batch(mt5, args.days)
    finally:
        mt5.shutdown()
    if args.out:
        with open(args.out, "w", encoding="utf-8") as file:
            json.dump(batch, file, ensure_ascii=False, indent=2)
        print(f"đã ghi {len(batch['deals'])} deal vào {args.out}")
    else:
        result = post(args.url, args.token, batch)
        print(f"TradeLens: thêm {result['inserted']}, cập nhật {result['updated']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
