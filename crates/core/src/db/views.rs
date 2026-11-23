//! Every synced entity as a document of portable [`Fields`]: read from the tables
//! ([`load`]), written back to them ([`store`], [`remove`]) and listed ([`keys`]).
//!
//! These are the raw operations: they never touch the change log, so applying a remote change
//! cannot echo. The wrappers that log local writes diff what [`load`] returns before and after.
//! References between entities (a journal's setup, its checklist ticks, a missed trade's
//! account) are translated to the other entity's portable key on the way out and back to
//! this database's row id on the way in.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::catalog::{CustomValues, FieldDef};
use crate::form::FormLayout;
use crate::missed::MissedTrade;
use crate::model::{DayNote, Deal, JournalEntry, Rules, TradeKey};
use crate::playbook::Playbook;
use crate::review::{Period, PeriodReview};
use crate::search::{missed_document, playbook_document, review_document};
use crate::sync::{Element, Fields, Shape, entity, flatten, join_key, split_key, unflatten};

use super::{
    DEAL_COLUMNS, DbError, Result, day_document, day_values_on, deal_from_row_at, enum_name, journal_on,
    save_field_def_raw, set_setting_on, setting_on, write_deal_raw, write_index, write_journal_raw,
};

const JOURNAL: Shape = Shape {
    sets: &["tags", "mistakes", "checklist"],
    lists: &[("screenshots", Element::Key("path"))],
    maps: &["custom"],
    falsy_unset: &["rating", "review_later"],
};
const PLAYBOOK: Shape =
    Shape { sets: &[], lists: &[("examples", Element::Key("path"))], maps: &["custom"], falsy_unset: &["archived"] };
const MISSED: Shape =
    Shape { sets: &[], lists: &[("screenshots", Element::Key("path"))], maps: &["custom"], falsy_unset: &[] };
const REVIEW: Shape =
    Shape { sets: &[], lists: &[("answers", Element::First)], maps: &["custom"], falsy_unset: &["rating"] };
const FIELD_DEF: Shape =
    Shape { sets: &[], lists: &[("options", Element::Whole)], maps: &[], falsy_unset: &["archived", "analysed"] };
const FORM_LAYOUT: Shape = Shape { sets: &[], lists: &[("items", Element::Key("field"))], maps: &[], falsy_unset: &[] };
const DAY_NOTE: Shape = Shape { sets: &[], lists: &[], maps: &["custom"], falsy_unset: &[] };

/// The part of a key that stands for "every account".
const ALL_ACCOUNTS: &str = "*";

fn bad_key(entity: &str, key: &str) -> DbError {
    DbError::Invalid(format!("khoá đồng bộ không hợp lệ ({entity}): {key}"))
}

fn to_map(value: &impl Serialize) -> Result<Map<String, Value>> {
    match serde_json::to_value(value)? {
        Value::Object(map) => Ok(map),
        other => Err(DbError::Invalid(format!("not an object: {other}"))),
    }
}

/// Writes a text field unless it is empty (an empty text is the same as none).
fn put_text(fields: &mut Fields, name: &str, text: &str) {
    if !text.is_empty() {
        fields.insert(name.to_owned(), Value::String(text.to_owned()));
    }
}

fn put_number(fields: &mut Fields, name: &str, number: Option<impl Into<Value>>) {
    if let Some(number) = number {
        fields.insert(name.to_owned(), number.into());
    }
}

fn text_of(fields: &Fields, name: &str) -> String {
    fields.get(name).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn float_of(fields: &Fields, name: &str) -> Option<f64> {
    fields.get(name).and_then(Value::as_f64)
}

fn int_of(fields: &Fields, name: &str) -> Option<i64> {
    fields.get(name).and_then(Value::as_i64)
}

fn small_of<T: TryFrom<i64>>(fields: &Fields, name: &str) -> Option<T> {
    int_of(fields, name).and_then(|n| T::try_from(n).ok())
}

// --- references -------------------------------------------------------------------------

/// The key of an account, from its row id.
pub(super) fn account_key(conn: &Connection, account_id: i64) -> Result<String> {
    let (login, server): (String, String) =
        conn.query_row("SELECT login, server FROM accounts WHERE id = ?1", [account_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    Ok(join_key(&[&login, &server]))
}

/// The key part of an account that may be "all accounts" (id 0 or none).
pub(super) fn account_part(conn: &Connection, account_id: Option<i64>) -> Result<String> {
    match account_id.filter(|id| *id != 0) {
        Some(id) => account_key(conn, id),
        None => Ok(ALL_ACCOUNTS.to_owned()),
    }
}

fn account_row(conn: &Connection, login: &str, server: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM accounts WHERE login = ?1 AND server = ?2", [login, server], |r| r.get(0))
        .optional()?)
}

/// The row of an account by its key; a bare one is made when this database has not met it yet.
fn ensure_account(conn: &Connection, key: &str) -> Result<i64> {
    let parts = split_key(key);
    let [login, server] = parts.as_slice() else { return Err(bad_key(entity::ACCOUNT, key)) };
    Ok(conn.query_row(
        "INSERT INTO accounts (login, server) VALUES (?1, ?2)
         ON CONFLICT(login, server) DO UPDATE SET login = login RETURNING id",
        [login, server],
        |r| r.get(0),
    )?)
}

/// The account of a key part that may be [`ALL_ACCOUNTS`]: its row id, 0 for all of them.
fn account_of_part(conn: &Connection, part: &str) -> Result<i64> {
    if part == ALL_ACCOUNTS { Ok(0) } else { ensure_account(conn, part) }
}

fn account_of_ref(conn: &Connection, key: Option<&str>) -> Result<Option<i64>> {
    key.map(|key| ensure_account(conn, key)).transpose()
}

pub(super) fn uid_of(conn: &Connection, table: &str, id: i64) -> Result<Option<String>> {
    Ok(conn.query_row(&format!("SELECT uid FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).optional()?)
}

fn id_of(conn: &Connection, table: &str, uid: &str) -> Result<Option<i64>> {
    Ok(conn.query_row(&format!("SELECT id FROM {table} WHERE uid = ?1"), [uid], |r| r.get(0)).optional()?)
}

/// The portable key of a trade.
pub(super) fn journal_key(conn: &Connection, key: &TradeKey) -> Result<String> {
    let account = split_key(&account_key(conn, key.account_id)?);
    let (position, segment) = (key.position_id.to_string(), key.segment.to_string());
    Ok(join_key(&[&account[0], &account[1], &position, &segment]))
}

/// The trade a journal key names, when its account exists here.
fn trade_key_of(conn: &Connection, key: &str) -> Result<Option<TradeKey>> {
    let parts = split_key(key);
    let [login, server, position, segment] = parts.as_slice() else { return Err(bad_key(entity::JOURNAL, key)) };
    let (Ok(position_id), Ok(segment)) = (position.parse(), segment.parse()) else {
        return Err(bad_key(entity::JOURNAL, key));
    };
    Ok(account_row(conn, login, server)?.map(|account_id| TradeKey { account_id, position_id, segment }))
}

pub(super) fn deal_key(conn: &Connection, account_id: i64, ticket: i64) -> Result<String> {
    let account = split_key(&account_key(conn, account_id)?);
    Ok(join_key(&[&account[0], &account[1], &ticket.to_string()]))
}

fn day_key(account_part: &str, day: &str) -> String {
    join_key(&[account_part, day])
}

pub(super) fn day_note_key(conn: &Connection, account_id: i64, day: chrono::NaiveDate) -> Result<String> {
    Ok(day_key(&account_part(conn, Some(account_id))?, &day.to_string()))
}

pub(super) fn review_key(conn: &Connection, review: &PeriodReview) -> Result<String> {
    Ok(join_key(&[&account_part(conn, review.account_id)?, &enum_name(&review.period)?, &review.start.to_string()]))
}

// --- load -----------------------------------------------------------------------------------

/// The entity `key` of kind `entity` as fields; `None` when this database has none.
pub(super) fn load(conn: &Connection, kind: &str, key: &str) -> Result<Option<Fields>> {
    match kind {
        entity::ACCOUNT => load_account(conn, key),
        entity::DEAL => load_deal(conn, key),
        entity::JOURNAL => load_journal(conn, key),
        entity::RULES => load_rules(conn, key),
        entity::CHECKLIST => load_checklist(conn, key),
        entity::DAY_NOTE => load_day_note(conn, key),
        entity::SETTING => Ok(setting_on(conn, key)?.map(|value| Fields::from([("value".to_owned(), json!(value))]))),
        entity::CATALOG => load_catalog(conn, key),
        entity::FIELD_DEF => load_field_def(conn, key),
        entity::FORM_LAYOUT => load_form_layout(conn, key),
        entity::PLAYBOOK => load_playbook(conn, key),
        entity::MISSED => load_missed(conn, key),
        entity::REVIEW => load_review(conn, key),
        entity::ATTACHMENT => Ok(conn
            .query_row("SELECT sha256 FROM sync_attachments WHERE path = ?1", [key], |r| r.get::<_, String>(0))
            .optional()?
            .map(|sha| Fields::from([("sha256".to_owned(), json!(sha))]))),
        other => Err(DbError::Invalid(format!("loại đồng bộ không biết: {other}"))),
    }
}

fn load_account(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [login, server] = parts.as_slice() else { return Err(bad_key(entity::ACCOUNT, key)) };
    let row = conn
        .query_row(
            "SELECT broker, name, currency, initial_balance, last_balance, last_equity, server_offset_min
             FROM accounts WHERE login = ?1 AND server = ?2",
            [login, server],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                    r.get::<_, Option<i32>>(6)?,
                ))
            },
        )
        .optional()?;
    Ok(row.map(|(broker, name, currency, initial, balance, equity, offset)| {
        let mut fields = Fields::new();
        put_text(&mut fields, "broker", &broker);
        put_text(&mut fields, "name", &name);
        put_text(&mut fields, "currency", &currency);
        put_number(&mut fields, "initial_balance", initial);
        put_number(&mut fields, "last_balance", balance);
        put_number(&mut fields, "last_equity", equity);
        put_number(&mut fields, "server_offset_min", offset);
        fields
    }))
}

fn store_account(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [login, server] = parts.as_slice() else { return Err(bad_key(entity::ACCOUNT, key)) };
    conn.execute(
        "INSERT INTO accounts (login, server, broker, name, currency, initial_balance, last_balance, last_equity,
                               server_offset_min)
         VALUES (?1, ?2, ?3, ?4, COALESCE(NULLIF(?5, ''), 'USD'), ?6, ?7, ?8, ?9)
         ON CONFLICT(login, server) DO UPDATE SET
             broker = excluded.broker, name = excluded.name, currency = excluded.currency,
             initial_balance = excluded.initial_balance, last_balance = excluded.last_balance,
             last_equity = excluded.last_equity, server_offset_min = excluded.server_offset_min",
        rusqlite::params![
            login,
            server,
            text_of(fields, "broker"),
            text_of(fields, "name"),
            text_of(fields, "currency"),
            float_of(fields, "initial_balance"),
            float_of(fields, "last_balance"),
            float_of(fields, "last_equity"),
            small_of::<i32>(fields, "server_offset_min"),
        ],
    )?;
    Ok(())
}

fn load_deal(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [login, server, ticket] = parts.as_slice() else { return Err(bad_key(entity::DEAL, key)) };
    let Ok(ticket) = ticket.parse::<i64>() else { return Err(bad_key(entity::DEAL, key)) };
    let Some(account_id) = account_row(conn, login, server)? else { return Ok(None) };
    let row = conn
        .prepare_cached(&format!("SELECT {DEAL_COLUMNS}, source FROM deals WHERE account_id = ?1 AND ticket = ?2"))?
        .query_row(rusqlite::params![account_id, ticket], |r| Ok((deal_from_row_at(r, 0)?, r.get::<_, String>(20)?)))
        .optional()?;
    row.map(|(deal, source)| {
        let mut object = to_map(&deal)?;
        object.insert("source".to_owned(), Value::String(source));
        Ok(Fields::from([("deal".to_owned(), Value::Object(object))]))
    })
    .transpose()
}

fn store_deal(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [login, server, _] = parts.as_slice() else { return Err(bad_key(entity::DEAL, key)) };
    let Some(record) = fields.get("deal") else { return Ok(()) };
    let deal: Deal = serde_json::from_value(record.clone())?;
    let source = record.get("source").and_then(Value::as_str).unwrap_or("sync");
    let account_id = ensure_account(conn, &join_key(&[login, server]))?;
    write_deal_raw(conn, account_id, &deal, source)
}

fn load_journal(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let Some(trade) = trade_key_of(conn, key)? else { return Ok(None) };
    let Some(entry) = journal_on(conn, &trade)? else { return Ok(None) };
    let mut object = to_map(&entry)?;
    object.remove("playbook_id");
    if let Some(uid) = entry.playbook_id.map(|id| uid_of(conn, "playbooks", id)).transpose()?.flatten() {
        object.insert("playbook".to_owned(), Value::String(uid));
    }
    let mut ticked = Vec::new();
    for id in &entry.checklist {
        if let Some(uid) = uid_of(conn, "checklist", *id)? {
            ticked.push(Value::String(uid));
        }
    }
    object.insert("checklist".to_owned(), Value::Array(ticked));
    Ok(Some(flatten(&object, &JOURNAL)))
}

fn store_journal(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [login, server, ..] = parts.as_slice() else { return Err(bad_key(entity::JOURNAL, key)) };
    let account_id = ensure_account(conn, &join_key(&[login, server]))?;
    let Some(TradeKey { position_id, segment, .. }) = trade_key_of(conn, key)? else {
        return Err(bad_key(entity::JOURNAL, key));
    };
    let mut object = unflatten(fields, &JOURNAL);
    if let Some(uid) = object.remove("playbook").as_ref().and_then(Value::as_str)
        && let Some(id) = id_of(conn, "playbooks", uid)?
    {
        object.insert("playbook_id".to_owned(), json!(id));
    }
    let mut ticked = Vec::new();
    for uid in object.remove("checklist").iter().flat_map(|v| v.as_array().into_iter().flatten()) {
        if let Some(id) = uid.as_str().map(|uid| id_of(conn, "checklist", uid)).transpose()?.flatten() {
            ticked.push(json!(id));
        }
    }
    object.insert("checklist".to_owned(), Value::Array(ticked));
    let entry: JournalEntry = serde_json::from_value(Value::Object(object))?;
    write_journal_raw(conn, &TradeKey { account_id, position_id, segment }, &entry)
}

fn load_rules(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [login, server] = parts.as_slice() else { return Err(bad_key(entity::RULES, key)) };
    let Some(account_id) = account_row(conn, login, server)? else { return Ok(None) };
    let rules = super::rules_on(conn, account_id)?;
    let mut fields = Fields::new();
    put_number(&mut fields, "max_daily_loss", rules.max_daily_loss);
    put_number(&mut fields, "max_daily_trades", rules.max_daily_trades);
    put_number(&mut fields, "max_risk_pct", rules.max_risk_pct);
    put_number(&mut fields, "max_consecutive_losses", rules.max_consecutive_losses);
    put_number(&mut fields, "daily_target", rules.daily_target);
    Ok(Some(fields))
}

fn store_rules(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let account_id = ensure_account(conn, key)?;
    let rules = Rules {
        max_daily_loss: float_of(fields, "max_daily_loss"),
        max_daily_trades: small_of(fields, "max_daily_trades"),
        max_risk_pct: float_of(fields, "max_risk_pct"),
        max_consecutive_losses: small_of(fields, "max_consecutive_losses"),
        daily_target: float_of(fields, "daily_target"),
    };
    super::save_rules_raw(conn, account_id, &rules)
}

fn load_checklist(conn: &Connection, uid: &str) -> Result<Option<Fields>> {
    let row = conn
        .query_row("SELECT text, position, account_id FROM checklist WHERE uid = ?1", [uid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?))
        })
        .optional()?;
    row.map(|(text, position, account_id)| {
        let mut fields = Fields::new();
        put_text(&mut fields, "text", &text);
        fields.insert("position".to_owned(), json!(position));
        // A checklist of an account this database does not know cannot be named on another device.
        if let Some(account) = account_id.and_then(|id| account_key(conn, id).ok()) {
            fields.insert("account".to_owned(), Value::String(account));
        }
        Ok(fields)
    })
    .transpose()
}

fn store_checklist(conn: &Connection, uid: &str, fields: &Fields) -> Result<()> {
    let account_id = account_of_ref(conn, fields.get("account").and_then(Value::as_str))?;
    conn.execute(
        "INSERT INTO checklist (uid, account_id, text, position) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uid) DO UPDATE SET account_id = excluded.account_id, text = excluded.text,
                                        position = excluded.position",
        rusqlite::params![uid, account_id, text_of(fields, "text"), int_of(fields, "position").unwrap_or(0)],
    )?;
    Ok(())
}

fn load_day_note(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [account, day] = parts.as_slice() else { return Err(bad_key(entity::DAY_NOTE, key)) };
    let account_id = if account == ALL_ACCOUNTS {
        Some(0)
    } else {
        let parts = split_key(account);
        let [login, server] = parts.as_slice() else { return Err(bad_key(entity::DAY_NOTE, key)) };
        account_row(conn, login, server)?
    };
    let Some(account_id) = account_id else { return Ok(None) };
    let row = conn
        .query_row(
            "SELECT plan, review, mood, custom_json FROM day_notes WHERE account_id = ?1 AND day = ?2",
            rusqlite::params![account_id, day],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
        )
        .optional()?;
    row.map(|(plan, review, mood, custom)| {
        let mut object = Map::new();
        object.insert("plan".to_owned(), Value::String(plan));
        object.insert("review".to_owned(), Value::String(review));
        object.insert("mood".to_owned(), Value::String(mood));
        object.insert("custom".to_owned(), serde_json::from_str(&custom)?);
        Ok(flatten(&object, &DAY_NOTE))
    })
    .transpose()
}

fn store_day_note(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [account, day] = parts.as_slice() else { return Err(bad_key(entity::DAY_NOTE, key)) };
    let Ok(date) = day.parse::<chrono::NaiveDate>() else { return Err(bad_key(entity::DAY_NOTE, key)) };
    let account_id = account_of_part(conn, account)?;
    let mut object = unflatten(fields, &DAY_NOTE);
    let custom: CustomValues =
        serde_json::from_value(object.remove("custom").unwrap_or_else(|| Value::Object(Map::new())))?;
    let note =
        DayNote { plan: text_of(fields, "plan"), review: text_of(fields, "review"), mood: text_of(fields, "mood") };
    conn.execute(
        "INSERT INTO day_notes (account_id, day, plan, review, mood, custom_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id, day) DO UPDATE SET
             plan = excluded.plan, review = excluded.review, mood = excluded.mood,
             custom_json = excluded.custom_json",
        rusqlite::params![account_id, day, note.plan, note.review, note.mood, serde_json::to_string(&custom)?],
    )?;
    write_index(conn, &day_document(account_id, date, &note, &day_values_on(conn, account_id, date)?))
}

fn load_catalog(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [kind, item] = parts.as_slice() else { return Err(bad_key(entity::CATALOG, key)) };
    let row = conn
        .query_row(
            "SELECT label, color, position, archived FROM catalog_items WHERE kind = ?1 AND key = ?2",
            [kind, item],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)),
        )
        .optional()?;
    Ok(row.map(|(label, color, position, archived)| {
        let mut fields = Fields::new();
        put_text(&mut fields, "label", &label);
        put_text(&mut fields, "color", color.as_deref().unwrap_or_default());
        fields.insert("position".to_owned(), json!(position));
        if archived {
            fields.insert("archived".to_owned(), Value::Bool(true));
        }
        fields
    }))
}

fn store_catalog(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [kind, item] = parts.as_slice() else { return Err(bad_key(entity::CATALOG, key)) };
    let color = Some(text_of(fields, "color")).filter(|color| !color.is_empty());
    conn.execute(
        "INSERT INTO catalog_items (kind, key, label, color, position, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, key) DO UPDATE SET label = excluded.label, color = excluded.color,
                                              position = excluded.position, archived = excluded.archived",
        rusqlite::params![
            kind,
            item,
            text_of(fields, "label"),
            color,
            int_of(fields, "position").unwrap_or(0),
            fields.get("archived").and_then(Value::as_bool).unwrap_or(false)
        ],
    )?;
    Ok(())
}

fn load_field_def(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let parts = split_key(key);
    let [scope, name] = parts.as_slice() else { return Err(bad_key(entity::FIELD_DEF, key)) };
    let json: Option<String> = conn
        .query_row("SELECT def_json FROM field_defs WHERE scope = ?1 AND key = ?2", [scope, name], |r| r.get(0))
        .optional()?;
    json.map(|json| {
        let def: FieldDef = serde_json::from_str(&json)?;
        // The scope and key are the entity's key; the row id is this database's own.
        let mut object = to_map(&def)?;
        for member in ["id", "scope", "key"] {
            object.remove(member);
        }
        Ok(flatten(&object, &FIELD_DEF))
    })
    .transpose()
}

fn store_field_def(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let parts = split_key(key);
    let [scope, name] = parts.as_slice() else { return Err(bad_key(entity::FIELD_DEF, key)) };
    let mut object = unflatten(fields, &FIELD_DEF);
    object.insert("id".to_owned(), json!(0));
    object.insert("scope".to_owned(), Value::String(scope.clone()));
    object.insert("key".to_owned(), Value::String(name.clone()));
    object.entry("label").or_insert_with(|| json!(""));
    object.entry("position").or_insert_with(|| json!(0));
    let mut def: FieldDef = serde_json::from_value(Value::Object(object))?;
    def.id = conn
        .query_row("SELECT id FROM field_defs WHERE scope = ?1 AND key = ?2", [scope, name], |r| r.get(0))
        .optional()?
        .unwrap_or(0);
    save_field_def_raw(conn, &def).map(drop)
}

fn form_layout_setting(scope: &str) -> String {
    format!("{}{scope}", crate::sync::FORM_LAYOUT_PREFIX)
}

fn load_form_layout(conn: &Connection, scope: &str) -> Result<Option<Fields>> {
    let Some(json) = setting_on(conn, &form_layout_setting(scope))? else { return Ok(None) };
    let layout: FormLayout = serde_json::from_str(&json)?;
    let mut object = to_map(&layout)?;
    object.remove("scope");
    Ok(Some(flatten(&object, &FORM_LAYOUT)))
}

fn store_form_layout(conn: &Connection, scope: &str, fields: &Fields) -> Result<()> {
    let mut object = unflatten(fields, &FORM_LAYOUT);
    object.insert("scope".to_owned(), Value::String(scope.to_owned()));
    let layout: FormLayout = serde_json::from_value(Value::Object(object))?;
    set_setting_on(conn, &form_layout_setting(scope), &serde_json::to_string(&layout)?)
}

fn load_playbook(conn: &Connection, uid: &str) -> Result<Option<Fields>> {
    let row = conn
        .query_row("SELECT json, position FROM playbooks WHERE uid = ?1", [uid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .optional()?;
    row.map(|(json, position)| {
        let playbook: Playbook = serde_json::from_str(&json)?;
        let mut object = to_map(&playbook)?;
        object.insert("position".to_owned(), json!(position));
        Ok(flatten(&object, &PLAYBOOK))
    })
    .transpose()
}

fn store_playbook(conn: &Connection, uid: &str, fields: &Fields) -> Result<()> {
    let mut object = unflatten(fields, &PLAYBOOK);
    let position = object.remove("position").and_then(|v| v.as_i64()).unwrap_or(0);
    object.entry("name").or_insert_with(|| json!(""));
    let mut playbook: Playbook = serde_json::from_value(Value::Object(object))?;
    let json = serde_json::to_string(&playbook)?;
    conn.execute(
        "INSERT INTO playbooks (uid, name, json, position, archived) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(uid) DO UPDATE SET name = excluded.name, json = excluded.json,
                                        position = excluded.position, archived = excluded.archived",
        rusqlite::params![uid, playbook.name, json, position, playbook.archived],
    )?;
    playbook.id = conn.query_row("SELECT id FROM playbooks WHERE uid = ?1", [uid], |r| r.get(0))?;
    write_index(conn, &playbook_document(&playbook))
}

fn load_missed(conn: &Connection, uid: &str) -> Result<Option<Fields>> {
    let json: Option<String> =
        conn.query_row("SELECT json FROM missed_trades WHERE uid = ?1", [uid], |r| r.get(0)).optional()?;
    json.map(|json| {
        let missed: MissedTrade = serde_json::from_str(&json)?;
        let mut object = to_map(&missed)?;
        object.remove("account_id");
        object.remove("playbook_id");
        if let Some(id) = missed.account_id.filter(|id| *id != 0) {
            object.insert("account".to_owned(), Value::String(account_key(conn, id)?));
        }
        if let Some(playbook) = missed.playbook_id.map(|id| uid_of(conn, "playbooks", id)).transpose()?.flatten() {
            object.insert("playbook".to_owned(), Value::String(playbook));
        }
        Ok(flatten(&object, &MISSED))
    })
    .transpose()
}

fn store_missed(conn: &Connection, uid: &str, fields: &Fields) -> Result<()> {
    let mut object = unflatten(fields, &MISSED);
    object.entry("symbol").or_insert_with(|| json!(""));
    let account_id = account_of_ref(conn, object.remove("account").as_ref().and_then(Value::as_str))?;
    if let Some(playbook) = object.remove("playbook").as_ref().and_then(Value::as_str)
        && let Some(id) = id_of(conn, "playbooks", playbook)?
    {
        object.insert("playbook_id".to_owned(), json!(id));
    }
    let mut missed: MissedTrade = serde_json::from_value(Value::Object(object))?;
    missed.account_id = account_id;
    conn.execute(
        "INSERT INTO missed_trades (uid, account_id, seen_at, json) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uid) DO UPDATE SET account_id = excluded.account_id, seen_at = excluded.seen_at,
                                        json = excluded.json",
        rusqlite::params![uid, account_id, missed.seen_at, serde_json::to_string(&missed)?],
    )?;
    missed.id = conn.query_row("SELECT id FROM missed_trades WHERE uid = ?1", [uid], |r| r.get(0))?;
    write_index(conn, &missed_document(&missed))
}

fn review_parts(key: &str) -> Result<(String, Period, chrono::NaiveDate)> {
    let parts = split_key(key);
    let [account, period, start] = parts.as_slice() else { return Err(bad_key(entity::REVIEW, key)) };
    let (Some(period), Ok(start)) = (Period::parse(period), start.parse()) else {
        return Err(bad_key(entity::REVIEW, key));
    };
    Ok((account.clone(), period, start))
}

fn load_review(conn: &Connection, key: &str) -> Result<Option<Fields>> {
    let (account, period, start) = review_parts(key)?;
    let account_id = if account == ALL_ACCOUNTS {
        Some(0)
    } else {
        let parts = split_key(&account);
        let [login, server] = parts.as_slice() else { return Err(bad_key(entity::REVIEW, key)) };
        account_row(conn, login, server)?
    };
    let Some(account_id) = account_id else { return Ok(None) };
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM reviews WHERE account_id = ?1 AND period = ?2 AND start = ?3",
            rusqlite::params![account_id, period.name(), start.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|json| {
        let review: PeriodReview = serde_json::from_str(&json)?;
        let mut object = to_map(&review)?;
        for member in ["account_id", "period", "start"] {
            object.remove(member);
        }
        Ok(flatten(&object, &REVIEW))
    })
    .transpose()
}

fn store_review(conn: &Connection, key: &str, fields: &Fields) -> Result<()> {
    let (account, period, start) = review_parts(key)?;
    let account_id = account_of_part(conn, &account)?;
    let mut object = unflatten(fields, &REVIEW);
    object.insert("period".to_owned(), json!(period));
    object.insert("start".to_owned(), json!(start));
    let mut review: PeriodReview = serde_json::from_value(Value::Object(object))?;
    review.account_id = Some(account_id).filter(|id| *id != 0);
    super::save_review_raw(conn, &review)?;
    write_index(conn, &review_document(&review))
}

// --- store and remove --------------------------------------------------------------------

/// Writes `fields` as the whole entity `key` of kind `kind`, creating it when this database has none.
pub(super) fn store(conn: &Connection, kind: &str, key: &str, fields: &Fields) -> Result<()> {
    match kind {
        entity::ACCOUNT => store_account(conn, key, fields),
        entity::DEAL => store_deal(conn, key, fields),
        entity::JOURNAL => store_journal(conn, key, fields),
        entity::RULES => store_rules(conn, key, fields),
        entity::CHECKLIST => store_checklist(conn, key, fields),
        entity::DAY_NOTE => store_day_note(conn, key, fields),
        entity::SETTING => set_setting_on(conn, key, &text_of(fields, "value")),
        entity::CATALOG => store_catalog(conn, key, fields),
        entity::FIELD_DEF => store_field_def(conn, key, fields),
        entity::FORM_LAYOUT => store_form_layout(conn, key, fields),
        entity::PLAYBOOK => store_playbook(conn, key, fields),
        entity::MISSED => store_missed(conn, key, fields),
        entity::REVIEW => store_review(conn, key, fields),
        entity::ATTACHMENT => {
            conn.execute(
                "INSERT INTO sync_attachments (path, sha256) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET sha256 = excluded.sha256",
                [key, &text_of(fields, "sha256")],
            )?;
            Ok(())
        }
        other => Err(DbError::Invalid(format!("loại đồng bộ không biết: {other}"))),
    }
}

/// Removes the entity `key` of kind `kind`. Only the kinds that can be deleted have a removal;
/// for the others nothing happens.
pub(super) fn remove(conn: &Connection, kind: &str, key: &str) -> Result<()> {
    let delete = |sql: &str| -> Result<()> {
        conn.execute(sql, [key])?;
        Ok(())
    };
    match kind {
        entity::CHECKLIST => delete("DELETE FROM checklist WHERE uid = ?1"),
        entity::PLAYBOOK => {
            if let Some(id) = id_of(conn, "playbooks", key)? {
                conn.execute(
                    "DELETE FROM search_index WHERE kind = ?1 AND reference = ?2",
                    [enum_name(&crate::search::DocKind::Playbook)?, id.to_string()],
                )?;
            }
            delete("DELETE FROM playbooks WHERE uid = ?1")
        }
        entity::MISSED => {
            if let Some(id) = id_of(conn, "missed_trades", key)? {
                conn.execute(
                    "DELETE FROM search_index WHERE kind = ?1 AND reference = ?2",
                    [enum_name(&crate::search::DocKind::Missed)?, id.to_string()],
                )?;
            }
            delete("DELETE FROM missed_trades WHERE uid = ?1")
        }
        _ => Ok(()),
    }
}

// --- listing ---------------------------------------------------------------------------------

/// The rows of `sql` as lists of their `count` first columns, each as text.
fn rows(conn: &Connection, sql: &str, count: usize) -> Result<Vec<Vec<String>>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        let mut parts = Vec::with_capacity(count);
        for index in 0..count {
            parts.push(match r.get::<_, rusqlite::types::Value>(index)? {
                rusqlite::types::Value::Text(text) => text,
                rusqlite::types::Value::Integer(n) => n.to_string(),
                other => format!("{other:?}"),
            });
        }
        Ok(parts)
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The keys of every entity of kind `kind` this database holds, in a stable order.
pub(super) fn keys(conn: &Connection, kind: &str) -> Result<Vec<String>> {
    let flat = |sql: &str, count: usize| -> Result<Vec<String>> {
        Ok(rows(conn, sql, count)?
            .iter()
            .map(|parts| join_key(&parts.iter().map(String::as_str).collect::<Vec<_>>()))
            .collect())
    };
    // An account's login and server, nested as one part of a longer key.
    let nested = |login: &str, server: &str| join_key(&[login, server]);
    let mut keys = match kind {
        entity::ACCOUNT => flat("SELECT login, server FROM accounts", 2)?,
        entity::DEAL => {
            flat("SELECT a.login, a.server, d.ticket FROM deals d JOIN accounts a ON a.id = d.account_id", 3)?
        }
        entity::JOURNAL => flat(
            "SELECT a.login, a.server, j.position_id, j.segment FROM journal j JOIN accounts a ON a.id = j.account_id",
            4,
        )?,
        entity::RULES => flat("SELECT a.login, a.server FROM rules r JOIN accounts a ON a.id = r.account_id", 2)?,
        entity::CHECKLIST => flat("SELECT uid FROM checklist", 1)?,
        entity::DAY_NOTE => {
            let own = rows(
                conn,
                "SELECT a.login, a.server, n.day FROM day_notes n JOIN accounts a ON a.id = n.account_id",
                3,
            )?;
            let shared = rows(conn, "SELECT '*', n.day FROM day_notes n WHERE n.account_id = 0", 2)?;
            own.iter()
                .map(|p| day_key(&nested(&p[0], &p[1]), &p[2]))
                .chain(shared.iter().map(|p| day_key(&p[0], &p[1])))
                .collect()
        }
        entity::SETTING => {
            let mut keys = flat("SELECT key FROM settings", 1)?;
            keys.retain(|key| crate::sync::is_synced_setting(key));
            keys
        }
        entity::CATALOG => flat("SELECT kind, key FROM catalog_items", 2)?,
        entity::FIELD_DEF => flat("SELECT scope, key FROM field_defs", 2)?,
        entity::FORM_LAYOUT => flat("SELECT key FROM settings WHERE key LIKE 'form.layout.%'", 1)?
            .into_iter()
            .filter_map(|key| key.strip_prefix(crate::sync::FORM_LAYOUT_PREFIX).map(str::to_owned))
            .collect(),
        entity::PLAYBOOK => flat("SELECT uid FROM playbooks", 1)?,
        entity::MISSED => flat("SELECT uid FROM missed_trades", 1)?,
        entity::REVIEW => {
            let own = rows(
                conn,
                "SELECT a.login, a.server, r.period, r.start FROM reviews r JOIN accounts a ON a.id = r.account_id",
                4,
            )?;
            let shared = rows(conn, "SELECT '*', r.period, r.start FROM reviews r WHERE r.account_id = 0", 3)?;
            own.iter()
                .map(|p| join_key(&[&nested(&p[0], &p[1]), &p[2], &p[3]]))
                .chain(shared.iter().map(|p| join_key(&[&p[0], &p[1], &p[2]])))
                .collect()
        }
        entity::ATTACHMENT => flat("SELECT path FROM sync_attachments", 1)?,
        other => return Err(DbError::Invalid(format!("loại đồng bộ không biết: {other}"))),
    };
    keys.sort();
    Ok(keys)
}

/// The image paths that journal entries, missed trades and setups refer to.
pub(super) fn attachment_paths(conn: &Connection) -> Result<Vec<String>> {
    let mut paths = std::collections::BTreeSet::new();
    let mut gather = |sql: &str, member: &str| -> Result<()> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for json in rows {
            let value: Value = serde_json::from_str(&json?)?;
            for shot in value.get(member).and_then(Value::as_array).into_iter().flatten() {
                if let Some(path) = shot.get("path").and_then(Value::as_str) {
                    paths.insert(path.to_owned());
                }
            }
        }
        Ok(())
    };
    gather("SELECT entry_json FROM journal", "screenshots")?;
    gather("SELECT json FROM missed_trades", "screenshots")?;
    gather("SELECT json FROM playbooks", "examples")?;
    Ok(paths.into_iter().collect())
}

/// The moment a legacy entity was last written, when the database knows it: the stamp its
/// fields get when they are first logged. Only journal entries remember it; the rest are 0,
/// so that any edit made on another device since beats them.
pub(super) fn legacy_time(conn: &Connection, kind: &str, key: &str) -> Result<i64> {
    if kind != entity::JOURNAL {
        return Ok(0);
    }
    let Some(trade) = trade_key_of(conn, key)? else { return Ok(0) };
    Ok(conn
        .query_row(
            "SELECT updated_at FROM journal WHERE account_id = ?1 AND position_id = ?2 AND segment = ?3",
            rusqlite::params![trade.account_id, trade.position_id, trade.segment],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}
