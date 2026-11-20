# EA TradeLensSync

1. Copy `TradeLensSync.mq5` vào `MQL5/Experts/` và thư mục `Include/TradeLens/` vào `MQL5/Include/`.
2. Mở MetaEditor, biên dịch `TradeLensSync.mq5`.
3. MT5 → Tools → Options → Expert Advisors → bật *Allow WebRequest for listed URL*, thêm `http://127.0.0.1:7878`.
4. Gắn EA vào một chart bất kỳ, dán token từ app (*Nguồn dữ liệu*), bật *Algo Trading*.

EA chỉ đọc lịch sử và gửi sang app; không mở hay sửa lệnh.
