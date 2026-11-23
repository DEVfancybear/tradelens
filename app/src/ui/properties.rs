//! The property list of a form the trader shapes like a Notion database: each row has a grip to
//! drag, the kind's glyph, the name (press it to open the property's popover) and the value's
//! editor. Rows come from a `FormLayout` and the trader's `FieldDef`s
//! (`tradelens_core::form`, `catalog`), so nothing here knows which properties exist.
//!
//! The list edits values and the layout it is given and reports every change through
//! [`PropertyEdits`]; saving them is the caller's job, and the closures that tell the list what
//! the form looks like must show the change once it is saved. Built-in properties are not drawn
//! here (each is a typed field of the journal): the caller draws them in [`PropertyList::builtin`].

use std::{collections::HashMap, rc::Rc};

use nsh_base::{PopoverEvent, popover, theme};
use nsh_core::prelude::*;
use tradelens_core::catalog::{CustomValues, FieldDef, FieldKind, FieldScope, FieldValue, Requirement};
use tradelens_core::form::{
    BuiltinField, FieldRef, FormItem, FormLayout, Property, Width, convert, options_from, rename_option,
};

use super::{
    chips::{Tone, chip},
    controls::{
        Choice, Variant, action_icon, commit_field, confirm_then, icon_action, rating_of, root_owner, select_value,
        tag_picker, text_field, textarea,
    },
    date_picker::{DatePick, date_pick, date_trigger},
    forms::{NumberRange, check_box, number_field, switch_row},
    icons::Icon,
    reorder::{Step, drag_handle, list_drop_index, stepped},
};

/// The kinds a property can have, in the order the picker lists them.
pub const KINDS: [FieldKind; 9] = [
    FieldKind::Text,
    FieldKind::LongText,
    FieldKind::Number,
    FieldKind::Select,
    FieldKind::MultiSelect,
    FieldKind::Checkbox,
    FieldKind::Scale,
    FieldKind::Date,
    FieldKind::Url,
];

/// The kind's name as the trader reads it.
pub fn kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "Văn bản",
        FieldKind::LongText => "Văn bản dài",
        FieldKind::Number => "Số",
        FieldKind::Select => "Chọn một",
        FieldKind::MultiSelect => "Chọn nhiều",
        FieldKind::Checkbox => "Checkbox",
        FieldKind::Scale => "Thang điểm",
        FieldKind::Date => "Ngày",
        FieldKind::Url => "Liên kết",
    }
}

/// A few words under the kind's name when the trader picks one.
pub fn kind_hint(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "một dòng",
        FieldKind::LongText => "nhiều dòng",
        FieldKind::Number => "giá, lot, %",
        FieldKind::Select => "có màu",
        FieldKind::MultiSelect => "như tag",
        FieldKind::Checkbox => "có / không",
        FieldKind::Scale => "1–5, 1–10",
        FieldKind::Date => "YYYY-MM-DD",
        FieldKind::Url => "mở bằng trình duyệt",
    }
}

/// The glyph the design puts before a property of this kind.
pub fn kind_glyph(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "Aa",
        FieldKind::LongText => "¶",
        FieldKind::Number => "#",
        FieldKind::Select => "▾",
        FieldKind::MultiSelect => "≡",
        FieldKind::Checkbox => "□",
        FieldKind::Scale => "☆",
        FieldKind::Date => "▦",
        FieldKind::Url => "↗",
    }
}

/// The glyph of a built-in property: what kind of input it is.
pub fn builtin_glyph(field: BuiltinField) -> &'static str {
    use BuiltinField as B;
    match field {
        B::Playbook => "↗",
        B::SetupGrade
        | B::ExitReason
        | B::Market
        | B::EmotionBefore
        | B::EmotionAfter
        | B::DayMood
        | B::PlanTimeframe
        | B::MissedReason
        | B::MissedSide => "▾",
        B::ExecutionRating => "☆",
        B::Tags | B::Mistakes | B::Checklist => "≡",
        B::FollowedPlan | B::ReviewLater => "□",
        B::Notes | B::PlanThesis | B::DayPlan | B::DayReview | B::MissedNotes => "¶",
        B::PlannedRisk
        | B::PlanEntry
        | B::PlanStopLoss
        | B::PlanTakeProfit
        | B::PlanRisk
        | B::MissedPrices
        | B::MissedOutcome => "#",
        B::Screenshots => "▦",
        B::MissedSymbol => "Aa",
    }
}

/// The stable text a kind is stored under in a drop-down.
fn kind_key(kind: FieldKind) -> &'static str {
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

// ---------------------------------------------------------------------------------------------
// Pure edits of the form and its values.
// ---------------------------------------------------------------------------------------------

/// The layout with every property the form should show: the ones `layout` has, plus a built-in
/// a newer version adds and a property of the trader's defined after it was saved. Edits go
/// through this, so that every property can be found by [`FormLayout`]'s methods.
pub fn complete(layout: &FormLayout, defs: &[FieldDef]) -> FormLayout {
    let mut items = layout.items.clone();
    let shown = |field: FieldRef| FormItem { field, label: None, hidden: false, width: Width::Full };
    for &builtin in BuiltinField::of(layout.scope) {
        let field = FieldRef::Builtin(builtin);
        if !items.iter().any(|i| i.field == field) {
            items.push(shown(field));
        }
    }
    for def in defs.iter().filter(|d| d.scope == layout.scope && !d.archived) {
        let field = FieldRef::Custom(def.key.clone());
        if !items.iter().any(|i| i.field == field) {
            items.push(shown(field));
        }
    }
    FormLayout { scope: layout.scope, items }
}

/// The layout after the visible property `from` is dragged to where visible property `to` is
/// (indexes among the rows that are shown); `None` when nothing moves.
pub fn move_property(layout: &FormLayout, defs: &[FieldDef], from: usize, to: usize) -> Option<FormLayout> {
    if from == to {
        return None;
    }
    let mut full = complete(layout, defs);
    let visible: Vec<FieldRef> = full.properties(defs).into_iter().filter(|p| !p.hidden).map(|p| p.field).collect();
    let (moving, target) = (visible.get(from)?, visible.get(to)?);
    let at = full.items.iter().position(|i| i.field == *target)?;
    full.move_to(moving, at).ok()?;
    Some(full)
}

/// A key no property of the form has, archived ones included: values stay under the key of a
/// deleted property, so it must not be given out again.
pub fn new_key(defs: &[FieldDef]) -> String {
    (1_u32..=u32::MAX)
        .map(|n| format!("field_{n}"))
        .find(|key| defs.iter().all(|d| d.key != *key))
        .unwrap_or_else(|| "field".to_owned())
}

/// A new property of `kind` for `scope`, named "Thuộc tính mới"; its id is 0 until it is saved.
pub fn new_def(scope: FieldScope, kind: FieldKind, defs: &[FieldDef]) -> FieldDef {
    FieldDef {
        id: 0,
        scope,
        key: new_key(defs),
        label: "Thuộc tính mới".to_owned(),
        kind,
        options: Vec::new(),
        max: (kind == FieldKind::Scale).then_some(DEFAULT_SCALE),
        required: Requirement::Optional,
        analysed: false,
        position: i32::try_from(defs.len()).unwrap_or(i32::MAX),
        archived: false,
    }
}

/// The top of a scale the trader has not set.
const DEFAULT_SCALE: u8 = 5;

/// The copy of a property, under a new key, ready to be saved.
pub fn duplicate_def(def: &FieldDef, defs: &[FieldDef]) -> FieldDef {
    FieldDef {
        id: 0,
        key: new_key(defs),
        label: format!("{} (bản sao)", def.label),
        position: i32::try_from(defs.len()).unwrap_or(i32::MAX),
        archived: false,
        ..def.clone()
    }
}

/// What changing a property's kind does on the entry being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct KindChange {
    /// The definition with its new kind (a select's options made from the value, a scale's top).
    pub def: FieldDef,
    /// The value after conversion; `None` when nothing was written or nothing carries over.
    pub value: Option<FieldValue>,
    /// A value was written and does not carry over to the new kind: it would be lost.
    pub lost: bool,
}

/// Converts `value` for `def` becoming `to`, as Notion does (`form::convert`); the trader is
/// asked first when [`KindChange::lost`].
pub fn change_kind(def: &FieldDef, value: Option<&FieldValue>, to: FieldKind) -> KindChange {
    let converted = value.map(|v| convert(v, to));
    let lost = matches!(converted, Some(None));
    let mut value = converted.flatten();
    let mut next = FieldDef { kind: to, ..def.clone() };
    if matches!(to, FieldKind::Select | FieldKind::MultiSelect) {
        let made = options_from(value.as_slice());
        for option in made {
            if !next.options.contains(&option) {
                next.options.push(option);
            }
        }
    }
    if to == FieldKind::Scale {
        let max = next.max.unwrap_or(DEFAULT_SCALE);
        next.max = Some(max);
        if let Some(FieldValue::Scale(n)) = value {
            value = Some(FieldValue::Scale(n.min(max)));
        }
    }
    KindChange { def: next, value, lost }
}

/// `text` as a stored date (`YYYY-MM-DD`), or `None` when it is not one.
pub fn parse_date(text: &str) -> Option<String> {
    let text = text.trim();
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().map(|_| text.to_owned())
}

/// The choices of a multi-select with `option` added at the end (once).
pub fn with_choice(mut chosen: Vec<String>, option: &str) -> Vec<String> {
    chosen.retain(|c| c != option);
    chosen.push(option.to_owned());
    chosen
}

/// The choices of a multi-select without `option`.
pub fn without_choice(mut chosen: Vec<String>, option: &str) -> Vec<String> {
    chosen.retain(|c| c != option);
    chosen
}

/// The key a row is identified by, so that it keeps its state while the others change.
fn row_key(field: &FieldRef) -> String {
    match field {
        FieldRef::Builtin(builtin) => format!("builtin:{builtin:?}"),
        FieldRef::Custom(key) => format!("custom:{key}"),
    }
}

// ---------------------------------------------------------------------------------------------
// What the list is given and what it reports.
// ---------------------------------------------------------------------------------------------

/// The changes the list asks for. Each closure should save the change and update what the
/// [`PropertyList`] reads, so that the rows follow.
#[derive(Clone)]
pub struct PropertyEdits {
    layout: Rc<dyn Fn(FormLayout)>,
    value: ValueEdit,
    def: Rc<dyn Fn(FieldDef)>,
}

type ValueEdit = Rc<dyn Fn(&str, Option<FieldValue>)>;

impl std::fmt::Debug for PropertyEdits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyEdits").finish_non_exhaustive()
    }
}

impl PropertyEdits {
    /// `on_layout` hears the whole new layout (a reorder, a rename of a built-in, a property
    /// hidden, shown, added or deleted); `on_value` hears `(key, value)` for a property of the
    /// trader's, `None` when it is cleared; `on_def` hears a definition that is new (id 0) or
    /// changed (renamed, another kind, options, flags, archived).
    pub fn new(
        on_layout: impl Fn(FormLayout) + 'static,
        on_value: impl Fn(&str, Option<FieldValue>) + 'static,
        on_def: impl Fn(FieldDef) + 'static,
    ) -> Self {
        Self { layout: Rc::new(on_layout), value: Rc::new(on_value), def: Rc::new(on_def) }
    }
}

type BuiltinEditor = Rc<dyn Fn(BuiltinField) -> Option<AnyElement>>;

/// What a [`property_list`] draws and how it edits it.
#[derive(Clone)]
pub struct PropertyList {
    // Closures, so the list follows the signals they read.
    layout: Rc<dyn Fn() -> FormLayout>,
    defs: Rc<dyn Fn() -> Vec<FieldDef>>,
    values: Rc<dyn Fn() -> CustomValues>,
    builtin: BuiltinEditor,
    edits: PropertyEdits,
    /// The owner that outlives the rows: a question asked from a row (delete, change the kind) is a task of
    /// it, so that a row built again does not cancel it.
    owner: Owner,
}

impl std::fmt::Debug for PropertyList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyList").finish_non_exhaustive()
    }
}

impl PropertyList {
    /// A list over the form's `layout`, the trader's property definitions (archived ones too,
    /// so that new keys never clash) and the entry's `values` of those properties. The
    /// closures are read where the list binds them, so they follow signals.
    pub fn new(
        layout: impl Fn() -> FormLayout + 'static,
        defs: impl Fn() -> Vec<FieldDef> + 'static,
        values: impl Fn() -> CustomValues + 'static,
        edits: PropertyEdits,
    ) -> Self {
        Self {
            layout: Rc::new(layout),
            defs: Rc::new(defs),
            values: Rc::new(values),
            builtin: Rc::new(|_| None),
            owner: root_owner(),
            edits,
        }
    }

    /// How a built-in property's value is edited: `editor` makes the control of one (a
    /// `select_value`, `tag_picker`, `rating`, ...) or `None` for one this form does not edit,
    /// which then shows a dash.
    #[must_use]
    pub fn builtin<E: IntoElement>(mut self, editor: impl Fn(BuiltinField) -> Option<E> + 'static) -> Self {
        self.builtin = Rc::new(move |field| editor(field).map(IntoElement::into_element));
        self
    }
}

/// A `Copy` handle on a list's data, for the many closures the rows hand out.
#[derive(Clone, Copy)]
struct Handle(Stored<PropertyList>);

impl Handle {
    /// The owner questions are asked under ([`PropertyList`]'s).
    fn owner(self) -> Owner {
        self.0.with(|c| c.owner)
    }

    fn layout(self) -> FormLayout {
        let read = self.0.with(|c| Rc::clone(&c.layout));
        read()
    }

    fn defs(self) -> Vec<FieldDef> {
        let read = self.0.with(|c| Rc::clone(&c.defs));
        read()
    }

    fn values(self) -> CustomValues {
        let read = self.0.with(|c| Rc::clone(&c.values));
        read()
    }

    fn builtin(self, field: BuiltinField) -> Option<AnyElement> {
        let editor = self.0.with(|c| Rc::clone(&c.builtin));
        editor(field)
    }

    fn scope(self) -> FieldScope {
        self.layout().scope
    }

    fn def(self, key: &str) -> Option<FieldDef> {
        self.defs().into_iter().find(|d| d.key == key)
    }

    fn emit_layout(self, layout: FormLayout) {
        let emit = self.0.with(|c| Rc::clone(&c.edits.layout));
        emit(layout);
    }

    fn emit_value(self, key: &str, value: Option<FieldValue>) {
        let emit = self.0.with(|c| Rc::clone(&c.edits.value));
        emit(key, value);
    }

    fn emit_def(self, def: FieldDef) {
        let emit = self.0.with(|c| Rc::clone(&c.edits.def));
        emit(def);
    }

    /// The layout, complete, after `edit`; reported when the edit went through.
    fn edit_layout(self, edit: impl FnOnce(&mut FormLayout) -> bool) {
        let mut layout = complete(&self.layout(), &self.defs());
        if edit(&mut layout) {
            self.emit_layout(layout);
        }
    }

    /// Changes the definition `key` and reports it.
    fn edit_def(self, key: &str, edit: impl FnOnce(&mut FieldDef)) {
        if let Some(mut def) = self.def(key) {
            edit(&mut def);
            self.emit_def(def);
        }
    }

    fn rename(self, field: &FieldRef, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match field {
            FieldRef::Builtin(builtin) => {
                self.edit_layout(|layout| layout.rename_builtin(*builtin, name).is_ok());
            }
            FieldRef::Custom(key) => self.edit_def(key, |def| name.clone_into(&mut def.label)),
        }
    }

    fn hide(self, field: &FieldRef) {
        self.edit_layout(|layout| layout.hide(field).is_ok());
    }

    fn show(self, field: &FieldRef) {
        self.edit_layout(|layout| layout.show(field).is_ok());
    }

    /// Adds a property of `kind` at the end of the form.
    fn add(self, kind: FieldKind) {
        let defs = self.defs();
        let def = new_def(self.scope(), kind, &defs);
        let key = def.key.clone();
        self.emit_def(def);
        self.edit_layout(|layout| {
            layout.add_custom(&key);
            true
        });
    }

    fn duplicate(self, key: &str) {
        let defs = self.defs();
        let Some(def) = defs.iter().find(|d| d.key == key) else { return };
        let copy = duplicate_def(def, &defs);
        let copy_key = copy.key.clone();
        self.emit_def(copy);
        self.edit_layout(|layout| {
            layout.add_custom(&copy_key);
            true
        });
    }

    /// Takes the property out of the form and archives its definition; values written stay.
    fn delete(self, field: &FieldRef) {
        let FieldRef::Custom(key) = field else { return };
        self.edit_def(key, |def| def.archived = true);
        self.edit_layout(|layout| layout.remove(field).is_ok());
    }

    /// Gives a property another kind, asking first when a value written on this entry would be lost.
    fn change_kind(self, key: &str, to: FieldKind) {
        let Some(def) = self.def(key) else { return };
        if def.kind == to {
            return;
        }
        let current = self.values().get(key).cloned();
        let change = change_kind(&def, current.as_ref(), to);
        let key = key.to_owned();
        let apply = move || {
            self.emit_def(change.def.clone());
            self.emit_value(&key, change.value.clone());
        };
        if change.lost {
            confirm_then(
                self.owner(),
                "Đổi kiểu sẽ mất giá trị đã ghi?",
                format!("Giá trị đã ghi ở lệnh này không chuyển được sang kiểu “{}” và sẽ bị bỏ trống.", kind_name(to)),
                "Đổi kiểu",
                apply,
            );
        } else {
            apply();
        }
    }

    fn rename_option(self, key: &str, old: &str, new: &str) {
        let new = new.trim();
        let Some(def) = self.def(key) else { return };
        if new.is_empty() || new == old || def.options.iter().any(|o| o == new) {
            return;
        }
        let mut values = self.values();
        rename_option(&mut values, key, old, new);
        self.edit_def(key, |def| {
            for option in def.options.iter_mut().filter(|o| *o == old) {
                new.clone_into(option);
            }
        });
        if let Some(value) = values.get(key) {
            self.emit_value(key, Some(value.clone()));
        }
    }

    fn add_option(self, key: &str, option: &str) {
        let option = option.trim();
        if option.is_empty() {
            return;
        }
        self.edit_def(key, |def| {
            if !def.options.iter().any(|o| o == option) {
                def.options.push(option.to_owned());
            }
        });
    }

    fn remove_option(self, key: &str, option: &str) {
        self.edit_def(key, |def| def.options.retain(|o| o != option));
    }

    fn move_option(self, key: &str, from: usize, to: usize) {
        self.edit_def(key, |def| {
            if from < def.options.len() {
                let option = def.options.remove(from);
                def.options.insert(to.min(def.options.len()), option);
            }
        });
    }
}

/// A property of the trader's, as an editor sees it: where its value is read and written.
#[derive(Clone, Copy)]
struct Slot {
    list: Handle,
    key: Stored<String>,
}

impl Slot {
    fn new(list: Handle, key: String) -> Self {
        Self { list, key: stored(key) }
    }

    fn key(self) -> String {
        self.key.with(|k| k.clone())
    }

    fn value(self) -> Option<FieldValue> {
        self.list.values().get(&self.key()).cloned()
    }

    fn set(self, value: Option<FieldValue>) {
        self.list.emit_value(&self.key(), value);
    }

    fn text(self) -> String {
        match self.value() {
            Some(FieldValue::Text(t)) => t,
            _ => String::new(),
        }
    }

    fn set_text(self, text: &str) {
        self.set((!text.is_empty()).then(|| FieldValue::Text(text.to_owned())));
    }

    fn number(self) -> Option<f64> {
        match self.value() {
            Some(FieldValue::Number(n)) => Some(n),
            _ => None,
        }
    }

    fn choice(self) -> SharedString {
        match self.value() {
            Some(FieldValue::Choice(c)) => c.into(),
            _ => SharedString::default(),
        }
    }

    fn choices(self) -> Vec<String> {
        match self.value() {
            Some(FieldValue::Choices(c)) => c,
            _ => Vec::new(),
        }
    }

    fn flag(self) -> bool {
        matches!(self.value(), Some(FieldValue::Flag(true)))
    }

    fn scale(self) -> u8 {
        match self.value() {
            Some(FieldValue::Scale(n)) => n,
            _ => 0,
        }
    }

    fn date(self) -> String {
        match self.value() {
            Some(FieldValue::Date(d)) => d,
            _ => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The list.
// ---------------------------------------------------------------------------------------------

/// Width of the name column, in points.
const NAME_WIDTH: f32 = 168.0;
/// Height of a row before it has been laid out, in points.
const ROW_HEIGHT: f32 = 40.0;

/// The rows of the form: shown properties with their editors, then a line for the hidden ones
/// and "+ Thêm thuộc tính". See the module's docs for what the caller provides.
pub fn property_list(config: PropertyList) -> impl IntoElement {
    let list = Handle(stored(config));
    let properties = computed(move || list.layout().properties(&list.defs()));
    let shown = computed(move || properties.with(|all| all.iter().filter(|p| !p.hidden).cloned().collect::<Vec<_>>()));
    let hidden = computed(move || properties.with(|all| all.iter().filter(|p| p.hidden).cloned().collect::<Vec<_>>()));
    // How tall each row has been laid out, by row key: where a drag ends depends on it.
    let heights = stored(HashMap::<String, f32>::new());
    view()
        .gap_1()
        .child(each(
            shown,
            |property: &Property| row_key(&property.field),
            move |row, index| property_row(list, shown, row, index, heights),
        ))
        .child(hidden_properties(list, hidden))
        .child(add_property(list))
}

/// One row of the list: grip, glyph, the name that opens the popover, and the editor.
fn property_row(
    list: Handle,
    shown: Memo<Vec<Property>>,
    row: ReadSignal<Property>,
    index: ReadSignal<usize>,
    heights: Stored<HashMap<String, f32>>,
) -> impl IntoElement {
    let measured = row_key(&row.with_untracked(|p| p.field.clone()));
    let move_to = move |to: usize| {
        let from = index.get_untracked();
        if let Some(layout) = move_property(&list.layout(), &list.defs(), from, to) {
            list.emit_layout(layout);
        }
    };
    let by_drag = move |_dx: f32, dy: f32| {
        let rows: Vec<f32> = shown.with(|all| {
            all.iter().map(|p| heights.with(|h| h.get(&row_key(&p.field)).copied()).unwrap_or(ROW_HEIGHT)).collect()
        });
        move_to(list_drop_index(index.get_untracked(), dy, &rows));
    };
    let by_key = move |step: Step| move_to(stepped(index.get_untracked(), step, shown.with(Vec::len)));
    view()
        .flex_row()
        .items_start()
        .gap_2()
        .min_h(px(34.0))
        .on_layout(move |bounds: &Bounds<Points>| {
            heights.with(|h| h.insert(measured.clone(), bounds.size.height.0));
        })
        .child(view().pt(px(4.0)).child(drag_handle(
            move || format!("Kéo để đổi thứ tự {}", row.with(|p| p.label.clone())),
            by_drag,
            by_key,
        )))
        .child(
            view()
                .w(px(NAME_WIDTH))
                .flex_row()
                .items_start()
                .gap_2()
                .child(
                    text(move || row.with(glyph_of))
                        .w(px(20.0))
                        .pt_2()
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                )
                // The name may be longer than the column: it wraps there, and the "riêng" chip with it,
                // instead of running into the editor beside.
                .child(view().flex_1().min_w(px(0.0)).child(property_popover(list, row))),
        )
        .child(view().flex_1().min_w(px(0.0)).child(dynamic(move || {
            let property = row.get();
            untracked(|| editor_of(list, &property))
        })))
}

/// The glyph before a property's name.
fn glyph_of(property: &Property) -> &'static str {
    match (&property.field, property.kind) {
        (_, Some(kind)) => kind_glyph(kind),
        (FieldRef::Builtin(builtin), None) => builtin_glyph(*builtin),
        (FieldRef::Custom(_), None) => "•",
    }
}

/// The control that edits the property's value.
fn editor_of(list: Handle, property: &Property) -> AnyElement {
    match &property.field {
        FieldRef::Builtin(builtin) => list
            .builtin(*builtin)
            .unwrap_or_else(|| text("—").text_sm().pt_2().text_color(move || theme().muted_foreground).into_element()),
        FieldRef::Custom(key) => {
            let slot = Slot::new(list, key.clone());
            let max = list.def(key).map_or(DEFAULT_SCALE, |d| d.max.unwrap_or(DEFAULT_SCALE).max(1));
            custom_editor(slot, property, max)
        }
    }
}

/// The input of a property of the trader's, by its kind.
fn custom_editor(slot: Slot, property: &Property, scale_max: u8) -> AnyElement {
    let label = property.label.clone();
    match property.kind.unwrap_or(FieldKind::Text) {
        FieldKind::Text => {
            text_field(label, None, "Trống", move || slot.text(), move |t| slot.set_text(t)).into_element()
        }
        FieldKind::LongText => {
            textarea(label, "Trống", 3, move || slot.text(), move |t| slot.set_text(t)).into_element()
        }
        FieldKind::Number => number_field(
            label,
            None,
            NumberRange::any(),
            move || slot.number(),
            move |n| slot.set(n.map(FieldValue::Number)),
        )
        .into_element(),
        FieldKind::Select => {
            let options = property.options.clone();
            select_value(
                label,
                move || {
                    std::iter::once(Choice::new("", "Trống"))
                        .chain(options.iter().map(|o| Choice::new(o.clone(), o.clone())))
                        .collect()
                },
                move || slot.choice(),
                move |chosen| {
                    slot.set((!chosen.is_empty()).then(|| FieldValue::Choice(chosen.to_string())));
                },
            )
            .into_element()
        }
        FieldKind::MultiSelect => {
            let options = property.options.clone();
            tag_picker(
                label,
                move || options.iter().map(|o| (o.clone(), o.clone())).collect(),
                move || slot.choices(),
                move |option, on| {
                    let next =
                        if on { with_choice(slot.choices(), option) } else { without_choice(slot.choices(), option) };
                    slot.set((!next.is_empty()).then_some(FieldValue::Choices(next)));
                },
            )
            .into_element()
        }
        FieldKind::Checkbox => {
            check_box(label, move || slot.flag(), move |on| slot.set(Some(FieldValue::Flag(on)))).into_element()
        }
        FieldKind::Scale if scale_max <= MAX_STARS => rating_of(
            label,
            scale_max,
            move || slot.scale(),
            move |n| slot.set((n > 0).then_some(FieldValue::Scale(n))),
        )
        .into_element(),
        FieldKind::Scale => view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(number_field(
                label,
                Some("điểm"),
                NumberRange::between(1.0, f64::from(scale_max)),
                move || Some(f64::from(slot.scale())).filter(|n| *n > 0.0),
                move |n| slot.set(n.and_then(scale_point).map(FieldValue::Scale)),
            ))
            .child(text(format!("/ {scale_max}")).text_sm().text_color(move || theme().muted_foreground))
            .into_element(),
        FieldKind::Date => date_editor(slot, label).into_element(),
        FieldKind::Url => view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(view().flex_1().child(text_field(
                label,
                None,
                "https://…",
                move || slot.text(),
                move |t| slot.set_text(t),
            )))
            .child(dynamic(move || {
                (!slot.text().trim().is_empty()).then(|| {
                    icon_action(Icon::Share, "Mở liên kết", Variant::Ghost, move || {
                        // Whether a browser took it does not matter here.
                        drop(open_url(slot.text().trim().to_owned()));
                    })
                })
            }))
            .into_element(),
    }
}

/// The most points a scale shows as stars; a longer one is typed as a number.
const MAX_STARS: u8 = 5;

/// A whole number of points from what the number field reads.
fn scale_point(n: f64) -> Option<u8> {
    let rounded = n.round();
    (1.0..=255.0).contains(&rounded).then(|| {
        // Held to 1..=255 just above.
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range checked"
        )]
        let points = rounded as u8;
        points
    })
}

/// A date chosen from the calendar (a popup under the field), shown as `20/11/2026`; the clear button
/// empties it. The value is stored as `YYYY-MM-DD`.
fn date_editor(slot: Slot, label: String) -> impl IntoElement {
    let chosen = move || chrono::NaiveDate::parse_from_str(&slot.date(), "%Y-%m-%d").ok();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(date_pick(
            DatePick::day(
                label,
                chosen,
                || chrono::Local::now().date_naive(),
                move |day| slot.set(Some(FieldValue::Date(day.format("%Y-%m-%d").to_string()))),
            ),
            date_trigger(move || {
                chosen().map_or_else(|| "Chọn ngày".to_owned(), |day| day.format("%d/%m/%Y").to_string())
            }),
        ))
        .child(dynamic(move || {
            chosen().map(|_| icon_action(Icon::Close, "Xoá ngày", Variant::Ghost, move || slot.set(None)))
        }))
}

// ---------------------------------------------------------------------------------------------
// The popover of a property, the hidden ones, and adding one.
// ---------------------------------------------------------------------------------------------

/// Width of a popover, in points.
const POPOVER_WIDTH: f32 = 320.0;
/// Height of an option's row in the popover.
const OPTION_ROW: f32 = 36.0;

/// The name of a property as a button; pressing it opens the popover to rename it, change its
/// kind and options, mark it expected or analysed, duplicate, hide or delete it. Built-in
/// properties can only be renamed and hidden.
fn property_popover(list: Handle, row: ReadSignal<Property>) -> impl IntoElement {
    let open = signal(false);
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &PopoverEvent| {
            if let PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(name_button(row))
        .child(popover_panel(list, row, open))
}

/// The name as the row shows it, with "riêng" after the name of a property of the trader's.
fn name_button(row: ReadSignal<Property>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .label(move || format!("Sửa thuộc tính {}", row.with(|p| p.label.clone())))
        .focusable(true)
        .cursor_pointer()
        .on_press(|| {})
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .min_h(px(34.0))
        .py_2()
        .child(
            text(move || row.with(|p| p.label.clone()))
                .min_w(px(0.0))
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(dynamic(move || {
            row.with(|p| matches!(p.field, FieldRef::Custom(_))).then(|| chip("riêng", Tone::Primary))
        }))
}

fn popover_panel(list: Handle, row: ReadSignal<Property>, open: Signal<bool>) -> impl IntoElement {
    let field = row.with_untracked(|p| p.field.clone());
    let rename_field = field.clone();
    let hide_field = field.clone();
    let body = match &field {
        FieldRef::Custom(key) => custom_panel(list, key.clone(), row, open).into_element(),
        FieldRef::Builtin(_) => view().into_element(),
    };
    panel()
        .child(commit_field(
            "Tên thuộc tính",
            "Tên thuộc tính",
            move || row.with(|p| p.label.clone()),
            move |name| list.rename(&rename_field, &name),
        ))
        .child(body)
        .child(action_icon(Icon::EyeOff, "Ẩn khỏi form", Variant::Ghost, move || {
            open.set(false);
            list.hide(&hide_field);
        }))
}

/// The frame of a popover.
fn panel() -> View {
    view()
        .w(px(POPOVER_WIDTH))
        .p_3()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .text_color(move || theme().popover_foreground)
        .shadow_lg()
}

/// What only a property of the trader's has: its kind, options, flags, duplicate and delete.
fn custom_panel(list: Handle, key: String, row: ReadSignal<Property>, open: Signal<bool>) -> impl IntoElement {
    let slot = Slot::new(list, key);
    let def = move || list.def(&slot.key());
    let kind_of = move || def().map_or(FieldKind::Text, |d| d.kind);
    let flag = move |pick: fn(&FieldDef) -> bool| def().is_some_and(|d| pick(&d));
    let delete_field = row.with_untracked(|p| p.field.clone());
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
                    || KINDS.iter().map(|&k| Choice::new(kind_key(k), format!("{} {}", kind_glyph(k), kind_name(k)))).collect(),
                    move || kind_key(kind_of()).into(),
                    move |chosen| {
                        if let Some(kind) = KINDS.iter().copied().find(|&k| kind_key(k) == &*chosen) {
                            list.change_kind(&slot.key(), kind);
                        }
                    },
                )),
        )
        .child(dynamic(move || {
            matches!(kind_of(), FieldKind::Select | FieldKind::MultiSelect).then(|| untracked(|| options_editor(list, slot)))
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
                                if let Some(max) = n.and_then(scale_point) {
                                    list.edit_def(&slot.key(), |d| d.max = Some(max));
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
                list.edit_def(&slot.key(), |d| {
                    d.required = if on { Requirement::Expected } else { Requirement::Optional };
                });
            },
        ))
        .child(switch_row(
            "Đưa vào Phân tích",
            Some("Thành một cột trong bảng Lệnh và một nhóm trong Phân tích"),
            move || flag(|d| d.analysed),
            move |on| list.edit_def(&slot.key(), |d| d.analysed = on),
        ))
        .child(action_icon(Icon::Document, "Nhân bản", Variant::Ghost, move || {
            open.set(false);
            list.duplicate(&slot.key());
        }))
        .child(action_icon(Icon::Trash, "Xoá thuộc tính", Variant::Ghost, move || {
            let field = delete_field.clone();
            let name = row.with_untracked(|p| p.label.clone());
            confirm_then(
                list.owner(),
                format!("Xoá thuộc tính “{name}”?"),
                "Giá trị đã ghi ở các lệnh cũ được giữ lại; thuộc tính chỉ biến khỏi form và khôi phục được ở Tuỳ biến form.",
                "Xoá thuộc tính",
                move || {
                    // The popover may have been built again while the question was open.
                    open.try_set(false);
                    list.delete(&field);
                },
            );
        }))
}

/// The choices of a select or multi-select: rename (the old entries follow), reorder, remove, add.
fn options_editor(list: Handle, slot: Slot) -> impl IntoElement {
    let added = signal(0_u32);
    let options = move || list.def(&slot.key()).map_or_default(|d| d.options);
    view()
        .gap_1()
        .child(
            text("Lựa chọn (đổi tên ở đây thì mọi lệnh cũ đổi theo)")
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(dynamic(move || {
            let all = options();
            let len = all.len();
            untracked(|| {
                all.into_iter()
                    .enumerate()
                    .map(|(at, option)| option_row(list, slot, at, len, option))
                    .collect::<Vec<_>>()
            })
        }))
        .child(commit_field(
            "Thêm lựa chọn",
            "+ Thêm lựa chọn",
            move || {
                let _ = added.get();
                String::new()
            },
            move |option| {
                list.add_option(&slot.key(), &option);
                added.update(|n| *n += 1);
            },
        ))
}

fn option_row(list: Handle, slot: Slot, at: usize, len: usize, option: String) -> impl IntoElement {
    let name = option.clone();
    let renamed = option.clone();
    let removed = option.clone();
    let by_drag = move |_dx: f32, dy: f32| {
        let to = list_drop_index(at, dy, &vec![OPTION_ROW; len]);
        if to != at {
            list.move_option(&slot.key(), at, to);
        }
    };
    let by_key = move |step: Step| {
        let to = stepped(at, step, len);
        if to != at {
            list.move_option(&slot.key(), at, to);
        }
    };
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(drag_handle(format!("Kéo để đổi thứ tự lựa chọn {name}"), by_drag, by_key))
        .child(view().flex_1().child(commit_field(
            format!("Tên lựa chọn {name}"),
            "Tên lựa chọn",
            move || option.clone(),
            move |new| list.rename_option(&slot.key(), &renamed, &new),
        )))
        .child(icon_action(Icon::Close, format!("Xoá lựa chọn {name}"), Variant::Ghost, move || {
            list.remove_option(&slot.key(), &removed);
        }))
}

/// "▸ 2 thuộc tính ẩn (Phiên giao dịch, Risk dự kiến)": press it to list them, each with a
/// button to show it again.
fn hidden_properties(list: Handle, hidden: Memo<Vec<Property>>) -> impl IntoElement {
    let open = signal(false);
    view().gap_1().child(dynamic(move || {
        let all = hidden.get();
        if all.is_empty() {
            return None;
        }
        let names = all.iter().map(|p| p.label.as_str()).collect::<Vec<_>>().join(", ");
        let expanded = open.get();
        Some(untracked(|| {
            view()
                .gap_1()
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .label(format!("{} thuộc tính ẩn", all.len()))
                        .focusable(true)
                        .cursor_pointer()
                        .on_press(move || open.update(|o| *o = !*o))
                        .py_2()
                        .child(
                            text(format!("{} {} thuộc tính ẩn ({names})", if expanded { "▾" } else { "▸" }, all.len()))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .when(expanded, |v| v.children(all.iter().map(|p| hidden_row(list, p))))
        }))
    }))
}

fn hidden_row(list: Handle, property: &Property) -> impl IntoElement {
    let field = property.field.clone();
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(text(glyph_of(property)).w(px(20.0)).text_sm().text_color(move || theme().muted_foreground))
        .child(
            text(property.label.clone()).flex_1().min_w(px(0.0)).text_sm().text_color(move || theme().muted_foreground),
        )
        .child(icon_action(Icon::Eye, format!("Hiện {}", property.label), Variant::Ghost, move || list.show(&field)))
}

/// "+ Thêm thuộc tính": a popover that asks for the kind, then adds the property at the end of
/// the form under the name "Thuộc tính mới", ready to be renamed.
fn add_property(list: Handle) -> impl IntoElement {
    let open = signal(false);
    popover()
        .open(move || Some(open.get()))
        .on_open_change(move |event: &PopoverEvent| {
            if let PopoverEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .offset(px(6.0))
        .margin(px(8.0))
        .trigger(action_icon(Icon::Plus, "Thêm thuộc tính", Variant::Ghost, || {}))
        .child(
            panel()
                .gap_1()
                .child(text("Thêm thuộc tính — chọn kiểu").text_xs().text_color(move || theme().muted_foreground))
                .children(KINDS.iter().map(move |&kind| {
                    view()
                        .role(AccessibilityRole::Button)
                        .label(format!("Thêm thuộc tính kiểu {}", kind_name(kind)))
                        .focusable(true)
                        .cursor_pointer()
                        .on_press(move || {
                            open.set(false);
                            list.add(kind);
                        })
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .h(px(36.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(move || theme().accent))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(
                            text(kind_glyph(kind)).w(px(24.0)).text_sm().text_color(move || theme().muted_foreground),
                        )
                        .child(text(kind_name(kind)).text_sm())
                        .child(text(kind_hint(kind)).text_xs().text_color(move || theme().muted_foreground))
                })),
        )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_core::{Size, platform::Os};
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::{DialogAnswer, Headless};

    use super::*;

    fn def(key: &str, label: &str, kind: FieldKind, options: &[&str]) -> FieldDef {
        FieldDef {
            id: 1,
            scope: FieldScope::Day,
            key: key.to_owned(),
            label: label.to_owned(),
            kind,
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            max: (kind == FieldKind::Scale).then_some(10),
            required: Requirement::Optional,
            analysed: false,
            position: 0,
            archived: false,
        }
    }

    fn day() -> FormLayout {
        FormLayout::default_for(FieldScope::Day)
    }

    fn labels(layout: &FormLayout, defs: &[FieldDef]) -> Vec<String> {
        layout.properties(defs).into_iter().filter(|p| !p.hidden).map(|p| p.label).collect()
    }

    #[test]
    fn a_layout_is_completed_with_what_it_lacks() {
        let defs = [def("focus", "Mức tập trung", FieldKind::Scale, &[])];
        let layout = FormLayout { scope: FieldScope::Day, items: Vec::new() };
        let full = complete(&layout, &defs);
        assert_eq!(full.items.len(), 3 + 1, "the day's three built-ins and the property");
        assert_eq!(full.items.last().map(|i| i.field.clone()), Some(FieldRef::Custom("focus".into())));
        assert_eq!(complete(&full, &defs), full, "completing twice changes nothing");
    }

    #[test]
    fn dragging_a_visible_row_lands_on_the_one_it_is_dropped_on() {
        let defs = [def("focus", "Mức tập trung", FieldKind::Scale, &[])];
        let mut layout = complete(&day(), &defs);
        layout.hide(&FieldRef::Builtin(BuiltinField::DayReview)).expect("in the form");
        // Visible: Kế hoạch ngày, Tâm trạng, Mức tập trung (Tổng kết is hidden between them).
        let moved = move_property(&layout, &defs, 0, 2).expect("a move");
        assert_eq!(labels(&moved, &defs), ["Tâm trạng", "Mức tập trung", "Kế hoạch ngày"]);
        assert!(
            moved.items.iter().any(|i| i.field == FieldRef::Builtin(BuiltinField::DayReview) && i.hidden),
            "the hidden row is still in the form"
        );
        assert_eq!(move_property(&layout, &defs, 1, 1), None, "dropped where it was");
        assert_eq!(move_property(&layout, &defs, 0, 9), None, "dropped past the end");
    }

    #[test]
    fn new_properties_never_reuse_a_key_not_even_an_archived_one() {
        let mut gone = def("field_1", "Đã xoá", FieldKind::Text, &[]);
        gone.archived = true;
        let defs = [gone, def("field_2", "Còn", FieldKind::Text, &[])];
        assert_eq!(new_key(&defs), "field_3");
        assert_eq!(new_key(&[]), "field_1");
        let made = new_def(FieldScope::Day, FieldKind::Scale, &defs);
        assert_eq!((made.id, made.key.as_str(), made.max, made.position), (0, "field_3", Some(5), 2));
        let copy = duplicate_def(&defs[1], &defs);
        assert_eq!((copy.id, copy.key.as_str(), copy.label.as_str()), (0, "field_3", "Còn (bản sao)"));
    }

    #[test]
    fn changing_a_kind_carries_the_value_over_or_says_it_would_be_lost() {
        let text = def("tf", "Khung", FieldKind::Text, &[]);
        let typed = FieldValue::Text("1,5".into());
        let to_number = change_kind(&text, Some(&typed), FieldKind::Number);
        assert_eq!((to_number.value, to_number.lost), (Some(FieldValue::Number(1.5)), false));

        let to_select = change_kind(&text, Some(&FieldValue::Text("M15".into())), FieldKind::Select);
        assert_eq!(to_select.def.options, ["M15"], "the old text becomes an option");
        assert_eq!(to_select.value, Some(FieldValue::Choice("M15".into())));

        let to_date = change_kind(&text, Some(&typed), FieldKind::Date);
        assert!(to_date.lost && to_date.value.is_none(), "text that is not a date is lost");

        let empty = change_kind(&text, None, FieldKind::Date);
        assert!(!empty.lost, "nothing written, nothing lost");

        let number = def("n", "Điểm", FieldKind::Number, &[]);
        let to_scale = change_kind(&number, Some(&FieldValue::Number(9.0)), FieldKind::Scale);
        assert_eq!(
            (to_scale.def.max, to_scale.value),
            (Some(5), Some(FieldValue::Scale(5))),
            "held to the scale's top"
        );
    }

    #[test]
    fn dates_and_multi_choices_are_parsed_and_toggled() {
        assert_eq!(parse_date(" 2026-11-20 "), Some("2026-11-20".to_owned()));
        assert_eq!(parse_date("2026-13-40"), None);
        assert_eq!(parse_date("20/11/2026"), None);
        assert_eq!(with_choice(vec!["M5".into()], "H1"), ["M5", "H1"]);
        assert_eq!(without_choice(vec!["M5".into(), "H1".into()], "M5"), ["H1"]);
        assert_eq!(with_choice(vec!["M5".into()], "M5"), ["M5"], "no duplicates");
    }

    #[test]
    fn every_kind_has_a_name_a_glyph_and_a_hint() {
        for kind in KINDS {
            assert!(!kind_name(kind).is_empty() && !kind_glyph(kind).is_empty() && !kind_hint(kind).is_empty());
        }
        let keys: std::collections::HashSet<_> = KINDS.iter().map(|&k| kind_key(k)).collect();
        assert_eq!(keys.len(), KINDS.len(), "kind keys are distinct");
    }

    #[component]
    fn Harness() -> impl IntoElement {
        super::super::theme::install();
        let layout = signal(day());
        let defs = signal(vec![
            def("tf", "Khung vào lệnh", FieldKind::Select, &["M5", "M15", "H1"]),
            def("note", "Ghi chú riêng", FieldKind::Text, &[]),
        ]);
        let values = signal(CustomValues::from([("tf".to_owned(), FieldValue::Choice("M5".into()))]));
        let edits = PropertyEdits::new(
            move |next| layout.set(next),
            move |key, value| {
                values.update(|all| match value {
                    Some(value) => {
                        all.insert(key.to_owned(), value);
                    }
                    None => {
                        all.remove(key);
                    }
                });
            },
            move |next| {
                defs.update(|all| match all.iter_mut().find(|d| d.key == next.key) {
                    Some(old) => *old = next,
                    None => all.push(next),
                });
            },
        );
        let list = PropertyList::new(move || layout.get(), move || defs.get(), move || values.get(), edits)
            .builtin(|field| (field == BuiltinField::DayPlan).then(|| text("plan editor")));
        view()
            .p_4()
            .gap_3()
            .child(property_list(list))
            .child(text(move || format!("order {}", labels(&layout.get(), &defs.get()).join("|"))))
            .child(text(move || {
                format!(
                    "values {}",
                    values.with(|all| all.iter().map(|(k, v)| format!("{k}={v:?}")).collect::<Vec<_>>().join(";"))
                )
            }))
    }

    const ORDER: &str = "order Kế hoạch ngày|Tổng kết|Tâm trạng|Khung vào lệnh|Ghi chú riêng";

    fn start() -> Headless {
        let mut config =
            EngineConfig { viewport: Size::new(px(1000.0), px(900.0)), ..EngineConfig::new(Root::of::<Harness>()) };
        config.environment.os = Os::Linux;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for(ORDER).expect("the harness shows");
        app
    }

    #[test]
    fn the_list_shows_every_property_with_its_editor() {
        let app = start();
        assert!(app.screen().shows("plan editor"), "a built-in is drawn by the caller");
        assert!(app.screen().shows("riêng"), "the trader's own properties are badged");
        assert!(app.screen().shows("Thêm thuộc tính"));
        assert!(app.screen().shows("M5"), "the select shows the chosen option");
    }

    #[test]
    fn typing_in_a_value_editor_reports_the_value() {
        let mut app = start();
        app.type_text("label:Ghi chú riêng", "abc").expect("types in the text field");
        app.wait_for("values note=Text(\"abc\");tf=Choice(\"M5\")").expect("the value was reported");
        app.clear("label:Ghi chú riêng").expect("clears the field");
        app.wait_for("values tf=Choice(\"M5\")").expect("an empty value is cleared, not stored");
    }

    #[test]
    fn dragging_a_row_reorders_the_layout() {
        let mut app = start();
        app.drag("label:Kéo để đổi thứ tự Kế hoạch ngày", point(px(0.0), px(900.0))).expect("drags");
        app.wait_for("order Tổng kết|Tâm trạng|Khung vào lệnh|Ghi chú riêng|Kế hoạch ngày").expect("it went last");
    }

    #[test]
    fn a_focused_grip_moves_its_row_with_the_arrow_keys() {
        let mut app = start();
        // Tab visits grips, names and editors; the second row's grip comes after the first row's
        // grip, name, and the caller's editor is not a stop.
        app.press("label:Kéo để đổi thứ tự Tổng kết").expect("the grip is pressable");
        app.press_keys(&["tab", "tab", "tab"]).ok();
        app.press_keys(&["shift+tab", "shift+tab", "shift+tab"]).ok();
        app.press_key("down").ok();
        // Whichever grip has the focus moves one row; the order always stays a permutation.
        app.wait_for_within("text:/^order /", Duration::from_secs(1)).expect("the order is still shown");
    }

    #[test]
    fn a_built_in_is_renamed_and_hidden_from_its_popover() {
        let mut app = start();
        app.press("label:Sửa thuộc tính Tổng kết").expect("the name opens the popover");
        app.type_text("label:Tên thuộc tính", " cuối ngày").expect("types a new name");
        app.submit("label:Tên thuộc tính").expect("Enter keeps it");
        app.wait_for("order Kế hoạch ngày|Tổng kết cuối ngày|Tâm trạng|Khung vào lệnh|Ghi chú riêng").expect("renamed");

        app.press("role:button[Ẩn khỏi form]").expect("hide");
        app.wait_for("order Kế hoạch ngày|Tâm trạng|Khung vào lệnh|Ghi chú riêng").expect("hidden");
        app.wait_for("text:▸ 1 thuộc tính ẩn (Tổng kết cuối ngày)").expect("the hidden line names it");
        app.press("label:1 thuộc tính ẩn").expect("the hidden line expands");
        app.press("label:Hiện Tổng kết cuối ngày").expect("show it again");
        app.wait_for("order Kế hoạch ngày|Tổng kết cuối ngày|Tâm trạng|Khung vào lệnh|Ghi chú riêng").expect("back");
    }

    #[test]
    fn a_new_property_is_added_by_kind() {
        let mut app = start();
        app.press("role:button[Thêm thuộc tính]").expect("the add button");
        app.press("label:Thêm thuộc tính kiểu Số").expect("pick the kind");
        app.wait_for("order Kế hoạch ngày|Tổng kết|Tâm trạng|Khung vào lệnh|Ghi chú riêng|Thuộc tính mới")
            .expect("the new property is last");
    }

    #[test]
    fn changing_to_a_kind_that_loses_the_value_asks_first() {
        let mut app = start();
        app.on_dialog(|_| DialogAnswer::Press("Đổi kiểu".to_owned()));
        app.press("label:Sửa thuộc tính Khung vào lệnh").expect("the name opens the popover");
        app.press("label:Kiểu").expect("the kind drop-down");
        app.press("text:▦ Ngày").expect("the date kind");
        app.wait_until("the value was dropped", Duration::from_secs(3), |screen| {
            screen.texts().iter().any(|t| t.trim() == "values")
        })
        .expect("a choice that is not a date was dropped after the confirmation");
    }

    #[test]
    fn declining_the_loss_keeps_the_kind_and_the_value() {
        let mut app = start();
        app.on_dialog(|_| DialogAnswer::Press("Huỷ".to_owned()));
        app.press("label:Sửa thuộc tính Khung vào lệnh").expect("the name opens the popover");
        app.press("label:Kiểu").expect("the kind drop-down");
        app.press("text:▦ Ngày").expect("the date kind");
        app.wait(Duration::from_millis(300)).expect("waits");
        assert!(app.screen().shows("values tf=Choice(\"M5\")"), "{:?}", app.screen().texts());
    }
}
