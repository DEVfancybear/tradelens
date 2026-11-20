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
| `action` Primary / Secondary / Ghost, nút chỉ có icon (cần `accessibility_label`) | mọi màn | `button` | có, thiếu icon |
| `sidebar_row` | shell | `button` | có |
| `icon` (bộ icon nét 1.75) | mọi màn | `svg` | v0.1 |
| `pnl_text` (màu + dấu +/−, font mono, căn phải) | bảng, KPI, lịch | `text` | v0.1 |
| `chip` (neutral / profit / loss / primary / warn) | Buy/Sell, setup, trạng thái | `view` + `text` | v0.1 |
| `segmented` | khoảng thời gian, Buy/Sell, chế độ | `toggle_group` | v0.1 |
| `select_field`, `text_field`, `number_field`, `textarea` | form | `select`, `input`, `number_input` | v0.1 |
| `tag_picker` (chọn nhiều, + Thêm) | setup/tag/lỗi/cảm xúc | `toggle` | v0.1 |
| `rating` (5 sao) | chi tiết lệnh, nhật ký ngày | `radio_group` | v0.1 |
| `switch_row`, `setting_row` | quy tắc, cài đặt, sync | `switch` | v0.1 |
| `checklist` | chi tiết lệnh, nhập tay, tài khoản | `checkbox` | v0.1 |
| `trades_table` | Lệnh, Tổng quan | `data_table` (ảo hoá) | v0.1 |
| `equity_chart`, `bar_chart`, `diverging_bars`, `heatmap`, `histogram` | Tổng quan, Phân tích | `nsh_base::plot` (Line, Area, Bar, Cell) | v0.1–v0.2 |
| `month_grid` | Lịch P/L, Tổng quan | `view` grid | v0.2 |
| `dropzone`, `image_slot` | Nguồn dữ liệu, chi tiết lệnh | `view` + `pick_paths`, kéo thả file | v0.1 |
| `progress_rule` (thanh ngưỡng quy tắc) | Tổng quan, Tài khoản | `progress` | v0.1 |
| `property_list` + `property_row` (kéo ⋮⋮ đổi thứ tự, glyph kiểu, tên bấm để sửa, giá trị theo `FieldKind`, nhãn "riêng" cho thuộc tính tự thêm, dòng thuộc tính ẩn, "+ Thêm thuộc tính") | chi tiết lệnh, nhập tay, nhật ký ngày, lệnh bỏ lỡ | `view` + `drag` + `input` | v0.2 |
| `property_popover` (đổi tên, kiểu, lựa chọn; Nên điền / Đưa vào Phân tích / Nhân bản / Ẩn / Xoá) | chi tiết lệnh | `popover` | v0.2 |
| `form_editor` (tab theo `FieldScope`, bảng thuộc tính, panel sửa, chọn kiểu, khôi phục đã xoá) | Tuỳ biến form | `tabs`, `data_table`, `alert_dialog` khi đổi kiểu mất dữ liệu | v0.2 |
| `gallery` (nhiều ảnh, badge EA vào/ra, kéo đổi thứ tự, ô "Thêm ảnh" nhận chọn nhiều / kéo thả / Ctrl+V) | chi tiết lệnh, playbook, lệnh bỏ lỡ | `view` grid + `pick_paths(multiple)` + clipboard | v0.2 |
| `plan_vs_actual` (bảng kế hoạch / thực tế / chênh) | chi tiết lệnh | `view` grid | v0.2 |
| `queue_row`, `mistake_cost_bar` | Tổng kết, Phân tích | `view` + `progress` | v0.2 |
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
| — | — | — |
