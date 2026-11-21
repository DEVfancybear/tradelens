//! Chi tiết lệnh: facts of one trade and the journal entry the trader keeps on it.
//! Design: `docs/design/screens/trade-detail.html` (v0.1 parts; plan vs actual and the
//! customisable form arrive in v0.2).

use std::rc::Rc;

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    DealEntry, DealKind, Side, TradeKey, attachments::Source, catalog::CatalogItem, time::format_duration,
};

use crate::{
    format::{self, Sign},
    journal_draft::{Draft, added, removed},
    nav,
    snapshot::TradeRow,
    store::store,
    ui::{
        chips::{Tone, chip, figure_text, pnl_text},
        controls::{Choice, Variant, action, action_when, rating, select_value, tag_picker, text_field, textarea},
        parts::{card, empty_state, page_with, planned, property_row},
    },
};

const PLANNED: &[&str] = &[
    "Kế hoạch lúc vào vs thực tế: giá vào, SL, TP, R:R",
    "Hạng setup, lý do thoát lệnh, trạng thái thị trường",
    "Thuộc tính riêng kiểu Notion (Tuỳ biến form)",
];

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

/// The signals of the entry being edited: one per field, so typing in one never redraws another.
#[derive(Clone, Copy)]
struct JournalFields {
    playbook: Signal<Option<i64>>,
    setup: Signal<String>,
    tags: Signal<Vec<String>>,
    mistakes: Signal<Vec<String>>,
    before: Signal<String>,
    after: Signal<String>,
    grade: Signal<u8>,
    notes: Signal<String>,
}

impl JournalFields {
    fn of(draft: Draft) -> Self {
        Self {
            playbook: signal(draft.playbook_id),
            setup: signal(draft.setup),
            tags: signal(draft.tags),
            mistakes: signal(draft.mistakes),
            before: signal(draft.emotion_before),
            after: signal(draft.emotion_after),
            grade: signal(draft.rating),
            notes: signal(draft.notes),
        }
    }

    /// The fields as they are now (tracked where it is read).
    fn draft(&self) -> Draft {
        Draft {
            playbook_id: self.playbook.get(),
            setup: self.setup.get(),
            tags: self.tags.get(),
            mistakes: self.mistakes.get(),
            emotion_before: self.before.get(),
            emotion_after: self.after.get(),
            rating: self.grade.get(),
            notes: self.notes.get(),
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
        let initial = Draft::of(&store.snapshot_now().journal(key));
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
                store.save_draft(TradeKey::clone(&key), draft, move |result| {
                    busy.set(false);
                    match result {
                        Ok(()) => {
                            saved.set(stored);
                            then();
                        }
                        Err(error) => failure.set(Some(error)),
                    }
                });
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
}

fn found_page(key: TradeKey, row: Memo<Option<TradeRow>>) -> impl IntoElement {
    let key = Rc::new(key);
    let editor = Editor::new(&key);
    let (dirty, busy) = (editor.dirty, editor.busy);
    page_with(
        "Chi tiết lệnh",
        move || {
            let (symbol, position) = from_row(row, |r| (r.trade.symbol.clone(), r.trade.key.position_id));
            format!("{symbol} · {position}")
        },
        action_when("Lưu", Variant::Primary, move || dirty.get() && !busy.get(), editor.save()),
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
                        .child(planned(PLANNED))
                        .child(screenshots_card(TradeKey::clone(&key)))
                        .child(deals_card(row)),
                )
                .child(view().w(px(460.0)).child(journal_card(
                    editor.fields,
                    editor.status(),
                    editor.step_button(&key, "‹ Lệnh trước", 1),
                    editor.step_button(&key, "Lệnh sau ›", -1),
                ))),
        ),
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
                    let origin = if r.trade.manual { "nhập tay" } else { "EA / báo cáo" };
                    format!("{} · nguồn: {origin} · gộp {} deal", r.account, r.deals.len())
                })
            })
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
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
                        if r.trade.is_closed() {
                            format::money(r.trade.net(), Sign::Always, &r.currency)
                        } else {
                            "—".to_owned()
                        }
                    })
                }))
                .child(pnl_text(move || {
                    from_row(row, |r| if r.trade.is_closed() { format::r_text(r.r) } else { String::new() })
                })),
        )
}

fn facts_grid(row: Memo<Option<TradeRow>>) -> impl IntoElement {
    let money = move |pick: fn(&TradeRow) -> f64| {
        move || from_row(row, |r| format::money(pick(r), Sign::OnlyNegative, &r.currency))
    };
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(fact("Vào lệnh", move || from_row(row, |r| format::full_stamp(r.opened()))))
        .child(fact("Đóng lệnh", move || {
            from_row(row, |r| r.closed().map_or_else(|| "Đang mở".to_owned(), format::full_stamp))
        }))
        .child(fact("Thời gian giữ", move || {
            from_row(row, |r| r.trade.duration_secs().map_or_else(|| "—".to_owned(), format_duration))
        }))
        .child(fact("Khối lượng", move || from_row(row, |r| format!("{} lot", format::lots(r.trade.volume)))))
        .child(fact("Giá vào", move || from_row(row, |r| format::price(r.trade.open_price))))
        .child(fact("Giá ra TB", move || {
            from_row(row, |r| r.trade.close_price.map_or_else(|| "—".to_owned(), format::price))
        }))
        .child(fact("Stop loss", move || from_row(row, |r| format::level(r.trade.sl))))
        .child(fact("Take profit", move || from_row(row, |r| format::level(r.trade.tp))))
        .child(fact("Risk tại SL", move || {
            from_row(row, |r| {
                r.trade.risk.map_or_else(|| "—".to_owned(), |risk| format::money(risk, Sign::OnlyNegative, &r.currency))
            })
        }))
        .child(fact("Commission", money(|r| r.trade.commission + r.trade.fee)))
        .child(fact("Swap", money(|r| r.trade.swap)))
        .child(fact("Ticket position", move || {
            from_row(row, |r| if r.trade.manual { "nhập tay".to_owned() } else { r.trade.key.position_id.to_string() })
        }))
}

/// The label a screenshot's origin goes by.
fn source_label(source: Source) -> &'static str {
    match source {
        Source::Upload => "Thêm tay",
        Source::EaEntry => "EA · lúc vào",
        Source::EaExit => "EA · lúc ra",
    }
}

fn screenshots_card(key: TradeKey) -> impl IntoElement {
    let store = store();
    let shots = computed(move || store.snapshot().journal(&key).screenshots);
    card(
        "Ảnh chart",
        view().gap_2().child(dynamic(move || {
            let list = shots.get();
            if list.is_empty() {
                return empty_state(
                    "Chưa có ảnh chart",
                    "EA tự chụp khi mở và đóng lệnh; thêm ảnh bằng tay có ở bản tới.",
                )
                .into_element();
            }
            view()
                .gap_2()
                .children(list.into_iter().map(|shot| {
                    let name = if shot.caption.trim().is_empty() { shot.path.clone() } else { shot.caption.clone() };
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(text(name).text_sm().flex_1().number_of_lines(1))
                        .when_some(shot.timeframe.clone(), |v, tf| v.child(chip(tf.to_uppercase(), Tone::Neutral)))
                        .child(chip(source_label(shot.source), Tone::Primary))
                }))
                .into_element()
        })),
    )
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
                        let at = r.local(d.time_msc).format("%d/%m %H:%M:%S").to_string();
                        let profit = match d.entry {
                            DealEntry::In => "—".to_owned(),
                            _ => format::money(d.profit, Sign::Always, &r.currency),
                        };
                        (at, entry_label(d.entry), d.kind, format::lots(d.volume), format::price(d.price), profit)
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

/// The catalog options still in use, as `(key, label)`.
fn options_of(items: &[CatalogItem]) -> Vec<(String, String)> {
    items.iter().filter(|i| !i.archived).map(|i| (i.key.clone(), i.label.clone())).collect()
}

/// Choices of a single-value field: nothing chosen first, then the catalog's options.
fn emotion_choices(items: &[CatalogItem]) -> Vec<Choice> {
    let mut choices = vec![Choice::new("", "Chưa chọn")];
    choices.extend(items.iter().filter(|i| !i.archived).map(|i| Choice::new(i.key.clone(), i.label.clone())));
    choices
}

fn journal_card(
    fields: JournalFields,
    status: impl Fn() -> String + 'static,
    older: impl IntoElement,
    newer: impl IntoElement,
) -> impl IntoElement {
    card(
        "Nhật ký lệnh",
        view().gap_3().child(classification_rows(fields)).child(assessment_rows(fields)).child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(text(status).text_xs().text_color(move || theme().muted_foreground))
                .child(view().flex_row().gap_2().child(older).child(newer)),
        ),
    )
}

/// What the trade was: its playbook setup, its setup name, tags and mistakes.
fn classification_rows(fields: JournalFields) -> impl IntoElement {
    let store = store();
    let JournalFields { playbook, setup, tags, mistakes, .. } = fields;
    // Memos, so the pickers are made again only when the trader's option lists change and
    // not at every reload the bridge causes.
    let tag_options = computed(move || options_of(&store.snapshot().catalogs.tags));
    let mistake_options = computed(move || options_of(&store.snapshot().catalogs.mistakes));
    let playbook_choices = computed(move || {
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
        let mut choices = vec![Choice::new("", "Chưa gắn")];
        choices.extend(named);
        choices
    });
    view()
        .gap_3()
        .child(dynamic(move || {
            // Playbook setups, once the trader has made some (v0.2).
            let choices = playbook_choices.get();
            (!choices.is_empty()).then(|| {
                property_row(
                    "Playbook",
                    select_value(
                        "Playbook",
                        move || choices.clone(),
                        move || SharedString::from(playbook.get().map_or_default(|id| id.to_string())),
                        move |value| playbook.set(value.parse().ok()),
                    ),
                )
            })
        }))
        .child(property_row(
            "Setup",
            text_field(
                "Setup",
                None,
                "Tên setup, ví dụ Breakout phiên Á",
                move || setup.get(),
                move |text| {
                    setup.set(text.to_owned());
                },
            ),
        ))
        .child(property_row(
            "Tag",
            tag_picker(
                "Tag",
                move || tag_options.get(),
                move || tags.get(),
                move |key, on| tags.update(|t| *t = if on { added(t, key) } else { removed(t, key) }),
            ),
        ))
        .child(property_row(
            "Lỗi",
            tag_picker(
                "Lỗi",
                move || mistake_options.get(),
                move || mistakes.get(),
                move |key, on| mistakes.update(|m| *m = if on { added(m, key) } else { removed(m, key) }),
            ),
        ))
}

/// How it went: the execution grade, the feelings around it and the notes.
fn assessment_rows(fields: JournalFields) -> impl IntoElement {
    let store = store();
    let JournalFields { before, after, grade, notes, .. } = fields;
    let emotion_options = computed(move || emotion_choices(&store.snapshot().catalogs.emotions));
    view()
        .gap_3()
        .child(property_row(
            "Điểm thực thi",
            rating("Điểm thực thi", move || grade.get(), move |stars| grade.set(stars)),
        ))
        .child(property_row(
            "Cảm xúc trước lệnh",
            select_value(
                "Cảm xúc trước lệnh",
                move || emotion_options.get(),
                move || SharedString::from(before.get()),
                move |value| before.set(value.to_string()),
            ),
        ))
        .child(property_row(
            "Cảm xúc sau lệnh",
            select_value(
                "Cảm xúc sau lệnh",
                move || emotion_options.get(),
                move || SharedString::from(after.get()),
                move |value| after.set(value.to_string()),
            ),
        ))
        .child(view().gap_2().child(text("Ghi chú").text_sm().text_color(move || theme().muted_foreground)).child(
            textarea(
                "Ghi chú",
                "Vì sao vào, quản lý lệnh ra sao, rút ra điều gì…",
                6,
                move || notes.get(),
                move |text| notes.set(text.to_owned()),
            ),
        ))
}
