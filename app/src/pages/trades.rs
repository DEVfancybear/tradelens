//! Lệnh (Trades): the list of trades grouped from MT5 deals, filtered, each opening its journal.
//! Design: `docs/design/screens/trades.html`.

use std::collections::BTreeSet;

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Origin, TradeKey,
    metrics::{self, Sample},
    time::format_duration,
};

use super::{account_picker::account_picker, manual_entry, trade_detail};
use crate::i18n::plural;
use crate::{
    format::{self, Sign},
    nav,
    snapshot::{Snapshot, TradeRow},
    store::{Load, store, writer},
    trade_list::{self, Column, Direction, Filter, Order, Outcome},
    ui::{
        chips::{Tone, pnl_text},
        controls::{
            Choice, Variant, action, action_icon, confirm_then, root_owner, segmented, select_field, text_field,
        },
        icons::Icon,
        parts::page_fill,
        table::{NOT_JOURNALED, Sorting, TableSort, TradeCells, journaled, trades_table},
    },
};

/// The list, or the page of the trade opened from it.
pub(crate) fn trades() -> impl IntoElement {
    // Each is keyed, so that opening or closing a trade's page starts at the top of what shows.
    let screen = |key: String, page: AnyElement| view().flex_1().min_h(px(0.0)).key(key).child(page);
    dynamic(move || match nav::open_trade() {
        Some(key) if nav::edit_trade().as_ref() == Some(&key) => {
            let id = format!("trade:{key:?}:edit");
            screen(id, untracked(|| manual_entry::edit_trade(key)).into_element())
        }
        Some(key) => {
            let id = format!("trade:{key:?}");
            screen(id, untracked(|| trade_detail::trade_detail(key)).into_element())
        }
        None if nav::manual_entry_open() => {
            screen("manual-entry".to_owned(), untracked(manual_entry::manual_entry).into_element())
        }
        None => screen("list".to_owned(), untracked(list).into_element()),
    })
}

/// The cells of the table for one trade.
pub(crate) fn cells_of(row: &TradeRow) -> TradeCells {
    let trade = &row.trade;
    let closed = trade.is_closed();
    TradeCells {
        // The list gives its rows a box (`select_cell`); the overview's table has none.
        select: String::new(),
        time: row.closed().map_or_else(
            || t!("format.open"),
            |at| if trade.time_known() { format::short_stamp(at) } else { format::short_day(at) },
        ),
        symbol: trade.symbol.clone(),
        side: trade.side.label().to_owned(),
        lot: if trade.volume_known() { format::lots(trade.volume) } else { "—".to_owned() },
        hold: trade.hold_secs().map_or_else(|| "—".to_owned(), format_duration),
        // A trade imported in R has no money: a dash, never "$0.00".
        net: if closed && trade.money_known() {
            format::money(trade.net(), Sign::Always, &row.currency)
        } else {
            "—".to_owned()
        },
        r: if closed { format::r_text(row.r) } else { "—".to_owned() },
        setup: row.setup.clone(),
        journal: if row.journaled { journaled() } else { NOT_JOURNALED.to_owned() },
    }
}

/// The numbers over the list: what the filtered trades add up to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Summary {
    pub(crate) trades: usize,
    pub(crate) net: String,
    pub(crate) win_rate: String,
    pub(crate) profit_factor: String,
    pub(crate) unjournaled: usize,
}

/// Adds up the rows at `picked` (indexes into the snapshot's rows).
pub(crate) fn summarise(snapshot: &Snapshot, picked: &[usize]) -> Summary {
    let rows: Vec<&TradeRow> = picked.iter().filter_map(|&i| snapshot.rows.get(i)).collect();
    let samples: Vec<Sample<'_>> =
        rows.iter().filter(|row| row.trade.is_closed()).map(|row| Sample { trade: &row.trade, r: row.r }).collect();
    let stats = metrics::stats(&samples, 0.0, &[], snapshot.zone.clock());
    let only_r = metrics::Basis::of(&samples).unit() == metrics::Unit::R;
    Summary {
        trades: rows.len(),
        // Only trades imported in R in the list: there is no money to add up.
        net: if only_r { "—".to_owned() } else { format::money(stats.net, Sign::Always, &snapshot.currency) },
        win_rate: if samples.is_empty() { "—".to_owned() } else { format::percent(stats.win_rate) },
        profit_factor: format::ratio(if only_r { stats.profit_factor_r } else { stats.profit_factor }),
        unjournaled: rows.iter().filter(|row| !row.journaled).count(),
    }
}

const OUTCOMES: &[Outcome] = &[Outcome::All, Outcome::Win, Outcome::Loss];
const DIRECTIONS: &[Direction] = &[Direction::All, Direction::Buy, Direction::Sell];
const ANY: &str = "";

fn outcome_label(outcome: Outcome) -> String {
    match outcome {
        Outcome::All => t!("common.all"),
        Outcome::Win => t!("trades.outcome.win"),
        Outcome::Loss => t!("trades.outcome.loss"),
    }
}

fn direction_label(direction: Direction) -> String {
    match direction {
        Direction::All => t!("common.all"),
        Direction::Buy => "Buy".to_owned(),
        Direction::Sell => "Sell".to_owned(),
    }
}

/// A choice list of `values` with "All" first.
fn any_or(values: &[String]) -> Vec<Choice> {
    let mut choices = vec![Choice::new(ANY, t!("common.all"))];
    choices.extend(values.iter().map(|v| Choice::new(v.clone(), v.clone())));
    choices
}

fn list() -> impl IntoElement {
    let store = store();
    let (filter, sort) = (store.trade_filter, store.trade_sort);
    let picked = computed(move || trade_list::select(&store.snapshot().rows, &filter.get(), sort.get()));
    // The trades ticked for deleting, and the one a Shift-press counts from.
    let selected = signal(BTreeSet::<TradeKey>::new());
    let anchor = signal(None::<TradeKey>);
    let rows = move || {
        let snapshot = store.snapshot();
        picked.with(|picked| {
            picked
                .iter()
                .filter_map(|&i| snapshot.rows.get(i))
                .map(|row| TradeCells { select: selected.with(|ticked| select_cell(row, ticked)), ..cells_of(row) })
                .collect()
        })
    };
    // The ticked trades that are in the list now: a filter that hides one takes it out of the count, so
    // that nothing the trader cannot see is deleted.
    let chosen = computed(move || {
        let snapshot = store.snapshot();
        picked.with(|picked| {
            selected.with(|selected| {
                picked
                    .iter()
                    .filter_map(|&i| snapshot.rows.get(i))
                    .filter(|row| selected.contains(row.key()))
                    .map(|row| (row.key().clone(), row.trade.symbol.clone(), row.origin))
                    .collect::<Vec<_>>()
            })
        })
    });
    let select = move |at: usize, shift: bool| {
        let snapshot = store.snapshot_now();
        let order: Vec<TradeKey> = picked.with_untracked(|picked| {
            picked.iter().filter_map(|&i| snapshot.rows.get(i)).map(|r| r.key().clone()).collect()
        });
        let range_from = anchor.get_untracked().filter(|_| shift);
        let next = trade_list::toggled(&selected.get_untracked(), &order, at, range_from.as_ref());
        selected.set(next);
        anchor.set(order.get(at).cloned());
    };
    let summary = computed(move || summarise(&store.snapshot(), &picked.get()));
    let open = move |at: usize| {
        let key = picked
            .with_untracked(|picked| picked.get(at).copied())
            .and_then(|i| store.snapshot_now().rows.get(i).map(|row| row.key().clone()));
        if let Some(key) = key {
            nav::show_trade(key);
        }
    };
    let empty = move || match store.load_state() {
        Load::Loading => (t!("trades.empty.loading"), String::new()),
        Load::Failed(error) => (error, t!("trades.empty.failed_hint")),
        Load::Ready if filter.get().is_active() && !store.snapshot().rows.is_empty() => {
            (t!("trades.empty.no_match"), t!("trades.empty.no_match_hint"))
        }
        Load::Ready => (t!("trades.empty.none"), t!("trades.empty.none_hint")),
    };

    // Ctrl+N adds a trade by hand, as the button does.
    on_key_with("mod+n", KeyOptions { pass_through: true, ..KeyOptions::default() }, |_| nav::open_manual_entry());
    page_fill(
        t!("trades.title"),
        t!("trades.summary"),
        view().flex_row().items_center().gap_3().child(account_picker()).child(action_icon(
            Icon::Plus,
            t!("trades.add"),
            Variant::Primary,
            nav::open_manual_entry,
        )),
        view()
            .flex_1()
            .gap_4()
            .child(filters(store))
            .child(summary_strip(summary))
            .child(trades_table(
                rows,
                Sorting::new(
                    move || {
                        sort.get().map(|(column, order)| TableSort {
                            key: column.key().into(),
                            ascending: order == Order::Ascending,
                        })
                    },
                    move |next| {
                        sort.set(next.and_then(|s| {
                            Column::from_key(&s.key)
                                .map(|c| (c, if s.ascending { Order::Ascending } else { Order::Descending }))
                        }));
                    },
                ),
                open,
                select,
                empty,
            ))
            .child(footer(picked, chosen, selected)),
    )
}

/// The box cell of a trade: ticked or not (`1` / `0`), and what screen readers call it.
fn select_cell(row: &TradeRow, ticked: &BTreeSet<TradeKey>) -> String {
    let when = row.closed().map_or_else(|| t!("format.open"), format::short_stamp);
    let name = t!("trades.select_row", symbol = row.trade.symbol, when = when);
    format!("{}|{name}", u8::from(ticked.contains(row.key())))
}

fn filters(store: crate::store::Store) -> impl IntoElement {
    let filter = store.trade_filter;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(view().flex_1().min_w(px(240.0)).child(text_field(
            t!("trades.filter.search"),
            Some("⌕"),
            t!("trades.filter.search_placeholder"),
            move || filter.with(|f| f.text.clone()),
            move |text| filter.update(|f| text.clone_into(&mut f.text)),
        )))
        .child(select_field(
            "Symbol",
            move || any_or(&store.snapshot().symbols),
            move || SharedString::from(filter.with(|f| f.symbol.clone().unwrap_or_default())),
            move |value| filter.update(|f| f.symbol = Some(value.to_string()).filter(|v| !v.is_empty())),
        ))
        .child(select_field(
            "Setup",
            move || any_or(&store.snapshot().setups),
            move || SharedString::from(filter.with(|f| f.setup.clone().unwrap_or_default())),
            move |value| filter.update(|f| f.setup = Some(value.to_string()).filter(|v| !v.is_empty())),
        ))
        .child(segmented(
            t!("trades.filter.outcome"),
            OUTCOMES,
            outcome_label,
            move || filter.with(|f| f.outcome),
            move |outcome| filter.update(|f| f.outcome = outcome),
        ))
        .child(segmented(
            t!("trades.filter.direction"),
            DIRECTIONS,
            direction_label,
            move || filter.with(|f| f.direction),
            move |direction| filter.update(|f| f.direction = direction),
        ))
        .child(dynamic(move || {
            filter.with(Filter::is_active).then(|| {
                action(t!("trades.filter.clear"), Variant::Ghost, move || {
                    filter.set(Filter::default());
                })
            })
        }))
}

/// A figure with its caption above it, in the strip over the table.
fn figure(
    caption: String,
    value: impl Fn() -> String + 'static,
    tone: impl Fn() -> Tone + 'static,
) -> impl IntoElement {
    view().gap_1().child(text(caption).text_xs().text_color(move || theme().muted_foreground)).child(
        text(value).font_family(crate::ui::theme::mono).tabular_nums().font_weight(FontWeight::SEMIBOLD).text_color(
            move || match tone() {
                Tone::Profit => crate::ui::theme::profit(),
                Tone::Loss => crate::ui::theme::loss(),
                Tone::Neutral | Tone::Primary => theme().foreground,
            },
        ),
    )
}

fn summary_strip(summary: Memo<Summary>) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_8()
        .child(figure(t!("trades.figure.trades"), move || summary.with(|s| s.trades.to_string()), || Tone::Neutral))
        .child(
            view()
                .gap_1()
                .child(text("Net").text_xs().text_color(move || theme().muted_foreground))
                .child(pnl_text(move || summary.with(|s| s.net.clone()))),
        )
        .child(figure("Win rate".to_owned(), move || summary.with(|s| s.win_rate.clone()), || Tone::Neutral))
        .child(figure("PF".to_owned(), move || summary.with(|s| s.profit_factor.clone()), || Tone::Neutral))
        .child(figure(
            t!("trades.figure.unjournaled"),
            move || summary.with(|s| s.unjournaled.to_string()),
            || Tone::Neutral,
        ))
}

/// What is under the list: how many trades show, and the bar of what the trader ticked.
fn footer(
    picked: Memo<Vec<usize>>,
    chosen: Memo<Vec<(TradeKey, String, Origin)>>,
    selected: Signal<BTreeSet<TradeKey>>,
) -> impl IntoElement {
    let store = store();
    let failure = signal(None::<String>);
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    text(move || {
                        let shown = picked.with(Vec::len);
                        let all = store.snapshot().rows.len();
                        plural!(all, "trades.showing", shown = shown, all = all)
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(dynamic(move || {
                    let ticked = chosen.with(Vec::len);
                    (ticked > 0).then(|| selection_bar(ticked, chosen, selected, failure))
                })),
        )
        .child(dynamic(move || failure.get().map(|error| text(error).text_xs().text_color(crate::ui::theme::loss))))
}

/// "N trades selected · Delete": what the ticked trades can be asked to do. One question covers all of them.
fn selection_bar(
    ticked: usize,
    chosen: Memo<Vec<(TradeKey, String, Origin)>>,
    selected: Signal<BTreeSet<TradeKey>>,
    failure: Signal<Option<String>>,
) -> impl IntoElement {
    let (asker, write) = (root_owner(), writer());
    let remove = move || {
        let trades = chosen.get_untracked();
        let symbol = trades.first().map_or_default(|(_, symbol, _)| symbol.clone());
        let origin =
            if trades.iter().any(|(_, _, origin)| *origin == Origin::Mt5) { Origin::Mt5 } else { Origin::Manual };
        let (title, said) = trade_detail::delete_question(trades.len(), &symbol, origin);
        let keys: Vec<TradeKey> = trades.into_iter().map(|(key, _, _)| key).collect();
        confirm_then(asker, title, said, t!("trades.delete_confirm"), move || {
            failure.try_set(None);
            trade_detail::delete_trades(write, keys, move |result| match result {
                Ok(()) => {
                    selected.try_set(BTreeSet::new());
                }
                Err(error) => {
                    failure.try_set(Some(error));
                }
            });
        });
    };
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(text(plural!(ticked, "trades.selected", n = ticked)).text_sm().font_weight(FontWeight::MEDIUM))
        .child(text("·").text_sm().text_color(move || theme().muted_foreground))
        .child(action(t!("common.delete"), Variant::Destructive, remove))
        .child(action(t!("trades.deselect"), Variant::Ghost, move || selected.set(BTreeSet::new())))
}

#[cfg(test)]
mod tests {
    use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, model::AccountInfo, time::parse_mt5_time};

    use super::*;
    use crate::snapshot::{Query, Range, load};

    fn deal(ticket: i64, position: i64, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "XAUUSD".into(),
            kind,
            entry,
            volume: 0.5,
            price: 2650.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            magic: 0,
            comment: String::new(),
            sl: 2640.0,
            tp: 0.0,
            risk: None,
            result_r: None,
            gaps: tradelens_core::Gaps::NONE,
            reason: None,
        }
    }

    fn snapshot() -> Snapshot {
        use DealEntry::{In, Out};
        use DealKind::{Buy, Sell};
        let mut db = Db::in_memory().expect("opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                currency: "USD".into(),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, 1, "2026.11.10 09:00:00", Buy, In, 0.0),
                deal(2, 1, "2026.11.10 10:00:00", Sell, Out, 100.0),
                deal(3, 2, "2026.11.11 09:00:00", Buy, In, 0.0),
                deal(4, 2, "2026.11.11 10:00:00", Sell, Out, -40.0),
                deal(5, 3, "2026.11.12 09:00:00", Buy, In, 0.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("stored");
        load(&db, &Query { account: None, range: Range::All, now_utc_ms: 0, local_offset_min: 0 }).expect("loads")
    }

    #[test]
    fn cells_show_what_a_closed_trade_made() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            let snapshot = snapshot();
            let cells = cells_of(&snapshot.rows[2]);
            assert_eq!(cells.time, "10/11 10:00");
            assert_eq!(cells.symbol, "XAUUSD");
            assert_eq!(cells.side, "Buy");
            assert_eq!(cells.lot, "0.50");
            assert_eq!(cells.hold, "1g 0p");
            assert_eq!(cells.net, "+$100.00");
            assert_eq!(cells.r, "0.0R", "the fixture closes at the entry price, so the stop-based R is zero");
            assert_eq!(cells.journal, NOT_JOURNALED);
        });
    }

    #[test]
    fn an_open_trade_shows_dashes() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            let snapshot = snapshot();
            let cells = cells_of(&snapshot.rows[0]);
            assert_eq!(cells.time, "đang mở");
            assert_eq!((cells.net.as_str(), cells.r.as_str(), cells.hold.as_str()), ("—", "—", "—"));
        });
        crate::i18n::with_text(crate::i18n::Lang::En, || {
            let snapshot = snapshot();
            assert_eq!(cells_of(&snapshot.rows[0]).time, "open");
        });
    }

    #[test]
    fn the_summary_adds_up_the_filtered_trades() {
        let snapshot = snapshot();
        let everything = summarise(&snapshot, &[0, 1, 2]);
        assert_eq!(everything.trades, 3);
        assert_eq!(everything.net, "+$60.00");
        assert_eq!(everything.win_rate, "50.0%");
        assert_eq!(everything.profit_factor, "2.50");
        assert_eq!(everything.unjournaled, 3);
        let none = summarise(&snapshot, &[]);
        assert_eq!((none.trades, none.win_rate.as_str(), none.net.as_str()), (0, "—", "$0.00"));
    }
}
