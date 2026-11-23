//! Lệnh: Danh sách lệnh đã gom từ deal MT5, lọc và mở nhật ký từng lệnh.
//! Design: `docs/design/screens/trades.html`.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    metrics::{self, Sample},
    time::format_duration,
};

use super::{account_picker::account_picker, manual_entry, trade_detail};
use crate::{
    format::{self, Sign},
    nav,
    snapshot::{Snapshot, TradeRow},
    store::{Load, store},
    trade_list::{self, Column, Direction, Filter, Order, Outcome},
    ui::{
        chips::{Tone, pnl_text},
        controls::{Choice, Variant, action, action_icon, segmented, select_field, text_field},
        icons::Icon,
        parts::page_fill,
        table::{JOURNALED, NOT_JOURNALED, Sorting, TableSort, TradeCells, trades_table},
    },
};

/// The list, or the page of the trade opened from it.
pub(crate) fn trades() -> impl IntoElement {
    dynamic(move || match nav::open_trade() {
        Some(key) => untracked(|| trade_detail::trade_detail(key)).into_element(),
        None if nav::manual_entry_open() => untracked(manual_entry::manual_entry).into_element(),
        None => untracked(list).into_element(),
    })
}

/// The cells of the table for one trade.
pub(crate) fn cells_of(row: &TradeRow) -> TradeCells {
    let trade = &row.trade;
    let closed = trade.is_closed();
    TradeCells {
        time: row.closed().map_or_else(|| "đang mở".to_owned(), format::short_stamp),
        symbol: trade.symbol.clone(),
        side: trade.side.label().to_owned(),
        lot: format::lots(trade.volume),
        hold: trade.duration_secs().map_or_else(|| "—".to_owned(), format_duration),
        // A trade imported in R has no money: a dash, never "$0.00".
        net: if closed && trade.money_known() {
            format::money(trade.net(), Sign::Always, &row.currency)
        } else {
            "—".to_owned()
        },
        r: if closed { format::r_text(row.r) } else { "—".to_owned() },
        setup: row.setup.clone(),
        journal: (if row.journaled { JOURNALED } else { NOT_JOURNALED }).to_owned(),
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
    Summary {
        trades: rows.len(),
        // Only trades imported in R in the list: there is no money to add up.
        net: if !samples.is_empty() && samples.iter().all(|x| !x.trade.money_known()) {
            "—".to_owned()
        } else {
            format::money(stats.net, Sign::Always, &snapshot.currency)
        },
        win_rate: if samples.is_empty() { "—".to_owned() } else { format::percent(stats.win_rate) },
        profit_factor: format::ratio(stats.profit_factor),
        unjournaled: rows.iter().filter(|row| !row.journaled).count(),
    }
}

const OUTCOMES: &[Outcome] = &[Outcome::All, Outcome::Win, Outcome::Loss];
const DIRECTIONS: &[Direction] = &[Direction::All, Direction::Buy, Direction::Sell];
const ANY: &str = "";

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::All => "Tất cả",
        Outcome::Win => "Thắng",
        Outcome::Loss => "Thua",
    }
}

fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::All => "Tất cả",
        Direction::Buy => "Buy",
        Direction::Sell => "Sell",
    }
}

/// A choice list of `values` with "Tất cả" first.
fn any_or(values: &[String]) -> Vec<Choice> {
    let mut choices = vec![Choice::new(ANY, "Tất cả")];
    choices.extend(values.iter().map(|v| Choice::new(v.clone(), v.clone())));
    choices
}

fn list() -> impl IntoElement {
    let store = store();
    let (filter, sort) = (store.trade_filter, store.trade_sort);
    let picked = computed(move || trade_list::select(&store.snapshot().rows, &filter.get(), sort.get()));
    let rows = move || {
        let snapshot = store.snapshot();
        picked.with(|picked| picked.iter().filter_map(|&i| snapshot.rows.get(i)).map(cells_of).collect())
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
        Load::Loading => ("Đang tải dữ liệu…".to_owned(), String::new()),
        Load::Failed(error) => (error, "Thử mở lại ứng dụng; dữ liệu trên đĩa không bị mất.".to_owned()),
        Load::Ready if filter.get().is_active() && !store.snapshot().rows.is_empty() => {
            ("Không có lệnh nào khớp bộ lọc".to_owned(), "Bỏ bớt bộ lọc để xem thêm lệnh.".to_owned())
        }
        Load::Ready => (
            "Chưa có lệnh nào".to_owned(),
            "Kết nối EA ở Nguồn dữ liệu, hoặc import báo cáo MT5, để thấy lệnh ở đây.".to_owned(),
        ),
    };

    // Ctrl+N adds a trade by hand, as the button does.
    on_key_with("mod+n", KeyOptions { pass_through: true, ..KeyOptions::default() }, |_| nav::open_manual_entry());
    page_fill(
        "Lệnh",
        "Gom từ deal MT5 theo position; bấm một lệnh để mở nhật ký",
        view().flex_row().items_center().gap_3().child(account_picker()).child(action_icon(
            Icon::Plus,
            "Thêm lệnh",
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
                empty,
            ))
            .child(footer(picked)),
    )
}

fn filters(store: crate::store::Store) -> impl IntoElement {
    let filter = store.trade_filter;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(view().flex_1().min_w(px(240.0)).child(text_field(
            "Tìm lệnh",
            Some("⌕"),
            "Tìm symbol, ticket, ghi chú…",
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
            "Kết quả",
            OUTCOMES,
            outcome_label,
            move || filter.with(|f| f.outcome),
            move |outcome| filter.update(|f| f.outcome = outcome),
        ))
        .child(segmented(
            "Chiều",
            DIRECTIONS,
            direction_label,
            move || filter.with(|f| f.direction),
            move |direction| filter.update(|f| f.direction = direction),
        ))
        .child(dynamic(move || {
            filter.with(Filter::is_active).then(|| {
                action("Xoá lọc", Variant::Ghost, move || {
                    filter.set(Filter::default());
                })
            })
        }))
}

/// A figure with its caption above it, in the strip over the table.
fn figure(
    caption: &'static str,
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
        .child(figure("Lệnh", move || summary.with(|s| s.trades.to_string()), || Tone::Neutral))
        .child(
            view()
                .gap_1()
                .child(text("Net").text_xs().text_color(move || theme().muted_foreground))
                .child(pnl_text(move || summary.with(|s| s.net.clone()))),
        )
        .child(figure("Win rate", move || summary.with(|s| s.win_rate.clone()), || Tone::Neutral))
        .child(figure("PF", move || summary.with(|s| s.profit_factor.clone()), || Tone::Neutral))
        .child(figure("Chưa ghi nhật ký", move || summary.with(|s| s.unjournaled.to_string()), || Tone::Neutral))
}

fn footer(picked: Memo<Vec<usize>>) -> impl IntoElement {
    let store = store();
    text(move || {
        let shown = picked.with(Vec::len);
        let all = store.snapshot().rows.len();
        format!("Hiển thị {shown} / {all} lệnh")
    })
    .text_xs()
    .text_color(move || theme().muted_foreground)
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
    }

    #[test]
    fn an_open_trade_shows_dashes() {
        let snapshot = snapshot();
        let cells = cells_of(&snapshot.rows[0]);
        assert_eq!(cells.time, "đang mở");
        assert_eq!((cells.net.as_str(), cells.r.as_str(), cells.hold.as_str()), ("—", "—", "—"));
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
