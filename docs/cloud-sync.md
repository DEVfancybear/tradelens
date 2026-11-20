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
