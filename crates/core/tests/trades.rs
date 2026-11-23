//! Deals → trades → statistics, on hand-made deals.

use tradelens_core::{
    model::{Deal, DealEntry, DealKind, Side},
    time::parse_mt5_time,
    trades::build_trades,
};

#[expect(clippy::too_many_arguments, reason = "a fixture that spells out one deal")]
fn deal(
    ticket: i64,
    position: i64,
    at: &str,
    kind: DealKind,
    entry: DealEntry,
    volume: f64,
    price: f64,
    profit: f64,
) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: parse_mt5_time(at).expect("fixture times parse"),
        symbol: "EURUSD".into(),
        kind,
        entry,
        volume,
        price,
        commission: -0.35 * volume * 10.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

#[test]
fn partial_closes_make_one_trade_with_average_exit() {
    use DealEntry::*;
    use DealKind::*;
    let deals = vec![
        deal(1, 100, "2026.11.18 09:00:00", Buy, In, 0.2, 1.1000, 0.0),
        deal(2, 100, "2026.11.18 10:00:00", Sell, Out, 0.1, 1.1020, 20.0),
        deal(3, 100, "2026.11.18 11:00:00", Sell, Out, 0.1, 1.1040, 40.0),
    ];
    let trades = build_trades(1, &deals);
    assert_eq!(trades.len(), 1);
    let t = &trades[0];
    assert_eq!(t.side, Side::Long);
    assert!(t.is_closed());
    assert!((t.volume - 0.2).abs() < 1e-9);
    assert!((t.close_price.unwrap() - 1.1030).abs() < 1e-9);
    assert!((t.profit - 60.0).abs() < 1e-9);
}

#[test]
fn a_netting_reversal_makes_two_trades() {
    use DealEntry::*;
    use DealKind::*;
    let deals = vec![
        deal(1, 7, "2026.11.18 09:00:00", Buy, In, 0.1, 1.1000, 0.0),
        deal(2, 7, "2026.11.18 10:00:00", Sell, InOut, 0.3, 1.0990, -10.0),
        deal(3, 7, "2026.11.18 12:00:00", Buy, Out, 0.2, 1.0970, 40.0),
    ];
    let trades = build_trades(1, &deals);
    assert_eq!(trades.len(), 2);
    assert_eq!((trades[0].side, trades[0].key.segment), (Side::Long, 0));
    assert_eq!((trades[1].side, trades[1].key.segment), (Side::Short, 1));
    assert!((trades[1].volume - 0.2).abs() < 1e-9);
    assert!(trades[1].is_closed());
}
