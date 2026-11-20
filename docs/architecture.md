# Kiến trúc

Mọi nguồn dữ liệu ghi vào **một file SQLite** trên máy; app đọc từ đó. Đám mây chỉ giữ bản sao.

```text
EA TradeLensSync ─┐
Script Python ────┼─ HTTP 127.0.0.1:7878 ─► tradelens-bridge ─┐
                  │                                            ├─► tradelens-core (SQLite) ◄─► app NSH
Báo cáo HTML/XLSX ┴─ chọn file ───────────► core::import ─────┤
Notion CSV / API ───────────────────────────────────────────────┘          ▲
                                                                             │ changelog
                                         Google Drive / OneDrive ◄─► tradelens-cloud
```

| Crate | Trách nhiệm | Không làm |
|---|---|---|
| `tradelens-core` | Model, deal → lệnh, thống kê, quy tắc, SQLite, đọc file | Mạng, UI |
| `tradelens-bridge` | HTTP local cho EA/Python, xác thực token | Logic nghiệp vụ |
| `tradelens-cloud` | OAuth, Drive API, Notion API, gộp thay đổi | Đọc báo cáo MT5 |
| `nsh-app-tradelens` | Giao diện, store, điều phối luồng nền | Tính toán nghiệp vụ |

## Quy ước

- Deal là nguồn sự thật; lệnh và chỉ số luôn tính lại từ deal.
- Thời gian MT5 là giờ server, lưu nguyên; quy đổi sang giờ địa phương khi hiển thị (`time::Clock`).
- Luồng nền (bridge, cloud) không chạm signal của UI: chúng tăng `Revision`, UI tải lại.
- Chất lượng code theo [BeautifulRust](https://github.com/morganlinton/BeautifulRust):
  `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` sạch trước mỗi commit.
