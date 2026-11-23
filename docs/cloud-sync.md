# Đăng nhập & sync đám mây

TradeLens không có server riêng. Người dùng đăng nhập bằng **Google** hoặc **Microsoft**,
dữ liệu nằm trong thư mục ẩn dành riêng cho app trên Drive của chính họ.

| | Google | Microsoft |
|---|---|---|
| Đăng nhập | OAuth 2.0 + PKCE, redirect `http://127.0.0.1:<cổng ngẫu nhiên>` | như bên trái, tenant `common` |
| Nơi lưu | Google Drive `appDataFolder` | OneDrive `/me/drive/special/approot` |
| Scope | `openid email profile drive.appdata` | `openid email profile offline_access Files.ReadWrite.AppFolder` |
| Token | Windows Credential Manager / macOS Keychain / Secret Service (crate `keyring`) | như bên trái |

App chỉ đọc/ghi được file của chính nó, không thấy file khác của người dùng.


## Bố cục thư mục app

```text
changes/<device_id>/<seq_from>-<seq_to>.json   các thay đổi của một thiết bị (tối đa 500 dòng/file)
snapshots/<YYYY-MM-DD>.db                      bản chụp cả DB, mỗi tuần một bản (để khôi phục)
attachments/<sha256>.<ext>                     ảnh chart, đặt tên theo nội dung
devices.json                                   tên thiết bị, lần sync cuối
```

Mã: lõi gộp nằm trong `crates/core` (`sync`, `db/sync.rs`, `db/views.rs`; thuần Rust, không mạng),
động cơ đẩy/kéo trong `crates/cloud/src/sync.rs` (`SyncEngine`) trên trait `CloudDrive`;
`MemoryDrive` (`crates/cloud/src/memory.rs`) là ổ đĩa trong RAM cho test và chế độ demo.

## Danh tính liên máy

`id` trong DB (account_id, rowid, `TradeKey.account_id`) khác nhau giữa các máy, nên mọi thứ được
đồng bộ có một **khóa di động** (`sync::join_key`: các phần nối bằng `|`, `|` và `\` trong phần được escape).

| Thực thể (`entity`) | Khóa | Trường |
|---|---|---|
| `account` | `login\|server` | broker, name, currency, initial_balance, last_balance, last_equity, server_offset_min (không có `last_sync`) |
| `deal` | `login\|server\|ticket` | cả bản ghi (một trường `deal`, kèm `source`) |
| `journal` | `login\|server\|position\|segment` | mọi trường của `JournalEntry`, xem dưới |
| `rules` | `login\|server` | 5 giới hạn |
| `checklist` | **uid** | text, position, account |
| `day_note` | `<login\|server hoặc *>`, ngày | plan, review, mood, `custom.<key>` |
| `setting` | tên setting | value |
| `catalog` | `kind\|key` | label, color, position, archived |
| `field_def` | `scope\|key` | label, kind, options, max, required, analysed, position, archived |
| `form_layout` | scope | `items#<trường>`, `items.order` |
| `playbook` | **uid** | mọi trường của `Playbook` + position |
| `missed` | **uid** | mọi trường của `MissedTrade`, account (khóa tài khoản), playbook (uid) |
| `review` | `<login\|server hoặc *>`, period, start | answers, rating, `custom.<key>` |
| `attachment` | đường dẫn ảnh trong thư mục dữ liệu | sha256 |

- **uid** (migration `0010_sync_uids.sql`): playbook, lệnh bỏ lỡ và mục checklist không có khóa tự nhiên nên
  có cột `uid` (128 bit ngẫu nhiên, điền sẵn cho dữ liệu cũ, trigger điền cho dòng mới). Review đã có khóa
  tự nhiên `(tài khoản, kỳ, ngày bắt đầu)` (UNIQUE) nên **không thêm uid**: hai máy cùng viết tổng kết một
  tuần là *cùng một* review chứ không phải hai.
- **Liên kết giữa thực thể** đi bằng khóa di động, không bằng id: `journal.playbook_id` ↔ `playbook` (uid),
  `journal.checklist` (id mục đã tick) ↔ `checklist#<uid>`, `missed.account_id` ↔ `account` (khóa tài khoản),
  `missed.playbook_id`. Áp thay đổi theo thứ tự thực thể (tài khoản, setting, catalog, field, form, checklist,
  playbook, deal, rules, journal, lệnh bỏ lỡ, nhật ký ngày, review, ảnh) nên thứ được tham chiếu luôn có trước.
  Liên kết tới thứ đã bị xoá (hoặc chưa có trên máy này) bị bỏ khi đồng bộ, đúng như thống kê vốn đọc nó là "setup đã xoá".
- **Setting không đồng bộ** (`sync::is_device_local`): mọi khóa bắt đầu bằng `sync.` (id thiết bị và sổ sách
  sync), `device.`, `window.` (trạng thái cửa sổ), `bridge.`, `cloud.`, và `appearance` (sáng/tối theo từng máy),
  `data_dir`, `data_folder`. `form.layout.*` đồng bộ qua thực thể `form_layout`, không qua `setting`.
  `time_display`, mẫu tổng kết `review.template.*` đồng bộ như setting thường.

## Ghi nhật ký thay đổi

Mọi ghi cục bộ (qua `Db`: `upsert_batch`, `add_manual_trade`, import Notion, `save_journal`, `save_rules`,
`save_checklist`, `save_day_note`/`save_day_values`, `set_setting`, danh mục, field, form, playbook, lệnh bỏ lỡ,
tổng kết, mẫu, xoá, và việc đổi kiểu field kéo theo các entry) đi qua `db::sync::logged`: đọc thực thể
**trước và sau** dưới dạng trường di động rồi ghi *những trường khác nhau* thành một dòng `changes`
`(device_id, seq, at, entity, key, value)`; `value` là mảng `{"f": tên trường, "v": giá trị}` (thiếu `v` = bỏ
trường / xoá phần tử), `NULL` = xoá thực thể. Ghi cùng giá trị thì không ghi gì.

Mỗi trường có **tem** `(at, device_id)` trong `sync_clock`. Một ghi cục bộ lấy `at = max(đồng hồ máy, tem mới nhất
của thực thể + 1)` để không bao giờ thua một giá trị máy này đã thấy dù đồng hồ máy chạy chậm.

Áp thay đổi của máy khác (`Db::apply_changes`) dùng các thao tác thô (`db/views.rs`), **không bao giờ ghi lại vào
`changes`** (không vọng). Dữ liệu có trước khi có sync được ghi một lần bởi `Db::log_existing` (trường nào chưa
có tem thì ghi, tem = `updated_at` của journal, các thứ khác tem 0 để mọi sửa đổi thật ở máy khác đều thắng).

## Gộp: quy tắc

1. **Mỗi trường là một thanh ghi LWW:** ghi có tem lớn hơn thắng; bằng `at` thì **device id lớn hơn thắng**
   (so sánh tem `(at, device)`; hai máy luôn ra cùng kết quả). Áp lại cùng một thay đổi không làm gì (tem bằng
   không thắng); thứ tự áp không ảnh hưởng kết quả (các lô được áp theo thứ tự thực thể rồi `(at, device, seq)`).
2. **Danh sách có thể cùng được thêm** (tag, lỗi, mục checklist đã tick) là **tập phần tử**: mỗi phần tử một
   trường `tags#london_open`, giá trị `true` hoặc vắng mặt. Mỗi phần tử có tem riêng nên thêm `A` ở máy 1 và
   bỏ `B` ở máy 2 đều giữ; cùng một phần tử bị thêm ở máy này, xoá ở máy kia thì ai ghi muộn hơn thắng
   (LWW-element-set). Danh sách luôn được dựng lại **theo thứ tự chữ cái** trên mọi máy.
3. **Danh sách có thứ tự** (ảnh chụp của lệnh/lệnh bỏ lỡ/setup, lựa chọn của field, mục của form, câu trả lời
   tổng kết): phần tử là các trường `shots#<id>` (id = đường dẫn ảnh, tên lựa chọn, field của mục form, câu
   hỏi) cộng một thanh ghi `<tên>.order` giữ thứ tự id. Thêm đồng thời đều giữ; thứ tự = theo `order` (thắng theo
   LWW), phần tử `order` chưa nhắc tới xếp sau, theo id.
4. **Giá trị tuỳ biến** (`custom.<key>`) mỗi key một thanh ghi; một field nhiều lựa chọn là *một* thanh ghi (hai
   máy cùng sửa lựa chọn của cùng field thì bản mới hơn thắng cả danh sách). Tương tự các danh sách còn lại của
   playbook (`entry_rules`, `exit_rules`, `symbols`...), kế hoạch lúc vào (`plan`) và văn bản dài (`notes`,
   `plan`/`review` của ngày): bản ghi muộn hơn thắng nguyên trường, **không gộp từng dòng**.
5. **Deal** gộp theo `(tài khoản, ticket)` như *một bản ghi*: bản có tem lớn hơn thắng; các số đo mà bản kia có
   (`risk`, `reason`, `sl`, `tp`) và bản thắng thiếu thì được điền vào (`merge_deal`, giống import báo cáo đè lên
   deal của EA). Hai máy import cùng báo cáo → cùng deal, không trùng.
6. **Xoá** (chỉ playbook, lệnh bỏ lỡ, mục checklist xoá được): xoá giữ lại nội dung ở `sync_tombstones`.
   *Xoá thắng mọi sửa đổi cũ hơn nó và thua mọi sửa đổi mới hơn nó.* Sửa mới hơn lệnh xoá (đến trước hay sau)
   thì thực thể **sống lại đúng như lúc bị xoá cộng với sửa đó**; sửa cũ hơn thì bị bỏ. Lệnh xoá thua một sửa
   mới hơn thì không xoá gì. Giới hạn đã biết: khi một lệnh xoá đến *sau* hai sửa đổi (một cũ hơn, một mới hơn
   nó) trên thiết bị đã áp sửa đổi cũ trước, sửa đổi cũ vẫn ở lại (không hoàn tác được); xảy ra ở ba máy cùng
   sửa/xoá một thực thể.
7. **Trùng tên playbook:** hai máy cùng tạo "Breakout" thì cả hai được giữ (bảng `playbooks` không còn UNIQUE
   trên `name`, giữ UNIQUE khi lưu bằng tay: lỗi "đã có setup tên …"); người dùng đổi tên một cái khi thấy.
8. Thay đổi hỏng hoặc thuộc loại thực thể chưa biết (bản app mới hơn) bị bỏ qua và ghi log, không làm hỏng sync.

## Đẩy và kéo

1. `Db::log_existing()` (một lần), rồi ảnh: ảnh nào journal/lệnh bỏ lỡ/setup nhắc tới và có file trong thư mục dữ
   liệu thì tính sha256, ghi thực thể `attachment` (đường dẫn → sha) và upload `attachments/<sha256>.<ext>` nếu
   ổ chưa có.
2. **Push:** các dòng `seq > pushed_seq` của máy này, tối đa 500 dòng mỗi file `changes/<device>/<từ>-<đến>.json`;
   upload xong mới ghi `pushed_seq` (bị ngắt giữa chừng thì lần sau gửi lại cùng tên file, ghi đè).
3. **Pull:** liệt kê `changes/`, bỏ file của chính máy này; mỗi thiết bị khác, tải các file có `đến` ≥ mốc+1
   theo thứ tự, lấy các dòng `seq` liên tiếp từ mốc+1. Gặp lỗ hổng (thiếu file) thì **dừng ở đó và ghi cảnh báo**
   chứ không bỏ qua; file hỏng thì báo `CloudError::Corrupt` và không áp gì. Áp tất cả trong một giao dịch cùng với
   việc nâng mốc `pulled` (bảng `cloud_state`, mỗi nhà cung cấp một dòng), nên sập giữa chừng không lặp, không sót.
4. Tải ảnh còn thiếu (tải xong đối chiếu sha256; đường dẫn ảnh từ xa không được thoát khỏi thư mục dữ liệu).
5. Nếu bản snapshot mới nhất đã ≥ 7 ngày (hoặc chưa có): upload snapshot. Ghi `devices.json` (id, tên, lần sync cuối;
   ghi đè theo kiểu đọc-sửa-ghi, mất một dòng khi hai máy ghi cùng lúc thì lần sync sau tự thêm lại).
6. Chạy khi bấm *Sync ngay*, mỗi 15 phút khi bật tự động, và khi đóng app.

## Snapshot và khôi phục sang máy mới

- `SyncEngine::snapshot` dùng `VACUUM INTO` (`Db::snapshot_to`): một bản sao nhất quán của cả DB (kể cả bảng
  `changes`, `sync_clock`, mốc kéo), tải lên `snapshots/<ngày>.db` (chưa nén).
- `SyncEngine::restore(db_path)`: tải snapshot **mới nhất**, ghi thành DB, rồi `Db::restored_as_new_device`:
  **device id mới**, xoá bảng `changes` (log đó là của máy cũ, máy cũ tự đẩy), mốc kéo của máy cũ = số thứ tự lớn
  nhất của nó trong snapshot, `pushed_seq` = 0. Sau đó sync như bình thường: kéo mọi thay đổi sau snapshot của các
  máy khác (và của máy đã chụp, nếu chúng đến sau) và tải ảnh.

## Kiểm thử

- `crates/core/tests/sync.rs`: quy tắc gộp qua `Db` (hai DB trao log), tem bằng nhau, lặp/đảo thứ tự, xoá và sửa,
  liên kết theo khóa, dữ liệu có từ trước, migration, snapshot.
- `crates/cloud/tests/sync.rs`: tiêu chí v0.3 "2 máy sửa cùng lúc, sync xong không mất dữ liệu" với 2–3 DB trên một
  `MemoryDrive`: sửa khác trường, cùng trường, tag thêm/bớt, deal import cả hai máy, playbook xoá/sửa, khôi phục máy mới,
  ảnh, sync bị ngắt, file hỏng, và 8 phiên sửa ngẫu nhiên trên 3 máy phải hội tụ. So sánh bằng `Db::portable_state()`.

- `app/tests/cloud_screens.rs`: màn Đăng nhập & sync không đầu, hai app (hai thư mục dữ liệu) trên một ổ chế độ thử:
  đăng nhập, Sync ngay, sửa nhật ký ở máy này hiện ở máy kia, khôi phục sang máy mới (có bản sao dữ liệu cũ), đăng xuất
  có hỏi, thông báo thiếu client id, công tắc tự động sync, sync khi bấm đóng cửa sổ.

## Để làm sau

- Mã hóa đầu cuối bằng passphrase (XChaCha20-Poly1305, khóa từ Argon2id).
- Dọn file `changes/` cũ (và `sync_tombstones`) sau mỗi snapshot; nén snapshot.
- Gộp văn bản dài theo dòng thay vì ghi đè theo trường; field nhiều lựa chọn như tập phần tử.
- Dropbox.

## Đăng nhập: cách hoạt động (crate `tradelens-cloud`)

1. `auth::sign_in` mở một cổng ngẫu nhiên trên `127.0.0.1`, tạo `state` và cặp PKCE (verifier 43 ký tự,
   challenge `S256`), rồi gọi hàm `open_browser(url)` do **UI truyền vào** (crate không tự mở cửa sổ).
2. Người dùng đồng ý trên trình duyệt; nhà cung cấp chuyển về `http://127.0.0.1:<cổng>/?code=…&state=…`,
   app trả một trang tiếng Việt nhỏ ("Đăng nhập thành công, bạn có thể đóng tab này").
   Yêu cầu có `state` sai (tab khác, favicon) bị bỏ qua; chờ tối đa `SignInOptions::timeout` (5 phút), hoặc
   dừng ngay khi `SignInOptions::cancel` được bật (nút Huỷ) → `CloudError::Cancelled`.
3. Đổi `code` lấy token, rồi đọc email và tên từ endpoint `userinfo`.
4. `TokenSource` giữ phiên: tự làm mới access token khi sắp hết hạn (đệm 60 giây) hoặc khi API trả 401
   (làm mới đúng một lần rồi gọi lại; 401 lần hai → `CloudError::SignedOut`). Refresh token bị thu hồi
   (`invalid_grant`) thì phiên bị xoá và app phải đăng nhập lại.
5. Kho khóa của hệ điều hành chỉ lưu **refresh token** và danh tính (email, tên); access token nằm trong bộ
   nhớ, vì token Microsoft có thể dài hơn giới hạn của Windows Credential Manager. Sau khi khởi động lại,
   lần gọi đầu tiên làm mới access token.
6. `sign_out` thu hồi token ở Google (Microsoft không có endpoint thu hồi) và luôn xoá token cục bộ, kể cả
   khi mất mạng.

Gọi từ UI (luồng nền, không gọi trên luồng giao diện):

```rust
let client = ClientConfig::resolve(Provider::Google, |key| settings.get(key))?;
let source = Arc::new(TokenSource::new(client, Arc::new(KeyringStore::new())));
let me = source.sign_in(&SignInOptions::default(), |url| open_in_browser(url))?;
let drive = GoogleDrive::new(Arc::clone(&source)); // hoặc OneDrive::new(source)
let engine = SyncEngine { drive: &drive, device_id };
```

## Trong app (`app/src/cloud.rs`, `app/src/pages/cloud.rs`)

Màn **Đăng nhập & sync** (route `Route::Cloud`) và dòng trạng thái ở chân sidebar ("Cloud: chưa đăng nhập",
"Google Drive · đã sync 14:02", "đang sync…", "sync lỗi") đọc cùng một trạng thái (`cloud::State`, một signal).

- **Đăng nhập:** nút "Đăng nhập với Google / Microsoft" chạy `TokenSource::sign_in` trên luồng nền, mở trình duyệt
  hệ thống (`rundll32 url.dll,FileProtocolHandler` / `open` / `xdg-open`), có nút Huỷ. Chưa cấu hình client id thì thẻ
  của nhà cung cấp hiện câu tiếng Việt nêu khoá cài đặt (`cloud.google.client_id`), biến môi trường
  (`TRADELENS_GOOGLE_CLIENT_ID`) và mục "Client id đọc ở đâu" của tài liệu này, nút đăng nhập tắt. Đăng xuất hỏi
  xác nhận; dữ liệu trên Drive còn nguyên.
- **Sync ngay**, **tự động mỗi 15 phút** (công tắc "Tự động sync", cài đặt `cloud.auto_sync`, chỉ của máy này) và
  **khi đóng app**: NSH chỉ chờ 200 ms cho việc làm lúc thoát (`on_app_quit`), quá ít cho mạng, nên cửa sổ chính đặt
  `close_on_press: false`; bấm đóng thì app sync xong rồi mới đóng (bấm đóng lần nữa trong lúc đang sync thì đóng
  ngay). Không bật tự động sync, hoặc chưa đăng nhập, thì đóng ngay như thường.
- **Luồng nền:** mỗi việc là một `spawn` chờ một `blocking` với kết nối `Db` riêng của nó (WAL, `busy_timeout`
  5 giây) rồi mới đổi signal trên luồng giao diện; sync xong gọi `data_changed()` nên mọi trang đọc lại trong một
  nhịp poll (400 ms). Luồng nền không bao giờ chạm signal. Việc chạy dưới owner của root (`Cloud::owner`), không
  phải của nút đã bấm: nút bị dựng lại khi trạng thái đổi (đang đăng nhập, đang sync) sẽ huỷ tác vụ của nó.
- **Khôi phục sang máy này:** hộp thoại xác nhận mạnh (thay toàn bộ dữ liệu, không hoàn tác). `Store::replace_database`
  chép DB đang mở sang `tradelens.before-restore.db` cạnh file dữ liệu, đóng nó (các đọc/ghi khác chờ khoá), để
  `SyncEngine::restore` ghi DB mới và sync, rồi mở lại; lỗi giữa chừng thì mở lại file đang có. Cài đặt của máy
  (`cloud.*`, `appearance`) được ghi lại sau khi khôi phục, không lấy từ snapshot của máy cũ.
- **Cài đặt của máy** (không đồng bộ, tiền tố `cloud.`): `cloud.provider` (nhà cung cấp đang dùng),
  `cloud.auto_sync`, `cloud.device_name` (tên máy trên danh sách thiết bị, mặc định tên máy tính),
  `cloud.<google|microsoft>.client_id` / `client_secret` / `redirect_host`.

### Chế độ thử (không cần tài khoản)

Đặt `TRADELENS_CLOUD=memory` (hoặc cài đặt `cloud.mode = memory`) để màn dùng `MemoryDrive` thay cho Google/Microsoft:
"Đăng nhập" xong ngay với tài khoản `demo@tradelens.local`, Sync ngay đẩy/nhận qua một ổ đĩa nằm trong bộ nhớ của
tiến trình, token Notion cũng giữ trong bộ nhớ (không chạm kho khoá của hệ điều hành). Hai máy (hai cửa sổ test) cùng
tên ổ (`cloud.demo_drive`, mặc định "mặc định") dùng chung ổ, nên thử được sync và khôi phục giữa hai thư mục dữ liệu.
Ổ đĩa mất khi đóng app. Cài đặt `cloud.notion.base_url` (hoặc `TRADELENS_NOTION_URL`) đổi địa chỉ Notion API, để test
bằng server giả.

### Client id đọc ở đâu

`ClientConfig::resolve(provider, setting)` tìm theo thứ tự, nguồn trước thắng:

| | Google | Microsoft |
|---|---|---|
| 1. Settings của app (người dùng nâng cao tự đổi) | `cloud.google.client_id`, `cloud.google.client_secret` | `cloud.microsoft.client_id` |
| 2. Biến môi trường lúc chạy (dev) | `TRADELENS_GOOGLE_CLIENT_ID`, `TRADELENS_GOOGLE_CLIENT_SECRET` | `TRADELENS_MICROSOFT_CLIENT_ID` |
| 3. Biến môi trường lúc **build** (`option_env!`, nhúng vào file chạy; dùng cho bản phát hành) | như dòng 2 | như dòng 2 |

Khoá tuỳ chọn `cloud.<google\|microsoft>.redirect_host` đổi host của redirect (mặc định `127.0.0.1`), xem mục Azure.
Không có nguồn nào → `CloudError::Config` với tên khoá cần đặt. **Không commit client id/secret vào repo**:
đặt chúng trong biến môi trường của máy build hoặc secret của CI. Client id và "secret" của app desktop không
phải là bí mật (Google ghi rõ như vậy), nhưng giữ ngoài repo để mỗi người dùng bản fork đăng ký app của mình.

```powershell
# thử khi phát triển
$env:TRADELENS_GOOGLE_CLIENT_ID = "1234-abc.apps.googleusercontent.com"
$env:TRADELENS_GOOGLE_CLIENT_SECRET = "GOCSPX-..."
$env:TRADELENS_MICROSOFT_CLIENT_ID = "00000000-0000-0000-0000-000000000000"
cargo run -p nsh-app-tradelens

# bản phát hành: đặt cùng các biến trước khi build để chúng được nhúng vào file chạy
cargo build --release -p nsh-app-tradelens
```

### Đăng ký app Google (Google Cloud)

1. Vào <https://console.cloud.google.com>, tạo project mới (ví dụ `TradeLens`).
2. **APIs & Services → Library** → tìm **Google Drive API** → *Enable*.
3. **APIs & Services → OAuth consent screen** (hoặc *Google Auth Platform*): chọn *External*, điền tên app
   `TradeLens`, email hỗ trợ, email liên hệ.
4. Mục **Scopes / Data access**: thêm `openid`, `.../auth/userinfo.email`, `.../auth/userinfo.profile` và
   `https://www.googleapis.com/auth/drive.appdata` (thư mục ẩn của app; Google xếp scope này vào loại không
   nhạy cảm nên không cần xét duyệt).
5. Trạng thái *Testing* chỉ cho các **Test users** đã thêm và làm refresh token hết hạn sau 7 ngày. Để dùng
   lâu dài, bấm **Publish app** (*In production*); với các scope trên không phải nộp xét duyệt.
6. **Credentials → Create credentials → OAuth client ID** → *Application type* = **Desktop app** → đặt tên.
7. Sao chép **Client ID** và **Client secret** vào các khoá/biến ở bảng trên. Với app desktop, secret vẫn
   cần gửi lên endpoint token nhưng không phải bí mật thật.
8. Redirect không phải đăng ký: client kiểu Desktop chấp nhận `http://127.0.0.1:<cổng bất kỳ>`.

### Đăng ký app Microsoft (Azure / Entra ID)

1. Vào <https://portal.azure.com> → **Microsoft Entra ID → App registrations → New registration**.
2. Tên `TradeLens`. *Supported account types*: **Accounts in any organizational directory and personal
   Microsoft accounts** (tương ứng tenant `common`).
3. *Redirect URI*: nền tảng **Public client/native (mobile & desktop)**, giá trị `http://127.0.0.1`
   (không ghi cổng: với địa chỉ loopback, Entra bỏ qua cổng nên cổng ngẫu nhiên vẫn khớp).
   Nếu cổng thông tin không nhận `127.0.0.1`, đăng ký `http://localhost` và đặt khoá
   `cloud.microsoft.redirect_host = localhost`.
4. Tab **Authentication → Advanced settings**: bật **Allow public client flows** = *Yes* (app không có secret).
5. Tab **API permissions → Add a permission → Microsoft Graph → Delegated permissions**: thêm
   `Files.ReadWrite.AppFolder`, `offline_access`, `openid`, `profile`, `email`. Không cần admin consent.
6. Tab **Overview**: sao chép **Application (client) ID** vào `cloud.microsoft.client_id` /
   `TRADELENS_MICROSOFT_CLIENT_ID`. Không tạo client secret.
7. Thư mục trên OneDrive của người dùng sẽ là `Apps/<tên app đã đăng ký>`.

Thử thật: chạy app với các biến trên, bấm *Đăng nhập*, đồng ý, rồi *Sync ngay*; file xuất hiện ở
`Apps/TradeLens` (OneDrive) hoặc trong dung lượng ẩn của app (Google Drive: *Settings → Manage apps →
TradeLens → Options → Delete hidden app data* cho thấy có dữ liệu).

### Tên file trên mỗi dịch vụ

- **Google Drive** `appDataFolder` phẳng: tên file là cả đường dẫn (`changes/<device>/1-5.json`, dấu `/`
  nằm trong tên). Drive không lọc theo đầu tên nên `list(prefix)` đọc cả thư mục (1000 file mỗi trang) rồi lọc.
  Ghi đè file cùng tên bằng cách tìm theo tên rồi `PATCH`.
- **OneDrive** `special/approot` có thư mục thật: `changes/<device>/…` là đường dẫn thật (tự tạo thư mục).
  `list(prefix)` mở thư mục sâu nhất có trong `prefix` và chỉ đi vào các thư mục con có thể khớp.
- File ≤ 5 MB (Google) / 4 MB (OneDrive) tải lên một yêu cầu; lớn hơn dùng phiên tải lên (resumable /
  upload session) theo từng phần 8 MB / 10 MB. Tải xuống đọc cả file vào bộ nhớ.
- Mọi yêu cầu gặp 429 hoặc 5xx được thử lại tối đa 5 lần với chờ 0,5 giây gấp đôi mỗi lần, tôn trọng
  `Retry-After` (tối đa 60 giây). Xoá file đã mất (404) coi như thành công.

## Notion API (import nhật ký không cần xuất file)

1. Vào <https://www.notion.so/my-integrations> → **New integration** (loại *Internal*), chỉ cần quyền
   *Read content*. Sao chép token (`ntn_…` hoặc `secret_…`).
2. Trong Notion, mở database nhật ký → `…` → **Connections** → thêm integration vừa tạo.
3. Trong app, dán token một lần; app lưu nó trong kho khóa của hệ điều hành dưới tên `notion-token`
   (`KeyringStore::save_secret`), không lưu trong DB hay settings.
4. `NotionClient::databases()` liệt kê database đã chia sẻ; `NotionClient::plan(id)` lấy mọi dòng (phân trang
   100 dòng, `Notion-Version: 2022-06-28`) và trả `NotionPlan` như khi đọc file CSV.

Mỗi property được làm phẳng thành chữ giống file CSV Notion xuất ra, rồi bảng `(headers, rows)` đi qua
`import::notion::infer_table` (cùng bộ nhận diện cột và kiểu với đường CSV; `infer_table_with` cho phép áp
cột tay). Quy ước: select/status theo tên; multi-select và people nối bằng `, `; checkbox `Yes`/`No`; số
viết thuần với `.` thập phân; ngày `2026-11-16 09:00` (giờ trên tường như Notion lưu, bỏ múi giờ) và khoảng
ngày `bắt đầu → kết thúc`; file `tên (url)`; formula/rollup theo kết quả tính; relation là id trang (API không
trả tiêu đề). Cột tiêu đề (`Name`) luôn đứng đầu; các cột còn lại theo thứ tự chữ cái vì API không cho biết
thứ tự hiển thị.
