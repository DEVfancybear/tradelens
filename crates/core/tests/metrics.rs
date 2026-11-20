//! Statistics over trades built from hand-made deals.

use tradelens_core::{
    metrics::{self, Sample},
    model::{Deal, DealEntry, DealKind},
    time::{Clock, parse_mt5_time},
    trades::{build_trades, cash_flows},
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
        reason: None,
    }
}

#[test]
fn statistics_and_drawdown() {
    use DealEntry::*;
    use DealKind::*;
    let mut deals = vec![deal(0, 0, "2026.11.17 08:00:00", Balance, None, 0.0, 0.0, 1000.0)];
    deals[0].commission = 0.0;
    // +100, -50, -30, +80 (gross); commissions are -0.35 per 0.1 lot per side.
    let results = [100.0, -50.0, -30.0, 80.0];
    for (i, p) in results.iter().enumerate() {
        let id = i64::try_from(i).expect("four trades") + 1;
        let day = format!("2026.11.{} 09:00:00", 18 + i);
        let close = format!("2026.11.{} 10:00:00", 18 + i);
        deals.push(deal(id * 10, id, &day, Buy, In, 0.1, 1.1, 0.0));
        deals.push(deal(id * 10 + 1, id, &close, Sell, Out, 0.1, 1.1, *p));
    }
    let trades = build_trades(1, &deals);
    let samples: Vec<Sample<'_>> = trades.iter().map(|t| Sample { trade: t, r: Option::None }).collect();
    let s = metrics::stats(&samples, 0.0, &cash_flows(&deals), Clock::default());

    assert_eq!((s.trades, s.wins, s.losses), (4, 2, 2));
    assert!((s.win_rate - 0.5).abs() < 1e-9);
    assert!((s.net - (100.0 - 50.0 - 30.0 + 80.0 - 4.0 * 0.7)).abs() < 1e-9);
    assert_eq!(s.max_loss_streak, 2);
    // Peak 1099.3 after the first trade, then two losses of 50.7 and 30.7.
    assert!((s.max_drawdown - 81.4).abs() < 1e-6);
    assert!(s.profit_factor.unwrap() > 2.0);
    assert_eq!(s.trading_days, 4);
    assert!((s.deposits - 1000.0).abs() < 1e-9);
}
