//! The second round of fixes found by hand on the real window, each pinned headless on the journal kept
//! in R (`notion_r_journal.zip`: five trades on 13 to 16 April 2026, no money, no times of day): the
//! figures in the monospaced family, each page starting at the top, the months named by what they show,
//! the day lists and the week strip in R, the links, the disabled buttons and the empty states.

use std::path::{Path, PathBuf};

use nsh_core::ViewKind;
use nsh_testing::Headless;
use tradelens_core::{Db, import::notion};

mod support;
use support::{Fixture, wait_until};

fn core_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/fixtures").join(name)
}

/// A database with the five trades of the R fixture, as the import stores them.
fn seed_r_journal(db: &mut Db) {
    let (plan, _) = notion::read_source(&core_fixture("notion_r_journal.zip")).expect("the zip is understood");
    db.import_notion(&plan).expect("imports");
}

fn started(width: f32, height: f32) -> (Fixture, Headless) {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    wait_until("the five trades being stored", || fixture.db().deals(None).is_ok_and(|deals| deals.len() == 10));
    let app = fixture.start(width, height, |config| config.environment.os = nsh_core::platform::Os::Windows);
    (fixture, app)
}

fn open(app: &mut Headless, page: &str) {
    app.press(&format!("role:button[{page}]")).expect("a sidebar row");
}

/// Shows every date, however old.
fn all_time(app: &mut Headless) {
    app.press("role:button[Khoảng thời gian: Tất cả]").expect("the range switch");
}

fn shows_nothing_like(app: &Headless, what: &str) {
    let found: Vec<&str> = app.screen().texts().into_iter().filter(|t| t.contains(what)).collect();
    assert!(found.is_empty(), "no text should contain {what:?}: {found:?}");
}

/// How many text views read exactly `text`.
fn count_of(app: &Headless, text: &str) -> usize {
    app.screen().texts().into_iter().filter(|t| t.trim() == text).count()
}

/// The family the first text view reading exactly `text` is drawn in.
fn family_of(app: &Headless, text: &str) -> String {
    let style = app
        .screen()
        .span_style(&format!("text:{text} >> nth=0"))
        .unwrap_or_else(|error| panic!("{text:?} is not on screen: {error}"));
    style.font_family.map_or_else(|| "(the system's)".to_owned(), |family| family.name().to_string())
}

/// Waits for the theme's tokens (they reach the page a frame after it is built), then checks the figure.
fn assert_mono(app: &mut Headless, text: &str, what: &str) {
    let is_mono = |screen: &nsh_testing::Screen| {
        screen
            .span_style(&format!("text:{text} >> nth=0"))
            .is_ok_and(|style| style.font_family.is_some_and(|family| &*family.name() == "JetBrains Mono"))
    };
    let waited = app.wait_until("the figure in mono", std::time::Duration::from_secs(5), is_mono);
    assert!(waited.is_ok(), "{what} ({text:?}) is a figure: rule 2 sets it in mono, not in {}", family_of(app, text));
}

/// The top of the first thing the page's body shows, which moves up when the page is scrolled.
fn body_top(app: &Headless, first_text: &str) -> f32 {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == first_text))
        .unwrap_or_else(|| panic!("{first_text:?} is not on screen"))
        .bounds
        .origin
        .y
        .0
}

#[test]
fn figures_are_drawn_in_the_monospaced_family_everywhere_they_show() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    all_time(&mut app);
    app.wait_for("+1.4R").expect("the total R of the five trades");
    // A KPI tile, the symbol bars and the recent trades table of the overview.
    assert_mono(&mut app, "+1.4R", "a KPI value");
    app.wait_for("BTCUSD").expect("the recent trades");
    assert_mono(&mut app, "+2.0R", "a figure of the trades table");
    assert_mono(&mut app, "−1.0R", "a loss of the trades table");
    // A day of the small calendar.
    assert!(app.screen().matches("label:16/04 +1.0R").expect("a selector"), "the small month shows 16 April");

    open(&mut app, "Lịch P/L");
    app.wait_for("Tháng 4, 2026").expect("the calendar opens on April");
    app.wait_for("Kết quả tháng").expect("the month summary");
    assert_mono(&mut app, "+1.4R", "the month's result");

    open(&mut app, "Lệnh");
    app.wait_for("BTCUSD").expect("the list");
    assert_mono(&mut app, "+2.0R", "the R column");
}

#[test]
fn each_page_opens_at_the_top_whatever_the_page_before_was_scrolled_to() {
    // A window too short for the pages, so that they scroll.
    let (_fixture, mut app) = started(1280.0, 680.0);
    open(&mut app, "Phân tích");
    app.wait_for("Tổng hợp").expect("the analysis");
    let top = body_top(&app, "Tổng hợp");

    open(&mut app, "Nguồn dữ liệu");
    app.wait_for("Nhật ký import").expect("the data sources page");
    let scroll = app
        .screen()
        .layout_nodes()
        .into_iter()
        .filter(|n| n.kind == ViewKind::Scroll)
        .max_by(|a, b| a.bounds.size.width.0.total_cmp(&b.bounds.size.width.0))
        .expect("the page scrolls");
    app.scroll(&format!("view:{}", scroll.id), nsh_core::prelude::px(300.0)).expect("the page scrolls down");

    open(&mut app, "Phân tích");
    app.wait_for("Tổng hợp").expect("the analysis again");
    assert!(
        (body_top(&app, "Tổng hợp") - top).abs() < 0.5,
        "the analysis starts at the top, not scrolled by what the page before was"
    );
}

/// Scrolls the page's own scroll view (the widest one) down by 300 points.
fn scroll_page_down(app: &mut Headless) {
    let scroll = app
        .screen()
        .layout_nodes()
        .into_iter()
        .filter(|n| n.kind == ViewKind::Scroll)
        .max_by(|a, b| a.bounds.size.width.0.total_cmp(&b.bounds.size.width.0))
        .expect("the page scrolls");
    app.scroll(&format!("view:{}", scroll.id), nsh_core::prelude::px(300.0)).expect("the page scrolls down");
}

#[test]
fn a_trade_opened_from_a_scrolled_page_opens_at_its_top() {
    let (_fixture, mut app) = started(1280.0, 680.0);
    open(&mut app, "Lệnh");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade opens");
    app.wait_for("Nhật ký Notion · BTCUSD").expect("the trade's page");
    let top = body_top(&app, "Vào lệnh");

    // Scrolled down, the page goes back to the list and a trade is opened again from the overview.
    scroll_page_down(&mut app);
    assert!((body_top(&app, "Vào lệnh") - top).abs() > 100.0, "the page did scroll");
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    open(&mut app, "Tổng quan");
    app.wait_for("Win rate").expect("the overview");
    app.press("role:button[Khoảng thời gian: Tất cả]").expect("every date");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade of the recent list");
    app.wait_for("Nhật ký Notion · BTCUSD").expect("the trade's page again");
    assert!((body_top(&app, "Vào lệnh") - top).abs() < 0.5, "a trade's page starts at its top");
}

#[test]
fn the_months_are_named_by_what_they_show_and_the_overview_shows_a_month_with_trades() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    // Today's month has no trade (the journal is from April 2026): the overview shows April, as the
    // calendar opens on it, and each day carries its figure.
    app.wait_for("Tháng 4/2026").expect("the small month is titled by the month it shows");
    assert!(app.screen().matches("label:16/04 +1.0R").expect("a selector"), "16 April: two trades, +1.0R");
    assert!(app.screen().matches("label:13/04 −0.2R").expect("a selector"), "13 April: one loss");
    shows_nothing_like(&app, "Tháng này");

    open(&mut app, "Lịch P/L");
    app.wait_for("Kết quả tháng").expect("the month summary");
    shows_nothing_like(&app, "Tháng này");
    assert_eq!(count_of(&app, "Tháng 4, 2026"), 2, "the toolbar and the side card both name the month in view");
    app.press("role:button[Tháng trước]").expect("the month before");
    app.wait_for("Không có lệnh đóng").ok();
    assert_eq!(count_of(&app, "Tháng 3, 2026"), 2, "the side card follows the month in view");
}

#[test]
fn the_overview_has_the_add_trade_button_and_teal_links() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    app.wait_for("Thêm lệnh").expect("the header's primary button");
    let link = app.screen().span_style("text:Mở lịch").expect("the link's text");
    let teal = nsh_base::tokens_of(Some(include_str!("../theme.json"))).expect("theme.json").color("primary");
    let teal = teal.expect("the token");
    assert!(
        link.color == nsh_core::Color::from(teal.light) || link.color == nsh_core::Color::from(teal.dark),
        "a link wears the primary token, not the foreground: {:?}",
        link.color
    );
    app.press_key("ctrl+n").expect("the shortcut");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("Ctrl+N opens the manual entry from the overview");

    open(&mut app, "Tổng quan");
    app.wait_for("Win rate").expect("the overview is back");
    app.press("Thêm lệnh").expect("the button");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("the button does the same as Ctrl+N");

    open(&mut app, "Tổng quan");
    app.wait_for("Win rate").expect("the overview is back");
    app.press("Xem tất cả").expect("the link to the trades");
    app.wait_for("Gom từ các deal theo position; bấm một lệnh để mở nhật ký").expect("it goes to the list");
}

#[test]
fn a_range_with_no_trade_shows_dashes_and_never_a_zero_amount() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    // 30N: the journal is from April.
    app.wait_for("Chưa có lệnh đóng trong khoảng này").expect("the first tile says why it has nothing");
    shows_nothing_like(&app, "$0.00");
    shows_nothing_like(&app, "0 lệnh");
    shows_nothing_like(&app, "0 thắng");
    assert!(count_of(&app, "—") >= 5, "every tile is a dash: {:?}", app.screen().texts());

    open(&mut app, "Nhật ký ngày");
    app.wait_for("Không có lệnh nào trong ngày").expect("today has no trade");
    shows_nothing_like(&app, "$0.00");
    shows_nothing_like(&app, "0.0R");
    app.wait_for("text:/^—$/").expect("the day's result is a dash");
}

#[test]
fn the_week_strip_and_the_day_list_agree_and_never_print_r_twice() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    open(&mut app, "Lịch P/L");
    app.wait_for("Tháng 4, 2026").expect("April");
    // The day panel: two trades, each with its R once (there is no money to put beside it).
    assert_eq!(count_of(&app, "+2.0R"), 1, "{:?}", app.screen().texts());
    assert_eq!(count_of(&app, "−1.0R"), 1, "{:?}", app.screen().texts());
    app.press("role:button[Mở nhật ký ngày]").expect("the link to the day");
    app.wait_for("Lệnh trong ngày").expect("the day journal");
    app.wait_for("2 lệnh · 1 thắng").expect("the day's trades");
    // The strip: Monday to Thursday of that week, in R as the calendar has them.
    for figure in ["−0.2R", "+0.6R", "+1.0R"] {
        app.wait_for(&format!("text:{figure} >> nth=0")).unwrap_or_else(|e| panic!("{figure} in the strip: {e}"));
    }
    // The list of the day: each trade's R once, and no money column to repeat it.
    assert_eq!(count_of(&app, "+2.0R"), 1, "a trade in R prints its R once: {:?}", app.screen().texts());
    assert_eq!(count_of(&app, "−1.0R"), 1, "{:?}", app.screen().texts());
    shows_nothing_like(&app, "$");
}

#[test]
fn a_trade_of_the_journal_has_no_time_no_source_key_and_no_ea_screenshots() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    all_time(&mut app);
    app.wait_for("BTCUSD").expect("the recent trades");
    shows_nothing_like(&app, "00:00");
    assert!(
        app.screen().texts().iter().any(|t| t.trim() == "16/04"),
        "a trade with a date only shows the date: {:?}",
        app.screen().texts()
    );
    open(&mut app, "Lệnh");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade opens");
    app.wait_for("Nhật ký Notion · BTCUSD").expect("the breadcrumb names the account, not its key");
    shows_nothing_like(&app, "(notion)");
    shows_nothing_like(&app, "tự chụp");
    shows_nothing_like(&app, "Tự chụp");
}

#[test]
fn the_heatmap_legend_goes_with_the_grid() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    open(&mut app, "Phân tích");
    all_time(&mut app);
    app.wait_for("Dữ liệu không có giờ vào lệnh").expect("every trade lacks an hour: an empty state");
    shows_nothing_like(&app, "Đậm hơn");
}

#[test]
fn a_button_that_is_switched_off_is_muted_not_just_a_little_dimmer() {
    let (_fixture, mut app) = started(1400.0, 1300.0);
    open(&mut app, "Đăng nhập & sync");
    app.wait_for("Sync ngay").expect("the cloud page");
    assert_eq!(app.screen().enabled("role:button[Sync ngay]"), Ok(false), "nobody is signed in");
    let ink = app.screen().span_style("text:Sync ngay >> nth=0").expect("the label").color;
    let tokens = nsh_base::tokens_of(Some(include_str!("../theme.json"))).expect("theme.json");
    let muted = tokens.color("muted-foreground").expect("the token");
    assert!(
        ink == nsh_core::Color::from(muted.light) || ink == nsh_core::Color::from(muted.dark),
        "the label of a switched-off button is the muted ink: {ink:?}"
    );
}
