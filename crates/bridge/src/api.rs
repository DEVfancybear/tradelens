//! The bridge's HTTP contract, shared by the server and its tests.

use serde::{Deserialize, Serialize};

pub const TOKEN_HEADER: &str = "X-TradeLens-Token";

pub const PING: &str = "/api/v1/ping";
pub const DEALS: &str = "/api/v1/deals";
pub const ACCOUNT: &str = "/api/v1/account";

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

/// Any error: `{"error": "..."}` with a 4xx/5xx status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
