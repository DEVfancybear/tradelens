# TradeLens — chuẩn thiết kế

Bản thiết kế đầy đủ (màn hình, trạng thái) nằm ở artifact *TradeLens — UI design*; file này là
chuẩn mà code trong `src/ui/` phải theo.

## Nền tảng

- **Đích:** desktop (Windows ưu tiên, macOS, Linux). Một bố cục: sidebar 236 pt + vùng nội dung,
  cửa sổ mặc định 1280 × 800, tối thiểu 1024 × 680. Điều khiển gọn cho chuột, cao 34 pt.
- **Giao diện tối là mặc định** (trader thường mở cạnh MT5 tối); sáng đầy đủ, theo hệ thống.
- **Ngôn ngữ:** tiếng Việt; số dùng dấu chấm thập phân như MT5, chữ số `tabular-nums`.

## Màu (token trong `theme.json`)

| Vai trò | Token | Dùng cho |
|---|---|---|
| Nền / thẻ / viền | `background`, `card`, `border` | khung trang, thẻ, đường kẻ |
| Chữ / chữ phụ | `foreground`, `muted-foreground` | nội dung, nhãn |
| Nhấn | `primary` (teal) | hành động chính, mục đang chọn, đường equity |
| Lãi / lỗ | `profit`, `loss` (+ `-soft` làm nền ô) | số P/L, ô lịch, thanh phân tích |
| Cảnh báo | `warning` | quy tắc sắp chạm ngưỡng |
| Biểu đồ | `chart-1…5` | chuỗi dữ liệu không mang nghĩa lãi/lỗ |

Không viết mã màu trong code; thiếu vai trò thì thêm token trước.

## Chữ

Inter cho chữ, JetBrains Mono cho số trong bảng lệnh. Thang: 12 (`text_xs`, nhãn), 14 (`text_sm`,
nội dung), 16, 20 (`text_xl`, số KPI), 24 (`text_2xl`, tiêu đề trang). Đậm 500 cho nhãn nút, 600 cho tiêu đề.
Font nằm ở `app/assets/fonts/` (Inter 400–700, JetBrains Mono 400–600) và được đăng ký bằng
`registry.assets(assets!())` trong `lib.rs`, nên `font-sans` / `font-mono` của `theme.json` luôn ra đúng font.

## Khoảng cách và bo góc

Lưới 4 pt; trang lề 32, thẻ đệm 20, khoảng giữa thẻ 24. Bo `radius` 8 cho thẻ và nút, 6 cho chip.

## Component (`src/ui/`)

| Component | File | Trạng thái |
|---|---|---|
| `page`, `page_with`, `page_fill`, `card`, `stat_tile`, `empty_state`, `planned`, `warning_banner(message)` (một dòng cảnh báo quy tắc, `role` Alert) | `parts.rs` | có |
| `parts::property_row(label, control)` (hàng nhãn + điều khiển đơn giản, khác `properties`) | `parts.rs` | có |
| `action` / `action_when` (Primary / Secondary / Ghost) | `controls.rs` | có |
| `action_icon` / `action_icon_when` (icon trước nhãn), `icon_action` (chỉ icon, bắt buộc `accessibility_label`) | `controls.rs` | có |
| `sidebar_row` (icon), `segmented`, `select_field`, `select_value`, `text_field`, `commit_field`, `textarea`, `tag_picker`, `rating` / `rating_of`, `confirm` | `controls.rs` | có |
| `icon(Icon, size)`, `Icon` (bộ icon nét 1.75 của thiết kế) | `icons.rs` (`pub`) | có |
| `number_field`, `switch_row`, `setting_row`, `check_box`, `checklist`, `progress_rule` | `forms.rs` (`pub`) | có |
| `dropzone`, `image_slot`, `Accept`, `pick_files`, `pasted_images`, `paths_of_uris` | `files.rs` (`pub`) | có |
| `gallery` (ảnh, badge EA, kéo/phím đổi thứ tự, Thêm ảnh) | `gallery.rs` (`pub`) | có |
| `drag_handle`, `grid_drop_index`, `list_drop_index` | `reorder.rs` (`pub`) | có |
| `property_list` + `PropertyList` + `PropertyEdits` (hàng thuộc tính, popover, thêm/ẩn/xoá, đổi kiểu) | `properties.rs` (`pub`) | có |
| `plan_vs_actual(plan, on_write)`, `plan_rows(&EntryPlan, &Trade, currency)` (kế hoạch lúc vào đặt cạnh thực tế, từ `journal::review`) | `plan_vs_actual.rs` | có |
| `tab_bar(group, options, label, value, on_change)` (hàng tab gạch chân, trên `tabs` / `tab`) | `tab_bar.rs` | có |
| `line_list(label, add_label, lines, on_change)` + `filled` (danh sách dòng chữ thêm/xoá/sửa: luật vào lệnh, checklist, câu hỏi mẫu) | `lines.rs` (`pub`) | có |
| `list_item`, `mini_table` + `Column` (thẻ chọn trong danh sách; bảng nhỏ vài dòng) | `list_item.rs` | có |
| `queue_row(title, detail, action_label, on_press)` (dòng hàng chờ xem lại) | `queue.rs` | có |
| `command_palette(rows, query, chips, hint, busy, on_select, on_cancel)` + `PaletteRow`, `palette_layer`, `search_trigger` (bảng tìm có nhóm kết quả, tô từ khoá) | `palette.rs` (`pub`) | có |
| `chip`, `pnl_text`, `figure_text` | `chips.rs` | có |
| `trades_table`, `recent_trades` | `table.rs` | có |
| `equity_chart` | `chart.rs` | có |
| `bar_chart(items, label, height)` + `BarItem`, `histogram(buckets, label, height)`, `heatmap(grid)` + `HeatGrid`, `diverging_bars(items)` + `DivergingItem`, `mistake_cost_bar(name, trades, net, r, share)` | `charts.rs` | có |
| `r_curve_chart(points, generation)` + `RPoint` (tổng R tích luỹ) | `r_chart.rs` | có |
| `month_grid(weeks, figure, selected, currency, today, on_pick)` + `Figure`, `mini_month(weeks, today)` (tuần lấy từ `metrics::month_weeks`) | `month_grid.rs` | có |
| `breakdown_table(name_title, rows, empty)` + `BreakdownRow` (bảng nhóm: lệnh, win rate, net, PF, R TB) | `breakdown.rs` | có |
| `columns(main, side)` (hai cột tự xuống dòng), `titled_card(title, trailing, body)` | `layout.rs` | có |
| `toast` | — | v0.1 |

Các module `icons`, `forms`, `files`, `gallery`, `reorder`, `properties` là `pub` (và `ui` là `pub mod`) để một
component chưa màn nào dùng không bị `dead_code`; module còn lại giữ `pub(crate)`. Component mới của màn khác
nên theo cách đó, không thêm `#[expect(dead_code)]`.

### Icon

`icon(Icon::Plus, 16.0)` vẽ bằng màu chữ đang kế thừa: đặt `.text_color(move || theme().muted_foreground)` lên
icon hoặc phần tử cha. Nút chỉ có icon dùng `icon_action(Icon::Trash, "Xoá dòng", Variant::Ghost, on_press)`
(nhãn truy cập là bắt buộc, test bấm bằng `label:Xoá dòng`); nút có icon và chữ dùng `action_icon`.

### Đọc và ghi dữ liệu từ một màn (`store::query`, `Writer`)

Màn không sửa `store.rs`. Trong thân component:

```rust
let trades = query(|db, account| db.journals(account));            // Reading<Vec<(TradeKey, JournalEntry)>>
let month = query_with(move || month.get(), |db, account, m| load_month(db, account, *m));
let write = writer();                                                // Copy, lấy lúc dựng, dùng trong listener
// ...
text(move || trades.value().map_or("Đang tải…".into(), |v| format!("{} lệnh", v.len())))
action("Lưu", Variant::Primary, move || {
    write.run(|db| db.save_day_note(1, day, &note), |result| { /* Err(String) tiếng Việt */ });
})
```

- `query` / `query_with` chạy `load` trên luồng `blocking`, đọc lại khi revision đổi (import, `Writer`), khi
  bộ lọc tài khoản đổi, và khi `input` của `query_with` đổi. Giá trị cũ giữ nguyên lúc đang đọc lại hay khi lỗi:
  `value()` (`Option<Arc<T>>`), `error()`, `loading()`, `refresh()`.
- `Writer::run(op, done)` chạy `op(&mut Db)` trên luồng `blocking`, thành công thì tăng revision (mọi `Reading` tự
  đọc lại) rồi gọi `done(Result<T, String>)` trên luồng UI. `writer()` và `query()` chỉ gọi lúc dựng component
  (dùng `use_context`), không gọi trong listener.
- Trong `dynamic(..)`, chỉ đọc cái quyết định hình dạng, còn lại bọc `untracked(|| ..)`; nếu không, đổi giao diện
  sáng/tối dựng lại vùng đó giữa lúc nút đang được tạo (panic "prop read after its owner was disposed").

### Danh sách thuộc tính

`property_list(PropertyList::new(layout, defs, values, PropertyEdits::new(on_layout, on_value, on_def)).builtin(|field| ..))`.
Cả ba hàm đọc (`layout`, `defs` gồm cả thuộc tính đã lưu trữ, `values`) và các callback sửa là của màn: callback lưu rồi
cập nhật cái mà hàm đọc trả về, hàng sẽ theo. Thuộc tính có sẵn do màn vẽ trong `.builtin(..)` (trả `None` thì hiện dấu
gạch). Đổi kiểu chuyển giá trị của **lệnh đang sửa** bằng `form::convert` (hỏi `alert_dialog` nếu mất dữ liệu); các lệnh
khác do `on_def` của màn xử lý (màn Tuỳ biến form).

### Tuỳ biến form, popup kế hoạch, nhập lệnh tay

- **Mở Tuỳ biến form từ màn khác** (Cài đặt, chi tiết lệnh): `nav::open_form_editor(FieldScope::Trade)` phủ trang
  hiện tại bằng `pages::form_editor`; `nav::close_form_editor()` (nút "Xong") hiện lại trang cũ, còn `nav::go(route)`
  đóng nó. Màn mở nó nên lưu phần đang sửa trước (chi tiết lệnh làm vậy qua `Editor::leave`).
- **Đổi kiểu thuộc tính** chuyển giá trị của *mọi* nhật ký đã lưu: `Db::apply_field_def(&FieldDef)` (đổi kiểu bằng
  `form::convert`, đổi tên một lựa chọn theo vào giá trị, thêm lựa chọn từ giá trị mới) và `Db::kind_change_impact`
  (đếm giá trị sẽ mất, để hỏi `alert_dialog`). Màn dùng `property_list` gọi `apply_field_def` trong `on_def`.
- **Popup ghi kế hoạch**: `pages::entry_plan::enqueue(PlanRequest)` xếp một yêu cầu vào hàng đợi (chi tiết lệnh dùng
  cho "Viết / Sửa kế hoạch"; kết nối MT5 của v0.4 sẽ đưa lệnh vừa mở vào); `entry_plan::host()`
  gắn một lần trong `shell`. Một việc chạy bằng `Writer::run` mà chủ của nó là hộp thoại phải xong *trước khi* đóng
  hộp thoại (đóng huỷ tác vụ), nên `finish` chỉ gọi trong `done`.
- **Phím tắt**: Ctrl+N (từ Lệnh) mở nhập lệnh tay, Ctrl+Enter lưu ở đó.
- Form nhật ký của lệnh dùng chung: `trade_detail::journal_form(fields, shots, suggestion, failure)` vẽ danh sách
  thuộc tính của `FieldScope::Trade` từ `FormLayout`; chi tiết lệnh và nhập lệnh tay đều dùng nó.

## Motion (`src/ui/motion.rs`)

| Thay đổi | Công cụ | Token |
|---|---|---|
| Hover nút / hàng sidebar | trạng thái native `.hover` | `FAST` 120 ms |
| Nhấn | `.active` scale | `PRESS_SCALE` 0.97 |
| Thanh chọn sidebar đổi mục | opacity của thanh nhấn | `MOVE` 200 ms (v0.1: trượt bằng LayoutAnimation) |
| Trang mới hiện | fade + dịch 8 pt | `ENTER` 240 ms (v0.1) |
| Biểu đồ đổi khoảng thời gian | `appear_generation` của plot | `ENTER` |

Tôn trọng *reduce motion*: chỉ đổi opacity, không dịch chuyển.

### Component của Tài khoản & kỷ luật, Nhật ký ngày, Cài đặt

| Component | File | Ghi chú |
|---|---|---|
| `rule_row(label, hint, unit, range, default, value, on_change)` | `rule_row.rs` (`pub`) | dòng quy tắc: ô số + công tắc; tắt thì `on_change(None)`, nhớ giá trị cũ |
| `checklist_editor(group, items, on_change)` + `renamed` / `without` / `appended` | `checklist_editor.rs` (`pub`) | sửa chữ, xoá, thêm mục; giữ id, mục mới có id 0 |
| `selectable_card(label, selected, on_press, body)` | `selectable_card.rs` (`pub`) | thẻ bấm để chọn (tài khoản, ngày trong tuần) |
| `forms::switch_control` | `forms.rs` | công tắc đứng riêng, không kèm nhãn |

Khi một ô nhập bám theo `Reading::value()`, bọc bằng `computed(move || data.value())`: một lần đọc lại đang chạy cũng
làm `value()` báo đổi (cờ `loading`), và nếu không có memo thì dữ liệu cũ ghi đè lên chỗ người dùng vừa sửa.

### Giờ hiển thị và "hôm nay"

Một nguồn duy nhất: `zone.rs`. Cài đặt `time_display` (`TimeDisplay::Local` / `Server`) cùng offset của server và của máy
làm nên `Zone`, nằm trong `Snapshot::zone` (đọc lại khi cài đặt đổi vì `Writer` tăng revision). Giờ lệnh (`TradeRow::opened`,
`closed`, `local`), ngày gom nhóm (`insight::summary_clock`) và "hôm nay" (`insight::today_now`, `Zone::today`) đều lấy từ đó;
đừng gọi `chrono::Local::now()` để suy ra ngày hay giờ lệnh. Phiên giao dịch luôn theo UTC (`TradeRow::utc`). Trang đọc thẳng
`Db` (`account_trades::load`) dùng `zone::time_display` và `AccountTrades::today(now_utc_ms)`.
Mở Nhật ký ngày ở một ngày khác (kết quả tìm, nút "Mở nhật ký ngày" ở Lịch): `nav::show_journal_day(day, account_id)`; màn
Nhật ký ngày đọc `nav::requested_journal_day()` rồi gọi `nav::take_journal_day()`.
