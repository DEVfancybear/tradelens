//! The account whose MT5 report the fixtures hold: what happened, deal by deal.
//!
//! `gen_fixtures.rs` renders it into the fixture files and `import.rs` checks what the
//! importers read back against it, so the two cannot drift apart.

/// A deal as the Deals table shows it, plus the position it belongs to.
pub(crate) struct Event {
    pub(crate) at: &'static str,
    pub(crate) ticket: i64,
    pub(crate) order: i64,
    pub(crate) symbol: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) entry: &'static str,
    pub(crate) volume: f64,
    pub(crate) price: f64,
    pub(crate) commission: f64,
    pub(crate) fee: f64,
    pub(crate) swap: f64,
    pub(crate) profit: f64,
    pub(crate) comment: &'static str,
    /// The position the deal belongs to (0 for cash).
    pub(crate) position: i64,
}

/// A closed position as the Positions table shows it.
pub(crate) struct Closed {
    pub(crate) position: i64,
    pub(crate) symbol: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) volume: f64,
    pub(crate) opened_at: &'static str,
    pub(crate) open_price: f64,
    pub(crate) sl: f64,
    pub(crate) tp: f64,
    pub(crate) ended_at: &'static str,
    pub(crate) close_price: f64,
    pub(crate) commission: f64,
    pub(crate) swap: f64,
    pub(crate) profit: f64,
}

pub(crate) const NAME: &str = "Nguyen Van A";
pub(crate) const LOGIN: &str = "52012345";
pub(crate) const CURRENCY: &str = "USD";
pub(crate) const SERVER: &str = "ICMarketsSC-Demo";
pub(crate) const COMPANY: &str = "Raw Trading Ltd";
pub(crate) const REPORT_DATE: &str = "2026.11.20 23:59";

/// Profit, commission, fee and swap of every trade, as the report's summary prints it.
pub(crate) const TOTAL_NET_PROFIT: f64 = 238.85;
/// The Balance column after the last deal.
pub(crate) const FINAL_BALANCE: f64 = 9738.85;
/// The report's "Total Trades": the closed positions.
pub(crate) const CLOSED_TRADES: usize = 7;

const BASE: Event = Event {
    at: "",
    ticket: 0,
    order: 0,
    symbol: "",
    kind: "",
    entry: "",
    volume: 0.0,
    price: 0.0,
    commission: 0.0,
    fee: 0.0,
    swap: 0.0,
    profit: 0.0,
    comment: "",
    position: 0,
};

/// Seventeen deals, oldest first.
///
/// - A: EURUSD long 0.20, closed in two halves (a partial close), the last by its target.
/// - B: XAUUSD short, closed by its stop.
/// - C: GBPUSD long held two nights (swap), closed by its target, with a fee.
/// - D, E: two EURUSD longs opened together on a hedging account; E is closed first.
/// - F, G: XAUUSD long and short closed against each other ("out by").
pub(crate) const EVENTS: [Event; 17] = [
    Event {
        at: "2026.11.02 00:00:00",
        ticket: 81_000_001,
        kind: "balance",
        profit: 10_000.0,
        comment: "Deposit",
        ..BASE
    },
    Event {
        at: "2026.11.16 09:00:12",
        ticket: 81_000_002,
        order: 91_000_001,
        symbol: "EURUSD",
        kind: "buy",
        entry: "in",
        volume: 0.20,
        price: 1.08500,
        commission: -0.70,
        position: 91_000_001,
        ..BASE
    },
    Event {
        at: "2026.11.16 11:30:45",
        ticket: 81_000_003,
        order: 91_000_002,
        symbol: "EURUSD",
        kind: "sell",
        entry: "out",
        volume: 0.10,
        price: 1.09000,
        commission: -0.35,
        profit: 50.0,
        position: 91_000_001,
        ..BASE
    },
    Event {
        at: "2026.11.16 15:10:03",
        ticket: 81_000_004,
        order: 91_000_003,
        symbol: "EURUSD",
        kind: "sell",
        entry: "out",
        volume: 0.10,
        price: 1.09500,
        commission: -0.35,
        profit: 100.0,
        comment: "[tp 1.09500]",
        position: 91_000_001,
        ..BASE
    },
    Event {
        at: "2026.11.17 08:15:30",
        ticket: 81_000_005,
        order: 91_000_004,
        symbol: "XAUUSD",
        kind: "sell",
        entry: "in",
        volume: 0.10,
        price: 2650.25,
        commission: -0.35,
        position: 91_000_004,
        ..BASE
    },
    Event {
        at: "2026.11.17 10:42:11",
        ticket: 81_000_006,
        order: 91_000_005,
        symbol: "XAUUSD",
        kind: "buy",
        entry: "out",
        volume: 0.10,
        price: 2660.25,
        commission: -0.35,
        profit: -100.0,
        comment: "[sl 2660.25]",
        position: 91_000_004,
        ..BASE
    },
    Event {
        at: "2026.11.17 22:50:00",
        ticket: 81_000_007,
        order: 91_000_006,
        symbol: "GBPUSD",
        kind: "buy",
        entry: "in",
        volume: 0.50,
        price: 1.26000,
        commission: -1.75,
        position: 91_000_006,
        ..BASE
    },
    Event {
        at: "2026.11.18 09:00:00",
        ticket: 81_000_008,
        order: 91_000_007,
        symbol: "EURUSD",
        kind: "buy",
        entry: "in",
        volume: 0.10,
        price: 1.09200,
        commission: -0.35,
        position: 91_000_007,
        ..BASE
    },
    Event {
        at: "2026.11.18 09:20:00",
        ticket: 81_000_009,
        order: 91_000_008,
        symbol: "EURUSD",
        kind: "buy",
        entry: "in",
        volume: 0.10,
        price: 1.09300,
        commission: -0.35,
        position: 91_000_008,
        ..BASE
    },
    Event {
        at: "2026.11.18 13:00:00",
        ticket: 81_000_010,
        order: 91_000_009,
        symbol: "EURUSD",
        kind: "sell",
        entry: "out",
        volume: 0.10,
        price: 1.09600,
        commission: -0.35,
        profit: 30.0,
        position: 91_000_008,
        ..BASE
    },
    Event {
        at: "2026.11.18 16:00:00",
        ticket: 81_000_011,
        order: 91_000_010,
        symbol: "EURUSD",
        kind: "sell",
        entry: "out",
        volume: 0.10,
        price: 1.09100,
        commission: -0.35,
        profit: -10.0,
        position: 91_000_007,
        ..BASE
    },
    Event {
        at: "2026.11.19 10:00:00",
        ticket: 81_000_012,
        order: 91_000_011,
        symbol: "XAUUSD",
        kind: "buy",
        entry: "in",
        volume: 0.10,
        price: 2640.00,
        commission: -0.35,
        position: 91_000_011,
        ..BASE
    },
    Event {
        at: "2026.11.19 10:30:00",
        ticket: 81_000_013,
        order: 91_000_012,
        symbol: "XAUUSD",
        kind: "sell",
        entry: "in",
        volume: 0.10,
        price: 2638.00,
        commission: -0.35,
        position: 91_000_012,
        ..BASE
    },
    Event {
        at: "2026.11.19 11:00:00",
        ticket: 81_000_014,
        order: 91_000_013,
        symbol: "XAUUSD",
        kind: "sell",
        entry: "out by",
        volume: 0.10,
        price: 2638.00,
        profit: -20.0,
        position: 91_000_011,
        ..BASE
    },
    Event {
        at: "2026.11.19 11:00:00",
        ticket: 81_000_015,
        order: 91_000_013,
        symbol: "XAUUSD",
        kind: "buy",
        entry: "out by",
        volume: 0.10,
        price: 2640.00,
        position: 91_000_012,
        ..BASE
    },
    Event {
        at: "2026.11.19 13:20:44",
        ticket: 81_000_016,
        order: 91_000_014,
        symbol: "GBPUSD",
        kind: "sell",
        entry: "out",
        volume: 0.50,
        price: 1.26400,
        commission: -1.75,
        fee: -0.25,
        swap: -3.20,
        profit: 200.0,
        comment: "[tp 1.26400]",
        position: 91_000_006,
    },
    Event {
        at: "2026.11.19 18:00:00",
        ticket: 81_000_017,
        kind: "balance",
        profit: -500.0,
        comment: "Withdrawal",
        ..BASE
    },
];

const CLOSED_BASE: Closed = Closed {
    position: 0,
    symbol: "",
    kind: "",
    volume: 0.0,
    opened_at: "",
    open_price: 0.0,
    sl: 0.0,
    tp: 0.0,
    ended_at: "",
    close_price: 0.0,
    commission: 0.0,
    swap: 0.0,
    profit: 0.0,
};

/// The seven closed positions, in the order the report lists them (by close time).
pub(crate) const CLOSED: [Closed; 7] = [
    Closed {
        position: 91_000_001,
        symbol: "EURUSD",
        kind: "buy",
        volume: 0.20,
        opened_at: "2026.11.16 09:00:12",
        open_price: 1.08500,
        sl: 1.08000,
        tp: 1.09500,
        ended_at: "2026.11.16 15:10:03",
        close_price: 1.09500,
        commission: -1.40,
        profit: 150.0,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_004,
        symbol: "XAUUSD",
        kind: "sell",
        volume: 0.10,
        opened_at: "2026.11.17 08:15:30",
        open_price: 2650.25,
        sl: 2660.25,
        tp: 2630.00,
        ended_at: "2026.11.17 10:42:11",
        close_price: 2660.25,
        commission: -0.70,
        profit: -100.0,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_008,
        symbol: "EURUSD",
        kind: "buy",
        volume: 0.10,
        opened_at: "2026.11.18 09:20:00",
        open_price: 1.09300,
        sl: 1.09000,
        tp: 1.09700,
        ended_at: "2026.11.18 13:00:00",
        close_price: 1.09600,
        commission: -0.70,
        profit: 30.0,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_007,
        symbol: "EURUSD",
        kind: "buy",
        volume: 0.10,
        opened_at: "2026.11.18 09:00:00",
        open_price: 1.09200,
        sl: 1.08800,
        tp: 1.09900,
        ended_at: "2026.11.18 16:00:00",
        close_price: 1.09100,
        commission: -0.70,
        profit: -10.0,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_011,
        symbol: "XAUUSD",
        kind: "buy",
        volume: 0.10,
        opened_at: "2026.11.19 10:00:00",
        open_price: 2640.00,
        ended_at: "2026.11.19 11:00:00",
        close_price: 2638.00,
        commission: -0.35,
        profit: -20.0,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_012,
        symbol: "XAUUSD",
        kind: "sell",
        volume: 0.10,
        opened_at: "2026.11.19 10:30:00",
        open_price: 2638.00,
        ended_at: "2026.11.19 11:00:00",
        close_price: 2640.00,
        commission: -0.35,
        ..CLOSED_BASE
    },
    Closed {
        position: 91_000_006,
        symbol: "GBPUSD",
        kind: "buy",
        volume: 0.50,
        opened_at: "2026.11.17 22:50:00",
        open_price: 1.26000,
        sl: 1.25500,
        tp: 1.26400,
        ended_at: "2026.11.19 13:20:44",
        close_price: 1.26400,
        commission: -3.50,
        swap: -3.20,
        profit: 200.0,
    },
];
