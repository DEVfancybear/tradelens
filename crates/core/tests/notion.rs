//! The Notion CSV import against the fixtures in `tests/fixtures/notion_*.csv`: what is
//! detected, what is read, what is reported, and that importing twice stores nothing twice.

use std::fs;
use std::path::PathBuf;

use tradelens_core::Db;
use tradelens_core::catalog::FieldDef;
use tradelens_core::catalog::{CatalogKind, FieldKind, FieldScope, FieldValue, Requirement};
use tradelens_core::form::FieldRef;
use tradelens_core::import::notion::{self, Field, NotionMapping, NotionPlan};
use tradelens_core::import::{self, ImportError};
use tradelens_core::model::{AccountInfo, Deal, DealBatch, DealEntry, DealKind, JournalEntry, TradeKey};
use tradelens_core::time::parse_mt5_time;
use tradelens_core::trades::build_trades;

const ENGLISH: &str = "notion_en.csv";
const VIETNAMESE: &str = "notion_vi.csv";
const SEMICOLON: &str = "notion_semicolon.csv";
const BAD_ROWS: &str = "notion_bad_rows.csv";
const EXTRAS: &str = "notion_extras.csv";

fn bytes(name: &str) -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name))
        .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn plan(name: &str) -> NotionPlan {
    notion::infer(&bytes(name)).unwrap_or_else(|e| panic!("{name} should be understood: {e}"))
}

fn time(at: &str) -> i64 {
    parse_mt5_time(at).expect("test times parse")
}

#[track_caller]
fn assert_close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-9, "{what}: got {got}, want {want}");
}

fn journal_of(db: &Db, account_id: i64, position_id: i64) -> JournalEntry {
    db.journal(&TradeKey { account_id, position_id, segment: 0 })
        .expect("journal reads")
        .unwrap_or_else(|| panic!("no journal on {position_id}"))
}

/// Net result of the account's trades as the app computes it.
fn stored_net(db: &Db, account_id: i64) -> f64 {
    let deals: Vec<Deal> = db.deals(Some(account_id)).expect("deals read").into_iter().map(|(_, d)| d).collect();
    build_trades(account_id, &deals).iter().map(tradelens_core::Trade::net).sum()
}

// --- detection ---------------------------------------------------------------------

#[test]
fn an_english_export_is_detected_without_any_mapping() {
    let plan = plan(ENGLISH);
    let column = |field| plan.mapping.column(field);
    assert_eq!(column(Field::OpenTime), Some("Date"));
    assert_eq!(column(Field::Symbol), Some("Pair"));
    assert_eq!(column(Field::Side), Some("Direction"));
    assert_eq!(column(Field::Volume), Some("Lot"));
    assert_eq!(column(Field::OpenPrice), Some("Entry"));
    assert_eq!(column(Field::ClosePrice), Some("Exit"));
    assert_eq!(column(Field::StopLoss), Some("SL"));
    assert_eq!(column(Field::TakeProfit), Some("TP"));
    assert_eq!(column(Field::Profit), Some("P&L"));
    assert_eq!(column(Field::Setup), Some("Setup"));
    assert_eq!(column(Field::Tags), Some("Tags"));
    assert_eq!(column(Field::Mistakes), Some("Mistakes"));
    assert_eq!(column(Field::Emotion), Some("Emotion"));
    assert_eq!(column(Field::Rating), Some("Rating"));
    assert_eq!(column(Field::Notes), Some("Notes"));
    assert_eq!(column(Field::Screenshot), Some("Screenshot"));
    assert_eq!(column(Field::CloseTime), None, "the close comes from the date range");
    assert_eq!(plan.title_column.as_deref(), Some("Name"));
    assert_eq!(plan.ignored_columns, ["Created time"], "Notion's own column is not a property");
    assert!(plan.properties.is_empty(), "every other column was a trade field");
    assert!(plan.issues.is_empty() && plan.warnings.is_empty(), "{:?} {:?}", plan.issues, plan.warnings);
    assert_eq!((plan.rows.len(), plan.total_rows), (5, 5));
}

#[test]
fn a_vietnamese_export_is_detected_by_its_headers() {
    let plan = plan(VIETNAMESE);
    let column = |field| plan.mapping.column(field);
    assert_eq!(column(Field::OpenTime), Some("Ngày vào"));
    assert_eq!(column(Field::Symbol), Some("Cặp"));
    assert_eq!(column(Field::Side), Some("Chiều"));
    assert_eq!(column(Field::Volume), Some("Khối lượng"));
    assert_eq!(column(Field::Profit), Some("Lãi/lỗ"));
    assert_eq!(column(Field::Setup), Some("Chiến lược"));
    assert_eq!(column(Field::Emotion), Some("Cảm xúc"));
    assert_eq!(column(Field::Mistakes), Some("Lỗi"));
    assert_eq!(column(Field::Notes), Some("Ghi chú"));
    assert_eq!(plan.title_column.as_deref(), Some("Tên"));
}

#[test]
fn the_required_columns_are_found_by_contents_when_the_names_say_nothing() {
    let csv = "Trade,When,Asset,Dir,Net\n\
               a,2026-11-20 09:00,EURUSD,Long,10\n\
               b,2026-11-21 10:00,XAUUSD,Short,-5\n";
    let plan = notion::infer(csv.as_bytes()).expect("detected by contents");
    assert_eq!(plan.mapping.column(Field::OpenTime), Some("When"));
    assert_eq!(plan.mapping.column(Field::Symbol), Some("Asset"));
    assert_eq!(plan.mapping.column(Field::Side), Some("Dir"));
    // "Net" is a profit by its name.
    assert_eq!(plan.mapping.column(Field::Profit), Some("Net"));
    assert_eq!(plan.rows.len(), 2);
}

#[test]
fn a_file_with_no_trade_columns_says_which_are_missing() {
    let err = notion::infer(b"Name,Mood\nx,calm\ny,fear\n").expect_err("not a trade journal");
    let message = err.to_string();
    assert!(
        message.contains("Thời gian vào") && message.contains("Symbol") && message.contains("Name, Mood"),
        "{message}"
    );
    assert!(matches!(err, ImportError::Invalid(_)));
    assert!(notion::infer(b"").is_err());
    assert!(notion::infer(b"Date,Pair,Direction\n").is_err());
}

#[test]
fn an_override_replaces_the_detection_of_one_field() {
    let mapping =
        NotionMapping { columns: vec![(Field::Notes, "Ghi chú thêm".into())], account_login: String::new() };
    let plan = notion::infer_with(&bytes(EXTRAS), &mapping).expect("reads");
    assert_eq!(plan.mapping.column(Field::Notes), Some("Ghi chú thêm"));
    assert!(plan.properties.iter().all(|p| p.label != "Ghi chú thêm"));
    assert_eq!(plan.rows[1].journal.notes, "Dời SL vì sợ");
    let missing = NotionMapping { columns: vec![(Field::Notes, "Nope".into())], account_login: String::new() };
    assert!(notion::infer_with(&bytes(EXTRAS), &missing).is_err());
}

#[test]
fn a_csv_is_not_read_as_an_mt5_report() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(ENGLISH);
    assert_eq!(import::Format::of(&path), Some(import::Format::NotionCsv));
    let err = import::read_file(&path).expect_err("a CSV carries a journal a batch cannot");
    assert!(err.to_string().contains("import::notion::infer"));
    assert_eq!(notion::read_file(&path).expect("reads").rows.len(), 5);
}

// --- reading -----------------------------------------------------------------------

#[test]
fn every_field_of_the_english_export_is_read() {
    let plan = plan(ENGLISH);
    let [breakout, fade, swing, scalp, yen] = plan.rows.as_slice() else { panic!("five rows") };

    // A date range gives the close; money has symbols and separators.
    assert_eq!((breakout.line, breakout.title.as_str()), (2, "EURUSD breakout"));
    assert_eq!(breakout.entry.kind, DealKind::Buy);
    assert_eq!(breakout.exit.kind, DealKind::Sell);
    assert_eq!((breakout.entry.entry, breakout.exit.entry), (DealEntry::In, DealEntry::Out));
    assert_eq!(breakout.entry.time_msc, time("2026.11.16 09:00:00"));
    assert_eq!(breakout.exit.time_msc, time("2026.11.16 15:10:00"));
    assert_close(breakout.entry.volume, 0.2, "volume");
    assert_close(breakout.entry.price, 1.0850, "entry price");
    assert_close(breakout.exit.price, 1.0950, "exit price");
    assert_close(breakout.entry.sl, 1.0800, "stop");
    assert_close(breakout.entry.tp, 1.0950, "target");
    assert_close(breakout.exit.profit, 150.0, "profit");
    assert!(breakout.entry.ticket < 0 && breakout.exit.ticket < 0);
    assert_eq!(breakout.entry.ticket + 1, breakout.exit.ticket, "the entry sorts first");
    assert_eq!(breakout.position_id, breakout.entry.ticket);
    assert_eq!(breakout.entry.position_id, breakout.exit.position_id);

    assert_eq!(fade.entry.kind, DealKind::Sell, "Short opens with a sell");
    assert_eq!(fade.exit.time_msc, fade.entry.time_msc, "no range, no close time: the open time");
    assert_close(fade.exit.profit, -100.0, "-$100.00");
    assert_eq!(swing.exit.time_msc, time("2026.11.19 16:30:00"));
    assert_close(swing.exit.profit, 1200.0, "$1,200.00");
    assert_eq!(scalp.entry.kind, DealKind::Sell, "Sell");
    assert_close(scalp.exit.profit, -30.0, "(30.00)");
    assert_close(scalp.entry.sl, 0.0, "no stop");
    assert_eq!(yen.entry.time_msc, time("2026.11.20 10:00:00"), "ISO dates mix with Notion's");
    assert_close(yen.exit.profit, -45.5, "a true minus sign");
    assert_close(plan.net_total(), 150.0 - 100.0 + 1200.0 - 30.0 - 45.5, "sum of the P&L column");
    assert_eq!(plan.period(), Some((time("2026.11.16 09:00:00"), time("2026.11.20 10:00:00"))));
    assert_eq!(plan.batch().deals.len(), 10);

    // The journal.
    let j = &breakout.journal;
    assert_eq!(j.setup, "Breakout");
    assert_eq!(j.tags, ["London open", "Theo trend H4"], "a multi-select is a list");
    assert_eq!(j.mistakes, Vec::<String>::new());
    assert_eq!(j.emotion_before, "Calm");
    assert_eq!(j.rating, 4);
    assert_eq!(j.notes, "Held to the target.");
    assert_eq!(j.screenshots, ["https://example.com/c1.png"], "a file cell gives its link");
    assert_eq!(fade.journal.mistakes, ["Early entry", "Revenge"]);
    assert_eq!(fade.journal.notes, "Stopped out, \"again\".", "quotes and commas survive");
    assert_eq!(swing.journal.rating, 3, "stars count");
    assert_eq!(yen.journal.rating, 0);
}

#[test]
fn a_vietnamese_export_reads_day_first_dates_and_mua_ban() {
    let plan = plan(VIETNAMESE);
    let [gold, euro, cable] = plan.rows.as_slice() else { panic!("three rows") };
    assert_eq!(gold.entry.time_msc, time("2026.11.20 09:30:00"), "20/11 can only be day first");
    assert_eq!(cable.entry.time_msc, time("2026.11.21 08:00:00"));
    assert_eq!(gold.entry.kind, DealKind::Buy, "Mua");
    assert_eq!(euro.entry.kind, DealKind::Sell, "Bán");
    assert_close(gold.exit.profit, 1250.0, "\"1,250.00\"");
    assert_close(cable.exit.profit, 200.0, "a bare integer");
    assert_eq!(euro.journal.mistakes, ["Vào sớm", "Dời SL"]);
    assert_eq!(euro.journal.emotion_before, "Lo lắng");
    assert_eq!(plan.account.login, "notion", "the default account, no login to pick");
    assert_close(plan.net_total(), 1390.0, "sum");
}

#[test]
fn a_semicolon_export_reads_decimal_commas_and_multiline_notes() {
    let plan = plan(SEMICOLON);
    let [a, b, c] = plan.rows.as_slice() else { panic!("three rows") };
    assert_eq!((a.line, b.line, c.line), (2, 4, 5), "a quoted newline takes two lines");
    assert_eq!(a.entry.time_msc, time("2026.11.20 14:05:00"));
    assert_close(a.entry.volume, 0.10, "0,10");
    assert_close(a.entry.price, 1.085, "1,08500");
    assert_close(a.exit.price, 1.09, "1,09000");
    assert_close(a.exit.profit, 1250.5, "\"1.250,50\"");
    assert_close(a.exit.commission, -0.70, "\"-0,70\"");
    assert_eq!(a.journal.notes, "Line one\nLine two; with a semicolon and \"quotes\"");
    assert_eq!(b.journal.notes, "Plain note");
    assert_eq!(b.entry.kind, DealKind::Sell);
    assert_close(c.entry.price, 150.25, "150,250 with a decimal comma");
    assert_close(c.exit.profit, -15.0, "-15,00");
    assert_close(plan.net_total(), 1250.5 - 0.7 - 25.0 - 15.0, "profit and commission");
}

#[test]
fn rows_that_cannot_be_read_are_reported_with_their_line() {
    let plan = plan(BAD_ROWS);
    assert_eq!(plan.total_rows, 8);
    assert_eq!(plan.rows.iter().map(|r| r.line).collect::<Vec<_>>(), [2, 4, 10], "good rows keep their lines");
    let lines: Vec<usize> = plan.issues.iter().map(|i| i.line).collect();
    assert_eq!(lines, [3, 6, 7, 8, 9], "the quoted newline of line 4 shifts nothing after it");
    let reason = |line: usize| plan.issues.iter().find(|i| i.line == line).map_or_default(|i| i.reason.as_str());
    assert!(reason(3).contains("thời gian vào") && reason(3).contains("not a date"), "{}", reason(3));
    assert!(reason(6).contains("chiều lệnh") && reason(6).contains("Hold"), "{}", reason(6));
    assert!(reason(7).contains("symbol"), "{}", reason(7));
    assert!(reason(8).contains("Khối lượng") && reason(8).contains("abc"), "{}", reason(8));
    assert!(reason(9).contains("Lãi/lỗ"), "{}", reason(9));
    assert_eq!(plan.issues[0].to_string(), format!("dòng 3: {}", reason(3)));
    assert_close(plan.net_total(), 125.0, "only the good rows");
}

#[test]
fn a_file_where_nothing_parses_is_an_error_that_says_why() {
    let csv = "Date,Pair,Direction,Mood\n2026-11-20 09:00,EURUSD,Long,calm\n2026-11-21 09:00,EURUSD,Long,fear\n";
    // Forced onto a text column, the profit cannot be read in any row.
    let mapping = NotionMapping { columns: vec![(Field::Profit, "Mood".into())], account_login: String::new() };
    let err = notion::infer_with(csv.as_bytes(), &mapping).expect_err("nothing to import").to_string();
    assert!(err.contains("không đọc được dòng nào") && err.contains("dòng 2"), "{err}");
}

#[test]
fn what_the_file_lacks_is_a_warning_not_an_error() {
    let csv = "Date,Pair,Direction\n2026-11-20 09:00,EURUSD,Long\n";
    let plan = notion::infer(csv.as_bytes()).expect("a bare list of trades");
    assert_eq!(plan.warnings.len(), 4, "{:?}", plan.warnings);
    assert!(plan.warnings.iter().any(|w| w.contains("Khối lượng") && w.contains("1 lot")));
    assert_close(plan.rows[0].entry.volume, 1.0, "assumed lot");
}

#[test]
fn the_same_file_gives_the_same_tickets_and_equal_rows_differ() {
    let first = plan(ENGLISH);
    let second = plan(ENGLISH);
    let ids = |p: &NotionPlan| p.rows.iter().map(|r| r.position_id).collect::<Vec<_>>();
    assert_eq!(ids(&first), ids(&second));
    let mut distinct = ids(&first);
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 5);

    let twins = "Name,Date,Pair,Direction,P&L\nx,2026-11-20 09:00,EURUSD,Long,5\nx,2026-11-20 09:00,EURUSD,Long,5\n";
    let plan = notion::infer(twins.as_bytes()).expect("reads");
    assert_eq!(plan.rows.len(), 2);
    assert_ne!(plan.rows[0].position_id, plan.rows[1].position_id, "two identical rows are two trades");
    assert_eq!(plan.rows[0].position_id, notion::infer(twins.as_bytes()).expect("reads").rows[0].position_id);
}

// --- properties --------------------------------------------------------------------

fn kinds(plan: &NotionPlan) -> Vec<(&str, FieldKind)> {
    plan.properties.iter().map(|p| (p.label.as_str(), p.kind)).collect()
}

#[test]
fn unknown_columns_become_properties_of_the_kind_their_values_show() {
    let plan = plan(EXTRAS);
    assert_eq!(
        kinds(&plan),
        [
            ("Tâm lý", FieldKind::Select),
            ("Khung thời gian", FieldKind::Select),
            ("Confluence", FieldKind::MultiSelect),
            ("Đã xem lại", FieldKind::Checkbox),
            ("Link TradingView", FieldKind::Url),
            ("Điểm", FieldKind::Number),
            ("Ngày xem lại", FieldKind::Date),
            ("Ghi chú thêm", FieldKind::Text),
        ]
    );
    let options = |label: &str| plan.properties.iter().find(|p| p.label == label).map(|p| p.options.clone());
    assert_eq!(options("Tâm lý"), Some(vec!["Bình tĩnh".to_owned(), "Lo lắng".to_owned()]));
    assert_eq!(options("Khung thời gian"), Some(vec!["M15".to_owned(), "H1".to_owned()]));
    assert_eq!(
        options("Confluence"),
        Some(vec!["Order block".to_owned(), "Fair value gap".to_owned(), "Liquidity sweep".to_owned()])
    );
    assert_eq!(options("Điểm"), Some(Vec::new()));
    let filled = |label: &str| plan.properties.iter().find(|p| p.label == label).map(|p| p.filled);
    assert_eq!(filled("Link TradingView"), Some(4), "one trade has no link");
    assert_eq!(filled("Ngày xem lại"), Some(4));
    assert_eq!(plan.ignored_columns, ["Trống", "Last edited time"], "an empty column and a Notion one");
    assert_eq!(plan.properties[0].key, "tam_ly");
    assert_eq!(plan.rows[0].journal.custom["tam_ly"], FieldValue::Choice("Bình tĩnh".into()));
}

#[test]
fn properties_are_stored_with_their_values_and_added_to_the_form() {
    let mut db = Db::in_memory().expect("db");
    let report = db.import_notion(&plan(EXTRAS)).expect("imports");
    assert_eq!((report.created, report.updated, report.unchanged, report.matched_mt5), (5, 0, 0, 0));
    assert_eq!(report.properties_reused, Vec::<String>::new());
    assert_eq!(report.properties_created.len(), 8);
    assert_eq!(
        (report.properties_created[2].label.as_str(), report.properties_created[2].kind),
        ("Confluence", FieldKind::MultiSelect)
    );

    let defs = db.field_defs(FieldScope::Trade).expect("defs");
    assert_eq!(defs.len(), 8);
    let def = |key: &str| defs.iter().find(|d| d.key == key).unwrap_or_else(|| panic!("no field {key}"));
    assert_eq!(def("tam_ly").kind, FieldKind::Select);
    assert_eq!(def("tam_ly").options, ["Bình tĩnh", "Lo lắng"]);
    assert_eq!(def("khung_thoi_gian").options, ["M15", "H1"]);
    assert_eq!(def("confluence").kind, FieldKind::MultiSelect);
    assert_eq!(def("da_xem_lai").kind, FieldKind::Checkbox);
    assert_eq!(def("link_tradingview").kind, FieldKind::Url);
    assert_eq!(def("diem").kind, FieldKind::Number);
    assert_eq!(def("ngay_xem_lai").kind, FieldKind::Date);
    assert!(defs.iter().all(|d| d.scope == FieldScope::Trade && !d.archived && d.required == Requirement::Optional));
    let positions: Vec<i32> = defs.iter().map(|d| d.position).collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "in the file's column order: {positions:?}");

    // The trade form shows them after its own properties.
    let layout = db.form_layout(FieldScope::Trade).expect("layout");
    let shown: Vec<FieldRef> = layout.properties(&defs).into_iter().filter(|p| !p.hidden).map(|p| p.field).collect();
    for key in ["tam_ly", "confluence", "diem"] {
        assert!(shown.contains(&FieldRef::Custom(key.into())), "{key} is in the form");
    }
    assert_eq!(
        layout.items.last().map(|i| i.field.clone()),
        Some(FieldRef::Custom("ghi_chu_them".into())),
        "appended in column order"
    );

    // The values reach each trade's journal, and every value fits its field.
    let plan = plan(EXTRAS);
    let first = journal_of(&db, report.account_id, plan.rows[0].position_id);
    assert_eq!(first.custom["tam_ly"], FieldValue::Choice("Bình tĩnh".into()));
    assert_eq!(first.custom["khung_thoi_gian"], FieldValue::Choice("M15".into()));
    assert_eq!(first.custom["confluence"], FieldValue::Choices(vec!["Order block".into(), "Fair value gap".into()]));
    assert_eq!(first.custom["da_xem_lai"], FieldValue::Flag(true));
    assert_eq!(first.custom["link_tradingview"], FieldValue::Text("https://www.tradingview.com/x/aaa/".into()));
    assert_eq!(first.custom["diem"], FieldValue::Number(8.0));
    assert_eq!(first.custom["ngay_xem_lai"], FieldValue::Date("2026-11-17".into()));
    assert_eq!(first.custom["ghi_chu_them"], FieldValue::Text("Vào theo plan, chờ nến đóng".into()));
    let second = journal_of(&db, report.account_id, plan.rows[1].position_id);
    assert_eq!(second.custom["da_xem_lai"], FieldValue::Flag(false), "an unticked box is a value too");
    assert_eq!(second.custom["diem"], FieldValue::Number(6.5));
    let last = journal_of(&db, report.account_id, plan.rows[4].position_id);
    assert!(!last.custom.contains_key("link_tradingview") && !last.custom.contains_key("ngay_xem_lai"));
    for (_, journal) in db.journals(Some(report.account_id)).expect("journals") {
        for (key, value) in &journal.custom {
            assert_eq!(def(key).validate(value), Ok(()), "{key}");
        }
    }
}

#[test]
fn a_property_with_the_same_name_takes_the_values_instead_of_a_duplicate() {
    let mut db = Db::in_memory().expect("db");
    // The trader already has "Tam ly" (no diacritics, another key), with one choice.
    let existing = FieldDef {
        id: 0,
        scope: FieldScope::Trade,
        key: "mood".into(),
        label: "Tam ly".into(),
        kind: FieldKind::Select,
        options: vec!["Bình tĩnh".into()],
        max: None,
        required: Requirement::Expected,
        analysed: true,
        position: 0,
        archived: false,
    };
    db.save_field_def(&existing).expect("saved");

    let report = db.import_notion(&plan(EXTRAS)).expect("imports");
    assert_eq!(report.properties_reused, ["Tam ly"]);
    assert_eq!(report.properties_created.len(), 7);
    let defs = db.field_defs(FieldScope::Trade).expect("defs");
    assert_eq!(defs.len(), 8, "no duplicate");
    let mood = defs.iter().find(|d| d.key == "mood").expect("kept");
    assert_eq!(mood.options, ["Bình tĩnh", "Lo lắng"], "the file's choices are added");
    assert_eq!((mood.required, mood.analysed), (Requirement::Expected, true), "its settings are kept");
    let plan = plan(EXTRAS);
    let journal = journal_of(&db, report.account_id, plan.rows[1].position_id);
    assert_eq!(journal.custom["mood"], FieldValue::Choice("Lo lắng".into()));
    assert!(!journal.custom.contains_key("tam_ly"));

    // A key taken by another property is not reused for a new one.
    let mut db = Db::in_memory().expect("db");
    db.save_field_def(&FieldDef { key: "diem".into(), label: "Something else".into(), ..existing }).expect("saved");
    let report = db.import_notion(&plan).expect("imports");
    assert_eq!(report.properties_created.len(), 8);
    let defs = db.field_defs(FieldScope::Trade).expect("defs");
    assert!(
        defs.iter().any(|d| d.key == "diem_2" && d.label == "Điểm"),
        "{:?}",
        defs.iter().map(|d| &d.key).collect::<Vec<_>>()
    );
}

// --- storing -----------------------------------------------------------------------

#[test]
fn trades_and_journals_are_stored_and_the_totals_match_the_file() {
    let mut db = Db::in_memory().expect("db");
    let plan = plan(ENGLISH);
    let report = db.import_notion(&plan).expect("imports");
    assert_eq!((report.created, report.updated, report.unchanged), (5, 0, 0));
    assert!(report.skipped.is_empty() && report.properties_created.is_empty());
    assert_eq!(report.options_added, 0, "every tag, mistake and emotion was in the lists by key or label");

    let account = db.accounts().expect("accounts").pop().expect("the Notion account");
    assert_eq!(
        (account.id, account.login.as_str(), account.name.as_str()),
        (report.account_id, "notion", "Nhật ký Notion")
    );
    let deals = db.deals(Some(account.id)).expect("deals");
    assert_eq!(deals.len(), 10);
    assert!(deals.iter().all(|(_, d)| d.ticket < 0));
    let trades = build_trades(account.id, &deals.iter().map(|(_, d)| d.clone()).collect::<Vec<_>>());
    assert_eq!(trades.len(), 5);
    assert!(trades.iter().all(|t| t.manual && t.is_closed()));
    assert_close(stored_net(&db, account.id), 1174.5, "the P&L column's sum");

    // Names become the keys of the lists.
    let breakout = journal_of(&db, account.id, plan.rows[0].position_id);
    assert_eq!(breakout.tags, ["london_open", "trend_h4"]);
    assert_eq!(breakout.emotion_before, "calm");
    assert_eq!((breakout.setup.as_str(), breakout.rating), ("Breakout", 4));
    assert_eq!(breakout.notes, "Held to the target.\n\nẢnh: https://example.com/c1.png");
    let fade = journal_of(&db, account.id, plan.rows[1].position_id);
    assert_eq!(fade.tags, ["news", "london_open"]);
    assert_eq!(fade.mistakes, ["early_entry", "revenge"]);
    assert_eq!(fade.emotion_before, "fear");
    assert!(
        db.journal(&TradeKey { account_id: account.id, position_id: plan.rows[4].position_id, segment: 0 })
            .expect("reads")
            .is_some()
    );
}

#[test]
fn names_missing_from_the_option_lists_are_added_to_them() {
    let mut db = Db::in_memory().expect("db");
    let plan = plan(VIETNAMESE);
    let report = db.import_notion(&plan).expect("imports");
    assert_eq!(report.options_added, 1, "only \"Lo lắng\" was new");
    let emotions = db.catalog(CatalogKind::Emotion).expect("catalog");
    let worried = emotions.iter().find(|e| e.label == "Lo lắng").expect("added");
    assert_eq!(worried.key, "lo_lang");
    let euro = journal_of(&db, report.account_id, plan.rows[1].position_id);
    assert_eq!(euro.emotion_before, "lo_lang");
    assert_eq!(euro.mistakes, ["early_entry", "moved_sl"], "matched by label");
    assert_close(stored_net(&db, report.account_id), 1390.0, "sum");
}

#[test]
fn importing_the_same_file_again_changes_nothing() {
    for name in [ENGLISH, EXTRAS, SEMICOLON] {
        let mut db = Db::in_memory().expect("db");
        let plan = plan(name);
        let first = db.import_notion(&plan).expect("first import");
        let deals = db.deals(None).expect("deals");
        let journals = db.journals(None).expect("journals");
        let defs = db.field_defs(FieldScope::Trade).expect("defs");
        let layout = db.form_layout(FieldScope::Trade).expect("layout");
        let options = db.catalog(CatalogKind::Emotion).expect("catalog");

        let again = db.import_notion(&notion::infer(&bytes(name)).expect("reads")).expect("second import");
        assert_eq!((again.created, again.updated), (0, 0), "{name}");
        assert_eq!(again.unchanged, plan.rows.len(), "{name}");
        assert!(again.properties_created.is_empty(), "{name}: no duplicate fields");
        assert_eq!(again.properties_reused.len(), first.properties_created.len(), "{name}");
        assert_eq!(again.options_added, 0, "{name}");
        assert_eq!(again.account_id, first.account_id);
        assert_eq!(db.deals(None).expect("deals"), deals, "{name}: same tickets, same deals");
        assert_eq!(db.journals(None).expect("journals"), journals, "{name}");
        assert_eq!(db.field_defs(FieldScope::Trade).expect("defs"), defs, "{name}");
        assert_eq!(db.form_layout(FieldScope::Trade).expect("layout"), layout, "{name}");
        assert_eq!(db.catalog(CatalogKind::Emotion).expect("catalog"), options, "{name}");
    }
}

#[test]
fn a_changed_row_is_updated_in_place() {
    let mut db = Db::in_memory().expect("db");
    db.import_notion(&plan(ENGLISH)).expect("imports");
    let edited = String::from_utf8(bytes(ENGLISH)).expect("utf-8").replace("-$100.00", "-$110.00");
    let report = db.import_notion(&notion::infer(edited.as_bytes()).expect("reads")).expect("imports");
    assert_eq!((report.created, report.updated, report.unchanged), (0, 1, 4));
    assert_close(stored_net(&db, report.account_id), 1164.5, "the edited P&L");
    assert_eq!(db.deals(None).expect("deals").len(), 10);
}

#[test]
fn what_the_trader_wrote_since_is_kept() {
    let mut db = Db::in_memory().expect("db");
    let plan = plan(ENGLISH);
    let report = db.import_notion(&plan).expect("imports");
    let key = TradeKey { account_id: report.account_id, position_id: plan.rows[0].position_id, segment: 0 };
    let mut mine = journal_of(&db, report.account_id, plan.rows[0].position_id);
    mine.notes = "My own review.".into();
    mine.tags.clear();
    mine.rating = 5;
    db.save_journal(&key, &mine).expect("saved");

    let again = db.import_notion(&plan).expect("imports again");
    assert_eq!((again.created, again.updated), (0, 1), "the empty tags were filled again");
    let after = journal_of(&db, report.account_id, plan.rows[0].position_id);
    assert_eq!((after.notes.as_str(), after.rating), ("My own review.", 5), "written fields are not overwritten");
    assert_eq!(after.tags, ["london_open", "trend_h4"], "empty ones are filled");
}

#[test]
fn bad_rows_are_skipped_and_reported_by_the_import() {
    let mut db = Db::in_memory().expect("db");
    let report = db.import_notion(&plan(BAD_ROWS)).expect("imports");
    assert_eq!(report.created, 3);
    assert_eq!(report.skipped.iter().map(|i| i.line).collect::<Vec<_>>(), [3, 6, 7, 8, 9]);
    assert_close(stored_net(&db, report.account_id), 125.0, "the good rows");
}

#[test]
fn a_trade_already_in_mt5_only_gets_the_journal() {
    let mut db = Db::in_memory().expect("db");
    // The same EURUSD long, opened a minute later by the broker's clock.
    let deal = |ticket: i64, kind: DealKind, entry: DealEntry, at: &str, price: f64| Deal {
        ticket,
        order: ticket,
        position_id: 7001,
        time_msc: time(at),
        symbol: "EURUSD".into(),
        kind,
        entry,
        volume: 0.2,
        price,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: if entry == DealEntry::Out { 150.0 } else { 0.0 },
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        reason: None,
    };
    let batch = DealBatch {
        account: AccountInfo { login: "52012345".into(), server: "Demo".into(), ..AccountInfo::default() },
        deals: vec![
            deal(5001, DealKind::Buy, DealEntry::In, "2026.11.16 09:01:00", 1.0850),
            deal(5002, DealKind::Sell, DealEntry::Out, "2026.11.16 15:10:00", 1.0950),
        ],
        positions: Vec::new(),
    };
    let mt5 = db.upsert_batch(&batch, "html").expect("MT5 deals");
    let account = db.accounts().expect("accounts").pop().expect("account");
    let mut plan = plan(ENGLISH);
    plan.use_account(&account);

    let report = db.import_notion(&plan).expect("imports");
    assert_eq!((report.created, report.matched_mt5), (4, 1));
    assert_eq!(report.account_id, mt5.account_id);
    assert_eq!(db.deals(Some(account.id)).expect("deals").len(), 2 + 4 * 2, "the matched row stored no deals");
    let journal = journal_of(&db, account.id, 7001);
    assert_eq!(journal.setup, "Breakout");
    assert_eq!(journal.tags, ["london_open", "trend_h4"]);

    let again = db.import_notion(&plan).expect("imports again");
    assert_eq!((again.created, again.updated, again.unchanged, again.matched_mt5), (0, 0, 5, 1));
}

#[test]
fn a_known_playbook_setup_is_linked() {
    use tradelens_core::playbook::Playbook;
    let mut db = Db::in_memory().expect("db");
    let id = db.save_playbook(&Playbook { name: "breakout".into(), ..Playbook::default() }, 0).expect("saved");
    let plan = plan(ENGLISH);
    let report = db.import_notion(&plan).expect("imports");
    let journal = journal_of(&db, report.account_id, plan.rows[0].position_id);
    assert_eq!((journal.playbook_id, journal.setup.as_str()), (Some(id), "Breakout"));
    assert_eq!(journal_of(&db, report.account_id, plan.rows[1].position_id).playbook_id, None);
}

// --- a table in memory (the Notion API) --------------------------------------------

fn table_row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_owned()).collect()
}

#[test]
fn a_table_in_memory_is_read_like_a_csv() {
    let headers = table_row(&["Name", "Date", "Pair", "Direction", "Lot", "Entry", "Exit", "P&L", "Setup", "Tags"]);
    let rows = vec![
        table_row(&[
            "EURUSD breakout",
            "2026-11-16 09:00 → 2026-11-16 15:10",
            "EURUSD",
            "Long",
            "0.2",
            "1.085",
            "1.095",
            "150",
            "Breakout",
            "London open, Theo trend H4",
        ]),
        table_row(&["Gold fade", "", "XAUUSD", "Short", "0.1", "2650.25", "2660.25", "-100", "Fade", ""]),
        table_row(&[
            "Gold swing",
            "2026-11-18 10:00",
            "XAUUSD",
            "Long",
            "0.3",
            "2640",
            "2644",
            "1,200",
            "Breakout",
            "A+ setup",
        ]),
    ];
    let plan = notion::infer_table(&headers, &rows).expect("a table is understood like a file");

    assert_eq!(plan.mapping.column(Field::OpenTime), Some("Date"));
    assert_eq!(plan.mapping.column(Field::Profit), Some("P&L"));
    assert_eq!(plan.total_rows, 3);
    assert_eq!(plan.rows.len(), 2, "the row without a date cannot be imported");
    assert_eq!(
        (plan.issues[0].line, plan.issues.len()),
        (3, 1),
        "line 1 is the header, the bad row is the second data row"
    );
    assert_close(plan.rows[0].net(), 150.0, "first trade");
    assert_close(plan.rows[1].net(), 1200.0, "a thousands comma in a table is a thousands comma");
    assert_eq!(plan.rows[0].journal.tags, ["London open", "Theo trend H4"]);
    assert_eq!(plan.rows[0].entry.time_msc, time("2026.11.16 09:00:00"));
}

#[test]
fn a_table_without_rows_or_columns_is_refused() {
    assert!(matches!(notion::infer_table(&[], &[]), Err(ImportError::Invalid(_))));
    let headers = table_row(&["Date", "Pair", "Direction"]);
    assert!(matches!(notion::infer_table(&headers, &[]), Err(ImportError::Invalid(_))));
    assert!(matches!(notion::infer_table(&headers, &[table_row(&["", "", ""])]), Err(ImportError::Invalid(_))));
}
