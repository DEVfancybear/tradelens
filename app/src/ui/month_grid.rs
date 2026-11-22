//! The month grid: one cell per day coloured by what the day made, with a column of weekly
//! totals. The full grid (Lịch P/L) picks a day; the compact one (Tổng quan) only shows the
//! weekdays.

use chrono::{Datelike, NaiveDate, Weekday};
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tradelens_core::metrics::{CalendarDay, CalendarWeek, DayStat};

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
            Self::Net => "Net $",
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
    match day.stat.map(|s| tone_of(s.net)) {
        Some(Tone::Profit) => profit_soft(),
        Some(Tone::Loss) => loss_soft(),
        _ => theme().background,
    }
}

fn edge(day: &CalendarDay) -> AdaptiveColor {
    match day.stat.map(|s| tone_of(s.net)) {
        Some(Tone::Profit) => profit().opacity(0.45),
        Some(Tone::Loss) => loss().opacity(0.45),
        _ => theme().border,
    }
}

/// The big figure of a day and its tone.
fn figure_of(stat: &DayStat, figure: Figure, currency: &str) -> (String, Tone) {
    match figure {
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

/// The weekly total at the right of a full row.
fn week_total(week: &CalendarWeek, currency: &str) -> impl IntoElement + use<> {
    let net = if week.trades == 0 { "—".to_owned() } else { format::money_whole(week.net, Sign::Always, currency) };
    let tone = if week.trades == 0 { Tone::Neutral } else { tone_of(week.net) };
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
                .text_color(move || ink(tone)),
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
        .on_press(move || on_pick(date))
        .accessibility_label(format!("Xem ngày {}", date.format("%d/%m")))
        .flex_1()
        .min_w(px(0.0))
        .h(px(CELL_HEIGHT))
        .p_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(edge(&day))
        .bg(fill(&day))
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
                    .child(week_total(week, &currency))
            }))
        })
    }))
}

/// A small month for the overview: the weekdays only, each day a short signed figure.
pub(crate) fn mini_month(weeks: impl Fn() -> Vec<CalendarWeek> + 'static, today: Memo<NaiveDate>) -> impl IntoElement {
    let header = view().flex_row().gap_1().children(WEEKDAY_NAMES.iter().take(5).map(|name| {
        text(name.replace("Thứ ", "T"))
            .flex_1()
            .min_w(px(0.0))
            .text_xs()
            .text_center()
            .text_color(move || theme().muted_foreground)
    }));
    view().gap_1().child(header).child(dynamic(move || {
        let weeks = weeks();
        let today = today.get();
        untracked(|| {
            view().gap_1().children(weeks.iter().map(|week| {
                view().flex_row().gap_1().children(week.days.iter().filter(|d| !is_weekend(d.date)).map(|&day| {
                    let label = match day.stat {
                        Some(stat) => format::money_whole(stat.net, Sign::Always, "").replace('$', ""),
                        None if day.in_month => "·".to_owned(),
                        None => String::new(),
                    };
                    let tone = day.stat.map_or(Tone::Neutral, |s| tone_of(s.net));
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(COMPACT_HEIGHT))
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(edge(&day))
                        .bg(fill(&day))
                        .opacity(if day.in_month { 1.0_f32 } else { 0.0_f32 })
                        .accessible(true)
                        .label(format!("{} {}", day.date.format("%d/%m"), label))
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
                }))
            }))
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(net: f64, trades: usize, wins: usize) -> DayStat {
        DayStat { net, trades, wins, r: 1.6, r_trades: 2 }
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
    fn a_flat_day_has_no_tone() {
        assert_eq!(tone_of(0.0), Tone::Neutral);
        assert_eq!(tone_of(-0.001), Tone::Neutral);
        assert_eq!(tone_of(-5.0), Tone::Loss);
    }
}
