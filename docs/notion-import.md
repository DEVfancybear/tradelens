# Import nhật ký từ Notion

Nhiều trader đã ghi nhật ký trong một database Notion. TradeLens nhận dữ liệu đó theo hai cách.

| Cách | Người dùng làm gì | Phiên bản |
|---|---|---|
| **File CSV** | Database → `•••` → Export → *Markdown & CSV* → giải nén → chọn file `.csv` trong *Nguồn dữ liệu → Import* | v0.2 |
| **Notion API** | Tạo *internal integration* tại notion.so/my-integrations, share database cho integration, dán token vào app | v0.3 |

## Ghép cột

Tên cột trong Notion do người dùng tự đặt, nên app đoán trước (`NotionMapping::guess`) rồi hiện
màn hình ghép cột để sửa. Ví dụ với template phổ biến:

| Cột Notion | Trường TradeLens |
|---|---|
| Date | Thời gian vào |
| Pair | Symbol |
| Direction (Long/Short) | Buy/Sell |
| Lot | Khối lượng |
| Entry / Exit | Giá vào / Giá ra |
| P&L | Lãi/lỗ |
| Setup, Tags, Mistakes | Setup, Tag, Lỗi (multi-select → danh sách) |
| Notes, Screenshot | Ghi chú, Ảnh |

## Quy tắc

- Mỗi dòng thành một lệnh nhập tay (2 deal, ticket âm) trong tài khoản người dùng chọn, cùng nhật ký của nó.
- Import lại cùng file không tạo trùng: khóa là `notion:<id dòng>` (CSV dùng hash của cột Name + Date).
- Lệnh Notion trùng một lệnh MT5 (cùng symbol, chiều, giờ vào lệch dưới 2 phút): chỉ lấy phần nhật ký, gắn vào lệnh MT5.
