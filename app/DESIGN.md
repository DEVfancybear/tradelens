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

## Khoảng cách và bo góc

Lưới 4 pt; trang lề 32, thẻ đệm 20, khoảng giữa thẻ 24. Bo `radius` 8 cho thẻ và nút, 6 cho chip.

## Component (`src/ui/`)

| Component | File | Trạng thái |
|---|---|---|
| `page`, `card`, `stat_tile`, `planned` | `parts.rs` | có |
| `action` (Primary / Secondary / Ghost) | `controls.rs` | có |
| `sidebar_row` | `controls.rs` | có |
| `pnl_text`, `chip`, `segmented`, `field`, `select`, `table`, `empty_state`, `toast` | — | v0.1 |

## Motion (`src/ui/motion.rs`)

| Thay đổi | Công cụ | Token |
|---|---|---|
| Hover nút / hàng sidebar | trạng thái native `.hover` | `FAST` 120 ms |
| Nhấn | `.active` scale | `PRESS_SCALE` 0.97 |
| Thanh chọn sidebar đổi mục | opacity của thanh nhấn | `MOVE` 200 ms (v0.1: trượt bằng LayoutAnimation) |
| Trang mới hiện | fade + dịch 8 pt | `ENTER` 240 ms (v0.1) |
| Biểu đồ đổi khoảng thời gian | `appear_generation` của plot | `ENTER` |

Tôn trọng *reduce motion*: chỉ đổi opacity, không dịch chuyển.
