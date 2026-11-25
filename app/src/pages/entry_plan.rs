//! Popup ghi kế hoạch: a modal over any screen that writes the plan of a trade (the reason,
//! entry, stop, target, risk, chart, setup). Only the trader opens it, from a trade's page
//! ("Viết kế hoạch" / "Sửa kế hoạch"); closing it (✕, Escape) writes nothing. Several plans
//! wait their turn, one modal at a time.
//! Design: `docs/design/screens/entry-plan-prompt.html`.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    ChecklistItem, Db, DbError, Side, Trade, TradeKey,
    catalog::{CatalogItem, CatalogKind},
    journal::EntryPlan,
    metrics::{self, Sample},
    rules::{self, ViolationKind},
};

use super::account_trades;
use crate::{
    format,
    journal_draft::{key_of_choice, key_or_empty, ticked, unticked},
    snapshot::TradeRow,
    store::{Reading, query, query_with, store, writer},
    ui::{
        chips::{Tone, chip},
        controls::{Choice, Variant, action_icon, select_value, textarea},
        forms::{NumberRange, checklist, number_field},
        icons::Icon,
        modal::{self, Modal},
        parts::warning_banner,
        theme as tokens,
    },
};

/// Width of the dialog's panel, in points.
const PANEL_WIDTH: f32 = 560.0;

/// A plan to be written: the trade it is for and what the dialog starts with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanRequest {
    pub(crate) target: TradeKey,
    pub(crate) symbol: String,
    pub(crate) side: Side,
    /// The lot size when the source gave one: a journal kept in R has none.
    pub(crate) volume: Option<f64>,
    /// The first line under the title: where it came from and how it stands.
    pub(crate) origin: String,
    /// The plan the dialog starts from: the MT5 levels, or the plan already written.
    pub(crate) seed: EntryPlan,
    /// The playbook setup already linked, when editing.
    pub(crate) playbook_id: Option<i64>,
}

impl PlanRequest {
    /// The form for a trade already listed, starting from `plan` when one was written and from
    /// the trade's own levels when not.
    pub(crate) fn of_row(row: &TradeRow, plan: Option<EntryPlan>, playbook_id: Option<i64>) -> Self {
        let level = |v: f64| (v > 0.0).then_some(v);
        let trade = &row.trade;
        Self {
            target: trade.key.clone(),
            symbol: trade.symbol.clone(),
            side: trade.side,
            volume: trade.volume_known().then_some(trade.volume),
            origin: match row.origin {
                tradelens_core::Origin::Mt5 => format!("{} · lệnh {}", row.account, trade.key.position_id),
                other => format!("{} · {}", row.account, other.label()),
            },
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

/// Puts a plan in line: it shows at once when no modal is on screen, else after the one that
/// is. A plan for a trade that is already in line is not added twice.
pub(crate) fn enqueue(request: PlanRequest) {
    let queue = global::<Queue>();
    if queue.with_untracked(|queue| queue.0.iter().any(|waiting| waiting.target == request.target)) {
        return;
    }
    queue.update(|queue| queue.0.push(request.clone()));
    modal::open(modal_of(request));
}

/// Takes the plan for `target` out of line, so that a second press of the same button cannot
/// take the next plan with it, and closes its modal: the next one shows.
fn finish(target: &TradeKey) {
    global::<Queue>().update(|queue| queue.0.retain(|waiting| waiting.target != *target));
    modal::close(&modal_id(target));
}

/// The name of the modal of the plan for `target`.
fn modal_id(target: &TradeKey) -> String {
    format!("entry-plan:{}:{}:{}", target.account_id, target.position_id, target.segment)
}

/// The modal of a plan. Escape and the close button just close it and show the next plan, writing
/// nothing; a press on the scrim does nothing, since what was typed in the form is not saved.
fn modal_of(request: PlanRequest) -> Modal {
    let target = request.target.clone();
    Modal::new(
        modal_id(&request.target),
        "Kế hoạch vào lệnh",
        move || panel(&request).into_element(),
        move || {
            finish(&target);
        },
    )
    .width(PANEL_WIDTH)
    .keeps_open_on_scrim()
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

/// The rules of the account that the plan, as it stands, would break by taking the trade:
/// the risk per trade (the planned `risk` in money over the account's balance), the trades of
/// the day, the day's loss and the losing streak. One line each, in Vietnamese; empty when
/// the account is unknown, has no rules on, or the plan is within them.
///
/// The trade is judged as the one just opened: the stored one when its deal is already in
/// (its own risk replaced by `planned_risk` when that is set), else one made from the
/// request. It is counted as a flat trade closed the instant after it opened, which puts it
/// among the day's trades without moving the day's loss or giving it a losing streak of its own.
fn plan_warnings(
    db: &Db,
    request: &PlanRequest,
    planned_risk: Option<f64>,
    now_utc_ms: i64,
) -> Result<Vec<String>, DbError> {
    let key = request.target.clone();
    let rules = db.rules(key.account_id)?;
    let Some(own) = account_trades::load(db, Some(key.account_id))?.into_iter().next() else {
        return Ok(Vec::new());
    };
    let mut planned = own.trades.iter().find(|t| t.key == key).cloned().unwrap_or_else(|| Trade {
        key: key.clone(),
        symbol: request.symbol.clone(),
        side: request.side,
        open_time: server_time(now_utc_ms, own.account.server_offset_min),
        close_time: None,
        volume: request.volume.unwrap_or_default(),
        open_price: request.seed.entry.unwrap_or_default(),
        close_price: None,
        sl: request.seed.stop_loss.unwrap_or_default(),
        tp: request.seed.take_profit.unwrap_or_default(),
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        magic: 0,
        comment: String::new(),
        manual: false,
    });
    if let Some(risk) = planned_risk.filter(|risk| *risk > 0.0) {
        planned.risk = Some(risk);
    }
    planned.close_time = Some(planned.open_time.saturating_add(1));
    planned.profit = 0.0;
    planned.commission = 0.0;
    planned.swap = 0.0;
    planned.fee = 0.0;
    let day = own.clock.day(planned.open_time);

    let curve = own.curve();
    let start = metrics::start_balance(&own.account, &own.samples(), &own.cash);
    let balance_at = |ms: i64| if curve.is_empty() { start } else { rules::balance_before(&curve, ms) };
    let samples: Vec<Sample<'_>> = own
        .trades
        .iter()
        .filter(|t| t.is_closed() && t.key != key)
        .chain(std::iter::once(&planned))
        .map(|trade| Sample { trade, r: None })
        .collect();
    let title = own.account.title();
    Ok(rules::evaluate(&rules, &samples, balance_at, own.clock)
        .into_iter()
        .filter(|v| v.trade.as_ref() == Some(&key) || (v.trade.is_none() && v.day == day))
        .map(|v| warning_line(&v.kind, &title))
        .collect())
}

/// What a broken rule says in the dialog: `Risk 1.9% vượt quy tắc 1.5% của tài khoản …`.
fn warning_line(kind: &ViolationKind, account: &str) -> String {
    match kind {
        ViolationKind::RiskPerTrade(percent, limit) => {
            format!("Risk {percent:.1}% vượt quy tắc {limit:.1}% của tài khoản {account}.")
        }
        ViolationKind::DailyTrades(count, limit) => {
            format!("Lệnh thứ {count} trong ngày vượt quy tắc {limit} lệnh/ngày của tài khoản {account}.")
        }
        ViolationKind::DailyLoss(loss, limit) => {
            format!("Lỗ trong ngày {loss:.2} đã vượt quy tắc {limit:.2} của tài khoản {account}.")
        }
        ViolationKind::AfterLosingStreak(streak, limit) => {
            format!("Vào lệnh sau {streak} lệnh thua liên tiếp, vượt quy tắc {limit} của tài khoản {account}.")
        }
    }
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
fn save_plan(db: &mut Db, key: &TradeKey, draft: &Draft, now_utc_ms: i64) -> Result<(), DbError> {
    let offset = db.accounts()?.into_iter().find(|a| a.id == key.account_id).and_then(|a| a.server_offset_min);
    let mut entry = db.journal(key)?.unwrap_or_default();
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
    db.save_journal(key, &entry)
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

/// The rule warnings of the plan, read again when the planned risk changes or the data does.
/// A memo, so that a read under way (which still holds the old lines) does not redraw them.
fn rule_warnings(request: PlanRequest, risk: Signal<Option<f64>>) -> Memo<Vec<String>> {
    let read = query_with(
        move || risk.get(),
        move |db, _, risk| plan_warnings(db, &request, *risk, chrono::Utc::now().timestamp_millis()),
    );
    computed(move || read.value().map_or_default(|lines| lines.to_vec()))
}

fn panel(request: &PlanRequest) -> impl IntoElement {
    let write = writer();
    let choices = query(load_choices);
    let fields = Fields::of(request);
    let busy = signal(false);
    let failure = signal(None::<String>);
    let waiting = computed(move || global::<Queue>().with(|q| q.0.len().saturating_sub(1)));
    let save = {
        let target = request.target.clone();
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
    let thesis = fields.thesis;
    let warnings = rule_warnings(request.clone(), fields.risk);
    view()
        .gap_4()
        .child(header(request, waiting))
        .child(dynamic(move || {
            let lines = warnings.get();
            untracked(|| view().gap_2().children(lines.into_iter().map(warning_banner)).into_element())
        }))
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
        .child(dynamic(move || failure.get().map(|error| text(error).text_sm().text_color(tokens::loss))))
        .child(footer(busy, save))
}

/// The title block: where the position came from, what it is, and how many wait behind it.
fn header(request: &PlanRequest, waiting: Memo<usize>) -> impl IntoElement {
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
    // A trade with no known lot (a journal kept in R) shows no lot, as the trade's page does.
    let figures: Vec<String> = [
        request.volume.map(|v| format!("{} lot", format::lots(v))),
        seed.entry.map(|p| format!("@ {}", format::price(p))),
    ]
    .into_iter()
    .flatten()
    .collect();
    view().flex_row().items_start().justify_between().gap_3().child(
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
                    .children(
                        (!figures.is_empty())
                            .then(|| text(figures.join(" ")).font_family(tokens::mono).tabular_nums().text_sm()),
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

fn footer(busy: Signal<bool>, save: impl Fn() + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text("Entry, SL, TP lấy từ lệnh · ảnh chart thêm ở chi tiết lệnh")
                .flex_1()
                .min_w(px(200.0))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(action_icon(Icon::Check, "Lưu kế hoạch", Variant::Primary, move || {
            if !busy.get_untracked() {
                save();
            }
        }))
}

#[cfg(test)]
mod tests {
    use tradelens_core::Rules;

    use super::*;

    /// The plan of position 77 of `account`, a buy of 0.5 XAUUSD the journal has not stored yet.
    fn request_for(account_id: i64) -> PlanRequest {
        PlanRequest {
            target: TradeKey { account_id, position_id: 77, segment: 0 },
            symbol: "XAUUSD".into(),
            side: Side::Long,
            volume: Some(0.5),
            origin: String::new(),
            seed: EntryPlan { entry: Some(2648.2), stop_loss: Some(2644.3), ..EntryPlan::default() },
            playbook_id: None,
        }
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

    /// An account with a balance of 10,000, one closed trade today, and `rules`.
    fn account_with_rules(rules: &Rules) -> (Db, PlanRequest) {
        use tradelens_core::{AccountInfo, Deal, DealBatch, DealEntry, DealKind, time::parse_mt5_time};
        let deal = |ticket: i64, at: &str, entry: DealEntry, profit: f64| Deal {
            ticket,
            order: ticket,
            position_id: 1,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "XAUUSD".into(),
            kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
            entry,
            volume: 0.5,
            price: 2648.2,
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
            result_r: None,
            gaps: tradelens_core::Gaps::NONE,
        };
        let mut db = Db::in_memory().expect("opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "52012345".into(),
                server: "Demo".into(),
                balance: Some(10_000.0),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, "2026.11.20 09:00:00", DealEntry::In, 0.0),
                deal(2, "2026.11.20 09:30:00", DealEntry::Out, -50.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        let account = db.accounts().expect("accounts").remove(0).id;
        db.save_rules(account, rules).expect("rules are saved");
        (db, request_for(account))
    }
    fn no_warnings() -> Vec<String> {
        Vec::new()
    }

    fn at_noon() -> i64 {
        tradelens_core::time::parse_mt5_time("2026.11.20 12:00:00").expect("a time")
    }

    #[test]
    fn a_planned_risk_over_the_accounts_limit_is_warned_about() {
        let (db, request) = account_with_rules(&Rules { max_risk_pct: Some(1.5), ..Rules::default() });
        let warned = plan_warnings(&db, &request, Some(300.0), at_noon()).expect("reads");
        assert_eq!(warned, ["Risk 3.0% vượt quy tắc 1.5% của tài khoản 52012345 · Demo."]);
        assert!(plan_warnings(&db, &request, Some(100.0), at_noon()).expect("reads").is_empty(), "1% is within it");
        assert!(plan_warnings(&db, &request, None, at_noon()).expect("reads").is_empty(), "no risk, nothing to judge");
    }

    #[test]
    fn the_trades_of_the_day_count_this_one() {
        let (db, request) = account_with_rules(&Rules { max_daily_trades: Some(1), ..Rules::default() });
        let warned = plan_warnings(&db, &request, None, at_noon()).expect("reads");
        assert_eq!(warned, ["Lệnh thứ 2 trong ngày vượt quy tắc 1 lệnh/ngày của tài khoản 52012345 · Demo."]);
        let (db, request) = account_with_rules(&Rules { max_daily_trades: Some(2), ..Rules::default() });
        assert_eq!(plan_warnings(&db, &request, None, at_noon()).expect("reads"), no_warnings());
        // Another day: the earlier trade is not today's.
        let tomorrow = at_noon() + 24 * 3_600_000;
        let (db, request) = account_with_rules(&Rules { max_daily_trades: Some(1), ..Rules::default() });
        assert_eq!(plan_warnings(&db, &request, None, tomorrow).expect("reads"), no_warnings());
    }

    #[test]
    fn a_losing_streak_and_a_blown_day_are_warned_about() {
        let rules = Rules { max_consecutive_losses: Some(1), max_daily_loss: Some(40.0), ..Rules::default() };
        let (db, request) = account_with_rules(&rules);
        let warned = plan_warnings(&db, &request, None, at_noon()).expect("reads");
        assert_eq!(warned.len(), 2, "{warned:?}");
        assert!(warned.iter().any(|w| w.starts_with("Lỗ trong ngày 50.00 đã vượt quy tắc 40.00")));
        assert!(warned.iter().any(|w| w.starts_with("Vào lệnh sau 1 lệnh thua liên tiếp")));
    }

    #[test]
    fn an_account_without_rules_or_unknown_has_no_warnings() {
        let (db, request) = account_with_rules(&Rules::default());
        assert_eq!(plan_warnings(&db, &request, Some(5_000.0), at_noon()).expect("reads"), no_warnings());
        let stranger = request_for(request.target.account_id + 1);
        assert_eq!(plan_warnings(&db, &stranger, Some(5_000.0), at_noon()).expect("reads"), no_warnings());
    }
    #[test]
    fn saving_a_plan_writes_it_and_leaves_the_review_flag_as_it_was() {
        use tradelens_core::AccountInfo;
        let mut db = Db::in_memory().expect("opens");
        let account = db
            .upsert_account(&AccountInfo { login: "52012345".into(), server: "S".into(), ..AccountInfo::default() })
            .expect("account");
        let key = TradeKey { account_id: account, position_id: 77, segment: 0 };
        // The trader flagged the trade for review in its journal: writing a plan leaves that alone.
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
        save_plan(&mut db, &key, &draft, 5_000).expect("the plan is saved");
        let entry = db.journal(&key).expect("reads").expect("an entry");
        let plan = entry.plan.expect("a plan");
        assert_eq!(plan.thesis, "Phá đỉnh phiên Á");
        assert_eq!(plan.timeframe, "M15");
        assert_eq!(plan.written_at, 5_000);
        assert_eq!(entry.setup_grade.as_deref(), Some("a"));
        assert_eq!(entry.planned_risk, Some(150.0));
        assert!(entry.review_later, "the flag the trader set is theirs to clear");
        assert_eq!(entry.checklist, [1]);
    }
}
