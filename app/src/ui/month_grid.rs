//! The month grid: one cell per day coloured by what the day made, with a column of weekly
//! totals. The full grid (Lịch P/L) picks a day; the compact one (Tổng quan) shows the weekdays
//! and opens the calendar on the day pressed.

use chrono::{Datelike, NaiveDate, Weekday};
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tradelens_core::metrics::{CalendarDay, CalendarWeek, DayStat, Unit};

use super::{
    chips::Tone,
    motion,
    theme::{self as tokens, loss, loss_soft, profit, profit_soft},
};
use crate::format::{self, Sign};

/// What each full cell shows of its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Figure {
    /// The day's net result in money.
    #[default]
    Net,
    /// The day's result in R.
    R,
    /// How many trades closed.
    Trades,
}

impl Figure {
    pub(crate) const ALL: [Self; 3] = [Self::Net, Self::R, Self::Trades];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Net => "Kết quả",
            Self::R => "R",
            Self::Trades => "Số lệnh",
        }
    }
}

const CELL_HEIGHT: f32 = 84.0;
const COMPACT_HEIGHT: f32 = 30.0;
const TOTAL_WIDTH: f32 = 92.0;
const WEEKDAY_NAMES: [&str; 7] = ["Thứ 2", "Thứ 3", "Thứ 4", "Thứ 5", "Thứ 6", "Thứ 7", "CN"];

fn ink(tone: Tone) -> AdaptiveColor {
    match tone {
        Tone::Profit => profit(),
        Tone::Loss => loss(),
        Tone::Neutral | Tone::Primary => theme().foreground,
    }
}

/// The tone of a day's result: green above zero, red below, none when flat.
fn tone_of(net: f64) -> Tone {
    if net > 0.005 {
        Tone::Profit
    } else if net < -0.005 {
        Tone::Loss
    } else {
        Tone::Neutral
    }
}

fn fill(day: &CalendarDay) -> AdaptiveColor {
    match day.stat.map(|s| tone_of(s.result())) {
        Some(Tone::Profit) => profit_soft(),
        Some(Tone::Loss) => loss_soft(),
        _ => theme().background,
    }
}

fn edge(day: &CalendarDay) -> AdaptiveColor {
    match day.stat.map(|s| tone_of(s.result())) {
        Some(Tone::Profit) => profit().opacity(0.45),
        Some(Tone::Loss) => loss().opacity(0.45),
        _ => theme().border,
    }
}

/// The big figure of a day and its tone. A day whose trades have no money shows its R where the
/// money would be, never an amount of zero.
fn figure_of(stat: &DayStat, figure: Figure, currency: &str) -> (String, Tone) {
    match figure {
        Figure::Net if stat.unit() == Unit::R => figure_of(stat, Figure::R, currency),
        Figure::Net => (format::money_whole(stat.net, Sign::Always, currency), tone_of(stat.net)),
        Figure::R if stat.r_trades > 0 => (format::r_text(Some(stat.r)), tone_of(stat.r)),
        Figure::R => ("—".to_owned(), Tone::Neutral),
        Figure::Trades => (format!("{} lệnh", stat.trades), Tone::Neutral),
    }
}

/// The small line under a day's figure.
fn note_of(stat: &DayStat, figure: Figure) -> String {
    match figure {
        Figure::Net | Figure::R => format!("{}/{} lệnh thắng", stat.wins, stat.trades),
        Figure::Trades => format!("{} thắng", stat.wins),
    }
}

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The weekly total as the cells show their days: money, R, or the number of trades, with its tone.
/// A week whose trades have no money shows its R where the money would be.
fn week_figure(week: &CalendarWeek, figure: Figure, currency: &str) -> (String, Tone) {
    if week.trades == 0 {
        return ("—".to_owned(), Tone::Neutral);
    }
    match figure {
        Figure::Net if week.unit == Unit::R => (format::r_text(Some(week.r)), tone_of(week.r)),
        Figure::Net => (format::money_whole(week.net, Sign::Always, currency), tone_of(week.net)),
        Figure::R => (format::r_text(Some(week.r)), tone_of(week.r)),
        Figure::Trades => (format!("{} lệnh", week.trades), Tone::Neutral),
    }
}

/// The weekly total at the right of a full row.
fn week_total(week: &CalendarWeek, figure: Signal<Figure>, currency: &str) -> impl IntoElement + use<> {
    let (week_a, week_b, currency_a, currency_b) =
        (week.clone(), week.clone(), currency.to_owned(), currency.to_owned());
    let net = move || week_figure(&week_a, figure.get(), &currency_a).0;
    let tone = move || week_figure(&week_b, figure.get(), &currency_b).1;
    view()
        .w(px(TOTAL_WIDTH))
        .h(px(CELL_HEIGHT))
        .p_2()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().secondary.opacity(0.5))
        .child(text("Tuần").text_xs().text_color(move || theme().muted_foreground))
        .child(
            text(net)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(tokens::mono)
                .tabular_nums()
                .number_of_lines(1)
                .text_color(move || ink(tone())),
        )
        .child(text(format!("{} lệnh", week.trades)).text_xs().text_color(move || theme().muted_foreground))
}

/// One full day: its number, its figure and a note, a button that picks the day.
fn day_cell(
    day: CalendarDay,
    figure: Signal<Figure>,
    selected: Signal<Option<NaiveDate>>,
    currency: &str,
    today: NaiveDate,
    on_pick: impl Fn(NaiveDate) + Copy + 'static,
) -> impl IntoElement {
    let date = day.date;
    let is_selected = move || selected.get() == Some(date);
    let currency = currency.to_owned();
    let shown = move || figure_of(&day.stat.unwrap_or_default(), figure.get(), &currency);
    let shown_ink = shown.clone();
    let body = view().flex_1().gap_1().child(
        view()
            .flex_row()
            .justify_between()
            .child(text(date.day().to_string()).text_xs().tabular_nums().text_color(move || theme().muted_foreground))
            .child(view().size(px(6.0)).rounded_full().bg(move || theme().primary).opacity(if date == today {
                1.0_f32
            } else {
                0.0_f32
            })),
    );
    let content = match day.stat {
        Some(stat) => body
            .child(
                text(move || shown().0)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .font_family(tokens::mono)
                    .tabular_nums()
                    .number_of_lines(1)
                    .text_color(move || ink(shown_ink().1)),
            )
            .child(
                text(move || note_of(&stat, figure.get()))
                    .text_xs()
                    .number_of_lines(1)
                    .text_color(move || theme().muted_foreground),
            ),
        None if day.in_month && date <= today && !is_weekend(date) => {
            body.child(text("Nghỉ").text_xs().text_color(move || theme().muted_foreground))
        }
        None => body,
    };
    button()
        .cursor_pointer()
        .on_press(move || on_pick(date))
        .accessibility_label(format!("Xem ngày {}", date.format("%d/%m")))
        .flex_1()
        .min_w(px(0.0))
        .h(px(CELL_HEIGHT))
        .p_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || edge(&day))
        .bg(move || fill(&day))
        .opacity(if day.in_month { 1.0_f32 } else { 0.4_f32 })
        .hover(|s| s.opacity(0.85))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .style_when(is_selected, |s| s.outline(px(2.0)).outline_color(move || theme().primary))
        .child(content)
}

/// The month as a grid of days, Monday first, with the weeks' totals at the right. `weeks` is
/// read again when the month or the data changes; `figure` is what the cells show, `selected`
/// the day that has the ring, `today` gets a dot, and `on_pick` hears a pressed day.
pub(crate) fn month_grid(
    weeks: impl Fn() -> Vec<CalendarWeek> + 'static,
    figure: Signal<Figure>,
    selected: Signal<Option<NaiveDate>>,
    currency: impl Fn() -> String + 'static,
    today: Memo<NaiveDate>,
    on_pick: impl Fn(NaiveDate) + Copy + 'static,
) -> impl IntoElement {
    let header = view()
        .flex_row()
        .gap_2()
        .children(WEEKDAY_NAMES.iter().map(|name| {
            text(*name).flex_1().min_w(px(0.0)).px_2().text_xs().text_color(move || theme().muted_foreground)
        }))
        .child(text("Tuần").w(px(TOTAL_WIDTH)).px_2().text_xs().text_color(move || theme().muted_foreground));
    view().gap_2().child(header).child(dynamic(move || {
        let weeks = weeks();
        let today = today.get();
        let currency = currency();
        untracked(|| {
            view().gap_2().children(weeks.iter().map(|week| {
                view()
                    .flex_row()
                    .gap_2()
                    .children(week.days.iter().map(|&day| day_cell(day, figure, selected, &currency, today, on_pick)))
                    .child(week_total(week, figure, &currency))
            }))
        })
    }))
}

/// The short figure of a day in the small month: money without its sign for a currency, R for a
/// day with no money, `·` for a day with no trade, nothing for a day of the neighbouring months.
fn compact_label(day: &CalendarDay) -> String {
    match day.stat {
        Some(stat) if stat.unit() == Unit::R => format::r_text(Some(stat.r)),
        Some(stat) => format::money_whole(stat.net, Sign::Always, "").replace('$', ""),
        None if day.in_month => "·".to_owned(),
        None => String::new(),
    }
}

/// The weeks of the small month: those with a weekday of the month in them. A week made only of
/// the last weekend of the month before (a month that starts on a Sunday) has nothing to show.
fn compact_weeks(weeks: Vec<CalendarWeek>) -> Vec<CalendarWeek> {
    weeks.into_iter().filter(|week| week.days.iter().any(|d| d.in_month && !is_weekend(d.date))).collect()
}

/// A small month for the overview: the weekdays only, each day a short signed figure, a dashed
/// empty cell for a weekday that belongs to a neighbouring month. Pressing a day of the month tells
/// `on_pick`.
pub(crate) fn mini_month(
    weeks: impl Fn() -> Vec<CalendarWeek> + 'static,
    today: Memo<NaiveDate>,
    on_pick: impl Fn(NaiveDate) + Copy + 'static,
) -> impl IntoElement {
    let header = view().flex_row().gap_1().children(WEEKDAY_NAMES.iter().take(5).map(|name| {
        text(name.replace("Thứ ", "T"))
            .flex_1()
            .min_w(px(0.0))
            .text_xs()
            .text_center()
            .text_color(move || theme().muted_foreground)
    }));
    view().gap_1().child(header).child(dynamic(move || {
        let weeks = compact_weeks(weeks());
        let today = today.get();
        untracked(|| {
            view().gap_1().children(weeks.iter().map(|week| {
                view().flex_row().gap_1().children(week.days.iter().filter(|d| !is_weekend(d.date)).map(|&day| {
                    if !day.in_month {
                        return view()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(px(COMPACT_HEIGHT))
                            .rounded(px(6.0))
                            .border_1()
                            .border_dashed()
                            .border_color(move || theme().border)
                            .into_element();
                    }
                    let label = compact_label(&day);
                    let tone = day.stat.map_or(Tone::Neutral, |s| tone_of(s.result()));
                    let date = day.date;
                    button()
                        .cursor_pointer()
                        .on_press(move || on_pick(date))
                        .accessibility_label(format!("Xem ngày {} {}", date.format("%d/%m"), label))
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(COMPACT_HEIGHT))
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || edge(&day))
                        .bg(move || fill(&day))
                        .hover(|s| s.border_color(move || theme().ring))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(
                            text(label)
                                .text_xs()
                                .font_family(tokens::mono)
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_color(move || {
                                    if day.date == today && tone == Tone::Neutral {
                                        theme().primary
                                    } else if tone == Tone::Neutral {
                                        theme().muted_foreground
                                    } else {
                                        ink(tone)
                                    }
                                }),
                        )
                        .into_element()
                }))
            }))
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(net: f64, trades: usize, wins: usize) -> DayStat {
        DayStat { net, trades, wins, r: 1.6, r_trades: 2, money_trades: trades }
    }

    #[test]
    fn a_day_shows_the_figure_the_switch_asks_for() {
        let s = stat(330.4, 3, 2);
        assert_eq!(figure_of(&s, Figure::Net, "USD"), ("+$330".to_owned(), Tone::Profit));
        assert_eq!(figure_of(&s, Figure::R, "USD"), ("+1.6R".to_owned(), Tone::Profit));
        assert_eq!(figure_of(&s, Figure::Trades, "USD"), ("3 lệnh".to_owned(), Tone::Neutral));
        assert_eq!(note_of(&s, Figure::Net), "2/3 lệnh thắng");
        let no_r = DayStat { r_trades: 0, ..s };
        assert_eq!(figure_of(&no_r, Figure::R, "USD").0, "—");
    }

    #[test]
    fn a_day_and_a_week_without_money_show_their_r_where_the_money_would_be() {
        let r_day = DayStat { net: 0.0, trades: 2, wins: 1, r: 1.5, r_trades: 2, money_trades: 0 };
        assert_eq!(figure_of(&r_day, Figure::Net, "USD"), ("+1.5R".to_owned(), Tone::Profit));
        assert_eq!(figure_of(&r_day, Figure::Trades, "USD").0, "2 lệnh");
        let week = CalendarWeek {
            days: std::array::from_fn(|i| CalendarDay {
                date: NaiveDate::from_ymd_opt(2026, 4, 13).expect("a date")
                    + chrono::Days::new(u64::try_from(i).expect("a small index")),
                in_month: true,
                stat: None,
            }),
            net: 0.0,
            r: -0.4,
            trades: 3,
            unit: Unit::R,
        };
        assert_eq!(week_figure(&week, Figure::Net, "USD"), ("−0.4R".to_owned(), Tone::Loss));
        assert_eq!(week_figure(&week, Figure::Trades, "USD").0, "3 lệnh");
        let money_week = CalendarWeek { unit: Unit::Money, net: 120.0, ..week };
        assert_eq!(week_figure(&money_week, Figure::Net, "USD").0, "+$120");
    }

    #[test]
    fn the_small_month_leaves_out_a_week_with_no_weekday_of_the_month() {
        use std::collections::BTreeMap;
        // 1 November 2026 is a Sunday: the first row of the grid is the last week of October.
        let november = compact_weeks(tradelens_core::metrics::month_weeks(2026, 11, &BTreeMap::new()));
        let first = november.first().expect("the month has weeks");
        assert_eq!(
            first.days[0].date,
            NaiveDate::from_ymd_opt(2026, 11, 2).expect("a date"),
            "starts on its first Monday"
        );
        assert_eq!(november.len(), 5, "2-6, 9-13, 16-20, 23-27 and 30 November");
        // A month that starts in the middle of a week keeps that week, its early days outside the month.
        let june = compact_weeks(tradelens_core::metrics::month_weeks(2026, 6, &BTreeMap::new()));
        assert_eq!(june.first().map(|w| w.days[0].in_month), Some(true), "1 June 2026 is a Monday");
        let july = compact_weeks(tradelens_core::metrics::month_weeks(2026, 7, &BTreeMap::new()));
        assert_eq!(july.first().map(|w| w.days[0].in_month), Some(false), "1 July 2026 is a Wednesday");
    }

    #[test]
    fn a_small_day_shows_its_figure_in_the_unit_it_has() {
        let day = |stat: Option<DayStat>, in_month: bool| CalendarDay {
            date: NaiveDate::from_ymd_opt(2026, 6, 19).expect("a date"),
            in_month,
            stat,
        };
        assert_eq!(compact_label(&day(Some(stat(330.4, 3, 2)), true)), "+330");
        let r_only = DayStat { net: 0.0, trades: 1, wins: 1, r: 2.0, r_trades: 1, money_trades: 0 };
        assert_eq!(compact_label(&day(Some(r_only), true)), "+2.0R");
        assert_eq!(compact_label(&day(None, true)), "·");
        assert_eq!(compact_label(&day(None, false)), "");
    }

    #[test]
    fn a_flat_day_has_no_tone() {
        assert_eq!(tone_of(0.0), Tone::Neutral);
        assert_eq!(tone_of(-0.001), Tone::Neutral);
        assert_eq!(tone_of(-5.0), Tone::Loss);
    }
}
