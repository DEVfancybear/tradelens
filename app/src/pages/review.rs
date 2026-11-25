//! Tổng kết & xem lại: câu hỏi tổng kết tuần/tháng, hàng chờ xem lại, mẫu ghi chú.
//! Design: `docs/design/screens/review.html`. The AI summary is not built yet: a disabled card.

use chrono::{Duration, NaiveDate};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    TradeKey,
    report::Privacy,
    review::{self, Period, PeriodReview, QueueReason, ReviewTemplate},
};

use super::{
    account_picker::account_picker,
    share::{self, PRIVACY_CHOICES, ShareTarget, privacy_label},
};
use crate::{
    format::{self, Sign},
    insight, month_report, nav,
    snapshot::Snapshot,
    store::{Reading, query, query_with, store, writer},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Variant, action, action_icon, action_icon_when, action_when, add_field, icon_action, rating, segmented,
            textarea,
        },
        date_picker::{DatePick, date_pick, date_trigger},
        icons::Icon,
        lines::{filled, line_list},
        list_item::{Column, mini_table},
        parts::{card, empty_state, page_with, stat_tile},
        queue::queue_row,
    },
};

const PERIODS: [Period; 2] = [Period::Week, Period::Month];

/// A period to show: the search sets it; the page applies it and clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Jump(Option<(Period, NaiveDate)>);

/// Shows the review of the `period` starting `start` the next time the page is on screen (or at once).
pub(super) fn jump(period: Period, start: NaiveDate) {
    global::<Jump>().set(Jump(Some((period, start))));
}

fn period_label(period: Period) -> &'static str {
    match period {
        Period::Week => "Tuần",
        Period::Month => "Tháng",
    }
}

fn template_name(period: Period) -> &'static str {
    match period {
        Period::Week => "Tổng kết tuần",
        Period::Month => "Tổng kết tháng",
    }
}

/// A day of the period before the one holding `day`.
fn before(period: Period, day: NaiveDate) -> NaiveDate {
    period.bounds(day).0 - Duration::days(1)
}

/// A day of the period after the one holding `day`.
fn after(period: Period, day: NaiveDate) -> NaiveDate {
    period.bounds(day).1 + Duration::days(1)
}

pub(crate) fn review() -> impl IntoElement {
    let period = signal(Period::Week);
    let store = store();
    let anchor = signal(insight::today_now(&store.snapshot_now()));
    // While the period shown holds today, it moves with today: the first snapshot (with the
    // server's offset) may arrive after the page, and midnight passes.
    let known_today = signal(anchor.get_untracked());
    effect(move || {
        let now = insight::today_now(&store.snapshot());
        let was = known_today.get_untracked();
        if now != was {
            known_today.set(now);
            if anchor.get_untracked() == was {
                anchor.set(now);
            }
        }
    });
    let requested = global::<Jump>();
    effect(move || {
        let Some((wanted, start)) = requested.get().0 else { return };
        batch(|| {
            period.set(wanted);
            anchor.set(start);
        });
        untracked(|| requested.set(Jump(None)));
    });
    page_with(
        "Tổng kết & xem lại",
        "Nhìn lại tuần/tháng, trả lời câu hỏi tổng kết và dọn hàng chờ xem lại.",
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(account_picker())
            .child(segmented("Kỳ", &PERIODS, period_label, move || period.get(), move |next| period.set(next)))
            .child(period_nav(period, anchor))
            .child(share_button(period, anchor)),
        view().gap_6().child(figures_row(period, anchor)).child(
            view()
                .flex_row()
                .flex_wrap()
                .items_start()
                .gap_6()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(560.0))
                        .gap_6()
                        .child(questions_card(period, anchor))
                        .child(highlights_card(period, anchor)),
                )
                .child(
                    view()
                        .w(px(400.0))
                        .gap_6()
                        .child(queue_card())
                        .child(report_card(period, anchor))
                        .child(ai_card())
                        .child(templates_card()),
                ),
        ),
    )
}

/// The arrows and the name of the period shown.
fn period_nav(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(icon_action(Icon::ChevronLeft, "Kỳ trước", Variant::Ghost, move || {
            anchor.set(before(period.get_untracked(), anchor.get_untracked()));
        }))
        // Any day of the week or month picks it: the period holding that day is shown.
        .child(date_pick(
            DatePick::day(
                "Chọn tuần hoặc tháng",
                move || Some(anchor.get()),
                || chrono::Local::now().date_naive(),
                move |day| anchor.set(day),
            ),
            date_trigger(move || period.get().title(period.get().bounds(anchor.get()).0)),
        ))
        .child(icon_action(Icon::ChevronRight, "Kỳ sau", Variant::Ghost, move || {
            anchor.set(after(period.get_untracked(), anchor.get_untracked()));
        }))
}

/// "Chia sẻ": the share dialog on the period shown.
fn share_button(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    action_icon(Icon::Share, "Chia sẻ", Variant::Secondary, move || {
        let shown = period.get_untracked();
        share::open(ShareTarget::Period(shown, shown.bounds(anchor.get_untracked()).0));
    })
}

/// The month's PDF report: how much money it shows, and the button that saves it. Only on the month tab.
fn report_card(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    let store = store();
    let month = move || Period::Month.bounds(anchor.get()).0;
    let extras = query_with(month, |db, account, start| month_report::load_extras(db, account, *start));
    let privacy = signal(Privacy::ShowMoney);
    let failure = signal(None::<String>);
    let note = signal(None::<String>);
    let export = move || {
        let Some(extras) = extras.value() else {
            return failure.set(Some(
                extras.error().unwrap_or_else(|| "Đang đọc dữ liệu của tháng, thử lại sau giây lát.".to_owned()),
            ));
        };
        let start = Period::Month.bounds(anchor.get_untracked()).0;
        let report = month_report::build(&store.snapshot_now(), store.accounts.get_untracked().id(), start, &extras);
        share::export_pdf(report, privacy.get_untracked(), move |result| match result {
            Ok(line) => batch(|| {
                failure.set(None);
                note.set(Some(line));
            }),
            Err(line) => failure.set(Some(line)),
        });
    };
    dynamic(move || {
        (period.get() == Period::Month).then(|| {
            untracked(|| {
                card(
                    "Báo cáo tháng",
                    view()
                        .gap_3()
                        .child(
                            text("PDF A4: tổng quan, đường balance, lịch, playbook, lỗi, lệnh bỏ lỡ và tổng kết của tháng.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(segmented(
                            "Mức hiển thị của PDF",
                            &PRIVACY_CHOICES,
                            privacy_label,
                            move || privacy.get(),
                            move |next| privacy.set(next),
                        ))
                        .child(dynamic(move || failure.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss()))))
                        .child(dynamic(move || {
                            note.get().map(|line| text(line).text_sm().text_color(move || theme().muted_foreground))
                        }))
                        .child(view().flex_row().child(action_icon(
                            Icon::Download,
                            "Xuất PDF tháng",
                            Variant::Secondary,
                            export,
                        ))),
                )
                .into_element()
            })
        })
    })
}

// --- the period's figures ----------------------------------------------------------------------

/// A closed trade as the review reads it.
#[derive(Debug, Clone, PartialEq)]
struct Closed {
    key: TradeKey,
    /// `XAUUSD Buy · 18/11`.
    title: String,
    setup: String,
    /// The day it closed, on the trader's clock.
    day: NaiveDate,
    net: f64,
    /// The money is known (a trade imported in R has none).
    money: bool,
    r: Option<f64>,
    mistakes: bool,
    journalled: bool,
    followed_plan: Option<bool>,
}

impl Closed {
    /// What decides whether the trade won or lost: its money, or its R when it has no money.
    fn score(&self) -> f64 {
        if self.money { self.net } else { self.r.unwrap_or(0.0) }
    }
}

fn closed_trades(snapshot: &Snapshot) -> Vec<Closed> {
    snapshot
        .rows
        .iter()
        .filter_map(|row| {
            let closed = row.closed()?;
            let journal = snapshot.journals.get(row.key());
            Some(Closed {
                key: row.key().clone(),
                title: format!("{} {} · {}", row.trade.symbol, row.trade.side.label(), closed.format("%d/%m")),
                setup: if row.setup.is_empty() { "Không playbook".to_owned() } else { row.setup.clone() },
                day: closed.date(),
                net: row.trade.net(),
                money: row.trade.money_known(),
                r: row.r,
                mistakes: journal.is_some_and(|j| !j.mistakes.is_empty()),
                journalled: row.journaled,
                followed_plan: journal.and_then(|j| j.followed_plan),
            })
        })
        .collect()
}

/// The trades that closed in the period starting at `start`.
fn within(trades: &[Closed], period: Period, start: NaiveDate) -> Vec<&Closed> {
    let (first, last) = period.bounds(start);
    trades.iter().filter(|t| t.day >= first && t.day <= last).collect()
}

/// What the tiles show of a period.
#[derive(Debug, Clone, PartialEq, Default)]
struct Figures {
    trades: usize,
    wins: usize,
    net: f64,
    total_r: f64,
    r_trades: usize,
    journalled: usize,
    /// What the trades marked with a mistake lost, in money and in R, and how many they are.
    mistake_net: f64,
    mistake_r: f64,
    mistake_trades: usize,
    /// How many of the trades have money.
    money_trades: usize,
}

impl Figures {
    /// The period has trades and none of them has money: its results are in R.
    fn in_r(&self) -> bool {
        self.trades > 0 && self.money_trades == 0
    }
}

fn figures(trades: &[&Closed]) -> Figures {
    let mut f = Figures { trades: trades.len(), ..Figures::default() };
    for t in trades {
        f.net += t.net;
        f.wins += usize::from(t.score() > 0.0);
        f.money_trades += usize::from(t.money);
        f.journalled += usize::from(t.journalled);
        if let Some(r) = t.r {
            f.total_r += r;
            f.r_trades += 1;
        }
        if t.mistakes && t.score() < 0.0 {
            f.mistake_net += if t.money { t.net } else { 0.0 };
            f.mistake_r += t.r.unwrap_or(0.0);
            f.mistake_trades += 1;
        }
    }
    f
}

/// Which trade a row of "Lệnh nổi bật" is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Highlight {
    Best,
    Worst,
    /// Followed the plan and lost anyway: the loss to leave alone.
    RightAndLost,
}

impl Highlight {
    fn label(self) -> &'static str {
        match self {
            Self::Best => "Tốt nhất",
            Self::Worst => "Tệ nhất",
            Self::RightAndLost => "Đúng kế hoạch, vẫn thua",
        }
    }

    fn tone(self) -> Tone {
        match self {
            Self::Best => Tone::Profit,
            Self::Worst => Tone::Loss,
            Self::RightAndLost => Tone::Neutral,
        }
    }
}

fn highlights<'t>(trades: &[&'t Closed]) -> Vec<(Highlight, &'t Closed)> {
    let by_net = |a: &&&Closed, b: &&&Closed| a.score().total_cmp(&b.score());
    let mut rows = Vec::new();
    let best = trades.iter().max_by(by_net).filter(|t| t.score() > 0.0);
    let worst = trades.iter().min_by(by_net).filter(|t| t.score() < 0.0);
    if let Some(t) = best {
        rows.push((Highlight::Best, *t));
    }
    if let Some(t) = worst {
        rows.push((Highlight::Worst, *t));
    }
    let right = trades
        .iter()
        .filter(|t| t.followed_plan == Some(true) && t.score() < 0.0 && worst.is_none_or(|w| w.key != t.key))
        .min_by(by_net);
    if let Some(t) = right {
        rows.push((Highlight::RightAndLost, *t));
    }
    rows
}

/// Green above zero, red below, neutral at zero.
fn tone_of(value: f64) -> Tone {
    if value > 0.0 {
        Tone::Profit
    } else if value < 0.0 {
        Tone::Loss
    } else {
        Tone::Neutral
    }
}

/// `part` of `whole` as a percentage with no decimals; a dash when there is no whole.
fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "—".to_owned();
    }
    let count = |n: usize| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    format!("{:.0}%", 100.0 * count(part) / count(whole))
}

/// The line under the R tile: what each measured trade made on average.
fn r_note(total: f64, measured: usize) -> String {
    if measured == 0 {
        return "chưa có lệnh đo được R".to_owned();
    }
    let average = total / f64::from(u32::try_from(measured).unwrap_or(u32::MAX));
    format!("kỳ vọng {} / lệnh", format::r_average(Some(average)))
}

fn figures_row(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    let store = store();
    let now = computed(move || {
        let (first, _) = period.get().bounds(anchor.get());
        let snapshot = store.snapshot();
        figures(&within(&closed_trades(&snapshot), period.get(), first))
    });
    let money = move |value: f64| format::money(value, Sign::Always, &store.snapshot().currency);
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(stat_tile(
            "Net",
            move || {
                // No trade of the period has money: the R tile has its result, and an amount of zero would lie.
                if now.with(Figures::in_r) {
                    return ("—".to_owned(), Tone::Neutral);
                }
                let net = now.with(|f| f.net);
                (money(net), tone_of(net))
            },
            move || {
                now.with(|f| {
                    if f.in_r() {
                        format!("{} lệnh ghi theo R", f.trades)
                    } else if f.money_trades < f.trades {
                        format!("{} lệnh có tiền, {} lệnh chỉ có R", f.money_trades, f.trades - f.money_trades)
                    } else {
                        format!("{} lệnh", f.trades)
                    }
                })
            },
        ))
        .child(stat_tile(
            "R",
            move || {
                let (r, n) = now.with(|f| (f.total_r, f.r_trades));
                (format::r_text((n > 0).then_some(r)), tone_of(r))
            },
            move || now.with(|f| r_note(f.total_r, f.r_trades)),
        ))
        .child(stat_tile(
            "Thắng",
            move || (now.with(|f| share(f.wins, f.trades)), Tone::Neutral),
            move || now.with(|f| format!("{} / {}", f.wins, f.trades)),
        ))
        .child(stat_tile(
            "Đã ghi nhật ký",
            move || (now.with(|f| share(f.journalled, f.trades)), Tone::Neutral),
            move || now.with(|f| format!("{} / {} lệnh", f.journalled, f.trades)),
        ))
        .child(stat_tile(
            "Chi phí lỗi",
            move || {
                if now.with(Figures::in_r) {
                    let cost = now.with(|f| f.mistake_r);
                    return (format::r_text(Some(cost)), if cost < 0.0 { Tone::Loss } else { Tone::Neutral });
                }
                let cost = now.with(|f| f.mistake_net);
                (money(cost), if cost < 0.0 { Tone::Loss } else { Tone::Neutral })
            },
            move || {
                now.with(|f| {
                    if f.in_r() {
                        format!("{} lệnh có lỗi", f.mistake_trades)
                    } else {
                        format!("{} · {} lệnh", format::r_text(Some(f.mistake_r)), f.mistake_trades)
                    }
                })
            },
        ))
}

// --- the review form --------------------------------------------------------------------------

/// The review of one period as the form opens it, once its stored review and template are read.
#[derive(Debug, Clone, PartialEq)]
struct Source {
    period: Period,
    start: NaiveDate,
    review: PeriodReview,
}

/// The stored review of the period, or a blank one made from the template.
fn source_of(
    period: Period,
    start: NaiveDate,
    read: Option<&(Period, NaiveDate, Option<PeriodReview>)>,
    template: Option<&ReviewTemplate>,
    account: Option<i64>,
) -> Option<Source> {
    let (read_period, read_start, stored) = read?;
    if (*read_period, *read_start) != (period, start) {
        // The read is for the period shown before: wait for the next.
        return None;
    }
    let review = match stored {
        Some(stored) => stored.clone(),
        None => PeriodReview::blank(template.filter(|t| t.period == period)?, start, account),
    };
    Some(Source { period, start, review })
}

fn questions_card(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    let store = store();
    let loaded = query_with(
        move || {
            let p = period.get();
            (p, p.bounds(anchor.get()).0)
        },
        |db, account, &(p, start)| db.review(account, p, start).map(|r| (p, start, r)),
    );
    let template = query_with(move || period.get(), |db, _, &p| db.review_template(p));
    let source = computed(move || {
        let p = period.get();
        let start = p.bounds(anchor.get()).0;
        source_of(p, start, loaded.value().as_deref(), template.value().as_deref(), store.accounts.get().id())
    });
    // Which review the form is for. A change in what is stored of it (a write of this form, a sync) is
    // not another review: the form follows it, instead of being made again under the typing.
    let shown = computed(move || source.with(|s| s.as_ref().map(|s| (s.period, s.start, s.review.account_id))));
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text("Câu hỏi tổng kết").text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(dynamic(move || match shown.get() {
            // The form is made once for the review it shows, then follows what is stored of it.
            Some(_) => untracked(|| match source.get_untracked() {
                Some(first) => answers_form(first, source).into_element(),
                None => view().into_element(),
            }),
            None => untracked(|| {
                text(move || loaded.error().unwrap_or_else(|| "Đang tải…".to_owned()))
                    .text_sm()
                    .text_color(move || theme().muted_foreground)
                    .into_element()
            }),
        }))
}

/// One box per question, made again only when the number of questions changes, never while one
/// is typed in.
fn answer_rows(answers: Signal<Vec<(String, String)>>) -> impl IntoElement {
    let count = computed(move || answers.with(Vec::len));
    dynamic(move || {
        let count = count.get();
        untracked(|| {
            view().gap_4().children((0..count).map(move |index| {
                let prompt = answers.with(|a| a.get(index).map_or_default(|(p, _)| p.clone()));
                view().gap_1().child(text(prompt.clone()).text_xs().text_color(move || theme().muted_foreground)).child(
                    textarea(
                        prompt,
                        "",
                        2,
                        move || answers.with(|a| a.get(index).map_or_default(|(_, answer)| answer.clone())),
                        move |typed| {
                            answers.update(|a| {
                                if let Some(slot) = a.get_mut(index) {
                                    typed.clone_into(&mut slot.1);
                                }
                            });
                        },
                    ),
                )
            }))
        })
    })
}

/// The questions of one review, the discipline grade and the button that stores them.
fn answers_form(first: Source, latest: Memo<Option<Source>>) -> impl IntoElement {
    let write = writer();
    let answers = signal(first.review.answers.clone());
    let grade = signal(first.review.rating);
    let saved = signal(first.review.clone());
    // The stored review the edits are laid over (its id, once it has one).
    let base = signal(first.review);
    let busy = signal(false);
    let failure = signal(None::<String>);
    let current = move || PeriodReview { answers: answers.get(), rating: grade.get(), ..base.get() };
    let dirty = computed(move || current() != saved.get());
    // What is stored of this review changed (this form's own save read back, another window, a sync):
    // shown unless the trader has edits that are not saved, which are theirs.
    effect(move || {
        let Some(next) = latest.get() else { return };
        untracked(|| {
            if !dirty.get_untracked() {
                batch(|| {
                    answers.set(next.review.answers.clone());
                    grade.set(next.review.rating);
                    saved.set(next.review.clone());
                    base.set(next.review);
                });
            }
        });
    });
    let save = move || {
        let review = current();
        busy.set(true);
        failure.set(None);
        let stored = review.clone();
        write.run(
            move |db| db.save_review(&review),
            move |result| {
                busy.set(false);
                match result {
                    Ok(_) => saved.set(stored),
                    Err(message) => failure.set(Some(message)),
                }
            },
        );
    };
    view()
        .gap_4()
        .child(answer_rows(answers))
        .child(add_field("Thêm câu hỏi", "+ Thêm câu hỏi cho kỳ này, rồi Enter", move |typed| {
            let prompt = typed.trim().to_owned();
            if !prompt.is_empty() {
                answers.update(|a| a.push((prompt, String::new())));
            }
        }))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(text("Điểm kỷ luật của kỳ").text_xs().text_color(move || theme().muted_foreground))
                .child(rating("Điểm kỷ luật", move || grade.get(), move |next| grade.set(next))),
        )
        .child(dynamic(move || failure.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss()))))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    text(move || {
                        if busy.get() {
                            "Đang lưu…"
                        } else if dirty.get() {
                            "Chưa lưu — bấm Lưu tổng kết trước khi đổi kỳ"
                        } else {
                            "Đã lưu"
                        }
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(action_icon_when(
                    Icon::Check,
                    "Lưu tổng kết",
                    Variant::Primary,
                    move || dirty.get() && !busy.get(),
                    save,
                )),
        )
}

// --- the standout trades ------------------------------------------------------------------------

fn highlights_card(period: Signal<Period>, anchor: Signal<NaiveDate>) -> impl IntoElement {
    let store = store();
    card(
        "Lệnh nổi bật",
        dynamic(move || {
            let p = period.get();
            let start = p.bounds(anchor.get()).0;
            let snapshot = store.snapshot();
            untracked(|| {
                let trades = closed_trades(&snapshot);
                let inside = within(&trades, p, start);
                let picked = highlights(&inside);
                if picked.is_empty() {
                    return empty_state("Chưa có lệnh nổi bật", "Kỳ này chưa có lệnh thắng hay thua đã đóng.")
                        .into_element();
                }
                let currency = snapshot.currency.clone();
                mini_table(
                    vec![Column::start("Lệnh"), Column::start("Setup"), Column::end("R"), Column::end("Net")],
                    picked
                        .into_iter()
                        .map(|(kind, t)| {
                            let key = t.key.clone();
                            let r = format::r_text(t.r);
                            let net =
                                if t.money { format::money(t.net, Sign::Always, &currency) } else { "—".to_owned() };
                            vec![
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(chip(kind.label(), kind.tone()))
                                    .child(action(t.title.clone(), Variant::Ghost, move || {
                                        nav::show_trade(key.clone());
                                    }))
                                    .into_element(),
                                text(t.setup.clone()).text_sm().number_of_lines(1).into_element(),
                                pnl_text(move || r.clone()).into_element(),
                                pnl_text(move || net.clone()).into_element(),
                            ]
                        })
                        .collect(),
                )
                .into_element()
            })
        }),
    )
}

// --- the queue ------------------------------------------------------------------------------------

/// How many trades the queue's detail line names.
const NAMED: usize = 3;

/// The queue of the snapshot grouped by why: `(reason, the trades with it)`, in the order a
/// trader works through them.
fn queue_groups(snapshot: &Snapshot) -> (usize, Vec<(QueueReason, Vec<TradeKey>)>) {
    let queue = review::queue(snapshot.rows.iter().map(|row| (&row.trade, snapshot.journals.get(row.key()), row.r)));
    let total = queue.len();
    let groups = [QueueReason::NotJournalled, QueueReason::Flagged, QueueReason::LostMoreThanRisk]
        .into_iter()
        .filter_map(|reason| {
            let keys: Vec<TradeKey> =
                queue.iter().filter(|(_, reasons)| reasons.contains(&reason)).map(|(key, _)| key.clone()).collect();
            (!keys.is_empty()).then_some((reason, keys))
        })
        .collect();
    (total, groups)
}

fn queue_title(reason: QueueReason, count: usize) -> String {
    match reason {
        QueueReason::NotJournalled => format!("{count} lệnh chưa ghi nhật ký"),
        QueueReason::Flagged => format!("{count} lệnh đánh dấu “Xem lại sau”"),
        QueueReason::LostMoreThanRisk => format!("{count} lệnh lỗ quá 1R"),
    }
}

fn queue_action(reason: QueueReason) -> &'static str {
    match reason {
        QueueReason::NotJournalled => "Ghi ngay",
        QueueReason::Flagged => "Mở",
        QueueReason::LostMoreThanRisk => "Gắn lỗi",
    }
}

fn queue_card() -> impl IntoElement {
    let store = store();
    view()
        .p_5()
        .gap_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(dynamic(move || {
            let snapshot = store.snapshot();
            untracked(|| {
                let (total, groups) = queue_groups(&snapshot);
                let titles: std::collections::HashMap<TradeKey, String> = snapshot
                    .rows
                    .iter()
                    .map(|row| (row.key().clone(), format!("{} {}", row.trade.symbol, row.opened().format("%d/%m"))))
                    .collect();
                view()
                    .gap_2()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                text(format!("Hàng chờ xem lại · {total}")).text_sm().font_weight(FontWeight::SEMIBOLD),
                            )
                            .when(total > 0, |row| row.child(chip(total.to_string(), Tone::Primary))),
                    )
                    .when(groups.is_empty(), |v| {
                        v.child(
                            text("Hàng chờ trống: không còn lệnh nào cần xem lại.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                    })
                    .children(groups.into_iter().map(|(reason, keys)| {
                        let names: Vec<&str> =
                            keys.iter().take(NAMED).filter_map(|k| titles.get(k).map(String::as_str)).collect();
                        let more = keys.len().saturating_sub(NAMED);
                        let detail = if more > 0 {
                            format!("{} và {more} lệnh nữa", names.join(", "))
                        } else {
                            names.join(", ")
                        };
                        let first = keys.first().cloned();
                        queue_row(queue_title(reason, keys.len()), detail, queue_action(reason), move || {
                            if let Some(key) = first.clone() {
                                nav::show_trade(key);
                            }
                        })
                    }))
                    .into_element()
            })
        }))
}

// --- the AI summary and the templates ----------------------------------------------------------------

/// The AI summary is not built yet: shown, and off.
fn ai_card() -> impl IntoElement {
    card(
        "Tóm tắt kỳ bằng AI",
        view()
            .gap_3()
            .child(chip("Chưa làm", Tone::Neutral))
            .child(
                text("Tắt mặc định. Khi có, chỉ gửi R, tag, lỗi và ghi chú — không gửi số tiền — để AI gợi ý điều cần tự kiểm tra.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground),
            )
            .child(view().flex_row().child(action_when("Tạo tóm tắt", Variant::Secondary, || false, || {}))),
    )
}

fn templates_card() -> impl IntoElement {
    let week = query(|db, _| db.review_template(Period::Week));
    let month = query(|db, _| db.review_template(Period::Month));
    let editing = signal(None::<Period>);
    card(
        "Mẫu ghi chú",
        view()
            .gap_2()
            .child(template_row(Period::Week, week, editing))
            .child(template_row(Period::Month, month, editing))
            .child(dynamic(move || {
                let period = editing.get()?;
                let source = if period == Period::Week { week } else { month };
                // Only pressing the pencil builds the editor: a re-read of the template while its
                // questions are being typed must not throw the typing away.
                untracked(|| {
                    let template = source.value().map(|t| ReviewTemplate::clone(&t))?;
                    Some(template_editor(template, editing).into_element())
                })
            })),
    )
}

/// A row of the templates list: its name, how many questions it asks, and the pencil.
fn template_row(
    period: Period,
    template: Reading<ReviewTemplate>,
    editing: Signal<Option<Period>>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(text(template_name(period)).text_sm())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || {
                        template.value().map_or_else(String::new, |t| {
                            let default = *t == ReviewTemplate::default_for(period);
                            format!("{} câu{}", t.prompts.len(), if default { " · mặc định" } else { "" })
                        })
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(icon_action(
                    Icon::Edit,
                    format!("Sửa mẫu {}", template_name(period)),
                    Variant::Ghost,
                    move || {
                        editing.set(Some(period));
                    },
                )),
        )
}

/// The questions of a template, edited as a list.
fn template_editor(template: ReviewTemplate, editing: Signal<Option<Period>>) -> impl IntoElement {
    let write = writer();
    let period = template.period;
    let prompts = signal(template.prompts);
    let failure = signal(None::<String>);
    let save = move || {
        let kept = filled(&prompts.get_untracked());
        if kept.is_empty() {
            return failure.set(Some("Cần ít nhất một câu hỏi.".to_owned()));
        }
        write.run(
            move |db| db.save_review_template(&ReviewTemplate { period, prompts: kept }),
            move |result| match result {
                Ok(()) => editing.set(None),
                Err(message) => failure.set(Some(message)),
            },
        );
    };
    view()
        .gap_3()
        .pt_3()
        .child(
            text(format!("Câu hỏi của “{}”", template_name(period)))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(line_list("Câu hỏi", "Thêm câu hỏi", move || prompts.get(), move |next| prompts.set(next)))
        .child(dynamic(move || failure.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss()))))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(action("Mặc định", Variant::Ghost, move || {
                    prompts.set(ReviewTemplate::default_for(period).prompts);
                }))
                .child(action("Huỷ", Variant::Ghost, move || editing.set(None)))
                .child(action_icon(Icon::Check, "Lưu mẫu", Variant::Secondary, save)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a test date")
    }

    fn closed(position: i64, on: &str, net: f64, r: Option<f64>) -> Closed {
        Closed {
            key: TradeKey { account_id: 1, position_id: position, segment: 0 },
            title: format!("T{position}"),
            setup: String::new(),
            day: day(on),
            net,
            money: true,
            r,
            mistakes: false,
            journalled: false,
            followed_plan: None,
        }
    }

    #[test]
    fn periods_step_to_their_neighbours() {
        assert_eq!(Period::Week.bounds(before(Period::Week, day("2026-11-18"))).0, day("2026-11-09"));
        assert_eq!(Period::Week.bounds(after(Period::Week, day("2026-11-18"))).0, day("2026-11-23"));
        assert_eq!(Period::Month.bounds(before(Period::Month, day("2026-01-15"))).0, day("2025-12-01"));
        assert_eq!(Period::Month.bounds(after(Period::Month, day("2026-12-15"))).0, day("2027-01-01"));
    }

    #[test]
    fn only_the_trades_closed_in_the_period_count() {
        let trades = [
            closed(1, "2026-11-15", 5.0, None),
            closed(2, "2026-11-16", -10.0, Some(-1.0)),
            closed(3, "2026-11-22", 30.0, Some(2.0)),
        ];
        let inside = within(&trades, Period::Week, day("2026-11-16"));
        assert_eq!(inside.len(), 2);
        let f = figures(&inside);
        assert_eq!((f.trades, f.wins), (2, 1));
        assert!((f.net - 20.0).abs() < 1e-9);
        assert!((f.total_r - 1.0).abs() < 1e-9);
        assert_eq!(f.r_trades, 2);
    }

    #[test]
    fn mistakes_cost_what_the_marked_losers_lost() {
        let mut a = closed(1, "2026-11-16", -50.0, Some(-1.0));
        a.mistakes = true;
        let mut b = closed(2, "2026-11-17", 40.0, Some(1.0));
        b.mistakes = true;
        let f = figures(&[&a, &b]);
        assert_eq!(f.mistake_trades, 1, "a winning trade with a mistake costs nothing");
        assert!((f.mistake_net + 50.0).abs() < 1e-9);
        assert!((f.mistake_r + 1.0).abs() < 1e-9);
    }

    #[test]
    fn highlights_pick_the_best_the_worst_and_the_loss_that_followed_the_plan() {
        let a = closed(1, "2026-11-16", 100.0, None);
        let b = closed(2, "2026-11-17", -80.0, None);
        let mut c = closed(3, "2026-11-18", -20.0, None);
        c.followed_plan = Some(true);
        let mut d = closed(4, "2026-11-19", -90.0, None);
        d.followed_plan = Some(true);
        let all = [&a, &b, &c, &d];
        let picked: Vec<(Highlight, i64)> = highlights(&all).into_iter().map(|(h, t)| (h, t.key.position_id)).collect();
        assert_eq!(picked, [(Highlight::Best, 1), (Highlight::Worst, 4), (Highlight::RightAndLost, 3)]);
        assert_eq!(highlights(&[]).len(), 0, "no trades, nothing to show");
        let flat = closed(5, "2026-11-16", 0.0, None);
        assert!(highlights(&[&flat]).is_empty(), "a flat trade is neither best nor worst");
    }

    #[test]
    fn a_review_form_waits_for_the_period_it_was_asked_for() {
        let template = ReviewTemplate::default_for(Period::Week);
        let start = day("2026-11-16");
        let read = (Period::Week, start, None);
        let source = source_of(Period::Week, start, Some(&read), Some(&template), Some(2)).expect("a blank review");
        assert_eq!(source.review.answers.len(), 4);
        assert_eq!(source.review.account_id, Some(2));
        assert!(
            source_of(Period::Week, day("2026-11-23"), Some(&read), Some(&template), None).is_none(),
            "an older read"
        );
        assert!(source_of(Period::Week, start, None, Some(&template), None).is_none(), "nothing read yet");
        assert!(source_of(Period::Month, start, Some(&read), Some(&template), None).is_none());
        let stored = PeriodReview { rating: 4, ..PeriodReview::blank(&template, start, None) };
        let read = (Period::Week, start, Some(stored.clone()));
        assert_eq!(
            source_of(Period::Week, start, Some(&read), None, None).map(|s| s.review),
            Some(stored),
            "a stored review needs no template"
        );
    }

    #[test]
    fn tiles_read_shares_tones_and_averages() {
        assert_eq!(share(13, 21), "62%");
        assert_eq!(share(0, 0), "—");
        assert_eq!(tone_of(1.0), Tone::Profit);
        assert_eq!(tone_of(-0.5), Tone::Loss);
        assert_eq!(tone_of(0.0), Tone::Neutral);
        assert_eq!(r_note(3.0, 4), "kỳ vọng +0.75R / lệnh");
        assert_eq!(r_note(0.0, 0), "chưa có lệnh đo được R");
    }

    #[test]
    fn the_queue_names_what_to_do() {
        assert_eq!(queue_title(QueueReason::NotJournalled, 3), "3 lệnh chưa ghi nhật ký");
        assert_eq!(queue_action(QueueReason::Flagged), "Mở");
    }
}
