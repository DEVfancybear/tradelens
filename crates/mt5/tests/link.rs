//! A link over the broker in memory: logging in, polling, orders, a refused password, and a
//! terminal that cannot be reached and comes back.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use tradelens_core::AccountInfo;
use tradelens_mt5::{
    AccountRef, Direction, Event, Levels, Link, LinkSettings, LinkState, Login, Mt5Error, Opener, Password, Poll,
    Position, Terminal,
    memory::{self, Broker},
};

const WAIT: Duration = Duration::from_secs(10);

fn settings() -> LinkSettings {
    LinkSettings {
        poll_every: Duration::from_millis(50),
        retry_after: Duration::from_millis(100),
        retry_longest: Duration::from_millis(400),
        first_days: 30,
        order_queue: Duration::from_secs(2),
        order_timeout: Duration::from_secs(5),
        settle_every: Duration::from_millis(20),
        settle_reads: 3,
    }
}

fn login(server: &str, password: &str) -> Login {
    Login {
        account: AccountRef::new("1001", server),
        password: Password::new(password),
        terminal: PathBuf::from("terminal64.exe"),
    }
}

fn broker(server: &str) -> Arc<Broker> {
    let broker = Broker::named(server);
    broker.add_account(
        "1001",
        "pass",
        AccountInfo {
            server: server.to_owned(),
            currency: "USD".to_owned(),
            balance: Some(1000.0),
            ..AccountInfo::default()
        },
    );
    broker
}

fn position(ticket: i64) -> Position {
    Position {
        ticket,
        time_msc: 1_763_650_000_000,
        symbol: "XAUUSD".to_owned(),
        side: Direction::Buy,
        volume: 0.2,
        open_price: 2650.0,
        current_price: 2655.0,
        sl: 2640.0,
        tp: 0.0,
        profit: 100.0,
        swap: 0.0,
        magic: 0,
        comment: String::new(),
    }
}

fn start(login: Login) -> (Link, Receiver<Event>) {
    let (events, heard) = mpsc::channel();
    let link = Link::start(
        login,
        memory::opener(),
        settings(),
        Arc::new(move |event| {
            drop(events.send(event));
        }),
    );
    (link, heard)
}

/// The next event `pick` takes, skipping the others.
fn next<T>(heard: &Receiver<Event>, mut pick: impl FnMut(Event) -> Option<T>) -> T {
    loop {
        let event = heard.recv_timeout(WAIT).expect("the link says what happens in time");
        if let Some(found) = pick(event) {
            return found;
        }
    }
}

#[test]
fn a_connected_link_reports_deals_and_positions_and_carries_orders() {
    let broker = broker("link-orders");
    broker.open("1001", position(77));
    let (link, heard) = start(login("link-orders", "pass"));
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Connected(_))).then_some(()));
    let (poll, first) = next(&heard, |e| match e {
        Event::Polled { poll, first, .. } => Some((poll, first)),
        Event::State(..) => None,
    });
    assert!(first, "the first poll after logging in says so");
    assert_eq!(poll.positions.len(), 1);
    assert_eq!(poll.deals.len(), 1, "the entry deal is read");
    let later = next(&heard, |e| match e {
        Event::Polled { first, .. } => Some(first),
        Event::State(..) => None,
    });
    assert!(!later, "later polls are not the first");

    link.modify(77, Levels { sl: Some(2645.0), tp: Some(2700.0) }).expect("the stop and target move");
    assert_eq!(broker.positions("1001")[0].tp, 2700.0);
    link.close(77, Some(0.05)).expect("part of the position closes");
    assert!((broker.positions("1001")[0].volume - 0.15).abs() < 1e-9, "0.15 lot is left");
    link.close(77, None).expect("the rest closes");
    assert!(broker.positions("1001").is_empty(), "nothing is left open");
    let refused = link.close(77, None).expect_err("a closed position cannot be closed again");
    assert!(matches!(refused, Mt5Error::Refused { .. }), "{refused:?}");
    let deals = next(&heard, |e| match e {
        Event::Polled { poll, .. } if poll.positions.is_empty() => Some(poll.deals),
        _ => None,
    });
    assert_eq!(deals.len(), 3, "the entry and the two exits");
    link.stop();
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Stopped)).then_some(()));
}

#[test]
fn a_wrong_password_is_reported_once_and_not_tried_again() {
    broker("link-refused");
    let (link, heard) = start(login("link-refused", "wrong"));
    let error = next(&heard, |e| match e {
        Event::State(_, LinkState::Refused(error)) => Some(error),
        _ => None,
    });
    assert!(matches!(error, Mt5Error::LoginRefused(_)), "{error:?}");
    assert!(link.close(1, None).is_err(), "orders are refused while not connected");
    assert!(
        heard.recv_timeout(Duration::from_millis(400)).is_err(),
        "a refused login waits instead of logging in again"
    );
    link.stop();
}

#[test]
fn a_terminal_that_cannot_be_reached_is_tried_again_until_it_answers() {
    let broker = broker("link-retry");
    broker.set_unreachable(Some("terminal không chạy"));
    let (link, heard) = start(login("link-retry", "pass"));
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Retrying(_))).then_some(()));
    broker.set_unreachable(None);
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Connected(_))).then_some(()));
    // A poll that fails after the login is retried too.
    broker.set_unreachable(Some("mất kết nối"));
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Retrying(_))).then_some(()));
    broker.set_unreachable(None);
    next(&heard, |e| matches!(e, Event::Polled { first: true, .. }).then_some(()));
    link.stop();
}

#[test]
fn logins_that_keep_failing_wait_longer_each_time_up_to_a_limit() {
    let real = LinkSettings::default();
    let waits: Vec<u64> = (1..=8).map(|failures| real.retry_delay(failures).as_secs()).collect();
    assert_eq!(waits, [30, 60, 120, 240, 480, 600, 600, 600], "doubled from 30 s, at most 10 min");
    assert_eq!(real.retry_delay(u32::MAX), Duration::from_mins(10), "no overflow after many failures");

    // On a live link: the third try waits four times as long as the first.
    let broker = broker("link-backoff");
    broker.set_unreachable(Some("terminal không nối được tới server"));
    let (link, heard) = start(login("link-backoff", "pass"));
    let mut tries = Vec::new();
    while tries.len() < 4 {
        next(&heard, |e| matches!(e, Event::State(_, LinkState::Connecting)).then_some(()));
        tries.push(Instant::now());
    }
    // Only the third wait is measured, against its own delay (400 ms) and never against the first:
    // a slow login on a loaded machine only lengthens a wait (macOS CI measured 188 ms for the
    // first 100 ms). The slack covers the test thread reading the third try's event late, which is
    // the one thing that shortens what it measures; without the backoff the wait would be 100 ms.
    let third = tries[3] - tries[2];
    let slack = Duration::from_millis(150);
    let expected = settings().retry_delay(3);
    assert!(third + slack >= expected, "the third try waits {expected:?}: measured {third:?}");
    broker.set_unreachable(None);
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Connected(_))).then_some(()));
    link.stop();
}

#[test]
fn the_demo_opens_a_sample_account_on_the_first_sign_in_and_keeps_its_password() {
    let server = "link-demo";
    Broker::named(server).open_demo_account("1001", "first", 1_763_650_000_000);
    Broker::named(server).open_demo_account("1001", "second", 1_763_650_000_000);
    let (link, heard) = start(login(server, "first"));
    let poll = next(&heard, |e| match e {
        Event::Polled { poll, .. } => Some(poll),
        Event::State(..) => None,
    });
    assert_eq!(poll.deals.len(), 4, "a deposit, a closed trade and an open position's entry");
    assert_eq!(poll.positions.len(), 1);
    link.stop();
    let (link, heard) = start(login(server, "second"));
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Refused(_))).then_some(()));
    link.stop();
}

/// A terminal over the broker in memory whose login takes `delay`.
struct SlowLogin {
    inner: Box<dyn Terminal>,
    delay: Duration,
}

impl Terminal for SlowLogin {
    fn connect(&mut self, login: &Login) -> Result<AccountInfo, Mt5Error> {
        std::thread::sleep(self.delay);
        self.inner.connect(login)
    }

    fn poll(&mut self, since_msc: Option<i64>, days: u32) -> Result<Poll, Mt5Error> {
        self.inner.poll(since_msc, days)
    }

    fn close(&mut self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.inner.close(ticket, volume)
    }

    fn modify(&mut self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        self.inner.modify(ticket, levels)
    }
}

/// Takes one from `count` when it is above zero; whether it was.
fn take_one(count: &AtomicUsize) -> bool {
    count.try_update(AtomicOrdering::SeqCst, AtomicOrdering::SeqCst, |n| n.checked_sub(1)).is_ok()
}

/// A terminal over the broker in memory that reports login `0` (as MT5 does while it signs in) on
/// the first `connects` logins and the first `polls` polls of all the terminals sharing the counts.
struct NotSignedInYet {
    inner: Box<dyn Terminal>,
    connects: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
}

impl Terminal for NotSignedInYet {
    fn connect(&mut self, login: &Login) -> Result<AccountInfo, Mt5Error> {
        let info = self.inner.connect(login)?;
        Ok(if take_one(&self.connects) { AccountInfo { login: "0".to_owned(), ..info } } else { info })
    }

    fn poll(&mut self, since_msc: Option<i64>, days: u32) -> Result<Poll, Mt5Error> {
        let mut poll = self.inner.poll(since_msc, days)?;
        if take_one(&self.polls) {
            "0".clone_into(&mut poll.account.login);
        }
        Ok(poll)
    }

    fn close(&mut self, ticket: i64, volume: Option<f64>) -> Result<(), Mt5Error> {
        self.inner.close(ticket, volume)
    }

    fn modify(&mut self, ticket: i64, levels: Levels) -> Result<(), Mt5Error> {
        self.inner.modify(ticket, levels)
    }
}

/// Starts a link over [`NotSignedInYet`] terminals, with how many logins and reads report login
/// `0`; the events, and how many terminals were started.
fn start_signing_in(server: &str, connects: usize, polls: usize) -> (Link, Receiver<Event>, Arc<AtomicUsize>) {
    let memory = memory::opener();
    let (connects, polls) = (Arc::new(AtomicUsize::new(connects)), Arc::new(AtomicUsize::new(polls)));
    let opened = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&opened);
    let open: Opener = Arc::new(move |login: &Login| -> Result<Box<dyn Terminal>, Mt5Error> {
        counted.fetch_add(1, AtomicOrdering::SeqCst);
        let (connects, polls) = (Arc::clone(&connects), Arc::clone(&polls));
        Ok(Box::new(NotSignedInYet { inner: memory(login)?, connects, polls }))
    });
    let (events, heard) = mpsc::channel();
    let link = Link::start(
        login(server, "pass"),
        open,
        settings(),
        Arc::new(move |event| {
            drop(events.send(event));
        }),
    );
    (link, heard, opened)
}

/// The first event `pick` takes; fails on any event that passes on what a terminal read while it
/// reported another account.
fn next_of_account<T>(heard: &Receiver<Event>, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
    next(heard, |event| {
        match &event {
            Event::State(_, LinkState::Connected(info)) => {
                assert_eq!(info.login, "1001", "connected as another account");
            }
            Event::Polled { poll, .. } => {
                assert_eq!(poll.account.login, "1001", "a read of another account passed on");
            }
            Event::State(..) => {}
        }
        pick(&event)
    })
}

#[test]
fn a_terminal_still_signing_in_is_read_again_without_being_started_again() {
    let broker = broker("link-signing-in");
    broker.open("1001", position(77));
    // The login and the read after it report login 0; then (a second link) two polls do.
    for (connects, polls) in [(1, 1), (0, 2)] {
        let (link, heard, opened) = start_signing_in("link-signing-in", connects, polls);
        let poll = next_of_account(&heard, |event| match event {
            Event::State(_, LinkState::Retrying(error)) => panic!("a terminal signing in is given time: {error:?}"),
            Event::Polled { poll, .. } => Some(poll.clone()),
            Event::State(..) => None,
        });
        assert_eq!(poll.positions.len(), 1);
        assert_eq!(opened.load(AtomicOrdering::SeqCst), 1, "the terminal signing in was kept, not started again");
        link.stop_and_wait();
    }
}

#[test]
fn a_terminal_that_keeps_reporting_another_account_is_logged_in_again() {
    let broker = broker("link-other-account");
    broker.open("1001", position(77));
    // The first login and the two reads after it report login 0 (`settle_reads` in all), then the
    // first three polls of the next login do: each time the link gives up and logs in again.
    let (link, heard, opened) = start_signing_in("link-other-account", 1, 2 + 3);
    for _ in 0..2 {
        let error = next_of_account(&heard, |event| match event {
            Event::State(_, LinkState::Retrying(error)) => Some(error.clone()),
            Event::State(..) | Event::Polled { .. } => None,
        });
        assert!(matches!(error, Mt5Error::Unreachable(_)), "tried again later: {error:?}");
    }
    let poll = next_of_account(&heard, |event| match event {
        Event::Polled { poll, .. } => Some(poll.clone()),
        Event::State(..) => None,
    });
    assert_eq!(poll.positions.len(), 1, "only what the account's own login reads is passed on");
    assert_eq!(opened.load(AtomicOrdering::SeqCst), 3);
    link.stop();
}

#[test]
fn an_order_given_up_while_the_link_logs_in_is_never_sent_later() {
    let broker = broker("link-late-order");
    broker.open("1001", position(77));
    let memory = memory::opener();
    let open: Opener = Arc::new(move |login: &Login| -> Result<Box<dyn Terminal>, Mt5Error> {
        Ok(Box::new(SlowLogin { inner: memory(login)?, delay: Duration::from_millis(800) }))
    });
    let (events, heard) = mpsc::channel();
    let quick = LinkSettings { order_queue: Duration::from_millis(100), ..settings() };
    let link = Link::start(
        login("link-late-order", "pass"),
        open,
        quick,
        Arc::new(move |event| {
            drop(events.send(event));
        }),
    );
    let refused = link.close(77, Some(0.05)).expect_err("the link is still logging in");
    assert!(matches!(refused, Mt5Error::Unreachable(_)), "{refused:?}");
    next(&heard, |e| matches!(e, Event::State(_, LinkState::Connected(_))).then_some(()));
    // Two polls later, the order the app gave up on has still not been carried out.
    for _ in 0..2 {
        next(&heard, |e| matches!(e, Event::Polled { .. }).then_some(()));
    }
    assert!((broker.positions("1001")[0].volume - 0.2).abs() < 1e-9, "nothing was closed");
    link.stop();
}
