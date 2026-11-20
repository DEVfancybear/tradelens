# TradeLens

Nhật ký giao dịch desktop cho trader **MetaTrader 5**: tự lấy lệnh từ terminal, gom thành giao dịch,
tính thống kê, viết nhật ký từng lệnh, theo dõi kỷ luật và sync lên Google Drive / OneDrive.

UI viết bằng [Native Shell (NSH)](https://github.com/longbridge/nsh) ở chế độ Rust: native trên
Windows, macOS và Linux, không có JavaScript VM.

> Trạng thái: **khung dự án (v0.0)**. Domain, gom deal → lệnh và thống kê đã có; các phần khác là
> module rỗng có chữ ký hàm, xem [docs/roadmap.md](docs/roadmap.md).

## Cấu trúc

```text
crates/core     tradelens-core   model, deal → lệnh, thống kê, quy tắc, SQLite, import báo cáo
crates/bridge   tradelens-bridge HTTP 127.0.0.1 nhận deal từ EA và script Python
crates/cloud    tradelens-cloud  đăng nhập Google/Microsoft, sync Google Drive / OneDrive
app/            nsh-app-tradelens  giao diện NSH
mt5/            EA TradeLensSync.mq5
python/         tradelens_sync.py (gói MetaTrader5)
docs/           kiến trúc, API bridge, sync đám mây, lộ trình
docs/design/    thiết kế UI: 12 màn (HTML tham chiếu + ảnh) và chuẩn để code
```

## Chạy thử

Cần Rust 1.100 (bản stable mới nhất; `rust-toolchain.toml` tự chọn đúng bản khi chạy `rustup`). Trên Linux cần thêm GTK 4 (≥ 4.14) và GStreamer dev.

```sh
cargo test --workspace          # domain + headless UI tests
cargo run -p nsh-app-tradelens  # mở app trên desktop
# hoặc với NSH CLI: nsh run app
```

## Kết nối MT5

1. Mở app → **Nguồn dữ liệu**, copy token.
2. MT5 → Tools → Options → Expert Advisors → bật *Allow WebRequest* và thêm `http://127.0.0.1:7878`.
3. Copy `mt5/TradeLensSync.mq5` vào `MQL5/Experts`, biên dịch, gắn vào một chart, dán token.

Chi tiết: [docs/sync-api.md](docs/sync-api.md).

## License

MIT
