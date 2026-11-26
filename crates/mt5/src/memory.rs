//! A broker in memory: accounts, their deals and open positions, and terminals over them. Tests
//! and the demo (`TRADELENS_MT5=memory`) use it in place of MetaTrader; a test changes the account
//! (opens a position, closes one) while the app is connected and checks the app follows.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use tradelens_core::{AccountInfo, Deal, DealEntry, DealKind};

use crate::{
    link::Opener,
    protocol::{Direction, Levels, Login, Mt5Error, Poll, Position, Terminal},
};

/// One account of the broker.
#[derive(Debug, Clone, Default)]
struct Account {
    password: String,
    info: AccountInfo,
    deals: Vec<Deal>,
    positions: Vec<Position>,
    /// The next ticket the broker gives a deal it makes (closing a position).
    next_ticket: i64,
}

/// A broker's server: its accounts by login. Shared by every terminal logged in to it.
#[derive(Debug, Default)]
pub struct Broker {
    accounts: Mutex<HashMap<String, Account>>,
    /// When set, every terminal fails to connect with this (the terminal cannot start).
    unreachable: Mutex<Option<String>>,
}

impl Broker {
    /// The broker of the server `name`, made empty the first time it is asked for. Every call with
    /// the same name in the process gets the same broker.
    ///
    /// # Panics
    ///
    /// When another thread panicked while holding the brokers' lock.
    pub fn named(name: &str) -> Arc<Self> {
        static BROKERS: OnceLock<Mutex<HashMap<String, Arc<Broker>>>> = OnceLock::new();
        let brokers = BROKERS.get_or_init(Mutex::default);
        Arc::clone(brokers.lock().expect("lock poisoned").entry(name.to_owned()).or_default())
    }

    fn accounts(&self) -> MutexGuard<'_, HashMap<String, Account>> {
        self.accounts.lock().expect("lock poisoned")
    }

    /// Opens (or resets the password of) the account `login`.
    pub fn add_account(&self, login: &str, password: &str, info: AccountInfo) {
        let mut accounts = self.accounts();
        let account = accounts
            .entry(login.to_owned())
            .or_insert_with(|| Account { next_ticket: 1_000_000, ..Account::default() });
        password.clone_into(&mut account.password);
        account.info = AccountInfo { login: login.to_owned(), ..info };
    }

    /// Makes every terminal fail to start with `why`, or start again with `None`.
    ///
    /// # Panics
    ///
    /// When another thread panicked while holding the lock.
    pub fn set_unreachable(&self, why: Option<&str>) {
        *self.unreachable.lock().expect("lock poisoned") = why.map(str::to_owned);
    }

    /// Adds a deal to the account's history (a deposit, a trade's entry or exit).
    pub fn push_deal(&self, login: &str, deal: Deal) {
        if let Some(account) = self.accounts().get_mut(login) {
            account.deals.push(deal);
        }
    }

    /// For the demo: when the broker has no account `login`, opens one with `password`, a deposit,
    /// a closed XAUUSD trade of yesterday and an open EURUSD position, all around `now_ms`. An
    /// account already there is left as it is (its password still decides the login).
    pub fn open_demo_account(&self, login: &str, password: &str, now_ms: i64) {
        if self.accounts().contains_key(login) {
            return;
        }
        let info = AccountInfo {
            name: "Tài khoản thử".to_owned(),
            broker: "TradeLens Demo".to_owned(),
            currency: "USD".to_owned(),
            balance: Some(10_120.0),
            equity: Some(10_135.5),
            server_offset_min: Some(0),
            ..AccountInfo::default()
        };
        self.add_account(login, password, info);
        let day = 24 * 3600 * 1000;
        let closed = Position {
            ticket: 900_001,
            time_msc: now_ms - day,
            symbol: "XAUUSD".to_owned(),
            side: Direction::Buy,
            volume: 0.1,
            open_price: 2650.0,
            current_price: 2662.0,
            sl: 2640.0,
            tp: 2665.0,
            profit: 120.0,
            swap: 0.0,
            magic: 0,
            comment: String::new(),
        };
        let mut deposit = deal_of(&closed, 1, DealEntry::None, 0.0, 10_000.0, now_ms - 2 * day);
        deposit.kind = DealKind::Balance;
        deposit.symbol.clear();
        deposit.position_id = 0;
        self.push_deal(login, deposit);
        self.push_deal(login, deal_of(&closed, 2, DealEntry::In, 0.1, 0.0, closed.time_msc));
        self.push_deal(login, deal_of(&closed, 3, DealEntry::Out, 0.1, 120.0, closed.time_msc + 3_600_000));
        self.open(
            login,
            Position {
                ticket: 900_002,
                time_msc: now_ms - 600_000,
                symbol: "EURUSD".to_owned(),
                side: Direction::Sell,
                volume: 0.5,
                open_price: 1.085,
                current_price: 1.0847,
                sl: 1.088,
                tp: 1.079,
                profit: 15.5,
                swap: 0.0,
                magic: 0,
                comment: String::new(),
            },
        );
    }

    /// Opens `position` on the account, with its entry deal.
    pub fn open(&self, login: &str, position: Position) {
        let mut accounts = self.accounts();
        let Some(account) = accounts.get_mut(login) else { return };
        let ticket = account.next_ticket;
        account.next_ticket += 1;
        account.deals.push(deal_of(&position, ticket, DealEntry::In, position.volume, 0.0, position.time_msc));
        account.positions.push(position);
    }

    /// The positions open on the account.
    pub fn positions(&self, login: &str) -> Vec<Position> {
        self.accounts().get(login).map_or_default(|account| account.positions.clone())
    }

    fn connect(&self, login: &Login) -> Result<AccountInfo, Mt5Error> {
        if let Some(why) = self.unreachable.lock().expect("lock poisoned").clone() {
            return Err(Mt5Error::Unreachable(why));
        }
        let accounts = self.accounts();
        match accounts.get(&login.account.login) {
            Some(account) if account.password == login.password.expose() => Ok(account.info.clone()),
            _ => Err(Mt5Error::LoginRefused("sai số tài khoản, mật khẩu hoặc server".to_owned())),
        }
    }

    fn poll(&self, login: &str, since_msc: Option<i64>) -> Result<Poll, Mt5Error> {
        if let Some(why) = self.unreachable.lock().expect("lock poisoned").clone() {
            return Err(Mt5Error::Unreachable(why));
        }
        let accounts = self.accounts();
        let account = accounts.get(login).ok_or_else(|| Mt5Error::Unreachable("tài khoản không còn".to_owned()))?;
        let from = since_msc.map_or(i64::MIN, |since| since.saturating_sub(SLACK_MS));
        Ok(Poll {
            account: account.info.clone(),
            deals: account.deals.iter().filter(|deal| deal.time_msc >= from).cloned().collect(),
            positions: account.positions.clone(),
        })
    }

    fn close(&self, login: &str, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        let mut accounts = self.accounts();
        let account = accounts.get_mut(login).ok_or_else(|| refused("tài khoản không còn"))?;
        let index = account
            .positions
            .iter()
            .position(|p| p.ticket == ticket)
            .ok_or_else(|| refused(&format!("không còn lệnh {ticket} đang mở")))?;
        let open = account.positions[index].volume;
        let lots = volume.unwrap_or(open);
        if lots <= 0.0 || lots > open + 1e-9 {
            return Err(refused(&format!("khối lượng đóng {lots} không hợp lệ (lệnh đang có {open})")));
        }
        let position = account.positions[index].clone();
        let share = lots / open;
        let deal_ticket = account.next_ticket;
        account.next_ticket += 1;
        let time = account.deals.iter().map(|d| d.time_msc).max().unwrap_or(position.time_msc) + 1_000;
        account.deals.push(deal_of(&position, deal_ticket, DealEntry::Out, lots, position.profit * share, time));
        if (open - lots).abs() < 1e-9 {
            account.positions.remove(index);
        } else {
            let left = &mut account.positions[index];
            left.volume = open - lots;
            left.profit -= position.profit * share;
        }
        Ok(())
    }

    fn modify(&self, login: &str, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        let mut accounts = self.accounts();
        let position = accounts
            .get_mut(login)
            .and_then(|account| account.positions.iter_mut().find(|p| p.ticket == ticket))
            .ok_or_else(|| refused(&format!("không còn lệnh {ticket} đang mở")))?;
        position.sl = levels.sl.unwrap_or(0.0);
        position.tp = levels.tp.unwrap_or(0.0);
        Ok(())
    }
}

/// A poll after the first reads a day before the newest deal, as the worker does.
const SLACK_MS: i64 = 24 * 3600 * 1000;

fn refused(message: &str) -> Mt5Error {
    Mt5Error::Refused { code: 10_013, message: message.to_owned() }
}

/// The deal of `position` that opens (`In`) or closes (`Out`) `volume` lots of it.
fn deal_of(position: &Position, ticket: i64, entry: DealEntry, volume: f64, profit: f64, time_msc: i64) -> Deal {
    let buy = (position.side == Direction::Buy) == (entry == DealEntry::In);
    Deal {
        ticket,
        order: ticket,
        position_id: position.ticket,
        time_msc,
        symbol: position.symbol.clone(),
        kind: if buy { DealKind::Buy } else { DealKind::Sell },
        entry,
        volume,
        price: if entry == DealEntry::In { position.open_price } else { position.current_price },
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: position.magic,
        comment: position.comment.clone(),
        sl: position.sl,
        tp: position.tp,
        risk: None,
        reason: Some("client".to_owned()),
        result_r: None,
        gaps: tradelens_core::model::Gaps::default(),
    }
}

/// A terminal logged in to a [`Broker`].
#[derive(Debug)]
struct MemoryTerminal {
    broker: Arc<Broker>,
    login: Option<String>,
}

impl MemoryTerminal {
    fn login(&self) -> Result<&str, Mt5Error> {
        self.login.as_deref().ok_or_else(|| Mt5Error::Unreachable("chưa đăng nhập MT5".to_owned()))
    }
}

impl Terminal for MemoryTerminal {
    fn connect(&mut self, login: &Login) -> Result<AccountInfo, Mt5Error> {
        let info = self.broker.connect(login)?;
        self.login = Some(login.account.login.clone());
        Ok(info)
    }

    fn poll(&mut self, since_msc: Option<i64>, _days: u32) -> Result<Poll, Mt5Error> {
        self.broker.poll(self.login()?, since_msc)
    }

    fn close(&mut self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.broker.close(self.login()?, ticket, volume)
    }

    fn modify(&mut self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        self.broker.modify(self.login()?, ticket, levels)
    }
}

/// Opens terminals on the broker named by each login's server ([`Broker::named`]).
pub fn opener() -> Opener {
    Arc::new(|login: &Login| -> Result<Box<dyn Terminal>, Mt5Error> {
        Ok(Box::new(MemoryTerminal { broker: Broker::named(&login.account.server), login: None }))
    })
}
