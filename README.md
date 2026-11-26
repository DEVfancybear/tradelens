# TradeLens

Nhật ký giao dịch desktop cho trader **MetaTrader 5**: lấy lệnh từ MT5 (đăng nhập ngay trong app hoặc
import báo cáo), gom thành giao dịch, tính thống kê, viết nhật ký từng lệnh, theo dõi kỷ luật và sync
lên Google Drive / OneDrive.

UI viết bằng [Native Shell (NSH)](https://github.com/longbridge/nsh) ở chế độ Rust: native trên
Windows, macOS và Linux, không có JavaScript VM.

## Tải về

**Bản mới nhất: [TradeLens 0.1.0 cho Windows](https://github.com/DEVfancybear/tradelens/releases/latest/download/TradeLens-setup.exe)**
(Windows 10/11, 64-bit) · [tất cả phiên bản](https://github.com/DEVfancybear/tradelens/releases) ·
[trang giới thiệu](https://devfancybear.github.io/tradelens/)

- Bộ cài chưa ký số: nếu Windows SmartScreen cảnh báo, chọn **Thông tin thêm** → **Vẫn chạy**.
- App tự kiểm tra và tải bản mới, cài khi khởi động lại (tắt ở Cài đặt › Về TradeLens › Tự động cập nhật).
- Kết nối MT5 trực tiếp cần terminal MetaTrader 5 trên máy. Nếu máy chưa có Python, bấm **Cài Python tự động** ở
  **Nguồn dữ liệu › Kết nối MT5**: app tự cài một bản Python riêng (khoảng 40 MB, trong thư mục dữ liệu của app, không
  cần quyền admin); máy đã có Python thì app chỉ cài thêm gói `MetaTrader5`. Nút **Đăng nhập** chỉ bật khi đã có cả
  Python và MetaTrader 5.

Lộ trình và nội dung từng bản: [docs/roadmap.md](docs/roadmap.md).

## Cấu trúc

```text
crates/core     tradelens-core    model, deal → lệnh, thống kê, quy tắc, SQLite, import báo cáo
crates/cloud    tradelens-cloud   đăng nhập Google/Microsoft, sync Google Drive / OneDrive
crates/mt5      tradelens-mt5     kết nối MT5: terminal portable ẩn + worker Python cho từng tài khoản
crates/update   tradelens-update  tự cập nhật qua nsh-updater (manifest ký Ed25519 trên GitHub Releases)
app/            nsh-app-tradelens giao diện NSH
python/         tradelens_worker.py (worker kết nối MT5), tradelens_sync.py (ghi lịch sử ra JSON để import)
docs/           kiến trúc, sync đám mây, lộ trình, trang giới thiệu (GitHub Pages)
docs/design/    thiết kế UI: HTML tham chiếu + ảnh, và chuẩn để code
```

## Chạy từ mã nguồn

Cần Rust 1.100 (bản stable mới nhất; `rust-toolchain.toml` tự chọn đúng bản khi chạy `rustup`). Trên Linux cần thêm GTK 4 (≥ 4.14) và GStreamer dev.

```sh
cargo test --workspace          # domain + headless UI tests
cargo run -p nsh-app-tradelens  # mở app trên desktop
# hoặc với NSH CLI: nsh run app
```

Phát hành: đẩy tag `v<version>` khớp `Cargo.toml` và `app/nsh.json`, xem mục "Tự cập nhật" trong
[docs/roadmap.md](docs/roadmap.md).

## Đưa lệnh MT5 vào app

- **Kết nối MT5** (Nguồn dữ liệu): đăng nhập số tài khoản, mật khẩu, server; app chạy một bản MT5 riêng, ẩn cho mỗi
  tài khoản và đưa lệnh vào nhật ký trong vài giây, không cần EA. Mật khẩu nằm trong Windows Credential Manager.
- Hoặc MT5 → *History* → chuột phải → *Report* → lưu HTML hoặc XLSX, rồi kéo thả vào **Nguồn dữ liệu**.
- Hoặc chạy `python python/tradelens_sync.py --days 365 --out deals.json` khi terminal đang mở, rồi import file JSON.

## License

MIT
