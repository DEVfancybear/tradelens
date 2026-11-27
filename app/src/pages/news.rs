//! Tin quan trọng: the red (high-impact) news of the week from Forex Factory's calendar, on the
//! machine's clock, with the next one counted down.
//! Design: `docs/design/screens/news.html`.

use std::{collections::BTreeMap, time::Duration};

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{db::NewsFetch, news::NewsEvent, time::weekday_vi};

use crate::{
    news::news,
    store::query_with,
    ui::{
        chips::{Tone, chip, figure_text},
        controls::{Variant, action_when},
        layout::titled_card,
        parts::{card, empty_state, error_line, page_with, warning_banner},
        theme::{self as tokens, profit},
    },
};

/// How often the countdown and the greying of past news move on.
const TICK: Duration = Duration::from_secs(20);

/// What the page reads: the week's red news and how the fetches went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Shown {
    events: Vec<NewsEvent>,
    fetch: NewsFetch,
}

pub(crate) fn news_page() -> impl IntoElement {
    let fetcher = news();
    let now = signal(Utc::now().timestamp_millis());
    set_interval(TICK, move || now.set(Utc::now().timestamp_millis()));
    let week = computed(move || local_week(now.get()));
    let shown = query_with(
        move || week.get(),
        |db, _, &(start_ms, end_ms)| Ok(Shown { events: db.news_between(start_ms, end_ms)?, fetch: db.news_fetch()? }),
    );

    let reload = action_when(
        move || if fetcher.busy() { "Đang tải…" } else { "Tải lại" },
        Variant::Secondary,
        move || !fetcher.busy(),
        move || fetcher.refresh(),
    );
    page_with(
        "Tin quan trọng",
        "Tin đỏ trong lịch tuần của Forex Factory · giờ theo máy này",
        reload,
        view()
            .gap_6()
            .child(dynamic(move || {
                let fetch = shown.value().map_or_default(|s| s.fetch.clone());
                let never = if fetcher.busy() {
                    "Đang tải lịch tin lần đầu…"
                } else {
                    "Chưa tải lịch tin lần nào."
                };
                status_line(&fetch, now.get(), never)
            }))
            .child(dynamic(move || shown.error().map(error_line)))
            .child(dynamic(move || {
                let read = shown.value()?;
                read.fetch.fetched_ms?;
                let at = now.get();
                Some(match read.events.iter().find(|e| e.at_ms > at) {
                    Some(next) => next_card(next, at).into_element(),
                    None => card(
                        "Sắp tới",
                        text("Tuần này không còn tin đỏ nào. Lịch tuần sau có từ Chủ nhật.")
                            .text_sm()
                            .text_color(move || theme().muted_foreground),
                    )
                    .into_element(),
                })
            }))
            .child(dynamic(move || {
                let (start_ms, end_ms) = week.get();
                let read = shown.value();
                let body = match read.as_deref() {
                    None => empty_state("Đang đọc lịch tin…", "").into_element(),
                    Some(Shown { events, fetch }) if events.is_empty() => {
                        let (title, hint) = if fetch.fetched_ms.is_some() {
                            ("Tuần này không có tin đỏ", "Lịch được tải lại mỗi 30 phút.")
                        } else {
                            ("Chưa có lịch tin", "Bấm Tải lại khi máy đã có mạng.")
                        };
                        empty_state(title, hint).into_element()
                    }
                    Some(Shown { events, .. }) => week_list(events, now.get()).into_element(),
                };
                titled_card(week_title(start_ms, end_ms), view(), body)
            })),
    )
}

/// The line under the header: when the shown feed was fetched, why the last fetch failed, or
/// `never` before the first fetch.
fn status_line(fetch: &NewsFetch, now_ms: i64, never: &'static str) -> AnyElement {
    match (&fetch.failure, fetch.fetched_ms) {
        (Some((failed_ms, why)), fetched) => {
            let shown = fetched.map_or_else(
                || " Chưa có bản nào trên máy.".to_owned(),
                |at| format!(" Đang hiện bản tải {}.", when(at, now_ms)),
            );
            warning_banner(format!(
                "Không tải được lịch tin {} ({}).{shown}",
                when(*failed_ms, now_ms),
                crate::news::failure_text(why)
            ))
            .into_element()
        }
        (None, Some(at)) => view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(view().size(px(8.0)).rounded(px(4.0)).bg(profit))
            .child(
                text(format!("Cập nhật {}", when(at, now_ms))).text_xs().text_color(move || theme().muted_foreground),
            )
            .into_element(),
        (None, None) => text(never).text_xs().text_color(move || theme().muted_foreground).into_element(),
    }
}

/// The next red news and how long until it is out.
fn next_card(event: &NewsEvent, now_ms: i64) -> impl IntoElement + use<> {
    let at = local(event.at_ms);
    let today = local(now_ms).date_naive();
    let left = countdown(event.at_ms - now_ms);
    card(
        "Sắp tới",
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(impact_mark())
            .child(chip(event.currency.clone(), Tone::Neutral))
            .child(text(event.title.clone()).text_lg().font_weight(FontWeight::SEMIBOLD).flex_1().min_w(px(160.0)))
            .child(
                view()
                    .items_end()
                    .gap_1()
                    .child(
                        figure_text(move || left.clone())
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens::loss),
                    )
                    .child(
                        text(format!("{} · {}", day_label(at.date_naive(), today), at.format("%H:%M")))
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    ),
            ),
    )
}

/// The week's news, under a heading per day; past news is greyed.
fn week_list(events: &[NewsEvent], now_ms: i64) -> impl IntoElement + use<> {
    let today = local(now_ms).date_naive();
    let mut days: BTreeMap<NaiveDate, Vec<&NewsEvent>> = BTreeMap::new();
    for event in events {
        days.entry(local(event.at_ms).date_naive()).or_default().push(event);
    }
    view().gap_4().children(days.into_iter().map(move |(day, list)| {
        view()
            .gap_1()
            .child(
                text(day_label(day, today))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || if day == today { theme().primary } else { theme().muted_foreground }),
            )
            .children(list.into_iter().map(|event| event_row(event, now_ms)))
    }))
}

/// One news: time, red mark, currency, title, forecast and previous; greyed once it is out at `now_ms`.
fn event_row(event: &NewsEvent, now_ms: i64) -> impl IntoElement + use<> {
    let past = event.at_ms <= now_ms;
    let time = local(event.at_ms).format("%H:%M").to_string();
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .opacity(if past { 0.5 } else { 1.0 })
        .child(figure_text(move || time.clone()).text_sm().w(px(44.0)))
        .child(impact_mark())
        .child(view().w(px(48.0)).child(chip(event.currency.clone(), Tone::Neutral)))
        .child(text(event.title.clone()).text_sm().flex_1().min_w(px(0.0)).number_of_lines(1))
        .child(figure_cell("Dự báo", event.forecast.clone()))
        .child(figure_cell("Trước", event.previous.clone()))
}

/// Forex Factory's red folder: a short bar in the colour of loss.
fn impact_mark() -> impl IntoElement {
    view().w(px(4.0)).h(px(18.0)).rounded(px(2.0)).bg(tokens::loss)
}

/// A labelled figure at the row's right; a dash when there is none.
fn figure_cell(label: &'static str, value: Option<String>) -> impl IntoElement {
    let shown = value.unwrap_or_else(|| "—".to_owned());
    view()
        .flex_row()
        .items_center()
        .justify_end()
        .gap_1()
        .w(px(108.0))
        .child(text(label).text_xs().text_color(move || theme().muted_foreground))
        .child(figure_text(move || shown.clone()).text_sm())
}

/// `instant` on the machine's clock.
fn local(at_ms: i64) -> DateTime<Local> {
    Local.timestamp_millis_opt(at_ms).single().unwrap_or_else(|| Utc::now().with_timezone(&Local))
}

/// The week holding `now_ms` on the machine's clock: Monday 00:00 to the next Monday, in Unix ms.
fn local_week(now_ms: i64) -> (i64, i64) {
    let today = local(now_ms).date_naive();
    let monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let start = |day: NaiveDate| {
        day.and_hms_opt(0, 0, 0)
            .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
            .map_or(now_ms, |at| at.timestamp_millis())
    };
    (start(monday), start(monday + chrono::Duration::days(7)))
}

/// The card's title: `Tuần này · 23/11 – 29/11`.
fn week_title(start_ms: i64, end_ms: i64) -> String {
    let first = local(start_ms).date_naive();
    let last = local(end_ms - 1).date_naive();
    format!("Tuần này · {} – {}", first.format("%d/%m"), last.format("%d/%m"))
}

/// A day's heading: `Hôm nay · Thứ 4, 25/11`, or without "Hôm nay".
fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    let name = format!("{}, {}", weekday_vi(day.weekday()), day.format("%d/%m"));
    if day == today { format!("Hôm nay · {name}") } else { name }
}

/// When something happened, from `now_ms`: `lúc 14:02 hôm nay` or `lúc 14:02 ngày 24/11`.
fn when(at_ms: i64, now_ms: i64) -> String {
    let at = local(at_ms);
    if at.date_naive() == local(now_ms).date_naive() {
        format!("lúc {} hôm nay", at.format("%H:%M"))
    } else {
        format!("lúc {} ngày {}", at.format("%H:%M"), at.format("%d/%m"))
    }
}

/// What is left until a news: `còn 2 ngày 3 giờ`, `còn 1 giờ 12 phút`, `còn 5 phút`.
fn countdown(left_ms: i64) -> String {
    let minutes = (left_ms.max(0) + 59_999) / 60_000;
    let (days, hours, mins) = (minutes / 1_440, minutes / 60 % 24, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("còn {mins} phút"),
        (0, _) if mins == 0 => format!("còn {hours} giờ"),
        (0, _) => format!("còn {hours} giờ {mins} phút"),
        (_, 0) => format!("còn {days} ngày"),
        _ => format!("còn {days} ngày {hours} giờ"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_reads_in_days_hours_and_minutes() {
        let minute = 60_000;
        assert_eq!(countdown(30_000), "còn 1 phút", "a part of a minute counts as one");
        assert_eq!(countdown(5 * minute), "còn 5 phút");
        assert_eq!(countdown(60 * minute), "còn 1 giờ");
        assert_eq!(countdown(72 * minute), "còn 1 giờ 12 phút");
        assert_eq!(countdown((2 * 24 + 3) * 60 * minute), "còn 2 ngày 3 giờ");
        assert_eq!(countdown(24 * 60 * minute), "còn 1 ngày");
    }

    #[test]
    fn the_week_runs_from_monday_on_the_machines_clock() {
        let thursday = Local.with_ymd_and_hms(2026, 11, 26, 15, 0, 0).single().expect("a local time");
        let (start, end) = local_week(thursday.timestamp_millis());
        assert_eq!(local(start).naive_local().to_string(), "2026-11-23 00:00:00");
        assert_eq!(local(end).naive_local().to_string(), "2026-11-30 00:00:00");
        assert_eq!(week_title(start, end), "Tuần này · 23/11 – 29/11");
    }

    #[test]
    fn today_is_named_in_the_day_heading() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 25).expect("a date");
        assert_eq!(day_label(day, day), "Hôm nay · Thứ 4, 25/11");
        assert_eq!(day_label(day, day.succ_opt().expect("a date")), "Thứ 4, 25/11");
    }
}
