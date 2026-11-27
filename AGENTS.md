# TradeLens — hướng dẫn cho agent

Nhật ký giao dịch MT5, desktop, viết bằng Rust và NSH (Rust mode). Đọc `docs/architecture.md` trước
khi sửa code lần đầu; lộ trình ở `docs/roadmap.md`.

File này là nguồn duy nhất của hướng dẫn; `CLAUDE.md` chỉ nạp nó (`@AGENTS.md`). Sửa ở đây, đừng
chép nội dung sang `CLAUDE.md`.

## Bắt buộc: skill trong `.claude/skills/`

Repo đóng gói sẵn các skill trong `.claude/skills/` (danh sách và nguồn ở `.claude/skills/README.md`):

1. **Trước khi viết hoặc sửa bất kỳ file `.rs` nào**, nạp skill `rust-best-practices` và làm theo nó.
   Nếu skill và hướng dẫn ở đây mâu thuẫn, hướng dẫn ở đây thắng, nhưng phải ghi lý do trong commit.
2. **Trước khi dựng hoặc sửa màn, component, test trong `app/`, migration hay import trong
   `crates/core`, và trước khi kiểm trên app thật hay đẩy lên PR**, nạp skill `tradelens-lessons`:
   các lỗi đã gặp trong repo này và cách tránh. Sửa xong một lỗi có bài học mới thì thêm vào skill đó.
3. **Trước khi bàn giao** (kết thúc lượt, mở PR, commit), tự review phần Rust vừa đổi theo skill
   `rust-review`. Sửa các phát hiện nghiêm trọng; phần còn lại thì báo lại cho người dùng.
4. **Cổng chất lượng** phải sạch: `scripts/quality.sh` (fmt, `clippy --workspace --all-targets -D warnings`
   với lint của BeautifulRust trong `Cargo.toml`, `cargo test --workspace`). Hook `Stop` trong
   `.claude/settings.json` chạy script này và không cho kết thúc khi còn lỗi. Không tắt hook, không
   thêm `#[allow]`; chỉ dùng `#[expect(lint, reason = "...")]` trên item hẹp nhất, có lý do thật.

## Quy ước dự án

- Toolchain: Rust 1.100.0 (`rust-toolchain.toml`), edition 2024; dependency luôn ở bản mới nhất
  (`cargo update`, kiểm tra bản major mới bằng `cargo search`); NSH ghim chính xác `=0.7.x`.

- Nghiệp vụ nằm trong `crates/core` (thuần Rust, không UI, không mạng). UI chỉ gọi vào; luồng nền
  (kết nối MT5, cloud) không chạm signal, chúng tăng `Revision`.
- Hàm chưa làm trả lỗi `NotImplemented("…")`, không dùng `todo!()` / `unimplemented!()`.
- Không `unwrap()` ngoài test; `expect("lý do")` chỉ cho bất biến thật.
- **UI theo thiết kế:** trước khi dựng hoặc sửa một màn, đọc `docs/design/README.md` (màn → route →
  file trang, component cần có) và mở `docs/design/screens/<màn>.html` + `screenshots/<màn>.png`.
  Không tự chế bố cục khác; lệch thì ghi vào bảng cuối README thiết kế.
- UI NSH: theo skill `nsh` của repo NSH (https://github.com/longbridge/nsh, `skills/nsh`). Màu chỉ lấy từ
  token `theme()` / `ui::theme`, component chỉ lấy từ `app/src/ui/`, không style lại control ở trang.
- Song ngữ Việt/Anh, mặc định tiếng Việt; người dùng chọn ở Cài đặt (setting `language`). Mọi chữ hiển thị
  và thông điệp lỗi người dùng thấy là khoá trong bảng, không viết thẳng trong code: ở `app/` dùng `t!("khu.khoá")`
  của NSH (https://nsh.dev/docs/i18n) với `app/locales/<khu>.yml`; ở `crates/*` dùng `tr!` của crate với
  `crates/<crate>/locales/<khu>.yml`. Bảng theo định dạng rust-i18n `_version: 2`, `vi` và `en` cạnh nhau, đủ
  cả hai. Thêm khoá thì viết cả câu tiếng Việt lẫn tiếng Anh. Comment và doc trong code bằng tiếng Anh.
- Mỗi commit một chức năng, tiêu đề dạng `feat(core): …`, `fix(app): …`, `docs: …`, `ci: …`.
