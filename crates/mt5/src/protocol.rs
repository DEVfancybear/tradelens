//! What a connection and a terminal say to each other: the account, the trader's password, a
//! poll's deals and open positions, the orders, and how a terminal refuses.

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use tradelens_core::{AccountInfo, Deal, DealBatch, Side};

/// An MT5 account as the trader names it: the login on a server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AccountRef {
    pub login: String,
    pub server: String,
}

impl AccountRef {
    /// The account `login` on `server`, both trimmed.
    pub fn new(login: &str, server: &str) -> Self {
        Self { login: login.trim().to_owned(), server: server.trim().to_owned() }
    }

    /// A name made only of letters, digits, `-` and `_`, for the account's folder.
    pub fn folder_name(&self) -> String {
        let clean = |s: &str| -> String {
            s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
        };
        format!("{}@{}", clean(&self.login), clean(&self.server))
    }
}

impl fmt::Display for AccountRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.login, self.server)
    }
}

/// The trader's MT5 password. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The password itself, to hand to the terminal.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(…)")
    }
}

/// What it takes to log in: the account, its password and the terminal to run it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub account: AccountRef,
    pub password: Password,
    /// `terminal64.exe` of the account's portable copy.
    pub terminal: PathBuf,
}

/// The way a position is held, as the worker writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Buy,
    Sell,
}

impl Direction {
    pub fn side(self) -> Side {
        match self {
            Self::Buy => Side::Long,
            Self::Sell => Side::Short,
        }
    }
}

/// A position open on the account (`positions_get`). Its ticket is the trade's position id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub ticket: i64,
    /// When it opened, in server time (as [`Deal::time_msc`]).
    #[serde(default)]
    pub time_msc: i64,
    pub symbol: String,
    pub side: Direction,
    pub volume: f64,
    #[serde(default)]
    pub open_price: f64,
    #[serde(default)]
    pub current_price: f64,
    /// The stop, 0 when there is none.
    #[serde(default)]
    pub sl: f64,
    /// The target, 0 when there is none.
    #[serde(default)]
    pub tp: f64,
    /// The floating profit, in the account's currency.
    #[serde(default)]
    pub profit: f64,
    #[serde(default)]
    pub swap: f64,
    #[serde(default)]
    pub magic: i64,
    #[serde(default)]
    pub comment: String,
}

/// A stop and a target to put on a position; `None` removes the level.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Levels {
    pub sl: Option<f64>,
    pub tp: Option<f64>,
}

/// What one poll reads: the account, the deals of the window asked for and the open positions.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Poll {
    pub account: AccountInfo,
    #[serde(default)]
    pub deals: Vec<Deal>,
    #[serde(default)]
    pub positions: Vec<Position>,
}

impl Poll {
    /// The deals as the database stores them.
    pub fn batch(&self) -> DealBatch {
        DealBatch { account: self.account.clone(), deals: self.deals.clone(), positions: Vec::new() }
    }

    /// The newest deal's time, `None` without deals.
    pub fn newest_deal(&self) -> Option<i64> {
        self.deals.iter().map(|deal| deal.time_msc).max()
    }
}

/// Why a terminal did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Mt5Error {
    /// The server refused the login or the password: trying again would not help.
    #[error("login refused: {0}")]
    LoginRefused(String),
    /// The terminal refused a request (no such position, the market is closed…).
    #[error("refused ({code}): {message}")]
    Refused { code: i64, message: String },
    /// The terminal or its worker cannot be reached: it did not start, stopped or stopped answering.
    #[error("terminal unreachable: {0}")]
    Unreachable(String),
    /// An order was sent to the terminal but no answer came: it may or may not have been carried out.
    #[error("outcome unknown: {0}")]
    Uncertain(String),
    /// Something the connection needs is missing (Python, the package, the terminal).
    #[error("not set up: {0}")]
    Setup(String),
}

impl Mt5Error {
    /// What the error says, without the kind: the worker's own words, which are Vietnamese.
    pub fn message(&self) -> &str {
        match self {
            Self::LoginRefused(message)
            | Self::Refused { message, .. }
            | Self::Unreachable(message)
            | Self::Uncertain(message)
            | Self::Setup(message) => message,
        }
    }
}

/// What a connection asks of a terminal. A terminal is used by one thread at a time; dropping it
/// logs out and stops it.
pub trait Terminal: Send {
    /// Logs in.
    ///
    /// # Errors
    ///
    /// [`Mt5Error::LoginRefused`] when the server refuses the account, others when the terminal
    /// cannot be reached.
    fn connect(&mut self, login: &Login) -> Result<AccountInfo, Mt5Error>;

    /// Reads the account, the deals since `since_msc` (server time, with a day of slack before it;
    /// `None` for the last `days` days) and the open positions.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read.
    fn poll(&mut self, since_msc: Option<i64>, days: u32) -> Result<Poll, Mt5Error>;

    /// Closes `volume` lots of the position `ticket` at market, all of it when `None`.
    ///
    /// # Errors
    ///
    /// [`Mt5Error::Refused`] when the terminal refuses the order.
    fn close(&mut self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error>;

    /// Puts `levels` on the position `ticket`.
    ///
    /// # Errors
    ///
    /// [`Mt5Error::Refused`] when the terminal refuses the change.
    fn modify(&mut self, ticket: i64, levels: Levels) -> Result<(), Mt5Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_never_shows_in_debug_output() {
        let login = Login {
            account: AccountRef::new("123", "Demo"),
            password: Password::new("s3cret"),
            terminal: PathBuf::from("terminal64.exe"),
        };
        assert!(!format!("{login:?}").contains("s3cret"));
    }

    #[test]
    fn an_account_folder_keeps_only_safe_characters() {
        assert_eq!(AccountRef::new(" 52012345 ", "ICMarkets SC-Demo/2").folder_name(), "52012345@ICMarkets_SC-Demo_2");
    }

    #[test]
    fn a_poll_reads_the_workers_reply() {
        let json = r#"{"account": {"login": "1", "server": "Demo", "server_offset_min": 120},
            "deals": [{"ticket": 5, "time_msc": 100, "type": "buy"}, {"ticket": 6, "time_msc": 300, "type": "sell"}],
            "positions": [{"ticket": 9, "symbol": "XAUUSD", "side": "sell", "volume": 0.1, "profit": -3.5}]}"#;
        let poll: Poll = serde_json::from_str(json).expect("the worker's reply parses");
        assert_eq!(poll.newest_deal(), Some(300));
        assert_eq!(poll.positions[0].side.side(), Side::Short);
        assert_eq!(poll.batch().deals.len(), 2);
    }
}
