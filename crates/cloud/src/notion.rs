//! The Notion API, for importing a trading journal database without exporting it:
//! an internal integration token, the databases shared with it, their rows.
//!
//! Every property is flattened to the text Notion's own CSV export would hold, so the rows
//! become a table that [`tradelens_core::import::notion::infer_table`] reads with the same
//! detection as a CSV file: [`NotionClient::table`] gives the table, [`NotionClient::plan`]
//! the import plan.

use chrono::{DateTime, NaiveDateTime, Timelike};
use serde_json::{Value, json};
use tradelens_core::import::notion::{self, NotionPlan};

use crate::CloudError;
use crate::http::{Http, Request, RetryPolicy, encode};

/// The API version the flattening below was written against.
const NOTION_VERSION: &str = "2022-06-28";
/// The most Notion returns per page.
const PAGE_SIZE: u32 = 100;
/// The name under which the integration token is kept in the OS credential store.
pub const TOKEN_SECRET: &str = "notion-token";

/// A database shared with the integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionDatabase {
    pub id: String,
    pub title: String,
    /// Property names, as the mapping screen lists them.
    pub properties: Vec<String>,
}

/// The rows of a database as the CSV export would lay them out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotionTable {
    /// Property names, the title property first.
    pub headers: Vec<String>,
    /// One row per page; `rows[i][j]` is the text of `headers[j]`, empty when unset.
    pub rows: Vec<Vec<String>>,
}

/// The API, `https://api.notion.com/v1`, `Notion-Version: 2022-06-28`.
pub struct NotionClient {
    token: String,
    base_url: String,
    http: Http,
}

impl std::fmt::Debug for NotionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionClient")
            .field("token", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl NotionClient {
    /// A client for the integration token (`secret_…` / `ntn_…`), kept in the OS store under
    /// [`TOKEN_SECRET`].
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into().trim().to_owned(),
            base_url: "https://api.notion.com/v1".to_owned(),
            http: Http::default(),
        }
    }

    /// Another API address, for tests.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        base_url.into().trim_end_matches('/').clone_into(&mut self.base_url);
        self
    }

    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.http = Http::new(retry);
        self
    }

    fn post(&self, path: &str, body: &Value) -> crate::Result<Value> {
        let request = Request::post(format!("{}{path}", self.base_url))
            .bearer(&self.token)
            .header("Notion-Version", NOTION_VERSION)
            .json(body);
        Ok(self.http.send(&request)?.ok()?.value())
    }

    /// The databases shared with the integration (`POST /search`, filter `database`).
    ///
    /// # Errors
    ///
    /// [`CloudError::Service`] when the token is refused; [`CloudError::Network`] when Notion
    /// cannot be reached.
    pub fn databases(&self) -> crate::Result<Vec<NotionDatabase>> {
        let mut found = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut body = json!({
                "filter": { "property": "object", "value": "database" },
                "page_size": PAGE_SIZE,
            });
            if let Some(cursor) = &cursor {
                body["start_cursor"] = json!(cursor);
            }
            let page = self.post("/search", &body)?;
            for database in page["results"].as_array().into_iter().flatten() {
                found.push(NotionDatabase {
                    id: database["id"].as_str().unwrap_or_default().to_owned(),
                    title: plain_text(&database["title"]),
                    properties: ordered_properties(&database["properties"]).into_iter().map(|(name, _)| name).collect(),
                });
            }
            match next_cursor(&page) {
                Some(next) => cursor = Some(next),
                None => return Ok(found),
            }
        }
    }

    /// Every row of a database as `(property, plain text)` pairs, the title property first
    /// (`POST /databases/{id}/query`, paginated by `start_cursor`).
    ///
    /// # Errors
    ///
    /// [`CloudError::Service`] when the database is not shared with the integration;
    /// [`CloudError::Network`] when Notion cannot be reached.
    pub fn rows(&self, database_id: &str) -> crate::Result<Vec<Vec<(String, String)>>> {
        let path = format!("/databases/{}/query", encode(database_id));
        let mut rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut body = json!({ "page_size": PAGE_SIZE });
            if let Some(cursor) = &cursor {
                body["start_cursor"] = json!(cursor);
            }
            let page = self.post(&path, &body)?;
            for row in page["results"].as_array().into_iter().flatten() {
                rows.push(
                    ordered_properties(&row["properties"])
                        .into_iter()
                        .map(|(name, property)| (name, flatten(property)))
                        .collect(),
                );
            }
            match next_cursor(&page) {
                Some(next) => cursor = Some(next),
                None => return Ok(rows),
            }
        }
    }

    /// The rows of a database as a table: the union of the properties as headers, in order of
    /// first appearance, every row filled to the same width.
    ///
    /// # Errors
    ///
    /// As [`NotionClient::rows`].
    pub fn table(&self, database_id: &str) -> crate::Result<NotionTable> {
        Ok(table_of(&self.rows(database_id)?))
    }

    /// The import plan of a database: [`NotionClient::table`] read by the same detection as a
    /// Notion CSV export.
    ///
    /// # Errors
    ///
    /// As [`NotionClient::rows`]; [`CloudError::Import`] when the table has no trade columns
    /// or no usable row.
    pub fn plan(&self, database_id: &str) -> crate::Result<NotionPlan> {
        let table = self.table(database_id)?;
        notion::infer_table(&table.headers, &table.rows).map_err(CloudError::from)
    }
}

fn next_cursor(page: &Value) -> Option<String> {
    if page["has_more"].as_bool() == Some(true) { page["next_cursor"].as_str().map(str::to_owned) } else { None }
}

/// The properties of a page or database, the title property first and the rest in the order
/// Notion lists them (alphabetical).
fn ordered_properties(properties: &Value) -> Vec<(String, &Value)> {
    let mut list: Vec<(String, &Value)> =
        properties.as_object().into_iter().flatten().map(|(name, property)| (name.clone(), property)).collect();
    list.sort_by_key(|(_, property)| property["type"].as_str() != Some("title"));
    list
}

fn table_of(rows: &[Vec<(String, String)>]) -> NotionTable {
    let mut headers: Vec<String> = Vec::new();
    for (name, _) in rows.iter().flatten() {
        if !headers.contains(name) {
            headers.push(name.clone());
        }
    }
    let cells = rows
        .iter()
        .map(|row| {
            headers
                .iter()
                .map(|header| row.iter().find(|(name, _)| name == header).map_or_default(|(_, v)| v.clone()))
                .collect()
        })
        .collect();
    NotionTable { headers, rows: cells }
}

// --- flattening --------------------------------------------------------------------

/// The text of rich text: its plain parts, joined.
fn plain_text(parts: &Value) -> String {
    parts.as_array().into_iter().flatten().filter_map(|part| part["plain_text"].as_str()).collect()
}

fn name_of(value: &Value) -> String {
    ["name", "id"].iter().find_map(|key| value[key].as_str()).unwrap_or_default().to_owned()
}

fn names(items: &Value) -> String {
    items.as_array().into_iter().flatten().map(name_of).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")
}

fn number_text(number: &Value) -> String {
    match number.as_f64() {
        Some(n) => n.to_string(),
        None => String::new(),
    }
}

/// A date as the importer reads it: `2026-11-22`, `2026-11-22 10:30`, a range as `a → b`.
/// The wall clock is kept as written; the zone offset is dropped.
fn date_text(date: &Value) -> String {
    let start = date["start"].as_str().map_or_default(moment);
    match date["end"].as_str() {
        Some(end) => format!("{start} → {}", moment(end)),
        None => start,
    }
}

fn moment(raw: &str) -> String {
    let naive = DateTime::parse_from_rfc3339(raw)
        .map(|t| t.naive_local())
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f"))
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M"));
    match naive {
        Ok(t) if t.second() == 0 => t.format("%Y-%m-%d %H:%M").to_string(),
        Ok(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        // A date without a time stays as Notion wrote it.
        Err(_) => raw.to_owned(),
    }
}

fn files_text(files: &Value) -> String {
    files
        .as_array()
        .into_iter()
        .flatten()
        .map(|file| {
            let name = file["name"].as_str().unwrap_or_default();
            let url = file["file"]["url"].as_str().or_else(|| file["external"]["url"].as_str()).unwrap_or_default();
            match (name.is_empty(), url.is_empty()) {
                (false, false) => format!("{name} ({url})"),
                (true, false) => url.to_owned(),
                _ => name.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A property's value as the text of a CSV export cell: select and status by name,
/// multi-select and people joined by `, `, checkbox as `Yes` / `No`, numbers plain, dates as
/// [`date_text`], files as `name (url)`, formulas and rollups by what they compute, relations
/// by the ids of the pages (the API does not give their titles).
fn flatten(property: &Value) -> String {
    let kind = property["type"].as_str().unwrap_or_default();
    let value = &property[kind];
    match kind {
        "title" | "rich_text" => plain_text(value),
        "number" => number_text(value),
        "select" | "status" => value["name"].as_str().unwrap_or_default().to_owned(),
        "multi_select" | "people" | "relation" => names(value),
        "date" | "created_time" | "last_edited_time" => match value {
            Value::String(time) => moment(time),
            other => date_text(other),
        },
        "checkbox" => (if value.as_bool() == Some(true) { "Yes" } else { "No" }).to_owned(),
        "url" | "email" | "phone_number" => value.as_str().unwrap_or_default().to_owned(),
        "files" => files_text(value),
        "created_by" | "last_edited_by" => name_of(value),
        "unique_id" => {
            let number = value["number"].as_u64().map_or_default(|n| n.to_string());
            match value["prefix"].as_str() {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}-{number}"),
                _ => number,
            }
        }
        "formula" => flatten_computed(value),
        "rollup" => match value["type"].as_str() {
            Some("array") => value["array"]
                .as_array()
                .into_iter()
                .flatten()
                .map(flatten)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            _ => flatten_computed(value),
        },
        // Buttons, verification and kinds Notion adds later have no text.
        _ => String::new(),
    }
}

/// The result of a formula or a scalar rollup: `{ "type": "number", "number": 3 }`.
fn flatten_computed(value: &Value) -> String {
    match value["type"].as_str().unwrap_or_default() {
        "string" => value["string"].as_str().unwrap_or_default().to_owned(),
        "number" => number_text(&value["number"]),
        "boolean" => (if value["boolean"].as_bool() == Some(true) { "Yes" } else { "No" }).to_owned(),
        "date" => date_text(&value["date"]),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(property: &Value) -> String {
        flatten(property)
    }

    #[test]
    fn text_like_properties_flatten_to_their_text() {
        assert_eq!(
            flat(&json!({"type": "title", "title": [{"plain_text": "EUR"}, {"plain_text": "USD breakout"}]})),
            "EURUSD breakout"
        );
        assert_eq!(flat(&json!({"type": "rich_text", "rich_text": []})), "");
        assert_eq!(flat(&json!({"type": "url", "url": null})), "");
        assert_eq!(flat(&json!({"type": "select", "select": {"name": "Long"}})), "Long");
        assert_eq!(flat(&json!({"type": "select", "select": null})), "");
        assert_eq!(flat(&json!({"type": "status", "status": {"name": "Done"}})), "Done");
        assert_eq!(
            flat(
                &json!({"type": "multi_select", "multi_select": [{"name": "London open"}, {"name": "Theo trend H4"}]})
            ),
            "London open, Theo trend H4"
        );
        assert_eq!(flat(&json!({"type": "people", "people": [{"id": "u1", "name": "Dat"}, {"id": "u2"}]})), "Dat, u2");
        assert_eq!(flat(&json!({"type": "unique_id", "unique_id": {"prefix": "TRD", "number": 7}})), "TRD-7");
    }

    #[test]
    fn numbers_checkboxes_and_dates_match_the_importer() {
        assert_eq!(flat(&json!({"type": "number", "number": 150})), "150");
        assert_eq!(flat(&json!({"type": "number", "number": -30.5})), "-30.5");
        assert_eq!(flat(&json!({"type": "number", "number": 1.0850})), "1.085");
        assert_eq!(flat(&json!({"type": "number", "number": null})), "");
        assert_eq!(flat(&json!({"type": "checkbox", "checkbox": true})), "Yes");
        assert_eq!(flat(&json!({"type": "checkbox", "checkbox": false})), "No");
        assert_eq!(flat(&json!({"type": "date", "date": {"start": "2026-11-16"}})), "2026-11-16");
        assert_eq!(
            flat(
                &json!({"type": "date", "date": {"start": "2026-11-16T09:00:00.000+07:00", "end": "2026-11-16T15:10:30.000+07:00"}})
            ),
            "2026-11-16 09:00 → 2026-11-16 15:10:30",
            "the wall clock stays as written"
        );
        assert_eq!(flat(&json!({"type": "date", "date": null})), "");
        assert_eq!(
            flat(&json!({"type": "created_time", "created_time": "2026-11-16T15:12:00.000Z"})),
            "2026-11-16 15:12"
        );
    }

    #[test]
    fn files_formulas_and_rollups_flatten_by_what_they_hold() {
        assert_eq!(
            flat(&json!({"type": "files", "files": [
                {"name": "chart1.png", "type": "file", "file": {"url": "https://s3/x"}},
                {"name": "", "type": "external", "external": {"url": "https://example.com/c.png"}},
            ]})),
            "chart1.png (https://s3/x), https://example.com/c.png"
        );
        assert_eq!(flat(&json!({"type": "formula", "formula": {"type": "number", "number": 2.5}})), "2.5");
        assert_eq!(flat(&json!({"type": "formula", "formula": {"type": "boolean", "boolean": true}})), "Yes");
        assert_eq!(flat(&json!({"type": "formula", "formula": {"type": "string", "string": "win"}})), "win");
        assert_eq!(flat(&json!({"type": "rollup", "rollup": {"type": "number", "number": 4}})), "4");
        assert_eq!(
            flat(&json!({"type": "rollup", "rollup": {"type": "array", "array": [
                {"type": "number", "number": 1}, {"type": "number", "number": 2}]}})),
            "1, 2"
        );
        assert_eq!(flat(&json!({"type": "button", "button": {}})), "");
    }

    #[test]
    fn the_title_property_comes_first_and_rows_are_padded() {
        let rows = vec![
            vec![("Name".to_owned(), "a".to_owned()), ("Pair".to_owned(), "EURUSD".to_owned())],
            vec![("Name".to_owned(), "b".to_owned()), ("Notes".to_owned(), "hi".to_owned())],
        ];
        let table = table_of(&rows);
        assert_eq!(table.headers, ["Name", "Pair", "Notes"]);
        assert_eq!(table.rows, [["a", "EURUSD", ""], ["b", "", "hi"]]);
        let props = json!({"A": {"type": "number"}, "Z": {"type": "title"}, "M": {"type": "date"}});
        assert_eq!(ordered_properties(&props).into_iter().map(|(n, _)| n).collect::<Vec<_>>(), ["Z", "A", "M"]);
    }
}
