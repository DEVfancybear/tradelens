//! Phân tích: Tìm lợi thế: kết quả theo symbol, thời gian, setup và lỗi.
//! Design: `docs/design/screens/analytics.html`.

use std::sync::Arc;

use chrono::Weekday;
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
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
    trade_list::Filter,
    ui::{
        breakdown::{BreakdownRow, breakdown_table, breakdown_table_pickable},
        charts::{BarItem, HeatGrid, HeatValue, heatmap, histogram, mistake_cost_bar},
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
    fn label(self) -> String {
        match self {
            Self::Overview => t!("analytics.tab.overview"),
            Self::Symbol => t!("analytics.tab.symbol"),
            Self::Time => t!("analytics.tab.time"),
            Self::Setup => t!("analytics.tab.setup"),
            Self::Mistakes => t!("analytics.tab.mistakes"),
            Self::Risk => t!("analytics.tab.risk"),
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
        t!("nav.analytics"),
        move || {
            let a = analysis.get();
            t!("analytics.summary", range = range.get().describe(), trades = format::trade_count(a.trades))
        },
        view().flex_row().flex_wrap().items_center().gap_3().child(account_picker()).child(segmented(
            t!("analytics.range"),
            &Range::ALL,
            Range::label,
            move || range.get(),
            move |next| range.set(next),
        )),
        view()
            .gap_5()
            .child(tab_bar(t!("nav.analytics"), &TABS, Tab::label, move || tab.get(), move |next| tab.set(next)))
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
            view().gap_6().child(picked_card(
                t!("analytics.by_symbol"),
                t!("analytics.symbol"),
                cx,
                |a| &a.by_symbol,
                Pick::Symbol,
            )),
            view().gap_6().child(table_card(t!("analytics.by_side"), t!("analytics.side"), cx, |a| &a.by_side)),
        )
        .into_element(),
        Tab::Time => columns(
            view().gap_6().child(heat_card(cx)).child(weekday_card(cx)),
            view().gap_6().child(session_card(cx)).child(hour_card(cx)),
        )
        .into_element(),
        Tab::Setup => columns(
            view().gap_6().child(picked_card(
                t!("analytics.by_setup"),
                t!("analytics.setup"),
                cx,
                |a| &a.by_setup,
                Pick::Setup,
            )),
            view().gap_6().child(picked_card(
                t!("analytics.by_tag"),
                t!("analytics.tag"),
                cx,
                |a| &a.by_tag,
                Pick::Tag,
            )),
        )
        .into_element(),
        Tab::Mistakes => columns(
            view().gap_6().child(mistake_card(cx)),
            view()
                .gap_6()
                .child(table_card(t!("analytics.by_emotion"), t!("analytics.emotion"), cx, |a| &a.by_emotion)),
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
    fn result_title(self) -> String {
        match self.unit() {
            Unit::Money => t!("analytics.result.money"),
            Unit::R => t!("analytics.result.r"),
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
    (a.time_unknown > 0).then(|| t!("analytics.time_unknown", trades = format::trade_count(a.time_unknown)))
}

fn table_card(title: String, name_title: String, cx: Cx, pick: fn(&Analysis) -> &Vec<Group>) -> impl IntoElement {
    card(
        title,
        breakdown_table(
            name_title,
            move || cx.result_title(),
            move || cx.a.with(|a| rows_of(pick(a), &cx.currency(), a.unit())),
            t!("analytics.no_trades"),
        ),
    )
}

/// The weekdays in the order of [`WEEKDAYS`], Monday first.
const WEEKDAYS_OF_WEEK: [Weekday; 7] =
    [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

/// The weekday head of a group `insight` labelled with its fixed short name, in the window's language.
fn weekday_title(label: &str) -> String {
    WEEKDAYS
        .iter()
        .position(|d| *d == label)
        .map_or_else(|| label.to_owned(), |at| format::weekday_short(WEEKDAYS_OF_WEEK[at]))
}

/// What a pressed line of a breakdown names, to open the trades under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Symbol,
    Setup,
    Tag,
}

impl Pick {
    /// The trades list filter that shows the trades under `name`: the symbol and setup filters of
    /// the list, and a tag as the list's search text (the list has no tag filter of its own).
    fn filter(self, name: &str) -> Filter {
        match self {
            Self::Symbol => Filter { symbol: Some(name.to_owned()), ..Filter::default() },
            Self::Setup => Filter { setup: Some(name.to_owned()), ..Filter::default() },
            Self::Tag => Filter { text: name.to_owned(), ..Filter::default() },
        }
    }
}

/// A [`table_card`] whose lines open the trades list on the trades under them.
fn picked_card(
    title: String,
    name_title: String,
    cx: Cx,
    pick: fn(&Analysis) -> &Vec<Group>,
    kind: Pick,
) -> impl IntoElement {
    card(
        title,
        breakdown_table_pickable(
            name_title,
            move || cx.result_title(),
            move || cx.a.with(|a| rows_of(pick(a), &cx.currency(), a.unit())),
            t!("analytics.no_trades"),
            move |name| {
                store().trade_filter.set(kind.filter(name));
                nav::go(Route::Trades);
            },
        ),
    )
}

/// The hour-by-weekday grid of net results by entry time.
fn heat_grid_of(cells: &[HeatCell], currency: &str, unit: Unit) -> HeatGrid {
    let first = cells.iter().map(|c| c.hour).min().unwrap_or(HEAT_FIRST_HOUR).min(HEAT_FIRST_HOUR);
    let last = cells.iter().map(|c| c.hour).max().unwrap_or(HEAT_LAST_HOUR).max(HEAT_LAST_HOUR);
    let weekend = cells.iter().any(|c| c.weekday >= 5);
    let days = if weekend { 7 } else { 5 };
    let column_labels = (first..=last).map(|h| h.to_string()).collect();
    let row_labels = WEEKDAYS_OF_WEEK.iter().take(days).map(|d| format::weekday_short(*d)).collect();
    let rows = (0..days)
        .map(|weekday| {
            (first..=last)
                .map(|hour| {
                    let cell = cells.iter().find(|c| usize::try_from(c.weekday) == Ok(weekday) && c.hour == hour)?;
                    Some(HeatValue {
                        value: cell.result(unit),
                        description: t!(
                            "analytics.heat.cell",
                            day = format::weekday_short(WEEKDAYS_OF_WEEK[weekday]),
                            hour = format!("{hour:02}"),
                            result = match unit {
                                Unit::Money => format::money(cell.net, Sign::Always, currency),
                                Unit::R => format::r_text(Some(cell.r)),
                            },
                            trades = format::trade_count(cell.trades)
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
                Some(((best, best_result), (worst, worst_result))) => t!(
                    "analytics.heat.hint_extremes",
                    best = format!("{best:02}"),
                    best_result = shown(best_result),
                    worst = format!("{worst:02}"),
                    worst_result = shown(worst_result)
                ),
                None => t!("analytics.heat.hint"),
            };
            if let Some(note) = time_note(a) {
                line.push_str(" · ");
                line.push_str(&note);
            }
            line
        })
    };
    let clock = move || match cx.store.snapshot().zone.display {
        TimeDisplay::Local => t!("analytics.heat.clock_local"),
        TimeDisplay::Server => t!("analytics.heat.clock_server"),
    };
    card(
        t!("analytics.heat.title"),
        view()
            .gap_3()
            .child(
                text(move || t!("analytics.heat.basis", clock = clock()))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            )
            .child(dynamic(move || {
                // No trade has a time of day: a grid of one column of zeros would say nothing.
                let none = cx.a.with(|a| a.heat.is_empty() && a.time_unknown > 0);
                if none {
                    let skipped = cx.a.with(|a| a.time_unknown);
                    empty_state(
                        t!("analytics.no_entry_time"),
                        t!("analytics.heat.day_only", trades = format::trade_count(skipped)),
                    )
                    .into_element()
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
        t!("analytics.hour.title"),
        view()
            .gap_3()
            .child(breakdown_table(
                t!("analytics.hour.name"),
                move || cx.result_title(),
                move || cx.a.with(|a| rows_of(&a.by_hour, &cx.currency(), a.unit())),
                t!("analytics.no_entry_time"),
            ))
            .child(
                text(move || cx.a.with(|a| time_note(a)).unwrap_or_default())
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

fn weekday_card(cx: Cx) -> impl IntoElement {
    card(
        t!("analytics.weekday.title"),
        view().gap_3().child(crate::ui::charts::bar_chart(
            move || {
                cx.a.with(|a| {
                    a.by_weekday
                        .iter()
                        .map(|g| {
                            BarItem::signed(
                                weekday_title(&g.label),
                                t!("analytics.weekday.won", pct = format!("{:.0}", g.win_rate() * 100.0)),
                                cx.result(g.net, g.total_r),
                                a.result_of(g),
                            )
                        })
                        .collect()
                })
            },
            t!("analytics.weekday.label"),
            150.0,
        )),
    )
}

fn session_card(cx: Cx) -> impl IntoElement {
    card(
        t!("analytics.session.title"),
        view()
            .gap_3()
            .child(breakdown_table(
                t!("analytics.session.name"),
                move || cx.result_title(),
                move || cx.a.with(|a| rows_of(&a.by_session, &cx.currency(), a.unit())),
                t!("analytics.no_entry_time"),
            ))
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
            Unit::Money => t!(
                "analytics.mistakes.summary_money",
                r = format::r_text(Some(a.mistakes.total_r.abs())),
                trades = format::trade_count(a.mistakes.trades),
                range = range.get().describe().to_lowercase()
            ),
            Unit::R => t!(
                "analytics.mistakes.summary_r",
                trades = format::trade_count(a.mistakes.trades),
                range = range.get().describe().to_lowercase()
            ),
        })
    };
    let note = move || {
        cx.a.with(|a| {
            if a.unit() == Unit::R {
                return None;
            }
            let top = a.mistakes.costs.first().filter(|c| c.net < 0.0)?;
            (a.net > 0.0).then(|| {
                t!("analytics.mistakes.note", mistake = top.label, pct = format!("{:.0}", -top.net / a.net * 100.0))
            })
        })
    };
    card(
        t!("analytics.mistakes.title"),
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
                    .child(link(t!("analytics.mistakes.view"), move || nav::go(Route::Trades))),
            )
            .child(dynamic(move || {
                let (costs, unit) = cx.a.with(|a| (a.mistakes.costs.clone(), a.unit()));
                let costs = in_unit(costs, unit);
                untracked(|| {
                    if costs.is_empty() {
                        return empty_state(t!("analytics.mistakes.empty"), t!("analytics.mistakes.empty_hint"))
                            .into_element();
                    }
                    view()
                        .children(costs.iter().map(|c| match unit {
                            Unit::Money => mistake_cost_bar(
                                &c.label,
                                c.trades,
                                cx.money(c.net, Sign::Always),
                                format::r_text(Some(c.total_r)),
                                c.share_of_losses,
                            ),
                            Unit::R => mistake_cost_bar(
                                &c.label,
                                c.trades,
                                format::r_text(Some(c.total_r)),
                                String::new(),
                                c.share_of_losses,
                            ),
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
                return t!("analytics.r.none");
            }
            t!(
                "analytics.r.facts",
                avg = format::r_average(a.avg_r),
                measured = format::trade_count(a.r_trades),
                beyond = format::trade_count(a.beyond_two_r)
            )
        })
    };
    card(
        t!("analytics.r.title"),
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
                t!("analytics.r.label"),
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
        let grid =
            crate::i18n::with_text(crate::i18n::Lang::Vi, || heat_grid_of(&[cell(0, 8, 100.0)], "USD", Unit::Money));
        assert_eq!(grid.row_labels, ["T2", "T3", "T4", "T5", "T6"]);
        assert_eq!(grid.column_labels.first().map(String::as_str), Some("7"));
        assert_eq!(grid.column_labels.last().map(String::as_str), Some("23"));
        assert!(grid.cells[0][1].is_some(), "Monday 08:00 has its cell");
        assert!(grid.cells[1][1].is_none());
    }

    #[test]
    fn the_heat_grid_grows_for_weekends_and_odd_hours() {
        let grid =
            crate::i18n::with_text(crate::i18n::Lang::Vi, || heat_grid_of(&[cell(6, 3, -5.0)], "USD", Unit::Money));
        assert_eq!(grid.row_labels.len(), 7);
        assert_eq!(grid.column_labels.first().map(String::as_str), Some("3"));
        let sunday = &grid.cells[6];
        assert_eq!(sunday.iter().flatten().count(), 1);
    }

    #[test]
    fn tabs_have_distinct_names() {
        let names: Vec<String> =
            crate::i18n::with_text(crate::i18n::Lang::Vi, || TABS.iter().map(|t| t.label()).collect());
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }
}
