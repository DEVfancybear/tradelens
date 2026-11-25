//! Lệnh bỏ lỡ: setup đúng nhưng không vào, và R đã bỏ lỡ.
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

use crate::{
    format,
    store::{Reading, query, store, writer},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Choice, Variant, action, action_icon, action_icon_when, add_field, confirm_then, root_owner, segmented,
            select_field, tag_picker, text_field, textarea,
        },
        date_picker::{DatePick, date_pick, date_trigger},
        forms::{NumberRange, number_field},
        gallery::gallery,
        icons::Icon,
        list_item::{Column, mini_table},
        parts::{card, empty_state, page_with, stat_tile},
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

pub(crate) fn missed() -> impl IntoElement {
    let list = query(|db, account| db.missed_trades(0, i64::MAX).map(|all| visible(all, account)));
    let reasons = query(|db, _| db.catalog(CatalogKind::MissedReason));
    // `Never`: pressing "Ghi lệnh bỏ lỡ" starts a clean form even when one was already blank.
    let editing = signal_with(None::<MissedTrade>, SignalEquals::Never);
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
        "Lệnh bỏ lỡ",
        "Setup đúng nhưng không vào — ghi lại để biết do dự tốn bao nhiêu.",
        action_icon(Icon::Plus, "Ghi lệnh bỏ lỡ", Variant::Primary, move || editing.set(None)),
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
                        untracked(|| form(draft, editing, reasons).into_element())
                    }))),
            )
            .child(dynamic(move || list.error().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss())))),
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
        return "Chưa ghi lý do".to_owned();
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
            "Lệnh bỏ lỡ",
            move || (summary().2.to_string(), Tone::Neutral),
            || "tất cả thời gian".to_owned(),
        ))
        .child(stat_tile(
            "R bỏ lỡ",
            move || {
                let r = summary().0.r_missed;
                (format::r_text(Some(r)), if r > 0.0 { Tone::Profit } else { Tone::Neutral })
            },
            move || {
                let (s, ..) = summary();
                format!("tránh được {:.1}R nhờ không vào lệnh thua", s.r_avoided)
            },
        ))
        .child(stat_tile(
            "Bỏ lỡ đúng",
            move || (summary().1.to_string(), Tone::Neutral),
            || "bị chặn bởi quy tắc".to_owned(),
        ))
        .child(stat_tile(
            "Lý do nhiều nhất",
            move || {
                (top_reason(&summary().0, reasons.value().as_deref().map_or(&[][..], Vec::as_slice)).0, Tone::Neutral)
            },
            move || {
                let (s, _, all) = summary();
                format!("{} / {all} lần", top_reason(&s, reasons.value().as_deref().map_or(&[][..], Vec::as_slice)).1)
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
        .child(text(move || {
            let total = list.value().map_or(0, |l| l.len());
            format!("Lệnh bỏ lỡ · {} gần nhất / {total}", total.min(LISTED))
        }).text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(dynamic(move || {
            let all = list.value().map_or_else(Vec::new, |l| l.to_vec());
            let catalog = reasons.value().map_or_else(Vec::new, |c| c.to_vec());
            let snapshot = store.snapshot();
            untracked(|| {
                if all.is_empty() {
                    return empty_state("Chưa có lệnh bỏ lỡ nào", "Ghi lại setup đúng mà bạn không vào ở form bên cạnh.")
                        .into_element();
                }
                mini_table(
                    vec![
                        Column::start("Thời điểm"),
                        Column::start("Symbol"),
                        Column::start("Chiều"),
                        Column::start("Setup"),
                        Column::start("Lý do không vào"),
                        Column::end("Nếu đã vào"),
                        Column::end("Ảnh"),
                    ],
                    all.iter()
                        .take(LISTED)
                        .map(|m| {
                            let setup = m
                                .playbook_id
                                .and_then(|id| snapshot.playbooks.iter().find(|p| p.id == id))
                                .map_or_else(|| "Không playbook".to_owned(), |p| p.name.clone());
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
                                reason
                                    .map_or_else(
                                        || text("—").text_sm().into_element(),
                                        |label| chip(label, if blocked { Tone::Primary } else { Tone::Neutral }).into_element(),
                                    ),
                                pnl_text(move || r.clone()).into_element(),
                                text(match m.screenshots.len() {
                                    0 => "—".to_owned(),
                                    n => format!("{n} ảnh"),
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
        .child(
            text("“Nếu đã vào” tính theo entry, SL và giá sau đó bạn ghi. Lệnh bị chặn bởi giới hạn lệnh/ngày là bỏ lỡ đúng, không tính là mất cơ hội.")
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

/// "Theo lý do": how often each reason came up and the R it left on the table.
fn reasons_card(list: Reading<Vec<MissedTrade>>, reasons: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    card(
        "Theo lý do",
        dynamic(move || {
            let all = list.value().map_or_else(Vec::new, |l| l.to_vec());
            let catalog = reasons.value().map_or_else(Vec::new, |c| c.to_vec());
            untracked(|| {
                let summary = missed::summarise(&all);
                if summary.by_reason.is_empty() {
                    return text("Chưa có dữ liệu.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                mini_table(
                    vec![Column::start("Lý do"), Column::end("Lần"), Column::end("R bỏ lỡ")],
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
fn labelled(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view().gap_1().child(text(label).text_xs().text_color(move || theme().muted_foreground)).child(control)
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
    /// Vietnamese) when the symbol is empty or the time is not one.
    fn collect(&self, base: &MissedTrade, account: Option<i64>) -> Result<MissedTrade, String> {
        let symbol = self.symbol.get_untracked();
        if symbol.trim().is_empty() {
            return Err("Symbol không được để trống.".to_owned());
        }
        let Some(seen_at) = parse_seen(&self.seen.get_untracked()) else {
            return Err("Thời điểm phải có dạng ngày/tháng/năm giờ:phút, ví dụ 21/11/2026 09:42.".to_owned());
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

/// The form of a missed trade: blank for `None`, else the one being edited.
fn form(
    draft: Option<MissedTrade>,
    editing: Signal<Option<MissedTrade>>,
    reasons: Reading<Vec<CatalogItem>>,
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
        confirm_then(asker, "Xoá lệnh bỏ lỡ?", "Không hoàn tác được.", "Xoá hẳn", move || {
            write.run(
                move |db| db.delete_missed_trade(id),
                move |result| match result {
                    Ok(()) => editing.set(None),
                    Err(message) => error.set(Some(message)),
                },
            );
        });
    };
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            text(if id == 0 { "Ghi lệnh bỏ lỡ" } else { "Sửa lệnh bỏ lỡ" }).text_sm().font_weight(FontWeight::SEMIBOLD),
        )
        .child(trade_fields(fields))
        .child(reason_field(fields.reason, reasons, error))
        .child(labelled(
            "Ghi chú",
            textarea(
                "Ghi chú",
                "Điều gì khiến bạn không vào?",
                3,
                move || fields.notes.get(),
                move |typed| {
                    fields.notes.set(typed.to_owned());
                },
            ),
        ))
        .child(gallery("Ảnh chart", move || fields.shots.get(), move |list: Vec<Attachment>| fields.shots.set(list)))
        .child(dynamic(move || error.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss()))))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(view().flex_row().when(id != 0, |v| v.child(action("Xoá", Variant::Ghost, remove))))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action("Huỷ", Variant::Ghost, move || editing.set(None)))
                        .child(action_icon_when(Icon::Check, "Lưu", Variant::Primary, move || !busy.get(), save)),
                ),
        )
}

/// Symbol, side, time, the prices and the playbook.
fn trade_fields(fields: Fields) -> impl IntoElement {
    let store = store();
    let Fields { symbol, side, seen, entry, stop, target, outcome, playbook, .. } = fields;
    view()
        .gap_4()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_3()
                .child(view().flex_1().min_w(px(140.0)).child(labelled(
                    "Symbol",
                    text_field(
                        "Symbol",
                        None,
                        "XAUUSD",
                        move || symbol.get(),
                        move |typed| symbol.set(typed.to_owned()),
                    ),
                )))
                .child(labelled(
                    "Chiều",
                    segmented("Chiều", &SIDES, Side::label, move || side.get(), move |next| side.set(next)),
                )),
        )
        .child(labelled("Thời điểm", {
            let chosen = move || parse_seen(&seen.get()).map(tradelens_core::time::naive);
            date_pick(
                DatePick::moment(
                    "Thời điểm",
                    chosen,
                    || chrono::Local::now().date_naive(),
                    move |at| {
                        seen.set(at.format("%d/%m/%Y %H:%M").to_string());
                    },
                ),
                date_trigger(move || {
                    chosen().map_or_else(|| "Chọn ngày giờ".to_owned(), |at| at.format("%d/%m/%Y %H:%M").to_string())
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
                .child(labelled("Giá sau đó", price_field("Giá sau đó", outcome))),
        )
        .child(labelled(
            "Playbook",
            select_field(
                "Playbook",
                move || {
                    let mut choices = vec![Choice::new("", "Không playbook")];
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

/// The options of a catalog that are in use, as `(key, label)`. A memo: it changes only when the
/// options do, so a picker over it is not made again every time the catalog is read.
pub(super) fn catalog_options(items: Reading<Vec<CatalogItem>>) -> Memo<Vec<(String, String)>> {
    computed(move || {
        items.value().map_or_default(|all| {
            all.iter().filter(|item| !item.archived).map(|item| (item.key.clone(), item.label.clone())).collect()
        })
    })
}

/// The reason for not entering: one of the trader's reasons, or a new one typed here.
fn reason_field(
    reason: Signal<Option<String>>,
    reasons: Reading<Vec<CatalogItem>>,
    error: Signal<Option<String>>,
) -> impl IntoElement {
    let write = writer();
    let add_reason = move |label: String| {
        let label = label.trim().to_owned();
        if label.is_empty() {
            return;
        }
        let key = format!("custom_{}", chrono::Utc::now().timestamp_millis());
        let chosen = key.clone();
        write.run(
            move |db| {
                db.save_catalog_item(&CatalogItem {
                    id: 0,
                    kind: CatalogKind::MissedReason,
                    key,
                    label,
                    color: None,
                    position: i32::MAX,
                    archived: false,
                })
            },
            move |result| match result {
                Ok(_) => reason.set(Some(chosen)),
                Err(message) => error.set(Some(message)),
            },
        );
    };
    let options = catalog_options(reasons);
    labelled(
        "Lý do không vào",
        view()
            .gap_2()
            .child(tag_picker(
                "Lý do",
                move || options.get(),
                move || reason.get().into_iter().collect(),
                move |key, on| reason.set(on.then(|| key.to_owned())),
            ))
            .child(add_field("Thêm lý do", "+ Thêm lý do mới, rồi Enter", add_reason)),
    )
}

/// A price: a number field with no unit.
fn price_field(label: &'static str, value: Signal<Option<f64>>) -> impl IntoElement {
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
