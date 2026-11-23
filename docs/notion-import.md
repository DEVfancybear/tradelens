# Import nhật ký từ Notion

Nhiều trader đã ghi nhật ký trong một database Notion. TradeLens nhận dữ liệu đó theo hai cách.

| Cách | Người dùng làm gì | Phiên bản |
|---|---|---|
| **File CSV** | Database → `•••` → Export → *Markdown & CSV* → giải nén → kéo thả file `.csv` vào *Nguồn dữ liệu → Import* | v0.2 |
| **File ZIP** | Kéo thả thẳng file `.zip` Notion xuất ra vào cùng ô đó: app tự chọn `…_all.csv` (không có thì file `.csv` duy nhất), lấy luôn ảnh trong zip gắn vào lệnh | v0.3 |
| **Notion API** | Tạo *internal integration* tại notion.so/my-integrations, share database cho integration, *Nguồn dữ liệu → Kết nối Notion (API)*: dán token, chọn database, xem trước, nhập | v0.3 |

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

## Nhận diện cột: theo nội dung, không theo tên

Mọi người dùng đặt tên cột khác nhau, bằng thứ tiếng khác nhau, nên **vai trò của cột được suy ra từ nội dung và
từ quan hệ giữa các cột**; tên cột chỉ là tiêu chí phụ khi hai cột phù hợp ngang nhau (bảng tên đồng nghĩa Anh–Việt:
`Pair`, `Cặp` → Symbol; `P&L`, `Lãi/lỗ` → Lãi/lỗ…, so không phân biệt hoa thường, dấu và đơn vị trong ngoặc).
Một file mà mọi cột đều đổi tên thành `c1`…`c15`, hoặc xáo thứ tự cột, cho ra đúng cùng các lệnh (có test).

| Vai trò | Cột nào |
|---|---|
| Thời gian vào | ≥ 80% ô đọc được thành ngày (chỉ ngày cũng được). Có nhiều cột ngày thì lấy cột **sớm hơn các cột kia ở nhiều dòng nhất** (ngày giao dịch sớm hơn giờ ghi), rồi mới đến tên, rồi cột đầy đủ nhất |
| Chiều lệnh | ≥ 80% ô có từ chỉ chiều ở đầu: Buy/Sell/Long/Short/Mua/Bán, kể cả `Buy (Long)` |
| Symbol | ≥ 80% ô là mã công cụ (3–14 ký tự A–Z, số, `/ . - _ # +`), **lặp lại** (ít giá trị khác nhau). `BTC/USD` thành `BTCUSD`, `xau-usd` thành `XAUUSD` để khớp symbol MT5 |
| Ảnh | ≥ 80% ô chỉ gồm tên file hoặc link có đuôi ảnh (cắt theo `, `, bỏ `(link)`, giải mã `%20`) |
| Kết quả | cột số có **cả dương lẫn âm**. Nhỏ, tròn (bội của 0,1, trong ±20) và file không có giá/khối lượng → **kết quả R**; còn lại → lãi/lỗ tiền. Cột chỉ toàn số dương (tỷ lệ R:R kế hoạch) không phải kết quả |
| Thắng/thua | cột có 2–6 giá trị lặp lại mà **dấu của kết quả chia các dòng gọn theo giá trị** (≥ 85%): giá trị đi với số dương là thắng, âm là thua, 0 là hòa vốn. Học từ dữ liệu, không có danh sách từ |

Giữa các cột kết quả cùng phù hợp, cột nào được cột thắng/thua sắp xếp gọn nhất thì thắng. Cột thắng/thua vẫn là một
thuộc tính của lệnh. Bản xem trước ghi rõ app hiểu gì: "Cột «X» → Kết quả (R)", "Cột «Y» → thắng/thua (Thắng = thắng, …)".
Không có bước ghép tay.

- Thiếu cột bắt buộc (thời gian vào, symbol, chiều lệnh) thì báo lỗi nêu rõ thiếu gì và file có những cột nào.
  Thiếu khối lượng, giá vào/ra hoặc lãi/lỗ thì vẫn import, dùng 1 lot / 0 / 0 và ghi một cảnh báo.
- Ngày không có giờ: lệnh mở lúc 00:00, các lệnh cùng ngày giữ thứ tự dòng trong file (cộng thêm vài giây theo thứ tự);
  mã lệnh chỉ phụ thuộc ngày và tiêu đề nên import lại sau khi đổi thứ tự dòng không tạo trùng.

## Nhật ký ghi bằng R (không có tiền)

Nhiều trader chỉ ghi kết quả theo R (không có giá, không có lot, không có tiền). Khi cột kết quả là R:

- Mỗi dòng là một lệnh có hai deal như thường nhưng deal ra có `profit = 0` và `result_r = <R>` (cột `deals.result_r`,
  migration 0011; đồng bộ theo deal như mọi trường). `Trade::money_known()` là `false`, `Trade::r_multiple()` trả
  đúng R đó.
- Thống kê (`metrics`) tính thắng/thua/hòa vốn, tỷ lệ thắng, R trung bình, tổng R, phân bố R, nhóm theo setup/lỗi bằng R;
  số tiền (lãi gộp, profit factor, expectancy bằng tiền…) chỉ cộng các lệnh có tiền. Ở giao diện số tiền của lệnh
  kiểu này hiện «—», không bao giờ «$0.00»; danh sách lệnh toàn lệnh R thì ô tổng tiền cũng là «—».
- Kết quả thắng/thua của dòng lấy từ cột thắng/thua đã học nếu có, không thì từ dấu của R.

## Ảnh trong file zip

Cột ảnh được tách theo `, `, giải mã `%20`, rồi ghép với các file ảnh trong zip (theo đường dẫn đầy đủ, không có thì theo tên
file khi chỉ có một ảnh mang tên đó). Ảnh được chép vào `attachments/images/` theo nội dung (`attachments::import_bytes`),
cùng một ảnh nhiều dòng chỉ lưu một bản, rồi gắn làm ảnh chụp của nhật ký lệnh. Tên không tìm thấy trong zip ở lại cuối ghi
chú ("Ảnh: …") và được đếm: "Ảnh gắn vào lệnh: N · không có trong file zip: M". Tên file trong zip của Notion không đánh
dấu UTF-8 nên tên tiếng Việt đọc ra sai; vì vậy CSV chọn theo đuôi `.csv` và hậu tố `_all`, không theo tên.

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
| Notes | Ghi chú |
| Screenshot (cột ảnh) | Ảnh chụp của lệnh khi import từ zip; từ CSV hoặc API (không có file ảnh) thì link/tên được ghi vào cuối ghi chú (`Ảnh: …`) |
| Kết quả R | `result_r` của deal ra (xem "Nhật ký ghi bằng R") |
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
  (tên + kiểu) và đã dùng lại, số lựa chọn mới thêm vào danh sách, số ảnh đã gắn / thiếu, dòng bị bỏ qua, cảnh báo.

## Notion API trong app

*Nguồn dữ liệu → Import từ Notion → Kết nối Notion (API)*: dán token (lưu bằng `KeyringStore::save_secret(notion::TOKEN_SECRET, …)`,
không vào DB hay cài đặt), "Lưu token & tải danh sách" (`NotionClient::databases`), chọn database, "Xem trước database"
(`NotionClient::plan`) — từ đó là **cùng một** luồng xem trước và nút "Nhập N lệnh" (`Db::import_notion`) như file. "Xoá token"
quên token. Lỗi của Notion (token sai, database chưa chia sẻ) hiện nguyên văn bằng tiếng Việt ở dưới thẻ. Địa chỉ API đổi được
bằng cài đặt `cloud.notion.base_url` hoặc biến `TRADELENS_NOTION_URL` (dùng khi test bằng server giả).

## Kiểm thử

`crates/core/tests/notion.rs` (CSV), `crates/core/tests/notion_r.rs` (zip, nhật ký ghi bằng R, đổi tên và xáo cột),
`app/tests/notion_screens.rs` (thả zip, API với server giả). Fixture zip `tests/fixtures/notion_r_journal.zip` (năm lệnh
tự nghĩ ra, hai ảnh một điểm ảnh, tên file không đánh dấu UTF-8) sinh bằng `gen_fixtures` (`cargo test -p tradelens-core
--test gen_fixtures -- --ignored`). Không có dữ liệu thật của ai trong repo.
