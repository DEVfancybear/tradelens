//! Phân tích: Tìm lợi thế: kết quả theo symbol, thời gian, setup và lỗi.
//! Design: `docs/design/screens/analytics.html`.

use std::sync::Arc;

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    TradeKey,
    metrics::{Group, MistakeCost, Unit},
    time::TimeDisplay,
};

use super::account_picker::account_picker;
use crate::{
    format::{self, Sign},
    insight::{self, Analysis, HeatCell, WEEKDAYS},
    nav::{self, Route},
    snapshot::Range,
    store::{Store, store},
    trade_list::{Filter, Focus},
    ui::{
        breakdown::{BreakdownRow, breakdown_table},
        charts::{BarItem, HeatGrid, HeatValue, heatmap, histogram, mistake_cost_bar_pickable},
        chips::pnl_text,
        controls::{link, segmented},
        layout::columns,
        parts::{card, empty_state, page_with},
        tab_bar::tab_bar,
    },
};

/// The views the page splits into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Symbol,
    Time,
    Setup,
    Mistakes,
    Risk,
}

const TABS: [Tab; 6] = [Tab::Overview, Tab::Symbol, Tab::Time, Tab::Setup, Tab::Mistakes, Tab::Risk];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Tổng hợp",
            Self::Symbol => "Symbol",
            Self::Time => "Thời gian",
            Self::Setup => "Setup & tag",
            Self::Mistakes => "Lỗi & cảm xúc",
            Self::Risk => "Rủi ro (R)",
        }
    }
}

/// The first hour and the last hour the heatmap shows at least.
const HEAT_FIRST_HOUR: u32 = 7;
const HEAT_LAST_HOUR: u32 = 23;

pub(crate) fn analytics() -> impl IntoElement {
    let store = store();
    let range = store.range;
    let tab = signal(Tab::Overview);
    let today = computed(move || insight::today_now(&store.snapshot()));
    let cx = Cx { a: computed(move || Arc::new(insight::analyse(&store.snapshot(), range.get(), today.get()))), store };
    let analysis = cx.a;

    page_with(
        "Phân tích",
        move || {
            let a = analysis.get();
            format!("{} · {} lệnh đã đóng", range.get().describe(), a.trades)
        },
        view().flex_row().flex_wrap().items_center().gap_3().child(account_picker()).child(segmented(
            "Khoảng thời gian",
            &Range::ALL,
            Range::label,
            move || range.get(),
            move |next| range.set(next),
        )),
        view()
            .gap_5()
            .child(tab_bar("Phân tích", &TABS, Tab::label, move || tab.get(), move |next| tab.set(next)))
            .child(dynamic(move || {
                let open = tab.get();
                untracked(|| content(open, cx).into_element())
            })),
    )
}

fn content(tab: Tab, cx: Cx) -> impl IntoElement {
    match tab {
        Tab::Overview => columns(
            view().gap_6().child(heat_card(cx)).child(mistake_card(cx)).child(weekday_card(cx)),
            view().gap_6().child(session_card(cx)).child(r_card(cx)),
        )
        .into_element(),
        Tab::Symbol => columns(
            view().gap_6().child(picked_card("Theo symbol", "Symbol", cx, |a| &a.by_symbol, Pick::Symbol)),
            view().gap_6().child(picked_card("Buy so với Sell", "Chiều", cx, |a| &a.by_side, Pick::Rows("Chiều"))),
        )
        .into_element(),
        Tab::Time => columns(
            view().gap_6().child(heat_card(cx)).child(weekday_card(cx)),
            view().gap_6().child(session_card(cx)).child(hour_card(cx)),
        )
        .into_element(),
        Tab::Setup => columns(
            view().gap_6().child(picked_card("Theo setup", "Setup", cx, |a| &a.by_setup, Pick::Setup)),
            view().gap_6().child(picked_card("Theo tag", "Tag", cx, |a| &a.by_tag, Pick::Rows("Tag"))),
        )
        .into_element(),
        Tab::Mistakes => columns(
            view().gap_6().child(mistake_card(cx)),
            view().gap_6().child(picked_card(
                "Theo cảm xúc trước lệnh",
                "Cảm xúc",
                cx,
                |a| &a.by_emotion,
                Pick::Rows("Cảm xúc"),
            )),
        )
        .into_element(),
        Tab::Risk => columns(view().gap_6().child(r_card(cx)), view()).into_element(),
    }
}

/// What the cards read: the analysis, and the store for the money's currency.
#[derive(Clone, Copy)]
struct Cx {
    a: Memo<Arc<Analysis>>,
    store: Store,
}

impl Cx {
    fn currency(self) -> String {
        self.store.snapshot().currency.clone()
    }

    fn money(self, value: f64, sign: Sign) -> String {
        format::money(value, sign, &self.currency())
    }

    /// The unit every figure of the page is in: R when none of the trades has money.
    fn unit(self) -> Unit {
        self.a.with(|a| a.unit())
    }

    /// A result as the page's unit shows it: the money, or the R.
    fn result(self, money: f64, r: f64) -> String {
        match self.unit() {
            Unit::Money => self.money(money, Sign::Always),
            Unit::R => format::r_text(Some(r)),
        }
    }

    /// The title of the column of results.
    fn result_title(self) -> &'static str {
        match self.unit() {
            Unit::Money => "Net",
            Unit::R => "Tổng R",
        }
    }
}

/// The rows of a breakdown table: every figure formatted for the screen, in `unit`.
fn rows_of(groups: &[Group], currency: &str, unit: Unit) -> Vec<BreakdownRow> {
    groups
        .iter()
        .map(|g| BreakdownRow {
            name: g.label.clone(),
            trades: g.trades.to_string(),
            win_rate: format::percent(g.win_rate()),
            net: match unit {
                Unit::Money => format::money(g.net, Sign::Always, currency),
                Unit::R => format::r_text(Some(g.total_r)),
            },
            profit_factor: format::ratio(match unit {
                Unit::Money => g.profit_factor(),
                Unit::R => g.profit_factor_r(),
            }),
            avg_r: format::r_average(g.avg_r()),
        })
        .collect()
}

/// What to say under the hour and session figures about the trades they leave out.
fn time_note(a: &Analysis) -> Option<String> {
    (a.time_unknown > 0).then(|| format!("{} lệnh không có giờ vào, không tính ở đây", a.time_unknown))
}

/// What a pressed line of a breakdown opens in the trades list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// The list's own symbol filter.
    Symbol,
    /// The list's own setup filter.
    Setup,
    /// Exactly the trades of the row (a tag, a side, a session, an hour, an emotion), named by
    /// the column's title and the row: the list has no filter of its own for these.
    Rows(&'static str),
}

impl Pick {
    /// The filter of the trades under the row `group`.
    fn filter(self, group: &Group) -> Filter {
        match self {
            Self::Symbol => Filter { symbol: Some(group.label.clone()), ..Filter::default() },
            Self::Setup => Filter { setup: Some(group.label.clone()), ..Filter::default() },
            Self::Rows(kind) => focus_on(format!("{kind} {}", group.label), &group.keys),
        }
    }
}

/// The trades list showing only `keys`, said as `title`.
fn focus_on(title: String, keys: &[TradeKey]) -> Filter {
    Filter { focus: Some(Focus { title, keys: keys.iter().cloned().collect() }), ..Filter::default() }
}

/// Opens the trades list with `filter`.
fn open_trades(filter: Filter) {
    store().trade_filter.set(filter);
    nav::go(Route::Trades);
}

/// A [`table_card`] whose lines open the trades list on the trades under them.
fn picked_card(
    title: &'static str,
    name_title: &'static str,
    cx: Cx,
    pick: fn(&Analysis) -> &Vec<Group>,
    kind: Pick,
) -> impl IntoElement {
    card(title, picked_table(name_title, cx, pick, kind, "Chưa có lệnh nào trong khoảng này"))
}

/// The table of `pick`'s groups, each line opening its trades.
fn picked_table(
    name_title: &'static str,
    cx: Cx,
    pick: fn(&Analysis) -> &Vec<Group>,
    kind: Pick,
    empty: &'static str,
) -> impl IntoElement {
    breakdown_table(
        name_title,
        move || cx.result_title(),
        move || cx.a.with(|a| rows_of(pick(a), &cx.currency(), a.unit())),
        empty,
        move |name| {
            let filter = cx.a.with(|a| pick(a).iter().find(|g| g.label == name).map(|g| kind.filter(g)));
            if let Some(filter) = filter {
                open_trades(filter);
            }
        },
    )
}

/// The hour-by-weekday grid of net results by entry time.
fn heat_grid_of(cells: &[HeatCell], currency: &str, unit: Unit) -> HeatGrid {
    let first = cells.iter().map(|c| c.hour).min().unwrap_or(HEAT_FIRST_HOUR).min(HEAT_FIRST_HOUR);
    let last = cells.iter().map(|c| c.hour).max().unwrap_or(HEAT_LAST_HOUR).max(HEAT_LAST_HOUR);
    let weekend = cells.iter().any(|c| c.weekday >= 5);
    let days = if weekend { 7 } else { 5 };
    let column_labels = (first..=last).map(|h| h.to_string()).collect();
    let row_labels = WEEKDAYS.iter().take(days).map(|d| (*d).to_owned()).collect();
    let rows = (0..days)
        .map(|weekday| {
            (first..=last)
                .map(|hour| {
                    let cell = cells.iter().find(|c| usize::try_from(c.weekday) == Ok(weekday) && c.hour == hour)?;
                    Some(HeatValue {
                        value: cell.result(unit),
                        description: format!(
                            "{} {hour:02}:00: {} ({} lệnh)",
                            WEEKDAYS[weekday],
                            match unit {
                                Unit::Money => format::money(cell.net, Sign::Always, currency),
                                Unit::R => format::r_text(Some(cell.r)),
                            },
                            cell.trades
                        ),
                    })
                })
                .collect()
        })
        .collect();
    HeatGrid { row_labels, column_labels, cells: rows }
}

fn heat_card(cx: Cx) -> impl IntoElement {
    let hint = move || {
        cx.a.with(|a| {
            let shown = |value: f64| cx.result(value, value);
            let mut line = match insight::hour_extremes(a) {
                Some(((best, best_result), (worst, worst_result))) => format!(
                    "Đậm hơn = lãi/lỗ lớn hơn · Tốt nhất: {best:02}:00 ({}) · Tệ nhất: {worst:02}:00 ({})",
                    shown(best_result),
                    shown(worst_result)
                ),
                None => "Đậm hơn = lãi/lỗ lớn hơn".to_owned(),
            };
            if let Some(note) = time_note(a) {
                line.push_str(" · ");
                line.push_str(&note);
            }
            line
        })
    };
    let clock = move || match cx.store.snapshot().zone.display {
        TimeDisplay::Local => "giờ của bạn",
        TimeDisplay::Server => "giờ server",
    };
    card(
        "Giờ vào lệnh × thứ",
        view()
            .gap_3()
            .child(
                text(move || format!("Tính theo {}", clock())).text_xs().text_color(move || theme().muted_foreground),
            )
            .child(dynamic(move || {
                // No trade has a time of day: a grid of one column of zeros would say nothing.
                let none = cx.a.with(|a| a.heat.is_empty() && a.time_unknown > 0);
                if none {
                    let skipped = cx.a.with(|a| a.time_unknown);
                    empty_state("Dữ liệu không có giờ vào lệnh", format!("{skipped} lệnh chỉ có ngày")).into_element()
                } else {
                    heatmap(move || cx.a.with(|a| heat_grid_of(&a.heat, &cx.currency(), a.unit()))).into_element()
                }
            }))
            // The legend explains cells: with none drawn (an empty state in their place) it has
            // nothing to explain.
            .child(dynamic(move || {
                let drawn = cx.a.with(|a| !a.heat.is_empty());
                drawn.then(|| text(hint).text_xs().text_color(move || theme().muted_foreground))
            })),
    )
}

/// The table by hour of entry; trades dated by day only have no hour and are counted apart.
fn hour_card(cx: Cx) -> impl IntoElement {
    card(
        "Theo giờ vào lệnh",
        view()
            .gap_3()
            .child(picked_table("Giờ", cx, |a| &a.by_hour, Pick::Rows("Giờ"), "Dữ liệu không có giờ vào lệnh"))
            .child(
                text(move || cx.a.with(|a| time_note(a)).unwrap_or_default())
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

fn weekday_card(cx: Cx) -> impl IntoElement {
    card(
        "Theo thứ trong tuần",
        view().gap_3().child(crate::ui::charts::bar_chart(
            move || {
                cx.a.with(|a| {
                    a.by_weekday
                        .iter()
                        .map(|g| {
                            BarItem::signed(
                                g.label.clone(),
                                format!("{:.0}% thắng", g.win_rate() * 100.0),
                                cx.result(g.net, g.total_r),
                                a.result_of(g),
                            )
                        })
                        .collect()
                })
            },
            "Kết quả theo thứ trong tuần",
            150.0,
        )),
    )
}

fn session_card(cx: Cx) -> impl IntoElement {
    card(
        "Theo phiên",
        view()
            .gap_3()
            .child(picked_table("Phiên", cx, |a| &a.by_session, Pick::Rows("Phiên"), "Dữ liệu không có giờ vào lệnh"))
            .child(
                text(move || cx.a.with(|a| time_note(a)).unwrap_or_default())
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

/// The mistakes' costs ordered and shared out in `unit`: in money as [`tradelens_core::metrics::mistake_cost`]
/// made them, in R (trades without money) by the R each mistake cost.
fn in_unit(mut costs: Vec<MistakeCost>, unit: Unit) -> Vec<MistakeCost> {
    if unit == Unit::R {
        costs.sort_by(|a, b| a.total_r.total_cmp(&b.total_r));
        let lost: f64 = costs.iter().map(|c| (-c.total_r).max(0.0)).sum();
        for cost in &mut costs {
            cost.share_of_losses = if lost > 0.0 { (-cost.total_r).max(0.0) / lost } else { 0.0 };
        }
    }
    costs
}

/// What the mistakes cost over the range, the costliest first.
fn mistake_card(cx: Cx) -> impl IntoElement {
    let store = store();
    let range = store.range;
    let total = pnl_text(move || cx.a.with(|a| cx.result(a.mistakes.net, a.mistakes.total_r)));
    let summary = move || {
        cx.a.with(|a| match a.unit() {
            Unit::Money => format!(
                "= {} · {} lệnh có gắn lỗi trong {}",
                format::r_text(Some(a.mistakes.total_r.abs())),
                a.mistakes.trades,
                range.get().describe().to_lowercase()
            ),
            Unit::R => {
                format!("{} lệnh có gắn lỗi trong {}", a.mistakes.trades, range.get().describe().to_lowercase())
            }
        })
    };
    let note = move || {
        cx.a.with(|a| {
            if a.unit() == Unit::R {
                return None;
            }
            let top = a.mistakes.costs.first().filter(|c| c.net < 0.0)?;
            (a.net > 0.0).then(|| {
                format!(
                    "Nếu bỏ được riêng lỗi {}, lợi nhuận tăng +{:.0}%. Tính trên P/L thực của lệnh có lỗi, không ước đoán.",
                    top.label,
                    -top.net / a.net * 100.0
                )
            })
        })
    };
    card(
        "Chi phí của lỗi",
        view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .items_end()
                    .justify_between()
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_end()
                            .gap_3()
                            .child(view().child(total))
                            .child(text(summary).text_xs().pb_1().text_color(move || theme().muted_foreground)),
                    )
                    .child(link("Xem lệnh", move || {
                        // Every trade with a mistake, once.
                        let keys: Vec<TradeKey> = cx.a.with(|a| {
                            let mut all: Vec<TradeKey> = a.mistakes.costs.iter().flat_map(|c| c.keys.clone()).collect();
                            all.sort();
                            all.dedup();
                            all
                        });
                        open_trades(focus_on("Lệnh có gắn lỗi".to_owned(), &keys));
                    })),
            )
            .child(dynamic(move || {
                let (costs, unit) = cx.a.with(|a| (a.mistakes.costs.clone(), a.unit()));
                let costs = in_unit(costs, unit);
                untracked(|| {
                    if costs.is_empty() {
                        return empty_state(
                            "Chưa có lệnh nào gắn lỗi",
                            "Gắn lỗi vào nhật ký của lệnh để thấy mỗi lỗi tốn bao nhiêu.",
                        )
                        .into_element();
                    }
                    view()
                        .children(costs.iter().map(|c| {
                            let (net, r) = match unit {
                                Unit::Money => (cx.money(c.net, Sign::Always), format::r_text(Some(c.total_r))),
                                Unit::R => (format::r_text(Some(c.total_r)), String::new()),
                            };
                            let filter = focus_on(format!("Lỗi {}", c.label), &c.keys);
                            mistake_cost_bar_pickable(&c.label, c.trades, net, r, c.share_of_losses, move || {
                                open_trades(filter.clone());
                            })
                        }))
                        .into_element()
                })
            }))
            .child(text(move || note().unwrap_or_default()).text_sm().text_color(move || theme().muted_foreground)),
    )
}

/// The R distribution and what it says of the trader's stops.
fn r_card(cx: Cx) -> impl IntoElement {
    let facts = move || {
        cx.a.with(|a| {
            if a.r_trades == 0 {
                return "Chưa có lệnh nào đo được R: đặt SL hoặc ghi rủi ro dự định.".to_owned();
            }
            format!(
                "Expectancy {} trên {} lệnh có SL · {} lệnh lỗ quá −2R",
                format::r_average(a.avg_r),
                a.r_trades,
                a.beyond_two_r
            )
        })
    };
    card(
        "Phân phối R",
        view()
            .gap_3()
            .child(histogram(
                move || {
                    cx.a.with(|a| {
                        a.r_buckets
                            .iter()
                            .enumerate()
                            .map(|(at, (label, count))| ((*label).to_owned(), *count, at < 3))
                            .collect()
                    })
                },
                "Phân phối kết quả theo R",
                140.0,
            ))
            .child(text(facts).text_xs().text_color(move || theme().muted_foreground)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(weekday: u32, hour: u32, net: f64) -> HeatCell {
        HeatCell { weekday, hour, net, r: 0.0, trades: 1 }
    }

    #[test]
    fn the_heat_grid_covers_the_working_week_and_the_usual_hours() {
        let grid = heat_grid_of(&[cell(0, 8, 100.0)], "USD", Unit::Money);
        assert_eq!(grid.row_labels, ["T2", "T3", "T4", "T5", "T6"]);
        assert_eq!(grid.column_labels.first().map(String::as_str), Some("7"));
        assert_eq!(grid.column_labels.last().map(String::as_str), Some("23"));
        assert!(grid.cells[0][1].is_some(), "Monday 08:00 has its cell");
        assert!(grid.cells[1][1].is_none());
    }

    #[test]
    fn the_heat_grid_grows_for_weekends_and_odd_hours() {
        let grid = heat_grid_of(&[cell(6, 3, -5.0)], "USD", Unit::Money);
        assert_eq!(grid.row_labels.len(), 7);
        assert_eq!(grid.column_labels.first().map(String::as_str), Some("3"));
        let sunday = &grid.cells[6];
        assert_eq!(sunday.iter().flatten().count(), 1);
    }

    #[test]
    fn tabs_have_distinct_names() {
        let names: Vec<&str> = TABS.iter().map(|t| t.label()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }
}
