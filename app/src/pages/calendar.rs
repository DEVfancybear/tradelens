//! Lịch P/L: Lãi lỗ theo từng ngày, tô màu theo kết quả.
//! Design: `docs/design/screens/calendar.html`.

use chrono::{Datelike, NaiveDate};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{DayNote, DbError, Rules, metrics, time::weekday_vi};

use super::account_picker::account_picker;
use crate::{
    format::{self, MoneyColumn, Position, Sign},
    insight, nav,
    store::{Load, query, query_with, store},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{Variant, action_icon, icon_action, segmented},
        icons::Icon,
        layout::{columns, titled_card},
        month_grid::{Figure, month_grid},
        parts::{empty_state, page_with, property_row},
        theme::{self as tokens, warning},
    },
};

/// The month the grid shows: a year and a month from 1 to 12.
type Month = (i32, u32);

fn month_of(day: NaiveDate) -> Month {
    (day.year(), day.month())
}

fn previous(month: Month) -> Month {
    if month.1 == 1 { (month.0 - 1, 12) } else { (month.0, month.1 - 1) }
}

fn next(month: Month) -> Month {
    if month.1 == 12 { (month.0 + 1, 1) } else { (month.0, month.1 + 1) }
}

/// The month as the calendar names it: `Tháng 6, 2026`.
fn month_name((year, month): Month) -> String {
    format!("Tháng {month}, {year}")
}

pub(crate) fn calendar() -> impl IntoElement {
    let store = store();
    let opened_on = insight::today_now(&store.snapshot_now());
    let today = computed(move || insight::today_now(&store.snapshot()));
    let month = signal(month_of(opened_on));
    let selected = signal(Some(opened_on));
    let figure = signal(Figure::Net);
    let days = computed(move || insight::daily_results(&store.snapshot()));
    // Once the data is read, a month with no trades gives way to the latest one that has some
    // ("Hôm nay" still goes back to today).
    let settled = stored(false);
    effect(move || {
        if store.load_state() != Load::Ready {
            return;
        }
        let found = days.get();
        untracked(|| {
            if settled.get() {
                return;
            }
            settled.replace(true);
            let open = insight::opening_day(&found, today.get());
            batch(|| {
                month.set(month_of(open));
                selected.set(Some(open));
            });
        });
    });

    page_with(
        "Lịch P/L",
        "Lãi lỗ theo ngày đóng lệnh · bấm một ngày để xem lệnh và nhật ký",
        account_picker(),
        columns(
            month_card(month, selected, figure, days, today),
            view().gap_6().child(day_card(selected)).child(month_totals(month, days)),
        ),
    )
}

fn month_card(
    month: Signal<Month>,
    selected: Signal<Option<NaiveDate>>,
    figure: Signal<Figure>,
    days: Memo<std::collections::BTreeMap<NaiveDate, metrics::DayStat>>,
    today: Memo<NaiveDate>,
) -> impl IntoElement {
    let store = store();
    let toolbar = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(icon_action(Icon::ChevronLeft, "Tháng trước", Variant::Ghost, move || {
                    month.update(|m| *m = previous(*m));
                }))
                .child(
                    text(move || month_name(month.get()))
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .min_w(px(150.0))
                        .text_center(),
                )
                .child(icon_action(Icon::ChevronRight, "Tháng sau", Variant::Ghost, move || {
                    month.update(|m| *m = next(*m));
                }))
                .child(action_icon(Icon::Calendar, "Hôm nay", Variant::Secondary, move || {
                    let now = today.get_untracked();
                    batch(|| {
                        month.set(month_of(now));
                        selected.set(Some(now));
                    });
                })),
        )
        .child(segmented("Hiển thị", &Figure::ALL, Figure::label, move || figure.get(), move |next| figure.set(next)));

    let weeks = move || {
        let (year, month) = month.get();
        days.with(|d| metrics::month_weeks(year, month, d))
    };
    let legend = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_4()
        .child(legend_mark("Ngày lãi", Tone::Profit))
        .child(legend_mark("Ngày lỗ", Tone::Loss))
        .child(
            text("Viền teal: ngày đang xem · chấm teal: hôm nay")
                .text_xs()
                .text_color(move || theme().muted_foreground),
        );
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(toolbar)
        .child(month_grid(
            weeks,
            figure,
            selected,
            move || store.snapshot().currency.clone(),
            today,
            move |day| {
                // Picking a day of a neighbouring month shows that month too.
                batch(|| {
                    selected.set(Some(day));
                    if month_of(day) != month.get_untracked() {
                        month.set(month_of(day));
                    }
                });
            },
        ))
        .child(legend)
}

fn legend_mark(label: &'static str, tone: Tone) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .size(px(14.0))
                .rounded(px(3.0))
                .border_1()
                .bg(move || match tone {
                    Tone::Loss => tokens::loss_soft(),
                    _ => tokens::profit_soft(),
                })
                .border_color(move || match tone {
                    Tone::Loss => tokens::loss(),
                    _ => tokens::profit(),
                }),
        )
        .child(text(label).text_xs().text_color(move || theme().muted_foreground))
}

/// What the trader wrote for a day, read from the database when the day changes.
fn day_note_of(selected: Signal<Option<NaiveDate>>) -> crate::store::Reading<DayNote> {
    query_with(
        move || selected.get(),
        |db, account, day| {
            let Some(day) = day else { return Ok(DayNote::default()) };
            // The note of all accounts is kept under account 0, as the day journal keeps it.
            db.day_note(account.unwrap_or(0), *day)
        },
    )
}

/// The big figure of a day and the line beside it. A day with no money shows its R where the amount
/// would be, never an amount of zero.
fn day_summary(stat: &metrics::DayStat, currency: &str) -> (String, String) {
    let figure = if stat.trades == 0 {
        "—".to_owned()
    } else if stat.unit() == metrics::Unit::R {
        format::r_text(Some(stat.r))
    } else {
        format::money(stat.net, Sign::Always, currency)
    };
    let detail = if stat.trades == 0 {
        "Không có lệnh đóng".to_owned()
    } else if stat.unit() == metrics::Unit::R || stat.r_trades == 0 {
        format!("{} lệnh", stat.trades)
    } else {
        format!("{} lệnh · {}", stat.trades, format::r_text(Some(stat.r)))
    };
    (figure, detail)
}

fn day_card(selected: Signal<Option<NaiveDate>>) -> impl IntoElement {
    let store = store();
    let note = day_note_of(selected);
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(dynamic(move || {
            let Some(day) = selected.get() else {
                return empty_state("Chưa chọn ngày", "Bấm một ngày trong lịch để xem lệnh của ngày đó.")
                    .into_element();
            };
            untracked(|| {
                let snapshot = store.snapshot();
                let currency = snapshot.currency.clone();
                let trades = insight::trades_on(&snapshot, day);
                let stats = metrics::daily(
                    &trades.iter().map(|row| metrics::Sample { trade: &row.trade, r: row.r }).collect::<Vec<_>>(),
                    insight::summary_clock(&snapshot),
                );
                let stat = stats.get(&day).copied().unwrap_or_default();
                let title = format!("{}, {}", weekday_vi(day.weekday()), day.format("%d/%m"));
                let (figure, detail) = day_summary(&stat, &currency);
                // A money column only when some trade of the day has money; the R is its own column.
                let column = MoneyColumn::from(trades.iter().any(|row| row.trade.money_known()));
                let rows = trades.iter().map(|row| {
                    let key = row.key().clone();
                    let symbol = row.trade.symbol.clone();
                    let money =
                        row.trade.money_known().then(|| format::money(row.trade.net(), Sign::Always, &row.currency));
                    let (r, money) = format::result_cells(Position::Closed, money, row.r, column);
                    let side = row.trade.side.label().to_owned();
                    let tone = if side == "Buy" { Tone::Profit } else { Tone::Loss };
                    let time = match row.closed() {
                        Some(t) if row.trade.time_known() => t.format("%H:%M").to_string(),
                        _ => "—".to_owned(),
                    };
                    nsh_base::button()
                        .cursor_pointer()
                        .on_press(move || nav::show_trade(key.clone()))
                        .accessibility_label(format!("Mở nhật ký lệnh {symbol}"))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .h(px(36.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(move || theme().accent))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(
                            text(time)
                                .w(px(44.0))
                                .text_xs()
                                .font_family(tokens::mono)
                                .tabular_nums()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(text(symbol).flex_1().text_sm().font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                        .child(chip(side, tone))
                        .child(
                            view()
                                .w(px(if money.is_some() { 56.0 } else { 92.0 }))
                                .items_end()
                                .child(pnl_text(move || r.clone()).text_xs()),
                        )
                        .children(
                            money.map(|money| view().w(px(92.0)).items_end().child(pnl_text(move || money.clone()))),
                        )
                });
                view()
                    .gap_3()
                    .child(text(title).text_sm().font_weight(FontWeight::SEMIBOLD))
                    .child(
                        view()
                            .flex_row()
                            .items_end()
                            .gap_2()
                            .child(
                                text(figure.clone())
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .font_family(tokens::mono)
                                    .tabular_nums()
                                    .text_color(move || match Tone::of_amount(&figure) {
                                        Tone::Profit => tokens::profit(),
                                        Tone::Loss => tokens::loss(),
                                        _ => theme().foreground,
                                    }),
                            )
                            .child(text(detail).text_xs().pb_1().text_color(move || theme().muted_foreground)),
                    )
                    .children(rows)
                    .child(note_block(note))
                    .child(action_icon(Icon::Journal, "Mở nhật ký ngày", Variant::Secondary, move || {
                        nav::show_journal_day(day, store.accounts.get_untracked().id());
                    }))
                    .into_element()
            })
        }))
}

/// The day's plan and review, when the trader wrote them.
fn note_block(note: crate::store::Reading<DayNote>) -> impl IntoElement {
    dynamic(move || {
        let written = note.value().map(|n| (n.plan.trim().to_owned(), n.review.trim().to_owned()));
        let (plan, review) = written.unwrap_or_default();
        view()
            .gap_3()
            .children((!plan.is_empty()).then(|| note_text("Kế hoạch ngày", plan)))
            .children((!review.is_empty()).then(|| note_text("Tổng kết", review)))
    })
}

fn note_text(caption: &'static str, body: String) -> impl IntoElement {
    view()
        .gap_1()
        .child(text(caption).text_xs().text_color(move || theme().muted_foreground))
        .child(text(body).text_sm())
}

/// "Tháng này": what the month adds up to.
fn month_totals(
    month: Signal<Month>,
    days: Memo<std::collections::BTreeMap<NaiveDate, metrics::DayStat>>,
) -> impl IntoElement {
    let store = store();
    let rules = query(|db, account| -> Result<Vec<(i64, Rules)>, DbError> {
        let ids: Vec<i64> = match account {
            Some(id) => vec![id],
            None => db.accounts()?.iter().map(|a| a.id).collect(),
        };
        ids.into_iter().map(|id| Ok((id, db.rules(id)?))).collect()
    });
    let summary = move || {
        let (year, month) = month.get();
        days.with(|d| metrics::month_summary(year, month, d))
    };
    let money = move |value: f64, sign: Sign| format::money(value, sign, &store.snapshot().currency);
    // The month's figures are in money, or in R when none of its trades has money.
    let shown = move |value: f64| match summary().unit {
        metrics::Unit::Money => money(value, Sign::Always),
        metrics::Unit::R => format::r_text(Some(value)),
    };
    let day_of = move |best: Option<(NaiveDate, f64)>| {
        best.map_or_else(|| "—".to_owned(), |(date, result)| format!("{} · {}", shown(result), date.format("%d/%m")))
    };
    let violation_days = move || {
        let (year, month) = month.get();
        let Some(rules) = rules.value() else { return 0 };
        let found = insight::violations(&store.snapshot(), &rules);
        let mut days: Vec<NaiveDate> =
            found.iter().map(|v| v.day).filter(|d| (d.year(), d.month()) == (year, month)).collect();
        days.dedup();
        days.len()
    };
    // Titled with the month in view, not "this month": the page may be on any month.
    titled_card(
        move || month_name(month.get()),
        view(),
        view()
            .gap_2()
            .child(property_row("Kết quả tháng", pnl_text(move || shown(summary().result))))
            .child(property_row(
                "Ngày lãi / lỗ",
                text(move || {
                    let s = summary();
                    format!("{} / {}", s.green_days, s.red_days)
                })
                .font_family(tokens::mono)
                .tabular_nums()
                .pt_2()
                .text_sm(),
            ))
            .child(property_row("Ngày tốt nhất", pnl_text(move || day_of(summary().best_day))))
            .child(property_row("Ngày tệ nhất", pnl_text(move || day_of(summary().worst_day))))
            .child(property_row(
                "Lệnh trong tháng",
                text(move || summary().trades.to_string()).font_family(tokens::mono).tabular_nums().pt_2().text_sm(),
            ))
            .child(property_row(
                "Vi phạm quy tắc",
                text(move || match violation_days() {
                    0 => "Không có".to_owned(),
                    n => format!("{n} ngày"),
                })
                .font_family(tokens::mono)
                .tabular_nums()
                .pt_2()
                .text_sm()
                .text_color(move || if violation_days() > 0 { warning() } else { theme().foreground }),
            )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_step_across_the_year() {
        assert_eq!(previous((2026, 1)), (2025, 12));
        assert_eq!(previous((2026, 11)), (2026, 10));
        assert_eq!(next((2026, 12)), (2027, 1));
        assert_eq!(next((2026, 11)), (2026, 12));
        let day = NaiveDate::from_ymd_opt(2026, 11, 18).expect("a real date");
        assert_eq!(month_of(day), (2026, 11));
    }
}
