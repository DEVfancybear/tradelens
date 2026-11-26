//! One account kept connected on a thread of its own.

use std::{
    fmt,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tradelens_core::AccountInfo;

use crate::protocol::{AccountRef, Levels, Login, Mt5Error, Poll, Terminal};

/// Starts a terminal for a login: the real worker, or a broker in memory.
pub type Opener = Arc<dyn Fn(&Login) -> Result<Box<dyn Terminal>, Mt5Error> + Send + Sync>;

/// How a [`Link`] paces itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkSettings {
    /// Between two polls: what a closed trade waits at most before the app has it.
    pub poll_every: Duration,
    /// Before logging in again after the terminal could not be reached.
    pub retry_after: Duration,
    /// How far back the first poll reads.
    pub first_days: u32,
    /// The longest an order waits for the link's thread.
    pub order_timeout: Duration,
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            poll_every: Duration::from_secs(2),
            retry_after: Duration::from_secs(30),
            first_days: 365,
            order_timeout: Duration::from_secs(60),
        }
    }
}

/// Where a link stands.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkState {
    /// Starting the terminal and logging in.
    Connecting,
    /// Logged in: polls follow.
    Connected(AccountInfo),
    /// The terminal could not be reached or read; the link tries again after a while.
    Retrying(Mt5Error),
    /// The server refused the login: the link waits to be stopped (a new password makes a new link).
    Refused(Mt5Error),
    /// Stopped by the app.
    Stopped,
}

/// What a link tells the app, from its own thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    State(AccountRef, LinkState),
    /// A poll's deals and positions, for the app to store. `first` is the first poll since the
    /// login: the positions it lists were open before, not just opened.
    Polled {
        account: AccountRef,
        poll: Poll,
        first: bool,
    },
}

/// What the app hears events with; it is called on the link's thread.
pub type Listener = Arc<dyn Fn(Event) + Send + Sync>;

/// What the app asks of a link's thread.
enum Order {
    Close { ticket: i64, volume: Option<f64>, reply: Sender<Result<(), Mt5Error>> },
    Modify { ticket: i64, levels: Levels, reply: Sender<Result<(), Mt5Error>> },
    Stop,
}

/// One account kept connected: a thread that logs in, polls, carries orders and logs in again
/// after a failure, until [`Link::stop`] (or the link is dropped).
pub struct Link {
    account: AccountRef,
    orders: Sender<Order>,
    order_timeout: Duration,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").field("account", &self.account).finish_non_exhaustive()
    }
}

impl Link {
    /// Starts keeping `login`'s account connected with terminals `open` makes; `listen` hears
    /// what happens.
    ///
    /// # Panics
    ///
    /// When the operating system cannot start a thread.
    pub fn start(login: Login, open: Opener, settings: LinkSettings, listen: Listener) -> Self {
        let account = login.account.clone();
        let (orders, inbox) = mpsc::channel();
        thread::Builder::new()
            .name(format!("mt5 {account}"))
            .spawn(move || run(&login, &open, settings, &listen, &inbox))
            .expect("the system starts a thread for the MT5 link");
        Self { account, orders, order_timeout: settings.order_timeout }
    }

    pub fn account(&self) -> &AccountRef {
        &self.account
    }

    /// Closes `volume` lots of the position `ticket` (all of it when `None`). Blocks until the
    /// terminal answers: call it off the UI thread.
    ///
    /// # Errors
    ///
    /// The terminal's refusal, or [`Mt5Error::Unreachable`] when the account is not connected.
    pub fn close(&self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.ask(|reply| Order::Close { ticket, volume, reply })
    }

    /// Puts `levels` on the position `ticket`. Blocks like [`Link::close`].
    ///
    /// # Errors
    ///
    /// As [`Link::close`].
    pub fn modify(&self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        self.ask(|reply| Order::Modify { ticket, levels, reply })
    }

    fn ask(&self, order: impl FnOnce(Sender<Result<(), Mt5Error>>) -> Order) -> Result<(), Mt5Error> {
        let (reply, answer) = mpsc::channel();
        self.orders.send(order(reply)).map_err(|_| unreachable_link())?;
        answer.recv_timeout(self.order_timeout).map_err(|error| match error {
            RecvTimeoutError::Timeout => Mt5Error::Unreachable("MT5 không trả lời kịp".to_owned()),
            RecvTimeoutError::Disconnected => unreachable_link(),
        })?
    }

    /// Tells the thread to log out and stop the terminal, without waiting for it: the link
    /// reports [`LinkState::Stopped`] once it has.
    pub fn stop(&self) {
        // The thread may have ended already; there is nothing left to stop then.
        drop(self.orders.send(Order::Stop));
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

fn unreachable_link() -> Mt5Error {
    Mt5Error::Unreachable("tài khoản chưa kết nối".to_owned())
}

fn not_connected() -> Mt5Error {
    Mt5Error::Unreachable("tài khoản đang kết nối lại, hãy thử lại sau ít giây".to_owned())
}

/// Whether the thread goes on after a wait.
enum Next {
    Go,
    Stop,
}

/// The link's thread: log in, poll until a failure, wait, log in again; until told to stop.
fn run(login: &Login, open: &Opener, settings: LinkSettings, listen: &Listener, inbox: &Receiver<Order>) {
    let account = &login.account;
    loop {
        listen(Event::State(account.clone(), LinkState::Connecting));
        let terminal = open(login).and_then(|mut terminal| terminal.connect(login).map(|info| (terminal, info)));
        let (mut terminal, info) = match terminal {
            Ok(connected) => connected,
            Err(error @ Mt5Error::LoginRefused(_)) => {
                log::warn!("mt5 {account}: {error}");
                listen(Event::State(account.clone(), LinkState::Refused(error)));
                // Nothing to do but answer orders until stopped: the password is wrong.
                while let Ok(order) = inbox.recv() {
                    if refuse(order, not_connected) {
                        break;
                    }
                }
                break;
            }
            Err(error) => {
                log::warn!("mt5 {account}: {error}");
                listen(Event::State(account.clone(), LinkState::Retrying(error)));
                match wait(inbox, settings.retry_after, None) {
                    Next::Go => continue,
                    Next::Stop => break,
                }
            }
        };
        log::info!("mt5 {account}: connected");
        listen(Event::State(account.clone(), LinkState::Connected(info)));
        match poll_until_failure(terminal.as_mut(), account, settings, listen, inbox) {
            Next::Stop => break,
            Next::Go => {
                drop(terminal);
                if matches!(wait(inbox, settings.retry_after, None), Next::Stop) {
                    break;
                }
            }
        }
    }
    listen(Event::State(account.clone(), LinkState::Stopped));
}

/// Polls `terminal` and carries orders until a poll fails (`Go`: log in again) or the app stops
/// the link (`Stop`).
fn poll_until_failure(
    terminal: &mut dyn Terminal,
    account: &AccountRef,
    settings: LinkSettings,
    listen: &Listener,
    inbox: &Receiver<Order>,
) -> Next {
    let mut since: Option<i64> = None;
    let mut first = true;
    loop {
        match terminal.poll(since, settings.first_days) {
            Ok(poll) => {
                // The next poll reads from the newest deal (with the worker's day of slack); an
                // account without deals reads from now.
                let newest = poll.newest_deal().or_else(|| since.or_else(now_ms));
                since = newest.max(since);
                listen(Event::Polled { account: account.clone(), poll, first });
                first = false;
            }
            Err(error) => {
                log::warn!("mt5 {account}: poll failed: {error}");
                listen(Event::State(account.clone(), LinkState::Retrying(error)));
                return Next::Go;
            }
        }
        if matches!(wait(inbox, settings.poll_every, Some(&mut *terminal)), Next::Stop) {
            return Next::Stop;
        }
    }
}

/// Waits `pause` while carrying the orders that arrive (on `terminal` when there is one); an
/// order cuts the wait short so the next poll shows what it did.
fn wait(inbox: &Receiver<Order>, pause: Duration, mut terminal: Option<&mut dyn Terminal>) -> Next {
    let deadline = Instant::now() + pause;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match inbox.recv_timeout(left) {
            Ok(Order::Stop) | Err(RecvTimeoutError::Disconnected) => return Next::Stop,
            Err(RecvTimeoutError::Timeout) => return Next::Go,
            Ok(order) => match terminal.as_deref_mut() {
                Some(terminal) => {
                    carry(terminal, order);
                    return Next::Go;
                }
                None => {
                    if refuse(order, not_connected) {
                        return Next::Stop;
                    }
                }
            },
        }
    }
}

/// Does an order on the terminal and answers it.
fn carry(terminal: &mut dyn Terminal, order: Order) {
    match order {
        Order::Close { ticket, volume, reply } => drop(reply.send(terminal.close(ticket, volume))),
        Order::Modify { ticket, levels, reply } => drop(reply.send(terminal.modify(ticket, levels))),
        Order::Stop => {}
    }
}

/// Answers an order that cannot be done now with `error`; `true` when the order was to stop.
fn refuse(order: Order, error: fn() -> Mt5Error) -> bool {
    match order {
        Order::Close { reply, .. } | Order::Modify { reply, .. } => {
            drop(reply.send(Err(error())));
            false
        }
        Order::Stop => true,
    }
}

/// The time now in milliseconds since 1970, `None` if the clock is before it.
fn now_ms() -> Option<i64> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}
