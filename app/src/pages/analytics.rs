//! Phân tích: Tìm lợi thế: kết quả theo symbol, thời gian, setup và lỗi.
//! Design: `docs/design/screens/analytics.html`.

use std::sync::Arc;

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::metrics::Group;

use super::account_picker::account_picker;
use crate::{
    format::{self, Sign},
    insight::{self, Analysis, HeatCell, WEEKDAYS},
    nav::{self, Route},
    snapshot::Range,
    store::{Store, store},
    ui::{
        breakdown::{BreakdownRow, breakdown_table},
        charts::{BarItem, HeatGrid, HeatValue, heatmap, histogram, mistake_cost_bar},
        chips::pnl_text,
        controls::{Variant, action, segmented},
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
            view().gap_6().child(table_card("Theo symbol", "Symbol", cx, |a| &a.by_symbol)),
            view().gap_6().child(table_card("Buy so với Sell", "Chiều", cx, |a| &a.by_side)),
        )
        .into_element(),
        Tab::Time => columns(
            view().gap_6().child(heat_card(cx)).child(weekday_card(cx)),
            view()
                .gap_6()
                .child(session_card(cx))
                .child(table_card("Theo giờ vào lệnh", "Giờ", cx, |a| &a.by_hour)),
        )
        .into_element(),
        Tab::Setup => columns(
            view().gap_6().child(table_card("Theo setup", "Setup", cx, |a| &a.by_setup)),
            view().gap_6().child(table_card("Theo tag", "Tag", cx, |a| &a.by_tag)),
        )
        .into_element(),
        Tab::Mistakes => columns(
            view().gap_6().child(mistake_card(cx)),
            view().gap_6().child(table_card("Theo cảm xúc trước lệnh", "Cảm xúc", cx, |a| &a.by_emotion)),
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
}

/// The rows of a breakdown table: every figure formatted for the screen.
fn rows_of(groups: &[Group], currency: &str) -> Vec<BreakdownRow> {
    groups
        .iter()
        .map(|g| BreakdownRow {
            name: g.label.clone(),
            trades: g.trades.to_string(),
            win_rate: format::percent(g.win_rate()),
            net: format::money(g.net, Sign::Always, currency),
            profit_factor: format::ratio(g.profit_factor()),
            avg_r: format::r_average(g.avg_r()),
        })
        .collect()
}

fn table_card(
    title: &'static str,
    name_title: &'static str,
    cx: Cx,
    pick: fn(&Analysis) -> &Vec<Group>,
) -> impl IntoElement {
    card(
        title,
        breakdown_table(
            name_title,
            move || cx.a.with(|a| rows_of(pick(a), &cx.currency())),
            "Chưa có lệnh nào trong khoảng này",
        ),
    )
}

/// The hour-by-weekday grid of net results by entry time.
fn heat_grid_of(cells: &[HeatCell], currency: &str) -> HeatGrid {
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
                        value: cell.net,
                        description: format!(
                            "{} {hour:02}:00: {} ({} lệnh)",
                            WEEKDAYS[weekday],
                            format::money(cell.net, Sign::Always, currency),
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
            let Some(((best, best_net), (worst, worst_net))) = insight::hour_extremes(a) else {
                return "Đậm hơn = lãi/lỗ lớn hơn".to_owned();
            };
            format!(
                "Đậm hơn = lãi/lỗ lớn hơn · Tốt nhất: {best:02}:00 ({}) · Tệ nhất: {worst:02}:00 ({})",
                cx.money(best_net, Sign::Always),
                cx.money(worst_net, Sign::Always)
            )
        })
    };
    card(
        "Giờ vào lệnh × thứ (giờ máy)",
        view()
            .gap_3()
            .child(heatmap(move || cx.a.with(|a| heat_grid_of(&a.heat, &cx.currency()))))
            .child(text(hint).text_xs().text_color(move || theme().muted_foreground)),
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
                                cx.money(g.net, Sign::Always),
                                g.net,
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
        breakdown_table(
            "Phiên",
            move || cx.a.with(|a| rows_of(&a.by_session, &cx.currency())),
            "Chưa có lệnh nào trong khoảng này",
        ),
    )
}

/// What the mistakes cost over the range, the costliest first.
fn mistake_card(cx: Cx) -> impl IntoElement {
    let store = store();
    let range = store.range;
    let total = pnl_text(move || cx.a.with(|a| cx.money(a.mistakes.net, Sign::Always)));
    let summary = move || {
        cx.a.with(|a| {
            format!(
                "= {} · {} lệnh có gắn lỗi trong {}",
                format::r_text(Some(a.mistakes.total_r.abs())),
                a.mistakes.trades,
                range.get().describe().to_lowercase()
            )
        })
    };
    let note = move || {
        cx.a.with(|a| {
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
                    .child(action("Xem lệnh", Variant::Ghost, move || nav::go(Route::Trades))),
            )
            .child(dynamic(move || {
                let costs = cx.a.with(|a| a.mistakes.costs.clone());
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
                            mistake_cost_bar(
                                &c.label,
                                c.trades,
                                cx.money(c.net, Sign::Always),
                                format::r_text(Some(c.total_r)),
                                c.share_of_losses,
                            )
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
        HeatCell { weekday, hour, net, trades: 1 }
    }

    #[test]
    fn the_heat_grid_covers_the_working_week_and_the_usual_hours() {
        let grid = heat_grid_of(&[cell(0, 8, 100.0)], "USD");
        assert_eq!(grid.row_labels, ["T2", "T3", "T4", "T5", "T6"]);
        assert_eq!(grid.column_labels.first().map(String::as_str), Some("7"));
        assert_eq!(grid.column_labels.last().map(String::as_str), Some("23"));
        assert!(grid.cells[0][1].is_some(), "Monday 08:00 has its cell");
        assert!(grid.cells[1][1].is_none());
    }

    #[test]
    fn the_heat_grid_grows_for_weekends_and_odd_hours() {
        let grid = heat_grid_of(&[cell(6, 3, -5.0)], "USD");
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
