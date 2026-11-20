# Lộ trình

Ngày là ước lượng cho một người làm toàn thời gian; mỗi mốc kết thúc khi đạt tiêu chí hoàn thành.

| Mốc | Phạm vi | Dự kiến | Tiêu chí hoàn thành |
|---|---|---|---|
| **v0.0 Khung** ✅ | Workspace, domain, gom deal → lệnh, thống kê, khung bridge/cloud/import/Notion, app NSH với shell và trang placeholder, EA, Python, CI | 21/11/2026 | `clippy -D warnings` và test sạch |
| **v0.1 Dữ liệu MT5** | Truy vấn DB, bridge chạy thật, EA và Python đầy đủ, import HTML/XLSX, trang Lệnh và chi tiết lệnh (nhật ký), Tổng quan với chỉ số và equity curve | 12/12/2026 | Lệnh đóng hiện trong app ≤ 5 giây; số liệu khớp báo cáo MT5 |
| **v0.2 Phân tích & kỷ luật** | Lịch P/L, Phân tích theo symbol/giờ/phiên/setup, quy tắc và vi phạm, checklist, nhật ký ngày, import Notion CSV | 26/12/2026 | Mọi trang placeholder đã thay bằng màn hình thật |
| **v0.3 Đám mây** | Đăng nhập Google/Microsoft, sync Google Drive/OneDrive, khôi phục sang máy mới, Notion API | 09/01/2027 | 2 máy sửa cùng lúc, sync xong không mất dữ liệu |
| **v1.0 Phát hành** | Bộ cài Windows (`nsh package`), tự cập nhật, macOS, kiểm thử E2E, tài liệu người dùng | 23/01/2027 | Cài mới đến lệnh đầu tiên trong app dưới 10 phút |

## Việc của v0.1, theo thứ tự

1. `core::db`: accounts, upsert_batch, deals, journal, rules, checklist, day notes (+ test).
2. `core::import`: đọc bảng HTML/XLSX, bộ file mẫu MT5 trong `crates/core/tests/fixtures/`.
3. `bridge::server`: route, token, upsert, revision; test bằng HTTP thật trên cổng ngẫu nhiên.
4. EA `Json.mqh` + `OnTimer`, Python `account_info`.
5. App: store đọc DB qua `blocking`, tải lại khi revision đổi; trang Lệnh (data_table), chi tiết lệnh, Tổng quan (plot).
