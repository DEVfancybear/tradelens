//! The share dialog (a trade, a week, a month), the monthly PDF and the AI placeholder on the
//! review screen, headless on the real engine over a database in a temporary data directory.
//! Files are saved to a path the test answers the save dialog with; the clipboard image cannot be
//! read back headless, so copying is checked by the line it reports.

use std::{path::Path, time::Duration};

use nsh_testing::Headless;
use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, model::AccountInfo};

mod support;
use support::{WAIT, wait_until};

/// The shared fixture with this file's window.
struct Fixture(support::Fixture);

impl std::ops::Deref for Fixture {
    type Target = support::Fixture;

    fn deref(&self) -> &support::Fixture {
        &self.0
    }
}

impl Fixture {
    fn new() -> Self {
        Self(support::Fixture::new())
    }

    fn start(&self) -> Headless {
        self.0.start(1400.0, 1000.0, |_| {})
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
    symbol: &str,
    at_ms: i64,
    kind: DealKind,
    entry: DealEntry,
    price: f64,
    profit: f64,
) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: symbol.into(),
        kind,
        entry,
        volume: 0.5,
        price,
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
        reason: None,
    }
}

/// Two trades that closed just now: EURUSD +100 and XAUUSD −40.
fn seed(db: &mut Db) {
    let exit = now_ms();
    let entry = exit - 600_000;
    let mut deals = Vec::new();
    for (position, symbol, profit) in [(1, "EURUSD", 100.0), (2, "XAUUSD", -40.0)] {
        deals.push(deal(position * 10, position, symbol, entry, DealKind::Buy, DealEntry::In, 1.1, 0.0));
        deals.push(deal(position * 10 + 1, position, symbol, exit, DealKind::Sell, DealEntry::Out, 1.1, profit));
    }
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

fn open_trade(app: &mut Headless, symbol: &str) {
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.press(&format!("role:button[Mở nhật ký lệnh {symbol}]")).expect("the trade opens");
    app.wait_for("Nhật ký lệnh").expect("the trade's page shows");
}

fn open_review_month(app: &mut Headless) {
    app.press("role:button[Tổng kết & xem lại]").expect("the review row is a button");
    app.wait_for("Câu hỏi tổng kết").expect("the review opens");
    app.press("role:button[Kỳ: Tháng]").expect("the month tab");
}

fn shows_text_containing(app: &mut Headless, what: &str, needle: &str) {
    app.wait_until(what, WAIT, |screen| screen.texts().iter().any(|t| t.contains(needle)))
        .unwrap_or_else(|error| panic!("{what}: {error}"));
}

/// Waits until the file exists with something in it.
fn wait_for_file(path: &Path) -> Vec<u8> {
    wait_until(&format!("{} being written", path.display()), || {
        std::fs::read(path).is_ok_and(|bytes| !bytes.is_empty())
    });
    std::fs::read(path).expect("the file was just read")
}

#[test]
fn a_trade_is_shared_as_a_picture_that_follows_the_privacy_choice() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trade(&mut app, "EURUSD");
    app.press("role:button[Chia sẻ]").expect("a closed trade has a share button");
    app.wait_for("Chia sẻ kết quả").expect("the dialog opens");
    app.wait_for("label:Xem trước thẻ chia sẻ").expect("the card is drawn and shown");

    let out = tempfile::tempdir().expect("a temp dir");
    let only_r = out.path().join("only-r.png");
    app.on_pick_save_path(Some(only_r.clone())).expect("answers the save dialog");
    app.press("role:button[Lưu ảnh]").expect("the save button");
    let first = wait_for_file(&only_r);
    assert_eq!(&first[..8], b"\x89PNG\r\n\x1a\n", "a PNG is saved");
    shows_text_containing(&mut app, "the dialog says where it went", "Đã lưu ảnh");

    app.press("role:button[Quyền riêng tư: Hiện tiền]").expect("the privacy choice");
    let with_money = out.path().join("money.png");
    app.on_pick_save_path(Some(with_money.clone())).expect("answers the next save dialog");
    // The picture is drawn again off the UI thread: save until the one with money is the saved one.
    let started = std::time::Instant::now();
    loop {
        app.press("role:button[Lưu ảnh]").expect("the save button");
        let second = wait_for_file(&with_money);
        if second != first {
            break;
        }
        std::fs::remove_file(&with_money).expect("the file is removed to be saved again");
        assert!(started.elapsed() < WAIT, "the picture never changed with the privacy choice");
        std::thread::sleep(Duration::from_millis(100));
    }

    app.press("role:button[Sao chép ảnh]").expect("the copy button");
    app.wait_for("Đã sao chép ảnh vào clipboard.").expect("copying is reported");
    app.press("role:button[Đóng]").expect("the close button");
    app.wait_for_gone("Chia sẻ kết quả", WAIT).expect("the dialog closes");
}

#[test]
fn a_trade_still_open_has_nothing_to_share() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let entry = deal(10, 7, "EURUSD", now_ms(), DealKind::Buy, DealEntry::In, 1.1, 0.0);
    let batch = DealBatch {
        account: AccountInfo { login: "1".into(), server: "Demo".into(), ..AccountInfo::default() },
        deals: vec![entry],
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("the batch is stored");
    drop(db);
    let mut app = fixture.start();
    open_trade(&mut app, "EURUSD");
    let enabled = app.screen().enabled("role:button[Chia sẻ]").expect("the share button is there");
    assert!(!enabled, "the share button is off for an open trade");
}

#[test]
fn the_months_card_and_pdf_come_from_the_review() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_review_month(&mut app);
    app.press("role:button[Chia sẻ]").expect("the review has a share button");
    app.wait_for("label:Xem trước thẻ chia sẻ").expect("the month's card is drawn");
    shows_text_containing(&mut app, "the dialog links to the month report", "Xuất báo cáo PDF tháng");

    let out = tempfile::tempdir().expect("a temp dir");
    let pdf = out.path().join("thang.pdf");
    let link = format!("role:button[Xuất báo cáo PDF tháng {} →]", chrono::Local::now().format("%m"));
    app.on_pick_save_path(Some(pdf.clone())).expect("answers the save dialog");
    let started = std::time::Instant::now();
    while !pdf.exists() {
        // The month's extra rows are read first: pressing before they arrive only says so.
        app.press(&link).ok();
        assert!(started.elapsed() < WAIT, "the PDF was not written");
        std::thread::sleep(Duration::from_millis(100));
    }
    let bytes = wait_for_file(&pdf);
    assert!(bytes.starts_with(b"%PDF-"), "a PDF is saved");
}

#[test]
fn the_review_exports_the_month_as_pdf_at_the_chosen_level() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_review_month(&mut app);
    app.wait_for("Báo cáo tháng").expect("the month tab has the report card");
    app.press("role:button[Mức hiển thị của PDF: Ẩn số]").expect("the PDF's privacy choice");

    let out = tempfile::tempdir().expect("a temp dir");
    let pdf = out.path().join("bao-cao.pdf");
    app.on_pick_save_path(Some(pdf.clone())).expect("answers the save dialog");
    let started = std::time::Instant::now();
    while !pdf.exists() {
        app.press("role:button[Xuất PDF tháng]").expect("the export button");
        assert!(started.elapsed() < WAIT, "the PDF was not written");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(wait_for_file(&pdf).starts_with(b"%PDF-"));
    shows_text_containing(&mut app, "the card says where it went", "Đã lưu báo cáo");

    app.press("role:button[Kỳ: Tuần]").expect("the week tab");
    app.wait_for_gone("Báo cáo tháng", WAIT).expect("the report card is only for months");
}

#[test]
fn the_ai_summary_is_a_disabled_placeholder_that_says_it_is_not_built() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Tổng kết & xem lại]").expect("the review row is a button");
    app.wait_for("Tóm tắt kỳ bằng AI").expect("the AI card shows");
    app.wait_for("Chưa làm").expect("it says it is not built yet");
    assert!(!app.screen().shows("Sẽ có ở v0.3"), "the old promise is gone");
    assert_eq!(app.screen().enabled("role:button[Tạo tóm tắt]"), Ok(false), "the button is off until it is built");
}
