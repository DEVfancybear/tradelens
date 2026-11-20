//! The shell's flows, headless on the real engine (the main window's content).

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn start() -> Headless {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<nsh_app_tradelens::Workspace>())).expect("the app starts");
    app.wait_for("Kết quả giao dịch và kỷ luật của tài khoản đang chọn.").expect("the dashboard opens first");
    app
}

#[test]
fn opens_on_the_dashboard() {
    let app = start();
    assert!(app.screen().shows("Net P/L"));
    assert!(app.screen().shows("Bridge MT5: chưa chạy"));
}

#[test]
fn the_sidebar_switches_pages() {
    let mut app = start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Danh sách lệnh đã gom từ deal MT5, lọc và mở nhật ký từng lệnh.").expect("the trades page opens");
    app.press("role:button[Đăng nhập & sync]").expect("the cloud row is a button");
    app.wait_for("Đăng nhập Google hoặc Microsoft để sync lên Google Drive / OneDrive.").expect("the cloud page opens");
}

#[test]
fn the_journal_extras_have_pages() {
    let mut app = start();
    app.press("role:button[Tổng kết & xem lại]").expect("the review row is a button");
    app.wait_for("Nhìn lại tuần/tháng, trả lời câu hỏi tổng kết và dọn hàng chờ xem lại.")
        .expect("the review page opens");
    app.press("role:button[Lệnh bỏ lỡ]").expect("the missed row is a button");
    app.wait_for("Setup đúng nhưng không vào — ghi lại để biết do dự tốn bao nhiêu.").expect("the missed page opens");
    app.press("role:button[Playbook]").expect("the playbook row is a button");
    app.wait_for("Mỗi setup một bộ quy tắc, checklist và ảnh mẫu, kèm số liệu thật.").expect("the playbook page opens");
}
