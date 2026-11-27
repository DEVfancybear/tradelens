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
    store::{Reading, Store, Writer, query, store, writer},
    ui::{
        chips::{Tone, chip},
        controls::{
            Choice, Variant, action, action_icon, add_field, commit_field, confirm_then, icon_action, select_value,
        },
        forms::{NumberRange, number_field, switch_row},
        icons::Icon,
        parts::{empty_state, error_line, page_with},
        properties::{KINDS, builtin_glyph, change_kind, complete, kind_glyph, kind_hint, kind_name, new_def},
        reorder::{Step, drag_handle, list_drop_index, stepped},
        tab_bar::tab_bar,
    },
};

/// The forms the editor has a tab for, in order.
const SCOPES: &[FieldScope] =
    &[FieldScope::Trade, FieldScope::EntryPlan, FieldScope::Day, FieldScope::MissedTrade, FieldScope::Review];

/// The tab's name.
fn scope_label(scope: FieldScope) -> String {
    match scope {
        FieldScope::Trade => t!("form_editor.scope.trade"),
        FieldScope::EntryPlan => t!("form_editor.scope.entry_plan"),
        FieldScope::Day => t!("form_editor.scope.day"),
        FieldScope::MissedTrade => t!("form_editor.scope.missed"),
        FieldScope::Review => t!("form_editor.scope.review"),
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
        t!("form_editor.title"),
        t!("form_editor.summary"),
        view().flex_row().items_center().gap_3().child(action(
            t!("form_editor.done"),
            Variant::Primary,
            nav::close_form_editor,
        )),
        view()
            .gap_4()
            .child(tab_bar(t!("form_editor.tabs"), SCOPES, scope_label, move || tab.get(), move |next| tab.set(next)))
            .child(dynamic(move || {
                let scope = tab.get();
                untracked(|| editor(scope))
            })),
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
                kind => kind_name(kind),
            }
        }
    }
}

fn builtin_hint(builtin: BuiltinField, _scope: FieldScope) -> String {
    match (builtin.catalog(), builtin) {
        (Some(catalog), _) => t!("form_editor.hint.catalog", kind = catalog.label().to_lowercase()),
        (None, BuiltinField::Playbook) => t!("form_editor.hint.playbook"),
        (None, BuiltinField::ExecutionRating) => t!("form_editor.hint.rating"),
        (None, BuiltinField::Checklist) => t!("form_editor.hint.checklist"),
        (None, BuiltinField::Screenshots) => t!("form_editor.hint.screenshots"),
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
        t!("form_editor.summary.shown", n = rows.len())
    } else {
        t!("form_editor.summary.hidden", n = rows.len(), hidden = hidden)
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
    store: Store,
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
        // Under the editor's owner: this runs from the drop-down's popup, which closes at once. What
        // would be lost is only read: a write announces a change, and the pages that read again on it
        // can end the owner this answer is waiting for.
        let store = self.store;
        self.owner.run(|| {
            store.read(
                move |db| db.kind_change_impact(scope, &key, to),
                move |result| match result {
                    Ok(KindImpact { lost, .. }) if lost > 0 => {
                        confirm_then(
                            self.owner,
                            t!("form_editor.kind.lose_title"),
                            t!("form_editor.kind.lose_body", lost = lost, kind = kind_name(to)),
                            t!("form_editor.kind.lose_confirm"),
                            move || apply(self),
                        );
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
        store: store(),
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
    // Whether there is a table to draw, not what is in it: renaming a property is not another table.
    let no_rows = computed(move || rows.with(Vec::is_empty));
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text(move || rows.with(|r| summary_of(r))).text_xs().text_color(move || theme().muted_foreground))
        .child(dynamic(move || {
            let empty = no_rows.get();
            untracked(|| {
                if empty {
                    return empty_state(t!("form_editor.empty"), t!("form_editor.empty_hint")).into_element();
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
        .child(dynamic(move || ops.notice.get().map(error_line)))
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
                                t!("form_editor.restore_deleted", n = n),
                                Variant::Ghost,
                                move || restoring.update(|r| *r = !*r),
                            )
                        })
                    })
                }))
                .child(action(t!("form_editor.reset"), Variant::Ghost, move || {
                    confirm_then(
                        ops.owner,
                        t!("form_editor.reset.title"),
                        t!("form_editor.reset.body"),
                        t!("form_editor.reset.confirm"),
                        move || ops.arrange(FormLayout::default_for(ops.scope)),
                    );
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
                    .child(action(t!("form_editor.restore", label = def.label), Variant::Secondary, move || {
                        ops.restore(&key);
                    }))
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
        .child(text(t!("form_editor.shared.title")).text_sm().font_weight(FontWeight::SEMIBOLD))
        .child(text(t!("form_editor.shared.body")).text_sm().text_color(move || theme().muted_foreground))
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
        .child(drag_handle(move || t!("form_editor.row.drag", name = row.with(|p| p.label.clone())), by_drag, by_key))
        .child(text(move || row.with(glyph_of)).w(px(20.0)).text_sm().text_color(move || theme().muted_foreground))
        .child(
            view()
                .role(AccessibilityRole::Button)
                .label(move || t!("form_editor.row.edit", label = row.with(|p| p.label.clone())))
                .focusable(true)
                .cursor_pointer()
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
            chip(
                if custom { t!("form_editor.row.custom") } else { t!("form_editor.row.builtin") },
                if custom { Tone::Primary } else { Tone::Neutral },
            )
        }))
        .child(dynamic(move || {
            let (hidden, label) = row.with(|p| (p.hidden, p.label.clone()));
            let field = row.with_untracked(|p| p.field.clone());
            untracked(|| {
                icon_action(
                    if hidden { Icon::EyeOff } else { Icon::Eye },
                    if hidden {
                        t!("form_editor.row.show", label = label)
                    } else {
                        t!("form_editor.row.hide", label = label)
                    },
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
        Some(p) => t!("form_editor.panel.title_of", label = p.label),
        None => t!("form_editor.panel.title"),
    };
    // Which property the panel is for: a memo, so that a change of the property's name or options (a
    // rename kept, a read of the database that brings another text) does not make the panel again.
    let which = computed(move || current.with(|p| p.as_ref().map(|p| p.field.clone())));
    panel_frame(
        title,
        view().gap_3().child(dynamic(move || {
            // Rebuilt when another property is picked, not on every keystroke of its name.
            let which = which.get();
            untracked(|| match which {
                None => text(t!("form_editor.panel.pick"))
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
        .child(
            view()
                .gap_1()
                .child(text(t!("form_editor.panel.name")).text_xs().text_color(move || theme().muted_foreground))
                .child(commit_field(
                    t!("form_editor.panel.name_label"),
                    t!("form_editor.panel.name_label"),
                    name,
                    move |typed| {
                        ops.rename(&rename_field, &typed);
                    },
                )),
        )
        .child(match key {
            Some(key) => custom_panel(ops, key).into_element(),
            None => text(t!("form_editor.panel.builtin_note"))
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
                    if hidden { t!("form_editor.panel.show") } else { t!("form_editor.panel.hide") },
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
    // A memo: editing the property (an option renamed, a limit typed) must not make again the editors
    // that only depend on its kind, under the hand that is typing in them.
    let kind = computed(kind_of);
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
                .child(text(t!("form_editor.panel.kind")).text_sm().text_color(move || theme().muted_foreground))
                .child(select_value(
                    t!("form_editor.panel.kind"),
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
            matches!(kind.get(), FieldKind::Select | FieldKind::MultiSelect)
                .then(|| untracked(|| options_editor(ops, key_text())))
        }))
        .child(dynamic(move || {
            (kind.get() == FieldKind::Scale).then(|| {
                untracked(|| {
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            text(t!("form_editor.panel.scale_max"))
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(number_field(
                            t!("form_editor.panel.scale_max"),
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
            t!("form_editor.panel.expected"),
            Some(t!("form_editor.panel.expected_hint")),
            move || flag(|d| d.required == Requirement::Expected),
            move |on| {
                ops.edit_def(&key_text(), |d| {
                    d.required = if on { Requirement::Expected } else { Requirement::Optional };
                });
            },
        ))
        .child(switch_row(
            t!("form_editor.panel.analysed"),
            Some(t!("form_editor.panel.analysed_hint")),
            move || flag(|d| d.analysed),
            move |on| ops.edit_def(&key_text(), |d| d.analysed = on),
        ))
        .child(kind_note())
        .child(action_icon(Icon::Trash, t!("form_editor.panel.delete"), Variant::Secondary, move || {
            ask_to_delete(ops, &def().map_or_default(|d| d.label), &key_text());
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
        .child(text(t!("form_editor.kind_note.title")).text_xs().font_weight(FontWeight::SEMIBOLD))
        .child(text(t!("form_editor.kind_note.body")).text_xs().text_color(move || theme().muted_foreground))
}

/// Asks before deleting the property `key` (named `name`): its values stay, and it can be restored.
fn ask_to_delete(ops: Ops, name: &str, key: &str) {
    let key = key.to_owned();
    // Under the editor's owner: the popover this is asked from is built again as the form changes.
    confirm_then(
        ops.owner,
        t!("form_editor.delete.title", name = name),
        t!("form_editor.delete.body"),
        t!("form_editor.delete.confirm"),
        move || ops.delete(&key),
    );
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
    let options = {
        let key = key.clone();
        computed(move || ops.defs.with(|all| all.iter().find(|d| d.key == key).map_or_default(|d| d.options.clone())))
    };
    let add_key = key.clone();
    view()
        .gap_1()
        .child(text(t!("form_editor.options.title")).text_xs().text_color(move || theme().muted_foreground))
        // A row is known by its option: renaming one makes that row again and leaves the others, so
        // the one the trader moves on to is still there when the rename of the one they leave is kept.
        .child(view().gap_1().child(each(
            options,
            |option: &String| option.clone(),
            move |row, _| option_row(ops, key.clone(), row.with_untracked(Clone::clone)),
        )))
        .child(add_field(t!("form_editor.options.add"), t!("form_editor.options.add_placeholder"), move |option| {
            let option = option.trim().to_owned();
            if !option.is_empty() {
                ops.edit_def(&add_key, |d| {
                    if !d.options.contains(&option) {
                        d.options.push(option.clone());
                    }
                });
            }
        }))
}

fn option_row(ops: Ops, key: String, option: String) -> impl IntoElement {
    let (name, shown, removed) = (option.clone(), option.clone(), option.clone());
    let rename_key = key.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().child(commit_field(
            t!("form_editor.options.name_of", name = name),
            t!("form_editor.options.name"),
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
        .child(icon_action(Icon::Close, t!("form_editor.options.remove", name = name), Variant::Ghost, move || {
            ops.edit_def(&key, |d| d.options.retain(|o| *o != removed));
        }))
}

/// "Add a property — choose its type": one button per type.
fn add_panel(ops: Ops) -> impl IntoElement {
    panel_frame(
        t!("form_editor.add.title"),
        view().gap_1().children(KINDS.iter().map(move |&kind| {
            view()
                .role(AccessibilityRole::Button)
                .label(t!("form_editor.add.of_kind", kind = kind_name(kind)))
                .focusable(true)
                .cursor_pointer()
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
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
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
        });
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
