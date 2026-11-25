//! The v0.3 criterion: two machines edit at the same time, and after they sync nothing is lost.
//!
//! Each "machine" is its own database file; all of them share one [`MemoryDrive`].

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tradelens_cloud::CloudError;
use tradelens_cloud::drive::{CloudDrive, RemoteFile};
use tradelens_cloud::memory::{Link, MemoryDrive};
use tradelens_cloud::sync::{SyncEngine, SyncReport};
use tradelens_core::{
    Db,
    attachments::{Attachment, Source},
    catalog::{CatalogItem, CatalogKind},
    model::{
        AccountInfo, ChecklistItem, Deal, DealBatch, DealEntry, DealKind, JournalEntry, ManualTrade, Rules, Side,
        TradeKey,
    },
    playbook::Playbook,
};

const PROVIDER: &str = "Memory";

/// A machine: a database in its own folder (the image folder is next to it).
struct Machine {
    // Fields drop in order: the database closes before its folder is removed. The other way
    // round, Windows cannot delete the open file and the folder is left in the temp directory.
    db: Db,
    dir: TempDir,
    name: &'static str,
}

impl Machine {
    fn new(name: &'static str) -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let db = Db::open(dir.path().join("tradelens.db")).expect("opens");
        Self { db, dir, name }
    }

    fn images(&self) -> std::path::PathBuf {
        self.dir.path().join("attachments")
    }

    fn engine<'a>(&self, drive: &'a MemoryDrive) -> SyncEngine<'a> {
        SyncEngine::new(drive, PROVIDER, self.name).with_attachments(self.images())
    }

    fn sync(&mut self, drive: &MemoryDrive) -> SyncReport {
        let engine = self.engine(drive);
        engine.sync(&mut self.db).unwrap_or_else(|e| panic!("{} fails to sync: {e}", self.name))
    }

    fn account(&self) -> i64 {
        self.db.accounts().expect("accounts")[0].id
    }

    fn trade(&self, position_id: i64) -> TradeKey {
        TradeKey { account_id: self.account(), position_id, segment: 0 }
    }

    fn journal(&self, position_id: i64) -> JournalEntry {
        self.db.journal(&self.trade(position_id)).expect("reads").unwrap_or_default()
    }

    fn write_journal(&mut self, position_id: i64, change: impl FnOnce(&mut JournalEntry)) {
        let key = self.trade(position_id);
        let mut entry = self.db.journal(&key).expect("reads").unwrap_or_default();
        change(&mut entry);
        self.db.save_journal(&key, &entry).expect("saved");
    }

    fn playbook(&self, name: &str) -> Option<Playbook> {
        self.db.playbooks().expect("playbooks").into_iter().find(|p| p.name == name)
    }
}

/// Later writes must be stamped later: the system clock of some machines ticks only every 16 ms.
fn tick() {
    std::thread::sleep(Duration::from_millis(20));
}

fn info() -> AccountInfo {
    AccountInfo {
        login: "5012345".into(),
        server: "Broker-Live".into(),
        name: "Chính".into(),
        ..AccountInfo::default()
    }
}

fn deal(ticket: i64, position: i64, entry: DealEntry, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: 1_763_000_000_000 + ticket * 1000,
        symbol: "XAUUSD".into(),
        kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
        entry,
        volume: 0.1,
        price: 2650.0 + f64::from(i32::try_from(ticket).expect("small")),
        commission: -0.7,
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
    DealBatch { account: info(), deals, positions: Vec::new() }
}

/// Syncs the machines in turn until a full pass has nothing left to move.
fn settle(machines: &mut [&mut Machine], drive: &MemoryDrive) {
    for pass in 0..6 {
        let mut moved = 0;
        for machine in machines.iter_mut() {
            let report = machine.sync(drive);
            moved += report.pushed + report.pulled;
        }
        if moved == 0 {
            return;
        }
        assert!(pass < 5, "the machines keep finding things to exchange");
    }
}

fn assert_same(a: &Machine, b: &Machine) {
    let (left, right) = (a.db.portable_state().expect("state"), b.db.portable_state().expect("state"));
    for (kind, entities) in &left {
        assert_eq!(Some(entities), right.get(kind), "{} and {} differ in {kind}", a.name, b.name);
    }
    assert_eq!(left, right, "{} and {} differ", a.name, b.name);
}

/// Machine A has an account, two trades with deals, a setup and a checklist; B has synced them.
fn started() -> (MemoryDrive, Machine, Machine) {
    let drive = MemoryDrive::new();
    let (mut a, mut b) = (Machine::new("A"), Machine::new("B"));
    a.db.upsert_batch(
        &batch(vec![
            deal(2, 100, DealEntry::In, 0.0),
            deal(3, 100, DealEntry::Out, 50.0),
            deal(4, 200, DealEntry::In, 0.0),
            deal(5, 200, DealEntry::Out, -20.0),
        ]),
        "html",
    )
    .expect("imports");
    a.db.save_playbook(&Playbook { name: "Breakout phiên Á".into(), ..Playbook::default() }, 0).expect("saved");
    a.db.save_checklist(None, &[ChecklistItem { id: 0, text: "Có vùng cản rõ".into() }]).expect("saved");
    a.write_journal(100, |entry| entry.notes = "lệnh đầu".into());
    settle(&mut [&mut a, &mut b], &drive);
    assert_same(&a, &b);
    (drive, a, b)
}

#[test]
fn a_first_sync_brings_everything_to_the_other_machine() {
    let (_, a, b) = started();
    assert_eq!(b.db.accounts().expect("accounts").len(), 1);
    assert_eq!(b.db.deals(None).expect("deals").len(), 4);
    assert_eq!(b.journal(100).notes, "lệnh đầu");
    assert!(b.playbook("Breakout phiên Á").is_some());
    assert_eq!(b.db.checklist(None).expect("checklist").len(), 1);
    assert_ne!(a.db.device_id().expect("id"), b.db.device_id().expect("id"));
}

#[test]
fn concurrent_edits_of_different_fields_of_one_journal_both_survive() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| {
        entry.rating = 4;
        entry.emotion_after = "calm".into();
    });
    tick();
    b.write_journal(100, |entry| {
        entry.setup = "Breakout".into();
        entry.review_later = true;
    });
    settle(&mut [&mut a, &mut b], &drive);
    for machine in [&a, &b] {
        let merged = machine.journal(100);
        assert_eq!(merged.rating, 4, "{}", machine.name);
        assert_eq!(merged.emotion_after, "calm");
        assert_eq!(merged.setup, "Breakout");
        assert!(merged.review_later);
        assert_eq!(merged.notes, "lệnh đầu", "what neither touched stays");
    }
    assert_same(&a, &b);
}

#[test]
fn the_same_field_edited_on_both_machines_takes_the_newest_write() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| entry.notes = "ghi chú trên A".into());
    tick();
    b.write_journal(100, |entry| entry.notes = "ghi chú trên B, muộn hơn".into());
    let report_a = a.sync(&drive);
    let report_b = b.sync(&drive);
    assert_eq!(report_a.pushed, 1);
    assert!(report_b.conflicts_resolved >= 1, "B met a different value of the field A wrote");
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(a.journal(100).notes, "ghi chú trên B, muộn hơn");
    assert_eq!(b.journal(100).notes, "ghi chú trên B, muộn hơn");
    assert_same(&a, &b);
}

#[test]
fn a_tag_added_on_one_machine_and_another_removed_on_the_other_both_apply() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| entry.tags = vec!["london_open".into(), "news".into()]);
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(b.journal(100).tags, ["london_open", "news"]);

    a.write_journal(100, |entry| entry.tags.push("trend_h4".into()));
    tick();
    b.write_journal(100, |entry| {
        entry.tags.retain(|tag| tag != "news");
        entry.mistakes.push("moved_sl".into());
    });
    settle(&mut [&mut a, &mut b], &drive);
    for machine in [&a, &b] {
        let merged = machine.journal(100);
        assert_eq!(merged.tags, ["london_open", "trend_h4"], "{}: B's removal and A's addition", machine.name);
        assert_eq!(merged.mistakes, ["moved_sl"]);
    }
    assert_same(&a, &b);
}

#[test]
fn checklist_confirmations_added_on_both_machines_are_both_kept() {
    let (drive, mut a, mut b) = started();
    a.db.save_checklist(
        None,
        &[a.db.checklist(None).expect("list")[0].clone(), ChecklistItem { id: 0, text: "Đã xem tin".into() }],
    )
    .expect("saved");
    settle(&mut [&mut a, &mut b], &drive);
    let items = |m: &Machine| m.db.checklist(None).expect("checklist");
    let first_a = items(&a)[0].id;
    let second_b = items(&b)[1].id;
    a.write_journal(100, |entry| entry.checklist = vec![first_a]);
    tick();
    b.write_journal(100, |entry| entry.checklist = vec![second_b]);
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(a.journal(100).checklist.len(), 2, "both confirmations");
    assert_eq!(b.journal(100).checklist.len(), 2);
    let texts = |m: &Machine| {
        let ids = m.journal(100).checklist;
        let mut texts: Vec<String> =
            items(m).into_iter().filter(|item| ids.contains(&item.id)).map(|item| item.text).collect();
        texts.sort();
        texts
    };
    assert_eq!(texts(&a), ["Có vùng cản rõ", "Đã xem tin"]);
    assert_eq!(texts(&b), texts(&a));
    assert_same(&a, &b);
}

#[test]
fn deals_imported_on_both_machines_are_stored_once() {
    let (drive, mut a, mut b) = started();
    // The same report is imported on both, and each also imports a deal the other lacks.
    let shared = deal(6, 300, DealEntry::In, 0.0);
    a.db.upsert_batch(&batch(vec![shared.clone(), deal(7, 300, DealEntry::Out, 12.0)]), "html").expect("imports");
    b.db.upsert_batch(&batch(vec![shared, deal(8, 400, DealEntry::In, 0.0)]), "xlsx").expect("imports");
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(a.db.deals(None).expect("deals").len(), 7);
    assert_eq!(b.db.deals(None).expect("deals").len(), 7);
    assert_eq!(a.db.accounts().expect("accounts").len(), 1, "one account, not one per machine");
    assert_same(&a, &b);
}

#[test]
fn a_setup_deleted_on_one_machine_while_edited_on_the_other_follows_the_documented_rule() {
    let (drive, mut a, mut b) = started();
    // The edit is newer than the deletion: the setup stays, with the edit and what it had.
    a.db.save_playbook(&Playbook { name: "Pullback EMA".into(), description: "gốc".into(), ..Playbook::default() }, 1)
        .expect("saved");
    settle(&mut [&mut a, &mut b], &drive);
    let id = a.playbook("Pullback EMA").expect("exists").id;
    a.db.delete_playbook(id).expect("deleted");
    tick();
    let mut edited = b.playbook("Pullback EMA").expect("exists");
    edited.invalidation = "đóng nến dưới EMA".into();
    b.db.save_playbook(&edited, 1).expect("saved");
    settle(&mut [&mut a, &mut b], &drive);
    for machine in [&a, &b] {
        let kept = machine.playbook("Pullback EMA").unwrap_or_else(|| panic!("{} lost the setup", machine.name));
        assert_eq!((kept.description.as_str(), kept.invalidation.as_str()), ("gốc", "đóng nến dưới EMA"));
    }
    assert_same(&a, &b);

    // The deletion is newer than the edit: the setup is gone from both.
    let mut edited = b.playbook("Pullback EMA").expect("exists");
    edited.description = "sửa trước khi xoá".into();
    b.db.save_playbook(&edited, 1).expect("saved");
    tick();
    let id = a.playbook("Pullback EMA").expect("exists").id;
    a.db.delete_playbook(id).expect("deleted");
    settle(&mut [&mut a, &mut b], &drive);
    assert!(a.playbook("Pullback EMA").is_none() && b.playbook("Pullback EMA").is_none());
    assert_same(&a, &b);
}

/// A trade the trader typed.
fn typed() -> ManualTrade {
    ManualTrade {
        symbol: "EURUSD".into(),
        side: Side::Long,
        volume: Some(0.5),
        open_time: 1_763_000_000_000,
        open_price: Some(1.0850),
        close_time: 1_763_003_600_000,
        close_price: Some(1.0905),
        date_only: false,
        sl: 1.0830,
        tp: 0.0,
        profit: Some(275.0),
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    }
}

/// How many deals the machine holds of the position.
fn deals_of(machine: &Machine, position_id: i64) -> usize {
    machine.db.deals(None).expect("deals").iter().filter(|(_, d)| d.position_id == position_id).count()
}

#[test]
fn a_trade_deleted_on_one_machine_is_deleted_on_the_other_with_its_journal() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| entry.screenshots = vec![shot("images/x.png", "H1")]);
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(b.journal(100).notes, "lệnh đầu");

    let deleted = a.db.delete_trade(&a.trade(100)).expect("deleted");
    assert_eq!((deleted.trades, deleted.unused_images), (1, vec!["images/x.png".to_owned()]));
    settle(&mut [&mut a, &mut b], &drive);

    for machine in [&a, &b] {
        assert_eq!(deals_of(machine, 100), 0, "{} still has the deals", machine.name);
        assert_eq!(machine.db.journal(&machine.trade(100)).expect("reads"), None, "{}", machine.name);
        assert_eq!(deals_of(machine, 200), 2, "the other trade stays on {}", machine.name);
    }
    assert!(b.db.search("lệnh đầu", 5).expect("searches").is_empty(), "no search entry on B either");
    assert_same(&a, &b);
}

#[test]
fn a_trade_edited_on_the_other_machine_before_it_was_deleted_is_deleted_everywhere() {
    let (drive, mut a, mut b) = started();
    let key = a.db.add_manual_trade(a.account(), &typed()).expect("typed");
    a.db.save_journal(&key, &JournalEntry { notes: "gốc".into(), ..JournalEntry::default() }).expect("journal");
    settle(&mut [&mut a, &mut b], &drive);

    // B edits the facts and the journal; later, without having seen that, A deletes the trade.
    let on_b = b.trade(key.position_id);
    b.db.update_manual_trade(&on_b, &ManualTrade { open_price: Some(1.0860), close_price: Some(1.0900), ..typed() })
        .expect("edited");
    b.write_journal(key.position_id, |entry| entry.notes = "sửa ở B".into());
    tick();
    a.db.delete_trade(&key).expect("deleted");
    settle(&mut [&mut a, &mut b], &drive);

    for machine in [&a, &b] {
        assert_eq!(deals_of(machine, key.position_id), 0, "{} kept the deals", machine.name);
        assert_eq!(machine.db.journal(&machine.trade(key.position_id)).expect("reads"), None, "{}", machine.name);
    }
    assert_same(&a, &b);
}

#[test]
fn an_edit_made_after_the_deletion_brings_the_trade_back_with_the_edit() {
    let (drive, mut a, mut b) = started();
    let key = a.db.add_manual_trade(a.account(), &typed()).expect("typed");
    a.db.save_journal(&key, &JournalEntry { notes: "gốc".into(), ..JournalEntry::default() }).expect("journal");
    settle(&mut [&mut a, &mut b], &drive);

    a.db.delete_trade(&key).expect("deleted");
    tick();
    let on_b = b.trade(key.position_id);
    let edited = ManualTrade { open_price: Some(1.0860), close_price: Some(1.0900), ..typed() };
    b.db.update_manual_trade(&on_b, &edited).expect("edited");
    b.write_journal(key.position_id, |entry| entry.setup = "sau khi xoá".into());
    settle(&mut [&mut a, &mut b], &drive);

    for machine in [&a, &b] {
        let back = machine.trade(key.position_id);
        assert_eq!(machine.db.manual_trade(&back).expect("reads"), Some(edited.clone()), "{}", machine.name);
        let journal = machine.journal(key.position_id);
        assert_eq!((journal.notes.as_str(), journal.setup.as_str()), ("gốc", "sau khi xoá"), "{}", machine.name);
    }
    assert_same(&a, &b);
}

#[test]
fn a_report_imported_again_after_the_deletion_brings_the_trade_back_on_both_machines() {
    let (drive, mut a, mut b) = started();
    a.db.delete_trade(&a.trade(200)).expect("deleted");
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!(deals_of(&b, 200), 0);

    tick();
    b.db.upsert_batch(&batch(vec![deal(4, 200, DealEntry::In, 0.0), deal(5, 200, DealEntry::Out, -20.0)]), "html")
        .expect("imports again");
    settle(&mut [&mut a, &mut b], &drive);
    assert_eq!((deals_of(&a, 200), deals_of(&b, 200)), (2, 2));
    assert_same(&a, &b);
}

#[test]
fn three_machines_editing_at_once_all_end_up_equal() {
    let (drive, mut a, mut b) = started();
    let mut c = Machine::new("C");
    settle(&mut [&mut a, &mut b, &mut c], &drive);
    assert_same(&a, &c);

    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a day");
    a.write_journal(200, |entry| entry.notes = "A viết".into());
    a.db.set_setting("time_display", "server").expect("saved");
    a.db.save_rules(a.account(), &Rules { max_daily_loss: Some(150.0), ..Rules::default() }).expect("saved");
    tick();
    b.write_journal(200, |entry| {
        entry.notes = "B viết sau".into();
        entry.tags = vec!["b".into()];
    });
    b.db.save_catalog_item(&CatalogItem {
        id: 0,
        kind: CatalogKind::Tag,
        key: "scalp".into(),
        label: "Scalp".into(),
        color: None,
        position: 20,
        archived: false,
    })
    .expect("saved");
    tick();
    c.write_journal(200, |entry| {
        entry.tags = vec!["c".into()];
        entry.rating = 2;
    });
    c.db.save_day_note(c.account(), day, &tradelens_core::DayNote { plan: "kế hoạch C".into(), ..Default::default() })
        .expect("saved");
    c.db.upsert_batch(&batch(vec![deal(9, 500, DealEntry::In, 0.0)]), "json").expect("imports");

    // Out of the usual order, and with a machine syncing twice in a row.
    c.sync(&drive);
    a.sync(&drive);
    a.sync(&drive);
    settle(&mut [&mut b, &mut c, &mut a], &drive);

    for machine in [&a, &b, &c] {
        let merged = machine.journal(200);
        assert_eq!(merged.notes, "B viết sau", "{}", machine.name);
        assert_eq!(merged.tags, ["b", "c"]);
        assert_eq!(merged.rating, 2);
        assert_eq!(machine.db.setting("time_display").expect("setting").as_deref(), Some("server"));
        assert_eq!(machine.db.rules(machine.account()).expect("rules").max_daily_loss, Some(150.0));
        assert_eq!(machine.db.day_note(machine.account(), day).expect("note").plan, "kế hoạch C");
        assert!(machine.db.catalog(CatalogKind::Tag).expect("catalog").iter().any(|t| t.key == "scalp"));
        assert_eq!(machine.db.deals(None).expect("deals").len(), 5);
    }
    assert_same(&a, &b);
    assert_same(&b, &c);
}

#[test]
fn a_sync_cut_in_the_middle_resumes_without_losing_or_repeating_changes() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| entry.rating = 5);
    drive.set_link(Link::Down);
    let engine = a.engine(&drive);
    assert!(matches!(engine.sync(&mut a.db), Err(CloudError::Network(_))));
    drive.set_link(Link::Up);
    let report = a.sync(&drive);
    assert_eq!(report.pushed, 1, "the change that was waiting goes up once");
    let again = a.sync(&drive);
    assert_eq!(again.pushed, 0, "and is not sent again");
    b.sync(&drive);
    assert_eq!(b.journal(100).rating, 5);
    assert_same(&a, &b);
}

#[test]
fn a_damaged_change_file_is_refused_and_leaves_the_data_alone() {
    let (drive, a, mut b) = started();
    let device = a.db.device_id().expect("id");
    let next = b.db.sync_state(PROVIDER).expect("state").pulled[&device] + 1;
    drive.put(&format!("changes/{device}/{next}-{}.json", next + 1), b"{ not json");
    let before = b.db.portable_state().expect("state");
    let engine = b.engine(&drive);
    assert!(matches!(engine.sync(&mut b.db), Err(CloudError::Corrupt(_))));
    assert_eq!(b.db.portable_state().expect("state"), before);
}

#[test]
fn a_new_machine_restores_the_newest_snapshot_and_the_changes_after_it() {
    let (drive, mut a, mut b) = started();
    // Everything so far is in the snapshot A uploaded on its first sync; what follows is not.
    assert!(drive.names().iter().any(|n| n.starts_with("snapshots/")));
    a.write_journal(200, |entry| entry.notes = "sau snapshot, trên A".into());
    a.db.save_playbook(&Playbook { name: "Mới sau snapshot".into(), ..Playbook::default() }, 5).expect("saved");
    tick();
    b.write_journal(100, |entry| entry.rating = 3);
    settle(&mut [&mut a, &mut b], &drive);

    let fresh = tempfile::tempdir().expect("a temp dir");
    let target = fresh.path().join("tradelens.db");
    let engine = SyncEngine::new(&drive, PROVIDER, "Máy mới").with_attachments(fresh.path().join("attachments"));
    let (restored, report) = engine.restore(&target).expect("restores");
    assert!(report.pulled >= 3, "the changes made after the snapshot came down: {report:?}");
    let mut n = Machine { dir: fresh, db: restored, name: "N" };
    assert_same(&a, &n);
    assert_eq!(n.journal(200).notes, "sau snapshot, trên A");
    assert_eq!(n.journal(100).rating, 3);
    assert!(n.playbook("Mới sau snapshot").is_some());
    let ids = [a.db.device_id().expect("id"), b.db.device_id().expect("id"), n.db.device_id().expect("id")];
    assert!(ids[2] != ids[0] && ids[2] != ids[1], "the restored machine is a new device");

    // It then works as a machine of its own: what it writes reaches the others.
    tick();
    n.write_journal(100, |entry| entry.emotion_before = "fomo".into());
    settle(&mut [&mut n, &mut a, &mut b], &drive);
    assert_eq!(a.journal(100).emotion_before, "fomo");
    assert_same(&a, &n);
    assert_same(&b, &n);
    let devices = engine.devices().expect("devices");
    assert_eq!(devices.len(), 3);
}

#[test]
fn restoring_from_a_drive_without_snapshots_says_so() {
    let drive = MemoryDrive::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let engine = SyncEngine::new(&drive, PROVIDER, "Máy mới");
    assert!(matches!(engine.restore(&dir.path().join("x.db")), Err(CloudError::NoSnapshot)));
}

static NOW: AtomicI64 = AtomicI64::new(0);

fn fake_now() -> i64 {
    NOW.load(Ordering::SeqCst)
}

#[test]
fn a_snapshot_is_taken_weekly_and_devices_are_listed() {
    let drive = MemoryDrive::new();
    let mut a = Machine::new("A");
    a.db.upsert_account(&info()).expect("account");
    let day = |n: i64| {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 1)
            .expect("a day")
            .and_hms_opt(12, 0, 0)
            .expect("noon")
            .and_utc()
            .timestamp_millis()
            + n * 86_400_000
    };
    let sync_on = |a: &mut Machine, n: i64| {
        NOW.store(day(n), Ordering::SeqCst);
        a.engine(&drive).with_clock(fake_now).sync(&mut a.db).expect("syncs")
    };
    assert!(sync_on(&mut a, 0).snapshot_taken, "the first sync takes one");
    assert!(!sync_on(&mut a, 3).snapshot_taken);
    assert!(!sync_on(&mut a, 6).snapshot_taken);
    assert!(sync_on(&mut a, 7).snapshot_taken, "a week on, another");
    let snapshots: Vec<String> = drive.names().into_iter().filter(|n| n.starts_with("snapshots/")).collect();
    assert_eq!(snapshots, ["snapshots/2026-11-01.db", "snapshots/2026-11-08.db"]);

    let devices = a.engine(&drive).devices().expect("devices");
    assert_eq!(devices.len(), 1);
    assert_eq!((devices[0].name.as_str(), devices[0].last_sync), ("A", day(7)));
}

fn write_image(machine: &Machine, relative: &str, bytes: &[u8]) {
    let path = machine.images().join(relative);
    std::fs::create_dir_all(path.parent().expect("has a folder")).expect("folder");
    std::fs::write(path, bytes).expect("writes");
}

fn shot(path: &str, caption: &str) -> Attachment {
    Attachment { path: path.into(), caption: caption.into(), timeframe: None, source: Source::Upload, added_at: 1 }
}

#[test]
fn images_are_uploaded_by_content_and_downloaded_where_missing() {
    let (drive, mut a, mut b) = started();
    write_image(&a, "images/aaaa.png", b"chart one");
    write_image(&a, "images/bbbb.jpg", b"chart two");
    a.write_journal(100, |entry| {
        entry.screenshots = vec![shot("images/aaaa.png", "H1 lúc vào"), shot("images/bbbb.jpg", "")];
    });
    let report = a.sync(&drive);
    assert_eq!(report.attachments_uploaded, 2);
    assert!(
        drive.names().iter().filter(|n| n.starts_with("attachments/")).all(|n| n.len() > "attachments/".len() + 64)
    );

    let report = b.sync(&drive);
    assert_eq!(report.attachments_downloaded, 2);
    assert_eq!(std::fs::read(b.images().join("images/aaaa.png")).expect("downloaded"), b"chart one");
    assert_eq!(b.journal(100).screenshots.len(), 2);
    assert_eq!(b.journal(100).screenshots[0].caption, "H1 lúc vào");
    assert_eq!(b.sync(&drive).attachments_downloaded, 0, "nothing more to fetch");
    assert_same(&a, &b);
}

#[test]
fn concurrent_screenshots_on_both_machines_are_both_kept_in_the_same_order() {
    let (drive, mut a, mut b) = started();
    write_image(&a, "images/aaaa.png", b"a");
    write_image(&b, "images/bbbb.png", b"b");
    a.write_journal(100, |entry| entry.screenshots.push(shot("images/aaaa.png", "từ A")));
    tick();
    b.write_journal(100, |entry| entry.screenshots.push(shot("images/bbbb.png", "từ B")));
    settle(&mut [&mut a, &mut b], &drive);
    settle(&mut [&mut a, &mut b], &drive);
    for machine in [&a, &b] {
        let paths: Vec<String> = machine.journal(100).screenshots.into_iter().map(|s| s.path).collect();
        assert_eq!(paths.len(), 2, "{}", machine.name);
        assert!(
            machine.images().join("images/aaaa.png").is_file() && machine.images().join("images/bbbb.png").is_file()
        );
    }
    assert_same(&a, &b);

    // Reordering on one machine reaches the other.
    tick();
    a.write_journal(100, |entry| entry.screenshots.reverse());
    let reversed: Vec<String> = a.journal(100).screenshots.into_iter().map(|s| s.path).collect();
    settle(&mut [&mut a, &mut b], &drive);
    let on_b: Vec<String> = b.journal(100).screenshots.into_iter().map(|s| s.path).collect();
    assert_eq!(on_b, reversed);
}

#[test]
fn a_hostile_image_path_cannot_write_outside_the_data_folder() {
    let (drive, mut a, mut b) = started();
    a.write_journal(100, |entry| entry.screenshots = vec![shot("../outside.png", "")]);
    // The path has no file on A, so nothing is uploaded; make the drive believe otherwise.
    let sha = "0".repeat(64);
    a.db.record_attachment("../outside.png", &sha).expect("recorded");
    drive.put(&format!("attachments/{sha}.png"), b"evil");
    a.sync(&drive);
    b.sync(&drive);
    let outside: &Path = &b.dir.path().join("outside.png");
    assert!(!outside.exists());
    assert!(!b.images().join("../outside.png").exists());
}

#[test]
fn the_drive_is_a_plain_folder_of_named_files() {
    let (drive, a, _) = started();
    let device = a.db.device_id().expect("id");
    let names = drive.names();
    assert!(names.iter().any(|n| n.starts_with(&format!("changes/{device}/1-"))), "{names:?}");
    assert!(names.iter().any(|n| n == "devices.json"));
    let listed = drive.list("changes/").expect("lists");
    assert!(listed.iter().all(|f| f.size > 0));
}

// --- the order of a sync, and what an image costs ----------------------------------------------

/// What a [`Spy`] refuses to upload.
#[derive(Debug, Default)]
enum Refuse {
    #[default]
    Nothing,
    /// Every image.
    Images,
    /// The image with exactly these bytes, going up or coming down.
    Bytes(Vec<u8>),
}

/// A drive that writes down every call, can refuse images, and tells how many uploads overlapped.
#[derive(Default)]
struct Spy {
    inner: MemoryDrive,
    events: Mutex<Vec<String>>,
    refuse: Mutex<Refuse>,
    running: AtomicUsize,
    peak: AtomicUsize,
    /// How long each upload and download of an image takes, as a slow link would.
    delay: Duration,
}

impl Spy {
    fn over(inner: &MemoryDrive) -> Self {
        Self { inner: inner.clone(), ..Self::default() }
    }

    fn slow(inner: &MemoryDrive, delay: Duration) -> Self {
        Self { delay, ..Self::over(inner) }
    }

    fn log(&self, event: String) {
        self.events.lock().expect("lock poisoned").push(event);
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().expect("lock poisoned").clone()
    }

    /// How many calls start with `prefix` (`"upload_new attachments/"`).
    fn count(&self, prefix: &str) -> usize {
        self.events().iter().filter(|event| event.starts_with(prefix)).count()
    }

    fn refuse(&self, what: Refuse) {
        *self.refuse.lock().expect("lock poisoned") = what;
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    fn refused(&self, name: &str, bytes: &[u8]) -> bool {
        match &*self.refuse.lock().expect("lock poisoned") {
            Refuse::Nothing => false,
            Refuse::Images => name.starts_with("attachments/"),
            Refuse::Bytes(wanted) => name.starts_with("attachments/") && wanted == bytes,
        }
    }

    /// The part every upload shares: time in flight, and the refusal of images.
    fn admit(&self, name: &str, bytes: &[u8]) -> Result<(), CloudError> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        // Long enough for the other uploads to start while this one is in flight.
        if name.starts_with("attachments/") {
            std::thread::sleep(self.delay.max(Duration::from_millis(15)));
        }
        let refused = self.refused(name, bytes);
        self.running.fetch_sub(1, Ordering::SeqCst);
        if refused {
            return Err(CloudError::Network("ảnh bị từ chối (thử nghiệm)".into()));
        }
        Ok(())
    }
}

impl CloudDrive for Spy {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteFile>, CloudError> {
        self.log(format!("list {prefix}"));
        self.inner.list(prefix)
    }

    fn upload(&self, name: &str, bytes: &[u8]) -> Result<RemoteFile, CloudError> {
        self.log(format!("upload {name}"));
        self.admit(name, bytes)?;
        self.inner.upload(name, bytes)
    }

    fn upload_new(&self, name: &str, bytes: &[u8]) -> Result<RemoteFile, CloudError> {
        self.log(format!("upload_new {name}"));
        self.admit(name, bytes)?;
        self.inner.upload_new(name, bytes)
    }

    fn download(&self, file: &RemoteFile) -> Result<Vec<u8>, CloudError> {
        self.log(format!("download {}", file.name));
        if file.name.starts_with("attachments/") {
            std::thread::sleep(self.delay);
        }
        let bytes = self.inner.download(file)?;
        if matches!(&*self.refuse.lock().expect("lock poisoned"), Refuse::Bytes(wanted) if *wanted == bytes) {
            return Err(CloudError::Network("ảnh bị từ chối khi tải (thử nghiệm)".into()));
        }
        Ok(bytes)
    }

    fn delete(&self, file: &RemoteFile) -> Result<(), CloudError> {
        self.log(format!("delete {}", file.name));
        self.inner.delete(file)
    }
}

impl Machine {
    /// A sync through any drive, the failure handed back.
    fn try_sync(&mut self, drive: &dyn CloudDrive) -> Result<SyncReport, CloudError> {
        SyncEngine::new(drive, PROVIDER, self.name).with_attachments(self.images()).sync(&mut self.db)
    }
}

/// Machine A with a trade and `count` charts (`images/c<n>.png`) in its journal; B has synced.
fn with_charts(count: usize) -> (MemoryDrive, Machine, Machine) {
    let (drive, mut a, b) = started();
    let paths: Vec<String> = (0..count).map(|n| format!("images/c{n}.png")).collect();
    for (n, path) in paths.iter().enumerate() {
        write_image(&a, path, format!("chart {n}").as_bytes());
    }
    a.write_journal(100, |entry| entry.screenshots = paths.iter().map(|path| shot(path, "")).collect());
    (drive, a, b)
}

fn positions(events: &[String], wanted: impl Fn(&str) -> bool) -> Vec<usize> {
    events.iter().enumerate().filter(|(_, event)| wanted(event)).map(|(at, _)| at).collect()
}

#[test]
fn the_journal_goes_up_before_the_images_and_comes_down_before_them() {
    let (drive, mut a, mut b) = with_charts(6);
    let spy = Spy::over(&drive);
    let report = a.try_sync(&spy).expect("syncs");
    assert_eq!(report.attachments_uploaded, 6);
    let events = spy.events();
    let journal = positions(&events, |e| {
        e.starts_with("upload changes/") || e.starts_with("upload snapshots/") || e.starts_with("upload devices.json")
    });
    let images = positions(&events, |e| e.starts_with("upload_new attachments/"));
    assert_eq!((images.len(), journal.is_empty()), (6, false), "{events:?}");
    assert!(
        journal.iter().max() < images.iter().min(),
        "the changes, the snapshot and the device list are on the drive before the first image: {events:?}"
    );

    let spy = Spy::over(&drive);
    let report = b.try_sync(&spy).expect("syncs");
    assert_eq!(report.attachments_downloaded, 6);
    let events = spy.events();
    let changes = positions(&events, |e| e.starts_with("download changes/"));
    let images = positions(&events, |e| e.starts_with("download attachments/"));
    assert!(changes.iter().max() < images.iter().min(), "the journal is read before the first image: {events:?}");
    assert_same(&a, &b);
}

#[test]
fn another_machine_gets_the_journal_while_the_images_cannot_be_uploaded() {
    let (drive, mut a, mut b) = with_charts(3);
    let spy = Spy::over(&drive);
    spy.refuse(Refuse::Images);
    let report = a.try_sync(&spy).expect("images that cannot go up do not fail the sync");
    assert_eq!((report.attachments_uploaded, report.attachments_failed), (0, 3));
    assert!(drive.names().iter().all(|name| !name.starts_with("attachments/")), "{:?}", drive.names());
    assert!(
        drive.names().iter().any(|name| name == "devices.json"),
        "the sync got as far as the device list: {:?}",
        drive.names()
    );

    // B restores everything but the pictures, and the missing ones are not an error.
    let report = b.try_sync(&drive).expect("B syncs although the images are not on the drive");
    assert_eq!((report.pulled > 0, report.attachments_downloaded), (true, 0));
    assert_eq!(b.journal(100).screenshots.len(), 3, "the journal names the images");
    assert!(!b.images().join("images/c0.png").exists());
    assert_same(&a, &b);

    // The next sync of A has nothing but the images to move; then B fetches them.
    spy.refuse(Refuse::Nothing);
    let report = a.try_sync(&spy).expect("syncs");
    assert_eq!((report.pushed, report.attachments_uploaded), (0, 3));
    let report = b.try_sync(&drive).expect("syncs");
    assert_eq!(report.attachments_downloaded, 3);
    assert_eq!(std::fs::read(b.images().join("images/c2.png")).expect("downloaded"), b"chart 2");
}

#[test]
fn an_image_that_fails_does_not_lose_the_ones_that_went_up() {
    let (drive, mut a, _) = with_charts(8);
    let spy = Spy::over(&drive);
    spy.refuse(Refuse::Bytes(b"chart 5".to_vec()));
    let report = a.try_sync(&spy).expect("one refused image does not fail the sync");
    assert_eq!((report.attachments_uploaded, report.attachments_failed), (7, 1));
    let uploaded = drive.names().iter().filter(|name| name.starts_with("attachments/")).count();
    assert_eq!(uploaded, 7, "every other image is on the drive: {:?}", drive.names());

    spy.refuse(Refuse::Nothing);
    let report = a.try_sync(&spy).expect("syncs");
    assert_eq!(report.attachments_uploaded, 1, "only the one that failed is sent again");
}

#[test]
fn a_restore_with_one_image_refused_still_restores_and_the_next_sync_fetches_it() {
    let (drive, mut a, _) = with_charts(6);
    a.try_sync(&Spy::over(&drive)).expect("syncs");
    a.engine(&drive).snapshot(&a.db).expect("a snapshot that names the images");

    let fresh = tempfile::tempdir().expect("a temp dir");
    let spy = Spy::over(&drive);
    spy.refuse(Refuse::Bytes(b"chart 3".to_vec()));
    let engine = SyncEngine::new(&spy, PROVIDER, "Máy mới").with_attachments(fresh.path().join("attachments"));
    let (restored, report) = engine.restore(&fresh.path().join("tradelens.db")).expect("restores despite the image");
    assert_eq!((report.attachments_downloaded, report.attachments_failed), (5, 1));
    for n in [0, 1, 2, 4, 5] {
        assert!(fresh.path().join(format!("attachments/images/c{n}.png")).is_file(), "c{n}");
    }
    assert!(!fresh.path().join("attachments/images/c3.png").exists());
    let mut n = Machine { dir: fresh, db: restored, name: "N" };
    assert_eq!(n.journal(100).screenshots.len(), 6, "the journal is all there");

    spy.refuse(Refuse::Nothing);
    let report = n.try_sync(&spy).expect("syncs");
    assert_eq!((report.attachments_downloaded, report.attachments_failed), (1, 0));
    assert_eq!(std::fs::read(n.images().join("images/c3.png")).expect("fetched"), b"chart 3");
}

/// Makes `path` unreadable for as long as the returned value lives; `None` when this system cannot.
#[cfg(windows)]
fn make_unreadable(path: &Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // Opened with no sharing: every other open fails with a sharing violation.
    std::fs::OpenOptions::new().read(true).share_mode(0).open(path).ok()
}

#[cfg(unix)]
struct Unreadable(std::path::PathBuf);

#[cfg(unix)]
impl Drop for Unreadable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        drop(std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o600)));
    }
}

#[cfg(unix)]
fn make_unreadable(path: &Path) -> Option<Unreadable> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).ok()?;
    let lock = Unreadable(path.to_owned());
    // A superuser reads it anyway.
    std::fs::read(path).is_err().then_some(lock)
}

#[test]
fn an_image_that_cannot_be_read_does_not_hold_the_journal_back() {
    let (drive, mut a, mut b) = with_charts(3);
    let locked = a.images().join("images/c1.png");
    let Some(lock) = make_unreadable(&locked) else { return };
    assert!(std::fs::read(&locked).is_err(), "the image is unreadable");

    let report = a.try_sync(&Spy::over(&drive)).expect("the sync goes through");
    assert!(report.pushed > 0, "the journal went up: {report:?}");
    assert_eq!(report.attachments_uploaded, 2, "the other images went up");
    let report = b.try_sync(&drive).expect("syncs");
    assert!(report.pulled > 0 && b.journal(100).screenshots.len() == 3, "the other machine has the journal");

    drop(lock);
    let report = a.try_sync(&Spy::over(&drive)).expect("syncs");
    assert_eq!(report.attachments_uploaded, 1, "the image is sent once it can be read");
}

#[test]
fn an_image_costs_one_upload_and_a_listing_is_shared_by_all() {
    let (drive, mut a, mut b) = with_charts(12);
    // The same picture under two paths is one file on the drive.
    write_image(&a, "images/copy.png", b"chart 0");
    a.write_journal(100, |entry| entry.screenshots.push(shot("images/copy.png", "")));
    let spy = Spy::over(&drive);
    let report = a.try_sync(&spy).expect("syncs");
    assert_eq!(report.attachments_uploaded, 12);
    assert_eq!(spy.count("upload_new attachments/"), 12, "{:?}", spy.events());
    assert_eq!(spy.count("upload attachments/"), 0, "an image is never sent through the replacing call");
    assert_eq!(spy.count("list attachments/"), 1, "one listing for all the images");
    let names: HashSet<String> =
        spy.events().iter().filter_map(|e| e.strip_prefix("upload_new ").map(str::to_owned)).collect();
    assert_eq!(names.len(), 12, "no image is sent twice");
    assert!(spy.peak() > 1, "the images go up side by side");
    assert!(spy.peak() <= 4, "but only a few at a time: {}", spy.peak());

    // Nothing changed: no image is sent again.
    let spy = Spy::over(&drive);
    assert_eq!(a.try_sync(&spy).expect("syncs").attachments_uploaded, 0);
    assert_eq!(spy.count("upload_new"), 0);
    assert_eq!(spy.count("list attachments/"), 0);

    let spy = Spy::over(&drive);
    let report = b.try_sync(&spy).expect("syncs");
    assert_eq!(report.attachments_downloaded, 13, "the copy is a path of its own on B");
    assert_eq!(spy.count("list attachments/"), 1);
    assert_eq!(spy.count("download attachments/"), 13, "one download per path");
}

#[test]
fn a_sync_with_nothing_new_reads_no_image_and_asks_the_drive_nothing_about_them() {
    let (drive, mut a, mut b) = with_charts(5);
    a.try_sync(&Spy::over(&drive)).expect("syncs");
    b.try_sync(&Spy::over(&drive)).expect("syncs");

    // The pictures on A can no longer be read as they were: a sync that opened them would find a
    // new hash and send them again.
    for n in 0..5 {
        write_image(&a, &format!("images/c{n}.png"), b"changed on disk");
    }
    for (machine, name) in [(&mut a, "A"), (&mut b, "B")] {
        let spy = Spy::over(&drive);
        let report = machine.try_sync(&spy).expect("syncs");
        assert_eq!(
            (report.pushed, report.pulled, report.attachments_uploaded, report.attachments_downloaded),
            (0, 0, 0, 0),
            "{name}"
        );
        for call in ["upload_new", "upload attachments/", "download attachments/", "list attachments/"] {
            assert_eq!(spy.count(call), 0, "{name}: {call} in {:?}", spy.events());
        }
        // What is left is the journal's own bookkeeping: the listings and the device list.
        println!("a sync with nothing new makes {} calls on {name}: {:?}", spy.events().len(), spy.events());
    }
}

#[test]
fn many_images_move_side_by_side_and_a_restore_fetches_each_exactly_once() {
    const IMAGES: usize = 16;
    const DELAY: Duration = Duration::from_millis(40);
    let (drive, mut a, _) = with_charts(IMAGES);
    let spy = Spy::slow(&drive, DELAY);
    let started = Instant::now();
    let report = a.try_sync(&spy).expect("syncs");
    let up = started.elapsed();
    assert_eq!(report.attachments_uploaded, IMAGES);
    let one_by_one = DELAY * u32::try_from(IMAGES).expect("a few");
    assert!(
        up < one_by_one / 2,
        "{IMAGES} uploads of {DELAY:?} took {up:?}, one by one they would take {one_by_one:?}"
    );
    // The snapshot a new machine restores holds the journal that names the images.
    a.engine(&drive).snapshot(&a.db).expect("a snapshot");

    let fresh = tempfile::tempdir().expect("a temp dir");
    let spy = Spy::slow(&drive, DELAY);
    let engine = SyncEngine::new(&spy, PROVIDER, "Máy mới").with_attachments(fresh.path().join("attachments"));
    let started = Instant::now();
    let (restored, report) = engine.restore(&fresh.path().join("tradelens.db")).expect("restores");
    let down = started.elapsed();
    assert_eq!(report.attachments_downloaded, IMAGES);
    assert_eq!(spy.count("download attachments/"), IMAGES, "one download per image: {:?}", spy.events());
    assert_eq!(spy.count("list attachments/"), 1, "and the folder is listed once");
    assert!(down < one_by_one / 2, "{IMAGES} downloads of {DELAY:?} took {down:?}, one by one {one_by_one:?}");
    for n in 0..IMAGES {
        let bytes = std::fs::read(fresh.path().join(format!("attachments/images/c{n}.png"))).expect("an image");
        assert_eq!(bytes, format!("chart {n}").as_bytes());
    }
    assert!(!fresh.path().join("attachments/images/c0.png.part").exists(), "no half-written file is left");
    drop(restored);
    println!("{IMAGES} images, {DELAY:?} each: upload {up:?}, restore {down:?}, one by one {one_by_one:?}");
}

/// A small deterministic generator, so that a failing run can be replayed from its seed.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(sides).expect("a few sides")).expect("fits")
    }
}

fn toggle(list: &mut Vec<String>, word: &str) {
    if let Some(at) = list.iter().position(|item| item == word) {
        list.remove(at);
    } else {
        list.push(word.to_owned());
    }
}

fn random_step(machine: &mut Machine, dice: &mut Dice, drive: &MemoryDrive, step: usize) {
    const WORDS: [&str; 4] = ["a", "b", "c", "d"];
    let position = [100, 200][dice.roll(2)];
    let word = WORDS[dice.roll(4)];
    let who = machine.name;
    match dice.roll(12) {
        0 => machine.write_journal(position, |entry| entry.notes = format!("{who} #{step}")),
        1 | 2 => machine.write_journal(position, |entry| toggle(&mut entry.tags, word)),
        3 => machine.write_journal(position, |entry| toggle(&mut entry.mistakes, word)),
        4 => machine.write_journal(position, |entry| entry.rating = u8::try_from(dice.roll(6)).expect("small")),
        5 => {
            let name = format!("Setup {word}");
            if machine.playbook(&name).is_none() {
                machine.db.save_playbook(&Playbook { name, ..Playbook::default() }, 0).expect("saved");
            }
        }
        6 => {
            let mut all = machine.db.playbooks().expect("playbooks");
            if !all.is_empty() {
                let mut chosen = all.swap_remove(dice.roll(all.len()));
                chosen.description = format!("{} #{step}", machine.name);
                machine.db.save_playbook(&chosen, 0).expect("saved");
            }
        }
        7 => {
            let all = machine.db.playbooks().expect("playbooks");
            if !all.is_empty() {
                machine.db.delete_playbook(all[dice.roll(all.len())].id).expect("deleted");
            }
        }
        8 => {
            let mut items = machine.db.checklist(None).expect("checklist");
            if dice.roll(3) == 0 && !items.is_empty() {
                items.remove(dice.roll(items.len()));
            } else {
                items.push(ChecklistItem { id: 0, text: format!("{} #{step}", machine.name) });
            }
            machine.db.save_checklist(None, &items).expect("saved");
        }
        9 => machine.db.set_setting("time_display", ["server", "local"][dice.roll(2)]).expect("saved"),
        10 => {
            let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 1 + u32::try_from(dice.roll(3)).expect("small"))
                .expect("a day");
            let note = tradelens_core::DayNote { plan: format!("{} #{step}", machine.name), ..Default::default() };
            machine.db.save_day_note(machine.account(), day, &note).expect("saved");
        }
        _ => {
            let ticket = 100 + i64::try_from(dice.roll(6)).expect("small");
            machine.db.upsert_batch(&batch(vec![deal(ticket, ticket, DealEntry::In, 1.0)]), "html").expect("imports");
        }
    }
    if dice.roll(4) == 0 {
        machine.sync(drive);
    }
}

#[test]
fn random_edits_on_three_machines_always_converge() {
    for seed in 1..=8_u64 {
        let (drive, mut a, mut b) = started();
        let mut c = Machine::new("C");
        settle(&mut [&mut a, &mut b, &mut c], &drive);
        let mut dice = Dice(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for step in 0..60 {
            let machine = match dice.roll(3) {
                0 => &mut a,
                1 => &mut b,
                _ => &mut c,
            };
            random_step(machine, &mut dice, &drive, step);
        }
        settle(&mut [&mut a, &mut b, &mut c], &drive);
        settle(&mut [&mut c, &mut b, &mut a], &drive);
        assert_same(&a, &b);
        assert_same(&b, &c);
        assert_eq!(a.db.deals(None).expect("deals").len(), b.db.deals(None).expect("deals").len(), "seed {seed}");
    }
}
