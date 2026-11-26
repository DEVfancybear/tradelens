//! Kết nối MT5, headless on the real engine, over the broker in memory (`mt5.mode = memory`): signing
//! in brings the account's deals into the journal, a position that opens puts the plan prompt on
//! screen, a position is closed from the app, a wrong password keeps nothing, and a saved account
//! connects by itself at the next start.

use nsh_core::platform::Os;
use nsh_testing::{DialogAnswer, Headless};
use tradelens_core::{AccountInfo, Db};
use tradelens_mt5::{Direction, Position, memory::Broker};

mod support;
use support::{Fixture, WAIT, wait_until};

const LOGIN: &str = "52012345";
const PASSWORD: &str = "mật-khẩu-thử";

/// A broker named `server` with the test's account on it, and a fixture set to the demo.
fn demo(server: &str) -> (Fixture, std::sync::Arc<Broker>) {
    let broker = Broker::named(server);
    broker.add_account(
        LOGIN,
        PASSWORD,
        AccountInfo {
            server: server.to_owned(),
            name: "Demo Thử".to_owned(),
            currency: "USD".to_owned(),
            balance: Some(10_000.0),
            equity: Some(10_050.0),
            ..AccountInfo::default()
        },
    );
    let fixture = Fixture::new();
    fixture.db().set_setting("mt5.mode", "memory").expect("the demo is chosen");
    (fixture, broker)
}

fn position(ticket: i64, symbol: &str) -> Position {
    Position {
        ticket,
        time_msc: chrono::Utc::now().timestamp_millis(),
        symbol: symbol.to_owned(),
        side: Direction::Buy,
        volume: 0.2,
        open_price: 2650.0,
        current_price: 2655.0,
        sl: 2640.0,
        tp: 2680.0,
        profit: 100.0,
        swap: 0.0,
        magic: 0,
        comment: String::new(),
    }
}

fn start(fixture: &Fixture) -> Headless {
    fixture.start(1400.0, 1300.0, |config| config.environment.os = Os::Linux)
}

fn open_data_sources(app: &mut Headless) {
    app.press("role:button[Nguồn dữ liệu]").expect("the data sources row is a button");
    app.wait_for_within("Kết nối MT5", WAIT).expect("the page opens");
    app.wait_for_within("text:/^Chế độ thử/", WAIT).expect("the demo says so");
}

fn sign_in(app: &mut Headless, server: &str, password: &str) {
    app.type_text("label:Số tài khoản", LOGIN).expect("types the login");
    app.type_text("label:Mật khẩu", password).expect("types the password");
    app.type_text("label:Server", server).expect("types the server");
    app.press("role:button[Đăng nhập]").expect("the sign-in button");
}

fn deals_of(db: &Db) -> usize {
    db.deals(None).map_or(0, |deals| deals.len())
}

#[test]
fn signing_in_brings_the_deals_prompts_a_plan_for_a_new_position_and_closes_one_from_the_app() {
    let server = "Thử-Đăng-Nhập";
    let (fixture, broker) = demo(server);
    broker.open(LOGIN, position(77, "XAUUSD"));
    let mut app = start(&fixture);
    open_data_sources(&mut app);
    sign_in(&mut app, server, PASSWORD);
    app.wait_for_within("Đã kết nối", WAIT).expect("the account connects");
    app.wait_for_within("Demo Thử (52012345)", WAIT).expect("the account is named as the broker names it");
    app.wait_for_within("MT5 · đã kết nối", WAIT).expect("the sidebar follows");
    app.wait_for_within("Lệnh đang mở (1)", WAIT).expect("the open position is listed");
    let db = fixture.db();
    wait_until("the entry deal being stored", || deals_of(&db) == 1);
    // The account is kept by a write that follows the login, on a thread of its own.
    let saved = || db.setting("mt5.accounts").ok().flatten().unwrap_or_default();
    wait_until("the account being kept once it logged in", || saved().contains(LOGIN));
    assert!(!saved().contains(PASSWORD), "the password is never in the settings");

    // A position that opens while connected puts the plan prompt on screen, seeded from its levels.
    broker.open(LOGIN, position(78, "EURUSD"));
    app.wait_for_within("Kế hoạch vào lệnh", WAIT).expect("the prompt opens");
    app.wait_for_within("text:/lệnh 78 vừa mở/", WAIT).expect("the prompt names the new position");
    wait_until("the new entry deal being stored", || deals_of(&db) == 2);
    app.press_key("Escape").expect("Escape closes the prompt");
    app.wait_until("the prompt closing", WAIT, |s| !s.shows("Kế hoạch vào lệnh")).expect("the prompt closes");

    // Close the first position from the card, after saying yes.
    app.wait_for_within("Lệnh đang mở (2)", WAIT).expect("both positions are listed");
    app.press("role:button[Quản lý] >> nth=0").expect("the first position's button");
    app.wait_for_within("Quản lý lệnh 77", WAIT).expect("the dialog opens");
    app.on_dialog(|dialog| {
        let title = dialog.title.clone().unwrap_or_default();
        assert!(title.contains("Đóng lệnh"), "{title}");
        DialogAnswer::Press("Đóng lệnh".to_owned())
    });
    app.press("role:button[Đóng lệnh]").expect("the close button");
    app.wait_for_within("Đã đóng lệnh.", WAIT).expect("the dialog says it closed");
    assert!(broker.positions(LOGIN).iter().all(|p| p.ticket != 77), "the broker closed the position");
    wait_until("the exit deal being stored", || deals_of(&db) == 3);
    app.wait_for_within("text:/không còn mở/", WAIT).expect("the dialog follows the closed position");
}

#[test]
fn a_wrong_password_is_said_and_nothing_is_kept() {
    let server = "Thử-Sai-Mật-Khẩu";
    let (fixture, _broker) = demo(server);
    let mut app = start(&fixture);
    open_data_sources(&mut app);
    sign_in(&mut app, server, "sai");
    app.wait_for_within("text:/^MT5 từ chối đăng nhập/", WAIT).expect("the form says why");
    app.wait_for_within("Chưa có tài khoản MT5 nào kết nối.", WAIT).expect("nothing is listed");
    let saved = fixture.db().setting("mt5.accounts").expect("the setting reads");
    assert!(saved.is_none_or(|json| !json.contains(LOGIN)), "a refused account is not kept");
}

#[test]
fn a_saved_account_connects_by_itself_at_the_next_start_and_can_be_forgotten() {
    let server = "Thử-Tự-Kết-Nối";
    let (fixture, _broker) = demo(server);
    {
        let mut app = start(&fixture);
        open_data_sources(&mut app);
        sign_in(&mut app, server, PASSWORD);
        app.wait_for_within("Đã kết nối", WAIT).expect("the account connects");
        let db = fixture.db();
        wait_until("the account being kept", || {
            db.setting("mt5.accounts").ok().flatten().is_some_and(|json| json.contains(LOGIN))
        });
    }
    let mut app = start(&fixture);
    app.wait_for_within("MT5 · đã kết nối", WAIT).expect("the saved account connects at the start");
    open_data_sources(&mut app);
    app.wait_for_within("Demo Thử (52012345)", WAIT).expect("the account is listed");
    app.on_dialog(|_| DialogAnswer::Press("Xoá".to_owned()));
    app.press("role:button[Xoá]").expect("the forget button");
    app.wait_for_within("Chưa có tài khoản MT5 nào kết nối.", WAIT).expect("the account is gone");
    app.wait_for_within("MT5: chưa kết nối", WAIT).expect("the sidebar follows");
    let db = fixture.db();
    wait_until("the account being forgotten", || {
        db.setting("mt5.accounts").ok().flatten().is_some_and(|json| !json.contains(LOGIN))
    });
}
