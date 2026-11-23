//! A date chosen from a calendar in a popup, on `nsh_base`'s date picker: the month as a grid (Monday
//! first, the weekdays named in Vietnamese), arrows and a month and a year view to move far, today and the
//! chosen day marked, a dot on the days the app marks, a "Hôm nay" button, the keyboard (arrows, Enter,
//! Escape). The app gives the trigger: the date's own text, or a field.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use nsh_base::{
    CalendarEntry, CalendarItemKind, DatePickerEvent, DateRangePreset, DateTime, PickedDate, TimeOfDay, TimePrecision,
    date_picker, theme,
};
use nsh_core::prelude::*;

use super::icons::{Icon, icon};

/// The names of the weekdays over the grid, Sunday first (as `nsh_base` numbers them).
const WEEKDAY_NAMES: [&str; 7] = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"];

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
    /// The day the "Hôm nay" button picks.
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
        .previous_accessibility_label("Tháng trước")
        .next_accessibility_label("Tháng sau")
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
        .presets(vec![DateRangePreset::single("Hôm nay", today())])
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
            .time_accessibility_label("Giờ")
            .time_label(text("Giờ").text_sm());
    }
    picker
}

/// One cell of the calendar as the app draws it: Vietnamese names for the weekdays, months and years, a
/// dot under a marked day.
fn render_entry(entry: &CalendarEntry, marked: &std::rc::Rc<Box<dyn Fn(NaiveDate) -> bool>>) -> AnyElement {
    let muted = move || theme().muted_foreground;
    match entry.kind {
        CalendarItemKind::Weekday => {
            let name = usize::try_from(entry.value).ok().and_then(|i| WEEKDAY_NAMES.get(i)).copied().unwrap_or("");
            text(name).text_xs().text_color(muted).into_element()
        }
        CalendarItemKind::MonthToggle => {
            text(format!("Tháng {}", entry.value)).text_sm().font_weight(FontWeight::SEMIBOLD).into_element()
        }
        CalendarItemKind::YearToggle | CalendarItemKind::Year => {
            text(entry.value.to_string()).text_sm().font_weight(FontWeight::SEMIBOLD).into_element()
        }
        CalendarItemKind::Month => text(format!("Th {}", entry.value)).text_sm().into_element(),
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

/// The date as the trigger of a page's title says it: `Thứ 7, 21/11/2026`.
#[must_use]
pub fn weekday_title(day: NaiveDate) -> String {
    let weekday = tradelens_core::time::weekday_vi(day.weekday());
    format!("{weekday}, {}", day.format("%d/%m/%Y"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_titled_by_its_weekday_in_vietnamese() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 21).expect("a date");
        assert_eq!(weekday_title(day), "Thứ 7, 21/11/2026");
        assert_eq!(WEEKDAY_NAMES[0], "CN");
    }
}
