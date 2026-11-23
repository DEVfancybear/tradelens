//! The Notion API against the mock: pagination, flattening and the import plan.

use serde_json::{Value, json};
use tradelens_cloud::CloudError;
use tradelens_cloud::notion::NotionClient;
use tradelens_core::import::notion::Field;

use crate::mock::{Mock, Reply, Seen};

fn text(kind: &str, value: &str) -> Value {
    json!({"type": kind, kind: [{"plain_text": value}]})
}

fn select(name: &str) -> Value {
    json!({"type": "select", "select": {"name": name}})
}

fn number(n: f64) -> Value {
    json!({"type": "number", "number": n})
}

/// One row of the journal database, as the API returns a page.
struct Trade<'a> {
    title: &'a str,
    date: Value,
    pair: &'a str,
    side: &'a str,
    lot: f64,
    entry: f64,
    exit: f64,
    pnl: f64,
    tags: &'a [&'a str],
}

impl Trade<'_> {
    fn page(&self) -> Value {
        json!({
            "object": "page",
            "properties": {
                "Name": text("title", self.title),
                "Date": {"type": "date", "date": self.date},
                "Pair": select(self.pair),
                "Direction": select(self.side),
                "Lot": number(self.lot),
                "Entry": number(self.entry),
                "Exit": number(self.exit),
                "P&L": number(self.pnl),
                "Setup": select("Breakout"),
                "Tags": {"type": "multi_select", "multi_select": self.tags.iter().map(|t| json!({"name": t})).collect::<Vec<_>>()},
                "Notes": text("rich_text", "Held to the target."),
                "Followed plan": {"type": "checkbox", "checkbox": true},
            },
        })
    }
}

fn journal_mock() -> Mock {
    Mock::start(|req: &Seen, _| {
        assert_eq!(req.header("notion-version"), Some("2022-06-28"), "the pinned API version");
        assert_eq!(req.header("authorization"), Some("Bearer ntn_secret"), "the integration token");
        let cursor = req.json()["start_cursor"].as_str().map(str::to_owned);
        match (req.path(), cursor.as_deref()) {
            ("/v1/search", None) => {
                assert_eq!(req.json()["filter"], json!({"property": "object", "value": "database"}), "databases only");
                Reply::ok(&json!({
                    "results": [{
                        "object": "database", "id": "db-1", "title": [{"plain_text": "Nhật ký "}, {"plain_text": "lệnh"}],
                        "properties": {"P&L": {"type": "number"}, "Name": {"type": "title"}, "Pair": {"type": "select"}},
                    }],
                    "has_more": true, "next_cursor": "s2",
                }))
            }
            ("/v1/search", Some("s2")) => Reply::ok(&json!({
                "results": [{"object": "database", "id": "db-2", "title": [], "properties": {}}],
                "has_more": false, "next_cursor": null,
            })),
            ("/v1/databases/db-1/query", None) => Reply::ok(&json!({
                "results": [
                    Trade {
                        title: "EURUSD breakout",
                        date: json!({"start": "2026-11-16T09:00:00.000+07:00", "end": "2026-11-16T15:10:00.000+07:00"}),
                        pair: "EURUSD",
                        side: "Long",
                        lot: 0.2,
                        entry: 1.085,
                        exit: 1.095,
                        pnl: 150.0,
                        tags: &["London open", "Theo trend H4"],
                    }.page(),
                    Trade {
                        title: "Gold fade",
                        date: json!({"start": "2026-11-17T08:15:00.000+07:00"}),
                        pair: "XAUUSD",
                        side: "Short",
                        lot: 0.1,
                        entry: 2650.25,
                        exit: 2660.25,
                        pnl: -100.0,
                        tags: &["News"],
                    }.page(),
                ],
                "has_more": true, "next_cursor": "q2",
            })),
            ("/v1/databases/db-1/query", Some("q2")) => Reply::ok(&json!({
                "results": [Trade {
                    title: "Gold swing",
                    date: json!({"start": "2026-11-18T10:00:00.000+07:00"}),
                    pair: "XAUUSD",
                    side: "Long",
                    lot: 0.3,
                    entry: 2640.0,
                    exit: 2644.0,
                    pnl: 1200.0,
                    tags: &[],
                }.page()],
                "has_more": false, "next_cursor": null,
            })),
            ("/v1/databases/missing/query", _) => Reply::json(
                404,
                &json!({"object": "error", "status": 404, "code": "object_not_found", "message": "Could not find database. Share it with the integration."}),
            ),
            other => panic!("unexpected {other:?}"),
        }
    })
}

fn client(mock: &Mock) -> NotionClient {
    NotionClient::new("ntn_secret").with_base_url(format!("{}/v1/", mock.base))
}

#[test]
fn databases_are_collected_across_pages() {
    let mock = journal_mock();
    let databases = client(&mock).databases().expect("lists");
    assert_eq!(databases.len(), 2);
    assert_eq!((databases[0].id.as_str(), databases[0].title.as_str()), ("db-1", "Nhật ký lệnh"));
    assert_eq!(databases[0].properties, ["Name", "P&L", "Pair"], "the title property first");
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn rows_are_flattened_like_the_csv_export() {
    let mock = journal_mock();
    let rows = client(&mock).rows("db-1").expect("queries");
    assert_eq!(rows.len(), 3, "both pages");
    let first: std::collections::HashMap<_, _> = rows[0].iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(rows[0][0].0, "Name", "the title property comes first");
    assert_eq!(first["Date"], "2026-11-16 09:00 → 2026-11-16 15:10");
    assert_eq!(first["Tags"], "London open, Theo trend H4");
    assert_eq!(first["P&L"], "150");
    assert_eq!(first["Entry"], "1.085");
    assert_eq!(first["Followed plan"], "Yes");
    assert_eq!(mock.requests()[1].json()["start_cursor"], json!("q2"));
}

#[test]
fn the_table_goes_through_the_importers_detection() {
    let mock = journal_mock();
    let table = client(&mock).table("db-1").expect("table");
    assert_eq!(table.headers[0], "Name");
    assert_eq!(table.rows.len(), 3);

    let plan = client(&mock).plan("db-1").expect("the importer understands the API's table");
    let column = |field| plan.mapping.column(field);
    assert_eq!(column(Field::OpenTime), Some("Date"));
    assert_eq!(column(Field::Symbol), Some("Pair"));
    assert_eq!(column(Field::Side), Some("Direction"));
    assert_eq!(column(Field::Volume), Some("Lot"));
    assert_eq!(column(Field::Profit), Some("P&L"));
    assert_eq!(column(Field::Setup), Some("Setup"));
    assert_eq!(column(Field::Tags), Some("Tags"));
    assert_eq!((plan.rows.len(), plan.issues.len(), plan.total_rows), (3, 0, 3), "{:?}", plan.issues);

    let first = &plan.rows[0];
    assert_eq!(first.title, "EURUSD breakout");
    assert!((first.net() - 150.0).abs() < 1e-9, "net {}", first.net());
    assert_eq!(first.journal.tags, ["London open", "Theo trend H4"]);
    assert_eq!(first.journal.setup, "Breakout");
    assert_eq!(first.journal.followed_plan, Some(true));
    assert!(plan.rows[1].exit.profit < 0.0, "the short lost");
}

#[test]
fn a_database_not_shared_with_the_integration_is_a_service_error() {
    let mock = journal_mock();
    let result = client(&mock).rows("missing");
    assert!(
        matches!(&result, Err(CloudError::Service { status: 404, message }) if message.contains("Share it")),
        "{result:?}"
    );
}

#[test]
fn the_token_never_shows_in_debug() {
    let shown = format!("{:?}", NotionClient::new("ntn_secret"));
    assert!(!shown.contains("ntn_secret"), "{shown}");
}
