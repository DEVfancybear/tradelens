//! What the trader customises: the option lists the journal picks from (tags, mistakes,
//! emotions, exit reasons, grades, …) and fields of their own on trades, plans, days,
//! reviews and missed trades.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A list of options the trader edits in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Tag,
    Mistake,
    Emotion,
    ExitReason,
    SetupGrade,
    MarketCondition,
    Timeframe,
}

impl CatalogKind {
    pub const ALL: [Self; 7] = [
        Self::Tag,
        Self::Mistake,
        Self::Emotion,
        Self::ExitReason,
        Self::SetupGrade,
        Self::MarketCondition,
        Self::Timeframe,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Tag => "Tag",
            Self::Mistake => "Lỗi",
            Self::Emotion => "Cảm xúc",
            Self::ExitReason => "Lý do thoát lệnh",
            Self::SetupGrade => "Hạng setup",
            Self::MarketCondition => "Trạng thái thị trường",
            Self::Timeframe => "Khung thời gian",
        }
    }

    /// The options a new database starts with: `(key, label)`.
    pub fn defaults(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Tag => &[
                ("london_open", "London open"),
                ("trend_h4", "Theo trend H4"),
                ("news", "Tin tức"),
                ("a_plus", "A+ setup"),
            ],
            Self::Mistake => &[
                ("early_entry", "Vào sớm"),
                ("moved_sl", "Dời SL"),
                ("no_close_confirm", "Không chờ nến đóng"),
                ("oversize", "Quá lot"),
                ("revenge", "Revenge"),
                ("early_exit", "Chốt non"),
            ],
            Self::Emotion => &[
                ("calm", "Bình tĩnh"),
                ("confident", "Tự tin"),
                ("fear", "Sợ hãi"),
                ("greed", "Tham"),
                ("fomo", "FOMO"),
                ("tired", "Mệt"),
            ],
            Self::ExitReason => &[
                ("tp", "Chạm TP"),
                ("sl", "Chạm SL"),
                ("manual", "Đóng tay"),
                ("trailing", "Trailing stop"),
                ("breakeven", "Hòa vốn"),
                ("time", "Hết thời gian"),
                ("ea", "EA đóng"),
                ("stop_out", "Stop out"),
            ],
            Self::SetupGrade => &[("a_plus", "A+"), ("a", "A"), ("b", "B"), ("c", "C")],
            Self::MarketCondition => {
                &[("trend", "Có xu hướng"), ("range", "Đi ngang"), ("volatile", "Biến động mạnh"), ("quiet", "Yên ắng")]
            }
            Self::Timeframe => &[("m5", "M5"), ("m15", "M15"), ("h1", "H1"), ("h4", "H4"), ("d1", "D1")],
        }
    }
}

/// One option of a catalog. Archived options stay readable on old entries and leave the pickers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: i64,
    pub kind: CatalogKind,
    /// Stable: journal entries store it, so renaming the label keeps them.
    pub key: String,
    pub label: String,
    /// A hex color the chip shows with, or none for the default.
    pub color: Option<String>,
    pub position: i32,
    pub archived: bool,
}

/// Where a field of the trader's own appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldScope {
    Trade,
    EntryPlan,
    Day,
    Review,
    MissedTrade,
}

/// The input a field shows and the value it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    LongText,
    Number,
    Select,
    MultiSelect,
    Checkbox,
    /// 1 to `max` stars or points.
    Scale,
    Date,
}

/// A field the trader added, such as "Khung vào lệnh" (select) or "Mức tập trung" (scale 1–10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub id: i64,
    pub scope: FieldScope,
    /// Stable key the values are stored under.
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    /// The choices of a select or multi-select.
    #[serde(default)]
    pub options: Vec<String>,
    /// The top of a scale (default 5).
    #[serde(default)]
    pub max: Option<u8>,
    #[serde(default)]
    pub required: Requirement,
    /// Shown as a column in the trades table and as a breakdown in Analytics.
    #[serde(default)]
    pub analysed: bool,
    pub position: i32,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    #[default]
    Optional,
    /// Saving warns until it is filled; nothing is blocked.
    Expected,
}

/// A value of a field of the trader's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Choice(String),
    Choices(Vec<String>),
    Flag(bool),
    Scale(u8),
    /// `YYYY-MM-DD`.
    Date(String),
}

/// The values of an entry's own fields, by field key.
pub type CustomValues = BTreeMap<String, FieldValue>;

/// Why a value does not fit its field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    #[error("trường \"{0}\" cần kiểu giá trị khác")]
    WrongKind(String),
    #[error("\"{value}\" không có trong lựa chọn của trường \"{field}\"")]
    UnknownOption { field: String, value: String },
    #[error("trường \"{field}\" chỉ nhận 1 – {max}")]
    OutOfScale { field: String, max: u8 },
    #[error("ngày không hợp lệ ở trường \"{0}\"")]
    BadDate(String),
}

impl FieldDef {
    /// The scale's top.
    pub fn scale_max(&self) -> u8 {
        self.max.unwrap_or(5).max(1)
    }

    /// Checks that `value` fits the field.
    ///
    /// # Errors
    ///
    /// [`FieldError`] naming what does not fit.
    pub fn validate(&self, value: &FieldValue) -> Result<(), FieldError> {
        let option = |v: &String| {
            if self.options.contains(v) {
                Ok(())
            } else {
                Err(FieldError::UnknownOption { field: self.label.clone(), value: v.clone() })
            }
        };
        match (self.kind, value) {
            (FieldKind::Text | FieldKind::LongText, FieldValue::Text(_))
            | (FieldKind::Number, FieldValue::Number(_))
            | (FieldKind::Checkbox, FieldValue::Flag(_)) => Ok(()),
            (FieldKind::Select, FieldValue::Choice(v)) => option(v),
            (FieldKind::MultiSelect, FieldValue::Choices(vs)) => vs.iter().try_for_each(option),
            (FieldKind::Scale, FieldValue::Scale(n)) => {
                if (1..=self.scale_max()).contains(n) {
                    Ok(())
                } else {
                    Err(FieldError::OutOfScale { field: self.label.clone(), max: self.scale_max() })
                }
            }
            (FieldKind::Date, FieldValue::Date(d)) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map(|_| ())
                .map_err(|_| FieldError::BadDate(self.label.clone())),
            _ => Err(FieldError::WrongKind(self.label.clone())),
        }
    }
}

/// The fields of `defs` expected for `scope` that `values` leaves empty, by label.
pub fn missing_expected<'defs>(defs: &'defs [FieldDef], scope: FieldScope, values: &CustomValues) -> Vec<&'defs str> {
    defs.iter()
        .filter(|d| d.scope == scope && !d.archived && d.required == Requirement::Expected)
        .filter(|d| !values.contains_key(&d.key))
        .map(|d| d.label.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(kind: FieldKind) -> FieldDef {
        FieldDef {
            id: 1,
            scope: FieldScope::Trade,
            key: "entry_tf".into(),
            label: "Khung vào lệnh".into(),
            kind,
            options: vec!["M5".into(), "M15".into()],
            max: Some(10),
            required: Requirement::Expected,
            analysed: true,
            position: 0,
            archived: false,
        }
    }

    #[test]
    fn values_are_checked_against_their_field() {
        assert!(def(FieldKind::Select).validate(&FieldValue::Choice("M15".into())).is_ok());
        assert!(matches!(
            def(FieldKind::Select).validate(&FieldValue::Choice("H4".into())),
            Err(FieldError::UnknownOption { .. })
        ));
        assert!(def(FieldKind::MultiSelect).validate(&FieldValue::Choices(vec!["M5".into()])).is_ok());
        assert!(def(FieldKind::Scale).validate(&FieldValue::Scale(10)).is_ok());
        assert!(def(FieldKind::Scale).validate(&FieldValue::Scale(11)).is_err());
        assert!(def(FieldKind::Date).validate(&FieldValue::Date("2026-11-20".into())).is_ok());
        assert!(def(FieldKind::Number).validate(&FieldValue::Text("x".into())).is_err());
    }

    #[test]
    fn expected_fields_left_empty_are_listed() {
        let defs = [def(FieldKind::Select)];
        assert_eq!(missing_expected(&defs, FieldScope::Trade, &CustomValues::new()), ["Khung vào lệnh"]);
        let filled = CustomValues::from([("entry_tf".to_owned(), FieldValue::Choice("M5".into()))]);
        assert!(missing_expected(&defs, FieldScope::Trade, &filled).is_empty());
        assert!(missing_expected(&defs, FieldScope::Day, &CustomValues::new()).is_empty());
    }

    #[test]
    fn values_round_trip_through_json() {
        let value = FieldValue::Choices(vec!["M5".into()]);
        let json = serde_json::to_string(&value).expect("serialises");
        assert_eq!(json, r#"{"type":"choices","value":["M5"]}"#);
        assert_eq!(serde_json::from_str::<FieldValue>(&json).expect("parses"), value);
    }

    #[test]
    fn every_catalog_has_unique_default_keys() {
        for kind in CatalogKind::ALL {
            let mut keys: Vec<_> = kind.defaults().iter().map(|(k, _)| *k).collect();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), kind.defaults().len(), "{kind:?}");
        }
    }
}
