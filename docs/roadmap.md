# Lộ trình

Ngày là ước lượng cho một người làm toàn thời gian; mỗi mốc kết thúc khi đạt tiêu chí hoàn thành.

| Mốc | Phạm vi | Dự kiến | Tiêu chí hoàn thành |
|---|---|---|---|
| **v0.0 Khung** ✅ | Workspace, domain, gom deal → lệnh, thống kê, khung bridge/cloud/import/Notion, app NSH với shell và trang placeholder, EA, Python, CI | 21/11/2026 | `clippy -D warnings` và test sạch |
| **v0.1 Dữ liệu MT5** | Truy vấn DB, bridge chạy thật, EA và Python đầy đủ, import HTML/XLSX, trang Lệnh và chi tiết lệnh (nhật ký), Tổng quan với chỉ số và equity curve | 12/12/2026 | Lệnh đóng hiện trong app ≤ 5 giây; số liệu khớp báo cáo MT5 |
| **v0.2 Phân tích & kỷ luật** | Lịch P/L, Phân tích theo symbol/giờ/phiên/setup, quy tắc và vi phạm, checklist, nhật ký ngày, import Notion CSV; nhật ký nâng cao: form tuỳ biến kiểu Notion, kế hoạch lúc vào vs thực tế (popup khi EA báo lệnh mới), hạng setup & điểm thực thi, lý do thoát, playbook, chi phí của lỗi, nhiều ảnh chart (EA tự chụp), lệnh bỏ lỡ, tổng kết tuần/tháng + hàng chờ, tìm kiếm toàn văn + mẫu ghi chú | 26/12/2026 | Mọi trang placeholder đã thay bằng màn hình thật |
| **v0.3 Đám mây** ✅ | Đăng nhập Google/Microsoft, sync Google Drive/OneDrive (tự động mỗi 15 phút và khi đóng app), khôi phục sang máy mới, Notion API và file zip Notion kèm ảnh (nhật ký ghi bằng R, không có tiền); thẻ chia sẻ (ẩn số tiền) và báo cáo PDF tháng | 09/01/2027 | 2 máy sửa cùng lúc, sync xong không mất dữ liệu |
| **v0.4 Kết nối MT5 & copier** | Đăng nhập tài khoản MT5 ngay trong app (mật khẩu trong Windows Credential Manager), app tự chạy terminal MT5 portable ẩn cho từng tài khoản và điều khiển qua gói Python `MetaTrader5` (không EA); lệnh tự vào nhật ký, popup ghi kế hoạch khi có lệnh mới; xem và quản lý lệnh trong app; copier nhiều tài khoản (cách tính lot, đổi symbol, copy ngược, lệnh chờ, sửa SL/TP, đóng một phần). Đặc tả đang chốt | — | Đăng nhập xong, lệnh đóng hiện trong app ≤ 5 giây không cần mở MT5 |
| **v1.0 Phát hành** | Bộ cài Windows (`nsh package`), tự cập nhật, macOS, kiểm thử E2E, tài liệu người dùng | 23/01/2027 | Cài mới đến lệnh đầu tiên trong app dưới 10 phút |

## Đã làm trong v0.3

- **Sync nhiều máy** (`crates/cloud`, `crates/core::sync`): nhật ký thay đổi theo từng trường, gộp theo lần sửa mới nhất,
  snapshot hằng tuần, khôi phục sang máy mới; Google Drive `appDataFolder` và OneDrive `approot` qua OAuth + PKCE.
- **Màn Đăng nhập & sync** (`app/src/pages/cloud.rs`, `app/src/cloud.rs`): đăng nhập/đăng xuất, Sync ngay, tự động mỗi
  15 phút và khi đóng app, danh sách thiết bị, "Khôi phục sang máy này"; chế độ thử `TRADELENS_CLOUD=memory`.
- **Import Notion**: API (token trong kho khoá), file `.zip` kèm ảnh, nhận diện cột theo nội dung (không theo tên cột),
  nhật ký ghi bằng R (không có tiền: `Deal::result_r`, thống kê theo R).
- **Chia sẻ**: thẻ ảnh (ẩn số tiền) và báo cáo PDF tháng.

## Để sau (chưa xếp lịch)

- **Tóm tắt tuần bằng AI** (tắt mặc định, key riêng): người dùng quyết định chưa cần; crate `tradelens-cloud::ai` và thẻ
  tắt ở Tổng kết giữ nguyên, chưa nối.
- Mã hoá đầu cuối bằng passphrase, dọn file `changes/` cũ, Dropbox (xem `docs/cloud-sync.md`).

## Thay đổi 23/11/2026: bỏ EA và bridge

EA `TradeLensSync`, crate `tradelens-bridge` (HTTP 127.0.0.1:7878, token) và `docs/sync-api.md` đã xoá
để không phải bảo trì; thay bằng kết nối trực tiếp của v0.4. Cho tới đó lệnh vào app bằng import báo
cáo HTML/XLSX hoặc file JSON của `python/tradelens_sync.py --out`. Bỏ luôn ảnh chart EA tự chụp (ảnh
chỉ còn thêm tay); popup ghi kế hoạch chỉ mở từ chi tiết lệnh cho tới v0.4. Các dòng về EA/bridge ở
dưới là lịch sử.

## Việc của v0.1, theo thứ tự

1. `core::db`: accounts, upsert_batch, deals, journal, rules, checklist, day notes (+ test).
2. `core::import`: đọc bảng HTML/XLSX, bộ file mẫu MT5 trong `crates/core/tests/fixtures/`.
3. `bridge::server`: route, token, upsert, revision; test bằng HTTP thật trên cổng ngẫu nhiên.
4. EA `Json.mqh` + `OnTimer`, Python `account_info`.
5. App: store đọc DB qua `blocking`, tải lại khi revision đổi; trang Lệnh (data_table), chi tiết lệnh, Tổng quan (plot).

## Việc nhật ký nâng cao (v0.2), theo thứ tự

Phần lõi đã có trong `crates/core` (model, migration 0003–0008, test); còn lại là truy vấn và màn hình.

1. Form tuỳ biến: `FormLayout` + `FieldDef` → `property_list`; màn Tuỳ biến form (`form-editor`), đổi kiểu qua `form::convert` có xác nhận.
2. Chi tiết lệnh: kế hoạch vs thực tế (`journal::review`), hạng setup, lý do thoát (EA gợi ý từ `DEAL_REASON`), gallery nhiều ảnh (`attachments::import`).
3. Popup ghi kế hoạch (`entry-plan-prompt`), mở bằng "Viết kế hoạch" ở chi tiết lệnh (EA đã bỏ nên không còn sự kiện lệnh vừa mở và không có "Ghi sau").
4. Playbook (`playbook`) và `metrics::mistake_cost` trên Phân tích.
5. Lệnh bỏ lỡ (`missed-trades`), tổng kết tuần/tháng + hàng chờ (`review`), tìm kiếm FTS5 (`search`).
