//! Lệnh bỏ lỡ (missed trades): the setup was right but the trader did not enter, and the R missed.
//! Design: `docs/design/screens/missed-trades.html`: figures, the latest missed trades, the
//! totals by reason, and the form that records one (or edits the one picked in the list).

use std::rc::Rc;

use chrono::{Local, NaiveDateTime};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Side,
    attachments::Attachment,
    catalog::{CatalogItem, CatalogKind, CustomValues},
    missed::{self, MissedSummary, MissedTrade},
    time::{naive, to_ms},
};

use super::catalog_tags::catalog_tags;
use crate::i18n::plural;
use crate::{
    format,
    store::{Reading, query, store, writer},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Choice, Variant, action, action_icon, action_icon_when, confirm_then, root_owner, segmented, select_field,
            text_field_ref, textarea,
        },
        date_picker::{DatePick, date_pick, date_trigger},
        forms::{NumberRange, number_field},
        gallery::gallery,
        icons::Icon,
        list_item::{Column, mini_table},
        parts::{card, empty_state, error_line, page_with, stat_tile},
    },
};

/// The reason whose trades the discipline rules blocked: a miss that was right, not lost chances.
const BLOCKED_REASON: &str = "limit_reached";

/// How many of the latest missed trades the table lists.
const LISTED: usize = 25;

/// The `seen_at` format typed in the form.
const SEEN_FORMAT: &str = "%d/%m/%Y %H:%M";

/// The missed trade the form edits (`None`: a new one). The search sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Picked(Option<i64>);

/// Shows the missed trade `id` in the form the next time the page is on screen (or at once).
pub(super) fn select(id: i64) {
    global::<Picked>().set(Picked(Some(id)));
}

const SIDES: [Side; 2] = [Side::Long, Side::Short];

/// How long the form stays marked as new after "Ghi lệnh bỏ lỡ".
const FRESH_FOR: std::time::Duration = std::time::Duration::from_millis(1500);

pub(crate) fn missed() -> impl IntoElement {
    let list = query(|db, account| db.missed_trades(0, i64::MAX).map(|all| visible(all, account)));
    let reasons = query(|db, _| db.catalog(CatalogKind::MissedReason));
    // `Never`: pressing "Ghi lệnh bỏ lỡ" starts a clean form even when one was already blank.
    let editing = signal_with(None::<MissedTrade>, SignalEquals::Never);
    // The symbol field of the form shown, which "Ghi lệnh bỏ lỡ" gives the keyboard to.
    let symbol_field = element_ref();
    // On for a moment after "Ghi lệnh bỏ lỡ": the form shows it is a new one (a blank form built
    // again looks the same, and the trader read the press as doing nothing).
    let fresh = signal(false);
    let picked = global::<Picked>();
    effect(move || {
        let Some(id) = picked.get().0 else { return };
        let Some(all) = list.value() else { return };
        if let Some(found) = all.iter().find(|m| m.id == id) {
            editing.set(Some(found.clone()));
        }
        untracked(|| picked.set(Picked(None)));
    });
    page_with(
        t!("missed.title"),
        t!("missed.summary"),
        action_icon(Icon::Plus, t!("missed.record"), Variant::Primary, move || {
            editing.set(None);
            fresh.set(true);
            // The blank form is drawn in the next frame: its field takes the focus then, so the trader
            // can type at once; the form stays marked as new for a moment.
            spawn(async move {
                next_frame().await;
                symbol_field.focus();
                timer(FRESH_FOR).await;
                fresh.set(false);
            });
        }),
        view()
            .gap_6()
            .child(stats_row(list, reasons))
            .child(
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
                            .child(latest_card(list, reasons, editing))
                            .child(reasons_card(list, reasons)),
                    )
                    .child(view().w(px(400.0)).child(dynamic(move || {
                        let draft = editing.get();
                        untracked(|| form(draft, editing, reasons, symbol_field, fresh).into_element())
                    }))),
            )
            .child(dynamic(move || list.error().map(error_line))),
    )
}

// --- what the lists show ------------------------------------------------------------------

/// The missed trades of the chosen account (all of them for no account), newest first. A trade
/// with no account belongs to every view.
fn visible(all: Vec<MissedTrade>, account: Option<i64>) -> Vec<MissedTrade> {
    all.into_iter().filter(|m| account.is_none_or(|a| m.account_id.is_none_or(|own| own == a))).collect()
}

/// The missed trades split into the ones that cost a chance and the ones the rules blocked.
fn split_blocked(list: &[MissedTrade]) -> (Vec<MissedTrade>, usize) {
    let (blocked, open): (Vec<&MissedTrade>, Vec<&MissedTrade>) =
        list.iter().partition(|m| m.reason.as_deref() == Some(BLOCKED_REASON));
    (open.into_iter().cloned().collect(), blocked.len())
}

/// The label of a reason key; a key the catalog lost reads as itself, none as "Chưa ghi lý do".
fn reason_label(catalog: &[CatalogItem], key: &str) -> String {
    if key.is_empty() {
        return t!("missed.no_reason");
    }
    catalog.iter().find(|item| item.key == key).map_or_else(|| key.to_owned(), |item| item.label.clone())
}

/// A time of the form: `21/11/2026 09:42`.
fn format_seen(ms: i64) -> String {
    naive(ms).format(SEEN_FORMAT).to_string()
}

/// A time typed in the form, as milliseconds (the wall clock read as UTC).
fn parse_seen(typed: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(typed.trim(), SEEN_FORMAT).ok().map(to_ms)
}

/// `21/11 09:42` for the table.
fn short_seen(ms: i64) -> String {
    format::short_stamp(naive(ms))
}

fn stats_row(list: Reading<Vec<MissedTrade>>, reasons: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    let summary = move || {
        let all = list.value().map_or_else(Vec::new, |l| l.to_vec());
        let (open, blocked) = split_blocked(&all);
        (missed::summarise(&open), blocked, all.len())
    };
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(stat_tile(
            t!("missed.title"),
            move || (summary().2.to_string(), Tone::Neutral),
            || t!("missed.tile.all_time"),
        ))
        .child(stat_tile(
            t!("missed.tile.r_missed"),
            move || {
                let r = summary().0.r_missed;
                (format::r_text(Some(r)), if r > 0.0 { Tone::Profit } else { Tone::Neutral })
            },
            move || {
                let (s, ..) = summary();
                t!("missed.tile.avoided", r = format!("{:.1}", s.r_avoided))
            },
        ))
        .child(stat_tile(
            t!("missed.tile.correct"),
            move || (summary().1.to_string(), Tone::Neutral),
            || t!("missed.tile.blocked"),
        ))
        .child(stat_tile(
            t!("missed.tile.top_reason"),
            move || {
                (top_reason(&summary().0, reasons.value().as_deref().map_or(&[][..], Vec::as_slice)).0, Tone::Neutral)
            },
            move || {
                let (s, _, all) = summary();
                plural!(
                    all,
                    "missed.tile.times",
                    n = top_reason(&s, reasons.value().as_deref().map_or(&[][..], Vec::as_slice)).1,
                    all = all
                )
            },
        ))
}

/// The most frequent reason's label and count; a dash when nothing was recorded.
fn top_reason(summary: &MissedSummary, catalog: &[CatalogItem]) -> (String, usize) {
    summary.by_reason.first().map_or_else(|| ("—".to_owned(), 0), |(key, n, _)| (reason_label(catalog, key), *n))
}

fn latest_card(
    list: Reading<Vec<MissedTrade>>,
    reasons: Reading<Vec<CatalogItem>>,
    editing: Signal<Option<MissedTrade>>,
) -> impl IntoElement {
    let store = store();
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            text(move || {
                let total = list.value().map_or(0, |l| l.len());
                t!("missed.latest", shown = total.min(LISTED), total = total)
            })
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD),
        )
        .child(dynamic(move || {
            let all = list.value().map_or_else(Vec::new, |l| l.to_vec());
            let catalog = reasons.value().map_or_else(Vec::new, |c| c.to_vec());
            let snapshot = store.snapshot();
            untracked(|| {
                if all.is_empty() {
                    return empty_state(t!("missed.empty"), t!("missed.empty_hint")).into_element();
                }
                mini_table(
                    vec![
                        Column::start(t!("missed.column.time")),
                        Column::start("Symbol"),
                        Column::start(t!("missed.column.side")),
                        Column::start("Setup"),
                        Column::start(t!("missed.column.reason")),
                        Column::end(t!("missed.column.if_entered")),
                        Column::end(t!("missed.column.images")),
                    ],
                    all.iter()
                        .take(LISTED)
                        .map(|m| {
                            let setup = m
                                .playbook_id
                                .and_then(|id| snapshot.playbooks.iter().find(|p| p.id == id))
                                .map_or_else(|| t!("missed.no_playbook"), |p| p.name.clone());
                            let reason = m.reason.as_deref().map(|key| reason_label(&catalog, key));
                            let r = format::r_text(m.would_be_r());
                            let blocked = m.reason.as_deref() == Some(BLOCKED_REASON);
                            let row = m.clone();
                            vec![
                                action(short_seen(m.seen_at), Variant::Ghost, move || editing.set(Some(row.clone())))
                                    .into_element(),
                                text(m.symbol.clone()).text_sm().into_element(),
                                chip(m.side.label(), if m.side == Side::Long { Tone::Profit } else { Tone::Loss })
                                    .into_element(),
                                text(setup).text_sm().number_of_lines(1).into_element(),
                                reason.map_or_else(
                                    || text("—").text_sm().into_element(),
                                    |label| {
                                        chip(label, if blocked { Tone::Primary } else { Tone::Neutral }).into_element()
                                    },
                                ),
                                pnl_text(move || r.clone()).into_element(),
                                text(match m.screenshots.len() {
                                    0 => "—".to_owned(),
                                    1 => t!("missed.images_one"),
                                    n => t!("missed.images_many", n = n),
                                })
                                .text_sm()
                                .into_element(),
                            ]
                        })
                        .collect(),
                )
                .into_element()
            })
        }))
        .child(text(t!("missed.note")).text_xs().text_color(move || theme().muted_foreground))
}

/// "Theo lý do": how often each reason came up and the R it left on the table.
fn reasons_card(list: Reading<Vec<MissedTrade>>, reasons: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    card(
        t!("missed.by_reason"),
        dynamic(move || {
            let all = list.value().map_or_else(Vec::new, |l| l.to_vec());
            let catalog = reasons.value().map_or_else(Vec::new, |c| c.to_vec());
            untracked(|| {
                let summary = missed::summarise(&all);
                if summary.by_reason.is_empty() {
                    return text(t!("missed.no_data"))
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                mini_table(
                    vec![
                        Column::start(t!("missed.reason")),
                        Column::end(t!("missed.column.times")),
                        Column::end(t!("missed.tile.r_missed")),
                    ],
                    summary
                        .by_reason
                        .iter()
                        .map(|(key, count, r)| {
                            let r = format::r_text(Some(*r));
                            vec![
                                text(reason_label(&catalog, key)).text_sm().into_element(),
                                text(count.to_string()).text_sm().into_element(),
                                pnl_text(move || r.clone()).into_element(),
                            ]
                        })
                        .collect(),
                )
                .into_element()
            })
        }),
    )
}

// --- the form -------------------------------------------------------------------------------

/// A label over a control.
fn labelled(label: impl Into<SharedString>, control: impl IntoElement) -> impl IntoElement {
    view().gap_1().child(text(label.into()).text_xs().text_color(move || theme().muted_foreground)).child(control)
}

/// A missed trade with nothing filled in, seen now.
fn blank() -> MissedTrade {
    MissedTrade {
        id: 0,
        account_id: None,
        symbol: String::new(),
        side: Side::Long,
        seen_at: to_ms(Local::now().naive_local()),
        playbook_id: None,
        setup_grade: None,
        reason: None,
        entry: None,
        stop_loss: None,
        take_profit: None,
        outcome_price: None,
        notes: String::new(),
        screenshots: Vec::new(),
        custom: CustomValues::new(),
    }
}

/// What the form edits, one signal per field: typing in one redraws no other.
#[derive(Clone, Copy)]
struct Fields {
    symbol: Signal<String>,
    side: Signal<Side>,
    seen: Signal<String>,
    entry: Signal<Option<f64>>,
    stop: Signal<Option<f64>>,
    target: Signal<Option<f64>>,
    outcome: Signal<Option<f64>>,
    playbook: Signal<Option<i64>>,
    reason: Signal<Option<String>>,
    notes: Signal<String>,
    shots: Signal<Vec<Attachment>>,
}

impl Fields {
    fn of(base: &MissedTrade) -> Self {
        Self {
            symbol: signal(base.symbol.clone()),
            side: signal(base.side),
            seen: signal(format_seen(base.seen_at)),
            entry: signal(base.entry),
            stop: signal(base.stop_loss),
            target: signal(base.take_profit),
            outcome: signal(base.outcome_price),
            playbook: signal(base.playbook_id),
            reason: signal(base.reason.clone()),
            notes: signal(base.notes.clone()),
            shots: signal(base.screenshots.clone()),
        }
    }

    /// The fields laid over `base`, kept for `account` when the trade has none; why not (in
    /// the app's language) when the symbol is empty or the time is not one.
    fn collect(&self, base: &MissedTrade, account: Option<i64>) -> Result<MissedTrade, String> {
        let symbol = self.symbol.get_untracked();
        if symbol.trim().is_empty() {
            return Err(t!("missed.error.symbol"));
        }
        let Some(seen_at) = parse_seen(&self.seen.get_untracked()) else {
            return Err(t!("missed.error.time"));
        };
        Ok(MissedTrade {
            symbol: symbol.trim().to_uppercase(),
            side: self.side.get_untracked(),
            seen_at,
            playbook_id: self.playbook.get_untracked(),
            reason: self.reason.get_untracked(),
            entry: self.entry.get_untracked(),
            stop_loss: self.stop.get_untracked(),
            take_profit: self.target.get_untracked(),
            outcome_price: self.outcome.get_untracked(),
            notes: self.notes.get_untracked().trim().to_owned(),
            screenshots: self.shots.get_untracked(),
            account_id: base.account_id.or(account),
            ..base.clone()
        })
    }
}

/// The form of a missed trade: blank for `None`, else the one being edited. `symbol_field` is pointed
/// at its symbol input.
fn form(
    draft: Option<MissedTrade>,
    editing: Signal<Option<MissedTrade>>,
    reasons: Reading<Vec<CatalogItem>>,
    symbol_field: ElementRef,
    fresh: Signal<bool>,
) -> impl IntoElement {
    let store = store();
    let write = writer();
    let base = Rc::new(draft.unwrap_or_else(blank));
    let id = base.id;
    let fields = Fields::of(&base);
    let error = signal(None::<String>);
    let busy = signal(false);
    let save = move || {
        let trade = match fields.collect(&base, store.accounts.get_untracked().id()) {
            Ok(trade) => trade,
            Err(message) => return error.set(Some(message)),
        };
        error.set(None);
        busy.set(true);
        write.run(
            move |db| db.save_missed_trade(&trade),
            move |result| {
                busy.set(false);
                match result {
                    Ok(_) => editing.set(None),
                    Err(message) => error.set(Some(message)),
                }
            },
        );
    };
    // The question is the root's task: this form is built again when the data is read again.
    let asker = root_owner();
    let remove = move || {
        confirm_then(
            asker,
            t!("missed.delete.title"),
            t!("missed.delete.said"),
            t!("missed.delete.confirm"),
            move || {
                write.run(
                    move |db| db.delete_missed_trade(id),
                    move |result| match result {
                        Ok(()) => editing.set(None),
                        Err(message) => error.set(Some(message)),
                    },
                );
            },
        );
    };
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || if fresh.get() { theme().primary } else { theme().border })
        .bg(move || theme().card)
        .child(form_title(id, fresh))
        .child(trade_fields(fields, symbol_field))
        .child(reason_field(fields.reason, reasons))
        .child(labelled(
            t!("missed.notes"),
            textarea(
                t!("missed.notes"),
                t!("missed.notes_placeholder"),
                3,
                move || fields.notes.get(),
                move |typed| {
                    fields.notes.set(typed.to_owned());
                },
            ),
        ))
        .child(gallery(
            t!("missed.shots"),
            move || fields.shots.get(),
            move |list: Vec<Attachment>| fields.shots.set(list),
        ))
        .child(dynamic(move || error.get().map(error_line)))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    view().flex_row().when(id != 0, |v| v.child(action(t!("common.delete"), Variant::Ghost, remove))),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action(t!("common.cancel"), Variant::Ghost, move || editing.set(None)))
                        .child(action_icon_when(
                            Icon::Check,
                            t!("common.save"),
                            Variant::Primary,
                            move || !busy.get(),
                            save,
                        )),
                ),
        )
}

/// The form's title, and the mark that says it was just made blank.
fn form_title(id: i64, fresh: Signal<bool>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(if id == 0 { t!("missed.record") } else { t!("missed.edit") })
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(
            text(move || if fresh.get() { t!("missed.fresh") } else { String::new() })
                .text_xs()
                .text_color(move || theme().primary),
        )
}

/// Symbol, side, time, the prices and the playbook.
fn trade_fields(fields: Fields, symbol_field: ElementRef) -> impl IntoElement {
    let store = store();
    let Fields { symbol, side, seen, entry, stop, target, outcome, playbook, .. } = fields;
    let symbol_input = text_field_ref(
        "Symbol",
        "XAUUSD",
        move || symbol.get(),
        move |typed| symbol.set(typed.to_owned()),
        &symbol_field,
    );
    view()
        .gap_4()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_3()
                .child(view().flex_1().min_w(px(140.0)).child(labelled("Symbol", symbol_input)))
                .child(labelled(
                    t!("missed.column.side"),
                    segmented(
                        t!("missed.column.side"),
                        &SIDES,
                        Side::label,
                        move || side.get(),
                        move |next| side.set(next),
                    ),
                )),
        )
        .child(labelled(t!("missed.column.time"), {
            let chosen = move || parse_seen(&seen.get()).map(tradelens_core::time::naive);
            date_pick(
                DatePick::moment(
                    t!("missed.column.time"),
                    chosen,
                    || chrono::Local::now().date_naive(),
                    move |at| {
                        seen.set(at.format("%d/%m/%Y %H:%M").to_string());
                    },
                ),
                date_trigger(move || {
                    chosen().map_or_else(|| t!("missed.pick_time"), |at| at.format("%d/%m/%Y %H:%M").to_string())
                }),
            )
        }))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_3()
                .child(labelled("Entry", price_field("Entry", entry)))
                .child(labelled("SL", price_field("SL", stop)))
                .child(labelled("TP", price_field("TP", target)))
                .child(labelled(t!("missed.later_price"), price_field(t!("missed.later_price"), outcome))),
        )
        .child(labelled(
            "Playbook",
            select_field(
                "Playbook",
                move || {
                    let mut choices = vec![Choice::new("", t!("missed.no_playbook"))];
                    choices.extend(
                        store
                            .snapshot()
                            .playbooks
                            .iter()
                            .filter(|p| !p.archived)
                            .map(|p| Choice::new(p.id.to_string(), p.name.clone())),
                    );
                    choices
                },
                move || SharedString::from(playbook.get().map_or_else(String::new, |id| id.to_string())),
                move |value| playbook.set(value.parse().ok()),
            ),
        ))
}

/// The reason for not entering: one of the trader's reasons, or a new one typed here.
fn reason_field(reason: Signal<Option<String>>, reasons: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    labelled(
        t!("missed.column.reason"),
        catalog_tags(
            CatalogKind::MissedReason,
            t!("missed.reason"),
            t!("missed.reason_placeholder"),
            move || reasons.value().map_or_default(|all| all.to_vec()),
            move || reason.get().into_iter().collect(),
            move |key, on| reason.set(on.then(|| key.to_owned())),
        ),
    )
}

/// A price: a number field with no unit.
fn price_field(label: impl Into<SharedString>, value: Signal<Option<f64>>) -> impl IntoElement {
    number_field(label, None, NumberRange::any().stepping(0.01), move || value.get(), move |next| value.set(next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(reason: Option<&str>, account: Option<i64>, out: f64) -> MissedTrade {
        MissedTrade {
            id: 0,
            account_id: account,
            symbol: "XAUUSD".into(),
            side: Side::Long,
            seen_at: 0,
            playbook_id: None,
            setup_grade: None,
            reason: reason.map(str::to_owned),
            entry: Some(100.0),
            stop_loss: Some(99.0),
            take_profit: None,
            outcome_price: Some(out),
            notes: String::new(),
            screenshots: Vec::new(),
            custom: CustomValues::new(),
        }
    }

    #[test]
    fn the_form_time_round_trips() {
        let ms = parse_seen(" 21/11/2026 09:42 ").expect("a valid time");
        assert_eq!(format_seen(ms), "21/11/2026 09:42");
        assert_eq!(short_seen(ms), "21/11 09:42");
        assert_eq!(parse_seen("2026-11-21"), None);
        assert_eq!(parse_seen("31/02/2026 10:00"), None, "no such day");
    }

    #[test]
    fn trades_the_rules_blocked_do_not_count_as_lost_chances() {
        let list =
            [trade(Some("fear"), None, 103.0), trade(Some(BLOCKED_REASON), None, 105.0), trade(None, None, 98.0)];
        let (open, blocked) = split_blocked(&list);
        assert_eq!(blocked, 1);
        assert_eq!(open.len(), 2);
        let summary = missed::summarise(&open);
        assert!((summary.r_missed - 3.0).abs() < 1e-9, "{}", summary.r_missed);
        assert!((summary.r_avoided - 2.0).abs() < 1e-9);
    }

    #[test]
    fn an_account_sees_its_own_trades_and_the_ones_with_no_account() {
        let list = vec![trade(None, Some(1), 0.0), trade(None, Some(2), 0.0), trade(None, None, 0.0)];
        assert_eq!(visible(list.clone(), None).len(), 3);
        assert_eq!(visible(list, Some(2)).len(), 2);
    }

    #[test]
    fn reasons_read_by_label_and_the_most_frequent_comes_first() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, reasons_read_by_label);
        crate::i18n::with_text(crate::i18n::Lang::En, || assert_eq!(reason_label(&[], ""), "No reason recorded"));
    }

    fn reasons_read_by_label() {
        let catalog = [CatalogItem {
            id: 1,
            kind: CatalogKind::MissedReason,
            key: "fear".into(),
            label: "Sợ sau lệnh thua".into(),
            color: None,
            position: 0,
            archived: false,
        }];
        assert_eq!(reason_label(&catalog, "fear"), "Sợ sau lệnh thua");
        assert_eq!(reason_label(&catalog, "gone"), "gone");
        assert_eq!(reason_label(&catalog, ""), "Chưa ghi lý do");
        let summary = missed::summarise(&[trade(Some("fear"), None, 103.0), trade(Some("fear"), None, 101.0)]);
        assert_eq!(top_reason(&summary, &catalog), ("Sợ sau lệnh thua".to_owned(), 2));
        assert_eq!(top_reason(&MissedSummary::default(), &catalog), ("—".to_owned(), 0));
    }
}
