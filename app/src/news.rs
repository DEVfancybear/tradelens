//! The red news, from the app's side: Forex Factory's week is fetched on a blocking thread when the
//! app opens (unless the last fetch is newer than [`EVERY`]), every [`EVERY`] after that, and when
//! the trader presses "Tải lại". A feed is stored, a failure is recorded next to the last feed, and
//! the pages read again.
//!
//! Nothing here touches a signal off the UI thread: the fetch runs in `blocking`, the write through
//! the [`Writer`], both started under the root's owner so that leaving the page cancels nothing.

use std::time::Duration;

use chrono::Utc;
use nsh_core::prelude::*;
use tradelens_cloud::{
    CloudError,
    news::{FOREX_FACTORY_WEEK, fetch_week},
};
use tradelens_core::{Db, DbError};

use crate::store::{Store, Writer, store, writer};

/// Another address for the feed (a test server), or `off` to fetch nothing. The variable
/// `TRADELENS_NEWS_URL` does the same when the setting is not there.
pub const URL_SETTING: &str = "news.url";
/// The value of [`URL_SETTING`] that turns fetching off.
pub const OFF: &str = "off";

/// How often the feed is fetched while the app stays open; Forex Factory revises a week's
/// forecasts and times now and then, and asking more often only loads their server.
pub(crate) const EVERY: Duration = Duration::from_mins(30);

/// Whether a fetch goes ahead when one was made a moment ago.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Due {
    /// Only when the last stored feed is older than [`EVERY`] (the app opening).
    IfStale,
    /// Whatever the last fetch (the timer, the button).
    Now,
}

/// The handle to the news fetches, made once by [`provide`].
#[derive(Clone, Copy)]
pub(crate) struct News {
    /// A fetch is under way.
    busy: Signal<bool>,
    /// The root's owner: a fetch outlives the page that asked for it.
    owner: Owner,
    store: Store,
    write: Writer,
}

impl std::fmt::Debug for News {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("News").finish_non_exhaustive()
    }
}

/// Makes the news handle for everything under the caller and starts the fetches. Call it once, in
/// the root.
pub(crate) fn provide() {
    let news = News { busy: signal(false), owner: Owner::current(), store: store(), write: writer() };
    provide_context(news);
    effect(move || {
        if news.store.is_ready() {
            untracked(|| news.fetch(Due::IfStale));
        }
    });
    set_interval(EVERY, move || news.fetch(Due::Now));
}

/// The news handle; read while building, never in a listener.
pub(crate) fn news() -> News {
    use_context::<News>().expect("the root provides the news")
}

impl News {
    /// Whether a fetch is under way (tracked).
    pub(crate) fn busy(&self) -> bool {
        self.busy.get()
    }

    /// Fetches the week now, unless a fetch is already under way.
    pub(crate) fn refresh(self) {
        self.fetch(Due::Now);
    }

    fn fetch(self, due: Due) {
        self.owner.run(move || {
            if self.busy.get_untracked() {
                return;
            }
            self.busy.set(true);
            let now_ms = Utc::now().timestamp_millis();
            self.store.read(
                move |db| address(db, due, now_ms),
                move |read| match read {
                    Ok(Some(url)) => self.download(url),
                    Ok(None) => self.busy.set(false),
                    Err(error) => {
                        log::warn!("news: {error}");
                        self.busy.set(false);
                    }
                },
            );
        });
    }

    fn download(self, url: String) {
        spawn(async move {
            let fetched = blocking(move || {
                fetch_week(&url).map_err(|error| {
                    log::warn!("news: {error}");
                    explain(&error)
                })
            })
            .await
            .unwrap_or_else(|error| Err(format!("luồng tải lịch tin bị dừng: {error}")));
            let at_ms = Utc::now().timestamp_millis();
            self.write.run(
                move |db| match &fetched {
                    Ok(feed) => db.store_news(feed, at_ms),
                    Err(message) => db.record_news_failure(message, at_ms),
                },
                move |written| {
                    if let Err(error) = written {
                        log::error!("news: {error}");
                    }
                    self.busy.set(false);
                },
            );
        });
    }
}

/// Why a fetch failed, for the trader: what to do about it rather than the operating system's words.
fn explain(error: &CloudError) -> String {
    match error {
        CloudError::Network(_) => "không kết nối được máy chủ lịch tin, hãy kiểm tra mạng".to_owned(),
        CloudError::Service { status, .. } => format!("máy chủ lịch tin trả lỗi {status}"),
        other => other.to_string(),
    }
}

/// Where to fetch the feed from, or `None` when nothing is to be fetched: fetching is off, or
/// `due` is [`Due::IfStale`] and a feed was stored less than [`EVERY`] before `now_ms`.
fn address(db: &Db, due: Due, now_ms: i64) -> Result<Option<String>, DbError> {
    let chosen = db.setting(URL_SETTING)?.or_else(|| std::env::var("TRADELENS_NEWS_URL").ok());
    let url = match chosen.as_deref().map(str::trim) {
        Some(off) if off.eq_ignore_ascii_case(OFF) => return Ok(None),
        Some(url) if !url.is_empty() => url.to_owned(),
        _ => match default_address(db) {
            Some(url) => url.to_owned(),
            None => return Ok(None),
        },
    };
    if due == Due::IfStale {
        let every_ms = i64::try_from(EVERY.as_millis()).expect("half an hour fits in i64 milliseconds");
        if db.news_fetch()?.fetched_ms.is_some_and(|at| now_ms - at < every_ms) {
            return Ok(None);
        }
    }
    Ok(Some(url))
}

/// The feed fetched when nothing names another: Forex Factory's. Tests never reach the network: the
/// unit tests fetch nothing, nor does an app keeping its data in memory (no data folder, as the
/// headless shell tests run it); the screen tests turn fetching off or name a local feed.
fn default_address(db: &Db) -> Option<&'static str> {
    (!cfg!(test) && db.path().is_some()).then_some(FOREX_FACTORY_WEEK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_chooses_the_address_or_turns_fetching_off() {
        let mut db = Db::in_memory().expect("a memory database opens");
        db.set_setting(URL_SETTING, " http://127.0.0.1:9/feed.json ").expect("stored");
        assert_eq!(address(&db, Due::Now, 0).expect("reads").as_deref(), Some("http://127.0.0.1:9/feed.json"));
        db.set_setting(URL_SETTING, "OFF").expect("stored");
        assert_eq!(address(&db, Due::Now, 0).expect("reads"), None);
    }

    #[test]
    fn a_failure_is_told_in_the_traders_words() {
        let refused = CloudError::Network("io: No connection could be made (os error 10061)".into());
        assert_eq!(explain(&refused), "không kết nối được máy chủ lịch tin, hãy kiểm tra mạng");
        let denied = CloudError::Service { status: 403, message: "Request Denied".into() };
        assert_eq!(explain(&denied), "máy chủ lịch tin trả lỗi 403");
    }

    #[test]
    fn opening_the_app_does_not_fetch_again_a_feed_fetched_a_moment_ago() {
        let mut db = Db::in_memory().expect("a memory database opens");
        db.set_setting(URL_SETTING, "http://127.0.0.1:9/feed.json").expect("stored");
        let feed = tradelens_core::news::Feed {
            week: tradelens_core::news::Week { start_ms: 0, end_ms: 1 },
            events: Vec::new(),
        };
        let fetched_ms = 1_000_000;
        db.store_news(&feed, fetched_ms).expect("stored");
        let soon = fetched_ms + 60_000;
        assert_eq!(address(&db, Due::IfStale, soon).expect("reads"), None);
        assert!(address(&db, Due::Now, soon).expect("reads").is_some(), "the button always fetches");
        let later = fetched_ms + 31 * 60_000;
        assert!(address(&db, Due::IfStale, later).expect("reads").is_some(), "half an hour later it is stale");
    }
}
