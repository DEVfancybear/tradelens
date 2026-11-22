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
| Popup ghi kế hoạch khi EA báo lệnh mới | `entry-plan-prompt` | cửa sổ phụ / `dialog` trên mọi màn | `entry_plan.rs` (mới) | v0.2 |
| Chia sẻ kết quả (thẻ ảnh, PDF tháng) | `share-card` | `dialog` từ chi tiết lệnh / Tổng kết | `share.rs` (mới) | v0.3 |
| Tìm kiếm toàn văn | `search` | `command_palette`, phím Ctrl+K | `search.rs` (mới) | v0.2 |
| Tài khoản & kỷ luật | `accounts-discipline` | `Route::Accounts` | `accounts.rs` | v0.2 |
| Nguồn dữ liệu (MT5, Notion) | `data-sources` | `Route::DataSources` | `data_sources.rs` | v0.1 (Notion v0.2) |
| Đăng nhập & sync | `cloud-sync` | `Route::Cloud` | `cloud.rs` | v0.3 |
| Cài đặt | `settings` | `Route::Settings` | `settings.rs` | v0.1 |

## Khung chung (mọi màn)

- **Sidebar 236 pt**: logo; ô tìm kiếm (Ctrl K); nhóm *Giao dịch* (Tổng quan, Lệnh, Lịch P/L, Phân tích,
  Nhật ký ngày, Tổng kết & xem lại, Lệnh bỏ lỡ), *Chiến lược* (Playbook), *Quản lý* (Tài khoản & kỷ luật, Nguồn dữ liệu, Đăng nhập & sync), *Hệ thống* (Cài đặt); mục đang chọn
  có nền `sidebar-accent` + thanh nhấn 3 pt bên trái; chân sidebar là thẻ trạng thái EA và sync.
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
| `gallery` (nhiều ảnh, badge EA vào/ra, kéo đổi thứ tự, ô "Thêm ảnh" nhận chọn nhiều / kéo thả / Ctrl+V) | chi tiết lệnh, playbook, lệnh bỏ lỡ | `view` grid + `pick_paths(multiple)` + clipboard | có (`gallery.rs`) |
| `plan_vs_actual` (bảng kế hoạch / thực tế / chênh) | chi tiết lệnh | `view` grid | v0.2 |
| `queue_row`, `mistake_cost_bar` | Tổng kết, Phân tích | `view` + `progress` | `mistake_cost_bar` có (`charts.rs`); `queue_row` v0.2 |
| `share_card` (render thành PNG theo `report::Privacy`) | Chia sẻ | `canvas` → ảnh | v0.3 |
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
qua `registry.assets(..)` vì desktop không tải Google Fonts.

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
| Chi tiết lệnh | Không có biểu đồ giá của lệnh và nút Chia sẻ / Mở trên MT5; lưu bằng nút Lưu (tự lưu khi chuyển sang lệnh khác hoặc mở Tuỳ biến form) thay cho "tự động lưu"; ảnh chart thì lưu ngay khi thêm, xoá, đổi thứ tự | Biểu đồ giá cần nguồn nến (chưa có); chia sẻ là v0.3 |
| Chi tiết lệnh | "Kế hoạch lúc vào vs thực tế" có thêm hàng Risk và nút "Sửa kế hoạch" mở lại popup ghi kế hoạch; "Lý do thoát lệnh" lấy gợi ý từ lý do của deal đóng và chỉ ghi cùng lần lưu kế tiếp (mở xem lệnh không tạo nhật ký); "Đúng kế hoạch" là ô chọn Chưa đánh giá / Có / Không; thuộc tính "Ảnh chart" trong danh sách chỉ đếm ảnh, thư viện ảnh vẫn là khối bên trái; không có nhãn "Thuộc tính kiểu Notion" | Danh sách thuộc tính vẽ từ `FormLayout` của người dùng nên mọi thuộc tính có sẵn là một hàng; nút "Tuỳ biến form" thay "Đã lưu tự động · Tuỳ biến form" vì lưu bằng nút |
| Tuỳ biến form | Không có "Xem trước form" và nút Huỷ / Lưu của panel sửa (mọi thay đổi áp dụng và lưu ngay như danh sách thuộc tính ở chi tiết lệnh); không có tab Playbook (Playbook không phải `FieldScope`); lựa chọn không có màu, không đếm số lệnh dùng và chưa kéo đổi thứ tự; "Khôi phục thuộc tính đã xoá" mở danh sách ngay dưới bảng; thêm nút Xong (Primary) và Đặt lại mặc định | Áp dụng ngay để khớp popover của `property_list`; màu và đếm lệnh cần thêm truy vấn đếm theo lựa chọn, làm khi có màn Danh mục |
| Popup ghi kế hoạch | Là `dialog()` native ở giữa cửa sổ, nền tối bằng token `background` mờ 72%; không có cảnh báo vượt quy tắc rủi ro, giờ mở lệnh, nút "+ Thêm" lựa chọn; Hạng setup và Cảm xúc là ô chọn thay chip; ✕ và Escape đều là "Ghi sau" | Cảnh báo quy tắc cần đánh giá `Rules` theo lệnh đang mở (chưa có); `dialog()` không có kích thước riêng; sự kiện của EA không mang giờ mở |
| Nhập lệnh tay | Chỉ có tab "Lệnh mới + nhật ký" (nhật ký cho lệnh có sẵn làm ở chi tiết lệnh, ghi chú ngày ở Nhật ký ngày); giờ vào / ra nhập bằng ô chữ `2026-11-20 16:05`; Lãi/lỗ bắt buộc nhập vì app chưa biết contract size để tự tính; không có khung "Nhập nhanh" và dán dòng MT5; ảnh nằm ở thẻ "Ảnh & đính kèm" bằng `gallery` (lưu vào lệnh khi bấm Lưu); "Lưu lệnh" mở trang chi tiết lệnh vừa lưu; Ctrl+Enter lưu | Date picker và bộ phân tích dòng MT5 chưa có; Ctrl+Shift+Enter của thiết kế thay bằng nút "Lưu & nhập tiếp" |
| Chi tiết lệnh | Mở bằng trạng thái `nav::open_trade` đè lên route Lệnh, không dùng `nav_stack` native | Chỉ cần một tầng; nút "‹ Lệnh" quay lại danh sách và giữ nguyên bộ lọc |
| Tổng quan | Biểu đồ chỉ có hai tab Balance và R (không có Equity); nút "Xem phân tích" thay "Phân tích" ở thẻ Theo symbol; thẻ lịch tên "Tháng này" kèm nút "Mở lịch", lưới chỉ T2–T6 và ô không có nút bấm; Kỷ luật hôm nay lấy quy tắc của tài khoản đầu tiên (trong phạm vi đang chọn) có đặt giới hạn, "vi phạm" đếm theo `rules::evaluate` | Equity cần lãi/lỗ đang mở mà nguồn dữ liệu chưa gửi theo thời gian; nút "Phân tích" trùng nhãn với mục sidebar nên test và trình đọc màn hình không phân biệt được; quy tắc thuộc từng tài khoản còn thiết kế chỉ vẽ một bộ |
| Lịch P/L | Chưa có nút "Xuất ảnh tháng"; "Mở nhật ký ngày" chỉ chuyển sang màn Nhật ký (chưa chọn đúng ngày); ô lịch là `view` lưới (không dùng `plot` Cell); chọn ngày của tháng khác chuyển luôn sang tháng đó | Xuất ảnh là v0.3 (chia sẻ); màn Nhật ký ngày do mục khác dựng; mỗi ô mang chữ để đọc và để test tìm |
| Phân tích | Chưa có "So sánh với 90N trước"; khoảng thời gian dùng chung bộ lọc 7N/30N/90N/YTD/Tất cả với Tổng quan; heatmap giờ × thứ và "Theo symbol" dựng bằng `view` (không dùng `plot` Cell/Bar); bảng "Theo giờ vào lệnh" thay biểu đồ giờ; giờ tính theo giờ của máy (không cố định giờ Việt Nam) | So sánh kỳ trước cần đọc hai khoảng cùng lúc, làm sau; mỗi ô và hàng mang chữ để đọc và để test tìm |
| Tổng quan | Tiền hiện kèm ký hiệu đô la khi tiền tệ là USD (hoặc tài khoản chưa báo), còn lại dạng `123.45 EUR` | Chưa có cài đặt tiền tệ; thiết kế chỉ vẽ USD |
| Nguồn dữ liệu | Mới có thẻ Bridge (trạng thái, cổng, token + sao chép, nhật ký); hướng dẫn cài EA và import vẫn là khối "Sẽ có ở bản tới" | Phần còn lại của v0.1 làm ở các mục 2 và 4 của lộ trình |
| Thuộc tính | Kiểu Ngày nhập bằng ô chữ `YYYY-MM-DD` (có báo sai định dạng), chưa có lịch chọn; popover sửa thuộc tính áp dụng ngay (không có Huỷ / Lưu), tên và lựa chọn lưu khi Enter hoặc rời ô | `date_picker` của NSH cần dựng riêng; mỗi lần gõ không được ghi DB |
| Thư viện ảnh | Mỗi ảnh có nút ✕ và tay nắm ⋮⋮ (kéo, hoặc phím mũi tên khi tay nắm đang focus); Ctrl+V nhận file ảnh đã copy hoặc ảnh trong clipboard | Thiết kế chỉ vẽ ⋮⋮; phím tắt và nút xoá cần cho thao tác bằng bàn phím |
| Mọi màn | Bộ icon có thêm `ChevronRight` (đối xứng `ChevronLeft`) | Thiết kế dùng một chiều cho cả hai nút trước / sau |
| Tài khoản & kỷ luật | Không có nút "Thêm tài khoản" và ô "Loại tài khoản" (Thật / Prop firm / Demo) / "Ghi chú"; thẻ tài khoản không có thanh mục tiêu lợi nhuận; checklist chỉ sửa chữ, xoá, thêm (chưa kéo đổi thứ tự); khối "Hôm nay" nằm cột phải | Tài khoản do EA hoặc import tạo ra; model `Account` chưa có loại và ghi chú; thứ tự checklist làm khi có `reorder` cho danh sách văn bản |
| Tài khoản & kỷ luật | Mỗi quy tắc là một dòng có công tắc: bật mới hiện ô số, tắt thì ô số ẩn (giá trị cũ được giữ cho lần bật sau); "Lưu thông tin" là nút riêng | `Rules` dùng `None` cho quy tắc tắt; ô số của NSH báo theo từng phím nên lưu bằng nút, không ghi mỗi lần gõ |
| Nhật ký ngày | Kế hoạch, tổng kết, tâm trạng nằm chung một thẻ "Kế hoạch và tổng kết" dựng bằng `property_list` (đúng thứ tự người dùng sắp trong form); chưa có thanh trượt giấc ngủ / tập trung / sẵn sàng, tin trong ngày, "Điểm kỷ luật ngày", thẻ "Thói quen ghi nhật ký" và nút "Dùng mẫu kế hoạch" | Form ngày do người dùng tuỳ biến (thuộc tính riêng thay cho các thanh trượt); mẫu kế hoạch thuộc màn Tổng kết, thói quen cần thống kê nhiều ngày |
| Nhật ký ngày | Lưu tự động mỗi 2 giây khi có thay đổi, khi đổi ngày hoặc tài khoản; nút "Lưu nhật ký" chỉ để lưu ngay | Ô nhập bám theo dữ liệu đọc nên cần lưu nháp riêng; tự lưu thay cho nút Lưu như thiết kế vì form ngày có thể rất dài |
| Cài đặt | Chỉ có Chế độ màu, Giờ hiển thị (giờ của bạn / giờ server), bridge (cổng, token), file dữ liệu, phím tắt, giới thiệu; chưa có màu nhấn, màu lãi/lỗ, mật độ, cỡ chữ, ngày bắt đầu tuần, định dạng số, ngôn ngữ, sao lưu / xuất / xoá dữ liệu, danh mục nhật ký, cập nhật tự động; "Tuỳ biến form" là nút tắt kèm chú thích | Ngoài phạm vi v0.2 của màn này; màn Tuỳ biến form do người khác làm. Giờ hiển thị mới áp dụng cho Tài khoản & kỷ luật và Nhật ký ngày, cho đến khi các màn khác đọc `time_display`; cổng và token mới có hiệu lực khi mở lại ứng dụng |
