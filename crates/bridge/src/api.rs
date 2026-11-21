//! The bridge's HTTP contract, shared by the server and its tests.

use serde::{Deserialize, Serialize};

pub const TOKEN_HEADER: &str = "X-TradeLens-Token";

pub const PING: &str = "/api/v1/ping";
pub const DEALS: &str = "/api/v1/deals";
pub const ACCOUNT: &str = "/api/v1/account";
pub const SCREENSHOT: &str = "/api/v1/screenshot";

/// Headers of `POST /api/v1/screenshot`, whose body is the PNG itself.
pub const LOGIN_HEADER: &str = "X-TradeLens-Login";
pub const SERVER_HEADER: &str = "X-TradeLens-Server";
pub const POSITION_HEADER: &str = "X-TradeLens-Position";
pub const MOMENT_HEADER: &str = "X-TradeLens-Moment";

/// The largest screenshot accepted, in bytes.
pub const MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;

/// When the EA took a screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Moment {
    /// Just after the position opened.
    Entry,
    /// Just after it closed.
    Exit,
}

impl Moment {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "entry" => Some(Self::Entry),
            "exit" => Some(Self::Exit),
            _ => None,
        }
    }
}

/// The answer to `POST /api/v1/screenshot`: where the image was kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotStored {
    /// Relative to the data folder's `attachments/`.
    pub path: String,
}

/// The attachment file name of a screenshot: `<login>/<position>-<moment>.png`, so a
/// resent screenshot replaces the earlier one.
pub fn screenshot_path(login: &str, position_id: i64, moment: Moment) -> String {
    let login: String = login.chars().filter(char::is_ascii_alphanumeric).collect();
    let moment = match moment {
        Moment::Entry => "entry",
        Moment::Exit => "exit",
    };
    format!("{login}/{position_id}-{moment}.png")
}

/// `GET /api/v1/ping`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    pub app: String,
    pub version: String,
}

/// The answer to `POST /api/v1/deals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DealsAccepted {
    pub account_id: i64,
    pub inserted: usize,
    pub updated: usize,
}

/// The answer to `POST /api/v1/account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAccepted {
    pub account_id: i64,
}

/// Any error: `{"error": "..."}` with a 4xx/5xx status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_paths_are_safe_and_stable() {
        assert_eq!(screenshot_path("52012345", 61_234_567, Moment::Entry), "52012345/61234567-entry.png");
        assert_eq!(screenshot_path("../../x", 1, Moment::Exit), "x/1-exit.png");
        assert_eq!(Moment::parse(" EXIT "), Some(Moment::Exit));
    }
}
