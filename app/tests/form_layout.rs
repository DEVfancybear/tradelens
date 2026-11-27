//! Layout fixes found on the real window, pinned on the boxes the engine lays out: the breakdown
//! tables of the analysis keep their figures whole and apart, the edit form of a trade fills the page
//! like the new-entry form does with fields wide enough for their digits, what is wrong with a
//! field of the form is said under that field, and the two columns of the settings are balanced.

use nsh_core::{ApproxTextEngine, TextEngine};
use nsh_testing::{Headless, LayoutNode};
use tradelens_core::{Db, ManualTrade, Side, TradeKey, model::AccountInfo, time::parse_mt5_time};

mod support;
use support::{Fixture, WAIT, wait_until};

fn time(text: &str) -> i64 {
    parse_mt5_time(text).expect("fixture times parse")
}

fn info() -> AccountInfo {
    AccountInfo { login: "5012345".into(), server: "Demo".into(), currency: "USD".into(), ..AccountInfo::default() }
}

/// A trade kept in R alone: no money, no lot, no prices, a day without a time.
fn in_r(symbol: &str, day: u32, r: f64) -> ManualTrade {
    ManualTrade {
        symbol: symbol.into(),
        side: Side::Long,
        volume: None,
        open_time: time(&format!("2026.11.{day:02} 00:00:00")),
        open_price: None,
        close_time: time(&format!("2026.11.{day:02} 00:00:00")),
        close_price: None,
        date_only: true,
        sl: 0.0,
        tp: 0.0,
        profit: None,
        commission: 0.0,
        swap: 0.0,
        result_r: Some(r),
    }
}

/// A trade with money, lot and prices, typed by hand.
fn with_money(symbol: &str, day: u32, profit: f64) -> ManualTrade {
    ManualTrade {
        symbol: symbol.into(),
        side: Side::Long,
        volume: Some(0.5),
        open_time: time(&format!("2026.11.{day:02} 08:00:00")),
        open_price: Some(1.0850),
        close_time: time(&format!("2026.11.{day:02} 09:00:00")),
        close_price: Some(1.08505),
        date_only: false,
        sl: 1.0830,
        tp: 0.0,
        profit: Some(profit),
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    }
}

fn typed_in(db: &mut Db, trade: &ManualTrade) -> TradeKey {
    let account =
        db.accounts().expect("accounts").first().map_or_else(|| db.upsert_account(&info()).expect("account"), |a| a.id);
    db.add_manual_trade(account, trade).expect("stored")
}

fn analysis_on(fixture: &Fixture, width: f32, tab: &str) -> Headless {
    let mut app = fixture.start(width, 1300.0, |_| {});
    app.press("role:button[Phân tích]").expect("the analysis row");
    app.press("role:button[Khoảng thời gian: Tất cả]").expect("the range switch");
    app.press(&format!("label:Phân tích: {tab}")).expect("the tab");
    app
}

/// Every text view whose text is one of `texts`.
fn texts_of<'a>(nodes: &'a [LayoutNode], texts: &[&str]) -> Vec<&'a LayoutNode> {
    nodes.iter().filter(|n| n.text.as_ref().is_some_and(|t| texts.contains(&t.text.text.as_str()))).collect()
}

/// Asserts that no text of `nodes` is cut: the engine measures what each needs and the box has it.
fn assert_whole(nodes: &[LayoutNode], texts: &[&str]) -> usize {
    let engine = ApproxTextEngine;
    let mut checked = 0;
    for node in texts_of(nodes, texts) {
        let shown = node.text.as_ref().expect("a text view");
        let needs = engine.measure(&engine.prepare(&shown.text), None).width.0;
        assert!(
            shown.content.size.width.0 + 0.5 >= needs,
            "{:?} is cut: it needs {needs} points and its column gives {}",
            shown.text.text,
            shown.content.size.width.0
        );
        checked += 1;
    }
    checked
}

const FIGURES: &[&str] = &["100.0%", "+10.8R", "50+", "+1.54R", "+7.7R", "91.7%", "+0.90R", "—", "12", "5"];
const TITLES: &[&str] = &["Lệnh", "Win", "Tổng R", "PF", "R TB"];

fn seed_r_symbols(db: &mut Db) {
    // XAUUSD: eleven wins and one small loss, so that its profit factor is huge (11 / 0.2 = 55).
    for day in 1..=11 {
        typed_in(db, &in_r("XAUUSD", day, 1.0));
    }
    typed_in(db, &in_r("XAUUSD", 12, -0.2));
    // EURUSD: five wins, so that its win rate is 100.0%.
    for day in 13..=17 {
        typed_in(db, &in_r("EURUSD", day, 1.54));
    }
}

#[test]
fn the_figures_of_the_symbol_table_are_whole_apart_and_under_their_titles_in_a_wide_card() {
    let fixture = Fixture::new();
    seed_r_symbols(&mut fixture.db());
    let mut app = analysis_on(&fixture, 1700.0, "Symbol");
    app.wait_for("XAUUSD").expect("the rows");
    app.wait_for("text:/^\\+1\\.54R$/").expect("the averages");
    app.wait_for("50+").expect("a profit factor made by one tiny loss is capped, not 55.00");
    let nodes = app.screen().layout_nodes();
    let cut = assert_whole(&nodes, FIGURES) + assert_whole(&nodes, TITLES);
    assert!(cut >= 10, "the figures and titles were found: {cut}");

    // The row of EURUSD: its figures from left to right, then the title of each column over them.
    let name = texts_of(&nodes, &["EURUSD"]).first().copied().expect("the name");
    let row: Vec<&LayoutNode> = {
        let mut figures: Vec<&LayoutNode> = texts_of(&nodes, &["5", "100.0%", "+7.7R", "—", "+1.54R"])
            .into_iter()
            .filter(|n| (n.bounds.origin.y.0 - name.bounds.origin.y.0).abs() < 12.0)
            .collect();
        figures.sort_by(|a, b| a.bounds.origin.x.0.total_cmp(&b.bounds.origin.x.0));
        figures
    };
    assert_eq!(row.len(), 5, "the five figures of the row: {:?}", row.iter().map(|n| n.bounds).collect::<Vec<_>>());
    for pair in row.windows(2) {
        let gap = pair[1].bounds.origin.x.0 - pair[0].bounds.right().0;
        assert!(gap >= 12.0, "figures stand {gap} points apart: {:?} and {:?}", pair[0].bounds, pair[1].bounds);
    }
    // The name takes what the figures leave: it ends before them and is not cut either.
    assert!(name.bounds.right().0 <= row[0].bounds.origin.x.0, "the name is left of the figures");
    assert!(name.bounds.size.width.0 > 200.0, "the name column takes the room that is left: {:?}", name.bounds);
    // Each title ends where its figures end.
    for (title, figure) in ["Lệnh", "Win", "Tổng R", "PF", "R TB"].into_iter().zip(&row) {
        // The title of the table above the figure (the sidebar has a "Lệnh" too, and the side card titles).
        let head = texts_of(&nodes, &[title])
            .into_iter()
            .filter(|n| n.bounds.origin.y.0 < figure.bounds.origin.y.0)
            .min_by(|a, b| {
                let off = |n: &LayoutNode| (n.bounds.right().0 - figure.bounds.right().0).abs();
                off(a).total_cmp(&off(b))
            })
            .expect("a title");
        assert!(
            (head.bounds.right().0 - figure.bounds.right().0).abs() <= 1.0,
            "{title} ends at {} and its figure at {}",
            head.bounds.right().0,
            figure.bounds.right().0
        );
    }
}

#[test]
fn money_in_the_tables_fits_and_the_side_card_stays_beside_the_main_one_in_the_default_window() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    typed_in(&mut db, &with_money("EURUSD", 3, 12_345.67));
    typed_in(&mut db, &with_money("EURUSD", 4, -1_126.4));
    typed_in(&mut db, &with_money("XAUUSD", 5, 40.0));
    drop(db);
    // 1280 is the default window: the side card (Buy and Sell) stands beside the main one, as it did
    // before the figures were given the room they need.
    for width in [1280.0_f32, 1500.0] {
        let mut app = analysis_on(&fixture, width, "Symbol");
        app.wait_for("XAUUSD").expect("the rows");
        app.wait_for("Chiều").expect("the side card");
        app.wait_for("Buy").expect("the Buy row");
        let nodes = app.screen().layout_nodes();
        let money = ["+$11,219.27", "+$40.00", "+$11,259.27"];
        let checked = assert_whole(&nodes, &money) + assert_whole(&nodes, &["Net", "Lệnh", "Win", "PF", "R TB"]);
        assert!(checked >= 10, "{width}: the figures were found: {checked}");
        let main = texts_of(&nodes, &["Theo symbol"]).first().copied().expect("the main card");
        let side = texts_of(&nodes, &["Buy so với Sell"]).first().copied().expect("the side card");
        assert!(
            side.bounds.origin.x.0 > main.bounds.origin.x.0 + 400.0,
            "{width}: the side card stays beside the main one"
        );
        // Nothing of the tables sticks out of the page on the right (the page's padding and the card's).
        for node in nodes.iter().filter(|n| n.text.is_some() && n.bounds.origin.y.0 > 150.0) {
            assert!(
                node.bounds.right().0 <= width - 32.0 - 20.0 + 0.5,
                "{width}: {:?} sticks out of its card",
                node.bounds
            );
        }
    }
}

#[test]
fn every_table_of_the_analysis_keeps_its_figures_whole_inside_its_card() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    typed_in(&mut db, &with_money("EURUSD", 3, 12_345.67));
    typed_in(&mut db, &with_money("XAUUSD", 5, -40.0));
    drop(db);
    // The tables of the session (side of the overview), the hour, the symbol and the side, the setup, the
    // tag and the emotion: in the default window and in a wide one.
    let mut checked = 0;
    for width in [1280.0_f32, 1700.0] {
        let mut app = analysis_on(&fixture, width, "Tổng hợp");
        for tab in ["Tổng hợp", "Symbol", "Thời gian", "Setup & tag", "Lỗi & cảm xúc"] {
            app.press(&format!("label:Phân tích: {tab}")).expect("the tab");
            app.wait_for("Lệnh").expect("a table is there");
            let nodes = app.screen().layout_nodes();
            let figures: Vec<&str> = nodes
                .iter()
                .filter_map(|n| n.text.as_ref().map(|t| t.text.text.as_str()))
                .filter(|t| t.len() <= 12 && !t.contains(' ') && t.chars().any(|c| c.is_ascii_digit()))
                .collect();
            checked += assert_whole(&nodes, &figures);
            for node in
                nodes.iter().filter(|n| n.text.is_some() && n.bounds.origin.x.0 > 250.0 && n.bounds.origin.y.0 > 150.0)
            {
                assert!(
                    node.bounds.right().0 <= width - 32.0 - 20.0 + 0.5,
                    "{width} {tab}: {:?} sticks out of its card",
                    node.bounds
                );
            }
        }
    }
    assert!(checked >= 20, "the figures of the tables were found: {checked}");
}

#[test]
fn the_edit_form_fills_the_page_with_a_side_card_and_its_number_fields_have_room_for_their_digits() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let key = typed_in(&mut db, &with_money("EURUSD", 19, 25.0));
    drop(db);
    let mut app = fixture.start(1700.0, 1000.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for_within("role:button[Mở nhật ký lệnh EURUSD]", WAIT).expect("the trade");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("opens");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("the page");
    app.press("role:button[Sửa lệnh]").expect("the edit button");
    app.wait_for_within("Thông tin lệnh", WAIT).expect("the form");
    app.wait_for("Nhật ký & ảnh").expect("the side card says what the trade keeps");
    app.wait_for("Nhật ký và ảnh sửa ở trang lệnh").expect("and where to edit it");

    let nodes = app.screen().layout_nodes();
    let main = texts_of(&nodes, &["Thông tin lệnh"]).first().copied().expect("the main card's title");
    let side = texts_of(&nodes, &["Nhật ký & ảnh"]).first().copied().expect("the side card's title");
    assert!(side.bounds.origin.x.0 > main.bounds.origin.x.0 + 500.0, "the side card is at the right of the form");
    assert!(side.bounds.origin.y.0 < main.bounds.origin.y.0 + 40.0, "beside it, not below it");
    let window_right = 1700.0 - 32.0;
    let link = texts_of(&nodes, &["Nhật ký và ảnh sửa ở trang lệnh"]).first().copied().expect("the link");
    assert!(link.bounds.right().0 < window_right, "inside the page");
    // The page is not left half empty: the last field of a row reaches towards the side card.
    let last = texts_of(&nodes, &["Take profit"]).first().copied().expect("a caption of the last row");
    assert!(last.bounds.origin.x.0 > 600.0, "the fields spread over the card: {:?}", last.bounds);

    // The number fields are wide enough for a price to five decimals and a lot of 100.00, and grow.
    // The labelled box is the input between the step buttons: `1.08505` is seven glyphs of 8.4 points.
    let digits = 7.0 * 8.4;
    for label in ["Giá vào", "Giá ra", "Stop loss", "Take profit", "Khối lượng", "Lãi/lỗ", "Commission", "Swap"]
    {
        let id = app.screen().id(&format!("label:{label}")).unwrap_or_else(|e| panic!("{label}: {e}"));
        let node = nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("{label} has a box"));
        assert!(
            node.bounds.size.width.0 >= digits + 40.0,
            "{label} gives its digits {} points",
            node.bounds.size.width.0
        );
    }
    // And it grows with the grid: the card is ~950 points wide, four fields share it.
    let id = app.screen().id("label:Giá vào").expect("the price field");
    let price = nodes.iter().find(|n| n.id == id).expect("a box");
    assert!(price.bounds.size.width.0 > 140.0, "the field grows with the grid: {:?}", price.bounds);

    // The wide form still saves what is typed in it.
    app.clear("label:Giá ra").expect("the field clears");
    app.type_text("label:Giá ra", "1.2").expect("a price");
    app.press("role:button[Lưu thay đổi]").expect("saves");
    let db = fixture.db();
    wait_until("the new closing price being stored", || {
        db.manual_trade(&key).is_ok_and(|t| t.is_some_and(|t| t.close_price == Some(1.2)))
    });
}

fn start_new_entry(fixture: &Fixture) -> Headless {
    fixture.db().upsert_account(&info()).expect("an account");
    let mut app = fixture.start(1700.0, 1100.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for_within("Nhập lệnh & nhật ký thủ công", WAIT).expect("the form opens");
    app
}

#[test]
fn a_missing_field_is_said_under_that_field_and_not_in_the_column_of_images() {
    let fixture = Fixture::new();
    let mut app = start_new_entry(&fixture);
    app.press("role:button[Lưu lệnh]").expect("saves");
    app.wait_for_within("text:/Nhập mã giao dịch/", WAIT).expect("the symbol is asked for");
    app.wait_for("text:/^Thời gian ra chưa chọn/").expect("so is the closing time");
    let nodes = app.screen().layout_nodes();
    let images = texts_of(&nodes, &["Ảnh & đính kèm"]).first().copied().expect("the images card");
    let by_prefix = |prefix: &str| {
        nodes
            .iter()
            .find(|n| n.text.as_ref().is_some_and(|t| t.text.text.starts_with(prefix)))
            .unwrap_or_else(|| panic!("no text starts with {prefix:?}"))
    };
    for (message, caption, control) in [
        ("Nhập mã giao dịch", "Symbol", "label:Symbol"),
        ("Thời gian ra chưa chọn", "Thời gian ra", "label:Thời gian ra"),
        ("Khối lượng phải là số", "Khối lượng", "label:Khối lượng"),
        ("Giá vào phải là số", "Giá vào", "label:Giá vào"),
        ("Nhập lãi/lỗ", "Lãi/lỗ", "label:Lãi/lỗ"),
    ] {
        let said = by_prefix(message);
        let caption = texts_of(&nodes, &[caption]).first().copied().expect("the caption");
        let id = app.screen().id(control).unwrap_or_else(|e| panic!("{control}: {e}"));
        let field = nodes.iter().find(|n| n.id == id).expect("the control's box");
        assert!(said.bounds.origin.x.0 < images.bounds.origin.x.0 - 100.0, "{message:?} is in the form, not beside it");
        assert!(
            said.bounds.origin.x.0 >= caption.bounds.origin.x.0 - 1.0
                && said.bounds.origin.x.0 < field.bounds.right().0,
            "{message:?} stands in the column of its field: {:?} vs {:?}",
            said.bounds,
            field.bounds
        );
        assert!(
            said.bounds.origin.y.0 >= field.bounds.origin.y.0 + field.bounds.size.height.0 - 1.0
                && said.bounds.origin.y.0 < field.bounds.origin.y.0 + field.bounds.size.height.0 + 30.0,
            "{message:?} is right under its field: {:?} vs {:?}",
            said.bounds,
            field.bounds
        );
    }
    // Editing a field takes its message away.
    app.type_text("label:Symbol", "EURUSD").expect("a symbol");
    app.wait_for_gone("text:/Nhập mã giao dịch/", WAIT).expect("the message goes when the field is edited");
    app.wait_for("text:/^Khối lượng phải là số/").expect("the others stay");
}

/// The box of the card whose title is `title` (right of the sidebar): the view the title is in.
fn card_of<'a>(nodes: &'a [LayoutNode], title: &str) -> &'a LayoutNode {
    let heading = texts_of(nodes, &[title])
        .into_iter()
        .find(|n| n.bounds.origin.x.0 > 250.0)
        .unwrap_or_else(|| panic!("the card {title:?}"));
    nodes.iter().find(|n| n.id == heading.parent).unwrap_or_else(|| panic!("the box of {title:?}"))
}

#[test]
fn the_settings_columns_are_balanced_and_fit_a_maximised_window_without_scrolling() {
    let fixture = Fixture::new();
    // The maximised window the user reported and the same less a title bar fit the page; the default
    // window only keeps its columns balanced (its height is a little short of the page).
    for (width, height, fits) in [(1685.0_f32, 1017.0_f32, true), (1685.0, 985.0, true), (1280.0, 800.0, false)] {
        let mut app = fixture.start(width, height, |_| {});
        app.press("role:button[Cài đặt]").expect("the settings row");
        app.wait_for_within("text:/tradelens.db/", WAIT).expect("the data file is read");
        let nodes = app.screen().layout_nodes();
        let column = |titles: &[&str]| {
            let cards: Vec<_> = titles.iter().map(|t| card_of(&nodes, t).bounds).collect();
            let x = cards[0].origin.x.0;
            assert!(
                cards.iter().all(|c| (c.origin.x.0 - x).abs() < 1.0),
                "{width}x{height}: {titles:?} stand in one column: {cards:?}"
            );
            (x, cards.iter().map(|c| c.origin.y.0 + c.size.height.0).fold(0.0_f32, f32::max))
        };
        let (left_x, left_end) = column(&["Giao diện", "Thời gian & ngôn ngữ", "Phím tắt"]);
        let (right_x, right_end) = column(&["Dữ liệu", "Về TradeLens"]);
        assert!(right_x > left_x + 400.0, "{width}x{height}: two columns side by side");
        assert!(
            (left_end - right_end).abs() <= 120.0,
            "{width}x{height}: neither column is far longer: the left ends at {left_end}, the right at {right_end}"
        );
        let page = nodes
            .iter()
            .filter(|n| n.scroll.as_ref().is_some_and(|s| !s.horizontal) && n.bounds.origin.x.0 > 200.0)
            .max_by(|a, b| a.bounds.size.height.0.total_cmp(&b.bounds.size.height.0))
            .expect("the page's scroll view");
        let content = page.scroll.as_ref().and_then(|s| s.content_size).expect("its content is laid out");
        assert!(
            !fits || content.height.0 <= page.bounds.size.height.0 + 0.5,
            "{width}x{height}: the page has nothing to scroll: {} points in {}",
            content.height.0,
            page.bounds.size.height.0
        );
    }
}
