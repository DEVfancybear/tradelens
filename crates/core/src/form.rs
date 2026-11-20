//! Forms the trader shapes like a Notion database: every input of the trade journal,
//! the entry plan, the day, the review and the missed trade is a property that can be
//! renamed, reordered, hidden or shown; their own properties can also be added, edited,
//! converted to another type and deleted.
//!
//! Built-in properties map to the journal's typed fields and cannot be deleted, only
//! hidden. Deleting one of the trader's own properties archives its definition, so the
//! values already written stay in the entries and come back if it is restored.

use serde::{Deserialize, Serialize};

use crate::catalog::{CatalogKind, CustomValues, FieldDef, FieldKind, FieldScope, FieldValue};

/// The typed inputs the app ships with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinField {
    // The trade's journal.
    Playbook,
    SetupGrade,
    ExecutionRating,
    ExitReason,
    Market,
    Tags,
    Mistakes,
    EmotionBefore,
    EmotionAfter,
    FollowedPlan,
    Checklist,
    Notes,
    Screenshots,
    PlannedRisk,
    ReviewLater,
    // The plan written at entry.
    PlanThesis,
    PlanEntry,
    PlanStopLoss,
    PlanTakeProfit,
    PlanRisk,
    PlanTimeframe,
    // The day.
    DayPlan,
    DayReview,
    DayMood,
    // A missed trade.
    MissedSymbol,
    MissedSide,
    MissedReason,
    MissedPrices,
    MissedOutcome,
    MissedNotes,
}

impl BuiltinField {
    const TRADE: [Self; 15] = [
        Self::Playbook,
        Self::SetupGrade,
        Self::ExecutionRating,
        Self::ExitReason,
        Self::Market,
        Self::Tags,
        Self::Mistakes,
        Self::EmotionBefore,
        Self::EmotionAfter,
        Self::FollowedPlan,
        Self::Checklist,
        Self::Notes,
        Self::Screenshots,
        Self::PlannedRisk,
        Self::ReviewLater,
    ];
    const PLAN: [Self; 6] = [
        Self::PlanThesis,
        Self::PlanEntry,
        Self::PlanStopLoss,
        Self::PlanTakeProfit,
        Self::PlanRisk,
        Self::PlanTimeframe,
    ];
    const DAY: [Self; 3] = [Self::DayPlan, Self::DayReview, Self::DayMood];
    const MISSED: [Self; 6] = [
        Self::MissedSymbol,
        Self::MissedSide,
        Self::MissedReason,
        Self::MissedPrices,
        Self::MissedOutcome,
        Self::MissedNotes,
    ];

    /// The built-in properties of a form, in their first order. A review's questions are
    /// its template ([`crate::review::ReviewTemplate`]), so it has none.
    pub fn of(scope: FieldScope) -> &'static [Self] {
        match scope {
            FieldScope::Trade => &Self::TRADE,
            FieldScope::EntryPlan => &Self::PLAN,
            FieldScope::Day => &Self::DAY,
            FieldScope::MissedTrade => &Self::MISSED,
            FieldScope::Review => &[],
        }
    }

    pub fn default_label(self) -> &'static str {
        match self {
            Self::Playbook => "Setup",
            Self::SetupGrade => "Hạng setup",
            Self::ExecutionRating => "Điểm thực thi",
            Self::ExitReason => "Lý do thoát lệnh",
            Self::Market => "Trạng thái thị trường",
            Self::Tags => "Tag",
            Self::Mistakes => "Lỗi",
            Self::EmotionBefore => "Cảm xúc trước lệnh",
            Self::EmotionAfter => "Cảm xúc sau lệnh",
            Self::FollowedPlan => "Đúng kế hoạch",
            Self::Checklist => "Checklist trước lệnh",
            Self::Notes | Self::MissedNotes => "Ghi chú",
            Self::Screenshots => "Ảnh chart",
            Self::PlannedRisk | Self::PlanRisk => "Risk dự kiến",
            Self::ReviewLater => "Xem lại sau",
            Self::PlanThesis => "Lý do vào lệnh",
            Self::PlanEntry => "Giá vào dự kiến",
            Self::PlanStopLoss => "Stop loss dự kiến",
            Self::PlanTakeProfit => "Take profit dự kiến",
            Self::PlanTimeframe => "Khung thời gian",
            Self::DayPlan => "Kế hoạch ngày",
            Self::DayReview => "Tổng kết",
            Self::DayMood => "Tâm trạng",
            Self::MissedSymbol => "Symbol",
            Self::MissedSide => "Chiều",
            Self::MissedReason => "Lý do không vào",
            Self::MissedPrices => "Entry / SL / TP dự kiến",
            Self::MissedOutcome => "Giá đi tới",
        }
    }

    /// The option list a built-in picks from, which the trader edits in Settings.
    pub fn catalog(self) -> Option<CatalogKind> {
        Some(match self {
            Self::SetupGrade => CatalogKind::SetupGrade,
            Self::ExitReason => CatalogKind::ExitReason,
            Self::Market => CatalogKind::MarketCondition,
            Self::Tags => CatalogKind::Tag,
            Self::Mistakes => CatalogKind::Mistake,
            Self::EmotionBefore | Self::EmotionAfter | Self::DayMood => CatalogKind::Emotion,
            Self::PlanTimeframe => CatalogKind::Timeframe,
            Self::MissedReason => CatalogKind::MissedReason,
            _ => return None,
        })
    }
}

/// A property of a form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum FieldRef {
    Builtin(BuiltinField),
    /// The key of one of the trader's [`FieldDef`]s.
    Custom(String),
}

/// How wide a property shows in the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Width {
    #[default]
    Full,
    Half,
}

/// One property as the trader arranged it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormItem {
    pub field: FieldRef,
    /// The trader's name for a built-in; a custom property's name lives in its definition.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub width: Width,
}

impl FormItem {
    fn shown(field: FieldRef) -> Self {
        Self { field, label: None, hidden: false, width: Width::Full }
    }
}

/// A property ready to draw: its name, its input and where its options come from.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub field: FieldRef,
    pub label: String,
    pub kind: Option<FieldKind>,
    pub catalog: Option<CatalogKind>,
    pub options: Vec<String>,
    pub hidden: bool,
    pub width: Width,
}

/// Why an edit of a form was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormError {
    #[error("thuộc tính có sẵn không xoá được, chỉ ẩn được")]
    BuiltinCannotBeDeleted,
    #[error("không có thuộc tính này trong form")]
    NotInForm,
    #[error("tên thuộc tính không được để trống")]
    EmptyLabel,
}

/// The arrangement of one form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormLayout {
    pub scope: FieldScope,
    pub items: Vec<FormItem>,
}

impl FormLayout {
    /// The built-ins in their first order, all shown.
    pub fn default_for(scope: FieldScope) -> Self {
        Self { scope, items: BuiltinField::of(scope).iter().map(|&b| FormItem::shown(FieldRef::Builtin(b))).collect() }
    }

    /// The form to draw from this layout and the trader's definitions: a property
    /// defined after the layout was saved (or a built-in a new version adds) is appended,
    /// and one deleted since is left out.
    pub fn properties(&self, defs: &[FieldDef]) -> Vec<Property> {
        let live = |key: &str| defs.iter().find(|d| d.scope == self.scope && d.key == key && !d.archived);
        let mut items = self.items.clone();
        for &builtin in BuiltinField::of(self.scope) {
            if !items.iter().any(|i| i.field == FieldRef::Builtin(builtin)) {
                items.push(FormItem::shown(FieldRef::Builtin(builtin)));
            }
        }
        for def in defs.iter().filter(|d| d.scope == self.scope && !d.archived) {
            if !items.iter().any(|i| i.field == FieldRef::Custom(def.key.clone())) {
                items.push(FormItem::shown(FieldRef::Custom(def.key.clone())));
            }
        }
        items
            .into_iter()
            .filter_map(|item| match &item.field {
                FieldRef::Builtin(b) => Some(Property {
                    label: item.label.clone().unwrap_or_else(|| b.default_label().to_owned()),
                    kind: None,
                    catalog: b.catalog(),
                    options: Vec::new(),
                    field: item.field,
                    hidden: item.hidden,
                    width: item.width,
                }),
                FieldRef::Custom(key) => live(key).map(|def| Property {
                    label: def.label.clone(),
                    kind: Some(def.kind),
                    catalog: None,
                    options: def.options.clone(),
                    field: item.field.clone(),
                    hidden: item.hidden,
                    width: item.width,
                }),
            })
            .collect()
    }

    fn index_of(&self, field: &FieldRef) -> Result<usize, FormError> {
        self.items.iter().position(|i| i.field == *field).ok_or(FormError::NotInForm)
    }

    /// Renames a built-in (an empty name gives back its default). Custom properties are
    /// renamed on their [`FieldDef`].
    ///
    /// # Errors
    ///
    /// [`FormError::NotInForm`] when `field` is not in the layout.
    pub fn rename_builtin(&mut self, field: BuiltinField, label: &str) -> Result<(), FormError> {
        let index = self.index_of(&FieldRef::Builtin(field))?;
        let label = label.trim();
        self.items[index].label = (!label.is_empty() && label != field.default_label()).then(|| label.to_owned());
        Ok(())
    }

    /// Hides a property: its values stay, the form no longer shows it.
    ///
    /// # Errors
    ///
    /// [`FormError::NotInForm`] when `field` is not in the layout.
    pub fn hide(&mut self, field: &FieldRef) -> Result<(), FormError> {
        let index = self.index_of(field)?;
        self.items[index].hidden = true;
        Ok(())
    }

    /// Shows a hidden property again.
    ///
    /// # Errors
    ///
    /// [`FormError::NotInForm`] when `field` is not in the layout.
    pub fn show(&mut self, field: &FieldRef) -> Result<(), FormError> {
        let index = self.index_of(field)?;
        self.items[index].hidden = false;
        Ok(())
    }

    /// Moves a property to `to` (clamped to the end), as a drag in the form does.
    ///
    /// # Errors
    ///
    /// [`FormError::NotInForm`] when `field` is not in the layout.
    pub fn move_to(&mut self, field: &FieldRef, to: usize) -> Result<(), FormError> {
        let item = self.items.remove(self.index_of(field)?);
        self.items.insert(to.min(self.items.len()), item);
        Ok(())
    }

    /// Adds one of the trader's properties at the end (after its definition is saved).
    pub fn add_custom(&mut self, key: &str) {
        let field = FieldRef::Custom(key.to_owned());
        if !self.items.iter().any(|i| i.field == field) {
            self.items.push(FormItem::shown(field));
        }
    }

    /// Takes a property out of the form; archive its definition with it.
    ///
    /// # Errors
    ///
    /// [`FormError::BuiltinCannotBeDeleted`] for a built-in; [`FormError::NotInForm`].
    pub fn remove(&mut self, field: &FieldRef) -> Result<(), FormError> {
        if matches!(field, FieldRef::Builtin(_)) {
            return Err(FormError::BuiltinCannotBeDeleted);
        }
        let index = self.index_of(field)?;
        self.items.remove(index);
        Ok(())
    }
}

/// A value carried over when a property changes type, as Notion converts; `None` when
/// nothing sensible carries over (the value is then dropped).
pub fn convert(value: &FieldValue, to: FieldKind) -> Option<FieldValue> {
    use FieldValue as V;
    let text = match value {
        V::Text(t) | V::Choice(t) | V::Date(t) => t.clone(),
        V::Number(n) => n.to_string(),
        V::Choices(c) => c.join(", "),
        V::Flag(f) => if *f { "Có" } else { "Không" }.to_owned(),
        V::Scale(n) => n.to_string(),
    };
    match to {
        FieldKind::Text | FieldKind::LongText => Some(V::Text(text)),
        FieldKind::Number => match value {
            V::Number(n) => Some(V::Number(*n)),
            V::Scale(n) => Some(V::Number(f64::from(*n))),
            _ => text.trim().replace(',', ".").parse().ok().map(V::Number),
        },
        FieldKind::Select => match value {
            V::Choices(c) => c.first().cloned().map(V::Choice),
            _ => (!text.is_empty()).then_some(V::Choice(text)),
        },
        FieldKind::MultiSelect => {
            if let V::Choices(c) = value {
                return Some(V::Choices(c.clone()));
            }
            let parts: Vec<String> =
                text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
            (!parts.is_empty()).then_some(V::Choices(parts))
        }
        FieldKind::Checkbox => match value {
            V::Flag(f) => Some(V::Flag(*f)),
            V::Number(n) => Some(V::Flag(*n != 0.0)),
            _ => Some(V::Flag(!text.is_empty())),
        },
        FieldKind::Scale => match value {
            V::Scale(n) => Some(V::Scale(*n)),
            V::Number(n) if (1.0..=255.0).contains(n) => format!("{n:.0}").parse().ok().map(V::Scale),
            _ => None,
        },
        FieldKind::Date => matches!(value, V::Date(_)).then(|| value.clone()),
    }
}

/// The options a property's values must be among after a type change, collected from
/// the converted values (as Notion creates options from existing text).
pub fn options_from(values: &[FieldValue]) -> Vec<String> {
    let mut options: Vec<String> = Vec::new();
    for value in values {
        let found: Vec<&String> = match value {
            FieldValue::Choice(c) => vec![c],
            FieldValue::Choices(cs) => cs.iter().collect(),
            _ => Vec::new(),
        };
        for option in found {
            if !options.contains(option) {
                options.push(option.clone());
            }
        }
    }
    options
}

/// Renames an option of a select property in every entry's values.
pub fn rename_option(values: &mut CustomValues, key: &str, old: &str, new: &str) {
    match values.get_mut(key) {
        Some(FieldValue::Choice(c)) if c == old => new.clone_into(c),
        Some(FieldValue::Choices(cs)) => {
            for c in cs.iter_mut().filter(|c| *c == old) {
                new.clone_into(c);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Requirement;

    fn def(key: &str, state: &str) -> FieldDef {
        let archived = state == "archived";
        FieldDef {
            id: 1,
            scope: FieldScope::Trade,
            key: key.into(),
            label: "Mức tập trung".into(),
            kind: FieldKind::Scale,
            options: Vec::new(),
            max: Some(10),
            required: Requirement::Optional,
            analysed: false,
            position: 0,
            archived,
        }
    }

    #[test]
    fn a_form_is_renamed_reordered_hidden_and_extended() {
        let mut form = FormLayout::default_for(FieldScope::Trade);
        form.rename_builtin(BuiltinField::Playbook, "Chiến lược").expect("in form");
        form.move_to(&FieldRef::Builtin(BuiltinField::Notes), 0).expect("in form");
        form.hide(&FieldRef::Builtin(BuiltinField::Market)).expect("in form");
        form.add_custom("focus");
        let props = form.properties(&[def("focus", "live")]);
        assert_eq!(props[0].label, "Ghi chú");
        assert_eq!(props[1].label, "Chiến lược");
        assert!(props.iter().any(|p| p.field == FieldRef::Builtin(BuiltinField::Market) && p.hidden));
        assert_eq!(props.last().map(|p| p.label.as_str()), Some("Mức tập trung"));
        // An empty name gives the default back.
        form.rename_builtin(BuiltinField::Playbook, " ").expect("in form");
        assert_eq!(form.properties(&[])[1].label, "Setup");
    }

    #[test]
    fn built_ins_hide_and_custom_properties_delete() {
        let mut form = FormLayout::default_for(FieldScope::Trade);
        assert_eq!(form.remove(&FieldRef::Builtin(BuiltinField::Tags)), Err(FormError::BuiltinCannotBeDeleted));
        form.add_custom("focus");
        form.remove(&FieldRef::Custom("focus".into())).expect("in form");
        // A definition still live comes back at the end; an archived one does not.
        assert!(
            form.properties(&[def("focus", "archived")]).iter().all(|p| p.field != FieldRef::Custom("focus".into()))
        );
        assert!(form.properties(&[def("focus", "live")]).iter().any(|p| p.field == FieldRef::Custom("focus".into())));
    }

    #[test]
    fn values_convert_between_types() {
        use FieldValue as V;
        assert_eq!(convert(&V::Text("1,5".into()), FieldKind::Number), Some(V::Number(1.5)));
        assert_eq!(
            convert(&V::Text("M5, M15".into()), FieldKind::MultiSelect),
            Some(V::Choices(vec!["M5".into(), "M15".into()]))
        );
        assert_eq!(convert(&V::Choices(vec!["A".into(), "B".into()]), FieldKind::Select), Some(V::Choice("A".into())));
        assert_eq!(convert(&V::Number(7.0), FieldKind::Scale), Some(V::Scale(7)));
        assert_eq!(convert(&V::Choice("x".into()), FieldKind::Date), None);
        assert_eq!(options_from(&[V::Choice("A".into()), V::Choices(vec!["B".into(), "A".into()])]), ["A", "B"]);
    }

    #[test]
    fn renaming_an_option_follows_into_values() {
        let mut values = CustomValues::from([("tf".to_owned(), FieldValue::Choices(vec!["M5".into(), "H1".into()]))]);
        rename_option(&mut values, "tf", "M5", "5 phút");
        assert_eq!(values["tf"], FieldValue::Choices(vec!["5 phút".into(), "H1".into()]));
    }
}
