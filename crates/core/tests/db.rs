//! The SQLite storage through its public API: accounts, deals, journal, rules, checklist, day notes.

use chrono::NaiveDate;
use tradelens_core::{
    Db,
    attachments::{Attachment, Source},
    catalog::FieldValue,
    db::UpsertReport,
    journal::EntryPlan,
    model::{
        AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, DealEntry, DealKind, JournalEntry, ManualTrade,
        PositionLevels, Rules, Side, TradeKey,
    },
    time::parse_mt5_time,
    trades::build_trades,
};

fn info(balance: f64) -> AccountInfo {
    AccountInfo {
        login: "5012345".into(),
        server: "Broker-Live".into(),
        broker: "Broker Ltd".into(),
        name: "Chính".into(),
        currency: "USD".into(),
        balance: Some(balance),
        equity: Some(balance + 10.0),
        server_offset_min: Some(120),
    }
}

fn deal(ticket: i64, position: i64, at: &str, kind: DealKind, entry: DealEntry, price: f64, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: parse_mt5_time(at).expect("fixture times parse"),
        symbol: "XAUUSD".into(),
        kind,
        entry,
        volume: 0.1,
        price,
        commission: -0.7,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: 7,
        comment: "fixture".into(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

/// Two closed trades (positions 100 and 200) and a deposit.
fn batch() -> DealBatch {
    use DealEntry::{In, Out};
    use DealKind::{Balance, Buy, Sell};
    DealBatch {
        account: info(1000.0),
        deals: vec![
            deal(1, 0, "2026.11.01 00:00:00", Balance, DealEntry::None, 0.0, 1000.0),
            deal(2, 100, "2026.11.20 09:00:00", Buy, In, 2650.0, 0.0),
            deal(3, 100, "2026.11.20 10:00:00", Sell, Out, 2655.0, 50.0),
            deal(4, 200, "2026.11.20 11:00:00", Sell, In, 2660.0, 0.0),
            deal(5, 200, "2026.11.20 12:00:00", Buy, Out, 2662.0, -20.0),
        ],
        positions: Vec::new(),
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).expect("a real day")
}

#[test]
fn upserting_the_same_batch_twice_adds_nothing() {
    let mut db = Db::in_memory().unwrap();
    let first = db.upsert_batch(&batch(), "ea").unwrap();
    assert_eq!((first.inserted, first.updated), (5, 0));

    let second = db.upsert_batch(&batch(), "ea").unwrap();
    assert_eq!(second, UpsertReport { account_id: first.account_id, inserted: 0, updated: 0 });
    assert_eq!(db.deals(None).unwrap().len(), 5, "no duplicate deals");
    assert_eq!(db.accounts().unwrap().len(), 1);
}

#[test]
fn the_import_log_lists_what_was_stored_newest_first() {
    let mut db = Db::in_memory().unwrap();
    assert_eq!(db.import_log(10).unwrap().len(), 0, "nothing imported yet");
    db.upsert_batch(&batch(), "html").unwrap();
    db.upsert_batch(&batch(), "html").unwrap();
    let mut next = batch();
    next.deals.push(deal(6, 300, "2026.11.21 08:00:00", DealKind::Buy, DealEntry::In, 2670.0, 0.0));
    db.upsert_batch(&next, "xlsx").unwrap();

    let log = db.import_log(10).unwrap();
    assert_eq!(log.len(), 2, "a batch that stored nothing leaves no line");
    assert_eq!((log[0].source.as_str(), log[0].inserted, log[0].updated), ("xlsx", 1, 0));
    assert_eq!((log[1].source.as_str(), log[1].inserted), ("html", 5));
    assert!(log[1].account_id.is_some());
    assert_eq!(db.import_log(1).unwrap().len(), 1, "the limit applies");
}

#[test]
fn a_changed_deal_is_updated_and_new_ones_inserted() {
    let mut db = Db::in_memory().unwrap();
    db.upsert_batch(&batch(), "ea").unwrap();

    let mut next = batch();
    next.deals[2].profit = 55.0;
    next.deals.push(deal(6, 300, "2026.11.21 08:00:00", DealKind::Buy, DealEntry::In, 2670.0, 0.0));
    let report = db.upsert_batch(&next, "ea").unwrap();
    assert_eq!((report.inserted, report.updated), (1, 1));

    let stored = db.deals(Some(report.account_id)).unwrap();
    let closed = stored.iter().find(|(_, d)| d.ticket == 3).expect("the deal is stored");
    assert!((closed.1.profit - 55.0).abs() < 1e-9);
}

#[test]
fn deals_round_trip_and_a_report_keeps_what_the_ea_measured() {
    let mut db = Db::in_memory().unwrap();
    let mut from_ea = batch();
    from_ea.deals[1].risk = Some(25.0);
    from_ea.deals[1].sl = 2640.0;
    from_ea.deals[2].reason = Some("tp".into());
    db.upsert_batch(&from_ea, "ea").unwrap();
    let stored: Vec<Deal> = db.deals(None).unwrap().into_iter().map(|(_, d)| d).collect();
    assert_eq!(stored, from_ea.deals, "every field comes back, oldest first");

    // The same deals from an HTML report: no risk, reason or stop, but one more deal.
    let mut report = batch();
    report.deals.push(deal(6, 300, "2026.11.21 08:00:00", DealKind::Buy, DealEntry::In, 2670.0, 0.0));
    let outcome = db.upsert_batch(&report, "html").unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (1, 0), "nothing the report knows is different");
    let kept = db.deals(None).unwrap();
    assert_eq!(kept[1].1.risk, Some(25.0));
    assert!((kept[1].1.sl - 2640.0).abs() < 1e-9);
    assert_eq!(kept[2].1.reason.as_deref(), Some("tp"));
}

#[test]
fn the_risk_measured_first_stays_when_a_later_poll_measures_it_again() {
    let mut db = Db::in_memory().unwrap();
    let mut first = batch();
    first.deals[1].risk = Some(25.0);
    db.upsert_batch(&first, "mt5").unwrap();
    let mut again = batch();
    again.deals[1].risk = Some(25.4);
    let outcome = db.upsert_batch(&again, "mt5").unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 0), "a moving tick value is not a change");
    assert_eq!(db.deals(None).unwrap()[1].1.risk, Some(25.0));
}

#[test]
fn position_levels_fill_deals_without_a_stop() {
    let mut db = Db::in_memory().unwrap();
    let mut with_levels = batch();
    with_levels.positions = vec![PositionLevels { position_id: 100, sl: 2640.0, tp: 2660.0 }];
    db.upsert_batch(&with_levels, "html").unwrap();
    let deals: Vec<Deal> = db.deals(None).unwrap().into_iter().map(|(_, d)| d).collect();
    let trades = build_trades(1, &deals);
    let first = trades.iter().find(|t| t.key.position_id == 100).expect("position 100 is a trade");
    assert!((first.sl - 2640.0).abs() < 1e-9 && (first.tp - 2660.0).abs() < 1e-9);
    let second = trades.iter().find(|t| t.key.position_id == 200).expect("position 200 is a trade");
    assert!(second.sl.abs() < 1e-9, "other positions are untouched");
}

#[test]
fn a_batch_without_login_is_refused_and_stores_nothing() {
    let mut db = Db::in_memory().unwrap();
    let mut bad = batch();
    bad.account.login = "  ".into();
    assert!(db.upsert_batch(&bad, "ea").is_err());
    assert_eq!(db.deals(None).unwrap(), Vec::new());
    assert_eq!(db.accounts().unwrap(), Vec::new());
}

#[test]
fn accounts_are_found_by_login_and_server_and_refreshed() {
    let mut db = Db::in_memory().unwrap();
    let id = db.upsert_account(&info(1000.0)).unwrap();

    let mut later = info(1250.5);
    later.equity = Some(1300.0);
    later.server_offset_min = Some(180);
    later.name = "Tên khác".into();
    assert_eq!(db.upsert_account(&later).unwrap(), id, "same login and server, same account");

    let other = AccountInfo { server: "Broker-Demo".into(), ..info(5.0) };
    let demo = db.upsert_account(&other).unwrap();
    assert_ne!(demo, id, "same login on another server is another account");

    let accounts = db.accounts().unwrap();
    assert_eq!(accounts.len(), 2);
    let account = &accounts[0];
    assert_eq!(account.id, id);
    assert_eq!(account.last_balance, Some(1250.5));
    assert_eq!(account.last_equity, Some(1300.0));
    assert_eq!(account.server_offset_min, Some(180));
    assert_eq!(account.name, "Chính", "a name already set is not replaced by the sync");
    assert!(account.last_sync.is_some());
}

#[test]
fn a_report_without_balance_keeps_the_last_known_one() {
    let mut db = Db::in_memory().unwrap();
    db.upsert_account(&info(1000.0)).unwrap();
    let bare = AccountInfo { login: "5012345".into(), server: "Broker-Live".into(), ..AccountInfo::default() };
    db.upsert_account(&bare).unwrap();
    let account = &db.accounts().unwrap()[0];
    assert_eq!(account.last_balance, Some(1000.0));
    assert_eq!(account.broker, "Broker Ltd");
    assert_eq!(account.currency, "USD");
}

#[test]
fn new_accounts_default_to_usd() {
    let mut db = Db::in_memory().unwrap();
    db.upsert_account(&AccountInfo { login: "9".into(), ..AccountInfo::default() }).unwrap();
    assert_eq!(db.accounts().unwrap()[0].currency, "USD");
}

#[test]
fn the_trader_edits_the_account_but_not_its_identity_or_balance() {
    let mut db = Db::in_memory().unwrap();
    let id = db.upsert_account(&info(1000.0)).unwrap();
    let mut account = db.accounts().unwrap().remove(0);
    account.name = "Quỹ nhỏ".into();
    account.initial_balance = Some(900.0);
    account.currency = "EUR".into();
    account.login = "changed".into();
    account.last_balance = Some(1.0);
    db.update_account(&account).unwrap();

    let read = db.accounts().unwrap().remove(0);
    assert_eq!((read.id, read.name.as_str(), read.currency.as_str()), (id, "Quỹ nhỏ", "EUR"));
    assert_eq!(read.initial_balance, Some(900.0));
    assert_eq!(read.login, "5012345");
    assert_eq!(read.last_balance, Some(1000.0));

    account.id = 999;
    assert!(db.update_account(&account).is_err(), "an unknown account is an error");
}

#[test]
fn deals_of_one_account_or_all() {
    let mut db = Db::in_memory().unwrap();
    let first = db.upsert_batch(&batch(), "ea").unwrap().account_id;
    let mut other = batch();
    other.account = AccountInfo { login: "777".into(), ..info(10.0) };
    other.deals.truncate(3);
    let second = db.upsert_batch(&other, "ea").unwrap().account_id;
    assert_ne!(first, second);

    assert_eq!(db.deals(Some(first)).unwrap().len(), 5);
    assert_eq!(db.deals(Some(second)).unwrap().len(), 3);
    let all = db.deals(None).unwrap();
    assert_eq!(all.len(), 8, "the same tickets in two accounts are different deals");
    assert!(all.iter().all(|(account, _)| *account == first || *account == second));
    assert_eq!(db.deals(Some(12345)).unwrap(), Vec::new());
}

fn manual(side: Side, open: &str, close: &str) -> ManualTrade {
    ManualTrade {
        symbol: "EURUSD".into(),
        side,
        volume: Some(0.5),
        open_time: parse_mt5_time(open).expect("fixture times parse"),
        open_price: Some(1.1000),
        close_time: parse_mt5_time(close).expect("fixture times parse"),
        close_price: Some(1.1040),
        date_only: false,
        sl: 1.0980,
        tp: 1.1050,
        profit: Some(200.0),
        commission: -3.5,
        swap: -0.4,
        result_r: None,
    }
}

#[test]
fn a_manual_trade_is_two_negative_deals_forming_one_trade() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_batch(&batch(), "ea").unwrap().account_id;

    let key =
        db.add_manual_trade(account_id, &manual(Side::Long, "2026.11.19 08:00:00", "2026.11.19 09:00:00")).unwrap();
    assert_eq!((key.account_id, key.segment), (account_id, 0));
    assert!(key.position_id < 0);

    let deals: Vec<Deal> = db.deals(Some(account_id)).unwrap().into_iter().map(|(_, d)| d).collect();
    let manual_deals: Vec<&Deal> = deals.iter().filter(|d| d.ticket < 0).collect();
    assert_eq!(manual_deals.len(), 2);
    assert!(manual_deals.iter().all(|d| d.position_id == key.position_id));

    let trades = build_trades(account_id, &deals);
    assert_eq!(trades.len(), 3, "two imported trades and the manual one");
    let trade = trades.iter().find(|t| t.key == key).expect("the manual trade is built");
    assert!(trade.manual && trade.is_closed());
    assert_eq!(trade.side, Side::Long);
    assert!((trade.volume - 0.5).abs() < 1e-9);
    assert!((trade.profit - 200.0).abs() < 1e-9);
    assert!((trade.net() - 196.1).abs() < 1e-9);
    assert!((trade.sl - 1.0980).abs() < 1e-9);
    assert!((trade.close_price.unwrap() - 1.1040).abs() < 1e-9);
}

#[test]
fn manual_trades_get_distinct_tickets_and_survive_a_zero_length_trade() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_account(&info(1000.0)).unwrap();
    let a =
        db.add_manual_trade(account_id, &manual(Side::Short, "2026.11.19 08:00:00", "2026.11.19 08:00:00")).unwrap();
    let b = db.add_manual_trade(account_id, &manual(Side::Long, "2026.11.19 08:00:00", "2026.11.19 08:30:00")).unwrap();
    assert_ne!(a.position_id, b.position_id);

    let deals: Vec<Deal> = db.deals(None).unwrap().into_iter().map(|(_, d)| d).collect();
    let mut tickets: Vec<i64> = deals.iter().map(|d| d.ticket).collect();
    tickets.sort_unstable();
    tickets.dedup();
    assert_eq!(tickets.len(), 4);
    assert!(tickets.iter().all(|t| *t < 0));
    let trades = build_trades(account_id, &deals);
    assert_eq!(trades.len(), 2);
    assert!(
        trades.iter().all(tradelens_core::Trade::is_closed),
        "an entry sorts before its exit even at the same instant"
    );
    let short = trades.iter().find(|t| t.key == a).expect("the short is built");
    assert_eq!(short.side, Side::Short);
}

#[test]
fn bad_manual_trades_are_refused() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_account(&info(1000.0)).unwrap();
    let good = manual(Side::Long, "2026.11.19 08:00:00", "2026.11.19 09:00:00");
    assert!(db.add_manual_trade(account_id, &ManualTrade { symbol: " ".into(), ..good.clone() }).is_err());
    assert!(db.add_manual_trade(account_id, &ManualTrade { volume: Some(0.0), ..good.clone() }).is_err());
    assert!(db.add_manual_trade(account_id, &ManualTrade { open_price: Some(f64::NAN), ..good.clone() }).is_err());
    assert!(db.add_manual_trade(404, &good).is_err(), "the account must exist");
    assert_eq!(db.deals(None).unwrap(), Vec::new(), "nothing half written");
}

fn full_entry() -> JournalEntry {
    JournalEntry {
        playbook_id: Some(3),
        setup: "Breakout phiên Á".into(),
        tags: vec!["london".into(), "tin".into()],
        mistakes: vec!["dời SL".into()],
        emotion_before: "tự tin".into(),
        emotion_after: "tiếc".into(),
        rating: 4,
        setup_grade: Some("A+".into()),
        exit_reason: Some("tp".into()),
        plan: Some(EntryPlan {
            thesis: "Retest vùng 2650 giữ được".into(),
            entry: Some(2650.0),
            stop_loss: Some(2640.0),
            take_profit: Some(2670.0),
            risk: Some(25.0),
            timeframe: "M15".into(),
            written_at: 1_763_000_000_000,
        }),
        market: Some("trending".into()),
        review_later: true,
        custom: [
            ("focus".to_owned(), FieldValue::Scale(8)),
            ("why".to_owned(), FieldValue::Text("ngủ đủ giấc".into())),
            ("tf".to_owned(), FieldValue::Choices(vec!["M5".into(), "M15".into()])),
        ]
        .into(),
        followed_plan: Some(false),
        checklist: vec![1, 2],
        notes: "Chốt non vì sợ tin Mỹ".into(),
        screenshots: vec![Attachment {
            path: "ab/cd.png".into(),
            caption: "Biểu đồ H1 lúc vào".into(),
            timeframe: Some("h1".into()),
            source: Source::EaEntry,
            added_at: 1_763_000_000_001,
        }],
        planned_risk: Some(25.0),
    }
}

fn changes(path: &std::path::Path) -> Vec<(String, i64, String, String, Option<String>)> {
    let conn = rusqlite::Connection::open(path).expect("the database opens a second time");
    let mut stmt = conn
        .prepare("SELECT device_id, seq, entity, key, value FROM changes ORDER BY device_id, seq")
        .expect("the changelog exists");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .expect("the query runs")
        .collect::<Result<_, _>>()
        .expect("the rows read")
}

#[test]
fn a_journal_entry_round_trips_every_field_and_is_logged_for_sync() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tradelens.db");
    let mut db = Db::open(&path).unwrap();
    let account_id = db.upsert_batch(&batch(), "ea").unwrap().account_id;
    let key = TradeKey { account_id, position_id: 100, segment: 0 };
    let entry = full_entry();

    assert_eq!(db.journal(&key).unwrap(), None);
    db.save_journal(&key, &entry).unwrap();
    assert_eq!(db.journal(&key).unwrap(), Some(entry.clone()));

    // The import logged the account and its deals, then the journal followed: under a portable
    // key (login, server, position, segment), as the fields that were written.
    let log = changes(&path);
    let journal: Vec<_> = log.iter().filter(|c| c.2 == "journal").collect();
    assert_eq!(journal.len(), 1);
    let (device, seq, entity, logged_key, value) = journal[0];
    assert_eq!(device.len(), 32, "a 128-bit hex id");
    assert_eq!((entity.as_str(), logged_key.as_str()), ("journal", "5012345|Broker-Live|100|0"));
    assert_eq!(*seq, i64::try_from(log.len()).unwrap(), "the journal is the last write, numbered after the rest");
    let ops: Vec<tradelens_core::sync::FieldOp> =
        serde_json::from_str(value.as_deref().expect("a save is not a delete")).unwrap();
    let written: Vec<&str> = ops.iter().map(|op| op.field.as_str()).collect();
    for field in
        ["notes", "rating", "tags#london", "tags#tin", "mistakes#dời SL", "custom.focus", "plan", "followed_plan"]
    {
        assert!(written.contains(&field), "{field} in {written:?}");
    }
    assert!(!written.contains(&"playbook"), "a link to a setup this database does not have is not synced");
    assert!(written.contains(&"screenshots#ab/cd.png") && written.contains(&"screenshots.order"));
    assert_eq!(log.iter().filter(|c| c.2 == "account").count(), 1);
    assert_eq!(log.iter().filter(|c| c.2 == "deal").count(), 5);

    // Saving the same thing again changes nothing; saving a change logs only what changed.
    db.save_journal(&key, &entry).unwrap();
    assert_eq!(changes(&path).len(), log.len());
    let edited = JournalEntry { rating: 2, ..entry };
    db.save_journal(&key, &edited).unwrap();
    assert_eq!(db.journal(&key).unwrap(), Some(edited));
    let log = changes(&path);
    let last = log.last().expect("a change was logged");
    assert_eq!(last.1, *seq + 1, "numbered after the previous one");
    assert_eq!(last.0, *device, "one device id");
    assert_eq!(last.4.as_deref(), Some(r#"[{"f":"rating","v":2}]"#), "only the rating changed");
}

#[test]
fn journals_are_listed_per_account_and_indexed_for_search() {
    let mut db = Db::in_memory().unwrap();
    let first = db.upsert_batch(&batch(), "ea").unwrap().account_id;
    let other = AccountInfo { login: "777".into(), ..info(10.0) };
    let second = db.upsert_account(&other).unwrap();

    let key = TradeKey { account_id: first, position_id: 100, segment: 0 };
    let note = |text: &str| JournalEntry { notes: text.into(), ..JournalEntry::default() };
    db.save_journal(&key, &note("Dời SL lần nữa, vào lệnh sớm")).unwrap();
    db.save_journal(&TradeKey { position_id: 200, ..key.clone() }, &note("Đúng kế hoạch")).unwrap();
    db.save_journal(&TradeKey { account_id: second, position_id: 5, segment: 0 }, &note("Tài khoản khác")).unwrap();

    assert_eq!(db.journals(Some(first)).unwrap().len(), 2);
    assert_eq!(db.journals(Some(second)).unwrap().len(), 1);
    let all = db.journals(None).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].0, key, "ordered by account, position, segment");

    let hits = db.search("doi sl", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, key.id());
    assert_eq!(hits[0].title, "XAUUSD Buy 20/11");

    db.save_journal(&key, &note("Giữ lệnh tới TP")).unwrap();
    assert_eq!(db.search("doi sl", 10).unwrap(), Vec::new(), "the old text leaves the index");
    assert_eq!(db.search("giu lenh", 10).unwrap().len(), 1);
}

#[test]
fn a_journal_for_a_missing_account_is_refused() {
    let mut db = Db::in_memory().unwrap();
    let key = TradeKey { account_id: 42, position_id: 1, segment: 0 };
    assert!(db.save_journal(&key, &full_entry()).is_err());
    assert_eq!(db.search("breakout", 10).unwrap(), Vec::new(), "the failed save left no index entry");
}

#[test]
fn rules_default_to_off_and_round_trip() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_account(&info(1000.0)).unwrap();
    assert_eq!(db.rules(account_id).unwrap(), Rules::default());

    let rules = Rules {
        max_daily_loss: Some(50.0),
        max_daily_trades: Some(3),
        max_risk_pct: Some(1.5),
        max_consecutive_losses: Some(2),
        daily_target: Some(100.0),
    };
    db.save_rules(account_id, &rules).unwrap();
    assert_eq!(db.rules(account_id).unwrap(), rules);

    let relaxed = Rules { max_daily_trades: None, ..rules };
    db.save_rules(account_id, &relaxed).unwrap();
    assert_eq!(db.rules(account_id).unwrap(), relaxed, "a rule turned off stays off");
    assert!(db.save_rules(404, &relaxed).is_err());
}

#[test]
fn checklists_keep_item_ids_and_separate_global_from_account_items() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_account(&info(1000.0)).unwrap();
    let item = |id: i64, text: &str| ChecklistItem { id, text: text.into() };

    db.save_checklist(None, &[item(0, "Có kế hoạch?"), item(0, "Đã đặt SL?")]).unwrap();
    db.save_checklist(Some(account_id), &[item(0, "Không trước tin")]).unwrap();

    let global = db.checklist(None).unwrap();
    assert_eq!(global.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["Có kế hoạch?", "Đã đặt SL?"]);
    let mine = db.checklist(Some(account_id)).unwrap();
    assert_eq!(mine.len(), 3);
    assert_eq!(mine[2].text, "Không trước tin", "global items first, then the account's");

    // Reorder, edit one and drop one: the survivors keep their ids.
    let (plan, stop) = (global[0].clone(), global[1].clone());
    db.save_checklist(None, &[item(stop.id, "Đã đặt SL và TP?"), item(0, "Risk ≤ 1%")]).unwrap();
    let updated = db.checklist(None).unwrap();
    assert_eq!(updated.len(), 2);
    assert_eq!(updated[0], item(stop.id, "Đã đặt SL và TP?"));
    assert!(updated.iter().all(|i| i.id != plan.id), "the dropped item is gone");
    assert_eq!(db.checklist(Some(account_id)).unwrap().len(), 3, "the account's own item is untouched");

    db.save_checklist(Some(account_id), &[]).unwrap();
    assert_eq!(db.checklist(Some(account_id)).unwrap().len(), 2);
}

#[test]
fn day_notes_round_trip_per_account_and_are_searchable() {
    let mut db = Db::in_memory().unwrap();
    let account_id = db.upsert_account(&info(1000.0)).unwrap();
    assert_eq!(db.day_note(account_id, day(20)).unwrap(), DayNote::default());

    let note = DayNote {
        plan: "Chỉ giao dịch phiên London".into(),
        review: "Giữ đúng kỷ luật".into(),
        mood: "bình tĩnh".into(),
    };
    db.save_day_note(account_id, day(20), &note).unwrap();
    assert_eq!(db.day_note(account_id, day(20)).unwrap(), note);
    assert_eq!(db.day_note(account_id, day(21)).unwrap(), DayNote::default(), "another day");
    assert_eq!(db.day_note(0, day(20)).unwrap(), DayNote::default(), "another scope");

    let edited = DayNote { mood: "mệt".into(), ..note };
    db.save_day_note(account_id, day(20), &edited).unwrap();
    assert_eq!(db.day_note(account_id, day(20)).unwrap(), edited);

    let all_accounts = DayNote { plan: "Nghỉ giao dịch".into(), ..DayNote::default() };
    db.save_day_note(0, day(20), &all_accounts).unwrap();
    assert_eq!(db.day_note(0, day(20)).unwrap(), all_accounts);

    let hits = db.search("london", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, format!("2026-11-20:{account_id}"));
    let hits = db.search("nghi", 10).unwrap();
    assert_eq!(hits.len(), 1, "the note of all accounts is indexed apart from the account's");
    assert_eq!(hits[0].reference, "2026-11-20");
}
