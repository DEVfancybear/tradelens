//! The economic news the trader watches: the high-impact ("red") events of Forex Factory's weekly
//! calendar. This module only reads the feed; fetching it is `tradelens-cloud`'s job and storing it
//! is [`crate::db`]'s.
//!
//! The feed (`ff_calendar_thisweek.json`) lists every event of the current week, Sunday to Saturday
//! in New York time, as `{title, country, date, impact, forecast, previous}`; `date` is RFC 3339 with
//! New York's offset. It has no actual values and no other week.

use chrono::{DateTime, Datelike, Duration, FixedOffset};
use serde::Deserialize;

/// The `impact` Forex Factory gives the events it marks red.
const HIGH_IMPACT: &str = "High";

/// One high-impact event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsEvent {
    /// When it is released, in Unix milliseconds (UTC).
    pub at_ms: i64,
    /// The currency it moves, as Forex Factory names it (`USD`, `EUR`…).
    pub currency: String,
    pub title: String,
    /// The consensus as published (`0.3%`); `None` when there is none.
    pub forecast: Option<String>,
    /// The previous release as published; `None` when there is none.
    pub previous: Option<String>,
}

/// The span a feed covers, in Unix milliseconds: `start_ms` included, `end_ms` excluded. Storing a
/// feed replaces every event of its span, so an event moved or dropped within the week goes away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Week {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// What a feed holds: its week and its high-impact events, in release order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub week: Week,
    pub events: Vec<NewsEvent>,
}

/// Why a feed could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("{}", crate::tr!("core.news.bad_format", error = .0))]
    Json(#[from] serde_json::Error),
    #[error("{}", crate::tr!("core.news.bad_time", error = .0))]
    Time(String),
    #[error("{}", crate::tr!("core.news.empty"))]
    Empty,
}

/// One row of the feed, as Forex Factory writes it.
#[derive(Debug, Deserialize)]
struct Row {
    title: String,
    country: String,
    date: String,
    impact: String,
    #[serde(default)]
    forecast: String,
    #[serde(default)]
    previous: String,
}

/// Reads Forex Factory's weekly JSON feed and keeps its high-impact events.
///
/// The week runs from 00:00 of the Sunday before the earliest event, in that event's own offset
/// (New York's), for seven days; it stretches to cover a later event if a clock change moved one past it.
///
/// # Errors
///
/// [`FeedError::Json`] when the body is not the feed's JSON, [`FeedError::Time`] when an event's
/// time cannot be read, [`FeedError::Empty`] when the feed lists no event at all (a week with no
/// red news still lists others), so an empty answer never wipes a stored week.
pub fn parse_forex_factory(json: &[u8]) -> Result<Feed, FeedError> {
    let rows: Vec<Row> = serde_json::from_slice(json)?;
    let mut times = Vec::with_capacity(rows.len());
    for row in &rows {
        let at =
            DateTime::parse_from_rfc3339(row.date.trim()).map_err(|e| FeedError::Time(format!("{}: {e}", row.date)))?;
        times.push(at);
    }
    let first = times.iter().min_by_key(|at| at.timestamp_millis()).ok_or(FeedError::Empty)?;
    let start_ms = sunday_before(*first)?;
    let last_ms = times.iter().map(DateTime::timestamp_millis).max().unwrap_or(start_ms);
    let end_ms = (start_ms + Duration::days(7).num_milliseconds()).max(last_ms + 1);
    let mut events: Vec<NewsEvent> = rows
        .into_iter()
        .zip(&times)
        .filter(|(row, _)| row.impact.trim().eq_ignore_ascii_case(HIGH_IMPACT))
        .map(|(row, at)| NewsEvent {
            at_ms: at.timestamp_millis(),
            currency: row.country.trim().to_owned(),
            title: row.title.trim().to_owned(),
            forecast: filled(&row.forecast),
            previous: filled(&row.previous),
        })
        .collect();
    // The order the database reads them back in.
    events.sort_by(|a, b| (a.at_ms, &a.currency, &a.title).cmp(&(b.at_ms, &b.currency, &b.title)));
    Ok(Feed { week: Week { start_ms, end_ms }, events })
}

/// 00:00 of the Sunday on or before `at`, in `at`'s own offset, as Unix milliseconds.
fn sunday_before(at: DateTime<FixedOffset>) -> Result<i64, FeedError> {
    let day = at.date_naive() - Duration::days(i64::from(at.weekday().num_days_from_sunday()));
    day.and_hms_opt(0, 0, 0)
        .and_then(|midnight| midnight.and_local_timezone(*at.offset()).single())
        .map(|start| start.timestamp_millis())
        .ok_or_else(|| FeedError::Time(crate::tr!("core.news.no_week_start", at = at)))
}

/// `None` for an empty value, else the value trimmed.
fn filled(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    /// A week shaped like the real feed (Sunday 22 to Saturday 28 November 2026, New York time),
    /// with made-up events.
    const FEED: &str = r#"[
        {"title":"Retail Sales q/q","country":"NZD","date":"2026-11-22T16:45:00-05:00","impact":"Low","forecast":"0.5%","previous":"-0.5%"},
        {"title":"Bank Holiday","country":"JPY","date":"2026-11-22T19:00:00-05:00","impact":"Holiday","forecast":"","previous":""},
        {"title":"CPI y/y","country":"AUD","date":"2026-11-24T19:30:00-05:00","impact":"High","forecast":"4.7%","previous":"4.3%"},
        {"title":"Flash PMI","country":"EUR","date":"2026-11-23T04:00:00-05:00","impact":"Medium","forecast":"49.8","previous":"49.3"},
        {"title":"Prelim GDP q/q","country":"USD","date":"2026-11-25T08:30:00-05:00","impact":"High","forecast":"","previous":"3.4%"},
        {"title":"Core PCE Price Index m/m","country":"USD","date":"2026-11-25T08:30:00-05:00","impact":"High","forecast":" 0.3% ","previous":"0.3%"}
    ]"#;

    fn utc_ms(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).single().expect("a valid UTC time").timestamp_millis()
    }

    #[test]
    fn only_red_events_are_kept_in_release_order() {
        let feed = parse_forex_factory(FEED.as_bytes()).expect("the feed reads");
        let titles: Vec<&str> = feed.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["CPI y/y", "Core PCE Price Index m/m", "Prelim GDP q/q"]);
        let cpi = &feed.events[0];
        assert_eq!(cpi.currency, "AUD");
        assert_eq!(cpi.at_ms, utc_ms(2026, 11, 25, 0, 30), "19:30 New York is 00:30 UTC the next day");
        assert_eq!(cpi.forecast.as_deref(), Some("4.7%"));
        assert_eq!(feed.events[1].forecast.as_deref(), Some("0.3%"), "values are trimmed");
        assert_eq!(feed.events[2].forecast, None, "an empty forecast is none");
    }

    #[test]
    fn the_week_runs_from_sunday_midnight_new_york_for_seven_days() {
        let feed = parse_forex_factory(FEED.as_bytes()).expect("the feed reads");
        assert_eq!(feed.week.start_ms, utc_ms(2026, 11, 22, 5, 0), "Sunday 00:00 at UTC-5");
        assert_eq!(feed.week.end_ms, utc_ms(2026, 11, 29, 5, 0));
    }

    #[test]
    fn a_week_with_no_red_event_still_has_its_span() {
        let feed = parse_forex_factory(
            br#"[{"title":"PMI","country":"EUR","date":"2026-11-26T04:00:00-05:00","impact":"Low"}]"#,
        )
        .expect("the feed reads");
        assert_eq!(feed.events, [], "no red event");
        assert_eq!(feed.week.start_ms, utc_ms(2026, 11, 22, 5, 0), "Thursday's week starts on Sunday");
    }

    #[test]
    fn an_empty_or_broken_feed_is_an_error() {
        assert!(matches!(parse_forex_factory(b"[]"), Err(FeedError::Empty)));
        assert!(matches!(parse_forex_factory(b"<html>Request Denied</html>"), Err(FeedError::Json(_))));
        let bad_time = br#"[{"title":"x","country":"USD","date":"tomorrow","impact":"High"}]"#;
        assert!(matches!(parse_forex_factory(bad_time), Err(FeedError::Time(_))));
    }
}
