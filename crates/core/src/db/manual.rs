//! Trades the trader owns: typed in, imported from their own journal, edited afterwards, and
//! deleted. A trade of the terminal can be deleted too, but its facts are the terminal's and
//! stay as reported.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use rusqlite::{Connection, TransactionBehavior};

use super::{
    DEAL_COLUMNS, Db, DbError, MANUAL_SOURCE, Result, deal_from_row_at, enum_name, journal_document, journal_on, sync,
    views, write_deal, write_index,
};
use crate::legacy::ATTACHMENTS_DIR;
use crate::model::{Deal, DealEntry, DealKind, JOURNAL_TICKET_BASE, JournalEntry, ManualTrade, Origin, Side, TradeKey};
use crate::search::DocKind;
use crate::symbol;
use crate::sync::entity;

/// The lot a trade that has none is stored with: a placeholder, marked by [`crate::Gaps::VOLUME`].
const PLACEHOLDER_VOLUME: f64 = 1.0;

/// What deleting trades removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Deleted {
    /// How many of the trades asked for were there.
    pub trades: usize,
    /// Images of the deleted journals that no journal, missed trade or setup uses any more, as paths
    /// relative to the data folder's `attachments/`; their files are removed.
    pub unused_images: Vec<String>,
}

/// The two deals of a trade that has exactly one entry and one exit, as stored.
struct Parts {
    entry: Deal,
    exit: Deal,
    origin: Origin,
    entry_source: String,
    exit_source: String,
}

impl Parts {
    /// The trade as the form for it shows it.
    fn to_manual(&self) -> ManualTrade {
        let (entry, exit) = (&self.entry, &self.exit);
        let gaps = entry.gaps.with(exit.gaps);
        let money = exit.result_r.is_none() || exit.profit + entry.profit != 0.0;
        let given = |value: f64, missing: bool| (!missing && value > 0.0).then_some(value);
        let prices_missing = gaps.has(crate::Gaps::PRICE);
        ManualTrade {
            symbol: entry.symbol.clone(),
            side: if entry.kind == DealKind::Buy { Side::Long } else { Side::Short },
            volume: given(entry.volume, gaps.has(crate::Gaps::VOLUME)),
            open_time: entry.time_msc,
            open_price: given(entry.price, prices_missing),
            close_time: exit.time_msc,
            close_price: given(exit.price, prices_missing),
            date_only: gaps.has(crate::Gaps::TIME),
            sl: entry.sl.max(exit.sl),
            tp: entry.tp.max(exit.tp),
            profit: money.then_some(exit.profit + entry.profit),
            commission: if money { entry.commission + exit.commission + entry.fee + exit.fee } else { 0.0 },
            swap: if money { entry.swap + exit.swap } else { 0.0 },
            result_r: exit.result_r,
        }
    }
}

/// The deals of the position, ordered as the trade is built; `None` when there are none.
fn position_deals(conn: &Connection, key: &TradeKey) -> Result<Vec<(Deal, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DEAL_COLUMNS}, source FROM deals
         WHERE account_id = ?1 AND (position_id = ?2 OR (position_id = 0 AND ticket = ?2))
         ORDER BY time_msc, ticket"
    ))?;
    let rows = stmt.query_map(rusqlite::params![key.account_id, key.position_id], |r| {
        Ok((deal_from_row_at(r, 0)?, r.get::<_, String>(21)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The two deals of the trade, `Ok(None)` when it has no deals, and an error in Vietnamese when it
/// is not made of one entry and one exit.
fn load_parts(conn: &Connection, key: &TradeKey) -> Result<Option<Parts>> {
    let deals: Vec<(Deal, String)> =
        position_deals(conn, key)?.into_iter().filter(|(deal, _)| deal.kind.is_trade()).collect();
    if deals.is_empty() {
        return Ok(None);
    }
    let many = || DbError::Invalid(crate::tr!("core.manual.many_deals"));
    let [first, second] = deals.as_slice() else { return Err(many()) };
    let (entry, exit) = match (first.0.entry, second.0.entry) {
        (DealEntry::In | DealEntry::None, DealEntry::Out | DealEntry::OutBy) => (first, second),
        (DealEntry::Out | DealEntry::OutBy, DealEntry::In | DealEntry::None) => (second, first),
        _ => return Err(many()),
    };
    // One source decides: a trade is never half the trader's and half the terminal's.
    let origin = if Origin::of_source(&entry.1).is_editable() && Origin::of_source(&exit.1).is_editable() {
        Origin::of_source(&entry.1)
    } else {
        Origin::Mt5
    };
    Ok(Some(Parts {
        entry: entry.0.clone(),
        exit: exit.0.clone(),
        origin,
        entry_source: entry.1.clone(),
        exit_source: exit.1.clone(),
    }))
}

/// `trade` as it will be stored, or what is wrong with it.
fn checked(trade: &ManualTrade) -> Result<ManualTrade> {
    let symbol = symbol::normalise(&trade.symbol);
    if symbol.is_empty() {
        return Err(DbError::Invalid(crate::tr!("core.manual.no_symbol")));
    }
    for (label, value) in [
        ("core.manual.volume", trade.volume),
        ("core.manual.open_price", trade.open_price),
        ("core.manual.close_price", trade.close_price),
    ] {
        if value.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Err(DbError::Invalid(crate::tr!("core.manual.must_be_positive", label = crate::tr!(label))));
        }
    }
    for (label, value) in [
        ("core.manual.stop_loss", Some(trade.sl)),
        ("core.manual.take_profit", Some(trade.tp)),
        ("core.manual.result_r", trade.result_r),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(DbError::Invalid(crate::tr!("core.manual.must_be_number", label = crate::tr!(label))));
        }
    }
    if [trade.profit, Some(trade.commission), Some(trade.swap)].iter().flatten().any(|v| !v.is_finite()) {
        return Err(DbError::Invalid(crate::tr!("core.manual.money_must_be_numbers")));
    }
    if trade.profit.is_none() && trade.result_r.is_none() {
        return Err(DbError::Invalid(crate::tr!("core.manual.need_profit_or_r")));
    }
    if trade.close_time < trade.open_time {
        return Err(DbError::Invalid(crate::tr!("core.manual.close_before_open")));
    }
    // A result in R next to a profit of exactly 0 is a trade kept in R: it has no money to count.
    let profit = trade.profit.filter(|profit| *profit != 0.0 || trade.result_r.is_none());
    let (commission, swap) = if profit.is_some() { (trade.commission, trade.swap) } else { (0.0, 0.0) };
    Ok(ManualTrade { symbol, profit, commission, swap, ..trade.clone() })
}

/// The entry and exit deals of `trade`, with the given tickets; the entry's ticket is the position.
fn deals_of(trade: &ManualTrade, entry_ticket: i64, exit_ticket: i64) -> (Deal, Deal) {
    let (entry_kind, exit_kind) = match trade.side {
        Side::Long => (DealKind::Buy, DealKind::Sell),
        Side::Short => (DealKind::Sell, DealKind::Buy),
    };
    let gaps = trade.gaps();
    let deal = |ticket: i64, kind: DealKind, entry: DealEntry, time_msc: i64, price: Option<f64>| Deal {
        ticket,
        order: ticket,
        position_id: entry_ticket,
        time_msc,
        symbol: trade.symbol.clone(),
        kind,
        entry,
        volume: trade.volume.unwrap_or(PLACEHOLDER_VOLUME),
        price: price.unwrap_or(0.0),
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: 0.0,
        magic: 0,
        comment: String::new(),
        sl: trade.sl,
        tp: trade.tp,
        risk: None,
        result_r: None,
        gaps,
        reason: None,
    };
    let entry = deal(entry_ticket, entry_kind, DealEntry::In, trade.open_time, trade.open_price);
    let exit = Deal {
        commission: trade.commission,
        swap: trade.swap,
        profit: trade.profit.unwrap_or(0.0),
        result_r: trade.result_r,
        ..deal(exit_ticket, exit_kind, DealEntry::Out, trade.close_time, trade.close_price)
    };
    (entry, exit)
}

impl Db {
    /// Stores a trade the trader typed as an entry and an exit deal with negative tickets, in the
    /// range of hand-entered tickets (above an imported journal's, whatever the account holds).
    ///
    /// The entry deal gets the lower ticket of the two, so that the two sort entry first
    /// even when they share a timestamp. The deals' position id is the entry's ticket;
    /// commission, swap and profit go on the exit. The symbol is stored normalised
    /// ([`symbol::normalise`]).
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when the symbol is empty, a lot or a price is not a positive number,
    /// neither the profit nor the result in R is given, or the trade closes before it opens;
    /// [`DbError::Sqlite`] when the account does not exist or the write fails.
    pub fn add_manual_trade(&mut self, account_id: i64, trade: &ManualTrade) -> Result<TradeKey> {
        let trade = checked(trade)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lowest: i64 = tx.query_row(
            "SELECT COALESCE(MIN(ticket), 0) FROM deals WHERE account_id = ?1 AND ticket <= 0 AND ticket > ?2",
            rusqlite::params![account_id, -JOURNAL_TICKET_BASE],
            |r| r.get(0),
        )?;
        let exit_ticket = lowest.min(0) - 1;
        let entry_ticket = exit_ticket - 1;
        let (entry, exit) = deals_of(&trade, entry_ticket, exit_ticket);
        write_deal(&tx, account_id, &entry, MANUAL_SOURCE)?;
        write_deal(&tx, account_id, &exit, MANUAL_SOURCE)?;
        tx.commit()?;
        Ok(TradeKey { account_id, position_id: entry_ticket, segment: 0 })
    }

    /// The facts of a trade the trader owns, to fill the form that edits them; `None` for a trade
    /// of the terminal, one made of more than an entry and an exit, or one that is not there.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn manual_trade(&self, key: &TradeKey) -> Result<Option<ManualTrade>> {
        let parts = match load_parts(&self.conn, key) {
            Ok(parts) => parts,
            Err(DbError::Invalid(_)) => None,
            Err(other) => return Err(other),
        };
        Ok(parts.filter(|p| p.origin.is_editable()).map(|p| p.to_manual()))
    }

    /// Rewrites the facts of a trade the trader owns, keeping its two deals' tickets and position,
    /// so that its journal, images and links stay on it. One transaction; every changed deal is
    /// logged for sync, and what is now given is no longer a gap (a lot, prices, a time of day).
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when the trade is not there, comes from the terminal (its facts are the
    /// terminal's), is made of more than an entry and an exit, or `trade` is not valid (see
    /// [`Db::add_manual_trade`]); [`DbError::Sqlite`] when the write fails.
    pub fn update_manual_trade(&mut self, key: &TradeKey, trade: &ManualTrade) -> Result<()> {
        let trade = checked(trade)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parts = load_parts(&tx, key)?.ok_or_else(|| DbError::Invalid(crate::tr!("core.manual.not_found")))?;
        if !parts.origin.is_editable() {
            return Err(DbError::Invalid(crate::tr!("core.manual.from_mt5")));
        }
        let (entry, exit) = deals_of(&trade, parts.entry.ticket, parts.exit.ticket);
        // What the trader never typed (the title a journal import gave, the magic number) stays.
        let keep = |new: Deal, old: &Deal| Deal {
            order: old.order,
            magic: old.magic,
            comment: old.comment.clone(),
            risk: old.risk,
            reason: old.reason.clone(),
            ..new
        };
        write_deal(&tx, key.account_id, &keep(entry, &parts.entry), &parts.entry_source)?;
        write_deal(&tx, key.account_id, &keep(exit, &parts.exit), &parts.exit_source)?;
        // The journal's search entry is titled by the symbol, side and day of its trade.
        for (journal_key, entry) in journals_of_position(&tx, key)? {
            write_index(&tx, &journal_document(&tx, &journal_key, &entry)?)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Where each trade came from, by (account id, position id), told by how its deals were
    /// written (their `source`), not by their tickets or the account they are filed under.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn trade_origins(&self, account_id: Option<i64>) -> Result<HashMap<(i64, i64), Origin>> {
        let mut stmt = self.conn.prepare(
            "SELECT account_id, CASE WHEN position_id = 0 THEN ticket ELSE position_id END, source FROM deals
             WHERE kind IN ('buy', 'sell') AND (?1 IS NULL OR account_id = ?1)",
        )?;
        let rows =
            stmt.query_map([account_id], |r| Ok(((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?), r.get::<_, String>(2)?)))?;
        let mut origins: HashMap<(i64, i64), Origin> = HashMap::new();
        for row in rows {
            let (position, source) = row?;
            let origin = Origin::of_source(&source);
            // A position the trader's own deals are part of is theirs; a mixed one cannot be edited.
            let slot = origins.entry(position).or_insert(origin);
            if *slot != origin {
                *slot = Origin::Mt5;
            }
        }
        Ok(origins)
    }

    /// Deletes one trade: see [`Db::delete_trades`].
    ///
    /// # Errors
    ///
    /// As [`Db::delete_trades`].
    pub fn delete_trade(&mut self, key: &TradeKey) -> Result<Deleted> {
        self.delete_trades(std::slice::from_ref(key))
    }

    /// Deletes trades: their deals, their journals and the search entries of the journals, in one
    /// transaction. Each deletion is logged for sync as a deletion of the deal or journal (it
    /// leaves a tombstone), so the other devices delete the trade too when they sync; a newer
    /// edit made elsewhere brings back what it edited (`docs/cloud-sync.md`).
    ///
    /// A position's deals belong to all its segments (a netting position reversed is two trades),
    /// so deleting one deletes the position. Importing the same report again brings a deleted MT5
    /// trade back.
    ///
    /// The image files of the deleted journals that no other journal, missed trade or setup uses are
    /// removed from the data folder's `attachments/` (a database in memory has none) and listed in
    /// [`Deleted::unused_images`].
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails, in which case nothing is deleted.
    pub fn delete_trades(&mut self, keys: &[TradeKey]) -> Result<Deleted> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let positions: BTreeSet<(i64, i64)> = keys.iter().map(|k| (k.account_id, k.position_id)).collect();
        let mut deleted = Deleted::default();
        let mut shots: BTreeSet<String> = BTreeSet::new();
        for (account_id, position_id) in positions {
            let key = TradeKey { account_id, position_id, segment: 0 };
            let mut found = false;
            for (deal, _) in position_deals(&tx, &key)?.into_iter().filter(|(deal, _)| deal.kind.is_trade()) {
                found = true;
                let portable = views::deal_key(&tx, account_id, deal.ticket)?;
                sync::logged_delete(&tx, entity::DEAL, &portable, || {
                    tx.execute(
                        "DELETE FROM deals WHERE account_id = ?1 AND ticket = ?2",
                        rusqlite::params![account_id, deal.ticket],
                    )?;
                    Ok(())
                })?;
            }
            for (journal_key, entry) in journals_of_position(&tx, &key)? {
                found = true;
                shots.extend(entry.screenshots.iter().map(|shot| shot.path.clone()));
                let portable = views::journal_key(&tx, &journal_key)?;
                sync::logged_delete(&tx, entity::JOURNAL, &portable, || {
                    views::remove(&tx, entity::JOURNAL, &portable)
                })?;
            }
            deleted.trades += usize::from(found);
        }
        let still_used: BTreeSet<String> = views::attachment_paths(&tx)?.into_iter().collect();
        deleted.unused_images = shots.difference(&still_used).cloned().collect();
        tx.commit()?;
        if let Some(folder) = self.path.as_deref().and_then(Path::parent) {
            crate::attachments::remove_unused(&folder.join(ATTACHMENTS_DIR), &deleted.unused_images);
        }
        Ok(deleted)
    }
}

/// The journal entries of every segment of the position `key` names.
pub(super) fn journals_of_position(conn: &Connection, key: &TradeKey) -> Result<Vec<(TradeKey, JournalEntry)>> {
    let mut stmt =
        conn.prepare("SELECT segment FROM journal WHERE account_id = ?1 AND position_id = ?2 ORDER BY segment")?;
    let segments: Vec<u32> = stmt
        .query_map(rusqlite::params![key.account_id, key.position_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut found = Vec::with_capacity(segments.len());
    for segment in segments {
        let segment_key = TradeKey { segment, ..key.clone() };
        if let Some(entry) = journal_on(conn, &segment_key)? {
            found.push((segment_key, entry));
        }
    }
    Ok(found)
}

/// Removes the search entry of the journal of `key` (the trade is gone).
pub(super) fn unindex_journal(conn: &Connection, key: &TradeKey) -> Result<()> {
    let kind = enum_name(&DocKind::Trade)?;
    conn.execute("DELETE FROM search_index WHERE kind = ?1 AND reference = ?2", [kind, key.id()])?;
    Ok(())
}
