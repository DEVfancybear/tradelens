//! The shell's flows, headless on the real engine (the main window's content).

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn start() -> Headless {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<nsh_app_tradelens::Workspace>())).expect("the app starts");
    app.wait_for("Tổng quan").expect("the dashboard opens first");
    app
}

#[test]
fn opens_on_the_dashboard() {
    let mut app = start();
    assert!(app.screen().shows("Net P/L"));
    app.wait_for("Cloud: chưa đăng nhập").expect("the sidebar says how the data sources stand");
}

#[test]
fn the_sidebar_switches_pages() {
    let mut app = start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Gom từ các deal theo position; bấm một lệnh để mở nhật ký").expect("the trades page opens");
    app.press("role:button[Đăng nhập & sync]").expect("the cloud row is a button");
    app.wait_for("Dùng TradeLens trên nhiều máy và không lo mất nhật ký").expect("the cloud page opens");
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
