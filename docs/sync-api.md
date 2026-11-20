# API của bridge (EA / Python → TradeLens)

Bridge chỉ lắng nghe `127.0.0.1`, cổng mặc định **7878** (đổi trong *Nguồn dữ liệu*).
Mọi request mang header `X-TradeLens-Token: <token>`; token sinh ngẫu nhiên lần đầu chạy app.

| Method | Đường dẫn | Body | Trả về |
|---|---|---|---|
| GET  | `/api/v1/ping`    | — | `{"app":"TradeLens","version":"0.1.0"}` |
| POST | `/api/v1/deals`   | `DealBatch` | `{"account_id":1,"inserted":12,"updated":0}` |
| POST | `/api/v1/account` | `{"account": AccountInfo}` | `{"account_id":1}` |

Lỗi: status 4xx/5xx với `{"error":"..."}` — `401` sai token, `400` JSON sai, `500` lỗi DB.

## DealBatch

```json
{
  "account": {
    "login": "52012345", "server": "ICMarketsSC-Demo", "broker": "Raw Trading Ltd",
    "name": "Demo", "currency": "USD", "balance": 10250.4, "equity": 10310.0,
    "server_offset_min": 120
  },
  "deals": [
    {
      "ticket": 81234567, "order": 91234567, "position_id": 61234567,
      "time_msc": 1763650000000, "symbol": "XAUUSD",
      "type": "buy", "entry": "in", "volume": 0.10, "price": 2650.25,
      "commission": -0.70, "swap": 0, "fee": 0, "profit": 0,
      "magic": 0, "comment": "", "sl": 2640.00, "tp": 2670.00, "risk": 102.50
    }
  ]
}
```

- `time_msc`: giờ **server** MT5 (`DEAL_TIME_MSC`), mili-giây.
- `type`: `buy` `sell` `balance` `credit` `charge` `correction` `bonus` `commission`, hoặc số `ENUM_DEAL_TYPE`.
- `entry`: `in` `out` `inout` `out_by`, hoặc số `ENUM_DEAL_ENTRY`.
- `risk`: tiền rủi ro tại SL của deal vào lệnh = |giá − SL| / tick size × tick value × volume (EA tính).
- `server_offset_min`: `(TimeTradeServer() − TimeGMT()) / 60`.

Gửi lại cùng deal nhiều lần là an toàn: khóa là `(tài khoản, ticket)`.
