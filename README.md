# TradeLens

Nhật ký giao dịch desktop cho trader **MetaTrader 5**: lấy lệnh từ báo cáo MT5, gom thành giao dịch,
tính thống kê, viết nhật ký từng lệnh, theo dõi kỷ luật và sync lên Google Drive / OneDrive.

UI viết bằng [Native Shell (NSH)](https://github.com/longbridge/nsh) ở chế độ Rust: native trên
Windows, macOS và Linux, không có JavaScript VM.

> Trạng thái: **khung dự án (v0.0)**. Domain, gom deal → lệnh và thống kê đã có; các phần khác là
> module rỗng có chữ ký hàm, xem [docs/roadmap.md](docs/roadmap.md).

## Cấu trúc

```text
crates/core     tradelens-core   model, deal → lệnh, thống kê, quy tắc, SQLite, import báo cáo
crates/cloud    tradelens-cloud  đăng nhập Google/Microsoft, sync Google Drive / OneDrive
app/            nsh-app-tradelens  giao diện NSH
python/         tradelens_sync.py (gói MetaTrader5): ghi lịch sử ra file JSON để import
docs/           kiến trúc, sync đám mây, lộ trình
docs/design/    thiết kế UI: 12 màn (HTML tham chiếu + ảnh) và chuẩn để code
```

## Chạy thử

Cần Rust 1.100 (bản stable mới nhất; `rust-toolchain.toml` tự chọn đúng bản khi chạy `rustup`). Trên Linux cần thêm GTK 4 (≥ 4.14) và GStreamer dev.

```sh
cargo test --workspace          # domain + headless UI tests
cargo run -p nsh-app-tradelens  # mở app trên desktop
# hoặc với NSH CLI: nsh run app
```

## Đưa lệnh MT5 vào app

- MT5 → *History* → chuột phải → *Report* → lưu HTML hoặc XLSX, rồi kéo thả vào **Nguồn dữ liệu**.
- Hoặc chạy `python python/tradelens_sync.py --days 365 --out deals.json` khi terminal đang mở, rồi import file JSON.

Kết nối trực tiếp (đăng nhập MT5 ngay trong app, không cần EA) là mốc v0.4, xem [docs/roadmap.md](docs/roadmap.md).

## License

MIT
