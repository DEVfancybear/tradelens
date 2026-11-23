//! The v0.3 criterion: two machines edit at the same time, and after they sync nothing is lost.
//!
//! Each "machine" is its own database file; all of them share one [`MemoryDrive`].

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tempfile::TempDir;
use tradelens_cloud::CloudError;
use tradelens_cloud::drive::CloudDrive;
use tradelens_cloud::memory::{Link, MemoryDrive};
use tradelens_cloud::sync::{SyncEngine, SyncReport};
use tradelens_core::{
    Db,
    attachments::{Attachment, Source},
    catalog::{CatalogItem, CatalogKind},
    model::{AccountInfo, ChecklistItem, Deal, DealBatch, DealEntry, DealKind, JournalEntry, Rules, TradeKey},
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
