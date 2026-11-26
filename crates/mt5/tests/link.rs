//! A link over the broker in memory: logging in, polling, orders, a refused password, and a
//! terminal that cannot be reached and comes back.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use tradelens_core::AccountInfo;
use tradelens_mt5::{
    AccountRef, Direction, Event, Levels, Link, LinkSettings, LinkState, Login, Mt5Error, Password, Position,
    memory::{self, Broker},
};

const WAIT: Duration = Duration::from_secs(10);

fn settings() -> LinkSettings {
    LinkSettings {
        poll_every: Duration::from_millis(50),
        retry_after: Duration::from_millis(100),
        first_days: 30,
        order_timeout: Duration::from_secs(5),
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
