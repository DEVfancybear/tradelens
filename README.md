# TradeLens

**Nhật ký giao dịch cho trader MetaTrader 5.** TradeLens tự lấy lệnh từ tài khoản MT5, gom thành từng giao dịch,
tính thống kê và giúp bạn ghi lại vì sao vào lệnh, đã làm đúng kế hoạch chưa, và rút ra bài học gì. Mục tiêu là biến
lịch sử giao dịch thành thứ bạn có thể học từ đó, thay vì chỉ là một bảng số.

App chạy trên máy tính (Windows), dữ liệu nằm trên máy của bạn, có thể đồng bộ qua Google Drive hoặc OneDrive của
chính bạn. Giao diện tiếng Việt và tiếng Anh.

[Tải bản mới nhất cho Windows](https://github.com/DEVfancybear/tradelens/releases/latest/download/TradeLens-setup.exe) ·
[Tất cả phiên bản](https://github.com/DEVfancybear/tradelens/releases) ·
[Trang giới thiệu](https://devfancybear.github.io/tradelens/)

## TradeLens làm được gì

### Lệnh tự vào nhật ký

- **Kết nối MT5 ngay trong app**: nhập số tài khoản, mật khẩu và server; lệnh đã đóng hiện trong nhật ký sau vài giây,
  không cần mở MetaTrader hay cài EA. Theo dõi nhiều tài khoản cùng lúc.
- **Hoặc import báo cáo** MT5 (HTML, XLSX) bằng cách kéo thả, và chuyển nhật ký cũ từ **Notion** sang.
- **Nhập tay** lệnh và nhật ký khi cần.
- **Quản lý lệnh đang mở**: sửa SL/TP, đóng toàn bộ hoặc một phần ngay trong app.

### Hiểu mình giao dịch thế nào

- **Tổng quan**: lợi nhuận, tỉ lệ thắng, profit factor, kỳ vọng, đường equity và drawdown.
- **Lịch P/L**: kết quả từng ngày trong tháng, bấm vào một ngày để xem các lệnh của ngày đó.
- **Phân tích** theo symbol, setup, tag, phiên, giờ, chiều Buy/Sell, cảm xúc và lỗi hay mắc; mỗi dòng mở thẳng ra các
  lệnh tạo nên nó.
- **Tìm kiếm** toàn văn qua lệnh, ghi chú và nhật ký.

### Ghi nhật ký có chiều sâu

- **Kế hoạch lúc vào vs thực tế**: khi có lệnh mới, app hỏi kế hoạch (lý do vào, SL/TP dự kiến); sau đó so với cách
  lệnh thực sự diễn ra.
- **Nhật ký từng lệnh**: setup, hạng setup, điểm thực thi, cảm xúc, lý do thoát, lỗi mắc phải và cái giá của lỗi đó,
  kèm nhiều ảnh chart.
- **Form tuỳ biến** kiểu Notion: tự thêm trường bạn muốn theo dõi.
- **Nhật ký ngày**, **tổng kết tuần/tháng** với hàng chờ những lệnh chưa xem lại, và mẫu ghi chú dùng lại được.
- **Playbook**: mô tả từng setup của bạn và xem setup nào thực sự hiệu quả.
- **Lệnh bỏ lỡ**: ghi lại cơ hội đã thấy mà không vào, để biết mình bỏ qua bao nhiêu.

### Giữ kỷ luật

- **Quy tắc riêng cho từng tài khoản** (lỗ tối đa trong ngày, số lệnh tối đa trong ngày, rủi ro mỗi lệnh, chuỗi
  thua liên tiếp), app đánh dấu khi vi phạm.
- **Checklist trước khi vào lệnh** và thẻ "Kỷ luật hôm nay".
- **Tin quan trọng**: tin đỏ trong tuần và đếm ngược tới tin kế tiếp, để không vào lệnh ngay trước tin lớn.

### Đồng bộ và chia sẻ

- **Đồng bộ nhiều máy** qua Google Drive hoặc OneDrive của bạn: tự động định kỳ và khi đóng app, sửa trên hai máy cùng
  lúc không mất dữ liệu; khôi phục toàn bộ sang máy mới.
- **Thẻ chia sẻ kết quả** (có thể ẩn số tiền, chỉ hiện R) và **báo cáo PDF** theo tháng.

## Quyền riêng tư

- Dữ liệu giao dịch và nhật ký nằm trên máy của bạn; khi bật đồng bộ, chúng chỉ đi tới Google Drive hoặc OneDrive của
  chính bạn.
- Mật khẩu MT5 được giữ trong trình quản lý mật khẩu của Windows, không lưu dạng chữ thường.
- Xem thêm [Chính sách quyền riêng tư](docs/privacy.md) và [Điều khoản sử dụng](docs/terms.md).

## Cài đặt

- Windows 10/11, 64-bit. Tải bộ cài ở trên và chạy.
- Bộ cài chưa ký số: nếu Windows SmartScreen cảnh báo, chọn **Thông tin thêm** → **Vẫn chạy**.
- App tự kiểm tra và tải bản mới, cài khi khởi động lại (có thể tắt trong Cài đặt).
- Muốn kết nối MT5 trực tiếp thì máy cần MetaTrader 5; những thành phần còn thiếu app tự cài giúp bạn, không cần quyền
  quản trị.

## Lộ trình

Đã có: nhật ký và phân tích, kỷ luật, đồng bộ đám mây, kết nối MT5 trực tiếp và quản lý lệnh. Sắp tới: **copy lệnh giữa
nhiều tài khoản**, bản macOS và tài liệu hướng dẫn người dùng. Chi tiết từng bản ở [lộ trình](docs/roadmap.md).

## Giấy phép

MIT
