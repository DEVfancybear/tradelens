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
| Phá huỷ | `destructive`, `destructive-foreground` | nút xoá / thay dữ liệu trong hộp xác nhận |
| Biểu đồ | `chart-1…5` | chuỗi dữ liệu không mang nghĩa lãi/lỗ |

Không viết mã màu trong code; thiếu vai trò thì thêm token trước.

## Chữ

Inter cho chữ, JetBrains Mono cho số trong bảng lệnh. Thang: 12 (`text_xs`, nhãn), 14 (`text_sm`,
nội dung), 16, 20 (`text_xl`, số KPI), 24 (`text_2xl`, tiêu đề trang). Đậm 500 cho nhãn nút, 600 cho tiêu đề.
Font nằm ở `app/assets/fonts/` (Inter 400–700, JetBrains Mono 400–600) và được đăng ký bằng
`registry.assets(assets!())` trong `lib.rs`, nhưng nền tảng chạy bằng `cargo run` không đọc nó, nên `root()` đăng ký thêm các file đó bằng `Root::font`; nhờ vậy `font-sans` / `font-mono` của `theme.json` luôn ra đúng font.

## Khoảng cách và bo góc

Lưới 4 pt; trang lề 32, thẻ đệm 20, khoảng giữa thẻ 24. Bo `radius` 8 cho thẻ và nút, 6 cho chip.

## Component (`src/ui/`)

| Component | File | Trạng thái |
|---|---|---|
| `page`, `page_with`, `page_fill`, `card`, `stat_tile`, `empty_state`, `warning_banner(message)` (một dòng cảnh báo quy tắc, `role` Alert) | `parts.rs` | có |
| `parts::property_row(label, control)` (hàng nhãn + điều khiển đơn giản, khác `properties`) | `parts.rs` | có |
| `action` / `action_when` (Primary / Secondary / Ghost / Destructive); nút tắt có một dáng chung (nền `muted`, chữ `muted-foreground`, mờ, không hover) | `controls.rs` | có |
| `link(label, on_press)` (liên kết chữ `primary`, gạch chân khi rê chuột: "Xem tất cả", "Mở lịch"…) | `controls.rs` | có |
| `confirm_then(owner, tiêu đề, mô tả, nhãn, then)` + `confirm::host()` (hộp xác nhận theme của app: Huỷ Secondary, nút Destructive, Escape huỷ, Enter xác nhận; gắn một lần trong `shell`) | `confirm.rs` | có |
| `action_icon` / `action_icon_when` (icon trước nhãn), `icon_action` (chỉ icon, bắt buộc `accessibility_label`) | `controls.rs` | có |
| `sidebar_row` (icon), `segmented`, `select_field`, `select_value`, `text_field`, `commit_field`, `textarea`, `tag_picker`, `rating` / `rating_of` | `controls.rs` | có |
| `icon(Icon, size)`, `Icon` (bộ icon nét 1.75 của thiết kế) | `icons.rs` (`pub`) | có |
| `number_field`, `switch_row`, `setting_row`, `check_box`, `checklist`, `progress_rule` | `forms.rs` (`pub`) | có |
| `dropzone`, `image_slot`, `Accept`, `pick_files`, `pasted_images`, `paths_of_uris` | `files.rs` (`pub`) | có |
| `gallery` (ảnh, chú thích, kéo/phím đổi thứ tự, Thêm ảnh) | `gallery.rs` (`pub`) | có |
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

- `Store::read(op, done)` đọc DB một lần trên luồng `blocking` (không theo revision) cho việc không phải dữ liệu của trang;
  `Store::replace_database(backup, replace, done)` chép DB đang mở sang `backup`, đóng nó, để `replace` ghi file mới (khôi phục
  sang máy mới) rồi mở lại; `Store::database_path()` và `Store::is_ready()` cho code ngoài `store.rs`.

### Đăng nhập & sync, Notion API (`src/cloud.rs`)

- `cloud::provide()` (gọi một lần trong `Workspace`) tạo `Cloud`, một handle `Copy`: `state: Signal<State>` (cài đặt `cloud.*`,
  ai đã đăng nhập, `Phase`, lần sync cuối, danh sách thiết bị), các hành động `sign_in`, `sign_out`, `sync(Why)`, `restore`,
  `set_auto_sync(AutoSync)`, và hẹn giờ 15 phút + chặn nút đóng cửa sổ để sync trước khi đóng. Trang lấy nó bằng `cloud::cloud()`
  lúc dựng, giống `store()`.
- **Mọi `spawn` / `Writer::run` của `Cloud` chạy dưới owner của root** (`Owner::run`), không phải của nút đã bấm: nút nằm trong
  `dynamic` bị dựng lại khi `state` đổi (đang đăng nhập, đang sync) và dựng lại thì huỷ tác vụ của nó, tức là một lần đăng nhập
  hay sync đang chạy bị bỏ dở mà không báo lỗi.
- Luồng nền (đăng nhập, sync, khôi phục, Notion) chỉ làm việc với dữ liệu `Send` (`Env` là ảnh chụp các cài đặt đã đọc sẵn) và
  không chạm signal; kết quả quay về luồng giao diện qua `spawn(async { blocking(..).await; .. })`, sync xong gọi `data_changed()`.
- `Env::load_secret / save_secret / clear_secret` giữ token Notion trong kho khoá của hệ điều hành, hoặc trong bộ nhớ ở chế độ
  thử (`cloud.mode = memory`); `Env::notion_client(token)` theo `cloud.notion.base_url` khi có.

### Danh sách thuộc tính

`property_list(PropertyList::new(layout, defs, values, PropertyEdits::new(on_layout, on_value, on_def)).builtin(|field| ..))`.
Cả ba hàm đọc (`layout`, `defs` gồm cả thuộc tính đã lưu trữ, `values`) và các callback sửa là của màn: callback lưu rồi
cập nhật cái mà hàm đọc trả về, hàng sẽ theo. Thuộc tính có sẵn do màn vẽ trong `.builtin(..)` (trả `None` thì hiện dấu
gạch). Đổi kiểu chuyển giá trị của **lệnh đang sửa** bằng `form::convert` (hỏi `alert_dialog` nếu mất dữ liệu); các lệnh
khác do `on_def` của màn xử lý (màn Tuỳ biến form).

### Chia sẻ kết quả và báo cáo PDF tháng

- **Mở hộp thoại chia sẻ:** `pages::share::open(ShareTarget::Trade(key))` hoặc `ShareTarget::Period(Period::Week | Month, start)`;
  `share::host()` (gắn một lần trong `shell`) hiện `dialog()` giống popup kế hoạch. Nút "Chia sẻ" có ở chi tiết lệnh (tắt khi lệnh còn mở)
  và ở header Tổng kết & xem lại.
- **Ảnh dựng trong `core`, không phải trong NSH:** `core::report::render_png(&ShareCard)` dựng SVG rồi rasterize bằng `resvg` với font đóng gói
  (Inter, JetBrains Mono từ `app/assets/fonts/`, `include_bytes!` ở `core/src/report/fonts.rs`); ảnh luôn theo giao diện tối của thiết kế.
  Hộp thoại dựng lại thẻ trên luồng `blocking` mỗi khi lựa chọn hoặc dữ liệu đổi, chỉ giữ bản mới nhất, ghi PNG ra file tạm để `image` hiện
  (file tự xoá khi ảnh bị thay) và giữ byte trong bộ nhớ cho "Lưu ảnh" (`pick_save_path`) và "Sao chép ảnh" (`set_clipboard_image`).
- **Quyền riêng tư:** `report::Privacy` (`ShowMoney` / `OnlyR` / `NoFigures`) quyết định số nào vào thẻ và PDF ngay trong `core`; UI chỉ chọn mức.
- **Tuần / tháng đọc từ snapshot:** `month_report::period_figures` (thẻ) và `month_report::build` (PDF) là hàm thuần trên `Snapshot` (có thêm
  `start_balance` và `cash`) cùng `month_report::Extras` đọc bằng `load_extras` (lệnh bỏ lỡ, tên lý do, tổng kết tháng).
- **Xuất PDF:** `share::export_pdf(report, privacy, done)` hỏi nơi lưu rồi ghi bằng `report::write_pdf` trên luồng `blocking`; dùng ở hộp thoại
  chia sẻ tháng và ở thẻ "Báo cáo tháng" của Tổng kết (tab Tháng).
- Icon `Icon::Copy` thêm cho "Sao chép ảnh".

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

### Test không đầu (`app/tests/`)

- Dùng chung `tests/support/mod.rs`: `Fixture` (thư mục dữ liệu tạm có DB, **tự dọn bằng `Drop` có thử lại** vì engine đóng DB sau
  khi test bỏ `app`, và Windows không xoá được file đang mở), một hằng `WAIT` (30 giây, chỉ tốn khi điều kiện chưa đúng) và
  `wait_until` để chờ thứ màn hình không cho thấy (DB, file). Không dùng `sleep` hay timeout 5 giây rải rác.
- Chờ **trạng thái đích**, không chờ thời gian: trang tải dữ liệu rồi mới dựng form (Nhật ký ngày chờ `label:Kế hoạch ngày`, vì gõ
  trước khi đọc xong thì chữ gõ bị ghi đè); bấm nút mà trang đang vẽ lại có thể mất lần bấm, nên bấm lại tới khi trạng thái đích
  hiện (`press_until`, `toggle_on`), không bao giờ bấm lại công tắc đã bật.
- Một ghi ở nền mà test đọc DB ngay phải xong **trong một transaction**: `save_playbook`, `save_missed_trade`, `save_review` ghi cả
  chỉ mục tìm kiếm cùng lúc, nên thấy dòng là tìm được dòng.
- Thứ tự drop trong test: DB mở trước thư mục chứa nó (`struct { db, dir }`, không phải `{ dir, db }`).

### Câu hỏi xác nhận, con trỏ, lịch chọn ngày, dữ liệu không đủ

- **Hỏi rồi mới làm: `controls::confirm_then(owner, tiêu đề, mô tả, nhãn, then)`** (định nghĩa ở `ui::confirm`). Hộp xác nhận là
  `dialog()` trong app, theme được (alert hệ thống thì không: nút Huỷ ra màu xanh của hệ điều hành); `confirm::host()` gắn một lần ở
  `shell`, test của một component có hỏi phải gắn nó trong harness. Trong test bấm nút của hộp bằng
  `label:<tiêu đề> >> role:button[<nhãn>]` khi trang phía sau có nút trùng tên (hộp che trang nên không bấm được trang). `then` chạy
  dưới `owner`, không phải owner của nút: thẻ chứa nút nằm trong `dynamic` bị dựng lại mỗi lần state đổi và huỷ việc của nó
  (đăng xuất, khôi phục, xoá thuộc tính, xoá playbook / lệnh bỏ lỡ đều từng bị). `owner` lấy lúc
  dựng bằng `controls::root_owner()` (Workspace gọi `provide_root_owner()`), hoặc `Cloud::confirm_sign_out` / `confirm_restore`.
  Việc trong `then` chạm signal của phần có thể đã dựng lại thì dùng `try_set`.
- **Mỗi trang là một phần tử có `key`** (`shell`, `pages::trades`): trang dựng thay cho trang khác bắt đầu ở đầu trang. Không có
  `key`, NSH giữ vị trí cuộn của nút cuộn dựng lại tại chỗ.
- **Font:** `root()` đăng ký bảy file font (`Root::font`) và token của `theme.json` (`Root::tokens`); `registry.assets(..)` một mình
  không đủ khi chạy bằng `cargo run`, vì nền tảng khởi động không kèm UI set của app.
- **Con trỏ:** mọi thứ bấm được có `.cursor_pointer()` trong component (`button_shell`, `sidebar_row`, `tab`, `toggle`, ô lịch, thẻ chọn,
  chip chọn, tên thuộc tính, dropzone, ảnh); nút tắt đặt `Cursor::Default` trong `disabled_style`; tay nắm kéo là `Cursor::Grab`.
  Component mới dùng `button()` / `toggle()` / `tab()` phải thêm `.cursor_pointer()`. Test không đầu đọc con trỏ qua `observe` các
  `MountItem::ViewStyle` (xem `tests/interaction_fixes.rs`).
- **Lịch chọn ngày: `ui::date_picker`** (`DatePick::day` / `DatePick::moment` + `date_pick(pick, trigger)` + `date_trigger`), bọc
  `DatePicker` của `nsh_base`: tuần bắt đầu thứ 2, tên thứ tiếng Việt, tháng / năm, nút "Hôm nay", chấm ở ngày `marking(..)` đánh dấu.
  Dùng ở Nhật ký ngày, Tổng kết, Lệnh bỏ lỡ, Nhập lệnh tay và thuộc tính kiểu Ngày. Nút ngày có nhãn truy cập là ngày ISO
  (`role:button[2026-11-15]`). Phím mũi tên / Enter / Escape là của `DatePicker` (NSH có test riêng); test không đầu của app chưa điều khiển được.
- **Dữ liệu không đủ:** deal mang `Gaps` (`TIME`, `VOLUME`, `PRICE`) do importer ghi; `Trade::money_known / volume_known / prices_known /
  time_known / hold_secs / origin` là quy tắc duy nhất, màn không tự suy từ giá trị (lot 1, giá 0, 00:00). Số liệu nhiều lệnh đi qua
  `metrics::Basis` (`unit()` là R khi không lệnh nào có tiền; `money_note()` khi trộn). Màn mới có số tiền phải chọn đơn vị bằng nó.
- **Gốc ứng dụng:** `nsh_app_tradelens::root()` (id và `window` đọc từ `nsh.json`) là thứ `main.rs` chạy; `App` trả thẳng `Workspace`,
  không bọc `window()` (sẽ mở cửa sổ thứ hai). Thư mục dữ liệu là `%APPDATA%\vn.dat.tradelens`; `store::adopt_earlier_data` chép bản cũ từ `default`.
