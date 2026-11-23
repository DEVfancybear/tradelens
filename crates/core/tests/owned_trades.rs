//! Trades the trader owns: typed in, edited and deleted, with their journal, images and sync log.

use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, Gaps, JournalEntry, ManualTrade, Origin, Side, Trade, TradeKey,
    attachments::{Attachment, Source},
    model::AccountInfo,
    sync::entity,
    time::parse_mt5_time,
    trades::build_trades,
};

fn info(login: &str, name: &str) -> AccountInfo {
    AccountInfo { login: login.into(), server: "S".into(), name: name.into(), ..AccountInfo::default() }
}

fn typed() -> ManualTrade {
    ManualTrade {
        symbol: "EURUSD".into(),
        side: Side::Long,
        volume: Some(0.5),
        open_time: parse_mt5_time("2026.11.19 08:00:00").expect("fixture"),
        open_price: Some(1.0850),
        close_time: parse_mt5_time("2026.11.19 09:00:00").expect("fixture"),
        close_price: Some(1.0905),
        date_only: false,
        sl: 1.0830,
        tp: 1.0910,
        profit: Some(275.0),
        commission: -3.0,
        swap: 0.0,
        result_r: None,
    }
}

fn mt5_deal(ticket: i64, position: i64, entry: DealEntry, at: &str) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: parse_mt5_time(at).expect("fixture"),
        symbol: "XAUUSD".into(),
        kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
        entry,
        volume: 0.1,
        price: 2650.0,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: if entry == DealEntry::In { 0.0 } else { 40.0 },
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: Gaps::NONE,
        reason: None,
    }
}

fn trades_of(db: &Db) -> Vec<Trade> {
    let mut all = Vec::new();
    for account in db.accounts().expect("accounts") {
        let deals: Vec<Deal> = db.deals(Some(account.id)).expect("deals").into_iter().map(|(_, d)| d).collect();
        all.extend(build_trades(account.id, &deals));
    }
    all
}

fn shot(path: &str) -> Attachment {
    Attachment { path: path.into(), caption: "H1".into(), timeframe: None, source: Source::Upload, added_at: 1 }
}

fn mt5_trade(db: &mut Db) -> TradeKey {
    let batch = DealBatch {
        account: info("500", "Live"),
        deals: vec![
            mt5_deal(11, 11, DealEntry::In, "2026.11.18 10:00:00"),
            mt5_deal(12, 11, DealEntry::Out, "2026.11.18 11:00:00"),
        ],
        positions: Vec::new(),
    };
    let account_id = db.upsert_batch(&batch, "html").expect("stored").account_id;
    TradeKey { account_id, position_id: 11, segment: 0 }
}

#[test]
fn the_symbol_is_stored_the_way_the_importer_stores_it() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let key = db.add_manual_trade(account, &ManualTrade { symbol: " eur/usd ".into(), ..typed() }).expect("stored");
    assert_eq!(trades_of(&db)[0].symbol, "EURUSD");
    db.update_manual_trade(&key, &ManualTrade { symbol: "xau_usd".into(), ..typed() }).expect("edited");
    assert_eq!(trades_of(&db)[0].symbol, "XAUUSD", "an edit normalises too");
}

#[test]
fn prices_are_stored_whole() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let key = db
        .add_manual_trade(account, &ManualTrade { open_price: Some(1.08505), close_price: Some(1.0905), ..typed() })
        .expect("stored");
    let back = db.manual_trade(&key).expect("reads").expect("a trade of the trader's");
    assert_eq!((back.open_price, back.close_price), (Some(1.08505), Some(1.0905)));
}

#[test]
fn a_manual_trade_in_a_journal_account_is_still_a_manual_trade() {
    let mut db = Db::in_memory().expect("opens");
    // The account a journal import files its trades under: its tickets are far below a typed trade's.
    let account = db.upsert_account(&info("notion", "Nhật ký Notion")).expect("account");
    db.upsert_batch(
        &DealBatch {
            account: info("notion", "Nhật ký Notion"),
            deals: vec![
                mt5_deal(-(1 << 40) - 10, -(1 << 40) - 11, DealEntry::Out, "2026.11.18 11:00:00"),
                mt5_deal(-(1 << 40) - 11, -(1 << 40) - 11, DealEntry::In, "2026.11.18 10:00:00"),
            ],
            positions: Vec::new(),
        },
        "notion",
    )
    .expect("imported");
    let typed_key = db.add_manual_trade(account, &typed()).expect("stored");
    let origins = db.trade_origins(None).expect("origins");
    assert_eq!(origins[&(account, typed_key.position_id)], Origin::Manual);
    assert_eq!(origins[&(account, -(1 << 40) - 11)], Origin::Notion);
    assert_eq!(Origin::of_ticket(typed_key.position_id), Origin::Manual, "and its tickets say the same");
}

#[test]
fn origin_comes_from_the_source_of_the_deals() {
    let mut db = Db::in_memory().expect("opens");
    let mt5 = mt5_trade(&mut db);
    let typed_key = db.add_manual_trade(mt5.account_id, &typed()).expect("stored");
    let origins = db.trade_origins(Some(mt5.account_id)).expect("origins");
    assert_eq!(origins[&(mt5.account_id, 11)], Origin::Mt5);
    assert_eq!(origins[&(mt5.account_id, typed_key.position_id)], Origin::Manual);
    for (source, origin) in [("ea", Origin::Mt5), ("html", Origin::Mt5), ("xlsx", Origin::Mt5), ("json", Origin::Mt5)] {
        assert_eq!(Origin::of_source(source), origin);
    }
    assert_eq!(Origin::of_source("manual"), Origin::Manual);
    assert_eq!(Origin::of_source("notion"), Origin::Notion);
}

#[test]
fn editing_rewrites_the_deals_and_keeps_the_tickets_and_the_journal() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let key = db.add_manual_trade(account, &typed()).expect("stored");
    let entry =
        JournalEntry { notes: "vào sớm".into(), screenshots: vec![shot("images/a.png")], ..JournalEntry::default() };
    db.save_journal(&key, &entry).expect("journal");
    let tickets_before: Vec<i64> = db.deals(Some(account)).expect("deals").iter().map(|(_, d)| d.ticket).collect();

    let edited = ManualTrade {
        open_price: Some(1.08505),
        close_price: Some(1.0910),
        open_time: parse_mt5_time("2026.11.19 07:30:00").expect("fixture"),
        side: Side::Short,
        profit: Some(-120.0),
        result_r: Some(-1.0),
        ..typed()
    };
    db.update_manual_trade(&key, &edited).expect("edited");

    let tickets_after: Vec<i64> = db.deals(Some(account)).expect("deals").iter().map(|(_, d)| d.ticket).collect();
    assert_eq!(tickets_before, tickets_after, "same deals, same tickets, same position");
    let back = db.manual_trade(&key).expect("reads").expect("still the trader's");
    assert_eq!(back, ManualTrade { commission: -3.0, ..edited });
    let trades = trades_of(&db);
    assert_eq!(trades.len(), 1);
    assert_eq!((trades[0].key.clone(), trades[0].side), (key.clone(), Side::Short));
    assert_eq!(db.journal(&key).expect("reads"), Some(entry), "the journal and its images stay on the trade");
    assert!(
        db.search("vào sớm", 5).expect("searches").iter().any(|hit| hit.title.contains("EURUSD Sell")),
        "the search entry is titled by the edited trade"
    );
}

#[test]
fn what_is_now_given_is_no_longer_a_gap() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let bare = ManualTrade {
        volume: None,
        open_price: None,
        close_price: None,
        date_only: true,
        profit: None,
        result_r: Some(1.5),
        ..typed()
    };
    let key = db.add_manual_trade(account, &bare).expect("an R-only trade");
    let trade = &trades_of(&db)[0];
    assert!(!trade.money_known() && !trade.volume_known() && !trade.prices_known() && !trade.time_known());
    assert_eq!(trade.result_r, Some(1.5));
    assert_eq!(db.manual_trade(&key).expect("reads"), Some(ManualTrade { commission: 0.0, swap: 0.0, ..bare.clone() }));

    db.update_manual_trade(&key, &ManualTrade { volume: Some(0.2), ..bare.clone() }).expect("a lot is given");
    let trade = &trades_of(&db)[0];
    assert!(trade.volume_known() && !trade.prices_known() && !trade.time_known(), "only the lot became known");

    db.update_manual_trade(&key, &ManualTrade { profit: Some(80.0), date_only: false, ..typed() }).expect("all given");
    let trade = &trades_of(&db)[0];
    assert_eq!(trade.gaps, Gaps::NONE);
    assert!(trade.money_known());
    assert_eq!(trade.result_r, None, "no R was typed: it is read from the stop again");
}

#[test]
fn a_result_in_r_does_not_hide_money_that_is_given() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    db.add_manual_trade(account, &ManualTrade { result_r: Some(2.5), ..typed() }).expect("stored");
    let trade = &trades_of(&db)[0];
    assert!(trade.money_known(), "the trader gave the money");
    assert!((trade.net() - 272.0).abs() < 1e-9);
    assert_eq!(trade.r_multiple(None), Some(2.5), "the R the trader typed stands");
}

#[test]
fn a_trade_needs_money_or_an_r() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let none = ManualTrade { profit: None, result_r: None, ..typed() };
    assert!(db.add_manual_trade(account, &none).is_err());
    assert_eq!(db.deals(None).expect("deals").len(), 0);
}

#[test]
fn a_trade_of_the_terminal_keeps_its_facts() {
    let mut db = Db::in_memory().expect("opens");
    let key = mt5_trade(&mut db);
    assert_eq!(db.manual_trade(&key).expect("reads"), None);
    let error = db.update_manual_trade(&key, &typed()).expect_err("refused").to_string();
    assert!(error.contains("MT5"), "{error}");
    assert_eq!(db.deals(None).expect("deals")[0].1.price, 2650.0, "nothing changed");
}

#[test]
fn deleting_removes_the_deals_the_journal_and_its_search_entry_and_logs_tombstones() {
    let mut db = Db::in_memory().expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let (gone, kept) = (
        db.add_manual_trade(account, &typed()).expect("stored"),
        db.add_manual_trade(account, &ManualTrade { symbol: "GBPUSD".into(), ..typed() }).expect("stored"),
    );
    let entry = JournalEntry {
        notes: "ghi chú duy nhất".into(),
        screenshots: vec![shot("images/only-here.png"), shot("images/shared.png")],
        ..JournalEntry::default()
    };
    db.save_journal(&gone, &entry).expect("journal");
    db.save_journal(
        &kept,
        &JournalEntry {
            notes: "khác".into(), screenshots: vec![shot("images/shared.png")], ..JournalEntry::default()
        },
    )
    .expect("journal");
    assert_eq!(db.search("duy nhất", 5).expect("searches").len(), 1);
    let logged_before = db.pending_changes(0, 1000).expect("log").len();

    let deleted = db.delete_trade(&gone).expect("deleted");
    assert_eq!(deleted.trades, 1);
    assert_eq!(deleted.unused_images, vec!["images/only-here.png".to_owned()], "the image another journal uses stays");
    assert_eq!(trades_of(&db).len(), 1);
    assert_eq!(db.deals(Some(account)).expect("deals").len(), 2);
    assert_eq!(db.journal(&gone).expect("reads"), None);
    assert_eq!(db.journal(&kept).expect("reads").map(|j| j.notes), Some("khác".into()));
    assert!(db.search("duy nhất", 5).expect("searches").is_empty(), "no search entry of a deleted trade");

    let log = db.pending_changes(0, 1000).expect("log");
    let deletions: Vec<&str> =
        log[logged_before..].iter().filter(|c| c.value.is_none()).map(|c| c.entity.as_str()).collect();
    assert_eq!(deletions.iter().filter(|e| **e == entity::DEAL).count(), 2, "a tombstone for each deal");
    assert_eq!(deletions.iter().filter(|e| **e == entity::JOURNAL).count(), 1, "and for the journal");
}

#[test]
fn several_trades_are_deleted_in_one_transaction_and_a_missing_one_counts_for_nothing() {
    let mut db = Db::in_memory().expect("opens");
    let mt5 = mt5_trade(&mut db);
    let a = db.add_manual_trade(mt5.account_id, &typed()).expect("stored");
    let b = db.add_manual_trade(mt5.account_id, &typed()).expect("stored");
    let nothing = TradeKey { account_id: mt5.account_id, position_id: 999, segment: 0 };
    let deleted = db.delete_trades(&[a, b, mt5, nothing]).expect("deleted");
    assert_eq!(deleted.trades, 3);
    assert_eq!(db.deals(None).expect("deals").len(), 0);
}

#[test]
fn importing_a_report_again_brings_a_deleted_terminal_trade_back() {
    let mut db = Db::in_memory().expect("opens");
    let key = mt5_trade(&mut db);
    db.delete_trade(&key).expect("deleted");
    assert_eq!(trades_of(&db).len(), 0);
    mt5_trade(&mut db);
    assert_eq!(trades_of(&db).len(), 1);
}

#[test]
fn the_image_files_of_a_deleted_journal_go_unless_another_journal_uses_them() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut db = Db::open(dir.path().join("tradelens.db")).expect("opens");
    let account = db.upsert_account(&info("1", "A")).expect("account");
    let (gone, kept) = (
        db.add_manual_trade(account, &typed()).expect("stored"),
        db.add_manual_trade(account, &typed()).expect("stored"),
    );
    let images = dir.path().join("attachments").join("images");
    std::fs::create_dir_all(&images).expect("folder");
    for name in ["only.png", "shared.png"] {
        std::fs::write(images.join(name), b"png").expect("writes");
    }
    let both = vec![shot("images/only.png"), shot("images/shared.png")];
    db.save_journal(&gone, &JournalEntry { screenshots: both, ..JournalEntry::default() }).expect("journal");
    db.save_journal(&kept, &JournalEntry { screenshots: vec![shot("images/shared.png")], ..JournalEntry::default() })
        .expect("journal");
    db.delete_trade(&gone).expect("deleted");
    assert!(!images.join("only.png").exists(), "nothing else uses it");
    assert!(images.join("shared.png").exists(), "another journal still shows it");
}
