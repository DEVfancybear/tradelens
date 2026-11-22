//! The advanced trade journal, the form editor, the entry-plan prompt and manual entry,
//! headless on the real engine over a database in a temporary data directory.

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use nsh_core::{Size, platform::Os, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless};
use tradelens_bridge::api;
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, TradeKey,
    catalog::{FieldDef, FieldKind, FieldScope, FieldValue, Requirement},
    journal::EntryPlan,
    model::AccountInfo,
};

const DAY_MS: i64 = 86_400_000;

/// A data directory with a database in it, and the engine's view of it.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut db = Db::open(dir.path().join("tradelens.db")).expect("the database opens");
        // An ephemeral port, so no test claims the bridge's default one.
        db.set_setting("bridge_port", "0").expect("the port is stored");
        Self { dir }
    }

    fn db(&self) -> Db {
        Db::open(self.dir.path().join("tradelens.db")).expect("the database opens")
    }

    fn start(&self) -> Headless {
        let mut config = EngineConfig {
            viewport: Size::new(px(1280.0), px(1000.0)),
            data_dir: Some(self.dir.path().to_path_buf()),
            ..EngineConfig::new(Root::of::<nsh_app_tradelens::Workspace>())
        };
        // A desktop that delivers keys to the focused view, and takes dropped files.
        config.environment.os = Os::Linux;
        let mut app = Headless::start(config).expect("the app starts");
        app.wait_for("Tổng quan").expect("the app opens");
        app
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_millis(),
    )
    .expect("milliseconds fit")
}

#[expect(clippy::too_many_arguments, reason = "a fixture that spells out one deal")]
fn deal(
    ticket: i64,
    position: i64,
    at_ms: i64,
    kind: DealKind,
    entry: DealEntry,
    price: f64,
    sl: f64,
    tp: f64,
) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: "XAUUSD".into(),
        kind,
        entry,
        volume: 0.5,
        price,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: if entry == DealEntry::Out { 100.0 } else { 0.0 },
        magic: 0,
        comment: String::new(),
        sl,
        tp,
        risk: None,
        reason: None,
    }
}

fn batch(deals: Vec<Deal>) -> DealBatch {
    DealBatch {
        account: AccountInfo {
            login: "52012345".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            balance: Some(10_000.0),
            ..AccountInfo::default()
        },
        deals,
        positions: Vec::new(),
    }
}

/// Two closed XAUUSD trades (positions 1 and 2), the entry at 2648.2 with a stop at 2640.
fn seed(db: &mut Db) {
    let exit = now_ms() - 2 * DAY_MS;
    let mut deals = Vec::new();
    for position in [1_i64, 2] {
        let at = exit + position * 60_000;
        deals.push(deal(position * 10, position, at - 3_600_000, DealKind::Buy, DealEntry::In, 2648.2, 2640.0, 2660.0));
        deals.push(deal(position * 10 + 1, position, at, DealKind::Sell, DealEntry::Out, 2656.0, 2640.0, 2660.0));
    }
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
}

fn key_of(db: &Db, position: i64) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: position, segment: 0 }
}

/// Waits until `check` holds, polling at most `within`.
fn wait_for_database(what: &str, within: Duration, check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < within, "{what} did not happen within {within:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn open_trade(app: &mut Headless, symbol: &str) {
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for(&format!("role:button[Mở nhật ký lệnh {symbol}]")).expect("the trades load");
    app.press(&format!("role:button[Mở nhật ký lệnh {symbol}] >> nth=0")).expect("a trade opens");
    app.wait_for("Nhật ký lệnh").expect("the journal card shows");
}

fn focus_property(scope: FieldScope, kind: FieldKind) -> FieldDef {
    FieldDef {
        id: 0,
        scope,
        key: "focus".into(),
        label: "Khung vào lệnh".into(),
        kind,
        options: Vec::new(),
        max: None,
        required: Requirement::Optional,
        analysed: false,
        position: 0,
        archived: false,
    }
}

#[test]
fn a_custom_property_edited_on_a_trade_is_saved_with_the_journal() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    db.apply_field_def(&focus_property(FieldScope::Trade, FieldKind::Text)).expect("the property is added");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("label:Sửa thuộc tính Khung vào lệnh").expect("the trader's own property is in the form");

    app.type_text("label:Khung vào lệnh", "M15").expect("types in the property");
    app.press("role:button[Điểm thực thi: 3 sao]").expect("a built-in is edited next to it");
    app.press("role:button[Lưu]").expect("saves");
    wait_for_database("the journal being saved", Duration::from_secs(5), || {
        let key = key_of(&db, 2);
        db.journal(&key).expect("reads").is_some_and(|e| e.custom.contains_key("focus"))
    });
    let entry = db.journal(&key_of(&db, 2)).expect("reads").expect("an entry");
    assert_eq!(entry.custom["focus"], FieldValue::Text("M15".into()));
    assert_eq!(entry.rating, 3);
}

#[test]
fn the_exit_deals_reason_prefills_the_exit_reason_without_creating_an_entry() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let exit = now_ms() - DAY_MS;
    let mut closing = deal(11, 1, exit, DealKind::Sell, DealEntry::Out, 2640.0, 2640.0, 2660.0);
    closing.reason = Some("DEAL_REASON_SL".into());
    let opening = deal(10, 1, exit - 3_600_000, DealKind::Buy, DealEntry::In, 2648.2, 2640.0, 2660.0);
    db.upsert_batch(&batch(vec![opening, closing]), "test").expect("stored");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("EA gợi ý: Chạm SL").expect("the deal's reason is suggested");
    app.wait_for("Đã lưu").expect("nothing is pending");
    assert!(db.journal(&key_of(&db, 1)).expect("reads").is_none(), "opening a trade writes no entry");
}

#[test]
fn the_plan_is_set_against_what_the_trade_did() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let planned = JournalEntry {
        plan: Some(EntryPlan {
            thesis: "Phá đỉnh phiên Á, retest giữ được".into(),
            entry: Some(2648.0),
            stop_loss: Some(2640.0),
            take_profit: Some(2660.0),
            timeframe: "M15".into(),
            written_at: 1,
            ..EntryPlan::default()
        }),
        ..JournalEntry::default()
    };
    db.save_journal(&key_of(&db, 2), &planned).expect("the plan is stored");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("Kế hoạch lúc vào vs thực tế").expect("the comparison shows");
    app.wait_for("Phá đỉnh phiên Á, retest giữ được").expect("the thesis shows");
    app.wait_for("+0.20 (trượt giá)").expect("the entry slippage is named");
    app.wait_for("Viết trước khi đóng").expect("the plan was written before the close");
    app.wait_for("đúng kế hoạch").expect("the stop was kept");

    // Another trade has no plan: it offers to write one.
    app.press("role:button[‹ Lệnh trước]").expect("steps to the other trade");
    app.wait_for("Chưa có kế hoạch cho lệnh này").expect("the empty state says so");
    app.press("role:button[Viết kế hoạch]").expect("the plan form opens over the page");
    app.wait_for("Lưu kế hoạch").expect("the plan dialog shows");
}

#[test]
fn a_picture_added_to_a_trade_is_saved_into_its_journal() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let source = fixture.dir.path().join("h1.png");
    std::fs::write(&source, b"png one").expect("writes the source");
    let uri = url::Url::from_file_path(&source).map(String::from).expect("absolute");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("label:Thêm ảnh vào Ảnh chart").expect("the gallery shows");
    app.drop_files("label:Thêm ảnh vào Ảnh chart", vec![uri]).expect("drops a picture");
    wait_for_database("the picture being saved", Duration::from_secs(5), || {
        db.journal(&key_of(&db, 2)).expect("reads").is_some_and(|e| e.screenshots.len() == 1)
    });
    let entry = db.journal(&key_of(&db, 2)).expect("reads").expect("an entry");
    assert!(entry.screenshots[0].path.starts_with("images/"), "{}", entry.screenshots[0].path);
}

#[test]
fn changing_a_propertys_type_in_the_form_editor_converts_every_entry() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    db.apply_field_def(&focus_property(FieldScope::Trade, FieldKind::Text)).expect("the property is added");
    for (position, text) in [(1_i64, "7"), (2, "cao")] {
        let mut entry = JournalEntry::default();
        entry.custom.insert("focus".into(), FieldValue::Text(text.into()));
        let key = key_of(&db, position);
        db.save_journal(&key, &entry).expect("stored");
    }
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.press("role:button[Tuỳ biến form]").expect("the link to the form editor");
    app.wait_for("Tuỳ biến form nhập liệu").expect("the editor opens over the page");
    app.wait_for("label:Sửa thuộc tính Khung vào lệnh").expect("the property is in the table");

    app.on_dialog(|_| DialogAnswer::Press("Đổi kiểu".to_owned()));
    app.press("label:Sửa thuộc tính Khung vào lệnh").expect("picks the property");
    app.press("label:Kiểu").expect("the kind drop-down");
    app.press("text:# Số").expect("the number kind");
    wait_for_database("the stored values following the new type", Duration::from_secs(5), || {
        let value =
            |position| db.journal(&key_of(&db, position)).expect("reads").and_then(|e| e.custom.get("focus").cloned());
        value(1) == Some(FieldValue::Number(7.0)) && value(2).is_none()
    });
    assert_eq!(db.field_defs(FieldScope::Trade).expect("reads")[0].kind, FieldKind::Number);

    // Hiding a property is saved with the form's arrangement; closing the editor shows the trade again.
    app.press("label:Ẩn Khung vào lệnh").expect("hides it");
    wait_for_database("the layout being saved", Duration::from_secs(5), || {
        db.form_layout(FieldScope::Trade).expect("reads").items.iter().any(|i| i.hidden)
    });
    app.press("Xong").expect("closes the editor");
    app.wait_for("Chi tiết lệnh").expect("back on the trade");
}

/// One request to the bridge on its own connection.
fn post(port: u16, token: &str, body: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream.set_read_timeout(Some(Duration::from_secs(20))).expect("sets a timeout");
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\n{}: {token}\r\nContent-Length: {}\r\n\r\n",
        api::DEALS,
        api::TOKEN_HEADER,
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("sends the head");
    stream.write_all(body.as_bytes()).expect("sends the body");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("reads the answer");
    raw.split(' ').nth(1).and_then(|s| s.parse().ok()).expect("a status line")
}

/// The port the bridge listens on, from the sidebar's status line.
fn bridge_port(app: &mut Headless) -> u16 {
    app.wait_until("the bridge listens", Duration::from_secs(10), |screen| {
        screen.texts().iter().any(|t| t.starts_with("Bridge MT5: đang nghe cổng"))
    })
    .expect("the bridge starts");
    app.screen()
        .texts()
        .iter()
        .find_map(|t| t.strip_prefix("Bridge MT5: đang nghe cổng ").and_then(|p| p.trim().parse().ok()))
        .expect("the status line names a port")
}

/// What the EA sends when two positions open: an `in` deal each.
fn opened_body(positions: &[i64]) -> String {
    let at = now_ms();
    let deals: Vec<String> = positions
        .iter()
        .map(|p| {
            format!(
                r#"{{"ticket":{},"order":{},"position_id":{p},"time_msc":{at},"symbol":"XAUUSD","type":"buy","entry":"in","volume":0.5,"price":2648.2,"commission":0,"swap":0,"profit":0,"sl":2644.3,"tp":2660.0,"risk":0}}"#,
                p * 10,
                p * 10
            )
        })
        .collect();
    format!(r#"{{"account":{{"login":"52012345","server":"Demo","currency":"USD"}},"deals":[{}]}}"#, deals.join(","))
}

fn token(fixture: &Fixture) -> String {
    fixture.db().setting("bridge_token").expect("reads").expect("the app made a token")
}

#[test]
fn a_position_the_ea_reports_asks_for_its_plan_and_waiting_ones_queue() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    let (port, token) = (bridge_port(&mut app), token(&fixture));

    // Two positions open together: one dialog, with the other waiting behind it.
    assert_eq!(post(port, &token, &opened_body(&[91, 92])), 200);
    app.wait_for_within("Lưu kế hoạch", Duration::from_secs(10)).expect("the prompt opens over the overview");
    app.wait_for("còn 1 lệnh đang chờ ghi kế hoạch").expect("the second one waits");
    app.wait_for("text:/^EA MT5 · lệnh vừa mở/").expect("it says where it came from");

    // "Ghi sau" keeps the first in the review queue and brings up the second.
    app.press("role:button[Ghi sau]").expect("postpones the plan");
    let db = fixture.db();
    wait_for_database("the first trade being flagged", Duration::from_secs(5), || {
        db.journal(&key_of(&db, 91)).expect("reads").is_some_and(|e| e.review_later)
    });
    app.wait_for_gone("còn 1 lệnh đang chờ ghi kế hoạch", Duration::from_secs(5)).expect("the queue shrank");
    app.wait_for("Lưu kế hoạch").expect("the second prompt is up");

    // The second plan is written and saved to that trade's journal.
    app.type_text("label:Lý do vào lệnh", "Phá đỉnh phiên Á").expect("the reason is typed");
    app.press("Lưu kế hoạch").expect("saves the plan");
    wait_for_database("the plan being saved", Duration::from_secs(5), || {
        db.journal(&key_of(&db, 92)).expect("reads").is_some_and(|e| e.plan.is_some())
    });
    let entry = db.journal(&key_of(&db, 92)).expect("reads").expect("an entry");
    let plan = entry.plan.expect("a plan");
    assert_eq!(plan.thesis, "Phá đỉnh phiên Á");
    assert_eq!(plan.entry, Some(2648.2), "the levels start from MT5's");
    assert_eq!(plan.stop_loss, Some(2644.3));
    assert!(!entry.review_later);
    app.wait_for_gone("Lưu kế hoạch", Duration::from_secs(5)).expect("the dialog closes with nothing waiting");
    assert!(db.journal(&key_of(&db, 91)).expect("reads").expect("an entry").plan.is_none());
}

#[test]
fn a_trade_typed_in_by_hand_is_stored_with_its_journal() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Hiển thị 2 / 2 lệnh").expect("the trades load");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("the form opens");

    app.press("role:button[Lưu lệnh]").expect("saves an empty form");
    app.wait_for("text:/Nhập mã giao dịch/").expect("the form says what is missing");

    app.type_text("label:Symbol", "GBPUSD").expect("symbol");
    app.press("role:button[Chiều: Sell]").expect("side");
    app.type_text("label:Thời gian vào", "2026-11-20 16:05").expect("opened");
    app.type_text("label:Thời gian ra", "2026-11-20 17:40").expect("closed");
    app.type_text("label:Khối lượng", "0.8").expect("volume");
    app.type_text("label:Giá vào", "1.2684").expect("open price");
    app.type_text("label:Giá ra", "1.2651").expect("close price");
    app.type_text("label:Lãi/lỗ", "264").expect("profit");
    app.type_text("label:Commission", "-5.6").expect("commission");
    app.wait_for("Tự tính: kết quả ròng +$258.40 · giữ 1g 35p").expect("the computed line follows");
    app.press("role:button[Tag: Tin tức]").expect("the journal below is the trade journal");
    app.type_text("label:Ghi chú", "Vào sớm vì sợ lỡ").expect("notes");
    app.press("role:button[Lưu lệnh]").expect("saves");

    let db = fixture.db();
    wait_for_database("the trade being stored", Duration::from_secs(5), || {
        db.deals(None).expect("reads").iter().any(|(_, d)| d.symbol == "GBPUSD")
    });
    let deals: Vec<Deal> =
        db.deals(None).expect("reads").into_iter().map(|(_, d)| d).filter(|d| d.symbol == "GBPUSD").collect();
    assert_eq!(deals.len(), 2, "an entry and an exit deal");
    assert!(deals.iter().all(|d| d.ticket < 0), "hand-entered deals have negative tickets");
    let key =
        TradeKey { account_id: db.accounts().expect("reads")[0].id, position_id: deals[0].position_id, segment: 0 };
    wait_for_database("the journal being stored", Duration::from_secs(5), || {
        db.journal(&key).expect("reads").is_some()
    });
    let entry = db.journal(&key).expect("reads").expect("an entry");
    assert_eq!(entry.notes, "Vào sớm vì sợ lỡ");
    assert_eq!(entry.tags, ["news"]);
    app.wait_for("Chi tiết lệnh").expect("the new trade opens");
}

#[test]
fn ctrl_n_opens_the_manual_entry_form_from_the_trades_list() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Chưa có lệnh nào").expect("the trades page opens");
    app.press_key("ctrl+n").expect("the shortcut");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("the form opens");
    app.press("role:button[Huỷ]").expect("cancels");
    app.wait_for("Chưa có lệnh nào").expect("back on the list");
}

#[test]
fn the_plan_prompt_warns_when_the_planned_risk_breaks_the_accounts_rule() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let account = db.accounts().expect("accounts read").remove(0).id;
    db.save_rules(account, &tradelens_core::Rules { max_risk_pct: Some(1.5), ..tradelens_core::Rules::default() })
        .expect("the rules are stored");
    drop(db);
    let mut app = fixture.start();
    let (port, token) = (bridge_port(&mut app), token(&fixture));

    assert_eq!(post(port, &token, &opened_body(&[93])), 200);
    app.wait_for_within("Lưu kế hoạch", Duration::from_secs(10)).expect("the prompt opens");
    app.wait_for_gone("text:/vượt quy tắc/", Duration::from_secs(1)).expect("no risk is planned yet, so no warning");

    // 300 of the 10,000 balance is 3%, over the 1.5% the account allows.
    app.type_text("label:Risk", "300").expect("the risk is typed");
    app.wait_for("Risk 3.0% vượt quy tắc 1.5% của tài khoản 52012345 · Demo.").expect("the rule is warned about");

    // Within the rule the warning goes.
    app.clear("label:Risk").expect("the risk is cleared");
    app.type_text("label:Risk", "100").expect("the risk is retyped");
    app.wait_for_gone("text:/vượt quy tắc/", Duration::from_secs(5)).expect("the warning goes with the risk");
}
