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

## Bản phát hành

| Bản | Ngày | Có gì | Tải |
|---|---|---|---|
| **0.1.0** | 26/11/2026 | Bản Windows đầu tiên: nhật ký và phân tích (v0.1–v0.2), đám mây (v0.3), kết nối MT5 trực tiếp và quản lý lệnh (v0.4, chưa có copier), tự cập nhật | [Release v0.1.0](https://github.com/DEVfancybear/tradelens/releases/tag/v0.1.0): `TradeLens-setup.exe` (bộ cài), `TradeLens-0.1.0-windows-x86_64.zip` (gói app tự cập nhật dùng), `latest.json` + `.sig` |

Bộ cài chưa ký số nên Windows SmartScreen có thể cảnh báo (Thông tin thêm → Vẫn chạy). Trang giới thiệu
(`docs/index.html`, GitHub Pages) lấy bản mới nhất qua GitHub API; khi API lỗi, nút tải trỏ thẳng
`releases/latest/download/TradeLens-setup.exe`, nên bộ cài phải giữ tên đó.

## Tự cập nhật (làm sớm cho v1.0)

- Bản cài bằng `nsh package` tự kiểm tra bản mới 10 giây sau khi mở và mỗi 6 giờ (tắt ở Cài đặt › Về TradeLens ›
  "Tự động cập nhật", setting `update.auto = 0`), tải và kiểm tra bản mới ở nền, cài khi thoát app hoặc khi bấm
  "Khởi động lại để cập nhật". Theo cơ chế của NSH (`nsh-updater`, qua `crates/update`): manifest `latest.json` có chữ ký
  Ed25519 lấy từ `releases/latest/download/latest.json` của GitHub Releases (`"updater"` trong `app/nsh.json`).
  Bản chạy từ mã nguồn (`cargo run`, bản debug) chỉ kiểm tra khi bấm nút và chỉ trang phát hành, không thay exe.
- Phát hành: tăng `version` ở `Cargo.toml` (`[workspace.package]`) **và** `app/nsh.json` cho bằng nhau, rồi đẩy tag
  `v<version>` (vd. `v0.4.1`, không có tiền tố khác). `.github/workflows/release.yml` từ chối tag lệch phiên bản, chạy
  `nsh package --target windows` (ra `TradeLens-<version>-windows-x86_64.zip`), `nsh update-manifest` (ra `latest.json`,
  `latest.json.sig`) và `gh release create`. Draft và prerelease không được app đề nghị.
- Khoá ký (đã làm 26/11/2026): `nsh keygen keys/update` tạo cặp khoá; `keys/update.pub` nằm ở `"updater": { "publicKey" }`
  của `app/nsh.json`, `keys/update.secret` ở secret `TRADELENS_UPDATE_SIGNING_KEY` của repo (thư mục `keys/` không vào
  git; giữ bản sao file secret ở nơi an toàn, mất nó thì các bản sau không ký được). App nhận chỉ `latest.json` có chữ ký
  của khoá này; workflow từ chối phát hành khi thiếu secret ký.
- Workflow đã chạy thật cho v0.1.0: trên runner Windows có Inno Setup nên `nsh package` ra cả `TradeLens-setup.exe`.
  Secret OAuth hiện có Google; chưa có `TRADELENS_MICROSOFT_CLIENT_ID` nên bản phát hành chưa đăng nhập OneDrive được.

## Đã làm trong v0.4: kết nối MT5 (copier chưa làm)

- **Kết nối** (`crates/mt5`): mỗi tài khoản một bản MT5 portable chép từ terminal đã cài
  (`%APPDATA%\vn.dat.tradelens\mt5\terminals\<login>@<server>`), chạy ẩn bằng worker
  `python/tradelens_worker.py` (gói `MetaTrader5`, nói chuyện với app bằng JSON từng dòng qua stdin/stdout). Mỗi tài
  khoản một luồng `Link`: đăng nhập, đọc deal và lệnh đang mở mỗi 2 giây, tự kết nối lại sau 30 giây khi mất kết nối,
  dừng hẳn khi server từ chối mật khẩu. Deal ghi thẳng vào DB với `source = mt5`.
- **Thẻ "Kết nối MT5"** ở Nguồn dữ liệu: kiểm tra Python 3 và gói MetaTrader5 (nút "Cài gói MetaTrader5" chạy
  `pip install --user`), chọn terminal tìm được (theo `origin.txt` của thư mục dữ liệu MT5 và `Program Files`) hoặc
  chọn `terminal64.exe`; đăng nhập số tài khoản / mật khẩu / server; tài khoản chỉ được lưu (setting `mt5.accounts`,
  mật khẩu trong Windows Credential Manager) sau lần đăng nhập đầu thành công và tự kết nối khi mở app; danh sách lệnh
  đang mở; "Kết nối lại", "Xoá" (xoá cả bản terminal riêng).
- **Tự cài Python** (`crates/mt5/src/runtime.rs`, sau 0.1.0): máy chưa có Python thì lần đăng nhập đầu (hoặc nút "Cài Python
  tự động") tải gói embeddable Python 3.14.10 của python.org (12,6 MB) vào `%APPDATA%\vn.dat.tradelens\mt5\runtime\python`,
  bật `site`, giải nén wheel pip 26.3, rồi `pip install --require-hashes --only-binary=:all: --no-deps` MetaTrader5 5.0.6231
  và numpy 2.5.5. Mọi file đều ghim kích thước hoặc SHA-256 trong `runtime::Source::official` (lấy từ API của python.org và
  PyPI); tải bằng kho chứng chỉ của Windows (chạy được sau proxy công ty hay antivirus kiểm HTTPS). Không cần quyền admin,
  không đổi `PATH`. Python riêng hỏng hoặc khác phiên bản (file `.tradelens-python`) thì được cài lại; máy có Python thiếu
  gói thì chỉ cài gói (`--user`). App chỉ dò Python khi có tài khoản đã lưu hoặc khi mở thẻ Kết nối MT5.
  Đổi phiên bản: sửa `Source::official` và `VERSION`.
- **Lệnh mới → popup kế hoạch** (tắt bằng công tắc "Hỏi kế hoạch khi có lệnh mới", setting `mt5.plan_prompt = 0`).
- **Quản lý lệnh** (`pages/position.rs`): sửa SL/TP, đóng toàn bộ hoặc một phần (hỏi trước), mở từ thẻ kết nối và từ nút
  "Quản lý lệnh" ở chi tiết lệnh đang mở.
- Chế độ thử: `TRADELENS_MT5=memory` hoặc setting `mt5.mode = memory` dùng broker giả trong bộ nhớ (test màn hình).
- **Tin quan trọng** (`pages/news.rs`, `app/src/news.rs`): tin đỏ (impact High) trong lịch tuần của Forex Factory, lấy từ
  feed công khai `ff_calendar_thisweek.json` khi mở app (nếu lần tải cuối đã quá 30 phút), mỗi 30 phút và khi bấm
  "Tải lại"; lưu theo tuần ở `news_events` (không sync), tải lỗi thì giữ bản cũ và báo lỗi. Feed không có số Actual
  và chỉ có tuần hiện tại. Setting `news.url` (hoặc `TRADELENS_NEWS_URL`) đổi địa chỉ, `off` để tắt.
- **Còn lại của v0.4:** copier nhiều tài khoản (cách tính lot, đổi symbol, copy ngược, lệnh chờ, chốt an toàn) — chờ chốt
  đặc tả với người dùng; kiểm tra trên MT5 thật (máy dev chưa có Python và MT5).

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
