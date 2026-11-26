//! The stored news: the high-impact events of each week fetched, and how the last fetch went.

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{Db, Result};
use crate::news::{Feed, NewsEvent};

/// How the fetches of the news have gone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewsFetch {
    /// When a feed was last stored, Unix milliseconds; `None` before the first.
    pub fetched_ms: Option<i64>,
    /// The last failure, when it is newer than the last success: when, and why in Vietnamese.
    pub failure: Option<(i64, String)>,
}

impl Db {
    /// Stores a fetched feed: its week's events replace those stored for that week, and the fetch
    /// is recorded as the last success at `now_ms`.
    ///
    /// # Errors
    ///
    /// [`super::DbError::Sqlite`] when the database fails; nothing is changed then.
    pub fn store_news(&mut self, feed: &Feed, now_ms: i64) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM news_events WHERE at_ms >= ?1 AND at_ms < ?2",
            params![feed.week.start_ms, feed.week.end_ms],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO news_events (at_ms, currency, title, forecast, previous)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for event in &feed.events {
                insert.execute(params![event.at_ms, event.currency, event.title, event.forecast, event.previous])?;
            }
        }
        tx.execute(
            "INSERT INTO news_fetch (id, fetched_ms, error, error_ms) VALUES (1, ?1, NULL, NULL)
             ON CONFLICT(id) DO UPDATE SET fetched_ms = excluded.fetched_ms, error = NULL, error_ms = NULL",
            [now_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records that a fetch failed at `now_ms` with `message`; the stored events stay.
    ///
    /// # Errors
    ///
    /// [`super::DbError::Sqlite`] when the database fails.
    pub fn record_news_failure(&mut self, message: &str, now_ms: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO news_fetch (id, error, error_ms) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET error = excluded.error, error_ms = excluded.error_ms",
            params![message, now_ms],
        )?;
        Ok(())
    }

    /// How the fetches have gone.
    ///
    /// # Errors
    ///
    /// [`super::DbError::Sqlite`] when the query fails.
    pub fn news_fetch(&self) -> Result<NewsFetch> {
        let row: Option<(Option<i64>, Option<String>, Option<i64>)> = self
            .conn
            .query_row("SELECT fetched_ms, error, error_ms FROM news_fetch WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((fetched_ms, error, error_ms)) = row else { return Ok(NewsFetch::default()) };
        let failure = error.zip(error_ms).map(|(message, at)| (at, message));
        Ok(NewsFetch { fetched_ms, failure })
    }

    /// The stored events released in `[from_ms, to_ms)`, in release order.
    ///
    /// # Errors
    ///
    /// [`super::DbError::Sqlite`] when the query fails.
    pub fn news_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<NewsEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT at_ms, currency, title, forecast, previous FROM news_events
             WHERE at_ms >= ?1 AND at_ms < ?2 ORDER BY at_ms, currency, title",
        )?;
        let rows = stmt.query_map([from_ms, to_ms], |r| {
            Ok(NewsEvent {
                at_ms: r.get(0)?,
                currency: r.get(1)?,
                title: r.get(2)?,
                forecast: r.get(3)?,
                previous: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::news::Week;

    use super::*;

    fn event(at_ms: i64, title: &str) -> NewsEvent {
        NewsEvent { at_ms, currency: "USD".into(), title: title.into(), forecast: Some("0.3%".into()), previous: None }
    }

    fn feed(start_ms: i64, events: Vec<NewsEvent>) -> Feed {
        Feed { week: Week { start_ms, end_ms: start_ms + 100 }, events }
    }

    #[test]
    fn a_fetch_replaces_its_week_and_keeps_the_others() {
        let mut db = Db::in_memory().expect("a memory database opens");
        db.store_news(&feed(0, vec![event(10, "CPI"), event(20, "GDP")]), 1).expect("stores");
        db.store_news(&feed(100, vec![event(150, "NFP")]), 2).expect("stores");
        db.store_news(&feed(0, vec![event(30, "CPI")]), 3).expect("stores again");
        let titles: Vec<(i64, String)> =
            db.news_between(0, 1_000).expect("reads").into_iter().map(|e| (e.at_ms, e.title)).collect();
        assert_eq!(titles, [(30, "CPI".to_owned()), (150, "NFP".to_owned())], "the moved CPI and the dropped GDP");
        assert_eq!(db.news_between(0, 100).expect("reads")[0].forecast.as_deref(), Some("0.3%"));
    }

    #[test]
    fn a_failure_is_shown_until_the_next_success() {
        let mut db = Db::in_memory().expect("a memory database opens");
        assert_eq!(db.news_fetch().expect("reads"), NewsFetch::default());
        db.record_news_failure("mất mạng", 5).expect("records");
        assert_eq!(
            db.news_fetch().expect("reads"),
            NewsFetch { fetched_ms: None, failure: Some((5, "mất mạng".into())) }
        );
        db.store_news(&feed(0, vec![event(10, "CPI")]), 7).expect("stores");
        db.record_news_failure("hết giờ", 9).expect("records");
        let fetch = db.news_fetch().expect("reads");
        assert_eq!(fetch.fetched_ms, Some(7), "the last success stays");
        assert_eq!(fetch.failure, Some((9, "hết giờ".into())));
        db.store_news(&feed(0, vec![event(10, "CPI")]), 11).expect("stores");
        assert_eq!(db.news_fetch().expect("reads").failure, None);
        assert_eq!(db.news_between(0, 100).expect("reads").len(), 1, "the events stayed through the failure");
    }
}
