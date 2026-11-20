//! The domain types: MT5 accounts and deals as the terminal reports them, the trades
//! built from them, and the journal the trader keeps on each trade.

use serde::{Deserialize, Serialize};

/// A trading account, one per MT5 login on one server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Account {
    pub id: i64,
    pub login: String,
    pub server: String,
    pub broker: String,
    pub name: String,
    pub currency: String,
    /// The balance before the first trade, set by the trader. Without it, the
    /// starting balance comes from deposits or from the last reported balance.
    pub initial_balance: Option<f64>,
    pub last_balance: Option<f64>,
    pub last_equity: Option<f64>,
    /// The broker server's offset from UTC in minutes (MT5 times are server times).
    pub server_offset_min: Option<i32>,
    pub last_sync: Option<i64>,
}

impl Account {
    pub fn title(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{} · {}", self.login, self.server)
        } else {
            format!("{} ({})", self.name, self.login)
        }
    }
}

/// What the terminal knows of an account when it sends deals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AccountInfo {
    pub login: String,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub broker: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub balance: Option<f64>,
    #[serde(default)]
    pub equity: Option<f64>,
    #[serde(default)]
    pub server_offset_min: Option<i32>,
}

/// `ENUM_DEAL_TYPE`, the deal kinds the journal distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DealKind {
    Buy,
    Sell,
    Balance,
    Credit,
    Charge,
    Correction,
    Bonus,
    Commission,
    Other,
}

impl DealKind {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "buy" | "0" | "deal_type_buy" => Self::Buy,
            "sell" | "1" | "deal_type_sell" => Self::Sell,
            "balance" | "2" | "deal_type_balance" => Self::Balance,
            "credit" | "3" | "deal_type_credit" => Self::Credit,
            "charge" | "4" | "deal_type_charge" => Self::Charge,
            "correction" | "5" | "deal_type_correction" => Self::Correction,
            "bonus" | "6" | "deal_type_bonus" => Self::Bonus,
            s if s.starts_with("commission") || s.starts_with("deal_type_commission") => Self::Commission,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
            Self::Balance => "balance",
            Self::Credit => "credit",
            Self::Charge => "charge",
            Self::Correction => "correction",
            Self::Bonus => "bonus",
            Self::Commission => "commission",
            Self::Other => "other",
        }
    }

    /// Money moved into or out of the account, not a trade.
    pub fn is_cash(self) -> bool {
        matches!(self, Self::Balance | Self::Credit | Self::Charge | Self::Correction | Self::Bonus)
    }

    pub fn is_trade(self) -> bool {
        matches!(self, Self::Buy | Self::Sell)
    }
}

/// `ENUM_DEAL_ENTRY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DealEntry {
    In,
    Out,
    InOut,
    OutBy,
    None,
}

impl DealEntry {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().replace(' ', "").as_str() {
            "in" | "0" | "deal_entry_in" => Self::In,
            "out" | "1" | "deal_entry_out" => Self::Out,
            "inout" | "in/out" | "2" | "deal_entry_inout" => Self::InOut,
            "outby" | "out_by" | "3" | "deal_entry_out_by" => Self::OutBy,
            _ => Self::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::InOut => "inout",
            Self::OutBy => "out_by",
            Self::None => "",
        }
    }
}

/// One MT5 deal (`HistoryDealGet*`), the unit every source delivers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deal {
    pub ticket: i64,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub position_id: i64,
    /// Server time, milliseconds since 1970 as if the server's clock were UTC.
    pub time_msc: i64,
    #[serde(default)]
    pub symbol: String,
    #[serde(rename = "type", with = "kind_serde")]
    pub kind: DealKind,
    #[serde(with = "entry_serde", default = "entry_none")]
    pub entry: DealEntry,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub swap: f64,
    #[serde(default)]
    pub fee: f64,
    #[serde(default)]
    pub profit: f64,
    #[serde(default)]
    pub magic: i64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
    /// Money at risk at the stop, computed by the EA from the symbol's tick value.
    #[serde(default)]
    pub risk: Option<f64>,
}

fn entry_none() -> DealEntry {
    DealEntry::None
}

mod kind_serde {
    use super::DealKind;
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's `with` passes a reference")]
    pub(super) fn serialize<S: Serializer>(k: &DealKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(k.as_str())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DealKind, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(match v {
            serde_json::Value::Number(n) => DealKind::parse(&n.to_string()),
            serde_json::Value::String(s) => DealKind::parse(&s),
            _ => DealKind::Other,
        })
    }
}

mod entry_serde {
    use super::DealEntry;
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's `with` passes a reference")]
    pub(super) fn serialize<S: Serializer>(e: &DealEntry, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(e.as_str())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DealEntry, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(match v {
            serde_json::Value::Number(n) => DealEntry::parse(&n.to_string()),
            serde_json::Value::String(s) => DealEntry::parse(&s),
            _ => DealEntry::None,
        })
    }
}

/// What a sync or an import delivers: one account and its deals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DealBatch {
    pub account: AccountInfo,
    #[serde(default)]
    pub deals: Vec<Deal>,
    /// Stops and targets by position, from a report's positions table.
    #[serde(default)]
    pub positions: Vec<PositionLevels>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PositionLevels {
    pub position_id: i64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Self::Long => "Buy",
            Self::Short => "Sell",
        }
    }
}

/// Identifies a trade across rebuilds, and the journal entry kept on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TradeKey {
    pub account_id: i64,
    pub position_id: i64,
    /// A netting position reversed by an in/out deal becomes two trades.
    pub segment: u32,
}

impl TradeKey {
    pub fn id(&self) -> String {
        format!("{}:{}:{}", self.account_id, self.position_id, self.segment)
    }

    pub fn parse(id: &str) -> Option<Self> {
        let mut parts = id.split(':');
        let key = Self {
            account_id: parts.next()?.parse().ok()?,
            position_id: parts.next()?.parse().ok()?,
            segment: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(key)
    }
}

/// A position from its first entry to its last exit.
#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    pub key: TradeKey,
    pub symbol: String,
    pub side: Side,
    pub open_time: i64,
    pub close_time: Option<i64>,
    /// The largest volume held.
    pub volume: f64,
    pub open_price: f64,
    pub close_price: Option<f64>,
    pub sl: f64,
    pub tp: f64,
    pub commission: f64,
    pub swap: f64,
    pub fee: f64,
    pub profit: f64,
    /// Money at risk at the stop when the EA could compute it.
    pub risk: Option<f64>,
    pub magic: i64,
    pub comment: String,
    pub manual: bool,
}

impl Trade {
    pub fn is_closed(&self) -> bool {
        self.close_time.is_some()
    }

    /// Profit after commission, swap and fees.
    pub fn net(&self) -> f64 {
        self.profit + self.commission + self.swap + self.fee
    }

    pub fn duration_secs(&self) -> Option<i64> {
        self.close_time.map(|c| (c - self.open_time) / 1000)
    }

    /// The trade's result in R: money over the money at risk when known, else the
    /// price move over the stop distance.
    pub fn r_multiple(&self, planned_risk: Option<f64>) -> Option<f64> {
        let risk = planned_risk.filter(|r| *r > 0.0).or(self.risk.filter(|r| *r > 0.0));
        if let Some(risk) = risk {
            return Some(self.net() / risk);
        }
        let close = self.close_price?;
        if self.sl <= 0.0 {
            return None;
        }
        let stop = (self.open_price - self.sl).abs();
        if stop < f64::EPSILON {
            return None;
        }
        let moved = match self.side {
            Side::Long => close - self.open_price,
            Side::Short => self.open_price - close,
        };
        Some(moved / stop)
    }
}

/// What the trader writes about one trade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct JournalEntry {
    #[serde(default)]
    pub setup: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mistakes: Vec<String>,
    #[serde(default)]
    pub emotion_before: String,
    #[serde(default)]
    pub emotion_after: String,
    /// 1 to 5 stars for the execution; 0 when not rated.
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub followed_plan: Option<bool>,
    /// The checklist items the trader confirmed before entering.
    #[serde(default)]
    pub checklist: Vec<i64>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub planned_risk: Option<f64>,
}

impl JournalEntry {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// An account's discipline rules; each is off when `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Rules {
    pub max_daily_loss: Option<f64>,
    pub max_daily_trades: Option<u32>,
    pub max_risk_pct: Option<f64>,
    pub max_consecutive_losses: Option<u32>,
    pub daily_target: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: i64,
    pub text: String,
}

/// The day's plan and review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DayNote {
    pub plan: String,
    pub review: String,
    pub mood: String,
}

/// A trade added by hand, stored as an entry and an exit deal.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualTrade {
    pub symbol: String,
    pub side: Side,
    pub volume: f64,
    pub open_time: i64,
    pub open_price: f64,
    pub close_time: i64,
    pub close_price: f64,
    pub sl: f64,
    pub tp: f64,
    pub profit: f64,
    pub commission: f64,
    pub swap: f64,
}
