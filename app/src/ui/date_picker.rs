//! A date chosen from a calendar in a popup, on `nsh_base`'s date picker: the month as a grid (Monday
//! first, the weekdays named in the language chosen), arrows and a month and a year view to move far, today and the
//! chosen day marked, a dot on the days the app marks, a "Today" button, the keyboard (arrows, Enter,
//! Escape). The app gives the trigger: the date's own text, or a field.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use nsh_base::{
    CalendarEntry, CalendarItemKind, DatePickerEvent, DateRangePreset, DateTime, PickedDate, TimeOfDay, TimePrecision,
    date_picker, input, theme,
};
use nsh_core::prelude::*;

use super::{
    icons::{Icon, icon},
    theme::{self as tokens, mono},
};
use crate::format;

/// The weekdays over the grid, Sunday first (as `nsh_base` numbers them).
const WEEKDAYS: [Weekday; 7] =
    [Weekday::Sun, Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat];

/// The weekday the grid starts on: Monday.
const MONDAY: u32 = 1;

/// Whether a picked date carries a time of day too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithTime {
    /// A day only.
    No,
    /// A day and a time, to the minute (a trade's opening, a missed setup's moment).
    Minute,
}

/// What a picked value is in the app: a day, and the time of day when the picker has one.
pub type Picked = NaiveDateTime;

/// What the date picker needs from the page.
pub struct DatePick {
    /// What opens the popup when pressed.
    pub label: SharedString,
    /// The chosen date (and time), or none.
    pub value: Box<dyn Fn() -> Option<NaiveDateTime>>,
    /// The days to mark with a dot (a day with trades, a day with a note).
    pub marked: Box<dyn Fn(NaiveDate) -> bool>,
    /// The day the "Today" button picks.
    pub today: Box<dyn Fn() -> NaiveDate>,
    /// Told when the trader picks.
    pub on_pick: Box<dyn Fn(NaiveDateTime)>,
    /// Whether a time of day goes with the day.
    pub time: WithTime,
}

impl std::fmt::Debug for DatePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatePick").field("label", &self.label).finish_non_exhaustive()
    }
}

impl DatePick {
    /// A picker of days. `marked` is asked for each day the calendar draws.
    pub fn day(
        label: impl Into<SharedString>,
        value: impl Fn() -> Option<NaiveDate> + 'static,
        today: impl Fn() -> NaiveDate + 'static,
        on_pick: impl Fn(NaiveDate) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            value: Box::new(move || value().map(|day| day.and_time(NaiveTime::MIN))),
            marked: Box::new(|_| false),
            today: Box::new(today),
            on_pick: Box::new(move |picked| on_pick(picked.date())),
            time: WithTime::No,
        }
    }

    /// A picker of a day and a time of day, to the minute.
    pub fn moment(
        label: impl Into<SharedString>,
        value: impl Fn() -> Option<NaiveDateTime> + 'static,
        today: impl Fn() -> NaiveDate + 'static,
        on_pick: impl Fn(NaiveDateTime) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            value: Box::new(value),
            marked: Box::new(|_| false),
            today: Box::new(today),
            on_pick: Box::new(on_pick),
            time: WithTime::Minute,
        }
    }

    /// Marks the days `marked` holds with a dot.
    #[must_use]
    pub fn marking(mut self, marked: impl Fn(NaiveDate) -> bool + 'static) -> Self {
        self.marked = Box::new(marked);
        self
    }
}

/// The date picker: `trigger` is what the trader presses to open the calendar.
pub fn date_pick(pick: DatePick, trigger: impl IntoElement) -> impl IntoElement {
    let DatePick { label, value, marked, today, on_pick, time } = pick;
    let marked = std::rc::Rc::new(marked);
    let shown = std::rc::Rc::new(value);
    let mut picker = date_picker()
        .accessibility_label(label)
        .first_day_of_week(MONDAY)
        .previous_accessibility_label(t!("ui.date.previous_month"))
        .next_accessibility_label(t!("ui.date.next_month"))
        .value({
            let shown = std::rc::Rc::clone(&shown);
            move || DateTime::Single(shown())
        })
        .offset(px(6.0))
        .popup_style(|s| {
            s.p_3()
                .gap_2()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().popover)
                .text_color(move || theme().popover_foreground)
                .shadow_md()
        })
        .presets_class("flex-row gap-2")
        .preset_class(
            "h-8 px-3 rounded-md text-sm font-medium border border-border hover:bg-accent focus-visible:ring-2",
        )
        .item_class(
            "flex items-center justify-center rounded-md text-sm min-w-9 h-9 hover:bg-accent \
             selected:bg-primary selected:text-primary-foreground data-today:font-bold data-outside:opacity-40 \
             focus-visible:ring-2 disabled:opacity-40",
        )
        .render_item(move |entry: &CalendarEntry| render_entry(entry, &marked))
        .presets(vec![DateRangePreset::single(t!("common.today"), today())])
        .trigger(trigger)
        .on_change(move |picked: &PickedDate| {
            if let Some(at) = picked.value.start() {
                on_pick(at);
            }
        })
        .on_open_change(|_: &DatePickerEvent| {});
    if time == WithTime::Minute {
        picker = picker
            .time_precision(Some(TimePrecision::Minute))
            .default_time(Some(TimeOfDay(NaiveTime::MIN)))
            .time_accessibility_label(t!("ui.date.time"))
            .time_label(text(t!("ui.date.time")).text_sm())
            // The segment being edited is marked, and the ring shows where the keyboard is: the hour,
            // the minute, the arrows and the digits all act on it.
            .segment_class(
                "px-1 rounded-sm tabular-nums selected:bg-primary selected:text-primary-foreground \
                 focus-visible:ring-2 hover:bg-accent",
            );
    }
    picker
}

/// One cell of the calendar as the app draws it: the names of the weekdays and months in the language
/// chosen, a dot under a marked day.
fn render_entry(entry: &CalendarEntry, marked: &std::rc::Rc<Box<dyn Fn(NaiveDate) -> bool>>) -> AnyElement {
    let muted = move || theme().muted_foreground;
    match entry.kind {
        CalendarItemKind::Weekday => {
            let name = usize::try_from(entry.value)
                .ok()
                .and_then(|i| WEEKDAYS.get(i))
                .map_or_else(String::new, |day| format::weekday_short(*day));
            text(name).text_xs().text_color(muted).into_element()
        }
        CalendarItemKind::MonthToggle => {
            text(month_of(entry.value, format::month_name)).text_sm().font_weight(FontWeight::SEMIBOLD).into_element()
        }
        CalendarItemKind::YearToggle | CalendarItemKind::Year => {
            text(entry.value.to_string()).text_sm().font_weight(FontWeight::SEMIBOLD).into_element()
        }
        CalendarItemKind::Month => text(month_of(entry.value, format::month_short)).text_sm().into_element(),
        CalendarItemKind::Previous => icon(Icon::ChevronLeft, 16.0).into_element(),
        CalendarItemKind::Next => icon(Icon::ChevronRight, 16.0).into_element(),
        CalendarItemKind::Day => {
            let date = entry.date.as_ref().and_then(|iso| NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok());
            let dot = date.is_some_and(|day| marked(day));
            view()
                .items_center()
                .child(text(entry.value.to_string()).text_sm().tabular_nums())
                .child(view().size(px(4.0)).rounded_full().bg(move || theme().primary).opacity(if dot {
                    1.0_f32
                } else {
                    0.0_f32
                }))
                .into_element()
        }
    }
}

/// The name of month `number` (1 to 12) as `name` gives it.
fn month_of(number: i32, name: fn(u32) -> String) -> String {
    u32::try_from(number).map_or_else(|_| String::new(), name)
}

/// The trigger of a date picker as a field: a calendar glyph and the text, which follows `shown`.
pub fn date_trigger(shown: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .focusable(true)
        .cursor_pointer()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(34.0))
        .px_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(Icon::Calendar, 16.0).text_color(move || theme().muted_foreground))
        .child(text(shown).text_sm().font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
}

/// The trigger of a date picker as a heading (the month of Lịch P/L): the text large, a ⌄ after it,
/// a fill under the pointer. `shown` is followed.
pub fn heading_trigger(shown: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .focusable(true)
        .cursor_pointer()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .min_w(px(150.0))
        .h(px(36.0))
        .px_2()
        .rounded(move || theme().radius)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(text(shown).text_xl().font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
        .child(icon(Icon::ChevronDown, 16.0).text_color(move || theme().muted_foreground))
}

/// What a trade's open or close is while it is typed: a day, and the time of day when it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MomentValue {
    pub day: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
}

impl MomentValue {
    /// A day and a time.
    pub fn of(at: NaiveDateTime) -> Self {
        Self { day: Some(at.date()), time: Some(at.time()) }
    }

    /// A day whose time of day is not known.
    pub fn of_day(day: NaiveDate) -> Self {
        Self { day: Some(day), time: None }
    }

    /// The day at its time, midnight when the time is not known; `None` while no day is chosen.
    pub fn at(self) -> Option<NaiveDateTime> {
        self.day.map(|day| day.and_time(self.time.unwrap_or(NaiveTime::MIN)))
    }
}

/// A day and a time of day the trader picks or types: the signals behind a [`moment_field`].
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    day: Signal<Option<NaiveDate>>,
    time: Signal<Option<NaiveTime>>,
    /// Whether the time typed is not one, until it is corrected.
    problem: Signal<bool>,
}

impl Moment {
    pub fn new(value: MomentValue) -> Self {
        Self { day: signal(value.day), time: signal(value.time), problem: signal(false) }
    }

    /// What is chosen now (not tracked: for a listener).
    pub fn value(&self) -> MomentValue {
        MomentValue { day: self.day.get_untracked(), time: self.time.get_untracked() }
    }

    /// What is chosen, tracked where it is read.
    pub fn tracked(&self) -> MomentValue {
        MomentValue { day: self.day.get(), time: self.time.get() }
    }

    /// What is wrong with the time typed, if anything.
    pub fn problem(&self) -> Option<String> {
        self.problem.get_untracked().then(|| t!("ui.date.bad_time"))
    }
}

/// A time of day typed as `0930`, `930`, `9:30`, `09:30`, `9.30`, `9h30` or just `9` (the hour); `None`
/// when `typed` is not a time between 00:00 and 23:59 (the field says how to type it).
pub fn parse_clock(typed: &str) -> Option<NaiveTime> {
    let typed = typed.trim();
    let (hour, minute) = if let Some((hour, minute)) = typed.split_once([':', '.', 'h', 'H']) {
        (hour.trim(), minute.trim().trim_end_matches(['p', 'P']))
    } else if typed.len() > 2 && typed.is_ascii() {
        typed.split_at(typed.len() - 2)
    } else {
        (typed, "00")
    };
    // An hour is one or two digits; the minutes are two (`9:5` is not `9:05`), or none.
    let number = |digits: &str, longest: usize| -> Option<u32> {
        (!digits.is_empty() && digits.len() <= longest && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    };
    let minute = if minute.is_empty() { Some(0) } else { number(minute, 2).filter(|_| minute.len() == 2) };
    let (Some(hour), Some(minute)) = (number(hour, 2), minute) else {
        return None;
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// The field for a time of day, next to the day: type `0930` or `9:30`, kept when Enter is pressed or the
/// field is left; a wrong time says why under the field. Empty means the time is not known.
fn time_input(label: impl Into<SharedString>, moment: Moment) -> impl IntoElement {
    let draft = signal(String::new());
    effect(move || draft.set(moment.time.get().map_or_default(|t| t.format("%H:%M").to_string())));
    let keep: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
        let typed = draft.get_untracked();
        if typed.trim().is_empty() {
            moment.problem.set(false);
            moment.time.set(None);
            return;
        }
        match parse_clock(&typed) {
            Some(time) => {
                moment.problem.set(false);
                moment.time.set(Some(time));
                draft.set(time.format("%H:%M").to_string());
            }
            None => moment.problem.set(true),
        }
    });
    let on_enter = std::rc::Rc::clone(&keep);
    view()
        .flex_row()
        .items_center()
        .h(px(34.0))
        .w(px(84.0))
        .px_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || if moment.problem.get() { theme().destructive } else { theme().input })
        .bg(move || theme().card)
        .focus_within(|s| s.border_color(move || theme().ring).outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            input()
                .flex_1()
                .label(label.into())
                .text_sm()
                .font_family(mono)
                .tabular_nums()
                .text_color(move || theme().foreground)
                .placeholder("HH:MM")
                .placeholder_color(move || color(theme().muted_foreground))
                .caret_color(move || color(theme().primary))
                .value(move || draft.get())
                .on_change(move |typed: &str| draft.set(typed.to_owned()))
                .on_submit(move |_: &str| on_enter())
                .on_blur(move || keep()),
        )
}

/// A day and a time of day, to enter either way: the day from the calendar (its popup has the time's
/// hour and minute too, set with the arrows or the digits), the time typed in the field beside it.
/// Both change `moment`.
pub fn moment_field(
    label: impl Into<SharedString>,
    time_label: impl Into<SharedString>,
    moment: Moment,
) -> impl IntoElement {
    let chosen = move || moment.tracked().at();
    let pick = DatePick::moment(
        label,
        chosen,
        || chrono::Local::now().date_naive(),
        move |at| {
            let unknown = moment.time.get_untracked().is_none() && at.time() == NaiveTime::MIN;
            moment.problem.set(false);
            moment.day.set(Some(at.date()));
            moment.time.set((!unknown).then_some(at.time()));
        },
    );
    let trigger = date_trigger(move || {
        moment.day.get().map_or_else(|| t!("ui.date.choose"), |day| day.format("%d/%m/%Y").to_string())
    });
    view()
        .gap_1()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(date_pick(pick, trigger))
                .child(time_input(time_label, moment)),
        )
        .child(dynamic(move || {
            moment.problem.get().then(|| text(t!("ui.date.bad_time")).text_xs().text_color(tokens::loss))
        }))
}

/// The date as the trigger of a page's title says it: `Thứ 7, 21/11/2026`.
#[must_use]
pub fn weekday_title(day: NaiveDate) -> String {
    let weekday = format::weekday_name(day.weekday());
    format!("{weekday}, {}", day.format("%d/%m/%Y"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_titled_by_its_weekday_in_the_language_chosen() {
        use crate::i18n::{Lang, with_text};
        let day = NaiveDate::from_ymd_opt(2026, 11, 21).expect("a date");
        assert_eq!(with_text(Lang::Vi, move || weekday_title(day)), "Thứ 7, 21/11/2026");
        assert_eq!(with_text(Lang::En, move || weekday_title(day)), "Saturday, 21/11/2026");
        assert_eq!(WEEKDAYS[0], Weekday::Sun, "nsh_base numbers Sunday 0");
    }

    #[test]
    fn a_time_is_typed_the_ways_a_trader_writes_it() {
        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0).expect("a time");
        for (typed, want) in [
            ("0930", at(9, 30)),
            ("930", at(9, 30)),
            ("9:30", at(9, 30)),
            ("09:30", at(9, 30)),
            (" 9.30 ", at(9, 30)),
            ("9h30", at(9, 30)),
            ("9", at(9, 0)),
            ("2359", at(23, 59)),
            ("00:00", at(0, 0)),
        ] {
            assert_eq!(parse_clock(typed), Some(want), "{typed:?}");
        }
    }

    #[test]
    fn a_time_that_is_not_one_says_how_to_type_it() {
        for typed in ["", "abc", "2460", "24:00", "9:5", "12:60", "99999", "9:30:15", "-1:30"] {
            assert_eq!(parse_clock(typed), None, "{typed:?}");
        }
        let said = crate::i18n::with_text(crate::i18n::Lang::Vi, || t!("ui.date.bad_time"));
        assert!(said.starts_with("Giờ không hợp lệ"), "{said}");
    }
}
