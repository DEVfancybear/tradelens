---
title: Chính sách quyền riêng tư · Privacy policy — TradeLens
---

# Chính sách quyền riêng tư

*Cập nhật: 25/11/2026. Bản tiếng Anh ở [cuối trang](#privacy-policy).*

TradeLens là ứng dụng máy tính. **Không có server TradeLens**: app không gửi dữ liệu của bạn cho chúng tôi hay
cho bên thứ ba nào.

## Dữ liệu app giữ

- Nhật ký, lệnh, playbook, ảnh và cài đặt nằm trong thư mục dữ liệu của app trên máy bạn.
- Token đăng nhập (Google, Microsoft, Notion) nằm trong kho khoá của hệ điều hành (Windows Credential Manager,
  macOS Keychain), không nằm trong file dữ liệu.

## Dữ liệu người dùng Google

Khi bạn bấm "Đăng nhập với Google", app xin các quyền sau, và chỉ dùng chúng như dưới đây:

| Quyền | Dùng để |
|---|---|
| `openid`, `email`, `profile` | Hiện tài khoản nào đang đăng nhập (tên, email) trên màn "Đăng nhập & sync". |
| `drive.appdata` | Đọc và ghi bản đồng bộ nhật ký trong **thư mục ẩn riêng của TradeLens** trên Google Drive của bạn. App không thấy, không đọc, không sửa file nào khác trên Drive. |

- Dữ liệu đi thẳng từ máy bạn tới Google; không qua server nào khác.
- App không chia sẻ, bán hay chuyển dữ liệu người dùng Google cho bất kỳ ai, không dùng nó cho quảng cáo, và
  không dùng nó để huấn luyện mô hình AI.
- Việc TradeLens dùng và chuyển thông tin nhận từ Google API tuân theo
  [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
  kể cả các yêu cầu Limited Use.

OneDrive (tài khoản Microsoft) được dùng y như vậy: chỉ thư mục riêng của app (`approot`).

## Notion

Nếu bạn nhập dữ liệu từ Notion, app dùng token tích hợp do bạn tạo để đọc các database bạn đã chia sẻ cho nó,
rồi lưu kết quả vào máy bạn.

## Xoá dữ liệu và thu hồi quyền

- Bấm "Đăng xuất" trong app: token bị xoá khỏi máy (với Google, app còn thu hồi token).
- Thu hồi quyền bất cứ lúc nào ở [myaccount.google.com/permissions](https://myaccount.google.com/permissions).
- Xoá bản đồng bộ trên Drive: Google Drive → Cài đặt → Quản lý ứng dụng → TradeLens → "Xoá dữ liệu ứng dụng ẩn".
- Xoá dữ liệu trên máy: gỡ app và xoá thư mục dữ liệu của nó.

## Liên hệ

duonglt.work@gmail.com

---

# Privacy policy

*Last updated: November 25, 2026.*

TradeLens is a desktop application. **There is no TradeLens server**: the app does not send your data to us or to
any third party.

## Data the app keeps

- Your journal, trades, playbooks, images and settings are stored in the app's data folder on your computer.
- Sign-in tokens (Google, Microsoft, Notion) are kept in the operating system's credential store (Windows
  Credential Manager, macOS Keychain), never in the data files.

## Google user data

When you choose "Sign in with Google", the app requests the following scopes and uses them only as described:

| Scope | Used for |
|---|---|
| `openid`, `email`, `profile` | Showing which account is signed in (name, email) on the sign-in and sync screen. |
| `drive.appdata` | Reading and writing your journal's sync files in **TradeLens's own hidden app folder** on your Google Drive. The app cannot see, read or change any other file on your Drive. |

- Data travels directly between your computer and Google; it passes through no other server.
- The app does not share, sell or transfer Google user data to anyone, does not use it for advertising, and does
  not use it to train AI models.
- TradeLens's use and transfer of information received from Google APIs adheres to the
  [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
  including the Limited Use requirements.

OneDrive (Microsoft account) is used the same way: only the app's own folder (`approot`).

## Notion

If you import from Notion, the app uses the integration token you created to read the databases you shared with
it, and stores the result on your computer.

## Deleting data and revoking access

- Choose "Sign out" in the app: the tokens are removed from your computer (for Google, the token is also revoked).
- Revoke access at any time at [myaccount.google.com/permissions](https://myaccount.google.com/permissions).
- Delete the synced copy on Drive: Google Drive → Settings → Manage apps → TradeLens → "Delete hidden app data".
- Delete local data: uninstall the app and remove its data folder.

## Contact

duonglt.work@gmail.com
