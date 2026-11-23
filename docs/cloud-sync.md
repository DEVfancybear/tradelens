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
changes/<device_id>/<seq_from>-<seq_to>.json   các thay đổi của một thiết bị
snapshots/<YYYY-MM-DD>.db                      bản chụp DB hàng tuần (để khôi phục)
attachments/<sha256>.<ext>                     ảnh chart, đặt tên theo nội dung
devices.json                                   tên thiết bị, lần sync cuối
```

## Luồng sync

1. Mọi thay đổi cục bộ (deal, journal, rules, checklist, day note, setting) ghi thêm một dòng
   vào bảng `changes` với `(device_id, seq)` tăng dần.
2. **Push:** gom các dòng có `seq > pushed_seq` thành một file, upload, cập nhật `pushed_seq`.
3. **Pull:** liệt kê `changes/` của các thiết bị khác, tải file có `seq` lớn hơn mốc đã áp dụng,
   áp vào DB theo thứ tự thời gian.
4. **Gộp:** deal theo `(tài khoản, ticket)`, giống nhau thì không xung đột. Journal và setting
   lấy giá trị mới nhất theo từng trường (`at`); danh sách (tag, lỗi) gộp hợp.
5. Chạy khi bấm *Sync ngay*, mỗi 15 phút khi bật tự động, và khi đóng app.
6. **Khôi phục sang máy mới:** tải snapshot mới nhất rồi áp các thay đổi sau nó.

## Để làm sau v0.1

- Mã hóa đầu cuối bằng passphrase (XChaCha20-Poly1305, khóa từ Argon2id).
- Dọn file `changes/` cũ sau mỗi snapshot.
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
