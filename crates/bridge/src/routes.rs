//! What each route does once a request is authenticated and its body is read: validate,
//! call [`tradelens_core`], bump the revision and report events. No HTTP in here.

use std::{
    collections::HashSet,
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tradelens_core::{
    Account, AccountInfo, Db, DbError, DealBatch, DealEntry, JournalEntry, TradeKey,
    attachments::{self, Attachment, Source},
    trades::build_trades,
};

use crate::{
    BridgeEvent, OpenedPosition, Revision,
    api::{self, AccountAccepted, DealsAccepted, Moment, Pong, ScreenshotStored},
    server::OnEvent,
};

/// Recorded as the origin of the deals; the EA and the Python script post the same way.
const SOURCE: &str = "ea";

/// The first eight bytes of every PNG file.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// A request the bridge refuses: the status to answer with and a short Vietnamese message.
#[derive(Debug)]
pub(crate) struct Failure {
    pub status: u16,
    pub message: String,
}

impl Failure {
    pub(crate) fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

impl From<DbError> for Failure {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Invalid(message) => Self::new(400, message),
            other => {
                log::error!("lỗi cơ sở dữ liệu: {other}");
                Self::new(500, "Lỗi cơ sở dữ liệu, xem nhật ký của TradeLens")
            }
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        log::error!("lỗi ghi file: {error}");
        Self::new(500, "Không ghi được ảnh vào thư mục dữ liệu")
    }
}

/// A successful answer: always JSON.
#[derive(Debug)]
pub(crate) struct Answer(pub String);

impl Answer {
    fn json<T: Serialize>(value: &T) -> Self {
        // Plain data structs always serialize; the fallback keeps the server panic-free anyway.
        Self(serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned()))
    }
}

/// The headers of `POST /api/v1/screenshot`, as sent.
#[derive(Debug, Default)]
pub(crate) struct ScreenshotHeaders {
    pub login: Option<String>,
    pub server: Option<String>,
    pub position: Option<String>,
    pub moment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountBody {
    account: AccountInfo,
}

/// The routes, with the database and folder they write to.
pub(crate) struct Routes {
    db: Db,
    /// The data folder's `attachments/`.
    store: PathBuf,
    revision: Revision,
    on_event: OnEvent,
}

impl std::fmt::Debug for Routes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Routes").field("store", &self.store).field("revision", &self.revision).finish_non_exhaustive()
    }
}

impl Routes {
    pub(crate) fn new(db: Db, store: PathBuf, revision: Revision, on_event: OnEvent) -> Self {
        Self { db, store, revision, on_event }
    }

    pub(crate) fn emit(&self, event: BridgeEvent) {
        (self.on_event)(event);
    }

    /// `GET /api/v1/ping`.
    pub(crate) fn ping() -> Answer {
        Answer::json(&Pong { app: "TradeLens".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() })
    }

    /// `POST /api/v1/deals`: stores the batch; the revision moves only when a deal was
    /// new or changed, so a resent batch costs the UI nothing.
    pub(crate) fn deals(&mut self, body: &[u8]) -> Result<Answer, Failure> {
        let batch: DealBatch = serde_json::from_slice(body).map_err(|e| bad_json(&e))?;
        let opened = self.opened_by(&batch)?;
        let report = self.db.upsert_batch(&batch, SOURCE)?;
        if report.inserted + report.updated > 0 {
            self.revision.bump();
        }
        self.emit(BridgeEvent::Received {
            source: SOURCE.to_owned(),
            account: batch.account.login.trim().to_owned(),
            inserted: report.inserted,
            updated: report.updated,
        });
        for position in opened {
            self.emit(BridgeEvent::PositionOpened(position));
        }
        Ok(Answer::json(&DealsAccepted {
            account_id: report.account_id,
            inserted: report.inserted,
            updated: report.updated,
        }))
    }

    /// The positions this batch opens: a buy or sell `in` deal the database has not seen,
    /// whose position is not already closed. Resent deals and the closed positions of a
    /// backfill therefore never ask for a plan.
    fn opened_by(&self, batch: &DealBatch) -> Result<Vec<OpenedPosition>, DbError> {
        if OpenedPosition::of_batch(batch).is_empty() {
            return Ok(Vec::new());
        }
        let stored: Vec<_> = match self.find_account(&batch.account.login, &batch.account.server)? {
            Some(account) => self.db.deals(Some(account.id))?.into_iter().map(|(_, deal)| deal).collect(),
            None => Vec::new(),
        };
        let known: HashSet<i64> = stored.iter().map(|deal| deal.ticket).collect();
        let closed: HashSet<i64> = stored
            .iter()
            .chain(&batch.deals)
            .filter(|deal| {
                deal.kind.is_trade() && matches!(deal.entry, DealEntry::Out | DealEntry::InOut | DealEntry::OutBy)
            })
            .map(|deal| deal.position_id)
            .collect();
        let fresh = DealBatch {
            account: batch.account.clone(),
            deals: batch.deals.iter().filter(|deal| !known.contains(&deal.ticket)).cloned().collect(),
            positions: Vec::new(),
        };
        Ok(OpenedPosition::of_batch(&fresh).into_iter().filter(|p| !closed.contains(&p.position_id)).collect())
    }

    /// `POST /api/v1/account`: balance and equity without deals. The revision moves only
    /// when something the UI shows changed, not on every periodic "still alive" post.
    pub(crate) fn account(&mut self, body: &[u8]) -> Result<Answer, Failure> {
        let AccountBody { account: info } = serde_json::from_slice(body).map_err(|e| bad_json(&e))?;
        let before = self.find_account(&info.login, &info.server)?;
        let account_id = self.db.upsert_account(&info)?;
        let after = self.find_account(&info.login, &info.server)?;
        let changed = match (before, after) {
            (Some(before), Some(after)) => Account { last_sync: after.last_sync, ..before } != after,
            _ => true,
        };
        if changed {
            self.revision.bump();
        }
        self.emit(BridgeEvent::Received {
            source: SOURCE.to_owned(),
            account: info.login.trim().to_owned(),
            inserted: 0,
            updated: 0,
        });
        Ok(Answer::json(&AccountAccepted { account_id }))
    }

    /// `POST /api/v1/screenshot`: keeps the PNG under `attachments/` and links it to the
    /// trade's journal entry, creating the entry when the trader has not written one.
    pub(crate) fn screenshot(&mut self, headers: &ScreenshotHeaders, png: &[u8]) -> Result<Answer, Failure> {
        let login = required(headers.login.as_deref(), api::LOGIN_HEADER)?;
        let position_id: i64 = required(headers.position.as_deref(), api::POSITION_HEADER)?
            .parse()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| Failure::new(400, format!("Header {} phải là số position", api::POSITION_HEADER)))?;
        let moment = Moment::parse(required(headers.moment.as_deref(), api::MOMENT_HEADER)?)
            .ok_or_else(|| Failure::new(400, format!("Header {} phải là entry hoặc exit", api::MOMENT_HEADER)))?;
        let relative = api::screenshot_path(login, position_id, moment);
        if relative.starts_with('/') {
            return Err(Failure::new(400, format!("Header {} không hợp lệ", api::LOGIN_HEADER)));
        }
        if !png.starts_with(&PNG_SIGNATURE) {
            return Err(Failure::new(400, "Nội dung gửi lên không phải ảnh PNG"));
        }

        let server = headers.server.as_deref().map_or_default(str::trim);
        let found = if server.is_empty() { self.find_by_login(login)? } else { self.find_account(login, server)? };
        let account_id = match found {
            Some(account) => account.id,
            None if !server.is_empty() => {
                // The shot can beat the account's first sync; the sync fills in the rest later.
                self.db.upsert_account(&AccountInfo {
                    login: login.trim().to_owned(),
                    server: server.to_owned(),
                    ..AccountInfo::default()
                })?
            }
            None => return Err(Failure::new(404, "Chưa có tài khoản này, hãy đồng bộ deal trước")),
        };

        let target = self.store.join(&relative);
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir)?;
        }
        // Written aside and renamed, so the app never reads half an image.
        let partial = target.with_extension("png.part");
        fs::write(&partial, png)?;
        fs::rename(&partial, &target)?;

        let key = self.trade_of(account_id, position_id, moment)?;
        let mut entry: JournalEntry = self.db.journal(&key)?.unwrap_or_default();
        let source = match moment {
            Moment::Entry => Source::EaEntry,
            Moment::Exit => Source::EaExit,
        };
        // A resent shot has the same path: the file is replaced and the link stays as it is.
        attachments::append(
            &mut entry.screenshots,
            [Attachment {
                path: relative.clone(),
                caption: String::new(),
                timeframe: None,
                source,
                added_at: now_ms(),
            }],
        );
        self.db.save_journal(&key, &entry)?;
        self.revision.bump();
        Ok(Answer::json(&ScreenshotStored { path: relative }))
    }

    /// The trade a screenshot belongs to: the position's first segment for the entry, its
    /// last for the exit. A position the database does not know yet gets segment 0.
    fn trade_of(&self, account_id: i64, position_id: i64, moment: Moment) -> Result<TradeKey, DbError> {
        let deals: Vec<_> = self.db.deals(Some(account_id))?.into_iter().map(|(_, deal)| deal).collect();
        let segments = build_trades(account_id, &deals)
            .into_iter()
            .filter(|trade| trade.key.position_id == position_id)
            .map(|trade| trade.key.segment);
        let segment = match moment {
            Moment::Entry => segments.min(),
            Moment::Exit => segments.max(),
        };
        Ok(TradeKey { account_id, position_id, segment: segment.unwrap_or(0) })
    }

    /// The account of a login on a server, as the database keys it.
    fn find_account(&self, login: &str, server: &str) -> Result<Option<Account>, DbError> {
        let (login, server) = (login.trim(), server.trim());
        Ok(self.db.accounts()?.into_iter().find(|a| a.login == login && a.server == server))
    }

    /// The first account of a login, for clients that do not say which server.
    fn find_by_login(&self, login: &str) -> Result<Option<Account>, DbError> {
        let login = login.trim();
        Ok(self.db.accounts()?.into_iter().find(|a| a.login == login))
    }
}

fn required<'a>(value: Option<&'a str>, header: &str) -> Result<&'a str, Failure> {
    value.map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| Failure::new(400, format!("Thiếu header {header}")))
}

fn bad_json(error: &serde_json::Error) -> Failure {
    Failure::new(400, format!("JSON không hợp lệ (dòng {}, cột {})", error.line(), error.column()))
}

/// Unix milliseconds; 0 when the clock is before 1970.
fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_millis()).ok()).unwrap_or(0)
}
