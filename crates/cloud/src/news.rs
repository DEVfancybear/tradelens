//! Fetching the week's economic calendar from Forex Factory's public feed. One GET, no key, no
//! account; reading the feed is the core's ([`tradelens_core::news`]).

use std::time::Duration;

use tradelens_core::news::{self, Feed};

use crate::http::{Http, Request};
use crate::{CloudError, RetryPolicy};

/// The feed of the current week (Sunday to Saturday, New York time).
pub const FOREX_FACTORY_WEEK: &str = "https://nfs.faireconomy.media/ff_calendar_thisweek.json";

/// Two tries a few seconds apart: the app fetches again on its own later, so a longer wait helps
/// no one.
const RETRY: RetryPolicy =
    RetryPolicy { max_attempts: 2, base_delay: Duration::from_secs(2), max_delay: Duration::from_secs(5) };

/// Fetches the feed at `url` (normally [`FOREX_FACTORY_WEEK`]) and reads it.
///
/// Blocks on the network: call it from a background thread.
///
/// # Errors
///
/// [`CloudError::Network`] when the address cannot be reached, [`CloudError::Service`] when it
/// answers with an error status, [`CloudError::News`] when the answer is not the feed (Forex
/// Factory answers a page of HTML to a client it refuses).
pub fn fetch_week(url: &str) -> crate::Result<Feed> {
    let response = Http::new(RETRY).send(&Request::get(url).header("Accept", "application/json"))?.ok()?;
    news::parse_forex_factory(&response.body).map_err(CloudError::News)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// Serves `body` with `status` to one request, and returns the address to ask.
    fn serve_once(status: u16, body: &'static str) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("a local port opens");
        let url = format!("http://{}/ff_calendar_thisweek.json", server.server_addr());
        thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let _answered = request.respond(tiny_http::Response::from_string(body).with_status_code(status));
            }
        });
        url
    }

    #[test]
    fn the_feed_is_fetched_and_read() {
        let url = serve_once(
            200,
            r#"[{"title":"CPI m/m","country":"USD","date":"2026-11-25T08:30:00-05:00","impact":"High","forecast":"0.3%","previous":"0.2%"}]"#,
        );
        let feed = fetch_week(&url).expect("the feed is fetched");
        assert_eq!(feed.events.len(), 1);
        assert_eq!(feed.events[0].title, "CPI m/m");
    }

    #[test]
    fn a_refusal_page_is_an_error_not_an_empty_week() {
        let url = serve_once(200, "<html><body>Request Denied</body></html>");
        assert!(matches!(fetch_week(&url), Err(CloudError::News(_))));
    }

    #[test]
    fn an_error_status_is_reported() {
        let url = serve_once(404, "not found");
        assert!(matches!(fetch_week(&url), Err(CloudError::Service { status: 404, .. })));
    }
}
