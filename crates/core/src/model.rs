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
    /// What the screens call the account: its name and, for a terminal's account, its login
    /// (`Demo (12345)`). The login of a journal's account (`notion`) is only a key the importer
    /// files it under, so the name stands alone (`Nhật ký Notion`).
    pub fn title(&self) -> String {
        let terminal_login = !self.login.is_empty() && self.login.chars().all(|c| c.is_ascii_digit());
        if self.name.trim().is_empty() {
            format!("{} · {}", self.login, self.server)
        } else if terminal_login {
            format!("{} ({})", self.name, self.login)
        } else {
            self.name.clone()
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

/// What a deal's source did not tell. A source that has no lot, no prices or no time of day (a
/// journal kept in R, a Notion export dated by day) still has to fill those fields of the deal with
/// something; the gaps record that what they hold is a placeholder, to be shown as a dash and left
/// out of every figure that needs it. Set by the importer that knows, never guessed from the values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Gaps(u8);

impl Gaps {
    /// Nothing is missing.
    pub const NONE: Self = Self(0);
    /// The source gave a date only: the time of day (and so the hold time) is unknown.
    pub const TIME: Self = Self(1);
    /// The source gave no lot size.
    pub const VOLUME: Self = Self(2);
    /// The source gave no price.
    pub const PRICE: Self = Self(4);

    /// Every gap there can be.
    const ALL_BITS: u8 = 7;

    /// The gaps of both `self` and `other`.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every gap of `other` is also one of `self`.
    pub const fn has(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// The stored form.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The gaps stored as `bits`; bits that mean nothing are dropped.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL_BITS)
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
    /// `ENUM_DEAL_REASON`, what triggered the deal (an exit's reason pre-fills the journal's).
    #[serde(default)]
    pub reason: Option<String>,
    /// The result of the exit in R, for a trade whose money is not known (a journal kept in R,
    /// imported from Notion): `profit` is then 0 and this is the result. `None` for a trade with money.
    #[serde(default)]
    pub result_r: Option<f64>,
    /// What the source did not tell (the fields then hold placeholders).
    #[serde(default, skip_serializing_if = "no_gaps")]
    pub gaps: Gaps,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's `skip_serializing_if` passes a reference")]
fn no_gaps(gaps: &Gaps) -> bool {
    gaps.is_none()
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

/// Where a trade came from, told by how its deals were written (their `source`, see
/// [`Origin::of_source`]), never by their tickets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    Mt5,
    Manual,
    /// Imported from a Notion journal.
    Notion,
}

impl Origin {
    /// What the screens call it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mt5 => "MT5",
            Self::Manual => "Nhập tay",
            Self::Notion => "Notion",
        }
    }

    /// The origin of deals stored with `source` (what wrote them: `manual`, `notion`, or an MT5
    /// feed or report: `ea`, `html`, `xlsx`, `json`, …).
    pub fn of_source(source: &str) -> Self {
        match source {
            "manual" => Self::Manual,
            "notion" => Self::Notion,
            _ => Self::Mt5,
        }
    }

    /// Whether the trader may change the facts of such a trade: only what they typed or
    /// imported from their own journal; what the terminal reported stays as it was reported.
    pub const fn is_editable(self) -> bool {
        !matches!(self, Self::Mt5)
    }
}

/// Tickets of imported journal trades are at most the negative of this, far from the small
/// negative tickets that hand-entered trades count down from (MetaTrader's are positive). It only
/// keeps the app's own tickets apart; what a trade is, is told by [`Origin::of_source`].
pub const JOURNAL_TICKET_BASE: i64 = 1 << 40;

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
    /// The result in R of a trade whose money is not known (see [`Deal::result_r`]).
    pub result_r: Option<f64>,
    pub manual: bool,
    /// What the deals of the trade did not tell (see [`Gaps`]).
    pub gaps: Gaps,
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

    /// Whether the trade's money is known. A trade imported with only its result in R has no
    /// money: statistics count it by its R, and money figures show a dash for it. A trade that has
    /// a result in R and also a profit (a result the trader gave in R although the money is known)
    /// keeps its money.
    pub fn money_known(&self) -> bool {
        self.result_r.is_none() || self.profit != 0.0
    }

    /// Whether the lot size is the source's own (not a placeholder).
    pub fn volume_known(&self) -> bool {
        !self.gaps.has(Gaps::VOLUME)
    }

    /// Whether the entry and exit prices are the source's own. Without them the stop, the target
    /// and the risk at the stop mean nothing either.
    pub fn prices_known(&self) -> bool {
        !self.gaps.has(Gaps::PRICE)
    }

    /// Whether the time of day of the open and the close is known (the source gave more than a date).
    pub fn time_known(&self) -> bool {
        !self.gaps.has(Gaps::TIME)
    }

    /// How long the position was held, when both ends are known times.
    pub fn hold_secs(&self) -> Option<i64> {
        if self.time_known() { self.duration_secs() } else { None }
    }

    /// What decides whether the trade won or lost: its money, or its R when it has no money.
    pub fn score(&self) -> f64 {
        self.result_r.unwrap_or_else(|| self.net())
    }

    /// The trade's result in R: money over the money at risk when known, else the
    /// price move over the stop distance.
    pub fn r_multiple(&self, planned_risk: Option<f64>) -> Option<f64> {
        if let Some(r) = self.result_r {
            return Some(r);
        }
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
    /// The setup from the playbook ([`crate::playbook::Playbook`]).
    #[serde(default)]
    pub playbook_id: Option<i64>,
    /// A free-text setup, for trades journalled before the playbook (or imported).
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
    /// The execution grade: 1 to 5 stars; 0 when not rated.
    #[serde(default)]
    pub rating: u8,
    /// The setup's quality, a key of the user's grade catalog (default `A+`, `A`, `B`, `C`),
    /// graded apart from the execution: a good setup can be executed badly and the reverse.
    #[serde(default)]
    pub setup_grade: Option<String>,
    /// Why the trade ended, a key of the user's exit-reason catalog (`tp`, `sl`, `manual`, …);
    /// pre-filled from the exit deal's reason when the EA sends it.
    #[serde(default)]
    pub exit_reason: Option<String>,
    /// What the trader planned when the position opened, before knowing the result.
    #[serde(default)]
    pub plan: Option<crate::journal::EntryPlan>,
    /// The market's state, a key of the user's market-condition catalog.
    #[serde(default)]
    pub market: Option<String>,
    /// The trader wants to look at this trade again (the review queue).
    #[serde(default)]
    pub review_later: bool,
    /// Values of the trader's own fields (scope `trade`), by field key.
    #[serde(default)]
    pub custom: crate::catalog::CustomValues,
    #[serde(default)]
    pub followed_plan: Option<bool>,
    /// The checklist items the trader confirmed before entering.
    #[serde(default)]
    pub checklist: Vec<i64>,
    #[serde(default)]
    pub notes: String,
    /// Chart images, several per trade, in the trader's order.
    #[serde(default)]
    pub screenshots: Vec<crate::attachments::Attachment>,
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

/// A trade the trader typed, or imported from their journal, as an entry and an exit deal. What
/// the trader did not give is `None`: it is stored as a placeholder and recorded as a [`Gaps`] bit,
/// so that it shows as a dash and no figure counts it.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualTrade {
    pub symbol: String,
    pub side: Side,
    /// The lot size; `None` when not given.
    pub volume: Option<f64>,
    /// Server time, milliseconds (see [`Deal::time_msc`]).
    pub open_time: i64,
    pub open_price: Option<f64>,
    pub close_time: i64,
    pub close_price: Option<f64>,
    /// The times are days only: the time of day is not known (the times then hold midnight).
    pub date_only: bool,
    /// The stop loss; 0 for none.
    pub sl: f64,
    /// The take profit; 0 for none.
    pub tp: f64,
    /// The profit before commission and swap; `None` when the money of the trade is not known, in
    /// which case `result_r` says how it went.
    pub profit: Option<f64>,
    /// Costs only count when the money is known (`profit` is given).
    pub commission: f64,
    pub swap: f64,
    /// The result in R when the trader gives one: it stands in for what the stop would say, or is
    /// the only result of a trade kept without money.
    pub result_r: Option<f64>,
}

impl ManualTrade {
    /// The result in R the prices and the stop say (the move over the distance to the stop), as a
    /// trade without a risk amount is measured; `None` without both prices or a stop.
    pub fn computed_r(&self) -> Option<f64> {
        let (open, close) = (self.open_price?, self.close_price?);
        let stop = (open - self.sl).abs();
        if self.sl <= 0.0 || stop < f64::EPSILON {
            return None;
        }
        let moved = match self.side {
            Side::Long => close - open,
            Side::Short => open - close,
        };
        Some(moved / stop)
    }

    /// What the trade does not tell, as the deals record it.
    pub fn gaps(&self) -> Gaps {
        let mut gaps = Gaps::NONE;
        if self.volume.is_none() {
            gaps = gaps.with(Gaps::VOLUME);
        }
        if self.open_price.is_none() || self.close_price.is_none() {
            gaps = gaps.with(Gaps::PRICE);
        }
        if self.date_only {
            gaps = gaps.with(Gaps::TIME);
        }
        gaps
    }
}
