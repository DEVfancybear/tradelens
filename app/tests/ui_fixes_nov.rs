//! Fixes found by hand on the real window, each pinned headless: a day of the overview's small month
//! opens the calendar on it, a symbol of the overview opens its trades, the calendar's month title
//! picks a day from a date picker, the playbook form keeps the list of timeframes, "Ghi lệnh bỏ lỡ"
//! gives the keyboard to the symbol, and the rows of the analytics tables hold their text in the
//! middle of their height.

use nsh_core::Command;
use nsh_core::platform::Os;
use nsh_testing::{DialogAnswer, Headless, Screen};
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, catalog::CatalogKind, model::AccountInfo, time::parse_mt5_time,
};

mod support;
use support::{Fixture, WAIT, wait_until};

const DAY_MS: i64 = 86_400_000;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn deal(ticket: i64, position: i64, symbol: &str, at_ms: i64, entry: DealEntry, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: symbol.into(),
        kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
        entry,
        volume: 0.5,
        price: 1.1,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

/// A closed trade that exits at `exit_ms` an hour after it entered.
fn closed_at(position: i64, symbol: &str, exit_ms: i64, profit: f64) -> Vec<Deal> {
    vec![
        deal(position * 10, position, symbol, exit_ms - 3_600_000, DealEntry::In, 0.0),
        deal(position * 10 + 1, position, symbol, exit_ms, DealEntry::Out, profit),
    ]
}

fn store(db: &mut Db, deals: Vec<Deal>) {
    let batch = DealBatch {
        account: AccountInfo {
            login: "52012345".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            balance: Some(10_000.0),
            ..AccountInfo::default()
        },
        deals,
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("the batch is stored");
}

/// Two trades on 10 March 2026 (EURUSD +100, XAUUSD −40) and one on the 11th (GBPJPY +20): the 11th
/// is the latest day with trades, which the calendar opens on by itself.
fn seed_march(db: &mut Db) {
    let at = |s: &str| parse_mt5_time(s).expect("fixture times parse");
    let mut deals = closed_at(1, "EURUSD", at("2026.03.10 10:00:00"), 100.0);
    deals.extend(closed_at(2, "XAUUSD", at("2026.03.10 15:00:00"), -40.0));
    deals.extend(closed_at(3, "GBPJPY", at("2026.03.11 09:00:00"), 20.0));
    store(db, deals);
}

/// Trades of the last three days: EURUSD +100, XAUUSD −40, GBPJPY +20.
fn seed_recent(db: &mut Db) {
    let mut deals = closed_at(1, "EURUSD", now_ms() - 3 * DAY_MS, 100.0);
    deals.extend(closed_at(2, "XAUUSD", now_ms() - 2 * DAY_MS, -40.0));
    deals.extend(closed_at(3, "GBPJPY", now_ms() - DAY_MS, 20.0));
    store(db, deals);
}

fn trade_button(symbol: &str) -> String {
    format!("role:button[Mở nhật ký lệnh {symbol}]")
}

fn shows_any(app: &Headless, needle: &str) -> bool {
    app.screen().texts().iter().any(|t| t.contains(needle))
}

#[test]
fn a_day_of_the_overviews_month_opens_the_calendar_on_that_day() {
    let fixture = Fixture::new();
    seed_march(&mut fixture.db());
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.wait_for_within("Tháng 3/2026", WAIT).expect("the small month shows March, the month of the trades");
    // The 10th is not the day the calendar opens on by itself (the 11th, the latest with trades).
    app.wait_for_within("role:button[Xem ngày 10/03 +60]", WAIT).expect("a day of the small month is a button");
    app.press("role:button[Xem ngày 10/03 +60]").expect("the day is pressed");
    app.wait_for_within("Tháng 3, 2026", WAIT).expect("the calendar opens on the day's month");
    app.wait_for_within("Thứ 3, 10/03", WAIT).expect("with the day pressed selected");
    app.wait_for_within(&trade_button("EURUSD"), WAIT).expect("its first trade is listed");
    app.wait_for_within(&trade_button("XAUUSD"), WAIT).expect("and its second");
    assert!(!shows_any(&app, "Thứ 4, 11/03"), "the latest day did not take the selection: {:?}", app.screen().texts());
    assert!(!shows_any(&app, "GBPJPY"), "the trade of the 11th is not in the 10th's list");
}

#[test]
fn a_symbol_of_the_overview_opens_the_trades_of_that_symbol() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.wait_for("Theo symbol").expect("the symbol card shows");
    app.wait_for_within("role:button[Xem lệnh EURUSD +$100.00]", WAIT).expect("a symbol's row is a button");
    app.press("role:button[Xem lệnh EURUSD +$100.00]").expect("the symbol is pressed");
    app.wait_for_within("Hiển thị 1 / 3 lệnh", WAIT).expect("the trades page lists that symbol's trade alone");
    app.wait_for(&trade_button("EURUSD")).expect("the symbol's trade is listed");
    assert!(app.screen().count(&trade_button("XAUUSD")) == Ok(0), "XAUUSD is filtered out");
    assert!(app.screen().count(&trade_button("GBPJPY")) == Ok(0), "GBPJPY is filtered out");
}

#[test]
fn the_calendars_month_title_picks_a_day_from_a_date_picker() {
    let fixture = Fixture::new();
    seed_march(&mut fixture.db());
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.press("role:button[Lịch P/L]").expect("the calendar row");
    app.wait_for_within("Thứ 4, 11/03", WAIT).expect("the calendar opens on the latest day with trades");
    // Another month is shown, so that the pick has to bring March back.
    app.press("role:button[Tháng sau]").expect("the next month button");
    app.wait_for("Tháng 4, 2026").expect("April is shown");
    app.press("label:Chọn ngày").expect("the month's title opens the date picker");
    app.wait_for("CN").expect("the picker shows its weekdays");
    app.press("role:button[2026-03-10]").expect("a day of the picker");
    app.wait_for_within("Tháng 3, 2026", WAIT).expect("the picked day's month is shown");
    app.wait_for_within("Thứ 3, 10/03", WAIT).expect("and the picked day is selected");
    app.wait_for_within(&trade_button("EURUSD"), WAIT).expect("with its trades");
}

/// The timeframe `label` as the catalog stores it: its key and whether it is archived.
fn stored_timeframe(fixture: &Fixture, label: &str) -> Vec<(String, bool)> {
    fixture
        .db()
        .catalog(CatalogKind::Timeframe)
        .expect("the timeframes read")
        .into_iter()
        .filter(|item| item.label.eq_ignore_ascii_case(label))
        .map(|item| (item.key, item.archived))
        .collect()
}

/// Whether the toggle `selector` names is on.
fn is_on(screen: &Screen, selector: &str) -> bool {
    screen.accessibility_state(selector).is_ok_and(|s| s.pressed == Some(nsh_core::Checked::True))
}

#[test]
fn the_playbook_form_adds_a_timeframe_and_removes_one_after_asking() {
    let fixture = Fixture::new();
    // Enter reaches the focused field only on a desktop.
    let mut app = fixture.start(1280.0, 1100.0, |config| config.environment.os = Os::Linux);
    app.press("role:button[Playbook]").expect("the playbook row");
    app.wait_for_within("Chưa có playbook nào", WAIT).expect("the empty list is read");
    app.press("role:button[Thêm playbook]").expect("the add button");
    app.wait_for_within("label:Thêm khung thời gian", WAIT).expect("the form has a field to add a timeframe");

    app.type_text("label:Thêm khung thời gian", "m30").expect("a timeframe is typed");
    app.press_key("enter").expect("Enter adds it");
    wait_until("M30 being stored", || stored_timeframe(&fixture, "M30").len() == 1);
    let [(key, archived)] = stored_timeframe(&fixture, "M30").try_into().expect("one M30");
    assert!(!archived, "the new timeframe is offered");
    let toggle = "role:button[Khung thời gian: M30]";
    app.wait_for_within(toggle, WAIT).expect("the new timeframe is an option, in capitals");
    app.wait_until("M30 being picked", WAIT, |screen| is_on(screen, toggle)).expect("the timeframe added is picked");
    assert_eq!(app.screen().input_text("label:Thêm khung thời gian"), Ok(Some("")), "the field is emptied");

    // The same name again, in another case, is the same timeframe.
    app.type_text("label:Thêm khung thời gian", "M30").expect("typed again");
    app.press_key("enter").expect("Enter");
    app.wait_until("the field emptied again", WAIT, |screen| {
        screen.input_text("label:Thêm khung thời gian") == Ok(Some(""))
    })
    .expect("the second one was taken");
    assert_eq!(stored_timeframe(&fixture, "M30"), [(key.clone(), false)], "no second M30");
    assert_eq!(app.screen().count(toggle), Ok(1), "one M30 option");
    assert!(is_on(app.screen(), toggle), "still picked");

    // ✕ asks, and the answer archives the timeframe and unpicks it.
    app.answer_dialogs_myself();
    app.press("role:button[Xoá Khung thời gian M30]").expect("the remove button of the option");
    let title = app.pending_dialog().and_then(|dialog| dialog.title.clone());
    assert!(title.as_deref().is_some_and(|t| t.contains("M30")), "the removal of M30 is asked first: {title:?}");
    assert_eq!(stored_timeframe(&fixture, "M30"), [(key.clone(), false)], "nothing is removed before the answer");
    app.answer_dialog(DialogAnswer::Press("Xoá".to_owned())).expect("answers");
    wait_until("M30 being archived", || stored_timeframe(&fixture, "M30") == [(key.clone(), true)]);
    app.wait_for_gone(toggle, WAIT).expect("the removed timeframe is no longer an option");

    // Typed once more, the archived one comes back rather than a second one.
    app.type_text("label:Thêm khung thời gian", "m30").expect("typed once more");
    app.press_key("enter").expect("Enter");
    wait_until("M30 being brought back", || stored_timeframe(&fixture, "M30") == [(key.clone(), false)]);
    app.wait_for_within(toggle, WAIT).expect("it is an option again");
}

#[test]
fn recording_a_missed_trade_gives_the_keyboard_to_its_symbol() {
    let fixture = Fixture::new();
    // Keys reach the focused view only on a desktop.
    let mut app = fixture.start(1280.0, 1000.0, |config| config.environment.os = Os::Linux);
    app.press("role:button[Lệnh bỏ lỡ]").expect("the missed trades row");
    app.wait_for_within("Chưa có lệnh bỏ lỡ nào", WAIT).expect("the empty list is read");
    app.press("role:button[Ghi lệnh bỏ lỡ]").expect("the header button");
    // The blank form is built again and its symbol input then takes the focus.
    app.wait_until("the symbol input taking the focus", WAIT, |screen| {
        screen.id("label:Symbol").is_ok_and(|id| {
            screen.commands().iter().any(|(to, command)| *to == id && matches!(command, Command::Focus))
        })
    })
    .expect("the symbol input is focused");
    for key in ["x", "a", "u"] {
        app.press_key(key).expect("a key is typed on the focused view");
    }
    app.wait_until("the keys reaching the symbol", WAIT, |screen| {
        screen.input_text("label:Symbol").is_ok_and(|held| held.is_some_and(|t| t.eq_ignore_ascii_case("xau")))
    })
    .expect("what is typed goes straight into the symbol");
}

#[test]
fn a_row_of_the_symbol_table_holds_its_name_in_the_middle_of_its_height() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.press("role:button[Phân tích]").expect("the analytics row");
    app.press("role:tab[Phân tích: Symbol]").expect("the symbol tab");
    app.wait_for_within("Theo symbol", WAIT).expect("the symbol breakdown shows");
    app.wait_for_within("EURUSD", WAIT).expect("a symbol has its row");
    let nodes = app.screen().layout_nodes();
    // The name of a table row: a text "EURUSD" whose parent is a row at least 40 points high.
    let (name, row) = nodes
        .iter()
        .filter(|node| node.text.as_ref().is_some_and(|t| t.text.text == "EURUSD"))
        .find_map(|name| {
            let row = nodes.iter().find(|node| node.id == name.parent)?;
            (row.bounds.size.height.0 >= 39.5).then_some((name, row))
        })
        .expect("a row of the table names EURUSD");
    let middle = |bounds: &nsh_core::Bounds<nsh_core::Points>| bounds.origin.y.0 + bounds.size.height.0 / 2.0;
    let (name_middle, row_middle) = (middle(&name.bounds), middle(&row.bounds));
    assert!(
        (name_middle - row_middle).abs() <= 1.0,
        "the name's middle is at {name_middle} and its row's at {row_middle} (row {:?}, name {:?})",
        row.bounds,
        name.bounds
    );
}

/// The options of `kind` labelled `label`: their key and whether they are archived.
fn stored_option(fixture: &Fixture, kind: CatalogKind, label: &str) -> Vec<(String, bool)> {
    fixture
        .db()
        .catalog(kind)
        .expect("the catalog reads")
        .into_iter()
        .filter(|item| item.label == label)
        .map(|item| (item.key, item.archived))
        .collect()
}

#[test]
fn a_tag_typed_in_a_trades_journal_joins_the_tags_and_is_ticked() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    // Enter reaches the focused field only on a desktop.
    let mut app = fixture.start(1280.0, 1100.0, |config| config.environment.os = Os::Linux);
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for_within(&trade_button("EURUSD"), WAIT).expect("the trades load");
    app.press(&trade_button("EURUSD")).expect("a trade opens");
    app.wait_for_within("label:Thêm tag", WAIT).expect("the journal has a field to add a tag");

    app.type_text("label:Thêm tag", "Phá đỉnh").expect("a tag is typed");
    app.press_key("enter").expect("Enter adds it");
    wait_until("the tag being stored", || stored_option(&fixture, CatalogKind::Tag, "Phá đỉnh").len() == 1);
    let [(_, archived)] = stored_option(&fixture, CatalogKind::Tag, "Phá đỉnh").try_into().expect("one tag");
    assert!(!archived, "the new tag is offered");
    let toggle = "role:button[Tag: Phá đỉnh]";
    app.wait_for_within(toggle, WAIT).expect("the new tag is an option");
    app.wait_until("the tag being ticked", WAIT, |screen| is_on(screen, toggle)).expect("the tag added is ticked");
}

#[test]
fn a_reason_of_a_missed_trade_is_removed_from_the_list_after_asking() {
    let fixture = Fixture::new();
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.press("role:button[Lệnh bỏ lỡ]").expect("the missed trades row");
    app.wait_for_within("Chưa có lệnh bỏ lỡ nào", WAIT).expect("the empty list is read");
    let toggle = "role:button[Lý do: Sợ sau lệnh thua]";
    app.wait_for_within(toggle, WAIT).expect("the reasons are read");
    assert_eq!(stored_option(&fixture, CatalogKind::MissedReason, "Sợ sau lệnh thua"), [("fear".to_owned(), false)]);

    app.answer_dialogs_myself();
    app.press("role:button[Xoá Lý do Sợ sau lệnh thua]").expect("the remove button of the reason");
    let title = app.pending_dialog().and_then(|dialog| dialog.title.clone());
    assert!(title.as_deref().is_some_and(|t| t.contains("Sợ sau lệnh thua")), "the removal is asked first: {title:?}");
    app.answer_dialog(DialogAnswer::Press("Xoá".to_owned())).expect("answers");
    wait_until("the reason being archived", || {
        stored_option(&fixture, CatalogKind::MissedReason, "Sợ sau lệnh thua") == [("fear".to_owned(), true)]
    });
    app.wait_for_gone(toggle, WAIT).expect("the removed reason is no longer an option");
}
