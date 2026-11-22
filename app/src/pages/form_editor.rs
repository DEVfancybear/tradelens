//! Tuỳ biến form: the forms of the journal shaped like a Notion database. One tab per form
//! (trade journal, entry plan, day, missed trade, review), a table of its properties to reorder,
//! rename, hide or show, a panel to edit the property picked (name, type, choices, flags) and
//! a way to add one or restore one that was deleted. Every change is saved at once; a new type
//! or a renamed choice is carried into the entries already written.
//! Design: `docs/design/screens/form-editor.html`.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Db, DbError,
    catalog::{FieldDef, FieldKind, FieldScope, Requirement},
    db::KindImpact,
    form::{BuiltinField, FieldRef, FormLayout, Property},
};

use crate::{
    nav,
    store::{Reading, Writer, query, writer},
    ui::{
        chips::{Tone, chip},
        controls::{Choice, Variant, action, action_icon, commit_field, confirm, icon_action, select_value},
        forms::{NumberRange, number_field, switch_row},
        icons::Icon,
        parts::{empty_state, page_with},
        properties::{KINDS, builtin_glyph, change_kind, complete, kind_glyph, kind_hint, kind_name, new_def},
        reorder::{Step, drag_handle, list_drop_index, stepped},
        tab_bar::tab_bar,
        theme as tokens,
    },
};

/// The forms the editor has a tab for, in order.
const SCOPES: &[FieldScope] =
    &[FieldScope::Trade, FieldScope::EntryPlan, FieldScope::Day, FieldScope::MissedTrade, FieldScope::Review];

/// The tab's name.
fn scope_label(scope: FieldScope) -> &'static str {
    match scope {
        FieldScope::Trade => "Nhật ký lệnh",
        FieldScope::EntryPlan => "Kế hoạch lúc vào",
        FieldScope::Day => "Nhật ký ngày",
        FieldScope::MissedTrade => "Lệnh bỏ lỡ",
        FieldScope::Review => "Tổng kết tuần/tháng",
    }
}

/// Height of a row of the table before it is laid out, in points (the drop target of a drag).
const ROW_HEIGHT: f32 = 48.0;
/// Width of the panel on the right.
const PANEL_WIDTH: f32 = 380.0;

/// The page, open on `scope`.
pub(crate) fn form_editor(scope: FieldScope) -> impl IntoElement {
    let tab = signal(scope);
    page_with(
        "Tuỳ biến form nhập liệu",
        "Như thuộc tính trong Notion: đổi tên, đổi thứ tự, ẩn, thêm, sửa, đổi kiểu, xoá",
        view().flex_row().items_center().gap_3().child(action("Xong", Variant::Primary, nav::close_form_editor)),
        view().gap_4().child(tab_bar("Form", SCOPES, scope_label, move || tab.get(), move |next| tab.set(next))).child(
            dynamic(move || {
                let scope = tab.get();
                untracked(|| editor(scope))
            }),
        ),
    )
}

/// What the editor reads of one form.
#[derive(Debug, Clone)]
struct FormData {
    layout: FormLayout,
    /// Every definition of the form, deleted ones included.
    defs: Vec<FieldDef>,
}

fn load(db: &Db, scope: FieldScope) -> Result<FormData, DbError> {
    Ok(FormData { layout: db.form_layout(scope)?, defs: db.field_defs(scope)? })
}

/// The rows of the table: every property of the form, hidden ones too, in the form's order.
fn rows_of(layout: &FormLayout, defs: &[FieldDef]) -> Vec<Property> {
    complete(layout, defs).properties(defs)
}

/// The layout after the row `from` is dragged to where row `to` is (hidden rows count);
/// `None` when nothing moves.
fn moved(layout: &FormLayout, defs: &[FieldDef], from: usize, to: usize) -> Option<FormLayout> {
    if from == to {
        return None;
    }
    let mut full = complete(layout, defs);
    let rows = full.properties(defs);
    let (moving, target) = (rows.get(from)?.field.clone(), rows.get(to)?.field.clone());
    let at = full.items.iter().position(|i| i.field == target)?;
    full.move_to(&moving, at).ok()?;
    Some(full)
}

/// A few words under a property's name: its type and choices.
fn hint_of(property: &Property, defs: &[FieldDef], scope: FieldScope) -> String {
    match &property.field {
        FieldRef::Builtin(builtin) => builtin_hint(*builtin, scope),
        FieldRef::Custom(key) => {
            let Some(def) = defs.iter().find(|d| d.key == *key) else { return String::new() };
            match def.kind {
                FieldKind::Select | FieldKind::MultiSelect if !def.options.is_empty() => {
                    format!("{} · {}", kind_name(def.kind), def.options.join(", "))
                }
                FieldKind::Scale => format!("{} · 1–{}", kind_name(def.kind), def.scale_max()),
                kind => kind_name(kind).to_owned(),
            }
        }
    }
}

fn builtin_hint(builtin: BuiltinField, _scope: FieldScope) -> String {
    match (builtin.catalog(), builtin) {
        (Some(catalog), _) => format!("Danh mục {}", catalog.label().to_lowercase()),
        (None, BuiltinField::Playbook) => "Liên kết Playbook".to_owned(),
        (None, BuiltinField::ExecutionRating) => "1–5 sao".to_owned(),
        (None, BuiltinField::Checklist) => "Checklist của tài khoản".to_owned(),
        (None, BuiltinField::Screenshots) => "Thư viện ảnh".to_owned(),
        (None, _) => String::new(),
    }
}

/// The properties of the trader's that were deleted, which can be restored.
fn deleted(defs: &[FieldDef]) -> Vec<FieldDef> {
    defs.iter().filter(|d| d.archived).cloned().collect()
}

/// A summary line over the table.
fn summary_of(rows: &[Property]) -> String {
    let hidden = rows.iter().filter(|p| p.hidden).count();
    if hidden == 0 {
        format!("{} thuộc tính · kéo ⋮⋮ để đổi thứ tự, bấm tên để sửa", rows.len())
    } else {
        format!("{} thuộc tính · {hidden} đang ẩn · kéo ⋮⋮ để đổi thứ tự, bấm tên để sửa", rows.len())
    }
}

/// The signals and the writer every edit of one form goes through.
#[derive(Clone, Copy)]
struct Ops {
    scope: FieldScope,
    layout: Signal<FormLayout>,
    defs: Signal<Vec<FieldDef>>,
    picked: Signal<Option<FieldRef>>,
    notice: Signal<Option<String>>,
    write: Writer,
    /// The editor's own owner: a write started from a popup or a dialog must not end with it.
    owner: Owner,
}

impl Ops {
    fn report(self) -> impl Fn(Result<(), String>) + Copy + 'static {
        move |result| {
            if let Err(error) = result {
                self.notice.set(Some(error));
            }
        }
    }

    /// Shows and saves a new arrangement.
    fn arrange(self, next: FormLayout) {
        self.layout.set(next.clone());
        self.owner.run(|| self.write.run(move |db| db.save_form_layout(&next), self.report()));
    }

    /// The layout with every property in it, after `edit`; saved when the edit went through.
    fn edit_layout(self, edit: impl FnOnce(&mut FormLayout) -> bool) {
        let mut next = complete(&self.layout.get_untracked(), &self.defs.get_untracked());
        if edit(&mut next) {
            self.arrange(next);
        }
    }

    /// Shows and saves a definition, carrying a new type or a renamed choice into every entry.
    fn define(self, def: FieldDef) {
        self.defs.update(|all| match all.iter_mut().find(|d| d.key == def.key) {
            Some(known) => *known = FieldDef { id: known.id, ..def.clone() },
            None => all.push(def.clone()),
        });
        self.owner.run(|| self.write.run(move |db| db.apply_field_def(&def).map(|_| ()), self.report()));
    }

    fn def(self, key: &str) -> Option<FieldDef> {
        self.defs.with_untracked(|all| all.iter().find(|d| d.key == key).cloned())
    }

    fn edit_def(self, key: &str, edit: impl FnOnce(&mut FieldDef)) {
        if let Some(mut def) = self.def(key) {
            edit(&mut def);
            self.define(def);
        }
    }

    fn rename(self, field: &FieldRef, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match field {
            FieldRef::Builtin(builtin) => self.edit_layout(|layout| layout.rename_builtin(*builtin, name).is_ok()),
            FieldRef::Custom(key) => self.edit_def(key, |def| name.clone_into(&mut def.label)),
        }
    }

    fn hide(self, field: &FieldRef) {
        self.edit_layout(|layout| layout.hide(field).is_ok());
    }

    fn show(self, field: &FieldRef) {
        self.edit_layout(|layout| layout.show(field).is_ok());
    }

    /// Adds a property of `kind` at the end of the form and picks it, to be named.
    fn add(self, kind: FieldKind) {
        let def = new_def(self.scope, kind, &self.defs.get_untracked());
        let field = FieldRef::Custom(def.key.clone());
        self.define(def.clone());
        self.edit_layout(|layout| {
            layout.add_custom(&def.key);
            true
        });
        self.picked.set(Some(field));
    }

    /// Takes a deleted property back into the form, with the values it still has.
    fn restore(self, key: &str) {
        self.edit_def(key, |def| def.archived = false);
        self.edit_layout(|layout| {
            layout.add_custom(key);
            true
        });
        self.picked.set(Some(FieldRef::Custom(key.to_owned())));
    }

    /// Deletes a property of the trader's: its definition is archived and it leaves the form;
    /// the values written stay and come back with it.
    fn delete(self, key: &str) {
        self.edit_def(key, |def| def.archived = true);
        let field = FieldRef::Custom(key.to_owned());
        self.edit_layout(|layout| layout.remove(&field).is_ok());
        self.picked.set(None);
    }

    /// Gives the property another type: the trader is told what would be lost first, counted
    /// over every entry of the form.
    fn change_kind(self, key: &str, to: FieldKind) {
        let Some(def) = self.def(key) else { return };
        if def.kind == to {
            return;
        }
        let (scope, owned) = (self.scope, key.to_owned());
        let apply = move |ops: Ops| {
            if let Some(current) = ops.def(&owned) {
                ops.define(change_kind(&current, None, to).def);
            }
        };
        let notice = self.notice;
        let key = key.to_owned();
        // Under the editor's owner: this runs from the drop-down's popup, which closes at once.
        let write = self.write;
        self.owner.run(|| {
            write.run(
                move |db| db.kind_change_impact(scope, &key, to),
                move |result| match result {
                    Ok(KindImpact { lost, .. }) if lost > 0 => {
                        spawn(async move {
                            let go = confirm(
                                "Đổi kiểu sẽ mất giá trị đã ghi?",
                                format!(
                                    "{lost} giá trị đã ghi không chuyển được sang kiểu “{}” và sẽ bị bỏ trống.",
                                    kind_name(to)
                                ),
                                "Đổi kiểu",
                            )
                            .await;
                            if go {
                                apply(self);
                            }
                        });
                    }
                    Ok(_) => apply(self),
                    Err(error) => notice.set(Some(error)),
                },
            );
        });
    }
}

fn editor(scope: FieldScope) -> impl IntoElement {
    let data: Reading<FormData> = query(move |db, _| load(db, scope));
    // The form as the trader arranged it, in signals so that an edit shows at once; what the
    // database says replaces them whenever it is read again.
    let layout = signal(FormLayout::default_for(scope));
    let defs = signal(Vec::<FieldDef>::new());
    effect(move || {
        if let Some(read) = data.value() {
            layout.set(read.layout.clone());
            defs.set(read.defs.clone());
        }
    });
    let ops = Ops {
        scope,
        layout,
        defs,
        picked: signal(None),
        notice: signal(None),
        write: writer(),
        owner: Owner::current(),
    };
    let rows = computed(move || rows_of(&layout.get(), &defs.get()));
    view()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap_6()
        .child(view().flex_1().min_w(px(480.0)).gap_6().child(table_card(ops, rows)).child(shared_options_note()))
        .child(view().w(px(PANEL_WIDTH)).gap_6().child(edit_panel(ops, rows)).child(add_panel(ops)))
}

fn table_card(ops: Ops, rows: Memo<Vec<Property>>) -> impl IntoElement {
    let heights = stored(std::collections::HashMap::<String, f32>::new());
    let restoring = signal(false);
    let defs = ops.defs;
    let archived = computed(move || deleted(&defs.get()));
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text(move || rows.with(|r| summary_of(r))).text_xs().text_color(move || theme().muted_foreground))
        .child(dynamic(move || {
            let empty = rows.with(Vec::is_empty);
            untracked(|| {
                if empty {
                    return empty_state("Form này chưa có thuộc tính", "Thêm một thuộc tính ở khung bên phải.")
                        .into_element();
                }
                view()
                    .child(each(
                        rows,
                        |p: &Property| key_of(&p.field),
                        move |row, index| table_row(ops, rows, row, index, heights),
                    ))
                    .into_element()
            })
        }))
        .child(dynamic(move || ops.notice.get().map(|error| text(error).text_sm().text_color(tokens::loss()))))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(dynamic(move || {
                    let n = archived.with(Vec::len);
                    (n > 0).then(|| {
                        untracked(|| {
                            action_icon(
                                Icon::Refresh,
                                format!("Khôi phục thuộc tính đã xoá ({n})"),
                                Variant::Ghost,
                                move || restoring.update(|r| *r = !*r),
                            )
                        })
                    })
                }))
                .child(action("Đặt lại mặc định", Variant::Ghost, move || {
                    spawn(async move {
                        let go = confirm(
                            "Đặt lại thứ tự và tên mặc định?",
                            "Thứ tự, tên đã đổi và thuộc tính đang ẩn của form này về như ban đầu. \
                             Thuộc tính riêng và dữ liệu đã ghi được giữ nguyên.",
                            "Đặt lại",
                        )
                        .await;
                        if go {
                            ops.arrange(FormLayout::default_for(ops.scope));
                        }
                    });
                })),
        )
        .child(dynamic(move || restoring.get().then(|| untracked(|| deleted_list(ops, archived)))))
}

/// The deleted properties, each with a button to bring it back.
fn deleted_list(ops: Ops, archived: Memo<Vec<FieldDef>>) -> impl IntoElement {
    view().gap_1().child(dynamic(move || {
        let all = archived.get();
        untracked(|| {
            view().gap_1().children(all.into_iter().map(|def| {
                let key = def.key.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        text(kind_glyph(def.kind)).w(px(24.0)).text_sm().text_color(move || theme().muted_foreground),
                    )
                    .child(text(def.label.clone()).flex_1().min_w(px(0.0)).text_sm())
                    .child(action(format!("Khôi phục {}", def.label), Variant::Secondary, move || ops.restore(&key)))
            }))
        })
    }))
}

fn shared_options_note() -> impl IntoElement {
    view()
        .p_5()
        .gap_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text("Danh mục lựa chọn dùng chung").text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(
            text(
                "Tag, lỗi, cảm xúc, lý do thoát, hạng setup, trạng thái thị trường, khung giờ, lý do không vào lệnh: \
                 thêm, đổi tên, lưu trữ trong Cài đặt → Danh mục. Xoá một lựa chọn chỉ ẩn khỏi danh sách chọn, \
                 nhật ký cũ giữ nguyên.",
            )
            .text_sm()
            .text_color(move || theme().muted_foreground),
        )
}

/// The key a row is identified by, so it keeps its state while the others change.
fn key_of(field: &FieldRef) -> String {
    match field {
        FieldRef::Builtin(builtin) => format!("builtin:{builtin:?}"),
        FieldRef::Custom(key) => format!("custom:{key}"),
    }
}

/// One row: grip, glyph, name (press it to edit the property), hint, badge and eye.
fn table_row(
    ops: Ops,
    rows: Memo<Vec<Property>>,
    row: ReadSignal<Property>,
    index: ReadSignal<usize>,
    heights: Stored<std::collections::HashMap<String, f32>>,
) -> impl IntoElement {
    let measured = key_of(&row.with_untracked(|p| p.field.clone()));
    let scope = ops.scope;
    let move_to = move |to: usize| {
        let from = index.get_untracked();
        let next = moved(&ops.layout.get_untracked(), &ops.defs.get_untracked(), from, to);
        if let Some(next) = next {
            ops.arrange(next);
        }
    };
    let by_drag = move |_dx: f32, dy: f32| {
        let heights_now: Vec<f32> = rows.with(|all| {
            all.iter().map(|p| heights.with(|h| h.get(&key_of(&p.field)).copied()).unwrap_or(ROW_HEIGHT)).collect()
        });
        move_to(list_drop_index(index.get_untracked(), dy, &heights_now));
    };
    let by_key = move |step: Step| move_to(stepped(index.get_untracked(), step, rows.with(Vec::len)));
    let selected = move || ops.picked.with(|p| *p == Some(row.with(|r| r.field.clone())));
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_2()
        .py_2()
        .border_t_1()
        .border_color(move || theme().border)
        .style_when(selected, |s| s.bg(move || theme().accent))
        .on_layout(move |bounds: &Bounds<Points>| {
            heights.with(|h| h.insert(measured.clone(), bounds.size.height.0));
        })
        .child(drag_handle(move || format!("Kéo để đổi thứ tự {}", row.with(|p| p.label.clone())), by_drag, by_key))
        .child(text(move || row.with(glyph_of)).w(px(20.0)).text_sm().text_color(move || theme().muted_foreground))
        .child(
            view()
                .role(AccessibilityRole::Button)
                .label(move || format!("Sửa thuộc tính {}", row.with(|p| p.label.clone())))
                .focusable(true)
                .on_press(move || ops.picked.set(Some(row.with_untracked(|p| p.field.clone()))))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(
                    text(move || row.with(|p| p.label.clone())).text_sm().font_weight(FontWeight::MEDIUM).text_color(
                        move || {
                            if row.with(|p| p.hidden) { theme().muted_foreground } else { theme().foreground }
                        },
                    ),
                )
                .child(
                    text(move || row.with(|p| hint_of(p, &ops.defs.get(), scope)))
                        .text_xs()
                        .number_of_lines(1)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(dynamic(move || {
            let custom = row.with(|p| matches!(p.field, FieldRef::Custom(_)));
            chip(if custom { "riêng" } else { "có sẵn" }, if custom { Tone::Primary } else { Tone::Neutral })
        }))
        .child(dynamic(move || {
            let (hidden, label) = row.with(|p| (p.hidden, p.label.clone()));
            let field = row.with_untracked(|p| p.field.clone());
            untracked(|| {
                icon_action(
                    if hidden { Icon::EyeOff } else { Icon::Eye },
                    if hidden { format!("Hiện {label}") } else { format!("Ẩn {label}") },
                    Variant::Ghost,
                    move || if hidden { ops.show(&field) } else { ops.hide(&field) },
                )
            })
        }))
}

/// The glyph before a property's name.
fn glyph_of(property: &Property) -> &'static str {
    match (&property.field, property.kind) {
        (_, Some(kind)) => kind_glyph(kind),
        (FieldRef::Builtin(builtin), None) => builtin_glyph(*builtin),
        (FieldRef::Custom(_), None) => "•",
    }
}

fn panel_frame(title: impl IntoReactive<SharedString>, body: impl IntoElement) -> impl IntoElement {
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text(title).text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(body)
}

/// The panel that edits the property picked in the table.
fn edit_panel(ops: Ops, rows: Memo<Vec<Property>>) -> impl IntoElement {
    let current = computed(move || {
        let field = ops.picked.get()?;
        rows.with(|all| all.iter().find(|p| p.field == field).cloned())
    });
    let title = move || match current.get() {
        Some(p) => format!("Sửa thuộc tính · {}", p.label),
        None => "Sửa thuộc tính".to_owned(),
    };
    panel_frame(
        title,
        view().gap_3().child(dynamic(move || {
            // Rebuilt when another property is picked, not on every keystroke of its name.
            let which = current.with(|p| p.as_ref().map(|p| p.field.clone()));
            untracked(|| match which {
                None => text("Bấm tên một thuộc tính ở bảng bên trái để sửa.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground)
                    .into_element(),
                Some(field) => property_panel(ops, current, &field).into_element(),
            })
        })),
    )
}

fn property_panel(ops: Ops, current: Memo<Option<Property>>, field: &FieldRef) -> impl IntoElement {
    let name = move || current.with(|p| p.as_ref().map_or_default(|p| p.label.clone()));
    let rename_field = field.clone();
    let key = match field {
        FieldRef::Custom(key) => Some(key.clone()),
        FieldRef::Builtin(_) => None,
    };
    let hide_field = field.clone();
    view()
        .gap_3()
        .child(view().gap_1().child(text("Tên").text_xs().text_color(move || theme().muted_foreground)).child(
            commit_field("Tên thuộc tính", "Tên thuộc tính", name, move |typed| {
                ops.rename(&rename_field, &typed);
            }),
        ))
        .child(match key {
            Some(key) => custom_panel(ops, key).into_element(),
            None => text(
                "Thuộc tính có sẵn gắn với một mục của nhật ký: chỉ đổi tên hoặc ẩn được. Danh sách lựa chọn của nó \
                 sửa ở Cài đặt → Danh mục.",
            )
            .text_sm()
            .text_color(move || theme().muted_foreground)
            .into_element(),
        })
        .child(dynamic(move || {
            let hidden = current.with(|p| p.as_ref().is_some_and(|p| p.hidden));
            let field = hide_field.clone();
            untracked(|| {
                action_icon(
                    if hidden { Icon::Eye } else { Icon::EyeOff },
                    if hidden { "Hiện trong form" } else { "Ẩn khỏi form" },
                    Variant::Secondary,
                    move || if hidden { ops.show(&field) } else { ops.hide(&field) },
                )
            })
        }))
}

/// What only a property of the trader's has: type, choices, flags and delete.
fn custom_panel(ops: Ops, key: String) -> impl IntoElement {
    // A `Copy` handle on the key, for the many closures below.
    let key = stored(key);
    let def = move || ops.defs.with(|all| all.iter().find(|d| key.with(|k| d.key == *k)).cloned());
    let kind_of = move || def().map_or(FieldKind::Text, |d| d.kind);
    let flag = move |pick: fn(&FieldDef) -> bool| def().is_some_and(|d| pick(&d));
    let key_text = move || key.with(|k| k.clone());
    view()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(text("Kiểu").text_sm().text_color(move || theme().muted_foreground))
                .child(select_value(
                    "Kiểu",
                    || {
                        KINDS
                            .iter()
                            .map(|&k| Choice::new(kind_id(k), format!("{} {}", kind_glyph(k), kind_name(k))))
                            .collect()
                    },
                    move || SharedString::from(kind_id(kind_of())),
                    move |chosen| {
                        if let Some(kind) = KINDS.iter().copied().find(|&k| kind_id(k) == &*chosen) {
                            ops.change_kind(&key_text(), kind);
                        }
                    },
                )),
        )
        .child(dynamic(move || {
            matches!(kind_of(), FieldKind::Select | FieldKind::MultiSelect)
                .then(|| untracked(|| options_editor(ops, key_text())))
        }))
        .child(dynamic(move || {
            (kind_of() == FieldKind::Scale).then(|| {
                untracked(|| {
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(text("Điểm tối đa").text_sm().text_color(move || theme().muted_foreground))
                        .child(number_field(
                            "Điểm tối đa",
                            None,
                            NumberRange::between(2.0, 10.0),
                            move || def().map(|d| f64::from(d.scale_max())),
                            move |n| {
                                if let Some(max) = n.map(f64::round).and_then(scale_max) {
                                    ops.edit_def(&key_text(), |d| d.max = Some(max));
                                }
                            },
                        ))
                })
            })
        }))
        .child(switch_row(
            "Nên điền khi lưu",
            Some("Nhắc nếu để trống; không chặn việc lưu"),
            move || flag(|d| d.required == Requirement::Expected),
            move |on| {
                ops.edit_def(&key_text(), |d| {
                    d.required = if on { Requirement::Expected } else { Requirement::Optional };
                });
            },
        ))
        .child(switch_row(
            "Đưa vào Phân tích",
            Some("Thành một cột trong bảng Lệnh và một nhóm trong Phân tích"),
            move || flag(|d| d.analysed),
            move |on| ops.edit_def(&key_text(), |d| d.analysed = on),
        ))
        .child(kind_note())
        .child(action_icon(Icon::Trash, "Xoá thuộc tính", Variant::Secondary, move || {
            ask_to_delete(ops, def().map_or_default(|d| d.label), key_text());
        }))
}

/// What changing a property's type does to the values written.
fn kind_note() -> impl IntoElement {
    view()
        .p_3()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(text("Đổi kiểu dữ liệu").text_xs().font_weight(FontWeight::SEMIBOLD))
        .child(
            text(
                "Chọn một → Chọn nhiều: giữ nguyên giá trị. Văn bản → Chọn một: tạo lựa chọn từ chữ đã ghi. \
                 Số → Thang điểm: làm tròn. Giá trị không chuyển được sẽ được báo trước khi đổi.",
            )
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
}

/// Asks before deleting the property `key` (named `name`): its values stay, and it can be restored.
fn ask_to_delete(ops: Ops, name: String, key: String) {
    spawn(async move {
        let go = confirm(
            format!("Xoá thuộc tính “{name}”?"),
            "Giá trị đã ghi ở các lệnh cũ được giữ lại; thuộc tính chỉ biến khỏi form và khôi phục được ở đây.",
            "Xoá thuộc tính",
        )
        .await;
        if go {
            ops.delete(&key);
        }
    });
}

/// The top of a scale from a number typed in the field.
fn scale_max(n: f64) -> Option<u8> {
    (2_u8..=10).find(|top| (f64::from(*top) - n).abs() < f64::EPSILON)
}

/// The stable text a kind is stored under in a drop-down.
fn kind_id(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::LongText => "long_text",
        FieldKind::Number => "number",
        FieldKind::Select => "select",
        FieldKind::MultiSelect => "multi_select",
        FieldKind::Checkbox => "checkbox",
        FieldKind::Scale => "scale",
        FieldKind::Date => "date",
        FieldKind::Url => "url",
    }
}

/// The choices of a select or multi-select: rename (entries follow), remove, add.
fn options_editor(ops: Ops, key: String) -> impl IntoElement {
    let added = signal(0_u32);
    let options = {
        let key = key.clone();
        move || ops.defs.with(|all| all.iter().find(|d| d.key == key).map_or_default(|d| d.options.clone()))
    };
    let add_key = key.clone();
    view()
        .gap_1()
        .child(
            text("Lựa chọn (đổi tên ở đây thì mọi lệnh cũ đổi theo)")
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(dynamic(move || {
            let all = options();
            let key = key.clone();
            untracked(|| view().gap_1().children(all.into_iter().map(|option| option_row(ops, key.clone(), option))))
        }))
        .child(commit_field(
            "Thêm lựa chọn",
            "+ Thêm lựa chọn",
            move || {
                let _ = added.get();
                String::new()
            },
            move |option| {
                let option = option.trim().to_owned();
                if !option.is_empty() {
                    ops.edit_def(&add_key, |d| {
                        if !d.options.contains(&option) {
                            d.options.push(option.clone());
                        }
                    });
                    added.update(|n| *n += 1);
                }
            },
        ))
}

fn option_row(ops: Ops, key: String, option: String) -> impl IntoElement {
    let (name, shown, removed) = (option.clone(), option.clone(), option.clone());
    let rename_key = key.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().child(commit_field(
            format!("Tên lựa chọn {name}"),
            "Tên lựa chọn",
            move || shown.clone(),
            move |new| {
                let new = new.trim().to_owned();
                let taken = ops.def(&rename_key).is_some_and(|d| d.options.contains(&new));
                if !new.is_empty() && !taken {
                    ops.edit_def(&rename_key, |d| {
                        for o in d.options.iter_mut().filter(|o| **o == option) {
                            o.clone_from(&new);
                        }
                    });
                }
            },
        )))
        .child(icon_action(Icon::Close, format!("Xoá lựa chọn {name}"), Variant::Ghost, move || {
            ops.edit_def(&key, |d| d.options.retain(|o| *o != removed));
        }))
}

/// "Thêm thuộc tính — chọn kiểu": one button per type.
fn add_panel(ops: Ops) -> impl IntoElement {
    panel_frame(
        "Thêm thuộc tính — chọn kiểu",
        view().gap_1().children(KINDS.iter().map(move |&kind| {
            view()
                .role(AccessibilityRole::Button)
                .label(format!("Thêm thuộc tính kiểu {}", kind_name(kind)))
                .focusable(true)
                .on_press(move || ops.add(kind))
                .flex_row()
                .items_center()
                .gap_3()
                .px_2()
                .h(px(36.0))
                .rounded(px(6.0))
                .hover(|s| s.bg(move || theme().accent))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(text(kind_glyph(kind)).w(px(24.0)).text_sm().text_color(move || theme().muted_foreground))
                .child(text(kind_name(kind)).text_sm())
                .child(
                    text(kind_hint(kind)).flex_1().text_xs().text_color(move || theme().muted_foreground).text_right(),
                )
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradelens_core::catalog::Requirement;

    fn def(key: &str, label: &str, kind: FieldKind, state: &str) -> FieldDef {
        FieldDef {
            id: 1,
            scope: FieldScope::Day,
            key: key.into(),
            label: label.into(),
            kind,
            options: vec!["M5".into(), "M15".into()],
            max: Some(10),
            required: Requirement::Optional,
            analysed: false,
            position: 0,
            archived: state == "archived",
        }
    }

    #[test]
    fn hidden_rows_stay_in_the_table_and_move_like_the_others() {
        let mut layout = FormLayout::default_for(FieldScope::Day);
        layout.hide(&FieldRef::Builtin(BuiltinField::DayReview)).expect("in form");
        let defs = [def("tf", "Khung", FieldKind::Select, "live")];
        let rows = rows_of(&layout, &defs);
        assert_eq!(
            rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            ["Kế hoạch ngày", "Tổng kết", "Tâm trạng", "Khung"]
        );
        assert!(rows[1].hidden, "the hidden row is listed");
        let next = moved(&layout, &defs, 3, 0).expect("the custom row moves to the top");
        let labels: Vec<String> = rows_of(&next, &defs).into_iter().map(|r| r.label).collect();
        assert_eq!(labels[0], "Khung");
        assert_eq!(moved(&layout, &defs, 2, 2), None);
        assert_eq!(moved(&layout, &defs, 9, 0), None, "a row that is not there does not move");
    }

    #[test]
    fn rows_say_what_they_are() {
        let defs = [def("tf", "Khung", FieldKind::Select, "live"), def("n", "Mức", FieldKind::Scale, "live")];
        let rows = rows_of(&FormLayout::default_for(FieldScope::Day), &defs);
        let hint = |label: &str| {
            let row = rows.iter().find(|r| r.label == label).expect("a row");
            hint_of(row, &defs, FieldScope::Day)
        };
        assert_eq!(hint("Khung"), "Chọn một · M5, M15");
        assert_eq!(hint("Mức"), "Thang điểm · 1–10");
        assert_eq!(hint("Tâm trạng"), "Danh mục cảm xúc");
        assert_eq!(summary_of(&rows), "5 thuộc tính · kéo ⋮⋮ để đổi thứ tự, bấm tên để sửa");
    }

    #[test]
    fn deleted_properties_are_the_archived_ones() {
        let defs = [def("a", "A", FieldKind::Text, "live"), def("b", "B", FieldKind::Text, "archived")];
        assert_eq!(deleted(&defs).iter().map(|d| d.key.as_str()).collect::<Vec<_>>(), ["b"]);
        // A deleted property is not a row until it is restored.
        let rows = rows_of(&FormLayout::default_for(FieldScope::Day), &defs);
        assert!(rows.iter().all(|r| r.label != "B"));
    }

    #[test]
    fn a_scale_tops_between_two_and_ten() {
        assert_eq!(scale_max(5.0), Some(5));
        assert_eq!(scale_max(1.0), None);
        assert_eq!(scale_max(11.0), None);
    }
}
