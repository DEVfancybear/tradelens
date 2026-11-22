# Import nhật ký từ Notion

Nhiều trader đã ghi nhật ký trong một database Notion. TradeLens nhận dữ liệu đó theo hai cách.

| Cách | Người dùng làm gì | Phiên bản |
|---|---|---|
| **File CSV** | Database → `•••` → Export → *Markdown & CSV* → giải nén → kéo thả file `.csv` vào *Nguồn dữ liệu → Import* | v0.2 |
| **Notion API** | Tạo *internal integration* tại notion.so/my-integrations, share database cho integration, dán token vào app | v0.3 |

Import CSV hoàn toàn tự động: người dùng chỉ chọn file, **không có bước ghép cột**. App đọc file,
tự nhận ra cột nào là gì, hiện bản tóm tắt (bao nhiêu lệnh, những thuộc tính mới nào, dòng nào bị
bỏ qua và vì sao) rồi lưu khi người dùng bấm *Import*.

## API (`tradelens_core::import::notion`)

```text
infer(&[u8]) -> Result<NotionPlan, ImportError>        // thuần, không ghi gì
Db::import_notion(&NotionPlan) -> Result<NotionImportReport, DbError>   // một transaction
```

`NotionPlan` chứa: tài khoản đích (`account`, mặc định tài khoản "Nhật ký Notion" — đổi bằng
`use_account`), cột nào thành trường nào (`mapping`), cột bị bỏ (`ignored_columns`), các thuộc tính
sẽ tạo (`properties`: tên, kiểu, lựa chọn, số lệnh có giá trị), từng lệnh đọc được (`rows`), dòng
không đọc được kèm lý do tiếng Việt (`issues`) và cảnh báo (`warnings`). `NotionMapping` chỉ là kết quả
nhận diện để hiển thị; truyền vào `infer_with` thì thành ghi đè cho từng trường, không bắt buộc.

## Nhận diện cột

1. **Theo tên cột**, không phân biệt hoa thường, dấu tiếng Việt, dấu câu và đơn vị trong ngoặc
   (`Lãi/lỗ ($)` = `lai lo`). Có bảng tên đồng nghĩa Anh–Việt cho từng trường: `Pair`, `Instrument`,
   `Cặp` → Symbol; `Result`, `PnL`, `P&L`, `Lãi/lỗ` → Lãi/lỗ; `Ngày vào`, `Date` → Thời gian vào;
   v.v. Tên khớp nhưng nội dung không hợp (cột `Result` toàn `Win/Loss` không phải số) thì không
   tính là trường đó.
2. **Theo nội dung**, cho ba trường bắt buộc mà tên không nói gì: thời gian vào (≥ 80% ô đọc được
   thành ngày), chiều lệnh (≥ 80% ô là Buy/Sell/Long/Short/Mua/Bán) và symbol (≥ 80% ô là mã viết hoa
   như `EURUSD`, `US30`, `BTC/USD`).
3. Thiếu cột bắt buộc thì báo lỗi nêu rõ thiếu gì và file có những cột nào. Thiếu khối lượng, giá
   vào/ra hoặc lãi/lỗ thì vẫn import, dùng 1 lot / 0 / 0 và ghi một cảnh báo.

Cột đầu tiên tên `Name` / `Title` / `Tên` là tiêu đề dòng (thành ghi chú của deal và là một phần
của khóa lệnh). Các cột `Created time`, `Last edited time`, `Created by`, `Last edited by` do Notion
tự thêm, ghi "khi nào / ai" chứ không nói chuyện gì xảy ra: **bị bỏ qua**, cùng với cột trống.

## Mọi cột còn lại thành thuộc tính

Cột không phải trường giao dịch trở thành một thuộc tính của form lệnh (`catalog::FieldDef`, phạm vi
`Trade`), kiểu đoán từ giá trị:

| Giá trị | Kiểu |
|---|---|
| ≥ 90% là `Yes/No`, `true/false`, `✓/✗`, `Có/Không` | Checkbox |
| ≥ 90% đọc được thành ngày (kể cả giờ) | Date (chỉ giữ ngày) |
| ≥ 90% là số (`$1,234.50`, `1.234,5`, `(45)`) | Number |
| ≥ 80% là link `http(s)://` | Url |
| Có ô dạng `a, b` và các mục lặp lại (≤ 30 mục khác nhau) | MultiSelect |
| ≤ 15 giá trị khác nhau và mỗi giá trị xuất hiện lặp lại | Select |
| Còn lại, có xuống dòng hoặc dài hơn 120 ký tự | LongText |
| Còn lại | Text |

Lựa chọn của Select/MultiSelect lấy từ dữ liệu, theo thứ tự xuất hiện. Nếu form đã có thuộc tính
trùng tên (không phân biệt hoa thường và dấu) thì dùng lại thuộc tính đó: giá trị được đổi sang
kiểu của nó, lựa chọn mới được thêm vào, cài đặt của người dùng giữ nguyên. Thuộc tính mới được nối
vào cuối `FormLayout` của form lệnh, theo thứ tự cột trong file.

## Trường giao dịch

| Cột Notion (ví dụ) | Trường TradeLens |
|---|---|
| Date, Ngày vào | Thời gian vào (khoảng `A → B` của Notion cho cả thời gian ra) |
| Pair, Cặp | Symbol |
| Direction (Long/Short, Mua/Bán) | Buy/Sell |
| Lot | Khối lượng |
| Entry / Exit, SL, TP | Giá vào / Giá ra, Stop loss, Take profit |
| P&L, Commission, Swap | Lãi/lỗ, Phí, Swap |
| Setup, Tags, Mistakes | Setup, Tag, Lỗi (multi-select → danh sách) |
| Emotion, Rating | Cảm xúc, Đánh giá (số 0–5 hoặc ⭐) |
| Notes, Screenshot | Ghi chú; link ảnh được ghi vào cuối ghi chú (`Ảnh: …`) |
| Setup grade, Exit reason, Market, Followed plan, Review later | Hạng setup, Lý do thoát, Thị trường, Đúng kế hoạch, Xem lại sau |

Tag, lỗi, cảm xúc, hạng setup, lý do thoát và trạng thái thị trường được khớp với danh sách lựa
chọn theo khóa hoặc nhãn (không phân biệt hoa thường, rồi không phân biệt dấu); chưa có thì thêm
mới vào danh sách. Setup trùng tên một setup trong Playbook thì gắn thêm `playbook_id`.

## Định dạng đọc được

- CSV theo RFC 4180: dấu nháy, nháy kép `""`, xuống dòng trong ô, BOM UTF-8, UTF-16. Dấu phân cách
  `,` `;` hoặc tab, đoán từ dòng tiêu đề. File `;` coi dấu phẩy đơn là dấu thập phân (`1,085`).
- Ngày: `November 20, 2026 9:30 AM`, `Nov 20, 2026`, `20 November 2026`, `2026-11-20`,
  `2026-11-20T09:30:00`, `2026/11/20 09:30`, `20/11/2026 09:30`, `20.11.2026`, `20 tháng 11, 2026`,
  ghi chú múi giờ `(GMT+7)` bị bỏ. `05/11/2026` mơ hồ: lấy theo cả file (có ngày > 12 ở phần đầu
  thì ngày-trước, ở phần giữa thì tháng-trước), mặc định ngày-trước. Giờ là giờ ghi trong file, như
  giờ server MT5.
- Số: ký hiệu tiền tệ, đơn vị tách bằng dấu cách (`125 USD`), phân cách nghìn, `(45.5)` và `−3` là số âm.
- Checkbox `Yes/No`, multi-select `a, b`, files `tên.png (https://…)`.

## Quy tắc lưu

- Mỗi dòng thành một lệnh nhập tay (2 deal, ticket âm, nguồn `notion`) trong tài khoản đích, cùng
  nhật ký của nó; tất cả trong một transaction.
- Import lại cùng file không tạo trùng. Ticket là hàm ổn định của cột tiêu đề + thời gian vào (+ số
  thứ tự nếu hai dòng giống hệt nhau), nằm dưới `-2^40` để không đụng ticket lệnh nhập tay. Dòng đã
  có thì cập nhật deal nếu số liệu trong file đổi.
- Nhật ký chỉ điền vào chỗ **còn trống**: ô người dùng đã viết sau khi import không bị ghi đè.
- Lệnh Notion trùng một lệnh MT5 (cùng symbol, chiều, giờ vào lệch dưới 2 phút): không tạo deal,
  chỉ gắn phần nhật ký vào lệnh MT5.
- Dòng không đọc được (ngày sai, chiều lạ, thiếu symbol, số sai…) bị bỏ qua và được liệt kê với số
  dòng trong file và lý do tiếng Việt; các dòng còn lại vẫn import. Không dòng nào đọc được thì báo
  lỗi.
- `NotionImportReport`: số lệnh mới / cập nhật / không đổi, số lệnh gắn vào MT5, thuộc tính đã tạo
  (tên + kiểu) và đã dùng lại, số lựa chọn mới thêm vào danh sách, dòng bị bỏ qua, cảnh báo.
