//! Building trades from deals: a position's entries and exits, by `position_id`.

use std::collections::BTreeMap;

use crate::model::{Deal, DealEntry, DealKind, Side, Trade, TradeKey};

const EPS: f64 = 1e-9;

struct Building {
    trade: Trade,
    open_volume: f64,
    entry_value: f64,
    entry_volume: f64,
    exit_value: f64,
    exit_volume: f64,
}

impl Building {
    fn start(account_id: i64, position_id: i64, segment: u32, deal: &Deal, volume: f64) -> Self {
        let side = if deal.kind == DealKind::Buy { Side::Long } else { Side::Short };
        Self {
            trade: Trade {
                key: TradeKey { account_id, position_id, segment },
                symbol: deal.symbol.clone(),
                side,
                open_time: deal.time_msc,
                close_time: None,
                volume: 0.0,
                open_price: deal.price,
                close_price: None,
                sl: deal.sl,
                tp: deal.tp,
                commission: 0.0,
                swap: 0.0,
                fee: 0.0,
                profit: 0.0,
                risk: None,
                magic: deal.magic,
                comment: deal.comment.clone(),
                manual: deal.ticket < 0,
            },
            open_volume: 0.0,
            entry_value: 0.0,
            entry_volume: 0.0,
            exit_value: 0.0,
            exit_volume: 0.0,
        }
        .entered(deal, volume, true)
    }

    fn entered(mut self, deal: &Deal, volume: f64, costs: bool) -> Self {
        self.open_volume += volume;
        self.entry_value += deal.price * volume;
        self.entry_volume += volume;
        self.trade.volume = self.trade.volume.max(self.open_volume);
        self.trade.open_price = self.entry_value / self.entry_volume.max(EPS);
        if costs {
            self.add_costs(deal);
        }
        if let Some(risk) = deal.risk.filter(|r| *r > 0.0) {
            self.trade.risk = Some(self.trade.risk.unwrap_or(0.0) + risk);
        }
        if deal.sl > 0.0 {
            self.trade.sl = deal.sl;
        }
        if deal.tp > 0.0 {
            self.trade.tp = deal.tp;
        }
        self
    }

    fn add_costs(&mut self, deal: &Deal) {
        self.trade.commission += deal.commission;
        self.trade.swap += deal.swap;
        self.trade.fee += deal.fee;
    }

    fn exited(&mut self, deal: &Deal, volume: f64, profit: f64) {
        self.open_volume = (self.open_volume - volume).max(0.0);
        self.exit_value += deal.price * volume;
        self.exit_volume += volume;
        self.trade.profit += profit;
        self.trade.close_price = Some(self.exit_value / self.exit_volume.max(EPS));
        if self.open_volume <= EPS {
            self.trade.close_time = Some(deal.time_msc);
        }
        if deal.sl > 0.0 {
            self.trade.sl = deal.sl;
        }
        if deal.tp > 0.0 {
            self.trade.tp = deal.tp;
        }
    }

    fn is_flat(&self) -> bool {
        self.open_volume <= EPS
    }
}

/// The trades of one account's deals, oldest first. Cash deals are ignored.
/// A position whose exits do not cover its entries stays open.
pub fn build_trades(account_id: i64, deals: &[Deal]) -> Vec<Trade> {
    let mut by_position: BTreeMap<i64, Vec<&Deal>> = BTreeMap::new();
    for deal in deals.iter().filter(|d| d.kind.is_trade()) {
        // A deal without a position (some old reports) is its own position.
        let position = if deal.position_id != 0 { deal.position_id } else { deal.ticket };
        by_position.entry(position).or_default().push(deal);
    }

    let mut trades = Vec::new();
    for (position_id, mut list) in by_position {
        list.sort_by_key(|d| (d.time_msc, d.ticket));
        let mut segment = 0_u32;
        let mut current: Option<Building> = None;
        for deal in list {
            match deal.entry {
                DealEntry::In | DealEntry::None => {
                    current = Some(match current.take() {
                        Some(b) => b.entered(deal, deal.volume, true),
                        None => Building::start(account_id, position_id, segment, deal, deal.volume),
                    });
                }
                DealEntry::Out | DealEntry::OutBy => {
                    let Some(b) = current.as_mut() else {
                        // An exit whose entry is outside the imported range.
                        continue;
                    };
                    b.add_costs(deal);
                    b.exited(deal, deal.volume, deal.profit);
                    if b.is_flat() {
                        trades.push(current.take().expect("present").trade);
                        segment += 1;
                    }
                }
                DealEntry::InOut => {
                    // Netting reversal: closes what is open, opens the rest the other way.
                    match current.take() {
                        Some(mut b) => {
                            let closing = b.open_volume.min(deal.volume);
                            b.add_costs(deal);
                            b.exited(deal, closing, deal.profit);
                            b.open_volume = 0.0;
                            b.trade.close_time = Some(deal.time_msc);
                            trades.push(b.trade);
                            segment += 1;
                            let rest = deal.volume - closing;
                            if rest > EPS {
                                current = Some(Building::start(account_id, position_id, segment, deal, rest));
                                // Costs were booked on the closed part.
                                if let Some(b) = current.as_mut() {
                                    b.trade.commission = 0.0;
                                    b.trade.swap = 0.0;
                                    b.trade.fee = 0.0;
                                }
                            }
                        }
                        None => {
                            current = Some(Building::start(account_id, position_id, segment, deal, deal.volume));
                        }
                    }
                }
            }
        }
        if let Some(b) = current {
            trades.push(b.trade);
        }
    }
    trades.sort_by_key(|t| (t.open_time, t.key.position_id, t.key.segment));
    trades
}

/// Deposits, withdrawals and other cash movements, oldest first: (time, amount).
pub fn cash_flows(deals: &[Deal]) -> Vec<(i64, f64)> {
    let mut flows: Vec<(i64, f64)> =
        deals.iter().filter(|d| d.kind.is_cash()).map(|d| (d.time_msc, d.profit)).collect();
    flows.sort_by_key(|f| f.0);
    flows
}
