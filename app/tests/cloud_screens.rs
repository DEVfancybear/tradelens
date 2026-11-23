//! Đăng nhập & sync, headless on the real engine. Two app instances (two data directories) share one
//! in-memory drive, which the demo mode (`cloud.mode = memory`, the drive named by `cloud.demo_drive`)
//! offers, so sync, restore and sign-out run for real without an account.

use nsh_testing::{DialogAnswer, Headless};
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, TradeKey, model::AccountInfo, playbook::Playbook,
};

mod support;
use support::{Fixture, WAIT, wait_until};

const EMAIL: &str = "demo@tradelens.local";

fn deal(ticket: i64, entry: DealEntry, kind: DealKind, at: i64, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: 7,
        time_msc: at,
        symbol: "EURUSD".into(),
        kind,
        entry,
        volume: 0.5,
        price: 1.1,
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

/// An account with one closed trade, the way an import leaves it.
fn seed(db: &mut Db) -> TradeKey {
    let now = 1_763_000_000_000;
    let batch = DealBatch {
        account: AccountInfo {
            login: "52012345".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            balance: Some(10_000.0),
            ..AccountInfo::default()
        },
        deals: vec![
            deal(10, DealEntry::In, DealKind::Buy, now, 0.0),
            deal(11, DealEntry::Out, DealKind::Sell, now + 3_600_000, 25.0),
        ],
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("the batch is stored");
    key_of(db)
}

fn key_of(db: &Db) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: 7, segment: 0 }
}

/// A fixture whose app is set to the demo, on the drive named `drive`.
/// How a demo machine starts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    Out,
    In,
    InWithAutoSync,
}

fn demo_fixture(drive: &str, start: Start) -> Fixture {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    db.set_setting("cloud.mode", "memory").expect("the demo is chosen");
    db.set_setting("cloud.demo_drive", drive).expect("the drive is named");
    if start != Start::Out {
        db.set_setting("cloud.demo_user", "google").expect("signed in");
        db.set_setting("cloud.provider", "google").expect("the provider is chosen");
    }
    if start == Start::InWithAutoSync {
        db.set_setting("cloud.auto_sync", "1").expect("auto sync is on");
    }
    fixture
}

fn start(fixture: &Fixture) -> Headless {
    fixture.start(1280.0, 1100.0, |_| {})
}

fn open_cloud(app: &mut Headless) {
    app.press("role:button[Đăng nhập & sync]").expect("the cloud row is a button");
    app.wait_for("Khôi phục & sync").ok();
    app.wait_for("Đồng bộ").expect("the page opens");
}

/// Presses "Sync ngay" until the database says a sync finished after `before` (a press made while the
/// settings are still being read does nothing).
fn sync_and_wait(app: &mut Headless, fixture: &Fixture, before: Option<i64>) {
    let db = fixture.db();
    let done = || db.sync_state("demo").expect("state").last_sync > before;
    let mut tries = 0;
    while !done() {
        assert!(tries < 40, "no sync finished");
        tries += 1;
        if app.screen().enabled("role:button[Sync ngay]") == Ok(true) {
            app.press("role:button[Sync ngay]").expect("the sync button");
        }
        app.next_change(std::time::Duration::from_millis(250)).expect("the app runs");
    }
}

#[test]
fn signing_in_syncing_and_a_journal_edit_reaches_the_other_machine() {
    let drive = "hai máy";
    let (one, two) = (demo_fixture(drive, Start::Out), demo_fixture(drive, Start::Out));
    let key_one = seed(&mut one.db());

    let mut a = start(&one);
    open_cloud(&mut a);
    a.wait_for("Chưa đăng nhập").expect("nobody is signed in");
    a.wait_for("text:/^Chế độ thử/").expect("the demo says so");
    a.press("role:button[Đăng nhập với Google]").expect("the sign-in button");
    a.wait_for(EMAIL).expect("the demo account is shown");
    a.wait_for("Đang dùng").expect("the provider is in use");
    a.wait_for("Google Drive · chưa sync").expect("the sidebar follows");
    sync_and_wait(&mut a, &one, None);
    a.wait_for("text:/^Google Drive · đã sync/").expect("the sidebar says it synced");
    a.wait_for("text:/Lần cuối .* · đẩy \\d+ thay đổi, nhận \\d+/").expect("the card says what the sync did");
    a.wait_for("thiết bị này").expect("this device is in the list");

    // The second machine signs in to the same drive and takes what the first one pushed.
    let mut b = start(&two);
    open_cloud(&mut b);
    b.press("role:button[Đăng nhập với Google]").expect("the sign-in button");
    b.wait_for(EMAIL).expect("the demo account is shown");
    sync_and_wait(&mut b, &two, None);
    let on_two = two.db();
    wait_until("the trade arriving on the second machine", || on_two.accounts().is_ok_and(|a| a.len() == 1));
    b.wait_for("text:/nhận [1-9]\\d* /").ok();

    // The trader writes a journal on the second machine; one sync there and one here bring it over.
    let account_two = on_two.accounts().expect("accounts").remove(0).id;
    let key_two = TradeKey { account_id: account_two, position_id: 7, segment: 0 };
    let mut db_two = two.db();
    db_two
        .save_journal(&key_two, &JournalEntry { notes: "viết ở máy hai".into(), rating: 4, ..JournalEntry::default() })
        .expect("the journal is stored");
    db_two.save_playbook(&Playbook { name: "Pullback từ máy hai".into(), ..Playbook::default() }, 0).expect("a setup");
    let before_two = on_two.sync_state("demo").expect("state").last_sync;
    sync_and_wait(&mut b, &two, before_two);
    let on_one = one.db();
    let before_one = on_one.sync_state("demo").expect("state").last_sync;
    sync_and_wait(&mut a, &one, before_one);
    wait_until("the edit arriving on the first machine", || {
        on_one.journal(&key_one).is_ok_and(|e| e.is_some_and(|e| e.notes == "viết ở máy hai" && e.rating == 4))
    });
    assert!(on_one.playbooks().expect("setups").iter().any(|p| p.name == "Pullback từ máy hai"));
    a.wait_for("text:/nhận [1-9]\\d* /").ok();
}

#[test]
fn a_new_machine_restores_the_data_of_the_old_one_and_keeps_a_copy_of_its_own() {
    let drive = "khôi phục";
    let (old, new) = (demo_fixture(drive, Start::In), demo_fixture(drive, Start::Out));
    seed(&mut old.db());
    let mut a = start(&old);
    open_cloud(&mut a);
    a.wait_for(EMAIL).expect("the old machine is signed in");
    sync_and_wait(&mut a, &old, None);

    // The new machine has a little data of its own, which the restore must keep a copy of.
    let mut own = new.db();
    own.save_playbook(&Playbook { name: "Chỉ có ở máy mới".into(), ..Playbook::default() }, 0).expect("a setup");
    drop(own);
    let mut b = start(&new);
    open_cloud(&mut b);
    b.press("role:button[Đăng nhập với Google]").expect("the sign-in button");
    b.wait_for(EMAIL).expect("signed in");
    b.wait_until("the restore button", WAIT, |s| s.enabled("role:button[Khôi phục sang máy này]") == Ok(true))
        .expect("the restore button is on");
    // Nothing happens until the trader confirms.
    b.on_dialog(|_| DialogAnswer::Press("Huỷ".to_owned()));
    b.press("role:button[Khôi phục sang máy này]").expect("the restore button");
    b.wait(std::time::Duration::from_millis(400)).expect("the app runs");
    assert!(new.db().accounts().expect("accounts").is_empty(), "declining the question restores nothing");

    b.on_dialog(|dialog| {
        let title = dialog.title.clone().unwrap_or_default();
        assert!(title.contains("Thay toàn bộ dữ liệu"), "{title}");
        DialogAnswer::Press("Thay dữ liệu".to_owned())
    });
    b.press("role:button[Khôi phục sang máy này]").expect("the restore button");
    b.wait_for("Đã khôi phục").expect("the card says it restored");
    let restored = new.db();
    assert_eq!(restored.accounts().expect("accounts").len(), 1, "the old machine's account came over");
    assert!(restored.playbooks().expect("setups").is_empty(), "what the new machine had is replaced");
    assert!(new.path().join("tradelens.before-restore.db").exists(), "a copy of the replaced data is kept");
    let copy = Db::open(new.path().join("tradelens.before-restore.db")).expect("the copy opens");
    assert_eq!(copy.playbooks().expect("setups")[0].name, "Chỉ có ở máy mới");
    assert_eq!(
        restored.setting("cloud.mode").expect("reads").as_deref(),
        Some("memory"),
        "the device's own settings stay"
    );
    assert_ne!(restored.device_id().expect("id"), copy.device_id().expect("id"));

    // The pages read the new database: the trade shows in the list.
    b.press("role:button[Lệnh]").expect("the trades row");
    b.wait_for("Hiển thị 1 / 1 lệnh").expect("the restored trade is listed");
    b.press("role:button[Đăng nhập & sync]").expect("back to the cloud page");
    b.wait_for("text:/thiết bị này/").expect("the devices are listed");
}

#[test]
fn without_a_client_id_the_screen_says_which_setting_and_variable_to_set() {
    if std::env::var("TRADELENS_GOOGLE_CLIENT_ID").is_ok() || std::env::var("TRADELENS_CLOUD").is_ok() {
        return;
    }
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    open_cloud(&mut app);
    app.wait_for("text:/cloud\\.google\\.client_id/").expect("the setting is named");
    app.wait_for("text:/TRADELENS_GOOGLE_CLIENT_ID/").expect("the variable is named");
    app.wait_for("text:/docs\\/cloud-sync\\.md/").expect("the doc is named");
    app.wait_for("text:/cloud\\.microsoft\\.client_id/").expect("the other provider too");
    app.wait_for("Cloud: chưa đăng nhập").expect("the sidebar says nobody is signed in");
    assert_eq!(app.screen().enabled("role:button[Sync ngay]"), Ok(false), "nothing to sync with");
}

#[test]
fn signing_out_asks_first_and_forgets_the_session() {
    let fixture = demo_fixture("đăng xuất", Start::In);
    let mut app = start(&fixture);
    open_cloud(&mut app);
    app.wait_for(EMAIL).expect("signed in");
    app.on_dialog(|_| DialogAnswer::Press("Huỷ".to_owned()));
    app.press("role:button[Đăng xuất]").expect("the sign-out button");
    app.wait(std::time::Duration::from_millis(400)).expect("the app runs");
    assert!(app.screen().shows(EMAIL), "cancelling the question keeps the session");
    app.on_dialog(|_| DialogAnswer::Press("Đăng xuất".to_owned()));
    app.press("role:button[Đăng xuất]").expect("the sign-out button");
    app.wait_for_gone(EMAIL, WAIT).expect("the account is gone");
    app.wait_for("Cloud: chưa đăng nhập").expect("the sidebar follows");
    let db = fixture.db();
    wait_until("the demo session being forgotten", || {
        db.setting("cloud.demo_user").is_ok_and(|v| v.as_deref() == Some(""))
    });
}

#[test]
fn the_automatic_sync_switch_is_kept_on_this_device() {
    let fixture = demo_fixture("tự động", Start::In);
    let mut app = start(&fixture);
    open_cloud(&mut app);
    app.press("label:Tự động sync").expect("the switch");
    let db = fixture.db();
    wait_until("the switch being kept", || db.setting("cloud.auto_sync").is_ok_and(|v| v.as_deref() == Some("1")));
    app.press("label:Tự động sync").expect("the switch again");
    wait_until("the switch being turned off", || {
        db.setting("cloud.auto_sync").is_ok_and(|v| v.as_deref() == Some("0"))
    });
}

#[test]
fn closing_the_window_syncs_first_when_the_automatic_sync_is_on() {
    let fixture = demo_fixture("đóng app", Start::InWithAutoSync);
    seed(&mut fixture.db());
    let mut app = fixture.start(1280.0, 1100.0, |config| config.environment.os = nsh_core::platform::Os::Windows);
    // The settings are read a moment after the app opens.
    open_cloud(&mut app);
    app.wait_for(EMAIL).expect("signed in");
    app.wait_for("Google Drive · chưa sync").expect("the settings are read, auto sync among them");
    let db = fixture.db();
    assert_eq!(db.sync_state("demo").expect("state").last_sync, None, "nothing synced yet");
    app.press_window_close("main").expect("the close button");
    wait_until("the sync that runs before the window closes", || {
        db.sync_state("demo").is_ok_and(|s| s.last_sync.is_some() && s.pushed_seq > 0)
    });
}
