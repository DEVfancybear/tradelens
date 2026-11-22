//! Nhật ký ngày: the plan before the session and the review after it, with the day's trades and
//! the rules they broke beside them. Design: `docs/design/screens/daily-journal.html`.

use std::{collections::HashMap, time::Duration};

use chrono::{Datelike, Days, NaiveDate, Weekday};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    ChecklistItem, DayNote, Db, DbError, Side, TradeKey,
    catalog::{CatalogKind, CustomValues, FieldDef, FieldScope, FieldValue},
    form::{BuiltinField, FormLayout},
    metrics,
    rules::{self, Violation},
    time::{format_day, weekday_vi},
};

use super::{
    account_picker::account_picker,
    account_trades::{self, AccountTrades},
};
use crate::{
    format::{self, Sign},
    nav,
    store::{Reading, Writer, query_with, writer},
    ui::{
        chips::{Tone, chip, figure_text, pnl_text},
        controls::{Choice, Variant, action, action_when, icon_action, select_value, textarea},
        icons::Icon,
        parts::{card, empty_state, page_with},
        properties::{PropertyEdits, PropertyList, property_list},
        selectable_card::selectable_card,
        theme::loss,
    },
};

/// How often unsaved edits are written (the page saves by itself, the button only hurries it).
const AUTOSAVE: Duration = Duration::from_secs(2);

/// One trade of the day as the summary lists it.
#[derive(Debug, Clone, PartialEq)]
struct DayTrade {
    key: TradeKey,
    /// When it opened, on the trader's clock.
    time: String,
    symbol: String,
    side: Side,
    closed: bool,
    net: f64,
    r: Option<f64>,
    currency: String,
    /// The rule it broke, when it broke one.
    violation: Option<String>,
}

/// One day of the week strip.
#[derive(Debug, Clone, PartialEq)]
struct WeekDay {
    day: NaiveDate,
    net: Option<f64>,
    written: bool,
}

/// What the page reads for one day.
#[derive(Debug, Clone, PartialEq)]
struct DayData {
    /// The account the note belongs to; 0 is all accounts.
    account: i64,
    day: NaiveDate,
    note: DayNote,
    values: CustomValues,
    layout: FormLayout,
    defs: Vec<FieldDef>,
    /// The options of the mood, archived ones left out.
    moods: Vec<(String, String)>,
    trades: Vec<DayTrade>,
    /// The rules broken that no single trade broke.
    day_violations: Vec<String>,
    /// The checklist items and how many of the day's trades confirmed each.
    checklist: Vec<(ChecklistItem, usize)>,
    week: Vec<WeekDay>,
    currency: String,
}

impl DayData {
    fn closed_net(&self) -> f64 {
        self.trades.iter().filter(|t| t.closed).map(|t| t.net).sum()
    }
}

/// The Monday-to-Sunday week `day` is in.
fn week_of(day: NaiveDate) -> Vec<NaiveDate> {
    let monday = day.week(Weekday::Mon).first_day();
    (0..7).filter_map(|n| monday.checked_add_days(Days::new(n))).collect()
}

/// The short name of a weekday on the strip: `T2`…`T7`, `CN`.
fn weekday_short(day: NaiveDate) -> &'static str {
    match day.weekday() {
        Weekday::Mon => "T2",
        Weekday::Tue => "T3",
        Weekday::Wed => "T4",
        Weekday::Thu => "T5",
        Weekday::Fri => "T6",
        Weekday::Sat => "T7",
        Weekday::Sun => "CN",
    }
}

/// The day as the header says it: `Thứ 6, 20/11/2026`.
fn day_title(day: NaiveDate) -> String {
    format!("{}, {}", weekday_vi(day.weekday()), format_day(day))
}

/// Adds the trades of `own` that fall on `day` to `sink` (with their opening time, to order by), each
/// marked with the rule it broke when `violations` holds one for it.
fn trades_of_day(own: &AccountTrades, day: NaiveDate, violations: &[Violation], sink: &mut Vec<(i64, DayTrade)>) {
    for trade in &own.trades {
        let at = trade.close_time.unwrap_or(trade.open_time);
        if own.clock.day(at) != day {
            continue;
        }
        sink.push((
            trade.open_time,
            DayTrade {
                key: trade.key.clone(),
                time: own.clock.local(trade.open_time).format("%H:%M").to_string(),
                symbol: trade.symbol.clone(),
                side: trade.side,
                closed: trade.is_closed(),
                net: trade.net(),
                r: None,
                currency: own.account.currency.clone(),
                violation: violations.iter().find(|v| v.trade.as_ref() == Some(&trade.key)).map(|v| v.kind.label()),
            },
        ));
    }
}

/// Reads the day `day` for the chosen account (`None`: all accounts).
fn load(db: &Db, account: Option<i64>, day: NaiveDate) -> Result<DayData, DbError> {
    let note_account = account.unwrap_or(0);
    let owned = account_trades::load(db, account)?;
    let journals: HashMap<TradeKey, tradelens_core::JournalEntry> = db.journals(account)?.into_iter().collect();

    let mut listed: Vec<(i64, DayTrade)> = Vec::new();
    let mut day_violations: Vec<String> = Vec::new();
    let mut per_day: HashMap<NaiveDate, f64> = HashMap::new();
    for own in &owned {
        let rules = db.rules(own.account.id)?;
        let samples = own.samples();
        let curve = own.curve();
        let found: Vec<Violation> =
            rules::evaluate(&rules, &samples, |ms| rules::balance_before(&curve, ms), own.clock)
                .into_iter()
                .filter(|v| v.day == day)
                .collect();
        day_violations.extend(found.iter().filter(|v| v.trade.is_none()).map(|v| v.kind.label()));
        trades_of_day(own, day, &found, &mut listed);
        for (date, stat) in metrics::daily(&samples, own.clock) {
            *per_day.entry(date).or_default() += stat.net;
        }
    }
    listed.sort_by_key(|(opened, _)| *opened);
    let mut trades: Vec<DayTrade> = listed.into_iter().map(|(_, t)| t).collect();
    for trade in &mut trades {
        let planned = journals.get(&trade.key).and_then(|j| j.planned_risk);
        trade.r = owned
            .iter()
            .flat_map(|o| o.trades.iter())
            .find(|t| t.key == trade.key)
            .filter(|t| t.is_closed())
            .and_then(|t| t.r_multiple(planned));
    }

    let items = match account {
        Some(id) => db.checklist(Some(id))?,
        None => db.checklist(None)?,
    };
    let checklist = items
        .into_iter()
        .map(|item| {
            let confirmed =
                trades.iter().filter(|t| journals.get(&t.key).is_some_and(|j| j.checklist.contains(&item.id))).count();
            (item, confirmed)
        })
        .collect();

    let mut week = Vec::new();
    for date in week_of(day) {
        let written = db.day_note(note_account, date)? != DayNote::default();
        week.push(WeekDay { day: date, net: per_day.get(&date).copied(), written });
    }

    let currencies: std::collections::BTreeSet<&str> = owned.iter().map(|o| o.account.currency.as_str()).collect();
    let currency = if currencies.len() == 1 {
        currencies.into_iter().next().unwrap_or_default().to_owned()
    } else {
        String::new()
    };
    let moods = db
        .catalog(CatalogKind::Emotion)?
        .into_iter()
        .filter(|item| !item.archived)
        .map(|item| (item.key, item.label))
        .collect();

    Ok(DayData {
        account: note_account,
        day,
        note: db.day_note(note_account, day)?,
        values: db.day_values(note_account, day)?,
        layout: db.form_layout(FieldScope::Day)?,
        defs: db.field_defs(FieldScope::Day)?,
        moods,
        trades,
        day_violations,
        checklist,
        week,
        currency,
    })
}

/// What the trader has typed for the day.
#[derive(Debug, Clone, PartialEq, Default)]
struct Edits {
    note: DayNote,
    values: CustomValues,
}

impl Edits {
    fn of(data: &DayData) -> Self {
        Self { note: data.note.clone(), values: data.values.clone() }
    }
}

/// The signals of the form, one per field so that typing in one never redraws another.
#[derive(Clone, Copy)]
struct Fields {
    plan: Signal<String>,
    review: Signal<String>,
    mood: Signal<String>,
    values: Signal<CustomValues>,
    layout: Signal<FormLayout>,
    defs: Signal<Vec<FieldDef>>,
}

impl Fields {
    fn new() -> Self {
        Self {
            plan: signal(String::new()),
            review: signal(String::new()),
            mood: signal(String::new()),
            values: signal(CustomValues::new()),
            layout: signal(FormLayout::default_for(FieldScope::Day)),
            defs: signal(Vec::new()),
        }
    }

    /// The edits as they are now (nothing subscribes).
    fn edits(&self) -> Edits {
        Edits {
            note: DayNote {
                plan: self.plan.get_untracked(),
                review: self.review.get_untracked(),
                mood: self.mood.get_untracked(),
            },
            values: self.values.get_untracked(),
        }
    }

    /// Shows what was read.
    fn adopt(&self, data: &DayData) {
        batch(|| {
            self.plan.set(data.note.plan.clone());
            self.review.set(data.note.review.clone());
            self.mood.set(data.note.mood.clone());
            self.values.set(data.values.clone());
        });
    }
}

/// The key the day's edits are kept under: the account (0 for all) and the day.
type Context = (i64, NaiveDate);

/// The form's state and how it is kept: what is typed, what was last read or saved, and the
/// day it belongs to. A `Copy` handle of signals.
#[derive(Clone, Copy)]
struct Session {
    fields: Fields,
    write: Writer,
    /// What was last read or saved, to know whether the form holds unsaved edits.
    saved: Signal<Edits>,
    /// The account and day the form shows.
    context: Signal<Option<Context>>,
    dirty: Memo<bool>,
    status: Signal<String>,
    busy: Signal<bool>,
}

impl Session {
    fn new(write: Writer) -> Self {
        let fields = Fields::new();
        let saved = signal(Edits::default());
        let dirty = computed(move || {
            Edits {
                note: DayNote { plan: fields.plan.get(), review: fields.review.get(), mood: fields.mood.get() },
                values: fields.values.get(),
            } != saved.get()
        });
        Self { fields, write, saved, context: signal(None), dirty, status: signal(String::new()), busy: signal(false) }
    }

    /// Writes `edits` as the note and the values of `at`.
    fn persist(self, at: Context, edits: Edits) {
        self.busy.set(true);
        let (account, date) = at;
        let written = edits.clone();
        self.write.run(
            move |db: &mut Db| {
                db.save_day_note(account, date, &edits.note)?;
                db.save_day_values(account, date, &edits.values)
            },
            move |result| {
                self.busy.set(false);
                match result {
                    Ok(()) => {
                        if self.context.get_untracked() == Some(at) {
                            self.saved.set(written);
                        }
                        self.status.set(format!("Đã lưu lúc {}", chrono::Local::now().format("%H:%M")));
                    }
                    Err(error) => self.status.set(error),
                }
            },
        );
    }

    /// Writes the edits when there are some and no write is under way.
    fn save_now(self) {
        let Some(at) = self.context.get_untracked() else { return };
        if !self.busy.get_untracked() && self.dirty.get_untracked() {
            self.persist(at, self.fields.edits());
        }
    }

    /// Shows what `data` reads. It replaces the form unless the form holds edits for the same
    /// day; edits for another day or account are written first. A memo, so that a read that
    /// is under way (and still holds the old data) never undoes an edit just made.
    fn follow(self, data: Reading<DayData>) {
        let latest = computed(move || data.value());
        effect(move || {
            let Some(read) = latest.get() else { return };
            untracked(|| {
                batch(|| {
                    self.fields.layout.set(read.layout.clone());
                    self.fields.defs.set(read.defs.clone());
                });
                let key = (read.account, read.day);
                let current = self.context.get_untracked();
                let holds_edits = self.dirty.get_untracked();
                if current == Some(key) && holds_edits {
                    return;
                }
                if let Some(old) = current.filter(|old| *old != key && holds_edits) {
                    self.persist(old, self.fields.edits());
                }
                batch(|| {
                    self.fields.adopt(&read);
                    self.saved.set(Edits::of(&read));
                    self.context.set(Some(key));
                });
            });
        });
    }
}

pub(crate) fn journal() -> impl IntoElement {
    let write = writer();
    let day = signal(account_trades::today());
    let data = query_with(move || day.get(), |db, account, day| load(db, account, *day));
    let session = Session::new(write);
    session.follow(data);
    set_interval(AUTOSAVE, move || session.save_now());
    let (fields, dirty, status, busy) = (session.fields, session.dirty, session.status, session.busy);
    let save_now = move || session.save_now();

    let go = move |to: NaiveDate| {
        if dirty.get_untracked() {
            save_now();
        }
        day.set(to);
    };
    let step = move |days: i64| {
        let from = day.get_untracked();
        let to = if days < 0 {
            from.checked_sub_days(Days::new(days.unsigned_abs()))
        } else {
            from.checked_add_days(Days::new(days.unsigned_abs()))
        };
        if let Some(to) = to {
            go(to);
        }
    };

    let actions = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(icon_action(Icon::ChevronLeft, "Ngày trước", Variant::Secondary, move || step(-1)))
        .child(
            text(move || day_title(day.get()))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .min_w(px(132.0))
                .text_color(move || theme().foreground),
        )
        .child(icon_action(Icon::ChevronRight, "Ngày sau", Variant::Secondary, move || step(1)))
        .child(action("Hôm nay", Variant::Secondary, move || go(account_trades::today())))
        .child(account_picker())
        .child(text(move || status.get()).text_xs().text_color(move || theme().muted_foreground))
        .child(action_when("Lưu nhật ký", Variant::Primary, move || dirty.get() && !busy.get(), save_now));

    page_with(
        "Nhật ký ngày",
        "Kế hoạch trước phiên, tổng kết cuối ngày và lệnh của ngày đặt cạnh nhau",
        actions,
        view()
            .gap_6()
            .child(dynamic(move || data.error().map(|error| text(error).text_sm().text_color(loss))))
            .child(week_strip(data, day, go))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_start()
                    .gap_6()
                    .child(view().flex_1().min_w(px(520.0)).gap_6().child(form_card(data, write, fields)))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(320.0))
                            .max_w(px(420.0))
                            .gap_6()
                            .child(trades_card(data))
                            .child(checklist_card(data)),
                    ),
            ),
    )
}

/// The week the chosen day is in, one card per day.
fn week_strip(
    data: Reading<DayData>,
    day: Signal<NaiveDate>,
    go: impl Fn(NaiveDate) + Copy + 'static,
) -> impl IntoElement {
    let weekdays = computed(move || week_of(day.get()));
    view().flex_row().gap_3().child(dynamic(move || {
        let dates = weekdays.get();
        untracked(|| {
            dates
                .into_iter()
                .map(|date| {
                    let figure = move || data.value().and_then(|d| d.week.iter().find(|w| w.day == date).cloned());
                    let net = move || {
                        figure().and_then(|w| w.net).map_or_else(
                            || "—".to_owned(),
                            |n| {
                                let currency = data.value().map_or_default(|d| d.currency.clone());
                                format::money(n, Sign::Always, &currency)
                            },
                        )
                    };
                    let body = view()
                        .items_center()
                        .gap_1()
                        .child(text(weekday_short(date)).text_xs().text_color(move || theme().muted_foreground))
                        .child(text(date.day().to_string()).text_xl().font_weight(FontWeight::SEMIBOLD))
                        .child(pnl_text(net).text_xs())
                        .child(
                            text(move || if figure().is_some_and(|w| w.written) { "✓ đã ghi" } else { "chưa ghi" })
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        );
                    view().flex_1().min_w(px(0.0)).child(selectable_card(
                        format!("Ngày {}", format_day(date)),
                        move || day.get() == date,
                        move || go(date),
                        body,
                    ))
                })
                .collect::<Vec<_>>()
        })
    }))
}

/// The form of the day: the built-in plan, review and mood, and the trader's own properties,
/// arranged the way the trader set the form up.
fn form_card(data: Reading<DayData>, write: Writer, fields: Fields) -> impl IntoElement {
    let on_layout = move |layout: FormLayout| {
        fields.layout.set(layout.clone());
        write.run(move |db: &mut Db| db.save_form_layout(&layout), |_| {});
    };
    let on_value = move |key: &str, value: Option<FieldValue>| {
        let key = key.to_owned();
        fields.values.update(|all| match value {
            Some(v) => {
                all.insert(key, v);
            }
            None => {
                all.remove(&key);
            }
        });
    };
    let on_def = move |def: FieldDef| {
        let mut def = def;
        // A property added a moment ago has no id until its first save has come back.
        if def.id == 0
            && let Some(known) = fields.defs.get_untracked().iter().find(|d| d.key == def.key)
        {
            def.id = known.id;
        }
        fields.defs.update(|all| match all.iter_mut().find(|d| d.key == def.key) {
            Some(slot) => *slot = def.clone(),
            None => all.push(def.clone()),
        });
        let key = def.key.clone();
        write.run(
            move |db: &mut Db| db.save_field_def(&def),
            move |result| {
                if let Ok(id) = result {
                    fields.defs.update(|all| {
                        if let Some(slot) = all.iter_mut().find(|d| d.key == key && d.id == 0) {
                            slot.id = id;
                        }
                    });
                }
            },
        );
    };
    let moods = move || data.value().map_or_default(|d| d.moods.clone());
    let list = PropertyList::new(
        move || fields.layout.get(),
        move || fields.defs.get(),
        move || fields.values.get(),
        PropertyEdits::new(on_layout, on_value, on_def),
    )
    .builtin(move |field| match field {
        BuiltinField::DayPlan => Some(
            textarea(
                "Kế hoạch ngày",
                "Xu hướng chính, vùng giá cần chú ý, tin trong ngày, cam kết hôm nay",
                6,
                move || fields.plan.get(),
                move |t| fields.plan.set(t.to_owned()),
            )
            .into_element(),
        ),
        BuiltinField::DayReview => Some(
            textarea(
                "Tổng kết cuối ngày",
                "Điều làm tốt, điều cần sửa, bài học cho ngày mai",
                6,
                move || fields.review.get(),
                move |t| fields.review.set(t.to_owned()),
            )
            .into_element(),
        ),
        BuiltinField::DayMood => Some(
            select_value(
                "Tâm trạng",
                move || {
                    let mut choices = vec![Choice::new("", "Chưa chọn")];
                    let known = moods();
                    let current = fields.mood.get_untracked();
                    // A mood whose option was removed since stays readable.
                    if !current.is_empty() && known.iter().all(|(key, _)| *key != current) {
                        choices.push(Choice::new(current.clone(), current));
                    }
                    choices.extend(known.into_iter().map(|(key, label)| Choice::new(key, label)));
                    choices
                },
                move || SharedString::from(fields.mood.get()),
                move |v| fields.mood.set(v.to_string()),
            )
            .into_element(),
        ),
        _ => None,
    });
    card("Kế hoạch và tổng kết", property_list(list))
}

/// The day's trades with their result and the rules they broke.
fn trades_card(data: Reading<DayData>) -> impl IntoElement {
    let currency = move || data.value().map_or_default(|d| d.currency.clone());
    let net = move || data.value().map_or(0.0, |d| d.closed_net());
    let count = move || data.value().map_or(0, |d| d.trades.len());
    card(
        "Lệnh trong ngày",
        view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
                    .child(
                        pnl_text(move || format::money(net(), Sign::Always, &currency()))
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        text(move || {
                            let wins =
                                data.value().map_or(0, |d| d.trades.iter().filter(|t| t.closed && t.net > 0.0).count());
                            format!("{} lệnh · {wins} thắng", count())
                        })
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(dynamic(move || {
                let Some(read) = data.value() else {
                    return text("Đang tải…").text_sm().text_color(move || theme().muted_foreground).into_element();
                };
                untracked(|| {
                    if read.trades.is_empty() {
                        return empty_state("Không có lệnh nào trong ngày", "Lệnh đóng hoặc mở trong ngày hiện ở đây.")
                            .into_element();
                    }
                    view()
                        .children(read.trades.iter().map(trade_row))
                        .children(
                            read.day_violations
                                .iter()
                                .map(|line| text(format!("Vi phạm: {line}")).pt_2().text_xs().text_color(loss)),
                        )
                        .into_element()
                })
            })),
    )
}

fn trade_row(trade: &DayTrade) -> impl IntoElement + use<> {
    let key = trade.key.clone();
    let net =
        if trade.closed { format::money(trade.net, Sign::Always, &trade.currency) } else { "đang mở".to_owned() };
    view()
        .py_2()
        .gap_1()
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    figure_text({
                        let time = trade.time.clone();
                        move || time.clone()
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(action(trade.symbol.clone(), Variant::Ghost, move || nav::show_trade(key.clone())))
                .child(chip(trade.side.label(), if trade.side == Side::Long { Tone::Profit } else { Tone::Loss }))
                .child(view().flex_1())
                .child(
                    figure_text({
                        let r = format::r_text(trade.r);
                        move || r.clone()
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(pnl_text(move || net.clone()).text_sm().font_weight(FontWeight::SEMIBOLD)),
        )
        .child(trade.violation.clone().map(|line| text(format!("Vi phạm: {line}")).text_xs().text_color(loss)))
}

/// The checklist before an entry, with how many of the day's trades confirmed each item.
fn checklist_card(data: Reading<DayData>) -> impl IntoElement {
    card(
        "Checklist trước lệnh",
        view().gap_2().child(dynamic(move || {
            let Some(read) = data.value() else {
                return text("Đang tải…").text_sm().text_color(move || theme().muted_foreground).into_element();
            };
            untracked(|| {
                if read.checklist.is_empty() {
                    return text("Chưa có mục nào: thêm ở Tài khoản & kỷ luật.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                let total = read.trades.len();
                view()
                    .gap_2()
                    .children(read.checklist.iter().map(|(item, confirmed)| {
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(text(item.text.clone()).text_sm().flex_1().min_w(px(0.0)))
                            .child(
                                figure_text({
                                    let line = if total == 0 {
                                        "—".to_owned()
                                    } else {
                                        format!("{confirmed} / {total} lệnh")
                                    };
                                    move || line.clone()
                                })
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                            )
                    }))
                    .into_element()
            })
        })),
    )
}

#[cfg(test)]
mod tests {
    use tradelens_core::{DealBatch, DealEntry, DealKind, Rules, model::AccountInfo, time::parse_mt5_time};

    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("fixture days parse")
    }

    #[test]
    fn the_week_runs_monday_to_sunday() {
        let week = week_of(date("2026-11-20"));
        assert_eq!(week.first().copied(), Some(date("2026-11-16")));
        assert_eq!(week.last().copied(), Some(date("2026-11-22")));
        assert_eq!(week.len(), 7);
        assert_eq!(weekday_short(date("2026-11-20")), "T6");
        assert_eq!(weekday_short(date("2026-11-22")), "CN");
        assert_eq!(day_title(date("2026-11-20")), "Thứ 6, 20/11/2026");
    }

    fn deal(
        ticket: i64,
        position: i64,
        at: &str,
        kind: DealKind,
        entry: DealEntry,
        profit: f64,
    ) -> tradelens_core::Deal {
        tradelens_core::Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "EURUSD".into(),
            kind,
            entry,
            volume: 0.1,
            price: 1.1,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            magic: 0,
            comment: String::new(),
            sl: 0.0,
            tp: 0.0,
            risk: None,
            reason: None,
        }
    }

    #[test]
    fn a_day_reads_its_trades_note_checklist_and_week() {
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                currency: "USD".into(),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, 1, "2026.11.20 09:00:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(2, 1, "2026.11.20 10:00:00", DealKind::Sell, DealEntry::Out, -60.0),
                deal(3, 2, "2026.11.20 11:00:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(4, 2, "2026.11.20 12:00:00", DealKind::Sell, DealEntry::Out, 25.0),
                deal(5, 3, "2026.11.19 11:00:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(6, 3, "2026.11.19 12:00:00", DealKind::Sell, DealEntry::Out, 10.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("stored");
        let account = db.accounts().expect("accounts").remove(0);
        db.save_rules(account.id, &Rules { max_daily_trades: Some(1), ..Rules::default() }).expect("rules");
        db.save_checklist(None, &[ChecklistItem { id: 0, text: "Có vùng cản rõ".into() }]).expect("checklist");
        let confirmed = db.checklist(None).expect("read").remove(0).id;
        let first = TradeKey { account_id: account.id, position_id: 1, segment: 0 };
        db.save_journal(&first, &tradelens_core::JournalEntry { checklist: vec![confirmed], ..Default::default() })
            .expect("journal");
        db.save_day_note(0, date("2026-11-20"), &DayNote { plan: "Chờ tin".into(), ..DayNote::default() })
            .expect("note");

        let read = load(&db, None, date("2026-11-20")).expect("loads");
        assert_eq!(read.account, 0);
        assert_eq!(read.note.plan, "Chờ tin");
        assert_eq!(read.trades.len(), 2);
        assert!((read.closed_net() + 35.0).abs() < 1e-9);
        assert_eq!(read.currency, "USD");
        assert_eq!(read.checklist.len(), 1);
        assert_eq!(read.checklist[0].1, 1, "one of the two trades confirmed the item");
        assert_eq!(read.day_violations.len(), 1, "two trades against a limit of one");
        assert_eq!(read.week.len(), 7);
        let friday = read.week.iter().find(|w| w.day == date("2026-11-20")).expect("the day is in its week");
        assert!(friday.written);
        assert!((friday.net.expect("a result") + 35.0).abs() < 1e-9);
        let thursday = read.week.iter().find(|w| w.day == date("2026-11-19")).expect("thursday");
        assert!(!thursday.written);
        assert!((thursday.net.expect("a result") - 10.0).abs() < 1e-9);
        assert!(!read.moods.is_empty(), "the mood options come from the emotion catalog");
    }

    #[test]
    fn another_day_has_no_trades() {
        let db = Db::in_memory().expect("a memory database opens");
        let read = load(&db, None, date("2026-11-21")).expect("loads");
        assert!(read.trades.is_empty() && read.checklist.is_empty());
        assert_eq!(read.note, DayNote::default());
    }
}
