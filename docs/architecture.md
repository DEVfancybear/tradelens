# Kiến trúc

Mọi nguồn dữ liệu ghi vào **một file SQLite** trên máy; app đọc từ đó. Đám mây chỉ giữ bản sao.

```text
Báo cáo HTML/XLSX ─┐
JSON (script Python) ┼─ chọn file ─► core::import ─┐
Notion CSV / API ──┘                                ├─► tradelens-core (SQLite) ◄─► app NSH
                                                    │          ▲
                                                    │          │ changelog
                            Google Drive / OneDrive ◄─► tradelens-cloud
```

Kết nối MT5 trực tiếp (v0.4, `tradelens-mt5`): mỗi tài khoản người dùng đăng nhập có một bản MT5 portable chạy ẩn
và một worker Python (`python/tradelens_worker.py`, gói `MetaTrader5`) nói chuyện với app bằng JSON từng dòng qua
stdin/stdout; luồng `Link` của tài khoản ghi deal thẳng vào SQLite (`source = mt5`) rồi tăng `Revision`. Không có EA;
EA và bridge HTTP của v0.1 đã bỏ.

| Crate | Trách nhiệm | Không làm |
|---|---|---|
| `tradelens-core` | Model, deal → lệnh, thống kê, quy tắc, SQLite, đọc file | Mạng, UI |
| `tradelens-cloud` | OAuth, Drive API, Notion API, gộp thay đổi | Đọc báo cáo MT5 |
| `tradelens-mt5` | Terminal portable, worker Python, luồng kết nối từng tài khoản, lệnh đóng / sửa SL-TP | Ghi DB, UI |
| `nsh-app-tradelens` | Giao diện, store, điều phối luồng nền | Tính toán nghiệp vụ |

## Quy ước

- Deal là nguồn sự thật; lệnh và chỉ số luôn tính lại từ deal.
- Lệnh nhập tay hoặc import (Notion…) chưa biết offset server được lưu theo giờ máy của trader, không có múi giờ; khi đó "hôm nay" và "bây giờ" cũng là giờ máy (`zone::stored_now`), không phải UTC.
- Thời gian MT5 là giờ server, lưu nguyên; quy đổi sang giờ địa phương khi hiển thị (`time::Clock`).
- Luồng nền (kết nối MT5, cloud) không chạm signal của UI: chúng tăng `Revision`, UI tải lại.
- Chất lượng code theo [BeautifulRust](https://github.com/morganlinton/BeautifulRust):
  `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` sạch trước mỗi commit.
