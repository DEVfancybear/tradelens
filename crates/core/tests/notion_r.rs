//! A trading journal kept in R (no money, no prices), read from the `.zip` Notion exports: the roles
//! of the columns found by what they hold, the results counted in R, the images attached.
//! The fixture `tests/fixtures/notion_r_journal.zip` (regenerate with `gen_fixtures`) holds five
//! invented trades in the shape of such an export.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use tradelens_core::Db;
use tradelens_core::import::notion::{self, Field, NotionPlan, Verdict};
use tradelens_core::metrics::{self, Sample};
use tradelens_core::model::{Deal, JournalEntry, TradeKey};
use tradelens_core::time::Clock;
use tradelens_core::trades::build_trades;

const R_ZIP: &str = "notion_r_journal.zip";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

#[track_caller]
fn assert_close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-9, "{what}: got {got}, want {want}");
}

fn journal_of(db: &Db, account_id: i64, position_id: i64) -> JournalEntry {
    db.journal(&TradeKey { account_id, position_id, segment: 0 }).expect("reads").unwrap_or_default()
}

/// The columns of the journal, with five invented trades (the zip fixture holds the same).
const HEADERS: [&str; 15] = [
    "Tên giao dịch",
    "Cặp tiền",
    "Ghi chú",
    "Loại lệnh",
    "Nhóm",
    "Có chờ?",
    "Ngày giao dịch",
    "Đủ điều kiện?",
    "Nhận định",
    "Risk/Reward",
    "Lưu ý",
    "Bối cảnh",
    "Đúng giờ?",
    "Trạng thái",
    "Ảnh",
];

const ROWS: [[&str; 15]; 5] = [
    [
        "BTC/USD LONG",
        "BTC/USD",
        "Ghi chú mẫu một",
        "Buy (Long)",
        "Bitcoin",
        "No",
        "April 16, 2026",
        "Yes",
        "Phân tích mẫu",
        "2",
        "Rủi ro mẫu",
        "Thuận trend",
        "Yes",
        "Thắng",
        "image.png",
    ],
    [
        "XAU/USD SHORT",
        "XAU/USD",
        "",
        "Sell (Short)",
        "XAU",
        "Yes",
        "April 16, 2026",
        "No",
        "",
        "-1",
        "",
        "Ngược trend",
        "No",
        "Thua",
        "image%201.png",
    ],
    [
        "USD/JPY LONG",
        "USD/JPY",
        "",
        "Buy (Long)",
        "Forex",
        "No",
        "April 15, 2026",
        "Yes",
        "",
        "0",
        "",
        "Thuận trend",
        "Yes",
        "Hòa",
        "",
    ],
    [
        "EUR/USD LONG",
        "EUR/USD",
        "",
        "Buy (Long)",
        "Forex",
        "No",
        "April 14, 2026",
        "Yes",
        "",
        "0.6",
        "",
        "Thuận trend",
        "No",
        "Thắng",
        "image.png, image%201.png",
    ],
    [
        "BTC/USD SHORT",
        "BTC/USD",
        "",
        "Sell (Short)",
        "Bitcoin",
        "No",
        "April 13, 2026",
        "No",
        "",
        "-0.2",
        "",
        "Ngược trend",
        "Yes",
        "Thua",
        "image%202.png",
    ],
];

/// The journal as CSV text, its columns in the order `order` names (indexes into the original columns)
/// under the names `names` gives.
fn csv(names: &[&str], order: &[usize]) -> String {
    let quote = |cell: &str| format!("\"{}\"", cell.replace('"', "\"\""));
    let line = |cells: Vec<&str>| cells.into_iter().map(quote).collect::<Vec<_>>().join(",");
    let mut lines = vec![line(order.iter().map(|&c| names[c]).collect())];
    lines.extend(ROWS.iter().map(|row| line(order.iter().map(|&c| row[c]).collect())));
    lines.join("\n")
}

/// What an import makes of each trade that does not depend on the columns' names or places.
type Understood = (String, String, i64, Option<f64>, Option<Verdict>, Vec<String>);

fn understood(plan: &NotionPlan) -> Vec<Understood> {
    plan.rows
        .iter()
        .map(|row| {
            (
                row.entry.symbol.clone(),
                row.entry.kind.as_str().to_owned(),
                row.entry.time_msc,
                row.result_r(),
                row.verdict,
                row.journal.screenshots.clone(),
            )
        })
        .collect()
}

#[test]
fn a_zip_export_is_read_from_its_all_csv_with_its_images_and_results_in_r() {
    let (plan, archive) = notion::read_source(&fixture_path(R_ZIP)).expect("the zip is understood");
    let archive = archive.expect("a zip has an archive");
    assert_eq!(plan.rows.len(), 5, "the `_all` file was read, not the one with two rows");
    assert!(plan.is_in_r() && plan.issues.is_empty());
    assert_close(plan.total_r(), 1.4, "2 - 1 + 0 + 0.6 - 0.2 R");
    assert_eq!(plan.verdicts(), (2, 2, 1));
    let found = |field| plan.mapping.column(field);
    assert_eq!(found(Field::ResultR), Some("Risk/Reward"));
    assert_eq!(found(Field::Symbol), Some("Cặp tiền"));
    assert_eq!(found(Field::Side), Some("Loại lệnh"));
    assert_eq!(found(Field::OpenTime), Some("Ngày giao dịch"));
    assert_eq!(found(Field::Screenshot), Some("Ảnh"));
    assert_eq!(found(Field::Profit), None);
    let outcome = plan.outcome.as_ref().expect("the status column was recognised");
    assert_eq!(outcome.column, "Trạng thái");
    assert!(plan.properties.iter().any(|p| p.label == "Trạng thái"), "the status stays a property too");

    let symbols: Vec<&str> = plan.rows.iter().map(|r| r.entry.symbol.as_str()).collect();
    assert_eq!(symbols, ["BTCUSD", "XAUUSD", "USDJPY", "EURUSD", "BTCUSD"], "separators dropped");
    let sides: Vec<&str> = plan.rows.iter().map(|r| r.entry.kind.as_str()).collect();
    assert_eq!(sides, ["buy", "sell", "buy", "buy", "sell"]);
    let verdicts: Vec<_> = plan.rows.iter().map(|r| r.verdict).collect();
    assert_eq!(
        verdicts,
        [Some(Verdict::Win), Some(Verdict::Loss), Some(Verdict::BreakEven), Some(Verdict::Win), Some(Verdict::Loss)]
    );
    // Two trades on 16 April keep the order of their rows; the date alone gives no time of day.
    assert!(plan.rows[0].entry.time_msc < plan.rows[1].entry.time_msc);
    assert_eq!(plan.rows[0].entry.time_msc / 86_400_000, plan.rows[1].entry.time_msc / 86_400_000);
    assert!(plan.rows.iter().all(|r| r.exit.profit == 0.0), "no money in the file");

    assert_eq!(archive.image_count(), 2);
    let counts = archive.count(&plan);
    assert_eq!((counts.attached, counts.missing), (4, 1), "image 2 is named by the last row but not in the zip");
}

#[test]
fn the_images_of_a_zip_become_the_screenshots_of_the_trades_once_each() {
    let (mut plan, archive) = notion::read_source(&fixture_path(R_ZIP)).expect("the zip is understood");
    let store = tempfile::tempdir().expect("a temp dir");
    let counts = archive.expect("an archive").attach_images(&mut plan, store.path(), 7).expect("attaches");
    assert_eq!((counts.attached, counts.missing), (4, 1));
    let stored: Vec<_> = fs::read_dir(store.path().join("images")).expect("the images folder").collect();
    assert_eq!(stored.len(), 2, "the same picture named by two rows is stored once");
    assert_eq!(plan.rows[3].journal.attachments.len(), 2);
    assert_eq!(plan.rows[0].journal.attachments[0].path, plan.rows[3].journal.attachments[0].path);
    assert_eq!(plan.rows[4].journal.missing_files, ["image%202.png"]);

    let mut db = Db::in_memory().expect("db");
    let report = db.import_notion(&plan).expect("imports");
    assert_eq!((report.created, report.skipped.len()), (5, 0));
    let account = db.accounts().expect("accounts").pop().expect("the account");
    let with_pictures = |row: usize| journal_of(&db, account.id, plan.rows[row].position_id).screenshots.len();
    assert_eq!((with_pictures(0), with_pictures(1), with_pictures(2), with_pictures(3)), (1, 1, 0, 2));
    let missing = journal_of(&db, account.id, plan.rows[4].position_id);
    assert!(missing.screenshots.is_empty() && missing.notes.contains("image%202.png"), "{missing:?}");
    let again = db.import_notion(&plan).expect("imports again");
    assert_eq!((again.created, again.unchanged), (0, 5), "the same export again stores nothing new");
}

#[test]
fn trades_in_r_count_by_their_r_and_have_no_money() {
    let (plan, _) = notion::read_source(&fixture_path(R_ZIP)).expect("the zip is understood");
    let mut db = Db::in_memory().expect("db");
    db.import_notion(&plan).expect("imports");
    let account = db.accounts().expect("accounts").pop().expect("the account");
    let deals: Vec<Deal> = db.deals(Some(account.id)).expect("deals").into_iter().map(|(_, d)| d).collect();
    let trades = build_trades(account.id, &deals);
    assert_eq!(trades.len(), 5);
    assert!(trades.iter().all(|t| !t.money_known() && t.net() == 0.0));
    let results: Vec<Option<f64>> = trades.iter().map(|t| t.r_multiple(None)).collect();
    assert_eq!(
        results,
        [Some(-0.2), Some(0.6), Some(0.0), Some(2.0), Some(-1.0)],
        "oldest first by open, the two of 16 April in row order"
    );

    let samples: Vec<_> = trades.iter().map(|t| Sample { trade: t, r: t.r_multiple(None) }).collect();
    let stats = metrics::stats(&samples, 0.0, &[], Clock::default());
    assert_eq!((stats.trades, stats.wins, stats.losses, stats.breakeven), (5, 2, 2, 1));
    assert_close(stats.win_rate, 0.4, "win rate by R");
    assert_close(stats.total_r, 1.4, "total R");
    assert_close(stats.avg_r.expect("an average R"), 0.28, "average R");
    assert_eq!((stats.net, stats.gross_profit, stats.gross_loss), (0.0, 0.0, 0.0), "no money was invented");
    let buckets = metrics::r_distribution(&samples);
    assert_eq!(buckets.iter().map(|(_, n)| n).sum::<usize>(), 5, "every trade is in the R distribution");
}

#[test]
fn columns_are_found_by_what_they_hold_whatever_they_are_called_and_wherever_they_stand() {
    let identity: Vec<usize> = (0..15).collect();
    let original = notion::infer(csv(&HEADERS, &identity).as_bytes()).expect("understood");
    let want = understood(&original);
    assert_eq!(want.len(), 5);
    let nonsense: Vec<String> = (1..=15).map(|n| format!("c{n}")).collect();
    let nonsense: Vec<&str> = nonsense.iter().map(String::as_str).collect();
    let other_language = [
        "名前",
        "ペア",
        "メモ",
        "方向",
        "種類",
        "指値",
        "日付",
        "質",
        "分析",
        "結果",
        "リスク",
        "トレンド",
        "米国",
        "状態",
        "画像",
    ];
    let reversed: Vec<usize> = (0..15).rev().collect();
    let shuffled = [7, 3, 12, 0, 9, 14, 1, 5, 10, 2, 13, 6, 11, 4, 8];
    for (what, names, order) in [
        ("renamed c1..c15", &nonsense[..], &identity[..]),
        ("renamed in another language", &other_language[..], &identity[..]),
        ("reordered back to front", &HEADERS[..], &reversed[..]),
        ("renamed and shuffled", &nonsense[..], &shuffled[..]),
    ] {
        let plan = notion::infer(csv(names, order).as_bytes()).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(understood(&plan), want, "{what}");
        assert!(plan.outcome.is_some(), "{what}: the status column is still recognised");
    }
}

#[test]
fn a_zip_with_several_csv_files_and_none_called_all_is_refused_and_a_single_one_is_used() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let write = |name: &str, entries: &[&str]| {
        let path = dir.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("creates"));
        for entry in entries {
            zip.start_file(*entry, zip::write::SimpleFileOptions::default()).expect("starts");
            zip.write_all(csv(&HEADERS, &(0..15).collect::<Vec<_>>()).as_bytes()).expect("writes");
        }
        zip.finish().expect("finishes");
        path
    };
    let single = write("single", &["Notion/Journal abc.csv", "__MACOSX/._Journal abc.csv"]);
    let (plan, _) = notion::read_source(&single).expect("one real csv is the journal");
    assert_eq!(plan.rows.len(), 5);
    let refused = notion::read_source(&write("two", &["a.csv", "b.csv"])).expect_err("two csv files, no `_all` one");
    assert!(refused.to_string().contains("2 file .csv"), "{refused}");
    let none = notion::read_source(&write("none", &["readme.txt"])).expect_err("no csv at all");
    assert!(none.to_string().contains("không có file .csv"), "{none}");
}
