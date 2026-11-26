//! Chi tiết lệnh: facts of one trade, the plan set against what happened, the chart images and
//! the journal entry the trader keeps on it, drawn from the trade form the trader shaped.
//! Design: `docs/design/screens/trade-detail.html`.

use std::{rc::Rc, sync::Arc};

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    ChecklistItem, Db, DbError, DealEntry, DealKind, Gaps, Origin, Side, TradeKey,
    attachments::Attachment,
    catalog::{CatalogItem, CatalogKind, CustomValues, FieldDef, FieldScope, missing_expected},
    form::{BuiltinField, FormLayout},
    time::format_duration,
};

use super::{
    catalog_tags::catalog_tags,
    entry_plan,
    share::{self, ShareTarget},
};
use crate::{
    format::{self, Sign},
    journal_draft::{Draft, added, key_of_choice, key_or_empty, removed, suggested_exit, ticked, unticked, with_value},
    nav,
    snapshot::TradeRow,
    store::{Reading, Store, Writer, query, store, writer},
    ui::{
        chips::{Tone, chip, figure_text, pnl_text},
        controls::{
            Choice, Variant, action, action_icon, action_icon_when, action_when, confirm_then, rating, root_owner,
            select_value, text_field, textarea,
        },
        forms::{NumberRange, check_box, checklist, number_field},
        gallery::gallery,
        icons::Icon,
        parts::{card, empty_state, page_with, save_status},
        plan_vs_actual::{PlanView, plan_vs_actual},
        properties::{PropertyEdits, PropertyList, property_list},
        theme as tokens,
    },
};

/// What runs once a save has gone through.
type Then = Box<dyn FnOnce()>;

/// The page of one trade, or what to do when the trade is not there.
pub(crate) fn trade_detail(key: TradeKey) -> impl IntoElement {
    let store = store();
    let row = {
        let key = key.clone();
        computed(move || store.snapshot().row(&key).cloned())
    };
    when(move || row.with(Option::is_some), move || untracked(|| found_page(key.clone(), row))).otherwise(|| {
        page_with(
            "Chi tiết lệnh",
            "Không tìm thấy lệnh này",
            action("‹ Lệnh", Variant::Secondary, nav::close_trade),
            empty_state("Lệnh không còn trong dữ liệu", "Quay lại danh sách để chọn lệnh khác."),
        )
    })
}

/// Reads one value out of the row, empty while the row is gone.
fn from_row<T: Default>(row: Memo<Option<TradeRow>>, pick: impl Fn(&TradeRow) -> T) -> T {
    row.with(|r| r.as_ref().map(&pick)).unwrap_or_default()
}

/// What the trade form needs from the database besides the entry: the trader's arrangement of
/// the form and own properties, the option lists the built-ins pick from, the checklist.
#[derive(Debug, Clone)]
struct TradeForm {
    layout: FormLayout,
    defs: Vec<FieldDef>,
    grades: Vec<CatalogItem>,
    exits: Vec<CatalogItem>,
    markets: Vec<CatalogItem>,
    checklist: Vec<ChecklistItem>,
}

impl Default for TradeForm {
    fn default() -> Self {
        Self {
            layout: FormLayout::default_for(FieldScope::Trade),
            defs: Vec::new(),
            grades: Vec::new(),
            exits: Vec::new(),
            markets: Vec::new(),
            checklist: Vec::new(),
        }
    }
}

fn load_form(db: &Db, account: Option<i64>) -> Result<TradeForm, DbError> {
    Ok(TradeForm {
        layout: db.form_layout(FieldScope::Trade)?,
        defs: db.field_defs(FieldScope::Trade)?,
        grades: db.catalog(CatalogKind::SetupGrade)?,
        exits: db.catalog(CatalogKind::ExitReason)?,
        markets: db.catalog(CatalogKind::MarketCondition)?,
        checklist: db.checklist(account)?,
    })
}

/// The signals of the entry being edited: one per field, so typing in one never redraws another.
#[derive(Clone, Copy)]
pub(crate) struct JournalFields {
    playbook: Signal<Option<i64>>,
    setup: Signal<String>,
    grade: Signal<Option<String>>,
    exit: Signal<Option<String>>,
    market: Signal<Option<String>>,
    tags: Signal<Vec<String>>,
    mistakes: Signal<Vec<String>>,
    before: Signal<String>,
    after: Signal<String>,
    stars: Signal<u8>,
    followed: Signal<Option<bool>>,
    checked: Signal<Vec<i64>>,
    risk: Signal<Option<f64>>,
    later: Signal<bool>,
    notes: Signal<String>,
    custom: Signal<CustomValues>,
}

impl JournalFields {
    pub(crate) fn of(draft: Draft) -> Self {
        Self {
            playbook: signal(draft.playbook_id),
            setup: signal(draft.setup),
            grade: signal(draft.setup_grade),
            exit: signal(draft.exit_reason),
            market: signal(draft.market),
            tags: signal(draft.tags),
            mistakes: signal(draft.mistakes),
            before: signal(draft.emotion_before),
            after: signal(draft.emotion_after),
            stars: signal(draft.rating),
            followed: signal(draft.followed_plan),
            checked: signal(draft.checklist),
            risk: signal(draft.planned_risk),
            later: signal(draft.review_later),
            notes: signal(draft.notes),
            custom: signal(draft.custom),
        }
    }

    /// The fields as they are now (tracked where it is read).
    pub(crate) fn draft(&self) -> Draft {
        Draft {
            playbook_id: self.playbook.get(),
            setup: self.setup.get(),
            setup_grade: self.grade.get(),
            exit_reason: self.exit.get(),
            market: self.market.get(),
            tags: self.tags.get(),
            mistakes: self.mistakes.get(),
            emotion_before: self.before.get(),
            emotion_after: self.after.get(),
            rating: self.stars.get(),
            followed_plan: self.followed.get(),
            checklist: self.checked.get(),
            planned_risk: self.risk.get(),
            review_later: self.later.get(),
            notes: self.notes.get(),
            custom: self.custom.get(),
        }
    }
}

/// The edits of one page: whether they are saved, saving them, and leaving the page safely.
struct Editor {
    fields: JournalFields,
    /// The fields differ from what was last saved.
    dirty: Memo<bool>,
    busy: Signal<bool>,
    failure: Signal<Option<String>>,
    /// Saves the edits, then runs `then`; a failed save leaves the page as it is.
    persist: Rc<dyn Fn(Then)>,
    /// Runs `then` after saving, when something changed; at once when nothing did.
    leave: Rc<dyn Fn(Then)>,
}

impl Editor {
    fn new(key: &Rc<TradeKey>) -> Self {
        let store = store();
        let write = writer();
        let snapshot = store.snapshot_now();
        let deals = snapshot.row(key).map_or_default(|r| r.deals.clone());
        let initial = Draft::of_trade(&snapshot.journal(key), &deals);
        let saved = signal(initial.clone());
        let fields = JournalFields::of(initial);
        let dirty = computed(move || fields.draft() != saved.get());
        let busy = signal(false);
        let failure = signal(None::<String>);
        let persist: Rc<dyn Fn(Then)> = {
            let key = Rc::clone(key);
            Rc::new(move |then: Then| {
                if busy.get_untracked() {
                    return;
                }
                let draft = fields.draft();
                busy.set(true);
                failure.set(None);
                let stored = draft.clone();
                let key = TradeKey::clone(&key);
                write.run(
                    // Read the entry again: the bridge may have attached screenshots since the page opened.
                    move |db| {
                        let mut entry = db.journal(&key)?.unwrap_or_default();
                        draft.apply(&mut entry);
                        db.save_journal(&key, &entry)
                    },
                    move |result| {
                        busy.set(false);
                        match result {
                            Ok(()) => {
                                saved.set(stored);
                                then();
                            }
                            Err(error) => failure.set(Some(error)),
                        }
                    },
                );
            })
        };
        let leave: Rc<dyn Fn(Then)> = {
            let persist = Rc::clone(&persist);
            Rc::new(move |go: Then| if dirty.get_untracked() { persist(go) } else { go() })
        };
        Self { fields, dirty, busy, failure, persist, leave }
    }

    /// What the line under the form says about the edits.
    fn status(&self) -> impl Fn() -> String + use<> {
        let (busy, failure, dirty) = (self.busy, self.failure, self.dirty);
        move || {
            if busy.get() {
                "Đang lưu…".to_owned()
            } else if let Some(error) = failure.get() {
                error
            } else if dirty.get() {
                "Có thay đổi chưa lưu".to_owned()
            } else {
                "Đã lưu".to_owned()
            }
        }
    }

    fn save(&self) -> impl Fn() + use<> {
        let persist = Rc::clone(&self.persist);
        move || persist(Box::new(|| {}))
    }

    fn back(&self) -> impl Fn() + use<> {
        let leave = Rc::clone(&self.leave);
        move || leave(Box::new(nav::close_trade))
    }

    /// The button that goes `step` trades away in the list (which runs newest first).
    fn step_button(&self, key: &Rc<TradeKey>, label: &'static str, step: isize) -> impl IntoElement + use<> {
        let (leave, key) = (Rc::clone(&self.leave), Rc::clone(key));
        action(label, Variant::Secondary, move || {
            let snapshot = store().snapshot_now();
            let target = snapshot
                .rows
                .iter()
                .position(|r| r.key() == &*key)
                .and_then(|at| at.checked_add_signed(step))
                .and_then(|at| snapshot.rows.get(at))
                .map(|r| r.key().clone());
            if let Some(target) = target {
                leave(Box::new(move || nav::show_trade(target)));
            }
        })
    }

    /// The link to the form editor: saves first, so no edit is lost to the page it replaces.
    fn form_editor_link(&self) -> impl IntoElement + use<> {
        let leave = Rc::clone(&self.leave);
        action("Tuỳ biến form", Variant::Ghost, move || {
            leave(Box::new(|| nav::open_form_editor(FieldScope::Trade)));
        })
    }
}

fn found_page(key: TradeKey, row: Memo<Option<TradeRow>>) -> impl IntoElement {
    let key = Rc::new(key);
    let editor = Editor::new(&key);
    let (dirty, busy) = (editor.dirty, editor.busy);
    page_with(
        "Chi tiết lệnh",
        move || {
            // The terminal's ticket for its trades; for the others, where they came from.
            from_row(row, |r| {
                let which = match r.origin {
                    Origin::Mt5 => r.trade.key.position_id.to_string(),
                    other => other.label().to_owned(),
                };
                format!("{} · {which}", r.trade.symbol)
            })
        },
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(edit_button(&key, row, &editor))
            .child(delete_button(&key, row, editor.failure))
            .child(share_button(&key, row))
            .child(action_when("Lưu", Variant::Primary, move || dirty.get() && !busy.get(), editor.save())),
        view().gap_4().child(breadcrumb(row, editor.back())).child(
            view()
                .flex_row()
                .flex_wrap()
                .items_start()
                .gap_6()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(460.0))
                        .gap_6()
                        .child(facts_card(row))
                        .child(plan_card(&key))
                        .child(screenshots_card(&key, editor.failure))
                        .child(dynamic(move || {
                            // The two deals the app wrote for an imported journal's trade hold nothing to list.
                            from_row(row, |r| r.origin == Origin::Mt5).then(|| deals_card(row))
                        })),
                )
                .child(view().w(px(520.0)).child(journal_card(
                    &editor,
                    row,
                    editor.step_button(&key, "‹ Lệnh trước", 1),
                    editor.step_button(&key, "Lệnh sau ›", -1),
                ))),
        ),
    )
}

/// What the trader may do with the facts of a trade, as the button and the shortcut name it.
const EDIT: &str = "Sửa lệnh";

/// "Sửa lệnh" (and Ctrl+E): the form of the trade's facts, for a trade the trader typed or imported
/// from their journal. A trade of the terminal has no such button: its facts are the terminal's.
/// The journal is saved first, so that no edit is lost to the form that replaces the page.
fn edit_button(key: &Rc<TradeKey>, row: Memo<Option<TradeRow>>, editor: &Editor) -> impl IntoElement {
    let open = {
        let (leave, key) = (Rc::clone(&editor.leave), Rc::clone(key));
        Rc::new(move || {
            if row.with_untracked(|r| r.as_ref().is_some_and(|r| r.origin.is_editable())) {
                let key = (*key).clone();
                leave(Box::new(move || nav::open_edit_trade(key)));
            }
        })
    };
    let shortcut = Rc::clone(&open);
    on_key_with("mod+e", KeyOptions { pass_through: true, ..KeyOptions::default() }, move |_| shortcut());
    dynamic(move || {
        let open = Rc::clone(&open);
        from_row(row, |r| r.origin.is_editable())
            .then(|| action_icon(Icon::Edit, EDIT, Variant::Secondary, move || open()))
    })
}

/// The question before a trade or several are deleted: its title and what it says.
pub(crate) fn delete_question(count: usize, symbol: &str, origin: Origin) -> (String, String) {
    let what = if count == 1 { format!("lệnh {symbol}") } else { format!("{count} lệnh") };
    let mut said = format!(
        "Xoá {what} cùng nhật ký và ảnh chart của {}. Không hoàn tác được.",
        if count == 1 { "nó" } else { "chúng" }
    );
    if origin == Origin::Mt5 {
        said.push_str(" Lệnh từ MT5: import lại cùng báo cáo sẽ đưa lệnh trở lại.");
    }
    (if count == 1 { "Xoá lệnh?".to_owned() } else { format!("Xoá {count} lệnh?") }, said)
}

/// Deletes the trades, and the images of their journals that nothing else uses, then runs `done`.
pub(crate) fn delete_trades(write: Writer, keys: Vec<TradeKey>, done: impl FnOnce(Result<(), String>) + 'static) {
    write.run(move |db| db.delete_trades(&keys).map(|_| ()), done);
}

/// "Xoá lệnh": asks, then deletes the trade with its journal, and goes back to the list.
fn delete_button(key: &Rc<TradeKey>, row: Memo<Option<TradeRow>>, failure: Signal<Option<String>>) -> impl IntoElement {
    let key = Rc::clone(key);
    let (asker, write) = (root_owner(), writer());
    action_icon(Icon::Trash, "Xoá lệnh", Variant::Destructive, move || {
        let (symbol, origin) = row
            .with_untracked(|r| r.as_ref().map(|r| (r.trade.symbol.clone(), r.origin)))
            .unwrap_or((String::new(), Origin::Manual));
        let (title, said) = delete_question(1, &symbol, origin);
        let key = (*key).clone();
        confirm_then(asker, title, said, "Xoá lệnh", move || {
            delete_trades(write, vec![key], move |result| match result {
                Ok(()) => nav::close_trade(),
                Err(error) => {
                    failure.try_set(Some(error));
                }
            });
        });
    })
}

/// "Chia sẻ": opens the share dialog on this trade; a trade still open has no result to share.
fn share_button(key: &Rc<TradeKey>, row: Memo<Option<TradeRow>>) -> impl IntoElement {
    let key = Rc::clone(key);
    action_icon_when(
        Icon::Share,
        "Chia sẻ",
        Variant::Secondary,
        move || row.with(|r| r.as_ref().is_some_and(|r| r.trade.is_closed())),
        move || share::open(ShareTarget::Trade((*key).clone())),
    )
}

/// The way back to the list, with where the trade belongs.
fn breadcrumb(row: Memo<Option<TradeRow>>, back: impl Fn() + 'static) -> impl IntoElement {
    view().flex_row().items_center().gap_2().child(action("‹ Lệnh", Variant::Ghost, back)).child(
        text(move || from_row(row, |r| format!("{} · {}", r.account, r.trade.symbol)))
            .text_sm()
            .text_color(move || theme().muted_foreground),
    )
}

/// A label over a value, in the facts grid.
fn fact(label: &'static str, value: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .gap_1()
        .w(px(168.0))
        .child(text(label).text_xs().text_color(move || theme().muted_foreground))
        .child(figure_text(value).text_sm())
}

fn facts_card(row: Memo<Option<TradeRow>>) -> impl IntoElement {
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(title_row(row))
        .child(
            text(move || {
                from_row(row, |r| {
                    // Where the trade came from is how its deals were written, not the account it is filed under.
                    let origin = r.origin;
                    // Only a trade of the terminal is made of real deals; the others hold the two the app wrote.
                    if origin == Origin::Mt5 {
                        format!("{} · nguồn: {} · gộp {} deal", r.account, origin.label(), r.deals.len())
                    } else {
                        format!("{} · nguồn: {}", r.account, origin.label())
                    }
                })
            })
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(dynamic(move || {
            // What the trader cannot change here, and what they still can.
            from_row(row, |r| r.origin == Origin::Mt5).then(|| {
                text("Số liệu của lệnh lấy từ MT5 nên không sửa được; nhật ký và ảnh chart vẫn sửa được.")
                    .text_xs()
                    .text_color(move || theme().muted_foreground)
            })
        }))
        .child(facts_grid(row))
}

/// The symbol, side and state at the left, the result at the right.
fn title_row(row: Memo<Option<TradeRow>>) -> impl IntoElement {
    view()
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
                .child(
                    text(move || from_row(row, |r| r.trade.symbol.clone())).text_xl().font_weight(FontWeight::SEMIBOLD),
                )
                .child(dynamic(move || {
                    let (side, closed) = from_row(row, |r| (Some(r.trade.side), r.trade.is_closed()));
                    let side = side.unwrap_or(Side::Long);
                    let tone = if side == Side::Long { Tone::Profit } else { Tone::Loss };
                    view()
                        .flex_row()
                        .gap_2()
                        .child(chip(side.label(), tone))
                        .child(chip(if closed { "Đã đóng" } else { "Đang mở" }, Tone::Neutral))
                })),
        )
        .child(
            view()
                .flex_row()
                .items_baseline()
                .gap_2()
                .child(pnl_text(move || {
                    from_row(row, |r| {
                        if r.trade.is_closed() && r.trade.money_known() {
                            format::money(r.trade.net(), Sign::Always, &r.currency)
                        } else {
                            String::new()
                        }
                    })
                }))
                .child(pnl_text(move || {
                    from_row(row, |r| if r.trade.is_closed() { format::r_text(r.r) } else { String::new() })
                })),
        )
}

/// The facts of a trade as the page words them: what its source did not tell is a dash, never the
/// placeholder the deal holds (lot 1, price 0, midnight, $0.00).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Facts {
    opened: String,
    closed: String,
    hold: String,
    volume: String,
    open_price: String,
    close_price: String,
    stop_loss: String,
    take_profit: String,
    risk: String,
    commission: String,
    swap: String,
    /// The terminal's ticket; `None` for a trade that has none (typed by hand, imported from a journal).
    ticket: Option<String>,
}

const UNKNOWN: &str = "—";

/// A moment, with its time of day only when the source gave one.
fn moment(row: &TradeRow, at: chrono::NaiveDateTime) -> String {
    if row.trade.time_known() { format::full_stamp(at) } else { at.format("%d/%m/%Y").to_string() }
}

fn facts_of(row: &TradeRow) -> Facts {
    let trade = &row.trade;
    let known = |shown: String, is_known: bool| if is_known { shown } else { UNKNOWN.to_owned() };
    let money = |value: f64| known(format::money(value, Sign::OnlyNegative, &row.currency), trade.money_known());
    let prices = trade.prices_known();
    Facts {
        opened: moment(row, row.opened()),
        closed: row.closed().map_or_else(|| "Đang mở".to_owned(), |at| moment(row, at)),
        hold: trade.hold_secs().map_or_else(|| UNKNOWN.to_owned(), format_duration),
        volume: known(format!("{} lot", format::lots(trade.volume)), trade.volume_known()),
        open_price: known(format::price_to(trade.open_price, row.digits), prices),
        close_price: known(
            trade.close_price.map_or_else(|| UNKNOWN.to_owned(), |p| format::price_to(p, row.digits)),
            prices,
        ),
        stop_loss: known(format::level_to(trade.sl, row.digits), prices),
        take_profit: known(format::level_to(trade.tp, row.digits), prices),
        risk: known(
            trade
                .risk
                .map_or_else(|| UNKNOWN.to_owned(), |risk| format::money(risk, Sign::OnlyNegative, &row.currency)),
            prices && trade.money_known(),
        ),
        commission: money(trade.commission + trade.fee),
        swap: money(trade.swap),
        ticket: (row.origin == Origin::Mt5).then(|| trade.key.position_id.to_string()),
    }
}

fn facts_grid(row: Memo<Option<TradeRow>>) -> impl IntoElement {
    let facts = computed(move || from_row(row, facts_of));
    let pick = move |get: fn(&Facts) -> &str| move || facts.with(|f| get(f).to_owned());
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(fact("Vào lệnh", pick(|f| &f.opened)))
        .child(fact("Đóng lệnh", pick(|f| &f.closed)))
        .child(fact("Thời gian giữ", pick(|f| &f.hold)))
        .child(fact("Khối lượng", pick(|f| &f.volume)))
        .child(fact("Giá vào", pick(|f| &f.open_price)))
        .child(fact("Giá ra TB", pick(|f| &f.close_price)))
        .child(fact("Stop loss", pick(|f| &f.stop_loss)))
        .child(fact("Take profit", pick(|f| &f.take_profit)))
        .child(fact("Risk tại SL", pick(|f| &f.risk)))
        .child(fact("Commission", pick(|f| &f.commission)))
        .child(fact("Swap", pick(|f| &f.swap)))
        .child(dynamic(move || {
            let ticket = facts.with(|f| f.ticket.clone());
            // A ticket is the terminal's: hand-typed and imported trades have none to show.
            ticket.map(|ticket| fact("Ticket position", move || ticket.clone()))
        }))
}

/// The plan set against what happened; its button opens the plan form over the page.
fn plan_card(key: &Rc<TradeKey>) -> impl IntoElement {
    let store = store();
    let key = Rc::clone(key);
    let plan_key = Rc::clone(&key);
    let view_of = move || {
        let snapshot = store.snapshot();
        let trade = snapshot.row(&plan_key)?;
        let plan = snapshot.journal(&plan_key).plan?;
        Some(PlanView { plan, trade: trade.trade.clone(), currency: trade.currency.clone(), digits: trade.digits })
    };
    let write_plan = move || {
        let snapshot = store.snapshot_now();
        if let Some(row) = snapshot.row(&key) {
            let entry = snapshot.journal(&key);
            entry_plan::enqueue(entry_plan::PlanRequest::of_row(row, entry.plan, entry.playbook_id));
        }
    };
    plan_vs_actual(view_of, write_plan)
}

/// The gallery of chart images saved into the journal, with how many there are.
fn screenshots_card(key: &Rc<TradeKey>, failure: Signal<Option<String>>) -> impl IntoElement {
    let store = store();
    let write = writer();
    let key = stored(TradeKey::clone(key));
    let shots = signal(store.snapshot_now().journal(&key.with(|k| k.clone())).screenshots);
    // Pictures the journal gains while the page is open (a sync) appear without a reload of the page.
    effect(move || {
        let current = store.snapshot().journal(&key.with(|k| k.clone())).screenshots;
        if current != shots.get_untracked() {
            shots.set(current);
        }
    });
    card(
        "Ảnh chart",
        view()
            .gap_2()
            .child(
                text(move || {
                    let all = shots.with(Vec::len);
                    if all == 0 { String::new() } else { format!("{all} ảnh") }
                })
                .text_xs()
                .text_color(move || theme().muted_foreground),
            )
            .child(gallery(
                "Ảnh chart",
                move || shots.get(),
                move |list| {
                    shots.set(list.clone());
                    let key = key.with(|k| k.clone());
                    write.run(
                        move |db| save_screenshots(db, &key, list),
                        move |result| {
                            if let Err(error) = result {
                                failure.set(Some(error));
                            }
                        },
                    );
                },
            )),
    )
}

/// Writes the gallery's list over the entry stored now, so the rest of the entry (and what the
/// EA stored meanwhile) stays as it is.
fn save_screenshots(db: &mut Db, key: &TradeKey, list: Vec<Attachment>) -> Result<(), DbError> {
    let mut entry = db.journal(key)?.unwrap_or_default();
    entry.screenshots = list;
    db.save_journal(key, &entry)
}

fn entry_label(entry: DealEntry) -> &'static str {
    match entry {
        DealEntry::In => "in",
        DealEntry::Out => "out",
        DealEntry::InOut => "in/out",
        DealEntry::OutBy => "out by",
        DealEntry::None => "—",
    }
}

fn deals_card(row: Memo<Option<TradeRow>>) -> impl IntoElement {
    card(
        "Deal của lệnh",
        view().gap_2().child(dynamic(move || {
            let lines = from_row(row, |r| {
                r.deals
                    .iter()
                    .map(|d| {
                        let at = r
                            .local(d.time_msc)
                            .format(if d.gaps.has(Gaps::TIME) { "%d/%m" } else { "%d/%m %H:%M:%S" })
                            .to_string();
                        let profit = match d.entry {
                            DealEntry::In => "—".to_owned(),
                            _ if !r.trade.money_known() => "—".to_owned(),
                            _ => format::money(d.profit, Sign::Always, &r.currency),
                        };
                        let lots = if d.gaps.has(Gaps::VOLUME) { "—".to_owned() } else { format::lots(d.volume) };
                        let price = if d.gaps.has(Gaps::PRICE) {
                            "—".to_owned()
                        } else {
                            format::price_to(d.price, r.digits)
                        };
                        (at, entry_label(d.entry), d.kind, lots, price, profit)
                    })
                    .collect::<Vec<_>>()
            });
            view().gap_2().children(lines.into_iter().map(|(at, entry, kind, lots, price, profit)| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(figure_text(move || at.clone()).text_xs().w(px(112.0)))
                    .child(chip(entry, Tone::Neutral))
                    .child(text(if kind == DealKind::Buy { "Buy" } else { "Sell" }).w(px(40.0)).text_sm())
                    .child(figure_text(move || lots.clone()).text_sm().w(px(56.0)))
                    .child(figure_text(move || price.clone()).text_sm().flex_1())
                    .child(pnl_text(move || profit.clone()))
            }))
        })),
    )
}

// ---------------------------------------------------------------------------------------------
// The journal: the trader's form, drawn by the property list.
// ---------------------------------------------------------------------------------------------

/// What a built-in property's editor needs: the entry's signals and the form's data.
#[derive(Clone, Copy)]
struct Context {
    fields: JournalFields,
    store: Store,
    form: Reading<TradeForm>,
    /// How many chart images the entry holds, for the "Ảnh chart" property.
    shots: Memo<usize>,
    /// The exit reason the exit deal suggests, shown beside the choice.
    suggestion: Option<&'static str>,
}

impl Context {
    /// The form's data, empty until it is read.
    fn data(&self) -> Arc<TradeForm> {
        self.form.value().unwrap_or_default()
    }
}

/// The choices of a single-value property: nothing chosen first, then the catalog's options
/// that are still in use.
fn choices_of(items: &[CatalogItem]) -> Vec<Choice> {
    let mut choices = vec![Choice::new("", "Chưa chọn")];
    choices.extend(items.iter().filter(|i| !i.archived).map(|i| Choice::new(i.key.clone(), i.label.clone())));
    choices
}

/// The label of the option `key` in `items`.
fn label_of(items: &[CatalogItem], key: &str) -> Option<String> {
    items.iter().find(|i| i.key == key).map(|i| i.label.clone())
}

/// A drop-down over an optional catalog key.
fn catalog_select(
    label: &'static str,
    ctx: &Context,
    pick: fn(&TradeForm) -> &Vec<CatalogItem>,
    value: Signal<Option<String>>,
) -> impl IntoElement {
    let ctx = *ctx;
    select_value(
        label,
        move || choices_of(pick(&ctx.data())),
        move || SharedString::from(key_or_empty(value.get().as_deref())),
        move |chosen| value.set(key_of_choice(&chosen)),
    )
}

/// A drop-down over an emotion, kept as its key (empty for none).
fn emotion_select(label: &'static str, ctx: &Context, value: Signal<String>) -> impl IntoElement {
    let ctx = *ctx;
    select_value(
        label,
        move || choices_of(&ctx.store.snapshot().catalogs.emotions),
        move || SharedString::from(value.get()),
        move |chosen| value.set(chosen.to_string()),
    )
}

/// The editor of a built-in property of the trade form; `None` for one this form does not have.
fn builtin_editor(ctx: &Context, field: BuiltinField) -> Option<AnyElement> {
    use BuiltinField as B;
    let ctx = *ctx;
    let fields = ctx.fields;
    Some(match field {
        B::Playbook => playbook_editor(fields).into_element(),
        B::SetupGrade => catalog_select("Hạng setup", &ctx, |f| &f.grades, fields.grade).into_element(),
        B::ExecutionRating => {
            let stars = fields.stars;
            rating("Điểm thực thi", move || stars.get(), move |n| stars.set(n)).into_element()
        }
        B::ExitReason => exit_editor(&ctx).into_element(),
        B::Market => catalog_select("Trạng thái thị trường", &ctx, |f| &f.markets, fields.market).into_element(),
        B::Tags => {
            let tags = fields.tags;
            catalog_tags(
                CatalogKind::Tag,
                "Tag",
                "+ Thêm tag, rồi Enter",
                move || ctx.store.snapshot().catalogs.tags.clone(),
                move || tags.get(),
                move |key, on| tags.update(|t| *t = if on { added(t, key) } else { removed(t, key) }),
            )
            .into_element()
        }
        B::Mistakes => {
            let mistakes = fields.mistakes;
            catalog_tags(
                CatalogKind::Mistake,
                "Lỗi",
                "+ Thêm lỗi, rồi Enter",
                move || ctx.store.snapshot().catalogs.mistakes.clone(),
                move || mistakes.get(),
                move |key, on| mistakes.update(|m| *m = if on { added(m, key) } else { removed(m, key) }),
            )
            .into_element()
        }
        B::EmotionBefore => emotion_select("Cảm xúc trước lệnh", &ctx, fields.before).into_element(),
        B::EmotionAfter => emotion_select("Cảm xúc sau lệnh", &ctx, fields.after).into_element(),
        B::FollowedPlan => followed_editor(fields.followed).into_element(),
        B::Checklist => {
            let checked = fields.checked;
            checklist(
                "Checklist trước lệnh",
                move || ctx.data().checklist.clone(),
                move || checked.get(),
                move |id, on| checked.update(|c| *c = if on { ticked(c, id) } else { unticked(c, id) }),
            )
            .into_element()
        }
        B::Notes => {
            let notes = fields.notes;
            textarea(
                "Ghi chú",
                "Vì sao vào, quản lý lệnh ra sao, rút ra điều gì…",
                6,
                move || notes.get(),
                move |text| notes.set(text.to_owned()),
            )
            .into_element()
        }
        B::Screenshots => {
            let shots = ctx.shots;
            text(move || match shots.get() {
                0 => "Chưa có ảnh · thêm ở khối Ảnh chart".to_owned(),
                n => format!("{n} ảnh · xem ở khối Ảnh chart"),
            })
            .text_sm()
            .pt_2()
            .text_color(move || theme().muted_foreground)
            .into_element()
        }
        B::PlannedRisk => {
            let risk = fields.risk;
            number_field(
                "Risk dự kiến",
                None,
                NumberRange::any().stepping(1.0),
                move || risk.get(),
                move |n| {
                    risk.set(n);
                },
            )
            .into_element()
        }
        B::ReviewLater => {
            let later = fields.later;
            check_box("Xem lại sau", move || later.get(), move |on| later.set(on)).into_element()
        }
        _ => return None,
    })
}

/// The setup: a playbook entry when the trader has some, else (or while none is linked) a name.
fn playbook_editor(fields: JournalFields) -> impl IntoElement {
    let store = store();
    let (playbook, setup) = (fields.playbook, fields.setup);
    let choices = computed(move || {
        let named: Vec<Choice> = store
            .snapshot()
            .playbooks
            .iter()
            .filter(|p| !p.archived)
            .map(|p| Choice::new(p.id.to_string(), p.name.clone()))
            .collect();
        if named.is_empty() {
            return named;
        }
        let mut all = vec![Choice::new("", "Chưa gắn")];
        all.extend(named);
        all
    });
    let any = computed(move || choices.with(|c| !c.is_empty()));
    // The name field is its own block: a playbook stored meanwhile (a sync) brings the drop-down but
    // must not make the field again while a name is being typed in it.
    let names_it = computed(move || !any.get() || playbook.get().is_none());
    view()
        .gap_2()
        .child(dynamic(move || {
            any.get().then(|| {
                untracked(|| {
                    select_value(
                        "Playbook",
                        move || choices.get(),
                        move || SharedString::from(playbook.get().map_or_default(|id| id.to_string())),
                        move |value| playbook.set(value.parse().ok()),
                    )
                })
            })
        }))
        .child(dynamic(move || {
            names_it.get().then(|| {
                untracked(|| {
                    text_field(
                        "Setup",
                        None,
                        "Tên setup, ví dụ Breakout phiên Á",
                        move || setup.get(),
                        move |text| setup.set(text.to_owned()),
                    )
                })
            })
        }))
}

/// The exit reason, with what the exit deal suggests beside it.
fn exit_editor(ctx: &Context) -> impl IntoElement {
    let ctx = *ctx;
    let exit = ctx.fields.exit;
    let suggestion = ctx.suggestion;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(catalog_select("Lý do thoát lệnh", &ctx, |f| &f.exits, exit))
        .child(dynamic(move || {
            let hint = suggestion.and_then(|key| label_of(&ctx.data().exits, key))?;
            Some(text(format!("Gợi ý từ deal đóng: {hint}")).text_xs().text_color(move || theme().muted_foreground))
        }))
}

/// Whether the trade followed its plan: not judged, yes, or no.
fn followed_editor(followed: Signal<Option<bool>>) -> impl IntoElement {
    select_value(
        "Đúng kế hoạch",
        || vec![Choice::new("", "Chưa đánh giá"), Choice::new("yes", "Có"), Choice::new("no", "Không")],
        move || {
            SharedString::from(match followed.get() {
                None => "",
                Some(true) => "yes",
                Some(false) => "no",
            })
        },
        move |chosen| {
            followed.set(match &*chosen {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            });
        },
    )
}

/// The line under the form that names what the trader expects to fill and has not.
fn expected_note(defs: Signal<Vec<FieldDef>>, custom: Signal<CustomValues>) -> impl IntoElement {
    text(move || {
        let missing = defs.with(|d| custom.with(|c| missing_expected(d, FieldScope::Trade, c).join(", ")));
        if missing.is_empty() { String::new() } else { format!("Nên điền: {missing}") }
    })
    .text_xs()
    .text_color(tokens::warning)
}

/// The journal's property list, drawn from the trade form the trader arranged: the entry's
/// `fields` are edited in place, the arrangement and the trader's own properties are saved at
/// once (a failure goes to `failure`). `shots` counts the chart images and `suggestion` is the
/// exit reason the exit deal hints at, for the properties that show them. Shared with the
/// manual entry form.
pub(crate) fn journal_form(
    fields: JournalFields,
    shots: Memo<usize>,
    suggestion: Option<&'static str>,
    failure: Signal<Option<String>>,
) -> impl IntoElement {
    let write = writer();
    // Edits come from popovers and dialogs that close at once; their writes belong to the page.
    let owner = Owner::current();
    let form = query(load_form);
    // The form as the trader arranged it, kept in signals so that an edit shows at once; what
    // the database says replaces them whenever it is read again.
    let layout = signal(FormLayout::default_for(FieldScope::Trade));
    let defs = signal(Vec::<FieldDef>::new());
    effect(move || {
        if let Some(read) = form.value() {
            layout.set(read.layout.clone());
            defs.set(read.defs.clone());
        }
    });
    let ctx = Context { fields, store: store(), form, shots, suggestion };
    let report = move |result: Result<(), String>| {
        if let Err(error) = result {
            failure.set(Some(error));
        }
    };
    let edits = PropertyEdits::new(
        move |next: FormLayout| {
            layout.set(next.clone());
            owner.run(|| write.run(move |db| db.save_form_layout(&next), report));
        },
        move |key: &str, value| fields.custom.update(|c| *c = with_value(std::mem::take(c), key, value)),
        move |def: FieldDef| {
            defs.update(|all| match all.iter_mut().find(|d| d.key == def.key) {
                Some(known) => *known = FieldDef { id: known.id, ..def.clone() },
                None => all.push(def.clone()),
            });
            // Every entry of the form follows a new type or a renamed option, not only this one.
            owner.run(|| write.run(move |db| db.apply_field_def(&def).map(|_| ()), report));
        },
    );
    let list = PropertyList::new(move || layout.get(), move || defs.get(), move || fields.custom.get(), edits)
        .builtin(move |field| builtin_editor(&ctx, field));
    view().gap_3().child(property_list(list)).child(expected_note(defs, fields.custom))
}

fn journal_card(
    editor: &Editor,
    row: Memo<Option<TradeRow>>,
    older: impl IntoElement,
    newer: impl IntoElement,
) -> impl IntoElement {
    let store = store();
    let shots = computed(move || {
        row.with(|r| r.as_ref().map(|r| store.snapshot().journal(r.key()).screenshots.len())).unwrap_or(0)
    });
    let suggestion = {
        let snapshot = store.snapshot_now();
        row.with_untracked(|r| r.as_ref().and_then(|r| suggested_exit(&snapshot.row(r.key())?.deals)))
    };
    card(
        "Nhật ký lệnh",
        view().gap_3().child(journal_form(editor.fields, shots, suggestion, editor.failure)).child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(save_status(editor.status()).text_xs())
                        .child(editor.form_editor_link()),
                )
                .child(view().flex_row().gap_2().child(older).child(newer)),
        ),
    )
}
