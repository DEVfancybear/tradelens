//! One account kept connected on a thread of its own.

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
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
    /// Before logging in again after the terminal could not be reached; doubled after each failure
    /// in a row, up to [`LinkSettings::retry_longest`].
    pub retry_after: Duration,
    /// The longest wait between two logins that fail: a failure that lasts (a server the terminal
    /// does not know) does not start a terminal every few minutes for ever. "Kết nối lại" starts a
    /// new link, which tries at once.
    pub retry_longest: Duration,
    /// How far back the first poll reads.
    pub first_days: u32,
    /// The longest an order may wait for the link's thread to take it (the thread may be logging in
    /// or polling). An order not taken by then is dropped, never sent later.
    pub order_queue: Duration,
    /// The longest an order taken by the thread waits for the terminal's answer. Longer than the
    /// worker's own limit on a call, so that the worker's answer (even a timeout) arrives first.
    pub order_timeout: Duration,
}

impl LinkSettings {
    /// The wait before the next login after `failures` failures in a row (at least one).
    #[must_use]
    pub fn retry_delay(&self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(16);
        self.retry_after.saturating_mul(1 << doublings).min(self.retry_longest.max(self.retry_after))
    }
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            poll_every: Duration::from_secs(2),
            retry_after: Duration::from_secs(30),
            retry_longest: Duration::from_mins(10),
            first_days: 365,
            order_queue: Duration::from_secs(10),
            order_timeout: Duration::from_secs(90),
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

/// An order waiting in the queue: not taken by the link's thread yet.
const QUEUED: u8 = 0;
/// The link's thread took the order and is sending it to the terminal.
const TAKEN: u8 = 1;
/// The app stopped waiting before the thread took it: the thread must drop it.
const DROPPED: u8 = 2;

/// Who has an order: the queue, the link's thread or nobody. The app and the thread race to move
/// it out of [`QUEUED`]; only one of them wins, so an order the app gave up on is never sent.
#[derive(Debug, Default)]
struct Claim(AtomicU8);

impl Claim {
    /// The thread takes the order; `false` when the app already dropped it.
    fn take(&self) -> bool {
        self.0.compare_exchange(QUEUED, TAKEN, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// The app drops the order; `false` when the thread already took it.
    fn drop_unsent(&self) -> bool {
        self.0.compare_exchange(QUEUED, DROPPED, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
}

/// What the app asks the terminal to do with a position.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Close { ticket: i64, volume: Option<f64> },
    Modify { ticket: i64, levels: Levels },
}

/// What the app asks of a link's thread.
enum Order {
    Trade { action: Action, claim: Arc<Claim>, reply: Sender<Result<(), Mt5Error>> },
    Stop,
}

/// One account kept connected: a thread that logs in, polls, carries orders and logs in again
/// after a failure, until [`Link::stop`] (or the link is dropped).
pub struct Link {
    account: AccountRef,
    orders: Sender<Order>,
    settings: LinkSettings,
    thread: Mutex<Option<JoinHandle<()>>>,
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
        let thread = thread::Builder::new()
            .name(format!("mt5 {account}"))
            .spawn(move || run(&login, &open, settings, &listen, &inbox))
            .expect("the system starts a thread for the MT5 link");
        Self { account, orders, settings, thread: Mutex::new(Some(thread)) }
    }

    pub fn account(&self) -> &AccountRef {
        &self.account
    }

    /// Closes `volume` lots of the position `ticket` (all of it when `None`). Blocks until the
    /// terminal answers: call it off the UI thread.
    ///
    /// # Errors
    ///
    /// The terminal's refusal; [`Mt5Error::Unreachable`] when the order was not sent (the account
    /// is not connected, or the link stayed busy past [`LinkSettings::order_queue`]); and
    /// [`Mt5Error::Uncertain`] when it was sent but no answer came.
    pub fn close(&self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.ask(Action::Close { ticket, volume })
    }

    /// Puts `levels` on the position `ticket`. Blocks like [`Link::close`].
    ///
    /// # Errors
    ///
    /// As [`Link::close`].
    pub fn modify(&self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        self.ask(Action::Modify { ticket, levels })
    }

    fn ask(&self, action: Action) -> Result<(), Mt5Error> {
        let (reply, answer) = mpsc::channel();
        let claim = Arc::new(Claim::default());
        self.orders.send(Order::Trade { action, claim: Arc::clone(&claim), reply }).map_err(|_| unreachable_link())?;
        match answer.recv_timeout(self.settings.order_queue) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Disconnected) => return Err(unreachable_link()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if claim.drop_unsent() {
            return Err(Mt5Error::Unreachable(
                "MT5 đang bận (đăng nhập lại hoặc đọc dữ liệu): lệnh chưa được gửi, hãy thử lại".to_owned(),
            ));
        }
        // The thread took it: the terminal is on it, and its answer comes within the worker's limit.
        let rest = self.settings.order_timeout.saturating_sub(self.settings.order_queue);
        answer.recv_timeout(rest).unwrap_or_else(|_| Err(uncertain()))
    }

    /// Tells the thread to log out and stop the terminal, without waiting for it: the link
    /// reports [`LinkState::Stopped`] once it has.
    pub fn stop(&self) {
        // The thread may have ended already; there is nothing left to stop then.
        drop(self.orders.send(Order::Stop));
    }

    /// Stops the link and waits for its thread to end: its terminal has then logged out and
    /// stopped, so another link may use the same portable terminal. Blocks for as long as a login
    /// in progress takes: call it off the UI thread.
    ///
    /// # Panics
    ///
    /// When another thread panicked while holding the link's lock.
    pub fn stop_and_wait(&self) {
        self.stop();
        let thread = self.thread.lock().expect("lock poisoned").take();
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            log::error!("mt5 {}: the link's thread panicked", self.account);
        }
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
    Mt5Error::Unreachable("tài khoản đang kết nối lại, lệnh chưa được gửi; hãy thử lại sau ít giây".to_owned())
}

fn uncertain() -> Mt5Error {
    Mt5Error::Uncertain("lệnh đã gửi tới MT5 nhưng chưa có trả lời; hãy xem lại lệnh trước khi gửi lần nữa".to_owned())
}

/// Whether the thread goes on after a wait.
enum Next {
    Go,
    Stop,
}

/// The link's thread: log in, poll until a failure, wait, log in again; until told to stop.
fn run(login: &Login, open: &Opener, settings: LinkSettings, listen: &Listener, inbox: &Receiver<Order>) {
    let account = &login.account;
    // Logins that failed in a row; each waits longer before the next ([`LinkSettings::retry_delay`]).
    let mut failures: u32 = 0;
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
                    if matches!(refuse(order), Next::Stop) {
                        break;
                    }
                }
                break;
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                let pause = settings.retry_delay(failures);
                log::warn!("mt5 {account}: {error} (next try in {}s)", pause.as_secs());
                listen(Event::State(account.clone(), LinkState::Retrying(error)));
                match wait(inbox, pause, None) {
                    Next::Go => continue,
                    Next::Stop => break,
                }
            }
        };
        failures = 0;
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
                    if matches!(refuse(order), Next::Stop) {
                        return Next::Stop;
                    }
                }
            },
        }
    }
}

/// Does an order on the terminal and answers it, unless the app already gave up on it.
fn carry(terminal: &mut dyn Terminal, order: Order) {
    let Order::Trade { action, claim, reply } = order else { return };
    if !claim.take() {
        log::info!("mt5: an order the app stopped waiting for is dropped unsent");
        return;
    }
    let done = match action {
        Action::Close { ticket, volume } => terminal.close(ticket, volume),
        Action::Modify { ticket, levels } => terminal.modify(ticket, levels),
    };
    drop(reply.send(done));
}

/// Answers an order that cannot be done now: it was not sent. `Stop` when the order was to stop.
fn refuse(order: Order) -> Next {
    match order {
        Order::Trade { claim, reply, .. } => {
            if claim.take() {
                drop(reply.send(Err(not_connected())));
            }
            Next::Go
        }
        Order::Stop => Next::Stop,
    }
}

/// The time now in milliseconds since 1970, `None` if the clock is before it.
fn now_ms() -> Option<i64> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}
