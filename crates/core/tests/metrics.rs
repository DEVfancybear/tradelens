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
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
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

#[test]
fn mistakes_are_priced_against_clean_trades() {
    use DealEntry::*;
    use DealKind::*;
    // Trades 1 and 2 carry "moved_sl", 3 is clean, 4 carries "early_exit".
    let results = [-120.0, -80.0, 150.0, 40.0];
    let mut deals = Vec::new();
    for (i, p) in results.iter().enumerate() {
        let id = i64::try_from(i).expect("four trades") + 1;
        let day = format!("2026.11.{} 09:00:00", 16 + i);
        let close = format!("2026.11.{} 10:00:00", 16 + i);
        deals.push(deal(id * 10, id, &day, Buy, In, 0.1, 1.1, 0.0));
        deals.push(deal(id * 10 + 1, id, &close, Sell, Out, 0.1, 1.1, *p));
    }
    let trades = build_trades(1, &deals);
    let samples: Vec<Sample<'_>> = trades.iter().map(|t| Sample { trade: t, r: Option::None }).collect();
    let mistakes_of = |x: &Sample<'_>| match x.trade.key.position_id {
        1 | 2 => vec!["moved_sl".to_owned()],
        4 => vec!["early_exit".to_owned()],
        _ => Vec::new(),
    };
    let costs =
        metrics::mistake_cost(&samples, mistakes_of, |k| if k == "moved_sl" { "Dời SL".into() } else { k.into() });
    assert_eq!(costs[0].label, "Dời SL");
    assert_eq!(costs[0].trades, 2);
    assert!((costs[0].net - (-200.0 - 1.4)).abs() < 1e-6, "{}", costs[0].net);
    assert!((costs[0].share_of_losses - 1.0).abs() < 1e-9, "only moved_sl trades lost");
    // Clean expectancy 149.3; moved_sl expectancy −100.7.
    assert!((costs[0].expectancy_gap + 250.0).abs() < 1e-6, "{}", costs[0].expectancy_gap);
    assert_eq!(costs[1].key, "early_exit");
}

#[test]
fn start_balance_follows_the_trader_then_the_terminal() {
    use DealEntry::*;
    use DealKind::*;
    let deals = vec![
        deal(1, 7, "2026.11.18 09:00:00", Buy, In, 0.1, 1.1, 0.0),
        deal(2, 7, "2026.11.18 10:00:00", Sell, Out, 0.1, 1.1, 100.0),
    ];
    let trades = build_trades(1, &deals);
    let samples: Vec<Sample<'_>> = trades.iter().map(|t| Sample { trade: t, r: Option::None }).collect();
    let net = samples[0].net();
    let cash = [(0, 500.0)];

    let mut account = tradelens_core::Account::default();
    assert_eq!(metrics::start_balance(&account, &samples, &cash), 0.0, "nothing known");

    account.last_balance = Some(1000.0);
    let derived = metrics::start_balance(&account, &samples, &cash);
    let curve = metrics::equity_curve(&samples, derived, &cash);
    let ended = curve.last().expect("a curve has a point").balance;
    assert!((ended - 1000.0).abs() < 1e-9, "the curve ends at the reported balance, not {ended}");
    assert!((derived - (1000.0 - net - 500.0)).abs() < 1e-9);

    account.initial_balance = Some(250.0);
    assert_eq!(metrics::start_balance(&account, &samples, &cash), 250.0, "the trader's figure wins");
}
