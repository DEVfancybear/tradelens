//! What a trader types is never lost to the data reading again.
//!
//! Every page reads the database again whenever anything is stored (a sync, an import, a write from
//! another window, a write of the page itself): the store's snapshot announces it every time, and so
//! does every `Reading`. A field built inside a reactive block that follows one of them is made again
//! at each announcement, and the rest of what is being typed goes to the field that is gone.
//!
//! Each test types into the forms of a page piece by piece, with a reload between the pieces
//! (`support::type_through_reloads`): the first piece focuses the field, the others are typed on
//! whatever has the focus, so a field that was made again (and lost the focus) shows it by missing
//! the rest. The tests that end in a stored value check that it is what was typed.

use std::time::Duration;

use nsh_core::platform::Os;
use nsh_testing::Headless;
use tradelens_core::{
    ChecklistItem, Db, ManualTrade, Side,
    catalog::{FieldDef, FieldKind, FieldScope, Requirement},
    model::AccountInfo,
    playbook::Playbook,
    review::{Period, PeriodReview},
    time::parse_mt5_time,
};

mod support;
use support::{Fixture, WAIT, wait_until};

/// How long the app is given to read the database again after [`reload`]: the store compares the
/// revision with the one it last read every 400 ms (`store::POLL`), and a read takes milliseconds.
const RELOAD: Duration = Duration::from_millis(1_000);

/// Makes every page read the database again, as a sync, an import or a write from another window
/// does (`data_changed` moves the process-wide revision), and runs the app for as long as that takes.
/// Nothing on screen says that the reading is done, so this is the one wait that is a time.
fn reload(app: &mut Headless) {
    nsh_app_tradelens::data_changed();
    app.wait(RELOAD).expect("the app runs while it reads again");
}

/// Types `pieces` one after the other into the field `selector` names, the data reading again between
/// every two pieces, then checks that the field ends with all of them.
///
/// The first piece goes in by selector, which gives the field the keyboard focus. Every later piece is
/// typed as a keyboard does, on the field that has the focus: a field that a reload made again has
/// lost it, and the rest of the typing goes nowhere, as it would for a trader. Pieces are letters and
/// digits (a key each), never spaces.
///
/// # Panics
///
/// When the field does not hold what was typed after the last piece.
fn type_through_reloads(app: &mut Headless, selector: &str, pieces: &[&str]) {
    type_through_changes(app, selector, pieces, || {});
}

/// [`type_through_reloads`] with `change` run before each reload: a write of another connection, so
/// that the data the pages read again is not the same as before.
///
/// # Panics
///
/// When the field does not hold what was typed after the last piece.
fn type_through_changes(app: &mut Headless, selector: &str, pieces: &[&str], mut change: impl FnMut()) {
    let Some((first, later)) = pieces.split_first() else { return };
    app.type_text(selector, first).unwrap_or_else(|e| panic!("`{selector}` takes the first piece: {e}"));
    for piece in later {
        change();
        reload(app);
        for key in piece.chars() {
            app.press_key(&key.to_string()).unwrap_or_else(|e| panic!("`{key}` is typed on the focused field: {e}"));
        }
    }
    let typed = pieces.concat();
    let held = app.screen().input_text(selector).unwrap_or_else(|e| panic!("`{selector}`: {e}"));
    assert!(
        held.is_some_and(|text| text.ends_with(&typed)),
        "`{selector}` was typed {pieces:?} with the data reading again between the pieces, and holds {held:?}"
    );
}

/// The window: a desktop that delivers keys to the focused field, and tall enough for the forms.
fn start(fixture: &Fixture) -> Headless {
    fixture.start(1500.0, 1300.0, |config| config.environment.os = Os::Linux)
}

fn open_page(app: &mut Headless, row: &str, marker: &str) {
    app.press(&format!("role:button[{row}]")).expect("the sidebar row is a button");
    app.wait_for(marker).expect("the page opens");
}

fn account(db: &mut Db) -> i64 {
    db.upsert_account(&AccountInfo {
        login: "5012345".into(),
        server: "Demo".into(),
        currency: "USD".into(),
        ..AccountInfo::default()
    })
    .expect("an account")
}

fn text_property(scope: FieldScope, key: &str, label: &str) -> FieldDef {
    FieldDef {
        id: 0,
        scope,
        key: key.into(),
        label: label.into(),
        kind: FieldKind::Text,
        options: Vec::new(),
        max: None,
        required: Requirement::Optional,
        analysed: false,
        position: 0,
        archived: false,
    }
}

fn time(text: &str) -> i64 {
    parse_mt5_time(text).expect("fixture times parse")
}

/// A trade with money, typed by hand.
fn with_money(db: &mut Db, account: i64, symbol: &str) -> tradelens_core::TradeKey {
    let trade = ManualTrade {
        symbol: symbol.into(),
        side: Side::Long,
        volume: Some(0.5),
        open_time: time("2026.11.10 08:00:00"),
        open_price: Some(1.0850),
        close_time: time("2026.11.10 09:00:00"),
        close_price: Some(1.0860),
        date_only: false,
        sl: 1.0830,
        tp: 0.0,
        profit: Some(25.0),
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    };
    db.add_manual_trade(account, &trade).expect("a trade")
}

/// A trade kept in R alone: no money, no lot, no prices.
fn in_r(db: &mut Db, account: i64, symbol: &str) {
    let trade = ManualTrade {
        symbol: symbol.into(),
        side: Side::Long,
        volume: None,
        open_time: time("2026.11.10 00:00:00"),
        open_price: None,
        close_time: time("2026.11.10 00:00:00"),
        close_price: None,
        date_only: true,
        sl: 0.0,
        tp: 0.0,
        profit: None,
        commission: 0.0,
        swap: 0.0,
        result_r: Some(1.5),
    };
    db.add_manual_trade(account, &trade).expect("a trade in R");
}

// --- Nhật ký ngày -----------------------------------------------------------------------------------

#[test]
fn the_plan_the_review_and_a_property_of_the_day_keep_their_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    account(&mut db);
    db.save_field_def(&text_property(FieldScope::Day, "sleep", "Giấc ngủ")).expect("a day property");
    let mut app = start(&fixture);
    open_page(&mut app, "Nhật ký ngày", "Kế hoạch và tổng kết");
    app.wait_for("label:Kế hoạch ngày").expect("the day is read and the form is there");

    type_through_reloads(&mut app, "label:Kế hoạch ngày", &["cho", "tin", "moi"]);
    type_through_reloads(&mut app, "label:Tổng kết cuối ngày", &["giu", "ky", "luat"]);
    type_through_reloads(&mut app, "label:Giấc ngủ", &["bay", "tieng"]);

    // The page saves by itself, and the saving reads the day again too: nothing typed goes missing.
    let today = chrono::Local::now().date_naive();
    let slept = tradelens_core::catalog::FieldValue::Text("baytieng".into());
    wait_until("the day being saved", || {
        db.day_values(0, today).is_ok_and(|values| values.get("sleep") == Some(&slept))
    });
    let note = db.day_note(0, today).expect("reads");
    assert_eq!((note.plan.as_str(), note.review.as_str()), ("chotinmoi", "giukyluat"));
}

// --- Nhật ký lệnh -----------------------------------------------------------------------------------

#[test]
fn the_notes_a_property_and_the_setup_of_a_trade_keep_their_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    db.apply_field_def(&text_property(FieldScope::Trade, "focus", "Khung vào lệnh")).expect("a trade property");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("the trade opens");
    app.wait_for("label:Ghi chú").expect("the journal form shows");

    type_through_reloads(&mut app, "label:Ghi chú", &["vao", "som", "qua"]);
    type_through_reloads(&mut app, "label:Khung vào lệnh", &["m", "15"]);

    // A playbook stored meanwhile (a sync) gives the setup a drop-down; the name being typed stays.
    let mut other = fixture.db();
    let mut n = 0;
    type_through_changes(&mut app, "label:Setup", &["brk", "out", "asia"], || {
        n += 1;
        other.save_playbook(&Playbook { name: format!("Mau {n}"), ..Playbook::default() }, 0).expect("a playbook");
    });
}

#[test]
fn the_toggles_and_boxes_of_a_trades_form_are_not_made_again_when_the_data_reads_again() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    db.save_checklist(None, &[ChecklistItem { id: 0, text: "Cung chieu H4".into() }]).expect("a checklist");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("the trade opens");
    app.wait_for("role:button[Tag: Tin tức]").expect("the tags show");
    app.wait_for("label:Checklist trước lệnh: Cung chieu H4").expect("the checklist shows");

    // A control made again is another node, and has lost the keyboard focus the trader gave it.
    let controls =
        ["role:button[Tag: Tin tức]", "role:button[Lỗi: Vào sớm]", "label:Checklist trước lệnh: Cung chieu H4"];
    let ids = |app: &Headless| controls.map(|selector| app.screen().id(selector).expect("the control shows"));
    let before = ids(&app);
    reload(&mut app);
    assert_eq!(ids(&app), before, "the controls {controls:?} are the same nodes after the data was read again");
}

// --- Playbook ---------------------------------------------------------------------------------------

#[test]
fn the_form_of_a_playbook_keeps_its_typing() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    open_page(&mut app, "Playbook", "Playbook của bạn");
    app.press("role:button[Thêm playbook]").expect("the add button");
    app.wait_for("label:Tên playbook").expect("the form opens");

    type_through_reloads(&mut app, "label:Tên playbook", &["pull", "back", "ema"]);
    type_through_reloads(&mut app, "label:Mô tả", &["gia", "cham", "ema"]);
    type_through_reloads(&mut app, "label:Symbol áp dụng", &["eur", "usd"]);
    type_through_reloads(&mut app, "label:Risk mặc định", &["1", "2"]);
    type_through_reloads(&mut app, "label:Huỷ setup khi", &["tin", "do"]);
    app.press("role:button[Thêm điều kiện vào]").expect("a rule is added");
    type_through_reloads(&mut app, "label:Điều kiện vào 1", &["gia", "cham", "ema"]);
    app.press("role:button[Thêm mục checklist]").expect("a checklist item is added");
    type_through_reloads(&mut app, "label:Mục checklist 1", &["co", "can"]);

    app.press("role:button[Lưu]").expect("saves");
    wait_until("the setup being stored", || fixture.db().playbooks().is_ok_and(|p| p.len() == 1));
    let stored = fixture.db().playbooks().expect("reads").remove(0);
    assert_eq!(stored.name, "pullbackema");
    assert_eq!(stored.description, "giachamema");
    assert_eq!(stored.entry_rules, ["giachamema"]);
    assert_eq!(stored.checklist, ["cocan"]);
}

// --- Lệnh bỏ lỡ -------------------------------------------------------------------------------------

#[test]
fn the_form_of_a_missed_trade_keeps_its_typing() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh bỏ lỡ", "Ghi lệnh bỏ lỡ");
    app.wait_for("label:Symbol").expect("the form shows");

    type_through_reloads(&mut app, "label:Symbol", &["xau", "usd"]);
    type_through_reloads(&mut app, "label:Ghi chú", &["do", "dua", "qua"]);
    type_through_reloads(&mut app, "label:Entry", &["26", "48"]);
    type_through_reloads(&mut app, "label:Thêm lý do", &["so", "mat"]);

    app.press("role:button[Lưu]").expect("saves");
    wait_until("the missed trade being stored", || fixture.db().missed_trades(0, i64::MAX).is_ok_and(|m| m.len() == 1));
    let stored = fixture.db().missed_trades(0, i64::MAX).expect("reads").remove(0);
    assert_eq!(stored.symbol, "XAUUSD");
    assert_eq!(stored.notes, "doduaqua");
    assert_eq!(stored.entry, Some(2648.0));
}

// --- Tổng kết & xem lại -----------------------------------------------------------------------------

#[test]
fn the_answers_of_a_review_and_the_questions_of_its_template_keep_their_typing() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    open_page(&mut app, "Tổng kết & xem lại", "Câu hỏi tổng kết");
    app.wait_for("label:3 điều làm tốt tuần này").expect("the questions show");

    type_through_reloads(&mut app, "label:3 điều làm tốt tuần này", &["cho", "retest", "dung"]);
    type_through_reloads(&mut app, "label:Thêm câu hỏi", &["cau", "moi"]);

    app.press("role:button[Sửa mẫu Tổng kết tháng]").expect("the template opens");
    app.wait_for("label:Câu hỏi 1").expect("its questions show");
    type_through_reloads(&mut app, "label:Câu hỏi 1", &["diem", "vao", "tot"]);
}

#[test]
fn a_review_stored_elsewhere_while_its_answers_are_typed_does_not_take_the_typing_away() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    open_page(&mut app, "Tổng kết & xem lại", "Câu hỏi tổng kết");
    app.wait_for("label:3 điều làm tốt tuần này").expect("the questions show");

    // Another window (or another device's sync) stores this week's review while the trader writes.
    // No account is known, so the page counts the week on the machine's own clock.
    let start_of_week = Period::Week.bounds(chrono::Local::now().date_naive()).0;
    let mut other = fixture.db();
    let mut rating = 0;
    type_through_changes(&mut app, "label:3 điều làm tốt tuần này", &["cho", "retest", "dung"], || {
        rating = rating % 5 + 1;
        let template = other.review_template(Period::Week).expect("the template");
        let review = PeriodReview { rating, ..PeriodReview::blank(&template, start_of_week, None) };
        other.save_review(&review).expect("a review is stored");
    });
}

// --- Tài khoản & kỷ luật ----------------------------------------------------------------------------

#[test]
fn the_rules_the_details_and_the_checklists_of_an_account_keep_their_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    db.save_checklist(None, &[ChecklistItem { id: 0, text: "Cung chieu H4".into() }]).expect("a checklist");
    let mut app = start(&fixture);
    open_page(&mut app, "Tài khoản & kỷ luật", "Quy tắc kỷ luật");
    app.wait_for("Cung chieu H4").expect("the checklist shows");

    app.press("label:Lỗ tối đa trong ngày").expect("the loss rule's switch");
    app.wait_for("label:Lỗ tối đa trong ngày: giá trị").expect("the limit shows");
    type_through_reloads(&mut app, "label:Lỗ tối đa trong ngày: giá trị", &["1", "2", "3"]);
    type_through_reloads(&mut app, "label:Tên hiển thị", &["tai", "khoan", "chinh"]);
    type_through_reloads(&mut app, "label:Broker", &["ex", "ness"]);
    type_through_reloads(&mut app, "label:Checklist dùng chung: mục mới", &["co", "can"]);
    type_through_reloads(&mut app, "label:Checklist dùng chung: mục 1", &["ok", "roi"]);
}

#[test]
fn leaving_a_checklist_item_for_the_next_one_does_not_take_the_next_one_away() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    db.save_checklist(
        None,
        &[ChecklistItem { id: 0, text: "Mot".into() }, ChecklistItem { id: 0, text: "Hai".into() }],
    )
    .expect("a checklist");
    let mut app = start(&fixture);
    open_page(&mut app, "Tài khoản & kỷ luật", "Quy tắc kỷ luật");
    app.wait_for("Hai").expect("the checklist shows");

    // Leaving the first item keeps its new name, which changes the list the rows are drawn from.
    app.type_text("label:Checklist dùng chung: mục 1", "a").expect("renames the first");
    type_through_reloads(&mut app, "label:Checklist dùng chung: mục 2", &["b", "c"]);
    wait_until("the first item being renamed", || {
        fixture.db().checklist(None).is_ok_and(|items| items.first().is_some_and(|i| i.text == "Mota"))
    });
    // The second is kept when it is left.
    app.submit("label:Checklist dùng chung: mục 2").expect("Enter keeps it");
    wait_until("the second item being renamed", || {
        fixture.db().checklist(None).is_ok_and(|items| items.get(1).is_some_and(|i| i.text == "Haibc"))
    });
}

// --- Tuỳ biến form ----------------------------------------------------------------------------------

#[test]
fn renaming_a_property_in_the_form_editor_keeps_its_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    db.apply_field_def(&text_property(FieldScope::Trade, "focus", "Khung vào lệnh")).expect("a trade property");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("the trade opens");
    app.press("role:button[Tuỳ biến form]").expect("the link to the form editor");
    app.wait_for("Tuỳ biến form nhập liệu").expect("the editor opens");
    app.press("role:button[Sửa thuộc tính Khung vào lệnh]").expect("the property is picked");
    app.wait_for("label:Tên thuộc tính").expect("its panel shows");

    type_through_reloads(&mut app, "label:Tên thuộc tính", &["moi", "ten"]);

    // The rename is kept once Enter is pressed, and what was typed is what is stored.
    app.submit("label:Tên thuộc tính").expect("Enter keeps the name");
    wait_until("the property being renamed", || {
        fixture
            .db()
            .field_defs(FieldScope::Trade)
            .is_ok_and(|defs| defs.iter().any(|d| d.key == "focus" && d.label.ends_with("moiten")))
    });
}

#[test]
fn leaving_an_option_of_a_property_for_the_next_one_does_not_take_the_next_one_away() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    let choice = FieldDef {
        kind: FieldKind::Select,
        options: vec!["Ca".into(), "Cb".into()],
        ..text_property(FieldScope::Trade, "frame", "Khung")
    };
    db.apply_field_def(&choice).expect("a choice property");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("the trade opens");

    // In the popover of the property on the trade's own page ...
    app.press("role:button[Sửa thuộc tính Khung]").expect("the property's popover");
    app.wait_for("label:Tên lựa chọn Cb").expect("its options show");
    app.type_text("label:Tên lựa chọn Ca", "1").expect("renames the first");
    type_through_reloads(&mut app, "label:Tên lựa chọn Cb", &["2", "3"]);
    app.submit("label:Tên lựa chọn Cb").expect("Enter keeps it");
    wait_until("the options being renamed", || {
        fixture.db().field_defs(FieldScope::Trade).is_ok_and(|defs| {
            defs.iter().any(|d| d.key == "frame" && d.options == ["Ca1".to_owned(), "Cb23".to_owned()])
        })
    });
}

#[test]
fn typing_the_top_of_a_scale_in_the_form_editor_is_not_cut_short_by_the_edit_it_makes() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    let scale = FieldDef { kind: FieldKind::Scale, max: Some(5), ..text_property(FieldScope::Trade, "grade", "Mức") };
    db.apply_field_def(&scale).expect("a scale property");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("the trade opens");
    app.press("role:button[Tuỳ biến form]").expect("the link to the form editor");
    app.press("role:button[Sửa thuộc tính Mức]").expect("the property is picked");
    app.wait_for("label:Điểm tối đa").expect("its limit shows");

    // Every digit is an edit of the property, which is read back before the next one comes.
    app.clear("label:Điểm tối đa").expect("clears the limit");
    type_through_reloads(&mut app, "label:Điểm tối đa", &["2", "0"]);
}

// --- Nhập lệnh tay ----------------------------------------------------------------------------------

#[test]
fn the_result_in_r_of_a_new_trade_keeps_its_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    in_r(&mut db, account, "EURUSD");
    let mut app = start(&fixture);
    open_page(&mut app, "Lệnh", "role:button[Mở nhật ký lệnh EURUSD]");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for_within("Nhập lệnh & nhật ký thủ công", WAIT).expect("the form opens");
    // The account keeps its trades in R: the result leads, once the data says so.
    app.wait_for("text:/^Nhật ký ghi bằng R/").expect("the form leads with R");

    type_through_reloads(&mut app, "label:Kết quả (R)", &["1", "2", "5"]);
    type_through_reloads(&mut app, "label:Symbol", &["eur", "usd"]);
    type_through_reloads(&mut app, "label:Ghi chú", &["vao", "som"]);
}

// --- Bảng tìm ---------------------------------------------------------------------------------------

#[test]
fn the_search_box_keeps_its_typing() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let account = account(&mut db);
    with_money(&mut db, account, "EURUSD");
    let mut app = start(&fixture);
    app.press("role:button[Mở tìm kiếm]").expect("the search opens");
    app.wait_for("label:Tìm trong nhật ký").expect("the field shows");

    type_through_reloads(&mut app, "label:Tìm trong nhật ký", &["eur", "usd"]);
    reload(&mut app);
}
