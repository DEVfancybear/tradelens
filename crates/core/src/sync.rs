//! The model of cloud sync, free of storage and network: changes, the field operations they
//! carry, the stamps that order them and the rules that merge them.
//!
//! Every synced thing (an *entity*) is a document of named *fields* ([`Fields`]) under a
//! portable key that means the same on every device (local row ids do not). A local write
//! becomes one [`Change`] holding only the fields that changed; applying changes merges them
//! field by field, the newest [`Stamp`] winning. Lists that two devices may extend at once
//! (tags, mistakes, attachments, options) are flattened into one field per element, so an
//! addition on one device and a removal on another both survive. The design and its rules are
//! in `docs/cloud-sync.md`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::model::Deal;

/// The kinds of entity that sync, as named in [`Change::entity`].
pub mod entity {
    /// An MT5 account, keyed by login and server.
    pub const ACCOUNT: &str = "account";
    /// One deal, keyed by account and ticket; merged as a whole record.
    pub const DEAL: &str = "deal";
    /// The journal of one trade, keyed by account, position and segment.
    pub const JOURNAL: &str = "journal";
    /// An account's discipline rules.
    pub const RULES: &str = "rules";
    /// One checklist item, keyed by its uid.
    pub const CHECKLIST: &str = "checklist";
    /// The note of one day, keyed by account (or all accounts) and day.
    pub const DAY_NOTE: &str = "day_note";
    /// A setting that is not specific to this device, keyed by its name.
    pub const SETTING: &str = "setting";
    /// One option of a catalog, keyed by kind and key.
    pub const CATALOG: &str = "catalog";
    /// A field of the trader's own, keyed by scope and key.
    pub const FIELD_DEF: &str = "field_def";
    /// The arrangement of one form, keyed by scope.
    pub const FORM_LAYOUT: &str = "form_layout";
    /// A setup of the playbook, keyed by its uid.
    pub const PLAYBOOK: &str = "playbook";
    /// A setup seen and not taken, keyed by its uid.
    pub const MISSED: &str = "missed";
    /// A weekly or monthly review, keyed by account, period and start.
    pub const REVIEW: &str = "review";
    /// The content hash of an attached image, keyed by its path in the data folder.
    pub const ATTACHMENT: &str = "attachment";

    /// Every entity, in the order changes are applied: what others refer to comes first.
    pub const APPLY_ORDER: [&str; 14] = [
        ACCOUNT,
        SETTING,
        CATALOG,
        FIELD_DEF,
        FORM_LAYOUT,
        CHECKLIST,
        PLAYBOOK,
        DEAL,
        RULES,
        JOURNAL,
        MISSED,
        DAY_NOTE,
        REVIEW,
        ATTACHMENT,
    ];
}

/// An entity's synced content: field name to JSON value. A field with no value is absent.
pub type Fields = BTreeMap<String, Value>;

/// One change, as stored in the `changes` table and in the files pushed to the cloud.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    /// The device that made it.
    pub device_id: String,
    /// Its number in that device's log, from 1 and without gaps.
    pub seq: u64,
    /// Unix milliseconds; with the device id it orders concurrent writes of the same field.
    pub at: i64,
    /// One of [`entity`].
    pub entity: String,
    /// The portable key of the entity (see [`join_key`]).
    pub key: String,
    /// The fields written, as a JSON array of [`FieldOp`]; `None` deletes the entity.
    pub value: Option<Value>,
}

/// What a [`Change`] does to its entity.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// Removes the entity.
    Delete,
    /// Writes (or unsets) fields.
    Fields(Vec<FieldOp>),
}

impl Change {
    /// The stamp that orders this change against others.
    pub fn stamp(&self) -> Stamp {
        Stamp { at: self.at, device: self.device_id.clone() }
    }

    /// What the change does.
    ///
    /// # Errors
    ///
    /// The JSON error when `value` is not a list of field operations.
    pub fn body(&self) -> Result<Body, serde_json::Error> {
        match &self.value {
            None => Ok(Body::Delete),
            Some(value) => Ok(Body::Fields(serde_json::from_value(value.clone())?)),
        }
    }
}

/// One field written by a change: set to `value`, or unset when `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldOp {
    /// The field's name.
    #[serde(rename = "f")]
    pub field: String,
    /// Its new value; `None` for a field that became empty or an element that was removed.
    #[serde(rename = "v", default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// When and where a write happened. Orders writes: later time first, then device id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Stamp {
    /// Unix milliseconds.
    pub at: i64,
    /// The writing device; the tiebreak between writes in the same millisecond.
    pub device: String,
}

impl Stamp {
    /// Whether a write stamped `self` replaces a value last written at `current`.
    pub fn beats(&self, current: Option<&Stamp>) -> bool {
        current.is_none_or(|current| self > current)
    }
}

/// The operations that turn `before` into `after`.
pub fn diff(before: &Fields, after: &Fields) -> Vec<FieldOp> {
    let mut ops = Vec::new();
    for (field, value) in after {
        if before.get(field) != Some(value) {
            ops.push(FieldOp { field: field.clone(), value: Some(value.clone()) });
        }
    }
    for field in before.keys().filter(|field| !after.contains_key(*field)) {
        ops.push(FieldOp { field: field.clone(), value: None });
    }
    ops
}

/// Sets or unsets `fields` as `op` says.
pub fn apply_op(fields: &mut Fields, op: &FieldOp) {
    match &op.value {
        Some(value) => {
            fields.insert(op.field.clone(), value.clone());
        }
        None => {
            fields.remove(&op.field);
        }
    }
}

// --- portable keys --------------------------------------------------------------------

const KEY_SEPARATOR: char = '|';

/// A portable key from its parts: the parts joined by `|`, with `|` and `\` inside a part escaped.
pub fn join_key(parts: &[&str]) -> String {
    let mut key = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            key.push(KEY_SEPARATOR);
        }
        for c in part.chars() {
            if c == KEY_SEPARATOR || c == '\\' {
                key.push('\\');
            }
            key.push(c);
        }
    }
    key
}

/// The parts of a key made by [`join_key`].
pub fn split_key(key: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut escaped = false;
    for c in key.chars() {
        if escaped {
            part.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == KEY_SEPARATOR {
            parts.push(std::mem::take(&mut part));
        } else {
            part.push(c);
        }
    }
    parts.push(part);
    parts
}

/// A setting that belongs to this device and never syncs: the device's id and the sync's own
/// bookkeeping (`sync.`), device identity and window state (`device.`, `window.`), the data
/// folder, the colour scheme (each machine follows its own), the old bridge's port and
/// anything under `cloud.`.
pub fn is_device_local(key: &str) -> bool {
    const KEYS: [&str; 3] = ["appearance", "data_dir", "data_folder"];
    const PREFIXES: [&str; 5] = ["sync.", "device.", "window.", "bridge.", "cloud."];
    KEYS.contains(&key) || PREFIXES.iter().any(|prefix| key.starts_with(prefix))
}

/// The prefix of the settings that hold a form's layout; those sync as [`entity::FORM_LAYOUT`].
pub(crate) const FORM_LAYOUT_PREFIX: &str = "form.layout.";

/// Whether the setting `key` syncs as an [`entity::SETTING`].
pub fn is_synced_setting(key: &str) -> bool {
    !is_device_local(key) && !key.starts_with(FORM_LAYOUT_PREFIX)
}

// --- flattening documents into fields ---------------------------------------------------

/// How an element of an ordered list names itself.
#[derive(Debug, Clone, Copy)]
pub enum Element {
    /// The element itself (a string).
    Whole,
    /// The member `name` of an object element.
    Key(&'static str),
    /// The first item of a pair.
    First,
}

impl Element {
    fn id(self, item: &Value) -> String {
        let named = |value: Option<&Value>| value.map_or_else(String::new, text);
        match self {
            Self::Whole => text(item),
            Self::Key(key) => named(item.get(key)),
            Self::First => named(item.get(0)),
        }
    }
}

fn text(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_owned)
}

/// How a JSON document breaks into [`Fields`]. Anything not named here is one field.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// Arrays of strings that are sets: one field `name#element` per element.
    pub sets: &'static [&'static str],
    /// Ordered arrays of elements: one field `name#id` per element and one `name.order` holding
    /// the ids in the trader's order, so that concurrent additions and a reorder both survive.
    pub lists: &'static [(&'static str, Element)],
    /// Objects whose members are independent: one field `name.member` per member.
    pub maps: &'static [&'static str],
    /// Numbers and booleans whose zero or `false` means "not set" (and so is left out).
    pub falsy_unset: &'static [&'static str],
}

fn is_unset(name: &str, value: &Value, shape: &Shape) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(false) => shape.falsy_unset.contains(&name),
        Value::Number(n) => shape.falsy_unset.contains(&name) && n.as_f64() == Some(0.0),
        Value::Bool(true) => false,
    }
}

/// Breaks `object` into fields. Empty values are left out: an empty text, list or map and a
/// `null` are the same as nothing written, so a device that never touched a field cannot
/// override another's value of it.
pub fn flatten(object: &Map<String, Value>, shape: &Shape) -> Fields {
    let mut fields = Fields::new();
    for (name, value) in object {
        if is_unset(name, value, shape) {
            continue;
        }
        let items = value.as_array().map_or_default(Vec::as_slice);
        if shape.sets.contains(&name.as_str()) {
            for item in items.iter().filter_map(Value::as_str) {
                fields.insert(format!("{name}#{item}"), Value::Bool(true));
            }
        } else if let Some((_, element)) = shape.lists.iter().find(|(list, _)| list == name) {
            let mut taken = BTreeSet::new();
            let mut order = Vec::new();
            for item in items {
                let mut id = element.id(item);
                for n in 1.. {
                    if taken.insert(id.clone()) {
                        break;
                    }
                    id = format!("{}\u{0}{n}", element.id(item));
                }
                fields.insert(format!("{name}#{id}"), item.clone());
                order.push(Value::String(id));
            }
            fields.insert(format!("{name}.order"), Value::Array(order));
        } else if shape.maps.contains(&name.as_str()) {
            for (member, member_value) in value.as_object().into_iter().flatten() {
                if !member_value.is_null() {
                    fields.insert(format!("{name}.{member}"), member_value.clone());
                }
            }
        } else {
            fields.insert(name.clone(), value.clone());
        }
    }
    fields
}

/// The document `fields` describe: the inverse of [`flatten`]. Every set, list and map of the
/// shape is present, empty when it has no elements; a list's elements follow the stored order
/// where it names them and are sorted by id after that, so every device builds the same list.
pub fn unflatten(fields: &Fields, shape: &Shape) -> Map<String, Value> {
    let mut object = Map::new();
    let mut sets: BTreeMap<&str, Vec<Value>> = shape.sets.iter().map(|name| (*name, Vec::new())).collect();
    let mut lists: BTreeMap<&str, BTreeMap<&str, &Value>> =
        shape.lists.iter().map(|(name, _)| (*name, BTreeMap::new())).collect();
    let mut maps: BTreeMap<&str, Map<String, Value>> = shape.maps.iter().map(|name| (*name, Map::new())).collect();
    for (field, value) in fields {
        let member_of = |name: &str, separator: char| field.strip_prefix(name)?.strip_prefix(separator);
        if let Some((name, element)) = shape.sets.iter().find_map(|name| Some((*name, member_of(name, '#')?))) {
            sets.entry(name).or_default().push(Value::String(element.to_owned()));
        } else if let Some((name, id)) = shape.lists.iter().find_map(|(name, _)| Some((*name, member_of(name, '#')?))) {
            lists.entry(name).or_default().insert(id, value);
        } else if shape.lists.iter().any(|(name, _)| member_of(name, '.') == Some("order")) {
            // Read below, with its list.
        } else if let Some((name, member)) = shape.maps.iter().find_map(|name| Some((*name, member_of(name, '.')?))) {
            maps.entry(name).or_default().insert(member.to_owned(), value.clone());
        } else {
            object.insert(field.clone(), value.clone());
        }
    }
    for (name, elements) in sets {
        object.insert(name.to_owned(), Value::Array(elements));
    }
    for (name, mut elements) in lists {
        let order = fields.get(&format!("{name}.order")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let mut items = Vec::with_capacity(elements.len());
        for id in order.iter().filter_map(Value::as_str) {
            if let Some(item) = elements.remove(id) {
                items.push(item.clone());
            }
        }
        items.extend(elements.into_values().cloned());
        object.insert(name.to_owned(), Value::Array(items));
    }
    for (name, members) in maps {
        object.insert(name.to_owned(), Value::Object(members));
    }
    object
}

// --- deals ---------------------------------------------------------------------------------

/// Which of two versions of a deal was written later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    /// The version already stored.
    Stored,
    /// The version that arrived.
    Incoming,
}

/// Merges two versions of the same deal. The one written later wins as a whole; what only the
/// other one knows of what the terminal measures (risk, reason, stop, target) fills what the
/// winner lacks, as an import over a deal already stored does. The result is the same whichever
/// version arrives first.
pub fn merge_deal(stored: &Deal, incoming: &Deal, newer: Winner) -> Deal {
    let (mut winner, loser) = match newer {
        Winner::Incoming => (incoming.clone(), stored),
        Winner::Stored => (stored.clone(), incoming),
    };
    if winner.risk.is_none() {
        winner.risk = loser.risk;
    }
    if winner.reason.is_none() {
        winner.reason.clone_from(&loser.reason);
    }
    if winner.sl <= 0.0 {
        winner.sl = loser.sl;
    }
    if winner.tp <= 0.0 {
        winner.tp = loser.tp;
    }
    winner
}

/// A new random identifier for an entity that has no natural key: 128 bits, hex.
pub fn new_uid() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let half = |salt: u8| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write(&nanos.to_le_bytes());
        hasher.write_u8(salt);
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(0), half(1))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::{DealEntry, DealKind};

    const SHAPE: Shape = Shape {
        sets: &["tags"],
        lists: &[("shots", Element::Key("path"))],
        maps: &["custom"],
        falsy_unset: &["rating"],
    };

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn keys_escape_their_separator() {
        let key = join_key(&["5012345", "Broker|Live", r"a\b"]);
        assert_eq!(split_key(&key), ["5012345", "Broker|Live", r"a\b"]);
        assert_eq!(split_key(&join_key(&["", ""])), ["", ""]);
        assert_eq!(split_key(&join_key(&["only"])), ["only"]);
    }

    #[test]
    fn stamps_order_by_time_then_device() {
        let at = |at, device: &str| Stamp { at, device: device.into() };
        assert!(at(2, "a").beats(Some(&at(1, "z"))));
        assert!(at(1, "b").beats(Some(&at(1, "a"))), "the later device id wins a tie");
        assert!(!at(1, "a").beats(Some(&at(1, "a"))), "applying the same write twice changes nothing");
        assert!(at(0, "").beats(None));
    }

    #[test]
    fn flattening_round_trips_and_leaves_out_what_is_empty() {
        let doc = object(json!({
            "tags": ["b", "a"],
            "shots": [{"path": "images/2.png", "caption": "x"}, {"path": "images/1.png", "caption": ""}],
            "custom": {"focus": {"type": "scale", "value": 4}},
            "notes": "hello", "setup": "", "rating": 0, "grade": null, "risk": 0.0
        }));
        let fields = flatten(&doc, &SHAPE);
        let names: Vec<&str> = fields.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "custom.focus",
                "notes",
                "risk",
                "shots#images/1.png",
                "shots#images/2.png",
                "shots.order",
                "tags#a",
                "tags#b"
            ]
        );
        let back = unflatten(&fields, &SHAPE);
        assert_eq!(back["tags"], json!(["a", "b"]), "sets come back sorted");
        assert_eq!(back["shots"][0]["path"], "images/2.png", "the list keeps the trader's order");
        assert_eq!(back["custom"]["focus"]["value"], 4);
        assert_eq!(back["notes"], "hello");
        assert!(!back.contains_key("setup"));
    }

    #[test]
    fn a_list_built_from_fields_is_the_same_whatever_the_order_they_arrive_in() {
        let mut fields = Fields::new();
        fields.insert("shots#b".into(), json!({"path": "b"}));
        fields.insert("shots#a".into(), json!({"path": "a"}));
        fields.insert("shots#c".into(), json!({"path": "c"}));
        fields.insert("shots.order".into(), json!(["c", "gone", "a"]));
        let list = unflatten(&fields, &SHAPE)["shots"].clone();
        let paths: Vec<&str> = list.as_array().expect("an array").iter().filter_map(|s| s["path"].as_str()).collect();
        assert_eq!(paths, ["c", "a", "b"], "the stored order first, then the others by id");
    }

    #[test]
    fn duplicate_elements_keep_their_places() {
        let doc = object(json!({"shots": [{"path": "p", "caption": "1"}, {"path": "p", "caption": "2"}]}));
        let back = unflatten(&flatten(&doc, &SHAPE), &SHAPE);
        assert_eq!(back["shots"], doc["shots"]);
    }

    #[test]
    fn a_diff_lists_what_was_set_and_what_was_unset() {
        let before: Fields = [("a".to_owned(), json!(1)), ("b".to_owned(), json!(2))].into();
        let after: Fields = [("a".to_owned(), json!(1)), ("c".to_owned(), json!(3))].into();
        let ops = diff(&before, &after);
        assert_eq!(
            ops,
            [FieldOp { field: "c".into(), value: Some(json!(3)) }, FieldOp { field: "b".into(), value: None }]
        );
        let mut merged = before;
        for op in &ops {
            apply_op(&mut merged, op);
        }
        assert_eq!(merged, after);
        assert_eq!(diff(&after, &after), Vec::new());
    }

    #[test]
    fn a_field_op_is_compact_on_the_wire() {
        let set = FieldOp { field: "rating".into(), value: Some(json!(3)) };
        assert_eq!(serde_json::to_string(&set).expect("serialises"), r#"{"f":"rating","v":3}"#);
        let unset = FieldOp { field: "tags#a".into(), value: None };
        assert_eq!(serde_json::to_string(&unset).expect("serialises"), r#"{"f":"tags#a"}"#);
        let back: FieldOp = serde_json::from_str(r#"{"f":"tags#a"}"#).expect("parses");
        assert_eq!(back, unset);
    }

    #[test]
    fn device_local_settings_do_not_sync() {
        for key in ["sync.device_id", "appearance", "window.size", "data_dir", "bridge.port"] {
            assert!(is_device_local(key), "{key}");
        }
        for key in ["time_display", "review.template.week"] {
            assert!(is_synced_setting(key), "{key}");
        }
        assert!(!is_synced_setting("form.layout.trade"), "layouts sync as their own entity");
    }

    fn deal(risk: Option<f64>, sl: f64, profit: f64) -> Deal {
        Deal {
            ticket: 1,
            order: 1,
            position_id: 1,
            time_msc: 0,
            symbol: "XAUUSD".into(),
            kind: DealKind::Buy,
            entry: DealEntry::In,
            volume: 0.1,
            price: 1.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            magic: 0,
            comment: String::new(),
            sl,
            tp: 0.0,
            risk,
            result_r: None,
            reason: None,
        }
    }

    #[test]
    fn deal_versions_merge_the_same_way_in_either_order() {
        let measured = deal(Some(25.0), 2640.0, 0.0);
        let report = deal(None, 0.0, 5.0);
        // The report is newer: its figures win, what it lacks comes from the measured one.
        let newer_report = merge_deal(&measured, &report, Winner::Incoming);
        assert_eq!((newer_report.profit, newer_report.risk, newer_report.sl), (5.0, Some(25.0), 2640.0));
        // The same two versions, seen in the other order, give the same deal.
        assert_eq!(merge_deal(&report, &measured, Winner::Stored), newer_report);
    }

    #[test]
    fn uids_differ() {
        assert_ne!(new_uid(), new_uid());
        assert_eq!(new_uid().len(), 32);
    }
}
