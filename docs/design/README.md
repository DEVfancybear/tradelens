# Thiết kế UI TradeLens — chuẩn để code

Đây là **nguồn sự thật cho giao diện**. Agent hay người làm màn nào thì đọc mục của màn đó,
mở `screens/<màn>.html` (HTML tĩnh, mở bằng trình duyệt) và `screenshots/<màn>.png`, rồi dựng
bằng NSH theo `app/DESIGN.md` và component trong `app/src/ui/`.

- Canvas gốc (chỉnh sửa, comment): https://claude.ai/artifact/2eJs5MYNAvzzTXPazbCDMW
- HTML ở đây là **bản tham chiếu**, không phải code của app: không copy HTML/CSS sang Rust; dựng lại
  bằng `nsh_base` + `app/src/ui`. Số liệu trong thiết kế là **dữ liệu mẫu**.
- Thiết kế đổi trên canvas thì xuất lại vào thư mục này trong cùng commit với code liên quan.

## Màn hình

| Màn | File thiết kế | Route (`app/src/nav.rs`) | Trang (`app/src/pages/`) | Mốc |
|---|---|---|---|---|
| Tổng quan | `dashboard` · `dashboard-light` | `Route::Dashboard` | `dashboard.rs` | v0.1 |
| Lệnh | `trades` | `Route::Trades` | `trades.rs` | v0.1 |
| Chi tiết lệnh & nhật ký | `trade-detail` | đẩy từ Lệnh (stack) | `trade_detail.rs` (mới) | v0.1 |
| Nhập lệnh & nhật ký thủ công | `manual-entry` | đẩy từ Lệnh / phím Ctrl+N | `manual_entry.rs` (mới) | v0.1 |
| Lịch P/L | `calendar` | `Route::Calendar` | `calendar.rs` | v0.2 |
| Phân tích | `analytics` | `Route::Analytics` | `analytics.rs` | v0.2 |
| Nhật ký ngày | `daily-journal` | `Route::Journal` | `journal.rs` | v0.2 |
| Tổng kết & xem lại (tuần/tháng, hàng chờ, tóm tắt AI, mẫu ghi chú) | `review` | `Route::Review` | `review.rs` | v0.2 |
| Lệnh bỏ lỡ | `missed-trades` | `Route::Missed` | `missed.rs` | v0.2 |
| Playbook | `playbook` | `Route::Playbook` | `playbook.rs` | v0.2 |
| Tuỳ biến form (kiểu Notion) | `form-editor` | đẩy từ Cài đặt / "Tuỳ biến form" ở chi tiết lệnh | `form_editor.rs` (mới) | v0.2 |
| Popup ghi kế hoạch (khi có lệnh mới từ v0.4; nay mở từ chi tiết lệnh) | `entry-plan-prompt` | cửa sổ phụ / `dialog` trên mọi màn | `entry_plan.rs` (mới) | v0.2 |
| Chia sẻ kết quả (thẻ ảnh, PDF tháng) | `share-card` | `dialog` từ chi tiết lệnh / Tổng kết | `share.rs` (mới) | v0.3 |
| Tìm kiếm toàn văn | `search` | `command_palette`, phím Ctrl+K | `search.rs` (mới) | v0.2 |
| Tài khoản & kỷ luật | `accounts-discipline` | `Route::Accounts` | `accounts.rs` | v0.2 |
| Nguồn dữ liệu (MT5, Notion) | `data-sources` | `Route::DataSources` | `data_sources.rs` | v0.1 (Notion v0.2) |
| Đăng nhập & sync | `cloud-sync` | `Route::Cloud` | `cloud.rs` | v0.3 |
| Cài đặt | `settings` | `Route::Settings` | `settings.rs` | v0.1 |

## Khung chung (mọi màn)

- **Sidebar 236 pt**: logo; ô tìm kiếm (Ctrl K); nhóm *Giao dịch* (Tổng quan, Lệnh, Lịch P/L, Phân tích,
  Nhật ký ngày, Tổng kết & xem lại, Lệnh bỏ lỡ), *Chiến lược* (Playbook), *Quản lý* (Tài khoản & kỷ luật, Nguồn dữ liệu, Đăng nhập & sync), *Hệ thống* (Cài đặt); mục đang chọn
  có nền `sidebar-accent` + thanh nhấn 3 pt bên trái; chân sidebar là thẻ trạng thái kết nối MT5 và sync.
- **Header trang**: tiêu đề 24/600, một dòng mô tả `muted-foreground`, hành động bên phải
  (tối đa một nút `Primary`).
- **Nội dung**: lề 32, khoảng giữa thẻ 24; bố cục hai cột (chính `flex 999 1 560`, phụ `flex 1 1 320–420`)
  tự xuống dòng khi cửa sổ hẹp.

## Component cần có trong `app/src/ui/`

| Component | Dùng ở | Dựng trên `nsh_base` | Trạng thái |
|---|---|---|---|
| `page`, `card`, `stat_tile` | mọi màn | `view`, `scroll`, `text` | có (`parts.rs`) |
| `action` Primary / Secondary / Ghost, `action_icon`, nút chỉ có icon `icon_action` (cần `accessibility_label`) | mọi màn | `button` | có |
| `sidebar_row` | shell | `button` | có |
| `icon` (bộ icon nét 1.75) | mọi màn | `svg` | có (`icons.rs`) |
| `pnl_text` (màu + dấu +/−, font mono, căn phải) | bảng, KPI, lịch | `text` | v0.1 |
| `chip` (neutral / profit / loss / primary / warn) | Buy/Sell, setup, trạng thái | `view` + `text` | v0.1 |
| `segmented` | khoảng thời gian, Buy/Sell, chế độ | `toggle_group` | v0.1 |
| `select_field`, `text_field`, `number_field`, `textarea` | form | `select`, `input`, `number_input` | có |
| `tag_picker` (chọn nhiều, + Thêm) | setup/tag/lỗi/cảm xúc | `toggle` | có |
| `rating` (5 sao) | chi tiết lệnh, nhật ký ngày | `button` | có |
| `switch_row`, `setting_row` | quy tắc, cài đặt, sync | `switch` | có (`forms.rs`) |
| `checklist` (và `check_box`) | chi tiết lệnh, nhập tay, tài khoản | `checkbox` | có (`forms.rs`) |
| `trades_table` | Lệnh, Tổng quan | `data_table` (ảo hoá) | v0.1 |
| `equity_chart`, `bar_chart`, `diverging_bars`, `heatmap`, `histogram`, `r_curve_chart` | Tổng quan, Phân tích | `nsh_base::plot` (Line, Area, Bar); `heatmap` và `diverging_bars` là `view` | có (`chart.rs`, `charts.rs`, `r_chart.rs`) |
| `month_grid`, `mini_month` | Lịch P/L, Tổng quan | `view` grid | có (`month_grid.rs`) |
| `dropzone`, `image_slot` | Nguồn dữ liệu, chi tiết lệnh | `view` + `pick_paths`, kéo thả file | có (`files.rs`) |
| `progress_rule` (thanh ngưỡng quy tắc) | Tổng quan, Tài khoản | `progress` | có (`forms.rs`) |
| `property_list` + `property_row` (kéo ⋮⋮ đổi thứ tự, glyph kiểu, tên bấm để sửa, giá trị theo `FieldKind`, nhãn "riêng" cho thuộc tính tự thêm, dòng thuộc tính ẩn, "+ Thêm thuộc tính") | chi tiết lệnh, nhập tay, nhật ký ngày, lệnh bỏ lỡ | `view` + `drag` + `input` | có (`properties.rs`) |
| `property_popover` (đổi tên, kiểu, lựa chọn; Nên điền / Đưa vào Phân tích / Nhân bản / Ẩn / Xoá) | chi tiết lệnh | `popover` | có (trong `properties.rs`, mở từ tên thuộc tính) |
| `form_editor` (tab theo `FieldScope`, bảng thuộc tính, panel sửa, chọn kiểu, khôi phục đã xoá) | Tuỳ biến form | `tabs`, `data_table`, `alert_dialog` khi đổi kiểu mất dữ liệu | v0.2 |
| `gallery` (nhiều ảnh, chú thích, kéo đổi thứ tự, ô "Thêm ảnh" nhận chọn nhiều / kéo thả / Ctrl+V) | chi tiết lệnh, playbook, lệnh bỏ lỡ | `view` grid + `pick_paths(multiple)` + clipboard | có (`gallery.rs`) |
| `plan_vs_actual` (bảng kế hoạch / thực tế / chênh) | chi tiết lệnh | `view` grid | v0.2 |
| `queue_row`, `mistake_cost_bar` | Tổng kết, Phân tích | `view` + `progress` | `mistake_cost_bar` có (`charts.rs`); `queue_row` v0.2 |
| `share_card` (render thành PNG theo `report::Privacy`) | Chia sẻ | không phải component: `core::report::render_png` dựng SVG rồi rasterize bằng `resvg`; hộp thoại ở `pages/share.rs` chỉ hiện ảnh (`image`) | v0.3 |
| `command_palette` (nhóm kết quả, tô sáng từ khoá, bỏ dấu) | Tìm kiếm | `command` | v0.2 |

## Token (khớp `app/theme.json`)

| Vai trò | Dark | Light |
|---|---|---|
| background / card / border | `#0b0e14` / `#121722` / `#232b3a` | `#f6f7f9` / `#ffffff` / `#e3e7ee` |
| foreground / muted-foreground | `#e6eaf2` / `#8a94a7` | `#0f172a` / `#5b6475` |
| primary (teal) | `#2dd4bf` | `#0d9488` |
| profit / profit-soft | `#34d399` / `#0f2e25` | `#059669` / `#d1fae5` |
| loss / loss-soft | `#f87171` / `#341417` | `#dc2626` / `#fee2e2` |
| warning | `#fbbf24` | `#d97706` |

Chữ: Inter (giao diện), JetBrains Mono (số); đóng gói font vào `app/assets/fonts/` và đăng ký
qua `Root::font` ở `root()` (và `registry.assets(..)` cho `nsh package`) vì desktop không tải Google Fonts.

## Quy tắc khi code theo thiết kế

1. Thuộc tính trong form không cố định: đọc `FormLayout` + `FieldDef` từ DB rồi vẽ; thiết kế chỉ là
   một cấu hình mẫu. Thuộc tính có sẵn không xoá được (chỉ ẩn/đổi tên); xoá thuộc tính riêng hay lựa chọn
   chỉ lưu trữ, dữ liệu cũ giữ nguyên.
2. Lãi/lỗ luôn có **cả** màu và dấu (+/−); số dùng mono, căn phải trong bảng.
3. Mỗi màn đúng một nút Primary; hành động phá huỷ dùng `alert_dialog` xác nhận.
4. Trạng thái rỗng / đang tải / lỗi cho mọi danh sách và biểu đồ (thiết kế chưa vẽ: dùng `empty_state`
   một dòng + một hành động).
5. Bàn phím: mọi thao tác dùng được bằng phím; phím tắt ở màn Cài đặt.
6. Tôn trọng *reduce motion*; thời gian chuyển động lấy từ `ui::motion`.
7. Lệch thiết kế vì giới hạn của NSH: ghi lý do trong commit và thêm dòng vào bảng dưới.

| Màn | Lệch so với thiết kế | Lý do |
|---|---|---|
| Lệnh | Chưa có cột chọn nhiều lệnh (checkbox), "Gắn tag", "Xuất CSV", nút Import và chip "Đang lọc"; chưa có bộ lọc ngày (chỉ symbol, setup, thắng/thua, Buy/Sell, tìm chữ và sắp xếp theo cột) | Chọn nhiều và xuất CSV cần hộp thoại lưu file, làm sau; "Xoá lọc" thay chip "Đang lọc". "Thêm lệnh" (Primary, cạnh bộ chọn tài khoản) và Ctrl+N đã mở màn nhập lệnh tay |
| Lệnh | Bấm một dòng hoặc nút ở cột Symbol mở chi tiết lệnh; Enter trên dòng đang chọn không mở | `data_table` của NSH 0.6.1 không có sự kiện Enter, nên đường dùng phím là nút Symbol (nhận Tab và Enter) |
| Chi tiết lệnh | Không có biểu đồ giá của lệnh và nút Mở trên MT5; nút "Chia sẻ" (Secondary, cạnh Lưu) mở hộp thoại chia sẻ, tắt khi lệnh còn mở; lưu bằng nút Lưu (tự lưu khi chuyển sang lệnh khác hoặc mở Tuỳ biến form) thay cho "tự động lưu"; ảnh chart thì lưu ngay khi thêm, xoá, đổi thứ tự | Biểu đồ giá cần nguồn nến (chưa có); hộp thoại chia sẻ: xem dòng "Chia sẻ kết quả" |
| Chi tiết lệnh | "Kế hoạch lúc vào vs thực tế" có thêm hàng Risk và nút "Sửa kế hoạch" mở lại popup ghi kế hoạch; "Lý do thoát lệnh" lấy gợi ý từ lý do của deal đóng và chỉ ghi cùng lần lưu kế tiếp (mở xem lệnh không tạo nhật ký); "Đúng kế hoạch" là ô chọn Chưa đánh giá / Có / Không; thuộc tính "Ảnh chart" trong danh sách chỉ đếm ảnh, thư viện ảnh vẫn là khối bên trái; không có nhãn "Thuộc tính kiểu Notion" | Danh sách thuộc tính vẽ từ `FormLayout` của người dùng nên mọi thuộc tính có sẵn là một hàng; nút "Tuỳ biến form" thay "Đã lưu tự động · Tuỳ biến form" vì lưu bằng nút |
| Tuỳ biến form | Không có "Xem trước form" và nút Huỷ / Lưu của panel sửa (mọi thay đổi áp dụng và lưu ngay như danh sách thuộc tính ở chi tiết lệnh); không có tab Playbook (Playbook không phải `FieldScope`); lựa chọn không có màu, không đếm số lệnh dùng và chưa kéo đổi thứ tự; "Khôi phục thuộc tính đã xoá" mở danh sách ngay dưới bảng; thêm nút Xong (Primary) và Đặt lại mặc định | Áp dụng ngay để khớp popover của `property_list`; màu và đếm lệnh cần thêm truy vấn đếm theo lựa chọn, làm khi có màn Danh mục |
| Popup ghi kế hoạch | Là `dialog()` native ở giữa cửa sổ, nền tối bằng token `background` mờ 72%; không có giờ mở lệnh, nút "+ Thêm" lựa chọn, và dòng đầu không ghi "risk $ (%)"; Hạng setup và Cảm xúc là ô chọn thay chip; ✕ và Escape đều là "Ghi sau"; cảnh báo vượt quy tắc (`ui::parts::warning_banner`) có thể hiện nhiều dòng, mỗi quy tắc đang bật bị phá một dòng (risk/lệnh, số lệnh/ngày, lỗ/ngày, chuỗi thua), tính bằng `rules::evaluate` với lệnh coi như vừa vào và đọc lại khi ô Risk đổi | `dialog()` không có kích thước riêng; sự kiện của EA không mang giờ mở; thiết kế chỉ vẽ cảnh báo risk |
| Nhập lệnh tay | Chỉ có tab "Lệnh mới + nhật ký" (nhật ký cho lệnh có sẵn làm ở chi tiết lệnh, ghi chú ngày ở Nhật ký ngày); giờ vào / ra chọn bằng lịch kèm ô giờ (`date_picker`, popup của NSH `DatePicker`); Lãi/lỗ bắt buộc nhập vì app chưa biết contract size để tự tính; không có khung "Nhập nhanh" và dán dòng MT5; ảnh nằm ở thẻ "Ảnh & đính kèm" bằng `gallery` (lưu vào lệnh khi bấm Lưu); "Lưu lệnh" mở trang chi tiết lệnh vừa lưu; Ctrl+Enter lưu | Date picker và bộ phân tích dòng MT5 chưa có; Ctrl+Shift+Enter của thiết kế thay bằng nút "Lưu & nhập tiếp" |
| Chi tiết lệnh | Mở bằng trạng thái `nav::open_trade` đè lên route Lệnh, không dùng `nav_stack` native | Chỉ cần một tầng; nút "‹ Lệnh" quay lại danh sách và giữ nguyên bộ lọc |
| Tổng quan | Biểu đồ chỉ có hai tab Balance và R (không có Equity); nút "Xem phân tích" thay "Phân tích" ở thẻ Theo symbol; thẻ lịch tên "Tháng M/YYYY" của tháng đang hiện (tháng hiện tại, hoặc tháng gần nhất có lệnh khi tháng này trống, như trang Lịch P/L) kèm liên kết "Mở lịch"; lưới chỉ T2–T6, mỗi ô ghi kết quả của ngày (tiền không ký hiệu, hoặc R khi ngày không có tiền, "·" khi không có lệnh), ô thuộc tháng khác viền đứt, tuần không có ngày nào của tháng bị bỏ (không còn hàng trống dưới tiêu đề thứ) và ô không có nút bấm; thêm nút Primary "Thêm lệnh" (cũng Ctrl+N) ở header; khi khoảng thời gian không có lệnh mọi chỉ số là "—"; Kỷ luật hôm nay lấy quy tắc của tài khoản đầu tiên (trong phạm vi đang chọn) có đặt giới hạn, "vi phạm" đếm theo `rules::evaluate` | Equity cần lãi/lỗ đang mở mà nguồn dữ liệu chưa gửi theo thời gian; nút "Phân tích" trùng nhãn với mục sidebar nên test và trình đọc màn hình không phân biệt được; quy tắc thuộc từng tài khoản còn thiết kế chỉ vẽ một bộ |
| Lịch P/L | Chưa có nút "Xuất ảnh tháng"; "Mở nhật ký ngày" mở màn Nhật ký đúng ngày đang chọn và tài khoản đang lọc (`nav::show_journal_day`); ghi chú ngày ở thẻ bên phải là ghi chú của tài khoản đang lọc (của tất cả tài khoản khi chọn Tất cả), như màn Nhật ký ngày; ô lịch là `view` lưới (không dùng `plot` Cell); chọn ngày của tháng khác chuyển luôn sang tháng đó | Xuất ảnh là v0.3 (chia sẻ); màn Nhật ký ngày do mục khác dựng; mỗi ô mang chữ để đọc và để test tìm |
| Phân tích | Chưa có "So sánh với 90N trước"; khoảng thời gian dùng chung bộ lọc 7N/30N/90N/YTD/Tất cả với Tổng quan; heatmap giờ × thứ và "Theo symbol" dựng bằng `view` (không dùng `plot` Cell/Bar); bảng "Theo giờ vào lệnh" thay biểu đồ giờ; giờ tính theo "Giờ hiển thị" trong Cài đặt (giờ của bạn, mặc định là giờ máy, hoặc giờ server), riêng phiên giao dịch theo UTC | So sánh kỳ trước cần đọc hai khoảng cùng lúc, làm sau; mỗi ô và hàng mang chữ để đọc và để test tìm |
| Tổng quan | Tiền hiện kèm ký hiệu đô la khi tiền tệ là USD (hoặc tài khoản chưa báo), còn lại dạng `123.45 EUR` | Chưa có cài đặt tiền tệ; thiết kế chỉ vẽ USD |
| Nguồn dữ liệu | Không có thẻ Bridge / Kết nối EA, hướng dẫn cài EA, ô Địa chỉ / Terminal kết nối và nút "Kiểm tra kết nối"; thay bằng thẻ "Script Python (MetaTrader5)" chỉ lệnh ghi file JSON để import. Hai ô kéo thả riêng (báo cáo MT5 HTML/XLSX/JSON và CSV Notion) thay cho một ô chung; "Nhật ký nhận dữ liệu" là "Nhật ký import" đọc từ bảng `import_log`; chân sidebar chỉ còn dòng Cloud | EA và bridge đã bỏ (23/11/2026, `docs/roadmap.md`); kết nối MT5 trực tiếp là v0.4, thiết kế lại thẻ này khi làm |
| Nguồn dữ liệu | **Import Notion không có bảng ghép cột** (cột Notion / Trường TradeLens / Ví dụ, "Xem trước 5 dòng", chọn "Lệnh trùng với MT5"): chọn file CSV → app tự nhận cột (`import::notion::infer`) → bản xem trước ngắn (số dòng, trường nhận ra, thuộc tính sẽ tạo kèm kiểu, cảnh báo, dòng bị bỏ qua) → một nút "Nhập N lệnh". Chỉ chọn được tài khoản đích (mặc định "Nhật ký Notion"). Chưa có tab "Notion API" | Quyết định của người dùng: nhập hoàn toàn tự động, không ghép cột (`docs/notion-import.md`); Notion API là v0.3. Lệnh trùng MT5 luôn chỉ gắn nhật ký vào lệnh MT5 |
| Playbook | Số liệu tính trên toàn bộ lịch sử (không phải 90 ngày); không có nút "+" trong thẻ danh sách (chỉ nút "Thêm playbook" ở tiêu đề); thêm "Xoá playbook" (có xác nhận) và "Bỏ lưu trữ" trong form sửa; Huỷ setup khi là một ô nhiều dòng (mỗi dòng một ý); ô màu là token `chart-1…5` theo thứ tự; checklist chỉ để xem (không tick) | Số liệu theo kỳ cần bộ lọc ngày dùng chung; một nút Primary mỗi màn; `Playbook.invalidation` là một chuỗi |
| Lệnh bỏ lỡ | Ô "Thời điểm" chọn bằng lịch kèm ô giờ (`date_picker`); thêm ô "Giá sau đó" (giá thực tế, để tính "Nếu đã vào"); Chiều là nút Buy/Sell; "+ Thêm" lý do là ô gõ + Enter; số liệu tính trên mọi lệnh bỏ lỡ (không phải 30 ngày); "Bỏ lỡ đúng" chỉ đếm lý do "Đã chạm giới hạn lệnh/ngày" (`limit_reached`) vì chưa nối với quy tắc kỷ luật thật; bấm giờ ở dòng bảng để sửa lệnh đó | Cần giá sau setup để tính R; `date_picker` chưa dựng; quy tắc kỷ luật là mục Tài khoản & kỷ luật |
| Tổng kết & xem lại | Tóm tắt AI là thẻ tắt ghi "Chưa làm" (chưa dựng, chưa có hạn); nút "Chia sẻ" đứng cuối hàng công cụ ở header và mở thẻ ảnh của kỳ đang xem; thêm thẻ "Báo cáo tháng" (chỉ ở tab Tháng: chọn mức hiển thị, nút "Xuất PDF tháng"); không có "Đánh dấu đã xong" và lưu tự động (thay bằng nút "Lưu tổng kết" và dòng "Chưa lưu / Đã lưu"); ô "Tuân thủ quy tắc" thay bằng "Đã ghi nhật ký" (chưa có đánh giá quy tắc theo kỳ); "Chi phí lỗi" là tổng lỗ của các lệnh có gắn lỗi trong nhật ký; chỉ có hai mẫu ghi chú (Tổng kết tuần, Tổng kết tháng — sửa được, đặt lại mặc định) chứ chưa có "Trước phiên", "Sau chuỗi thua", "+ Mẫu mới"; thêm chọn tài khoản và điểm kỷ luật 1–5; hàng chờ tính trên mọi lệnh (không theo kỳ), mỗi nhóm có một nút mở lệnh đầu tiên | `core::review` có một mẫu cho mỗi kỳ; hàng chờ theo kỳ chưa có yêu cầu |
| Tìm kiếm | Bảng tìm là lớp phủ trong cửa sổ (không phải `dialog` native); các nhãn lọc theo loại nằm trên ô gõ (khe `header` của `command`); ô tìm kiếm ở sidebar là nút mở bảng chứ không gõ trực tiếp; kết quả Nhật ký ngày mở route Nhật ký ngày qua `nav::show_journal_day` (màn đó đọc ngày và tài khoản được yêu cầu rồi xoá yêu cầu) | `command` của NSH chỉ cho chèn nội dung trên hoặc dưới cả ô gõ; lớp phủ chạy được trong test không đầu |
| Mọi màn | Inter và JetBrains Mono được đóng gói ở `app/assets/fonts/` và đăng ký ngay ở `nsh_app_tradelens::root()` (`Root::font`) cùng token của `theme.json`; số (KPI, bảng, lịch, nhãn trục biểu đồ `font-mono tabular-nums`) dùng `font-mono` | `cargo run` chạy nền tảng không kèm UI set của app nên `registry.assets(..)` không bao giờ được đọc và mọi chữ rơi về font hệ thống; registry vẫn giữ cho `nsh package` |
| Mọi màn | Hộp xác nhận (xoá, thay dữ liệu, đổi kiểu mất giá trị) là `dialog()` của cửa sổ (`ui::confirm`), không phải alert hệ thống: Huỷ là nút Secondary, hành động phá huỷ dùng token `destructive` / `destructive-foreground`, Escape huỷ, Enter xác nhận (nút đang có focus nhận phím trước, nên Tab tới Huỷ rồi Enter là huỷ), phần trang phía sau bị che | Alert hệ thống vẽ nút theo màu của hệ điều hành, không theme được |
| Mọi màn | Nút bị tắt có một dáng chung cho mọi biến thể: nền `muted` (nút Ghost không nền), chữ `muted-foreground`, mờ 60%, không hover, không hiệu ứng bấm; liên kết ("Xem tất cả", "Mở lịch", "Xem phân tích", "Xem lệnh", "Sửa kế hoạch") là `ui::controls::link`: chữ `primary` 12 pt, gạch chân khi rê chuột, con trỏ bàn tay | Thiết kế không vẽ nút tắt; trước đây liên kết là nút Ghost chữ trắng đậm |
| Mọi màn | Mỗi trang (và mỗi trang lệnh mở từ danh sách) là một phần tử có `key` riêng nên luôn mở ở đầu trang, không thừa vị trí cuộn của trang trước | NSH giữ vị trí cuộn của nút cuộn được dựng lại tại chỗ |
| Chi tiết lệnh | Thư viện ảnh không còn badge "Tự chụp · vào/ra lệnh" và dòng đếm chỉ ghi "N ảnh"; breadcrumb ghi tên tài khoản không kèm mã đăng nhập khi đó chỉ là khoá của importer ("Nhật ký Notion · XAUUSD") | EA đã bỏ; tên "notion" chỉ là khoá file import |
| Thuộc tính | Kiểu Ngày chọn bằng lịch (`date_picker`), có nút Xoá ngày; popover sửa thuộc tính áp dụng ngay (không có Huỷ / Lưu), tên và lựa chọn lưu khi Enter hoặc rời ô | Lịch là `DatePicker` của NSH bọc trong `ui::date_picker`; mỗi lần gõ tên không được ghi DB |
| Thư viện ảnh | Mỗi ảnh có nút ✕ và tay nắm ⋮⋮ (kéo, hoặc phím mũi tên khi tay nắm đang focus); Ctrl+V nhận file ảnh đã copy hoặc ảnh trong clipboard | Thiết kế chỉ vẽ ⋮⋮; phím tắt và nút xoá cần cho thao tác bằng bàn phím |
| Mọi màn | Bộ icon có thêm `ChevronRight` (đối xứng `ChevronLeft`) | Thiết kế dùng một chiều cho cả hai nút trước / sau |
| Tài khoản & kỷ luật | Không có nút "Thêm tài khoản" và ô "Loại tài khoản" (Thật / Prop firm / Demo) / "Ghi chú"; thẻ tài khoản không có thanh mục tiêu lợi nhuận; checklist chỉ sửa chữ, xoá, thêm (chưa kéo đổi thứ tự); khối "Hôm nay" nằm cột phải | Tài khoản do EA hoặc import tạo ra; model `Account` chưa có loại và ghi chú; thứ tự checklist làm khi có `reorder` cho danh sách văn bản |
| Tài khoản & kỷ luật | Mỗi quy tắc là một dòng có công tắc: bật mới hiện ô số, tắt thì ô số ẩn (giá trị cũ được giữ cho lần bật sau); "Lưu thông tin" là nút riêng | `Rules` dùng `None` cho quy tắc tắt; ô số của NSH báo theo từng phím nên lưu bằng nút, không ghi mỗi lần gõ |
| Nhật ký ngày | Kế hoạch, tổng kết, tâm trạng nằm chung một thẻ "Kế hoạch và tổng kết" dựng bằng `property_list` (đúng thứ tự người dùng sắp trong form); chưa có thanh trượt giấc ngủ / tập trung / sẵn sàng, tin trong ngày, "Điểm kỷ luật ngày", thẻ "Thói quen ghi nhật ký" và nút "Dùng mẫu kế hoạch" | Form ngày do người dùng tuỳ biến (thuộc tính riêng thay cho các thanh trượt); mẫu kế hoạch thuộc màn Tổng kết, thói quen cần thống kê nhiều ngày |
| Nhật ký ngày | Lưu tự động mỗi 2 giây khi có thay đổi, khi đổi ngày hoặc tài khoản; nút "Lưu nhật ký" chỉ để lưu ngay | Ô nhập bám theo dữ liệu đọc nên cần lưu nháp riêng; tự lưu thay cho nút Lưu như thiết kế vì form ngày có thể rất dài |
| Cài đặt | Chỉ có Chế độ màu, Giờ hiển thị (giờ của bạn / giờ server), file dữ liệu, phím tắt, giới thiệu; chưa có màu nhấn, màu lãi/lỗ, mật độ, cỡ chữ, ngày bắt đầu tuần, định dạng số, ngôn ngữ, sao lưu / xuất / xoá dữ liệu, danh mục nhật ký, cập nhật tự động; "Tuỳ biến form" là nút tắt kèm chú thích | Ngoài phạm vi v0.2 của màn này; màn Tuỳ biến form do người khác làm. Giờ hiển thị áp dụng cho mọi màn (giờ lệnh, ngày, "hôm nay", giờ gõ ở nhập lệnh tay) qua `snapshot.zone` và `zone::today_on`, riêng phiên giao dịch (Á/Âu/Mỹ) luôn tính theo UTC, còn giờ ghi chép ("Đã lưu lúc", nhật ký bridge), giờ "Thời điểm" của lệnh bỏ lỡ và ngày trong file import luôn là giờ máy hoặc như trong file; cổng và token mới có hiệu lực khi mở lại ứng dụng |
| Chia sẻ kết quả | Không có nhóm chọn phạm vi Lệnh / Tuần / Tháng: phạm vi là nơi mở hộp thoại (nút "Chia sẻ" ở chi tiết lệnh cho một lệnh, ở Tổng kết cho tuần hoặc tháng đang xem); ba mức hiện số (Hiện tiền / Chỉ R / Ẩn số) là `segmented`, mặc định "Chỉ R"; ô "Tuân thủ" thay bằng "Profit factor" (chưa có điểm tuân thủ theo kỳ); thẻ lệnh có hàng Chiều / Thời gian giữ / Setup và ảnh chart (ảnh thoát lệnh trước) bên phải khi bật "Kèm ảnh chart"; "Ẩn số" chỉ còn từ Thắng / Thua / Hòa vốn, tên setup và hình dạng đường R (không số, không trục); thêm công tắc "Kèm ảnh chart" (chỉ lệnh) kèm lời nhắc ảnh chụp có thể hiện số tiền; "Lưu PNG" đổi thành "Lưu ảnh"; liên kết "Xuất báo cáo PDF tháng MM →" chỉ hiện khi chia sẻ một tháng; "Sao chép ảnh" ghi ảnh vào clipboard (chưa kiểm được trên máy thật, test không đầu không đọc lại clipboard ảnh) | Phạm vi theo nơi mở đỡ phải đọc lại hai nguồn dữ liệu trong một hộp thoại; thẻ vẽ bằng SVG + `resvg` trong `core` để test được ảnh trong cổng chất lượng (NSH render offscreen không chạy được không đầu) nên thẻ luôn theo giao diện tối của thiết kế, không theo chế độ màu của app |
| Báo cáo PDF tháng | Không có trong thiết kế: trang A4 sáng (token sáng, in được) gồm bìa (tài khoản, tháng, Net, tỷ lệ thắng, profit factor, số lệnh, drawdown, ngày xanh), đường balance (đường R tích luỹ khi không hiện tiền), lịch tháng, bảng playbook, ba lỗi tốn kém nhất, lệnh bỏ lỡ theo lý do, câu trả lời của tổng kết tháng; "Ẩn số" giữ số lệnh và tỷ lệ nhưng bỏ tiền và R | Thiết kế chỉ vẽ thẻ ảnh; PDF là báo cáo in nên theo token sáng |
| Đăng nhập & sync | Không có công tắc "Sync ảnh chart" (ảnh luôn sync khi có thư mục dữ liệu), "Mã hoá đầu cuối bằng passphrase" (để sau, `docs/cloud-sync.md`), nút "Tạo bản chụp ngay" và "Xuất file .db"; "Khôi phục từ Drive" là "Khôi phục sang máy này" (xác nhận mạnh, giữ `tradelens.before-restore.db`); biểu tượng nhà cung cấp là icon đám mây chung thay logo Google / Microsoft; thêm thông báo thiếu client id (nêu khoá cài đặt, biến môi trường, tài liệu), nhãn "Chế độ thử" khi `TRADELENS_CLOUD=memory`, nút Huỷ khi chờ trình duyệt; chân sidebar chỉ còn dòng Cloud (không có thẻ MT5) | Ảnh chart đi cùng nhật ký là một phần của sync nên không cần tách; mã hoá, chụp tay và xuất `.db` chưa có yêu cầu; logo cần tệp ảnh có bản quyền thương hiệu; chân sidebar theo bỏ EA (23/11/2026) |
| Nguồn dữ liệu | Thẻ "Import từ Notion" có thêm phần **Kết nối Notion (API)** (token, danh sách database, "Xem trước database") ngay dưới ô thả file thay cho tab "Notion API"; ô thả nhận cả `.zip`; bản xem trước liệt kê "Cột «X» → vai trò" (kết quả R, thắng/thua, ảnh…) và số ảnh tìm thấy trong zip; nhật ký ghi bằng R hiện tổng R, số tiền «—» ở Lệnh, Lịch, Nhật ký ngày | Một thẻ chung một luồng xem trước cho cả ba nguồn; thiết kế chưa có zip và nhật ký ghi bằng R |
| Mọi màn | Nhật ký tính bằng R (không tiền, không lot, không giá, ngày không giờ) hiện R ở chỗ tiền (thẻ Tổng quan "Net R", lịch, phân tích, nhật ký ngày, tổng kết, thẻ chia sẻ, PDF) và dấu «—» cho lot, giá, SL/TP, phí, thời gian giữ, giờ; khi có cả hai loại, số tiền chỉ tính trên lệnh có tiền và ghi rõ; lệnh chỉ có ngày bị loại khỏi heatmap giờ và bảng phiên | Thiết kế chỉ vẽ dữ liệu MT5 đầy đủ; quy tắc nằm ở `core` (`Trade::money_known/volume_known/prices_known/time_known`, `metrics::Basis`, `Gaps` trên deal do importer ghi) |
| Tổng quan | Lệnh gần đây theo cùng khoảng thời gian với các chỉ số; biểu đồ mở ở tab R khi không có tiền, tab Balance báo "xem tab R"; bảng có đúng bảy cột như thiết kế, cột Setup nhận phần chiều rộng còn lại | Bảng `data_table` có chiều rộng cột cố định nên đo chiều rộng thẻ rồi chia lại |
| Lịch P/L | Mở ở tháng gần nhất có lệnh khi tháng hiện tại trống ("Hôm nay" vẫn về hôm nay); nút chọn "Net $" đổi tên "Kết quả" vì ô hiện R khi ngày không có tiền | Nhật ký nhập từ quá khứ không nên mở ra một lịch trống |
| Nhật ký ngày, Tổng kết & xem lại | Bấm nhãn ngày (hoặc kỳ) mở lịch chọn ngày (`ui::date_picker`): tuần bắt đầu thứ 2, nút "Hôm nay", chấm ở ngày có lệnh hoặc ghi chú; ‹ › vẫn ở cạnh | Thiết kế chỉ có ‹ ›; chọn xa phải qua nhiều lần bấm |
| Mọi màn | Con trỏ bàn tay trên mọi thứ bấm được (nút, hàng sidebar, tab, chip chọn, ô lịch, thẻ chọn), mũi tên khi bị tắt, tay nắm kéo là `grab`; nút Ghost có nền khi rê chuột | Thiết kế HTML không mô tả con trỏ; làm ở mức component trong `app/src/ui/` |
| Cửa sổ | Cửa sổ mở 1280 × 800 pt, nhỏ nhất 1024 × 680 (khai báo ở `nsh.json` `window`, đọc bằng `nsh_app_tradelens::root()`); dữ liệu nằm ở `%APPDATA%\vn.dat.tradelens` (id của `nsh.json`), bản cũ ở `default` được chép sang một lần và để nguyên | Chạy bằng `cargo run` không đi qua công cụ `nsh` nên root phải tự mang id và cửa sổ; `window()` bọc ngoài `App` từng mở một cửa sổ thứ hai cỡ hộp thoại |
