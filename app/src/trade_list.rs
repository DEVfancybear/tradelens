//! The trades page's filters and sort order: pure functions over [`TradeRow`]s, so the
//! table is a view of `select(rows, filter, sort)` and nothing else.

use std::{cmp::Ordering, collections::BTreeSet};

use tradelens_core::TradeKey;

use crate::snapshot::TradeRow;

/// Winners, losers or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Outcome {
    #[default]
    All,
    Win,
    Loss,
}

/// Buy, sell or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Direction {
    #[default]
    All,
    Buy,
    Sell,
}

/// The filters of the trades page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Filter {
    /// Words looked for in the symbol, ticket, comment, setup and notes.
    pub(crate) text: String,
    pub(crate) symbol: Option<String>,
    pub(crate) setup: Option<String>,
    pub(crate) outcome: Outcome,
    pub(crate) direction: Direction,
    /// Only these trades: a row of Phân tích pressed (a tag, a session, an hour, a mistake…), whose
    /// trades the list has no filter of its own for.
    pub(crate) focus: Option<Focus>,
}

/// The trades of one row of a breakdown, by key, with what the row was called.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Focus {
    /// What the list says it shows: `Tag London open`, `Phiên Á`, `Lỗi Dời SL`.
    pub(crate) title: String,
    pub(crate) keys: BTreeSet<TradeKey>,
}

impl Filter {
    /// Whether any filter narrows the list.
    pub(crate) fn is_active(&self) -> bool {
        *self != Self::default()
    }

    pub(crate) fn matches(&self, row: &TradeRow) -> bool {
        let needle = self.text.trim().to_lowercase();
        let score = row.trade.score();
        (needle.is_empty() || row.contains(&needle))
            && self.focus.as_ref().is_none_or(|focus| focus.keys.contains(row.key()))
            && self.symbol.as_deref().is_none_or(|s| s == row.trade.symbol)
            && self.setup.as_deref().is_none_or(|s| s == row.setup)
            && match self.outcome {
                Outcome::All => true,
                Outcome::Win => row.trade.is_closed() && score > 0.0,
                Outcome::Loss => row.trade.is_closed() && score < 0.0,
            }
            && match self.direction {
                Direction::All => true,
                Direction::Buy => row.trade.side == tradelens_core::Side::Long,
                Direction::Sell => row.trade.side == tradelens_core::Side::Short,
            }
    }
}

/// A column the list can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    Time,
    Symbol,
    Lot,
    Hold,
    Net,
    R,
}

impl Column {
    /// The table column's key.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Symbol => "symbol",
            Self::Lot => "lot",
            Self::Hold => "hold",
            Self::Net => "net",
            Self::R => "r",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        [Self::Time, Self::Symbol, Self::Lot, Self::Hold, Self::Net, Self::R].into_iter().find(|c| c.key() == key)
    }
}

/// Which way a sorted column runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Ascending,
    Descending,
}

/// The column the list is ordered by and the way.
pub(crate) type Sort = (Column, Order);

fn compare(a: &TradeRow, b: &TradeRow, column: Column) -> Ordering {
    match column {
        Column::Time => a.sort_time().cmp(&b.sort_time()),
        Column::Symbol => a.trade.symbol.cmp(&b.trade.symbol),
        Column::Lot => a.trade.volume.total_cmp(&b.trade.volume),
        // An open trade has no hold time yet: it sorts as the longest.
        Column::Hold => a.trade.duration_secs().unwrap_or(i64::MAX).cmp(&b.trade.duration_secs().unwrap_or(i64::MAX)),
        Column::Net => a.trade.net().total_cmp(&b.trade.net()),
        // A trade with no R sorts below every trade that has one.
        Column::R => a.r.unwrap_or(f64::NEG_INFINITY).total_cmp(&b.r.unwrap_or(f64::NEG_INFINITY)),
    }
}

/// The indexes into `rows` of the rows that pass `filter`, in the order `sort` says
/// (the rows' own order, newest first, without one). Ties keep that order.
pub(crate) fn select(rows: &[TradeRow], filter: &Filter, sort: Option<Sort>) -> Vec<usize> {
    let mut picked: Vec<usize> = (0..rows.len()).filter(|&i| filter.matches(&rows[i])).collect();
    if let Some((column, order)) = sort {
        picked.sort_by(|&a, &b| {
            let ordering = compare(&rows[a], &rows[b], column);
            match order {
                Order::Ascending => ordering,
                Order::Descending => ordering.reverse(),
            }
        });
    }
    picked
}

/// The ticked trades after the box of the trade at `at` in `order` (the list as shown) is pressed: it
/// flips that trade, or, when `range_from` is a trade still in the list (the one pressed before, with Shift
/// held), ticks every trade from that one to this one.
pub(crate) fn toggled(
    selected: &BTreeSet<TradeKey>,
    order: &[TradeKey],
    at: usize,
    range_from: Option<&TradeKey>,
) -> BTreeSet<TradeKey> {
    let Some(key) = order.get(at) else { return selected.clone() };
    let mut next = selected.clone();
    let from = range_from.and_then(|anchor| order.iter().position(|key| key == anchor));
    if let Some(from) = from {
        next.extend(order[from.min(at)..=from.max(at)].iter().cloned());
    } else if !next.remove(key) {
        next.insert(key.clone());
    }
    next
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_press_flips_one_box_and_a_shift_press_ticks_the_range_from_the_last_one() {
        let key = |n: i64| TradeKey { account_id: 1, position_id: n, segment: 0 };
        let order: Vec<TradeKey> = (1..=6).map(key).collect();
        let none = BTreeSet::new();
        let one = toggled(&none, &order, 1, None);
        assert_eq!(one, BTreeSet::from([key(2)]));
        assert_eq!(toggled(&one, &order, 1, None), none, "pressed again, it is unticked");
        let range = toggled(&one, &order, 4, Some(&key(2)));
        assert_eq!(range, BTreeSet::from([key(2), key(3), key(4), key(5)]));
        let backwards = toggled(&none, &order, 0, Some(&key(3)));
        assert_eq!(backwards, BTreeSet::from([key(1), key(2), key(3)]), "a range counts from either side");
        assert_eq!(toggled(&none, &order, 2, Some(&key(99))), BTreeSet::from([key(3)]), "no anchor: a flip");
        assert_eq!(toggled(&one, &order, 40, None), one, "a press beyond the list is nothing");
    }

    use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, model::AccountInfo, time::parse_mt5_time};

    use super::*;
    use crate::snapshot::{Query, Range, load};

    fn deal(ticket: i64, position: i64, symbol: &str, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: symbol.into(),
            kind,
            entry,
            volume: 0.1 * f64::from(u8::try_from(position).expect("small ids")),
            price: 1.0,
            commission: 0.0,
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

    /// Positions 1 (EURUSD, +10), 2 (XAUUSD, −5), 3 (EURUSD, still open), by open time.
    fn rows() -> Vec<TradeRow> {
        use DealEntry::{In, Out};
        use DealKind::{Buy, Sell};
        let mut db = Db::in_memory().expect("opens");
        let batch = DealBatch {
            account: AccountInfo { login: "1".into(), server: "S".into(), ..AccountInfo::default() },
            deals: vec![
                deal(1, 1, "EURUSD", "2026.11.10 09:00:00", Buy, In, 0.0),
                deal(2, 1, "EURUSD", "2026.11.10 10:00:00", Sell, Out, 10.0),
                deal(3, 2, "XAUUSD", "2026.11.11 09:00:00", Sell, In, 0.0),
                deal(4, 2, "XAUUSD", "2026.11.11 13:00:00", Buy, Out, -5.0),
                deal(5, 3, "EURUSD", "2026.11.12 09:00:00", Buy, In, 0.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("stored");
        let query = Query { account: None, range: Range::All, now_utc_ms: 0, local_offset_min: 0 };
        load(&db, &query).expect("loads").rows
    }

    fn positions(rows: &[TradeRow], picked: &[usize]) -> Vec<i64> {
        picked.iter().map(|&i| rows[i].trade.key.position_id).collect()
    }

    #[test]
    fn no_filter_keeps_the_rows_order() {
        let rows = rows();
        assert_eq!(positions(&rows, &select(&rows, &Filter::default(), None)), [3, 2, 1]);
    }

    #[test]
    fn filters_combine() {
        let rows = rows();
        let only = |f: Filter| positions(&rows, &select(&rows, &f, None));
        assert_eq!(only(Filter { symbol: Some("EURUSD".into()), ..Filter::default() }), [3, 1]);
        assert_eq!(only(Filter { outcome: Outcome::Win, ..Filter::default() }), [1]);
        assert_eq!(only(Filter { outcome: Outcome::Loss, ..Filter::default() }), [2]);
        assert_eq!(only(Filter { direction: Direction::Sell, ..Filter::default() }), [2]);
        assert_eq!(only(Filter { text: "  xau ".into(), ..Filter::default() }), [2]);
        assert_eq!(
            only(Filter { text: "xau".into(), direction: Direction::Buy, ..Filter::default() }),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn an_open_trade_is_neither_a_win_nor_a_loss() {
        let rows = rows();
        let open = Filter { outcome: Outcome::Win, ..Filter::default() };
        assert!(!open.matches(&rows[0]), "position 3 is open");
        assert!(Filter::default().matches(&rows[0]));
    }

    #[test]
    fn sorting_runs_both_ways_and_is_stable() {
        let rows = rows();
        let sorted = |column, order| positions(&rows, &select(&rows, &Filter::default(), Some((column, order))));
        assert_eq!(sorted(Column::Net, Order::Descending), [1, 3, 2]);
        assert_eq!(sorted(Column::Net, Order::Ascending), [2, 3, 1]);
        assert_eq!(sorted(Column::Symbol, Order::Ascending), [3, 1, 2], "ties keep the newest first");
        assert_eq!(sorted(Column::Lot, Order::Descending), [3, 2, 1]);
        assert_eq!(sorted(Column::Hold, Order::Ascending), [1, 2, 3], "the open trade sorts as the longest");
    }

    #[test]
    fn columns_round_trip_through_their_keys() {
        for column in [Column::Time, Column::Symbol, Column::Lot, Column::Hold, Column::Net, Column::R] {
            assert_eq!(Column::from_key(column.key()), Some(column));
        }
        assert_eq!(Column::from_key("setup"), None);
    }

    #[test]
    fn an_untouched_filter_is_inactive() {
        assert!(!Filter::default().is_active());
        assert!(Filter { text: "x".into(), ..Filter::default() }.is_active());
    }
}
