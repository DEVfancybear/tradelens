//! Which position each deal of a report belongs to.
//!
//! The Deals table of a report has no position column, but the model needs one: a trade
//! is a position's entries and exits. Entries are matched to the Positions table (a
//! position's id is the ticket of the order that opened it), and exits are matched to the
//! positions still open on their symbol, narrowed down by the Positions table's times and
//! prices, then by volume, and last by age.

use crate::model::{Deal, DealEntry, DealKind, Side};

/// Volumes closer than this are the same volume.
const VOLUME_EPS: f64 = 1e-9;
/// Prices closer than this are the same price (the report prints at most 8 decimals).
const PRICE_EPS: f64 = 1e-7;

/// A closed position of the report's Positions table.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Position {
    pub(super) id: i64,
    pub(super) symbol: String,
    pub(super) side: Side,
    pub(super) open_ms: i64,
    pub(super) open_price: f64,
    pub(super) close_ms: i64,
    pub(super) close_price: f64,
    pub(super) sl: f64,
    pub(super) tp: f64,
}

/// A position that the deals seen so far have opened and not yet closed.
struct Held {
    id: i64,
    symbol: String,
    side: Side,
    remaining: f64,
}

/// How the account's positions behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Accounting {
    /// Every entry opens a new position.
    Hedging,
    /// One position per symbol: an entry on the side already held adds to it.
    Netting,
}

/// Sets `position_id` on every buy and sell deal. Cash deals keep their zero.
pub(super) fn assign(deals: &mut [Deal], positions: &[Position], accounting: Accounting) {
    let mut order: Vec<usize> = (0..deals.len()).collect();
    order.sort_by_key(|&i| (deals[i].time_msc, deals[i].ticket));
    let mut held: Vec<Held> = Vec::new();
    for i in order {
        let deal = &mut deals[i];
        let side = match deal.kind {
            DealKind::Buy => Side::Long,
            DealKind::Sell => Side::Short,
            _ => continue,
        };
        deal.position_id = match deal.entry {
            DealEntry::Out | DealEntry::OutBy => exit(deal, side, positions, &mut held),
            DealEntry::InOut => reverse(deal, side, positions, &mut held),
            DealEntry::In | DealEntry::None => enter(deal, side, positions, &mut held, accounting),
        };
    }
}

fn enter(deal: &Deal, side: Side, positions: &[Position], held: &mut Vec<Held>, accounting: Accounting) -> i64 {
    if accounting == Accounting::Netting
        && let Some(open) = held.iter_mut().find(|h| h.symbol == deal.symbol && h.side == side)
    {
        open.remaining += deal.volume;
        return open.id;
    }
    let same_entry = |p: &&Position| {
        p.symbol == deal.symbol
            && p.side == side
            && p.open_ms == deal.time_msc
            && (p.open_price - deal.price).abs() < PRICE_EPS
            && held.iter().all(|h| h.id != p.id)
    };
    let own_order = if deal.order != 0 { deal.order } else { deal.ticket };
    let id = positions
        .iter()
        .find(|p| p.id == deal.order)
        .or_else(|| positions.iter().find(same_entry))
        .map_or(own_order, |p| p.id);
    held.push(Held { id, symbol: deal.symbol.clone(), side, remaining: deal.volume });
    id
}

/// An exit closes (part of) a position of the other side on the same symbol.
fn exit(deal: &Deal, side: Side, positions: &[Position], held: &mut Vec<Held>) -> i64 {
    let row = |id: i64| positions.iter().find(|p| p.id == id);
    let mut open: Vec<usize> =
        (0..held.len()).filter(|&i| held[i].symbol == deal.symbol && held[i].side != side).collect();
    // Alive at the deal's time, according to the Positions table.
    narrow(&mut open, |i| row(held[i].id).is_none_or(|p| p.open_ms <= deal.time_msc && deal.time_msc <= p.close_ms));
    // Closed by this very deal, at this very price.
    narrow(&mut open, |i| {
        row(held[i].id).is_some_and(|p| p.close_ms == deal.time_msc && (p.close_price - deal.price).abs() < PRICE_EPS)
    });
    // Closed whole by this volume.
    narrow(&mut open, |i| (held[i].remaining - deal.volume).abs() < VOLUME_EPS);
    let Some(&pick) = open.first() else {
        // The entry is outside the report: the Positions table may still know the position.
        let closed_here = |p: &&Position| p.symbol == deal.symbol && p.side != side && p.close_ms == deal.time_msc;
        return positions.iter().find(closed_here).map_or(deal.order, |p| p.id);
    };
    let position = &mut held[pick];
    position.remaining -= deal.volume;
    let (id, flat) = (position.id, position.remaining <= VOLUME_EPS);
    if flat {
        held.remove(pick);
    }
    id
}

/// A netting reversal: closes the position held the other way and opens the rest on the
/// deal's side, under the same position id.
fn reverse(deal: &Deal, side: Side, positions: &[Position], held: &mut Vec<Held>) -> i64 {
    let Some(pick) = held.iter().position(|h| h.symbol == deal.symbol && h.side != side) else {
        return enter(deal, side, positions, held, Accounting::Hedging);
    };
    let position = &mut held[pick];
    let rest = deal.volume - position.remaining;
    if rest > VOLUME_EPS {
        position.side = side;
        position.remaining = rest;
    } else {
        position.remaining -= deal.volume;
    }
    let (id, flat) = (position.id, position.remaining <= VOLUME_EPS);
    if flat {
        held.remove(pick);
    }
    id
}

/// Keeps only the candidates that satisfy `keep`, unless none does.
fn narrow(candidates: &mut Vec<usize>, keep: impl Fn(usize) -> bool) {
    if candidates.iter().any(|&i| keep(i)) {
        candidates.retain(|&i| keep(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deal(ticket: i64, kind: DealKind, entry: DealEntry, at: i64, volume: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: 0,
            time_msc: at,
            symbol: "EURUSD".into(),
            kind,
            entry,
            volume,
            price: 1.1,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            magic: 0,
            comment: String::new(),
            sl: 0.0,
            tp: 0.0,
            risk: None,
            result_r: None,
            gaps: crate::model::Gaps::NONE,
            reason: None,
        }
    }

    fn ids(deals: &[Deal]) -> Vec<i64> {
        deals.iter().map(|d| d.position_id).collect()
    }

    #[test]
    fn partial_closes_stay_on_their_position() {
        use DealEntry::*;
        use DealKind::*;
        let mut deals =
            vec![deal(10, Buy, In, 1000, 0.2), deal(11, Sell, Out, 2000, 0.1), deal(12, Sell, Out, 3000, 0.1)];
        assign(&mut deals, &[], Accounting::Hedging);
        assert_eq!(ids(&deals), vec![10, 10, 10]);
    }

    #[test]
    fn hedged_positions_on_one_symbol_are_told_apart_by_the_positions_table() {
        use DealEntry::*;
        use DealKind::*;
        let row = |id, open_ms, close_ms, close_price| Position {
            id,
            symbol: "EURUSD".into(),
            side: Side::Long,
            open_ms,
            open_price: 1.1,
            close_ms,
            close_price,
            sl: 0.0,
            tp: 0.0,
        };
        // Two longs are open together; the later one is closed first, at 1.2.
        let positions = [row(20, 1000, 5000, 1.3), row(21, 2000, 3000, 1.2)];
        let mut deals = vec![
            deal(20, Buy, In, 1000, 0.1),
            deal(21, Buy, In, 2000, 0.1),
            deal(22, Sell, Out, 3000, 0.1),
            deal(23, Sell, Out, 5000, 0.1),
        ];
        deals[2].price = 1.2;
        deals[3].price = 1.3;
        assign(&mut deals, &positions, Accounting::Hedging);
        assert_eq!(ids(&deals), vec![20, 21, 21, 20]);
    }

    #[test]
    fn without_a_positions_table_the_volume_then_the_age_decide() {
        use DealEntry::*;
        use DealKind::*;
        let mut deals = vec![
            deal(30, Buy, In, 1000, 0.3),
            deal(31, Buy, In, 2000, 0.1),
            deal(32, Sell, Out, 3000, 0.1),
            deal(33, Sell, Out, 4000, 0.2),
        ];
        assign(&mut deals, &[], Accounting::Hedging);
        // 0.1 closes the 0.1 position whole; the rest goes to the older one.
        assert_eq!(ids(&deals), vec![30, 31, 31, 30]);
    }

    #[test]
    fn netting_adds_to_the_position_and_a_reversal_keeps_its_id() {
        use DealEntry::*;
        use DealKind::*;
        let mut deals = vec![
            deal(40, Buy, In, 1000, 0.1),
            deal(41, Buy, In, 2000, 0.1),
            deal(42, Sell, InOut, 3000, 0.5),
            deal(43, Buy, Out, 4000, 0.3),
        ];
        assign(&mut deals, &[], Accounting::Netting);
        assert_eq!(ids(&deals), vec![40, 40, 40, 40]);
    }

    #[test]
    fn cash_deals_have_no_position_and_an_orphan_exit_keeps_its_order() {
        use DealEntry::*;
        use DealKind::*;
        let mut deals = vec![deal(50, Balance, None, 1000, 0.0), deal(51, Sell, Out, 2000, 0.1)];
        assign(&mut deals, &[], Accounting::Hedging);
        assert_eq!(ids(&deals), vec![0, 51]);
    }
}
