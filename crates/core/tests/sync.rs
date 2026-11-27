//! The merge rules of sync, through the database: two databases that exchange their change logs
//! end up holding the same data, whatever each wrote, in whatever order the changes arrive.

use std::time::Duration;

use tradelens_core::{
    Db,
    catalog::{CatalogItem, CatalogKind},
    db::AccountDeletion,
    model::{AccountInfo, ChecklistItem, Deal, DealBatch, DealEntry, DealKind, JournalEntry, Rules, TradeKey},
    playbook::Playbook,
    sync::{Change, FieldOp, entity},
};

fn db() -> Db {
    Db::in_memory().expect("a database in memory")
}

/// Later writes must be stamped later: the system clock of some machines ticks only every 16 ms.
fn tick() {
    std::thread::sleep(Duration::from_millis(20));
}

/// Every change `from` has made, applied to `to`.
fn send(from: &Db, to: &mut Db) {
    let changes = from.pending_changes(0, usize::MAX).expect("the log reads");
    to.apply_changes(&changes).expect("the changes apply");
}

/// A full round: each database receives the other's log.
fn round(a: &mut Db, b: &mut Db) {
    send(a, b);
    send(b, a);
}

fn assert_same(a: &Db, b: &Db) {
    let (left, right) = (a.portable_state().expect("state"), b.portable_state().expect("state"));
    assert_eq!(left, right, "the two databases hold different synced data");
}

fn account(db: &mut Db) -> i64 {
    db.upsert_account(&AccountInfo {
        login: "5012345".into(),
        server: "Broker-Live".into(),
        name: "Chính".into(),
        ..AccountInfo::default()
    })
    .expect("account")
}

fn trade(account_id: i64) -> TradeKey {
    TradeKey { account_id, position_id: 100, segment: 0 }
}

fn entry(notes: &str) -> JournalEntry {
    JournalEntry { notes: notes.into(), ..JournalEntry::default() }
}

fn journal(db: &Db, account_id: i64) -> JournalEntry {
    db.journal(&trade(account_id)).expect("reads").expect("an entry")
}

fn deal(ticket: i64, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: ticket,
        time_msc: 1_763_000_000_000 + ticket,
        symbol: "XAUUSD".into(),
        kind: DealKind::Buy,
        entry: DealEntry::In,
        volume: 0.1,
        price: 2650.0,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

fn batch(deals: Vec<Deal>) -> DealBatch {
    DealBatch {
        account: AccountInfo { login: "5012345".into(), server: "Broker-Live".into(), ..AccountInfo::default() },
        deals,
        positions: Vec::new(),
    }
}

/// Two devices that have both seen the account and an empty-ish journal of one trade.
fn paired() -> (Db, Db, i64, i64) {
    let (mut a, mut b) = (db(), db());
    let (ia, ib) = (account(&mut a), account(&mut b));
    a.save_journal(&trade(ia), &entry("")).expect("saved");
    round(&mut a, &mut b);
    (a, b, ia, ib)
}

#[test]
fn what_is_applied_is_not_logged_again() {
    let (mut a, mut b) = (db(), db());
    let ia = account(&mut a);
    a.save_journal(&trade(ia), &entry("từ máy A")).expect("saved");
    let before = b.pending_changes(0, usize::MAX).expect("reads").len();
    send(&a, &mut b);
    assert_eq!(b.pending_changes(0, usize::MAX).expect("reads").len(), before, "no echo");
    assert_eq!(journal(&b, b.accounts().expect("accounts")[0].id).notes, "từ máy A");
}

#[test]
fn different_fields_of_the_same_journal_both_survive() {
    let (mut a, mut b, ia, ib) = paired();
    a.save_journal(&trade(ia), &JournalEntry { rating: 4, ..entry("") }).expect("saved");
    tick();
    b.save_journal(&trade(ib), &JournalEntry { notes: "chốt non".into(), ..entry("") }).expect("saved");
    round(&mut a, &mut b);
    for (db, id) in [(&a, ia), (&b, ib)] {
        let merged = journal(db, id);
        assert_eq!((merged.rating, merged.notes.as_str()), (4, "chốt non"));
    }
    assert_same(&a, &b);
}

#[test]
fn the_same_field_takes_the_newest_write_on_both_devices() {
    let (mut a, mut b, ia, ib) = paired();
    a.save_journal(&trade(ia), &entry("viết trên A")).expect("saved");
    tick();
    b.save_journal(&trade(ib), &entry("viết trên B, sau")).expect("saved");
    round(&mut a, &mut b);
    assert_eq!(journal(&a, ia).notes, "viết trên B, sau");
    assert_eq!(journal(&b, ib).notes, "viết trên B, sau");
    assert_same(&a, &b);
}

#[test]
fn a_clock_running_behind_cannot_lose_to_what_it_has_already_seen() {
    let (mut a, mut b, ia, ib) = paired();
    // A writes with a far-future stamp (a clock running ahead); B pulls it, then edits itself.
    let future = Change {
        device_id: "z-device".into(),
        seq: 1,
        at: i64::MAX / 2,
        entity: entity::JOURNAL.into(),
        key: "5012345|Broker-Live|100|0".into(),
        value: Some(serde_json::json!([{"f": "notes", "v": "từ tương lai"}])),
    };
    b.apply_changes(&[future]).expect("applies");
    b.save_journal(&trade(ib), &entry("sửa sau khi đã thấy")).expect("saved");
    send(&b, &mut a);
    assert_eq!(journal(&a, ia).notes, "sửa sau khi đã thấy", "a write after seeing another is later than it");
}

#[test]
fn a_tie_goes_to_the_greater_device_id() {
    let mut db = db();
    let id = account(&mut db);
    let change = |device: &str, notes: &str| Change {
        device_id: device.into(),
        seq: 1,
        at: 5_000,
        entity: entity::JOURNAL.into(),
        key: "5012345|Broker-Live|100|0".into(),
        value: Some(serde_json::json!([{"f": "notes", "v": notes}])),
    };
    let (low, high) = (change("a-device", "thấp"), change("b-device", "cao"));
    let mut other = self::db();
    account(&mut other);
    db.apply_changes(&[low.clone(), high.clone()]).expect("applies");
    other.apply_changes(&[high, low]).expect("applies");
    assert_eq!(journal(&db, id).notes, "cao");
    assert_same(&db, &other);
}

#[test]
fn a_tag_added_on_one_device_and_another_removed_on_the_other_both_apply() {
    let (mut a, mut b, ia, ib) = paired();
    let tagged = JournalEntry { tags: vec!["news".into(), "london".into()], ..entry("") };
    a.save_journal(&trade(ia), &tagged).expect("saved");
    round(&mut a, &mut b);
    assert_eq!(journal(&b, ib).tags, ["london", "news"], "the lists are sorted, the same on every device");

    // Concurrently: A adds "trend", B removes "news" and adds "fomo".
    a.save_journal(
        &trade(ia),
        &JournalEntry { tags: vec!["london".into(), "news".into(), "trend".into()], ..entry("") },
    )
    .expect("saved");
    tick();
    b.save_journal(&trade(ib), &JournalEntry { tags: vec!["fomo".into(), "london".into()], ..entry("") })
        .expect("saved");
    round(&mut a, &mut b);
    for (db, id) in [(&a, ia), (&b, ib)] {
        assert_eq!(journal(db, id).tags, ["fomo", "london", "trend"]);
    }
    assert_same(&a, &b);
}

#[test]
fn a_removal_holds_until_the_element_is_added_again() {
    let (mut a, mut b, ia, ib) = paired();
    let revenge = JournalEntry { mistakes: vec!["revenge".into()], ..entry("") };
    a.save_journal(&trade(ia), &revenge).expect("saved");
    round(&mut a, &mut b);
    // B removes it while A, which has not heard, keeps writing other fields: the removal stands.
    b.save_journal(&trade(ib), &entry("")).expect("saved");
    tick();
    a.save_journal(&trade(ia), &JournalEntry { notes: "vẫn thế".into(), ..revenge.clone() }).expect("saved");
    round(&mut a, &mut b);
    assert!(journal(&a, ia).mistakes.is_empty() && journal(&b, ib).mistakes.is_empty());
    assert_eq!(journal(&b, ib).notes, "vẫn thế", "and the other field written meanwhile is kept");
    // Added again after hearing of the removal, the element is back on both.
    tick();
    a.save_journal(&trade(ia), &JournalEntry { notes: "vẫn thế".into(), ..revenge }).expect("saved");
    round(&mut a, &mut b);
    assert_eq!(journal(&a, ia).mistakes, ["revenge"]);
    assert_eq!(journal(&b, ib).mistakes, ["revenge"]);
    assert_same(&a, &b);
}

#[test]
fn applying_is_idempotent_and_does_not_depend_on_order() {
    let (mut a, mut b, ia, ib) = paired();
    a.save_journal(&trade(ia), &JournalEntry { rating: 3, tags: vec!["x".into()], ..entry("một") }).expect("saved");
    tick();
    a.save_journal(&trade(ia), &JournalEntry { rating: 5, tags: vec!["y".into()], ..entry("hai") }).expect("saved");
    tick();
    b.save_journal(&trade(ib), &JournalEntry { setup: "B".into(), ..entry("ba") }).expect("saved");
    let mut changes = a.pending_changes(0, usize::MAX).expect("reads");
    changes.extend(b.pending_changes(0, usize::MAX).expect("reads"));

    let (mut first, mut second) = (db(), db());
    first.apply_changes(&changes).expect("applies");
    first.apply_changes(&changes).expect("applies twice");
    let mut reversed = changes.clone();
    reversed.reverse();
    second.apply_changes(&reversed).expect("applies");
    assert_same(&first, &second);
    let fresh = first.accounts().expect("accounts")[0].id;
    let merged = journal(&first, fresh);
    assert_eq!((merged.notes.as_str(), merged.rating, merged.setup.as_str()), ("ba", 5, "B"));
    assert_eq!(merged.tags, ["y"]);
}

#[test]
fn deals_imported_on_both_devices_are_stored_once() {
    let (mut a, mut b) = (db(), db());
    a.upsert_batch(&batch(vec![deal(1, 10.0), deal(2, 20.0)]), "html").expect("imports");
    b.upsert_batch(&batch(vec![deal(2, 20.0), deal(3, 30.0)]), "xlsx").expect("imports");
    round(&mut a, &mut b);
    assert_eq!(a.deals(None).expect("deals").len(), 3);
    assert_eq!(b.deals(None).expect("deals").len(), 3);
    assert_same(&a, &b);
    // Running the exchange again changes nothing.
    round(&mut a, &mut b);
    assert_eq!(a.deals(None).expect("deals").len(), 3);
}

#[test]
fn what_one_device_measured_of_a_deal_survives_the_other_devices_report() {
    let (mut a, mut b) = (db(), db());
    let measured = Deal { risk: Some(25.0), sl: 2640.0, ..deal(1, 10.0) };
    a.upsert_batch(&batch(vec![measured]), "python").expect("imports");
    tick();
    b.upsert_batch(&batch(vec![deal(1, 12.0)]), "html").expect("imports");
    round(&mut a, &mut b);
    for db in [&a, &b] {
        let stored = &db.deals(None).expect("deals")[0].1;
        assert_eq!((stored.profit, stored.risk, stored.sl), (12.0, Some(25.0), 2640.0));
    }
    assert_same(&a, &b);
}

#[test]
fn links_between_entities_follow_their_portable_keys_not_row_ids() {
    let (mut a, mut b) = (db(), db());
    // B has other setups and checklist items first, so that the same things get other row ids.
    b.save_playbook(&Playbook { name: "Cũ của B".into(), ..Playbook::default() }, 0).expect("saved");
    b.save_checklist(None, &[ChecklistItem { id: 0, text: "Của B".into() }]).expect("saved");
    let ia = account(&mut a);
    let setup = a.save_playbook(&Playbook { name: "Breakout".into(), ..Playbook::default() }, 0).expect("saved");
    a.save_checklist(None, &[ChecklistItem { id: 0, text: "Có vùng cản".into() }]).expect("saved");
    let item = a.checklist(None).expect("checklist")[0].id;
    a.save_journal(&trade(ia), &JournalEntry { playbook_id: Some(setup), checklist: vec![item], ..entry("") })
        .expect("saved");
    send(&a, &mut b);
    let ib = b.accounts().expect("accounts")[0].id;
    let got = journal(&b, ib);
    let breakout =
        b.playbooks().expect("playbooks").into_iter().find(|p| p.name == "Breakout").expect("the setup came");
    let ticked =
        b.checklist(None).expect("checklist").into_iter().find(|c| c.text == "Có vùng cản").expect("the item came");
    assert_eq!(got.playbook_id, Some(breakout.id));
    assert_eq!(got.checklist, [ticked.id]);
    assert_ne!(setup, breakout.id, "the row ids differ between the devices");
}

#[test]
fn a_setup_deleted_on_one_device_is_deleted_on_the_other() {
    let (mut a, mut b) = (db(), db());
    a.save_playbook(&Playbook { name: "Tạm".into(), ..Playbook::default() }, 0).expect("saved");
    send(&a, &mut b);
    assert_eq!(b.playbooks().expect("playbooks").len(), 1);
    tick();
    let id = a.playbooks().expect("playbooks")[0].id;
    a.delete_playbook(id).expect("deleted");
    send(&a, &mut b);
    assert_eq!(b.playbooks().expect("playbooks").len(), 0, "the setup is gone from B");
    assert_same(&a, &b);
}

#[test]
fn an_edit_newer_than_a_delete_brings_the_setup_back_as_it_was() {
    let (mut a, mut b) = (db(), db());
    let base = Playbook { name: "Breakout".into(), description: "mô tả gốc".into(), ..Playbook::default() };
    a.save_playbook(&base, 0).expect("saved");
    send(&a, &mut b);
    // A deletes it; later, B (which has not heard) edits another field of it.
    let id = a.playbooks().expect("playbooks")[0].id;
    a.delete_playbook(id).expect("deleted");
    tick();
    let mut edited = b.playbooks().expect("playbooks").remove(0);
    edited.invalidation = "đóng nến dưới vùng".into();
    b.save_playbook(&edited, 0).expect("saved");
    round(&mut a, &mut b);
    for db in [&a, &b] {
        let kept = db.playbooks().expect("playbooks");
        assert_eq!(kept.len(), 1, "the newer edit keeps it");
        assert_eq!(kept[0].description, "mô tả gốc", "with what it had when it was deleted");
        assert_eq!(kept[0].invalidation, "đóng nến dưới vùng");
    }
    assert_same(&a, &b);
}

#[test]
fn a_delete_newer_than_an_edit_wins() {
    let (mut a, mut b) = (db(), db());
    a.save_playbook(&Playbook { name: "Breakout".into(), ..Playbook::default() }, 0).expect("saved");
    send(&a, &mut b);
    let mut edited = b.playbooks().expect("playbooks").remove(0);
    edited.description = "sửa trên B".into();
    b.save_playbook(&edited, 0).expect("saved");
    tick();
    let id = a.playbooks().expect("playbooks")[0].id;
    a.delete_playbook(id).expect("deleted");
    round(&mut a, &mut b);
    assert_eq!(a.playbooks().expect("playbooks").len(), 0, "the setup is gone from A");
    assert_eq!(b.playbooks().expect("playbooks").len(), 0, "the setup is gone from B");
    assert_same(&a, &b);
}

#[test]
fn two_setups_of_the_same_name_made_on_two_devices_are_both_kept() {
    let (mut a, mut b) = (db(), db());
    a.save_playbook(&Playbook { name: "Breakout".into(), description: "của A".into(), ..Playbook::default() }, 0)
        .expect("saved");
    b.save_playbook(&Playbook { name: "Breakout".into(), description: "của B".into(), ..Playbook::default() }, 0)
        .expect("saved");
    round(&mut a, &mut b);
    let names = |db: &Db| {
        let mut list: Vec<(String, String)> =
            db.playbooks().expect("playbooks").into_iter().map(|p| (p.name, p.description)).collect();
        list.sort();
        list
    };
    assert_eq!(names(&a).len(), 2);
    assert_eq!(names(&a), names(&b));
    assert_eq!(names(&a)[0].0, names(&a)[1].0, "the same name twice: nothing is renamed behind the trader's back");
    assert_same(&a, &b);
    // By hand, a name is still refused when taken.
    let again = Playbook { name: "Breakout".into(), ..Playbook::default() };
    assert!(a.save_playbook(&again, 0).is_err());
}

#[test]
fn a_database_from_before_sync_gets_uids_and_keeps_its_setups() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("old.db");
    {
        let raw = rusqlite::Connection::open(&path).expect("opens");
        let migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for number in 1..=9 {
            let file = std::fs::read_dir(&migrations)
                .expect("lists")
                .map(|entry| entry.expect("an entry").path())
                .find(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{number:04}_")))
                })
                .expect("a migration file");
            raw.execute_batch(&std::fs::read_to_string(file).expect("reads")).expect("applies");
        }
        raw.pragma_update(None, "user_version", 9).expect("sets");
        raw.execute("INSERT INTO playbooks (name, json, position) VALUES ('Cũ 1', '{\"name\":\"Cũ 1\"}', 0)", [])
            .expect("inserts");
        raw.execute("INSERT INTO playbooks (name, json, position) VALUES ('Cũ 2', '{\"name\":\"Cũ 2\"}', 1)", [])
            .expect("inserts");
        raw.execute("DELETE FROM playbooks WHERE name = 'Cũ 2'", []).expect("deletes");
        raw.execute("INSERT INTO checklist (text) VALUES ('Mục cũ')", []).expect("inserts");
    }
    let mut db = Db::open(&path).expect("migrates");
    let setups = db.playbooks().expect("playbooks");
    assert_eq!(setups.len(), 1);
    assert_eq!(setups[0].name, "Cũ 1");
    assert_eq!(db.checklist(None).expect("checklist").len(), 1);
    let state = db.portable_state().expect("state");
    assert_eq!(state["playbook"].len(), 1, "each setup has a key of its own");
    assert!(state["playbook"].keys().all(|uid| uid.len() == 32));
    let fresh = db.save_playbook(&Playbook { name: "Mới".into(), ..Playbook::default() }, 2).expect("saved");
    assert!(fresh > 2, "an id that was used and deleted is not given again: {fresh}");
}

#[test]
fn catalogs_settings_rules_and_day_notes_merge_per_field() {
    let (mut a, mut b) = (db(), db());
    let (ia, ib) = (account(&mut a), account(&mut b));
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a day");

    let mut mistake = CatalogItem {
        id: 0,
        kind: CatalogKind::Mistake,
        key: "no_journal".into(),
        label: "Không ghi lý do".into(),
        color: None,
        position: 9,
        archived: false,
    };
    mistake.id = a.save_catalog_item(&mistake).expect("saved");
    a.set_setting("time_display", "server").expect("saved");
    a.set_setting("appearance", "dark").expect("a device-local setting");
    a.save_rules(ia, &Rules { max_daily_loss: Some(100.0), ..Rules::default() }).expect("saved");
    a.save_day_note(ia, day, &tradelens_core::DayNote { plan: "kế hoạch A".into(), ..Default::default() })
        .expect("saved");
    tick();
    b.save_day_note(ib, day, &tradelens_core::DayNote { mood: "calm".into(), ..Default::default() }).expect("saved");
    round(&mut a, &mut b);

    assert!(b.catalog(CatalogKind::Mistake).expect("catalog").iter().any(|m| m.key == "no_journal"));
    assert_eq!(b.setting("time_display").expect("setting").as_deref(), Some("server"));
    assert_eq!(b.setting("appearance").expect("setting"), None, "the colour scheme is each device's own");
    assert_eq!(b.rules(ib).expect("rules").max_daily_loss, Some(100.0));
    for (db, id) in [(&a, ia), (&b, ib)] {
        let note = db.day_note(id, day).expect("note");
        assert_eq!((note.plan.as_str(), note.mood.as_str()), ("kế hoạch A", "calm"));
    }
    assert_same(&a, &b);
}

#[test]
fn data_written_before_sync_is_logged_once_and_reaches_a_new_device() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("old.db");
    {
        let mut old = Db::open(&path).expect("opens");
        let ia = account(&mut old);
        old.upsert_batch(&batch(vec![deal(1, 10.0)]), "html").expect("imports");
        old.save_journal(&trade(ia), &JournalEntry { rating: 3, ..entry("ghi từ trước") }).expect("saved");
    }
    // Forget the log, as a database from before sync would have none.
    let raw = rusqlite::Connection::open(&path).expect("opens raw");
    raw.execute_batch("DELETE FROM changes; DELETE FROM sync_clock; DELETE FROM settings WHERE key = 'sync.exported';")
        .expect("forgets");
    drop(raw);

    let mut a = Db::open(&path).expect("opens again");
    assert_eq!(a.pending_changes(0, usize::MAX).expect("reads").len(), 0, "the log starts empty");
    assert!(a.log_existing().expect("logs") >= 3, "the account, the deal and the journal");
    assert_eq!(a.log_existing().expect("logs"), 0, "and the next call finds nothing");
    let mut b = db();
    send(&a, &mut b);
    assert_same(&a, &b);
    let ib = b.accounts().expect("accounts")[0].id;
    assert_eq!((journal(&b, ib).notes.as_str(), journal(&b, ib).rating), ("ghi từ trước", 3));
}

#[test]
fn a_snapshot_restores_as_a_new_device_that_catches_up_on_later_changes() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut a = db();
    let ia = account(&mut a);
    a.save_journal(&trade(ia), &entry("trước snapshot")).expect("saved");
    let snapshot = dir.path().join("snapshot.db");
    a.snapshot_to(&snapshot).expect("snapshot");
    let restored = dir.path().join("restored.db");
    std::fs::copy(&snapshot, &restored).expect("copies");
    let mut r = Db::open(&restored).expect("opens");
    let old = r.restored_as_new_device("Memory").expect("restores");
    assert_eq!(old, a.device_id().expect("id"));
    assert_ne!(r.device_id().expect("id"), old, "a new device id");
    assert!(r.pending_changes(0, usize::MAX).expect("reads").is_empty(), "the old device's log is not ours");
    assert_eq!(
        r.sync_state("Memory").expect("state").pulled.get(&old).copied(),
        Some(2),
        "the account and the journal"
    );
    assert_same(&a, &r);

    // A change made after the snapshot reaches the restored device, and its own writes reach A.
    tick();
    a.save_journal(&trade(ia), &entry("sau snapshot")).expect("saved");
    let after: Vec<Change> = a.pending_changes(2, usize::MAX).expect("reads");
    r.apply_changes(&after).expect("applies");
    let rid = r.accounts().expect("accounts")[0].id;
    assert_eq!(journal(&r, rid).notes, "sau snapshot");
    tick();
    r.save_journal(&trade(rid), &JournalEntry { rating: 2, ..entry("sau snapshot") }).expect("saved");
    send(&r, &mut a);
    assert_eq!(journal(&a, ia).rating, 2);
    assert_same(&a, &r);
}

#[test]
fn a_change_that_cannot_be_stored_is_skipped_without_leaving_half_of_itself_or_stopping_the_rest() {
    let mut db = db();
    let id = account(&mut db);
    let change = |seq, entity: &str, key: &str, value: serde_json::Value| Change {
        device_id: "other".into(),
        seq,
        at: 10 + i64::try_from(seq).expect("small"),
        entity: entity.into(),
        key: key.into(),
        value: Some(value),
    };
    let report = db
        .apply_changes(&[
            // A deal that is not a deal, and a journal key that is not a key.
            change(1, entity::DEAL, "5012345|Broker-Live|7", serde_json::json!([{"f": "deal", "v": {"ticket": "x"}}])),
            change(2, entity::JOURNAL, "nonsense", serde_json::json!([{"f": "notes", "v": "x"}])),
            change(3, entity::JOURNAL, "5012345|Broker-Live|100|0", serde_json::json!([{"f": "notes", "v": "ổn"}])),
        ])
        .expect("does not fail");
    assert_eq!((report.applied, report.stale), (1, 2));
    assert_eq!(journal(&db, id).notes, "ổn");
    assert_eq!(db.deals(None).expect("deals").len(), 0, "nothing of the bad deal stays");
}

#[test]
fn a_change_of_a_kind_this_version_does_not_know_is_skipped() {
    let mut db = db();
    let report = db
        .apply_changes(&[Change {
            device_id: "other".into(),
            seq: 1,
            at: 1,
            entity: "hologram".into(),
            key: "x".into(),
            value: Some(serde_json::to_value(vec![FieldOp { field: "f".into(), value: None }]).expect("json")),
        }])
        .expect("does not fail");
    assert_eq!((report.applied, report.stale), (0, 1));
}

/// A second account that must outlive the deletion of the first.
fn other_account(db: &mut Db) -> i64 {
    db.upsert_account(&AccountInfo { login: "notion".into(), name: "Nhật ký".into(), ..AccountInfo::default() })
        .expect("account")
}

/// An account with deals, a journal entry, rules and a day note, beside another account.
fn account_with_data(db: &mut Db) -> (i64, i64) {
    let id = account(db);
    let other = other_account(db);
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a day");
    db.upsert_batch(&batch(vec![deal(100, 0.0), deal(101, 25.0)]), "mt5").expect("stored");
    db.save_journal(&trade(id), &entry("vào sớm trước tin")).expect("saved");
    db.save_journal(&trade(other), &entry("giữ lại")).expect("saved");
    db.save_rules(id, &Rules { max_daily_loss: Some(100.0), ..Rules::default() }).expect("saved");
    db.save_day_note(id, day, &tradelens_core::DayNote { plan: "kế hoạch riêng".into(), ..Default::default() })
        .expect("saved");
    db.save_checklist(Some(id), &[ChecklistItem { id: 0, text: "Kiểm tra tin".into() }]).expect("saved");
    let missed = tradelens_core::missed::MissedTrade {
        id: 0,
        account_id: Some(id),
        symbol: "XAUUSD".into(),
        side: tradelens_core::model::Side::Long,
        seen_at: 1_763_000_000_000,
        playbook_id: None,
        setup_grade: None,
        reason: None,
        entry: None,
        stop_loss: None,
        take_profit: None,
        outcome_price: None,
        notes: "bỏ lỡ phiên Âu".into(),
        screenshots: Vec::new(),
        custom: tradelens_core::catalog::CustomValues::new(),
    };
    db.save_missed_trade(&missed).expect("saved");
    (id, other)
}

/// The logins held, sorted: an account made again takes a new row id.
fn logins(db: &Db) -> Vec<String> {
    let mut logins: Vec<String> = db.accounts().expect("accounts").into_iter().map(|a| a.login).collect();
    logins.sort();
    logins
}

#[test]
fn a_deleted_account_goes_with_everything_it_owns_here_and_on_the_other_device() {
    let (mut a, mut b) = (db(), db());
    let (id, other) = account_with_data(&mut a);
    send(&a, &mut b);
    assert_eq!(logins(&b), ["5012345", "notion"]);
    tick();

    // No other account holds its deals: its journal entry, day note, checklist item and missed trade go
    // with it.
    let deletion = AccountDeletion { twin: None, moved: 0, deleted: 4 };
    assert_eq!(a.account_deletion(id).expect("planned"), deletion);
    assert_eq!(a.delete_account(id).expect("deleted"), deletion);
    assert_eq!(logins(&a), ["notion"]);
    assert!(a.deals(None).expect("deals").is_empty(), "its deals are gone");
    assert!(a.search("sớm", 10).expect("search").is_empty(), "its journal is no longer found");
    assert_eq!(journal(&a, other).notes, "giữ lại", "the other account keeps its journal");
    assert!(a.delete_account(id).is_err(), "an account that is gone cannot be deleted again");

    send(&a, &mut b);
    assert_eq!(logins(&b), ["notion"], "the other device deletes it too");
    assert_eq!(b.deals(None).expect("deals").len(), 0, "its deals are gone there too");
    assert_eq!(b.search("sớm", 10).expect("search").len(), 0, "its journal is not found there");
    assert!(b.search("riêng", 10).expect("search").is_empty(), "nor is its day note");
    for db in [&a, &b] {
        assert!(db.search("phiên", 10).expect("search").is_empty(), "its missed trades are gone");
        assert_eq!(db.missed_trades(0, i64::MAX).expect("missed").len(), 0, "no missed trade is left");
        let checklist = db.checklist(None).expect("checklist");
        assert!(checklist.iter().all(|item| item.text != "Kiểm tra tin"), "its checklist is gone");
    }
    assert_same(&a, &b);
}

#[test]
fn a_device_that_learns_of_an_account_after_it_was_deleted_never_makes_it() {
    let mut a = db();
    let (id, _) = account_with_data(&mut a);
    tick();
    a.delete_account(id).expect("deleted");
    // A new device receives the whole log at once: the account's creation, its data, its deletion.
    let mut fresh = db();
    send(&a, &mut fresh);
    assert_eq!(logins(&fresh), ["notion"], "no bare account is made for its older data");
    assert_eq!(fresh.deals(None).expect("deals").len(), 0, "nor any of its deals");
    assert_same(&a, &fresh);
}

#[test]
fn an_account_stored_again_after_its_deletion_comes_back_with_only_its_new_data() {
    let (mut a, mut b) = (db(), db());
    let (id, _) = account_with_data(&mut a);
    send(&a, &mut b);
    tick();
    a.delete_account(id).expect("deleted");
    tick();
    // The terminal reports the account again: it is stored anew, with what it reads now.
    a.upsert_batch(&batch(vec![deal(102, 10.0)]), "mt5").expect("stored again");
    let mut fresh = db();
    for device in [&mut b, &mut fresh] {
        send(&a, device);
        assert_eq!(logins(device), ["5012345", "notion"]);
        let tickets: Vec<i64> = device.deals(None).expect("deals").iter().map(|(_, d)| d.ticket).collect();
        assert_eq!(tickets, [102], "the deals deleted with it stay deleted");
    }
    assert_same(&a, &b);
    assert_same(&a, &fresh);
}

/// A copy of the account under another login on its server, holding the same deals: what a terminal
/// still signing in stored under login `0` before such a login was refused.
fn copy_under_another_login(db: &mut Db) -> i64 {
    let copy = DealBatch {
        account: AccountInfo { login: "5099999".into(), server: "Broker-Live".into(), ..AccountInfo::default() },
        ..batch(vec![deal(100, 0.0), deal(101, 25.0)])
    };
    db.upsert_batch(&copy, "mt5").expect("stored").account_id
}

fn account_id_of(db: &Db, login: &str) -> i64 {
    db.accounts().expect("accounts").into_iter().find(|a| a.login == login).expect("the account is held").id
}

#[test]
fn deleting_a_copy_under_another_login_moves_its_notes_to_the_account_with_the_same_deals() {
    let (mut a, mut b) = (db(), db());
    let real = account(&mut a);
    a.upsert_batch(&batch(vec![deal(100, 0.0), deal(101, 25.0)]), "mt5").expect("stored");
    let copy = copy_under_another_login(&mut a);
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a day");
    // Written on the copy: a trade the account has no journal of, a trade it has one of, a day note.
    let other_trade = |account_id| TradeKey { position_id: 101, ..trade(account_id) };
    a.save_journal(&trade(copy), &entry("ghi trên bản sao")).expect("saved");
    a.save_journal(&other_trade(copy), &entry("bản sao của 101")).expect("saved");
    a.save_journal(&other_trade(real), &entry("của tài khoản")).expect("saved");
    let note = tradelens_core::DayNote { plan: "kế hoạch trên bản sao".into(), ..Default::default() };
    a.save_day_note(copy, day, &note).expect("saved");
    send(&a, &mut b);
    tick();

    let planned = a.account_deletion(copy).expect("planned");
    assert_eq!(planned, AccountDeletion { twin: Some(real), moved: 2, deleted: 1 });
    assert_eq!(a.delete_account(copy).expect("deleted"), planned, "it does what it said it would");
    send(&a, &mut b);
    for device in [&a, &b] {
        assert_eq!(logins(device), ["5012345"]);
        let real = account_id_of(device, "5012345");
        assert_eq!(journal(device, real).notes, "ghi trên bản sao", "the copy's note moved to the account");
        let kept = device.journal(&other_trade(real)).expect("read").expect("a journal");
        assert_eq!(kept.notes, "của tài khoản", "the account's own note is kept");
        assert_eq!(device.day_note(real, day).expect("read").plan, "kế hoạch trên bản sao");
        assert_eq!(device.deals(None).expect("deals").len(), 2, "the account keeps its deals");
        assert_eq!(device.search("bản sao", 10).expect("search").len(), 2, "what moved is found under it");
    }
    assert_same(&a, &b);
}

#[test]
fn a_journal_edited_after_its_account_was_deleted_comes_back_whole_on_every_device() {
    let (mut a, mut b, mut c) = (db(), db(), db());
    let id = account(&mut a);
    a.upsert_batch(&batch(vec![deal(100, 0.0)]), "mt5").expect("stored");
    let written = JournalEntry { setup: "phá đỉnh".into(), rating: 4, ..entry("vào đúng kế hoạch") };
    a.save_journal(&trade(id), &written).expect("saved");
    send(&a, &mut b);
    send(&a, &mut c);
    tick();
    a.delete_account(id).expect("deleted");
    // C learns of the deletion first; B, still connected to the account, has not and goes on.
    send(&a, &mut c);
    tick();
    let on_b = account_id_of(&b, "5012345");
    b.upsert_account(&AccountInfo {
        login: "5012345".into(),
        server: "Broker-Live".into(),
        equity: Some(1_010.0),
        ..AccountInfo::default()
    })
    .expect("stored");
    b.save_journal(&trade(on_b), &JournalEntry { notes: "thêm sau".into(), ..written.clone() }).expect("saved");
    for device in [&mut a, &mut c] {
        send(&b, device);
    }
    send(&a, &mut b);
    for device in [&a, &b, &c] {
        let back = journal(device, account_id_of(device, "5012345"));
        assert_eq!((back.notes.as_str(), back.setup.as_str(), back.rating), ("thêm sau", "phá đỉnh", 4));
    }
    assert_same(&a, &b);
    assert_same(&a, &c);
}

#[test]
fn login_0_is_never_stored_from_a_terminal_or_from_another_device() {
    let mut db = db();
    let signing_in = DealBatch {
        account: AccountInfo { login: " 0 ".into(), server: "Broker-Live".into(), ..AccountInfo::default() },
        ..batch(vec![deal(100, 0.0)])
    };
    assert!(db.upsert_batch(&signing_in, "mt5").is_err(), "what a terminal still signing in reads is refused");
    // A device on an older version sends its copy under login 0: the account and what it holds.
    let change = |seq, entity: &str, key: &str, value: serde_json::Value| Change {
        device_id: "older".into(),
        seq,
        at: 10 + i64::try_from(seq).expect("small"),
        entity: entity.into(),
        key: key.into(),
        value: Some(value),
    };
    let report = db
        .apply_changes(&[
            change(1, entity::ACCOUNT, "0|Broker-Live", serde_json::json!([{"f": "name", "v": "Standard"}])),
            change(2, entity::JOURNAL, "0|Broker-Live|100|0", serde_json::json!([{"f": "notes", "v": "bản sao"}])),
        ])
        .expect("does not fail");
    assert_eq!((report.applied, report.stale), (0, 2), "both are skipped");
    assert!(db.accounts().expect("accounts").is_empty(), "no account 0 is made");
    assert!(db.deals(None).expect("deals").is_empty(), "nor any deal of it");
    let real = account(&mut db);
    assert!(db.accounts().expect("accounts").iter().all(|a| a.id == real), "a real login is stored as before");
}

#[test]
fn typed_trades_of_two_accounts_do_not_make_them_twins() {
    let mut db = db();
    let typed = tradelens_core::model::ManualTrade {
        symbol: "EURUSD".into(),
        side: tradelens_core::model::Side::Long,
        volume: Some(0.5),
        open_time: 1_763_000_000_000,
        open_price: Some(1.1),
        close_time: 1_763_000_600_000,
        close_price: Some(1.104),
        date_only: false,
        sl: 0.0,
        tp: 0.0,
        profit: Some(20.0),
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    };
    let mut ids = Vec::new();
    for login in ["111111", "222222"] {
        let info = AccountInfo { login: login.into(), server: "Broker-Live".into(), ..AccountInfo::default() };
        let id = db.upsert_account(&info).expect("account");
        // Typed trades are numbered per account (-1, -2…): both accounts hold the same tickets.
        let key = db.add_manual_trade(id, &typed).expect("typed");
        db.save_journal(&key, &entry("của riêng tài khoản này")).expect("saved");
        ids.push(id);
    }
    let deletion = db.account_deletion(ids[0]).expect("planned");
    assert_eq!(deletion, AccountDeletion { twin: None, moved: 0, deleted: 1 }, "nothing moves to an unrelated account");
    db.delete_account(ids[0]).expect("deleted");
    let left = db.journals(None).expect("journals");
    assert_eq!(left.len(), 1, "the other account keeps only its own journal");
    assert_eq!(left[0].0.account_id, ids[1]);
}

#[test]
fn a_copys_missed_trades_and_own_checklist_move_with_its_journal() {
    let (mut a, mut b) = (db(), db());
    let real = account(&mut a);
    a.upsert_batch(&batch(vec![deal(100, 0.0), deal(101, 25.0)]), "mt5").expect("stored");
    let copy = copy_under_another_login(&mut a);
    a.save_checklist(Some(copy), &[ChecklistItem { id: 0, text: "Đợi nến đóng".into() }]).expect("saved");
    let item =
        a.checklist(Some(copy)).expect("checklist").into_iter().find(|i| i.text == "Đợi nến đóng").expect("item");
    let ticked = JournalEntry { checklist: vec![item.id], ..entry("đã đợi nến") };
    a.save_journal(&trade(copy), &ticked).expect("saved");
    let missed = tradelens_core::missed::MissedTrade {
        id: 0,
        account_id: Some(copy),
        symbol: "XAUUSD".into(),
        side: tradelens_core::model::Side::Long,
        seen_at: 1_763_000_000_000,
        playbook_id: None,
        setup_grade: None,
        reason: None,
        entry: None,
        stop_loss: None,
        take_profit: None,
        outcome_price: None,
        notes: "bỏ lỡ trên bản sao".into(),
        screenshots: Vec::new(),
        custom: tradelens_core::catalog::CustomValues::new(),
    };
    a.save_missed_trade(&missed).expect("saved");
    send(&a, &mut b);
    tick();

    let deletion = a.delete_account(copy).expect("deleted");
    assert_eq!(deletion, AccountDeletion { twin: Some(real), moved: 3, deleted: 0 });
    send(&a, &mut b);
    for device in [&a, &b] {
        let real = account_id_of(device, "5012345");
        let missed = device.missed_trades(0, i64::MAX).expect("missed");
        assert_eq!(missed.len(), 1, "the missed trade is kept");
        assert_eq!(missed[0].account_id, Some(real), "under the account");
        let own = device.checklist(Some(real)).expect("checklist");
        let item = own.iter().find(|i| i.text == "Đợi nến đóng").expect("the item moved to the account");
        assert_eq!(journal(device, real).checklist, [item.id], "the moved journal keeps its tick");
    }
    assert_same(&a, &b);
}
