//! Popup ghi kế hoạch: when the EA reports a position that just opened, a dialog over any
//! screen asks for its plan (the reason, entry, stop, target, risk, chart, setup) while the
//! result is still unknown. "Ghi sau" keeps the trade in the review queue instead; several
//! opens wait their turn, one dialog at a time. The same dialog edits the plan of a trade
//! already in the journal.
//! Design: `docs/design/screens/entry-plan-prompt.html`.

use nsh_base::{dialog, theme};
use nsh_core::prelude::*;
use tradelens_bridge::OpenedPosition;
use tradelens_core::{
    ChecklistItem, Db, DbError, Side, TradeKey,
    catalog::{CatalogItem, CatalogKind},
    journal::EntryPlan,
};

use crate::{
    format,
    journal_draft::{key_of_choice, key_or_empty, ticked, unticked},
    snapshot::TradeRow,
    store::{Reading, Writer, query, store, writer},
    ui::{
        chips::{Tone, chip},
        controls::{Choice, Variant, action, action_icon, icon_action, select_value, textarea},
        forms::{NumberRange, checklist, number_field},
        icons::Icon,
        theme as tokens,
    },
};

/// Width of the dialog's panel, in points.
const PANEL_WIDTH: f32 = 560.0;
/// How far the scrim darkens the screen behind the dialog.
const SCRIM_OPACITY: f32 = 0.72;

/// Which trade a plan is for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    /// A trade the journal already knows.
    Trade(TradeKey),
    /// A position the EA just reported, found by the account's login once it is saved.
    Reported { login: String, position_id: i64 },
}

/// A plan to be written: the trade it is for and what the dialog starts with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanRequest {
    pub(crate) target: Target,
    pub(crate) symbol: String,
    pub(crate) side: Side,
    pub(crate) volume: f64,
    /// The first line under the title: where it came from and how it stands.
    pub(crate) origin: String,
    /// The plan the dialog starts from: the MT5 levels, or the plan already written.
    pub(crate) seed: EntryPlan,
    /// The playbook setup already linked, when editing.
    pub(crate) playbook_id: Option<i64>,
}

impl PlanRequest {
    /// The prompt for a position the EA just reported: entry, stop and target come from MT5.
    pub(crate) fn of_position(position: &OpenedPosition) -> Self {
        let level = |v: f64| (v > 0.0).then_some(v);
        Self {
            target: Target::Reported { login: position.account.clone(), position_id: position.position_id },
            symbol: position.symbol.clone(),
            side: position.side,
            volume: position.volume,
            origin: format!("EA MT5 · lệnh vừa mở · tài khoản {}", position.account),
            seed: EntryPlan {
                entry: level(position.price),
                stop_loss: level(position.sl),
                take_profit: level(position.tp),
                ..EntryPlan::default()
            },
            playbook_id: None,
        }
    }

    /// The form for a trade already listed, starting from `plan` when one was written and from
    /// the trade's own levels when not.
    pub(crate) fn of_row(row: &TradeRow, plan: Option<EntryPlan>, playbook_id: Option<i64>) -> Self {
        let level = |v: f64| (v > 0.0).then_some(v);
        let trade = &row.trade;
        Self {
            target: Target::Trade(trade.key.clone()),
            symbol: trade.symbol.clone(),
            side: trade.side,
            volume: trade.volume,
            origin: format!("{} · lệnh {}", row.account, trade.key.position_id),
            seed: plan.unwrap_or_else(|| EntryPlan {
                entry: level(trade.open_price),
                stop_loss: level(trade.sl),
                take_profit: level(trade.tp),
                ..EntryPlan::default()
            }),
            playbook_id,
        }
    }
}

/// The plans waiting for the trader, the one on screen first.
#[derive(Debug, Clone, PartialEq, Default)]
struct Queue(Vec<PlanRequest>);

/// Puts a plan in line: it shows at once when none is on screen. A plan for a trade that is
/// already in line is not added twice (the EA may send a deal again).
pub(crate) fn enqueue(request: PlanRequest) {
    global::<Queue>().update(|queue| {
        if !queue.0.iter().any(|waiting| waiting.target == request.target) {
            queue.0.push(request);
        }
    });
}

/// Takes the plan for `target` out of line, so that a second press of the same button cannot
/// take the next plan with it.
fn finish(target: &Target) {
    global::<Queue>().update(|queue| queue.0.retain(|waiting| waiting.target != *target));
}

/// The dialog, mounted once over the whole window.
pub(crate) fn host() -> impl IntoElement {
    let (queue, write) = (global::<Queue>(), writer());
    let head = computed(move || queue.with(|q| q.0.first().cloned()));
    dynamic(move || {
        head.get().map(|request| {
            dialog()
                .on_cancel(move || {
                    if let Some(target) = head.with_untracked(|r| r.as_ref().map(|r| r.target.clone())) {
                        later(write, &target);
                    }
                })
                .flex_1()
                .child(untracked(|| scrim(&request)))
        })
    })
}

/// What "Ghi sau" and Escape do: flag the trade for review, then show the next plan. The
/// dialog stays until the flag is written: the write runs in the dialog's own scope, which
/// closing it would cancel.
fn later(write: Writer, target: &Target) {
    let target = target.clone();
    let flagged = target.clone();
    write.run(
        move |db| {
            let key = resolve(db, &flagged)?;
            let mut entry = db.journal(&key)?.unwrap_or_default();
            entry.review_later = true;
            db.save_journal(&key, &entry)
        },
        move |result| {
            if let Err(error) = result {
                log::warn!("entry plan: {error}");
            }
            finish(&target);
        },
    );
}

/// The journal key of a target; an error in Vietnamese when the account is not known.
fn resolve(db: &Db, target: &Target) -> Result<TradeKey, DbError> {
    match target {
        Target::Trade(key) => Ok(key.clone()),
        Target::Reported { login, position_id } => {
            let account = db
                .accounts()?
                .into_iter()
                .find(|a| a.login == *login)
                .ok_or_else(|| DbError::Invalid(format!("chưa có tài khoản {login} trong dữ liệu")))?;
            Ok(TradeKey { account_id: account.id, position_id: *position_id, segment: 0 })
        }
    }
}

/// Planned reward over planned risk as the dialog shows it: `1 : 3.2`.
pub(crate) fn reward_risk_text(entry: Option<f64>, stop_loss: Option<f64>, take_profit: Option<f64>) -> String {
    let plan = EntryPlan { entry, stop_loss, take_profit, ..EntryPlan::default() };
    plan.reward_risk().map_or_else(|| "—".to_owned(), |rr| format!("1 : {rr:.1}"))
}

/// The time of writing on the broker server's clock, which is what trades are timed in, so
/// that "written before the close" compares like with like.
pub(crate) fn server_time(now_utc_ms: i64, server_offset_min: Option<i32>) -> i64 {
    now_utc_ms + server_offset_min.map_or(0, |m| i64::from(m) * 60_000)
}

/// The plan as the fields of the dialog hold it.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    thesis: String,
    entry: Option<f64>,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
    risk: Option<f64>,
    timeframe: Option<String>,
    playbook_id: Option<i64>,
    grade: Option<String>,
    emotion: Option<String>,
    checklist: Vec<i64>,
}

/// Writes the plan, the setup and the checklist over the entry stored now.
fn save_plan(db: &mut Db, target: &Target, draft: &Draft, now_utc_ms: i64) -> Result<(), DbError> {
    let key = resolve(db, target)?;
    let offset = db.accounts()?.into_iter().find(|a| a.id == key.account_id).and_then(|a| a.server_offset_min);
    let mut entry = db.journal(&key)?.unwrap_or_default();
    entry.plan = Some(EntryPlan {
        thesis: draft.thesis.trim().to_owned(),
        entry: draft.entry,
        stop_loss: draft.stop_loss,
        take_profit: draft.take_profit,
        risk: draft.risk,
        timeframe: draft.timeframe.clone().unwrap_or_default(),
        written_at: server_time(now_utc_ms, offset),
    });
    if draft.playbook_id.is_some() {
        entry.playbook_id = draft.playbook_id;
    }
    if draft.grade.is_some() {
        entry.setup_grade.clone_from(&draft.grade);
    }
    if let Some(emotion) = &draft.emotion {
        entry.emotion_before.clone_from(emotion);
    }
    if !draft.checklist.is_empty() {
        entry.checklist.clone_from(&draft.checklist);
    }
    if entry.planned_risk.is_none() {
        entry.planned_risk = draft.risk;
    }
    // The plan is written: nothing is waiting for it in the review queue any more.
    entry.review_later = false;
    db.save_journal(&key, &entry)
}

/// What the dialog's drop-downs and checklist read from the database.
#[derive(Debug, Clone, Default)]
struct Choices {
    timeframes: Vec<CatalogItem>,
    grades: Vec<CatalogItem>,
    emotions: Vec<CatalogItem>,
    checklist: Vec<ChecklistItem>,
}

fn load_choices(db: &Db, account: Option<i64>) -> Result<Choices, DbError> {
    Ok(Choices {
        timeframes: db.catalog(CatalogKind::Timeframe)?,
        grades: db.catalog(CatalogKind::SetupGrade)?,
        emotions: db.catalog(CatalogKind::Emotion)?,
        checklist: db.checklist(account)?,
    })
}

/// What a drop-down of the dialog stores for an option: its key, or its label (the plan keeps
/// the timeframe as the trader reads it, `M15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stores {
    Key,
    Label,
}

fn choices_of(items: &[CatalogItem], stores: Stores) -> Vec<Choice> {
    let mut choices = vec![Choice::new("", "Chưa chọn")];
    choices.extend(items.iter().filter(|i| !i.archived).map(|i| {
        let value = match stores {
            Stores::Key => i.key.clone(),
            Stores::Label => i.label.clone(),
        };
        Choice::new(value, i.label.clone())
    }));
    choices
}

/// The darkened screen with the panel in its middle.
fn scrim(request: &PlanRequest) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .p_6()
        .bg(move || theme().background.opacity(SCRIM_OPACITY))
        .child(panel(request))
}

/// The signals of the plan being written, one per field.
#[derive(Clone, Copy)]
struct Fields {
    thesis: Signal<String>,
    entry: Signal<Option<f64>>,
    stop: Signal<Option<f64>>,
    take: Signal<Option<f64>>,
    risk: Signal<Option<f64>>,
    timeframe: Signal<Option<String>>,
    playbook: Signal<Option<i64>>,
    grade: Signal<Option<String>>,
    emotion: Signal<Option<String>>,
    checked: Signal<Vec<i64>>,
}

impl Fields {
    fn of(request: &PlanRequest) -> Self {
        let seed = &request.seed;
        Self {
            thesis: signal(seed.thesis.clone()),
            entry: signal(seed.entry),
            stop: signal(seed.stop_loss),
            take: signal(seed.take_profit),
            risk: signal(seed.risk),
            timeframe: signal((!seed.timeframe.is_empty()).then(|| seed.timeframe.clone())),
            playbook: signal(request.playbook_id),
            grade: signal(None),
            emotion: signal(None),
            checked: signal(Vec::new()),
        }
    }

    fn draft(&self) -> Draft {
        Draft {
            thesis: self.thesis.get_untracked(),
            entry: self.entry.get_untracked(),
            stop_loss: self.stop.get_untracked(),
            take_profit: self.take.get_untracked(),
            risk: self.risk.get_untracked(),
            timeframe: self.timeframe.get_untracked(),
            playbook_id: self.playbook.get_untracked(),
            grade: self.grade.get_untracked(),
            emotion: self.emotion.get_untracked(),
            checklist: self.checked.get_untracked(),
        }
    }
}

/// A drop-down over an optional key of the catalogs the dialog read.
fn catalog_pick(
    label: &'static str,
    choices: Reading<Choices>,
    pick: fn(&Choices) -> &Vec<CatalogItem>,
    stores: Stores,
    value: Signal<Option<String>>,
) -> impl IntoElement {
    half(
        label,
        select_value(
            label,
            move || choices_of(pick(&choices.value().unwrap_or_default()), stores),
            move || SharedString::from(key_or_empty(value.get().as_deref())),
            move |chosen| value.set(key_of_choice(&chosen)),
        ),
    )
}

/// The setup, timeframe, grade, feeling and checklist the trader may add to the plan.
fn setup_fields(fields: Fields, choices: Reading<Choices>) -> impl IntoElement {
    let store = store();
    let playbooks = computed(move || {
        let mut all = vec![Choice::new("", "Chưa gắn")];
        all.extend(
            store
                .snapshot()
                .playbooks
                .iter()
                .filter(|p| !p.archived)
                .map(|p| Choice::new(p.id.to_string(), p.name.clone())),
        );
        all
    });
    let (playbook, checked) = (fields.playbook, fields.checked);
    view()
        .gap_4()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .child(half(
                    "Playbook",
                    select_value(
                        "Playbook",
                        move || playbooks.get(),
                        move || SharedString::from(playbook.get().map_or_default(|id| id.to_string())),
                        move |v| playbook.set(v.parse().ok()),
                    ),
                ))
                .child(catalog_pick("Khung vào lệnh", choices, |c| &c.timeframes, Stores::Label, fields.timeframe)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .child(catalog_pick("Hạng setup", choices, |c| &c.grades, Stores::Key, fields.grade))
                .child(catalog_pick("Cảm xúc trước lệnh", choices, |c| &c.emotions, Stores::Key, fields.emotion)),
        )
        .child(field(
            "Checklist trước lệnh",
            checklist(
                "Checklist trước lệnh",
                move || choices.value().unwrap_or_default().checklist.clone(),
                move || checked.get(),
                move |id, on| checked.update(|c| *c = if on { ticked(c, id) } else { unticked(c, id) }),
            ),
        ))
}

fn panel(request: &PlanRequest) -> impl IntoElement {
    let write = writer();
    let choices = query(load_choices);
    let target = request.target.clone();
    let fields = Fields::of(request);
    let busy = signal(false);
    let failure = signal(None::<String>);
    let waiting = computed(move || global::<Queue>().with(|q| q.0.len().saturating_sub(1)));
    let save = {
        let target = target.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let (target, draft) = (target.clone(), fields.draft());
            busy.set(true);
            failure.set(None);
            let saved = target.clone();
            write.run(
                move |db| save_plan(db, &target, &draft, chrono::Utc::now().timestamp_millis()),
                move |result| {
                    busy.set(false);
                    match result {
                        Ok(()) => finish(&saved),
                        Err(error) => failure.set(Some(error)),
                    }
                },
            );
        }
    };
    let skip = move || later(write, &target);
    let thesis = fields.thesis;
    view()
        .w(px(PANEL_WIDTH))
        .max_h(px(720.0))
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .shadow_lg()
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_4()
                    .child(header(request, waiting, skip.clone()))
                    .child(field(
                        "Lý do vào lệnh (viết ngay, trước khi biết kết quả)",
                        textarea(
                            "Lý do vào lệnh",
                            "Setup thấy gì, chờ xác nhận gì…",
                            4,
                            move || thesis.get(),
                            move |t| thesis.set(t.to_owned()),
                        ),
                    ))
                    .child(levels(fields.entry, fields.stop, fields.take, fields.risk))
                    .child(setup_fields(fields, choices))
                    .child(dynamic(move || failure.get().map(|error| text(error).text_sm().text_color(tokens::loss()))))
                    .child(footer(busy, skip, save)),
            ),
        )
}

/// The title block: where the position came from, what it is, and how many wait behind it.
fn header(request: &PlanRequest, waiting: Memo<usize>, skip: impl Fn() + 'static) -> impl IntoElement {
    let (side, tone) = (request.side, if request.side == Side::Long { Tone::Profit } else { Tone::Loss });
    let level = |label: &str, value: Option<f64>| value.map(|v| format!("{label} {}", format::price(v)));
    let seed = &request.seed;
    let facts: Vec<String> = [
        level("SL", seed.stop_loss),
        level("TP", seed.take_profit),
        (seed.reward_risk().is_some())
            .then(|| format!("R:R {}", reward_risk_text(seed.entry, seed.stop_loss, seed.take_profit))),
    ]
    .into_iter()
    .flatten()
    .collect();
    let entry = seed.entry.map_or_else(String::new, |p| format!(" @ {}", format::price(p)));
    view()
        .flex_row()
        .items_start()
        .justify_between()
        .gap_3()
        .child(
            view()
                .gap_1()
                .min_w(px(0.0))
                .child(text(request.origin.clone()).text_xs().text_color(move || theme().primary))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(text(request.symbol.clone()).text_xl().font_weight(FontWeight::SEMIBOLD))
                        .child(chip(side.label(), tone))
                        .child(
                            text(format!("{} lot{entry}", format::lots(request.volume)))
                                .font_family(tokens::mono)
                                .tabular_nums()
                                .text_sm(),
                        ),
                )
                .child(
                    text(facts.join(" · "))
                        .font_family(tokens::mono)
                        .tabular_nums()
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(dynamic(move || {
                    let more = waiting.get();
                    (more > 0).then(|| chip(format!("còn {more} lệnh đang chờ ghi kế hoạch"), Tone::Neutral))
                })),
        )
        .child(icon_action(Icon::Close, "Đóng và ghi kế hoạch sau", Variant::Ghost, skip))
}

/// A caption over a control.
fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view().gap_2().child(text(label).text_sm().text_color(move || theme().muted_foreground)).child(control)
}

/// A caption over a control that shares a row with another.
fn half(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view().flex_1().min_w(px(200.0)).child(field(label, control))
}

/// Entry, stop, target and risk, with the reward to risk they make.
fn levels(
    entry: Signal<Option<f64>>,
    stop: Signal<Option<f64>>,
    take: Signal<Option<f64>>,
    risk: Signal<Option<f64>>,
) -> impl IntoElement {
    let price = |label: &'static str, value: Signal<Option<f64>>| {
        field(
            label,
            number_field(label, None, NumberRange::any().stepping(0.01), move || value.get(), move |n| value.set(n)),
        )
    };
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .child(price("Giá vào", entry))
                .child(price("Stop loss", stop))
                .child(price("Take profit", take))
                .child(price("Risk", risk)),
        )
        .child(
            text(move || format!("R:R dự kiến {}", reward_risk_text(entry.get(), stop.get(), take.get())))
                .font_family(tokens::mono)
                .tabular_nums()
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

fn footer(busy: Signal<bool>, skip: impl Fn() + 'static, save: impl Fn() + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text("Entry, SL, TP lấy từ MT5 · ảnh chart lúc vào do EA tự chụp")
                .flex_1()
                .min_w(px(200.0))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(view().flex_row().items_center().gap_2().child(action("Ghi sau", Variant::Ghost, skip)).child(
            action_icon(Icon::Check, "Lưu kế hoạch", Variant::Primary, move || {
                if !busy.get_untracked() {
                    save();
                }
            }),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position() -> OpenedPosition {
        OpenedPosition {
            account: "52012345".into(),
            position_id: 77,
            symbol: "XAUUSD".into(),
            side: Side::Long,
            volume: 0.5,
            price: 2648.2,
            sl: 2644.3,
            tp: 0.0,
        }
    }

    #[test]
    fn the_prompt_starts_from_the_levels_mt5_reported() {
        let request = PlanRequest::of_position(&position());
        assert_eq!(request.seed.entry, Some(2648.2));
        assert_eq!(request.seed.stop_loss, Some(2644.3));
        assert_eq!(request.seed.take_profit, None, "a level of 0 means none was set");
        assert_eq!(request.target, Target::Reported { login: "52012345".into(), position_id: 77 });
    }

    #[test]
    fn reward_to_risk_follows_the_three_levels() {
        assert_eq!(reward_risk_text(Some(2648.0), Some(2644.0), Some(2660.0)), "1 : 3.0");
        assert_eq!(reward_risk_text(Some(2648.0), Some(2644.0), None), "—");
    }

    #[test]
    fn the_time_of_writing_is_on_the_servers_clock() {
        assert_eq!(server_time(1_000_000, Some(120)), 1_000_000 + 7_200_000);
        assert_eq!(server_time(1_000_000, None), 1_000_000);
    }

    #[test]
    fn saving_a_plan_writes_it_and_takes_the_trade_out_of_the_review_queue() {
        use tradelens_core::AccountInfo;
        let mut db = Db::in_memory().expect("opens");
        let account = db
            .upsert_account(&AccountInfo { login: "52012345".into(), server: "S".into(), ..AccountInfo::default() })
            .expect("account");
        let target = Target::Reported { login: "52012345".into(), position_id: 77 };
        let key = TradeKey { account_id: account, position_id: 77, segment: 0 };
        // "Ghi sau" first: the trade is flagged.
        let mut flagged = db.journal(&key).expect("reads").unwrap_or_default();
        flagged.review_later = true;
        db.save_journal(&key, &flagged).expect("saves");

        let draft = Draft {
            thesis: " Phá đỉnh phiên Á ".into(),
            entry: Some(2648.2),
            stop_loss: Some(2644.3),
            take_profit: Some(2660.0),
            risk: Some(150.0),
            timeframe: Some("M15".into()),
            playbook_id: None,
            grade: Some("a".into()),
            emotion: None,
            checklist: vec![1],
        };
        save_plan(&mut db, &target, &draft, 5_000).expect("the plan is saved");
        let entry = db.journal(&key).expect("reads").expect("an entry");
        let plan = entry.plan.expect("a plan");
        assert_eq!(plan.thesis, "Phá đỉnh phiên Á");
        assert_eq!(plan.timeframe, "M15");
        assert_eq!(plan.written_at, 5_000);
        assert_eq!(entry.setup_grade.as_deref(), Some("a"));
        assert_eq!(entry.planned_risk, Some(150.0));
        assert!(!entry.review_later);
        assert_eq!(entry.checklist, [1]);
    }

    #[test]
    fn a_plan_for_an_unknown_account_says_so() {
        let mut db = Db::in_memory().expect("opens");
        let target = Target::Reported { login: "999".into(), position_id: 1 };
        let draft = Draft {
            thesis: String::new(),
            entry: None,
            stop_loss: None,
            take_profit: None,
            risk: None,
            timeframe: None,
            playbook_id: None,
            grade: None,
            emotion: None,
            checklist: Vec::new(),
        };
        let error = save_plan(&mut db, &target, &draft, 0).expect_err("no such account");
        assert!(error.to_string().contains("999"));
    }
}
