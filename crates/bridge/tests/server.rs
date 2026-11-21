//! The bridge over real HTTP: a server on a free port, a temp-file database, and a plain
//! `TcpStream` client, the way the EA's `WebRequest` and the Python script reach it.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use tempfile::TempDir;
use tradelens_bridge::{
    Bridge, BridgeConfig, BridgeError, BridgeEvent, Revision,
    api::{self, AccountAccepted, ApiError, DealsAccepted, Pong, ScreenshotStored},
};
use tradelens_core::{Db, DealEntry, Side, TradeKey, attachments::Source};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body {:?} is not the expected JSON: {e}", self.body))
    }

    fn error(&self) -> String {
        self.json::<ApiError>().error
    }
}

/// One request on its own connection, answered once.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream.set_read_timeout(Some(Duration::from_secs(20))).expect("sets a timeout");
    let mut lines =
        vec![format!("{method} {path} HTTP/1.1"), "Host: 127.0.0.1".to_owned(), "Connection: close".to_owned()];
    if method == "POST" {
        lines.push(format!("Content-Length: {}", body.len()));
    }
    lines.extend(headers.iter().map(|(name, value)| format!("{name}: {value}")));
    let head = format!("{}\r\n\r\n", lines.join("\r\n"));
    stream.write_all(head.as_bytes()).expect("sends the head");
    stream.write_all(body).expect("sends the body");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("reads the answer");

    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a complete head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).expect("a status line");
    let headers =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    Reply { status, headers, body: String::from_utf8_lossy(&raw[split + 4..]).into_owned() }
}

struct Fixture {
    dir: TempDir,
    bridge: Bridge,
    events: Receiver<BridgeEvent>,
}

impl Fixture {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (bridge, events) = start_in(dir.path(), 0).expect("the bridge starts");
        let fixture = Self { dir, bridge, events };
        let first = fixture.events.recv_timeout(Duration::from_secs(5)).expect("a first event");
        assert_eq!(first, BridgeEvent::Listening { port: fixture.port() }, "the first event announces the port");
        fixture
    }

    fn port(&self) -> u16 {
        self.bridge.port()
    }

    /// A request carrying the right token.
    fn call(&self, method: &str, path: &str, extra: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut headers = vec![(api::TOKEN_HEADER, TOKEN)];
        headers.extend_from_slice(extra);
        http(self.port(), method, path, &headers, body)
    }

    fn post(&self, path: &str, body: &str) -> Reply {
        self.call("POST", path, &[("Content-Type", "application/json")], body.as_bytes())
    }

    fn screenshot(&self, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        let mut all = vec![("Content-Type", "image/png")];
        all.extend_from_slice(headers);
        self.call("POST", api::SCREENSHOT, &all, body)
    }

    /// Another connection to the same database file, like the UI's.
    fn db(&self) -> Db {
        Db::open(self.dir.path().join("tradelens.db")).expect("opens the database")
    }

    fn events(&self) -> Vec<BridgeEvent> {
        self.events.try_iter().collect()
    }

    fn revision(&self) -> u64 {
        self.bridge.revision.get()
    }

    fn attachments(&self) -> std::path::PathBuf {
        self.dir.path().join("attachments")
    }
}

fn start_in(dir: &Path, port: u16) -> Result<(Bridge, Receiver<BridgeEvent>), BridgeError> {
    let (tx, events) = mpsc::channel();
    let config = BridgeConfig { db_path: dir.join("tradelens.db"), port, token: TOKEN.to_owned() };
    let bridge = Bridge::start(config, Revision::default(), move |event| {
        // The test may have stopped listening; events are then simply dropped.
        tx.send(event).ok();
    })?;
    Ok((bridge, events))
}

const ACCOUNT: &str = r#"{"login":"52012345","server":"Demo-Server","broker":"Raw Trading Ltd","currency":"USD","balance":10250.4,"equity":10310.0,"server_offset_min":120}"#;

fn deal(ticket: i64, position: i64, entry: &str, kind: &str, profit: f64) -> String {
    format!(
        r#"{{"ticket":{ticket},"order":{ticket},"position_id":{position},"time_msc":{},"symbol":"XAUUSD","type":"{kind}","entry":"{entry}","volume":0.1,"price":2650.25,"commission":-0.7,"swap":0,"profit":{profit},"sl":2640.0,"tp":2670.0,"risk":102.5,"reason":"expert"}}"#,
        1_763_650_000_000_i64 + ticket
    )
}

fn batch(deals: &[String]) -> String {
    format!(r#"{{"account":{ACCOUNT},"deals":[{}]}}"#, deals.join(","))
}

/// A tiny but valid-looking PNG: the signature plus padding.
fn png(fill: u8) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend(std::iter::repeat_n(fill, 2_000));
    bytes
}

fn shot_headers<'a>(position: &'a str, moment: &'a str) -> [(&'a str, &'a str); 4] {
    [
        (api::LOGIN_HEADER, "52012345"),
        (api::SERVER_HEADER, "Demo-Server"),
        (api::POSITION_HEADER, position),
        (api::MOMENT_HEADER, moment),
    ]
}

#[test]
fn ping_answers_with_the_app_name() {
    let fx = Fixture::start();
    let reply = fx.call("GET", api::PING, &[], b"");
    assert_eq!(reply.status, 200);
    assert!(reply.header("Content-Type").is_some_and(|t| t.starts_with("application/json")));
    let pong: Pong = reply.json();
    assert_eq!(pong.app, "TradeLens");
    assert_ne!(pong.version, "", "the version is reported");
}

#[test]
fn a_missing_or_wrong_token_is_refused_before_anything_is_stored() {
    let fx = Fixture::start();
    let body = batch(&[deal(1, 100, "in", "buy", 0.0)]);
    let anonymous = http(fx.port(), "POST", api::DEALS, &[], body.as_bytes());
    assert_eq!(anonymous.status, 401);
    assert_ne!(anonymous.error(), "", "the refusal says why");
    for wrong in ["", "nope", &format!("{TOKEN}0"), &TOKEN[..TOKEN.len() - 1]] {
        let reply = http(fx.port(), "POST", api::DEALS, &[(api::TOKEN_HEADER, wrong)], body.as_bytes());
        assert_eq!(reply.status, 401, "token {wrong:?}");
    }
    assert_eq!(http(fx.port(), "GET", api::PING, &[], b"").status, 401, "ping needs the token too");
    // Header names are case-insensitive on the wire.
    assert_eq!(http(fx.port(), "GET", api::PING, &[("x-tradelens-token", TOKEN)], b"").status, 200);

    assert_eq!(fx.revision(), 0);
    assert_eq!(fx.db().deals(None).expect("reads"), Vec::new(), "nothing was stored");
    assert!(fx.events().iter().any(|e| matches!(e, BridgeEvent::Rejected { .. })));
}

#[test]
fn a_batch_is_stored_and_visible_to_the_ui_at_once() {
    let fx = Fixture::start();
    let started = Instant::now();
    let reply = fx.post(api::DEALS, &batch(&[deal(1, 100, "in", "buy", 0.0), deal(2, 100, "out", "sell", 25.5)]));
    assert_eq!(reply.status, 200, "{}", reply.body);
    let accepted: DealsAccepted = reply.json();
    assert_eq!((accepted.inserted, accepted.updated), (2, 0));

    // Through a second connection, as the UI reads it, right after the answer.
    let db = fx.db();
    let deals = db.deals(Some(accepted.account_id)).expect("reads");
    assert_eq!(deals.len(), 2);
    assert!(started.elapsed() < Duration::from_secs(5), "a closed trade must show within 5 s");
    let account = &db.accounts().expect("reads")[0];
    assert_eq!((account.login.as_str(), account.server.as_str()), ("52012345", "Demo-Server"));
    assert_eq!(account.last_balance, Some(10250.4));
    assert_eq!(fx.revision(), 1);

    let events = fx.events();
    assert!(events.contains(&BridgeEvent::Received {
        source: "ea".to_owned(),
        account: "52012345".to_owned(),
        inserted: 2,
        updated: 0
    }));
    assert!(
        !events.iter().any(|e| matches!(e, BridgeEvent::PositionOpened(_))),
        "a position that is already closed asks for no plan"
    );
}

#[test]
fn resending_a_batch_changes_nothing_and_does_not_move_the_revision() {
    let fx = Fixture::start();
    let body = batch(&[deal(1, 100, "in", "buy", 0.0), deal(2, 100, "out", "sell", 25.5)]);
    assert_eq!(fx.post(api::DEALS, &body).status, 200);
    assert_eq!(fx.revision(), 1);
    fx.events();

    let again: DealsAccepted = fx.post(api::DEALS, &body).json();
    assert_eq!((again.inserted, again.updated), (0, 0));
    assert_eq!(fx.revision(), 1, "nothing changed, so the UI is not told to reload");
    assert_eq!(fx.db().deals(None).expect("reads").len(), 2);

    // A changed deal is an update and moves the revision.
    let corrected = batch(&[deal(2, 100, "out", "sell", 30.0)]);
    let changed: DealsAccepted = fx.post(api::DEALS, &corrected).json();
    assert_eq!((changed.inserted, changed.updated), (0, 1));
    assert_eq!(fx.revision(), 2);
}

#[test]
fn an_opening_deal_announces_the_position_once() {
    let fx = Fixture::start();
    let body = batch(&[deal(1, 100, "in", "sell", 0.0)]);
    assert_eq!(fx.post(api::DEALS, &body).status, 200);
    let opened: Vec<_> = fx
        .events()
        .into_iter()
        .filter_map(|e| match e {
            BridgeEvent::PositionOpened(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 1);
    assert_eq!((opened[0].account.as_str(), opened[0].position_id), ("52012345", 100));
    assert_eq!((opened[0].symbol.as_str(), opened[0].side), ("XAUUSD", Side::Short));
    assert_eq!((opened[0].volume, opened[0].price, opened[0].sl, opened[0].tp), (0.1, 2650.25, 2640.0, 2670.0));

    // The EA resends deals around its cursor: no second prompt for the same position.
    assert_eq!(fx.post(api::DEALS, &body).status, 200);
    assert!(!fx.events().iter().any(|e| matches!(e, BridgeEvent::PositionOpened(_))));

    // Its later exit announces nothing, and a backfilled closed position neither.
    let rest =
        batch(&[deal(2, 100, "out", "buy", 12.0), deal(3, 200, "in", "buy", 0.0), deal(4, 200, "out", "sell", 5.0)]);
    assert_eq!(fx.post(api::DEALS, &rest).status, 200);
    assert!(!fx.events().iter().any(|e| matches!(e, BridgeEvent::PositionOpened(_))));
    let stored = fx.db().deals(None).expect("reads");
    assert!(stored.iter().any(|(_, d)| d.entry == DealEntry::Out));
}

#[test]
fn an_account_post_updates_the_balance_and_moves_the_revision_only_when_it_changed() {
    let fx = Fixture::start();
    let post = |balance: f64| {
        let body = format!(
            r#"{{"account":{{"login":"52012345","server":"Demo-Server","balance":{balance},"equity":{balance}}}}}"#
        );
        fx.post(api::ACCOUNT, &body)
    };
    let first = post(10_000.0);
    assert_eq!(first.status, 200, "{}", first.body);
    let accepted: AccountAccepted = first.json();
    assert_eq!(fx.revision(), 1);
    assert_eq!(fx.db().accounts().expect("reads")[0].last_balance, Some(10_000.0));

    assert_eq!(post(10_000.0).status, 200);
    assert_eq!(fx.revision(), 1, "a periodic post with the same numbers is not news");

    let moved: AccountAccepted = post(10_500.0).json();
    assert_eq!(moved.account_id, accepted.account_id);
    assert_eq!(fx.revision(), 2);
    assert_eq!(fx.db().accounts().expect("reads")[0].last_balance, Some(10_500.0));
    assert!(fx.events().iter().any(|e| matches!(e, BridgeEvent::Received { inserted: 0, updated: 0, .. })));
}

#[test]
fn bad_json_is_a_400_with_a_vietnamese_message_and_stores_nothing() {
    let fx = Fixture::start();
    for (path, body) in [
        (api::DEALS, "{not json"),
        (api::DEALS, r#"{"deals":[]}"#),
        (api::DEALS, &batch(&[r#"{"ticket":"abc"}"#.to_owned()])),
        (api::ACCOUNT, r#"{"balance":1}"#),
        (api::ACCOUNT, ""),
    ] {
        let reply = fx.post(path, body);
        assert_eq!(reply.status, 400, "{path} {body}");
        assert!(
            reply.error().starts_with("JSON không hợp lệ") || reply.error().starts_with("Nội dung"),
            "{}",
            reply.body
        );
    }
    // A batch without a login parses but is not accepted either.
    let nameless = r#"{"account":{"login":"  "},"deals":[]}"#;
    assert_eq!(fx.post(api::DEALS, nameless).status, 400);
    assert_eq!(fx.revision(), 0);
    assert_eq!(fx.db().accounts().expect("reads"), Vec::new(), "no account was made");
    // The server is still up.
    assert_eq!(fx.call("GET", api::PING, &[], b"").status, 200);
}

#[test]
fn a_big_batch_passes_and_an_oversized_body_is_a_413() {
    let fx = Fixture::start();
    let deals: Vec<String> = (1..=200).map(|i| deal(i, 1000 + i, "in", "buy", 0.0)).collect();
    let big = batch(&deals);
    assert!(big.len() > 40_000, "a realistic 200-deal batch");
    let accepted: DealsAccepted = fx.post(api::DEALS, &big).json();
    assert_eq!(accepted.inserted, 200);

    let huge = vec![b' '; 5 * 1024 * 1024];
    let refused = fx.call("POST", api::DEALS, &[("Content-Type", "application/json")], &huge);
    assert_eq!(refused.status, 413);
    assert_ne!(refused.error(), "", "the refusal says why");
    assert_eq!(fx.call("GET", api::PING, &[], b"").status, 200, "the server survives a refused body");
}

#[test]
fn other_methods_and_paths_are_405_and_404() {
    let fx = Fixture::start();
    let wrong_method = fx.call("GET", api::DEALS, &[], b"");
    assert_eq!(wrong_method.status, 405);
    assert_eq!(wrong_method.header("Allow"), Some("POST"));
    assert_eq!(fx.call("POST", api::PING, &[], b"{}").status, 405);
    assert_eq!(fx.call("DELETE", api::ACCOUNT, &[], b"").status, 405);
    assert_eq!(fx.call("GET", "/api/v1/nothing", &[], b"").status, 404);
    assert_eq!(fx.call("GET", "/", &[], b"").status, 404);
    // A query string does not change the route.
    assert_eq!(fx.call("GET", "/api/v1/ping?x=1", &[], b"").status, 200);
}

#[test]
fn a_screenshot_is_saved_and_linked_to_the_trade_journal() {
    let fx = Fixture::start();
    let accepted: DealsAccepted = fx.post(api::DEALS, &batch(&[deal(1, 100, "in", "buy", 0.0)])).json();
    let key = TradeKey { account_id: accepted.account_id, position_id: 100, segment: 0 };
    let revision = fx.revision();

    let first = png(1);
    let reply = fx.screenshot(&shot_headers("100", "entry"), &first);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let stored: ScreenshotStored = reply.json();
    assert_eq!(stored.path, "52012345/100-entry.png");
    assert_eq!(fs::read(fx.attachments().join(&stored.path)).expect("the file is there"), first);
    assert!(fx.revision() > revision);

    let journal = fx.db().journal(&key).expect("reads").expect("the entry was created");
    assert_eq!(journal.screenshots.len(), 1);
    assert_eq!(journal.screenshots[0].path, stored.path);
    assert_eq!(journal.screenshots[0].source, Source::EaEntry);

    // Sent again, the image is replaced and still linked once.
    let second = png(2);
    assert_eq!(fx.screenshot(&shot_headers("100", "entry"), &second).status, 200);
    assert_eq!(fs::read(fx.attachments().join(&stored.path)).expect("the file is there"), second);
    assert_eq!(fx.db().journal(&key).expect("reads").expect("kept").screenshots.len(), 1);

    // The exit shot joins the same entry, next to the trader's own words.
    let mut written = fx.db().journal(&key).expect("reads").expect("kept");
    written.notes = "Vào theo kế hoạch".to_owned();
    let mut db = fx.db();
    db.save_journal(&key, &written).expect("saves");
    let exit: ScreenshotStored = fx.screenshot(&shot_headers("100", "exit"), &png(3)).json();
    assert_eq!(exit.path, "52012345/100-exit.png");
    let journal = fx.db().journal(&key).expect("reads").expect("kept");
    assert_eq!(journal.notes, "Vào theo kế hoạch");
    assert_eq!(journal.screenshots.iter().map(|s| s.source).collect::<Vec<_>>(), [Source::EaEntry, Source::EaExit]);
    assert!(
        !fs::read_dir(fx.attachments().join("52012345"))
            .expect("lists")
            .any(|f| { f.expect("an entry").file_name().to_string_lossy().ends_with(".part") })
    );
}

#[test]
fn a_screenshot_with_missing_or_bad_parts_is_refused() {
    let fx = Fixture::start();
    assert_eq!(fx.post(api::DEALS, &batch(&[deal(1, 100, "in", "buy", 0.0)])).status, 200);
    let good = png(1);

    let without = |skip: &str| -> Vec<(&'static str, &'static str)> {
        shot_headers("100", "entry").into_iter().filter(|(name, _)| *name != skip).collect()
    };
    assert_eq!(fx.screenshot(&without(api::LOGIN_HEADER), &good).status, 400);
    assert_eq!(fx.screenshot(&without(api::POSITION_HEADER), &good).status, 400);
    assert_eq!(fx.screenshot(&without(api::MOMENT_HEADER), &good).status, 400);
    assert_eq!(fx.screenshot(&shot_headers("abc", "entry"), &good).status, 400);
    assert_eq!(fx.screenshot(&shot_headers("-5", "entry"), &good).status, 400);
    assert_eq!(fx.screenshot(&shot_headers("100", "middle"), &good).status, 400);
    let hostile = [(api::LOGIN_HEADER, "../.."), (api::POSITION_HEADER, "100"), (api::MOMENT_HEADER, "exit")];
    assert_eq!(fx.screenshot(&hostile, &good).status, 400, "a login with no safe characters");
    let reply = fx.screenshot(&shot_headers("100", "entry"), b"<html>not an image</html>");
    assert_eq!(reply.status, 400);
    assert!(reply.error().contains("PNG"));
    // Without a server header the account is found by login; an unknown login is a 404.
    let unknown = [(api::LOGIN_HEADER, "777"), (api::POSITION_HEADER, "100"), (api::MOMENT_HEADER, "entry")];
    assert_eq!(fx.screenshot(&unknown, &good).status, 404);
    let by_login = [(api::LOGIN_HEADER, "52012345"), (api::POSITION_HEADER, "100"), (api::MOMENT_HEADER, "entry")];
    assert_eq!(fx.screenshot(&by_login, &good).status, 200);

    assert!(!fx.attachments().join("777").exists());
}

#[test]
fn a_screenshot_over_the_limit_is_a_413_and_one_for_a_new_account_creates_it() {
    let fx = Fixture::start();
    let mut huge = png(0);
    huge.resize(api::MAX_SCREENSHOT_BYTES + 1, 0);
    let refused = fx.screenshot(&shot_headers("100", "entry"), &huge);
    assert_eq!(refused.status, 413);
    assert!(!fx.attachments().join("52012345").exists());

    // The shot arrives before the account's first sync: the account is made, the sync completes it.
    assert_eq!(fx.screenshot(&shot_headers("100", "entry"), &png(1)).status, 200);
    let accounts = fx.db().accounts().expect("reads");
    assert_eq!(
        (accounts.len(), accounts[0].login.as_str(), accounts[0].server.as_str()),
        (1, "52012345", "Demo-Server")
    );
    assert_eq!(fx.post(api::DEALS, &batch(&[deal(1, 100, "in", "buy", 0.0)])).status, 200);
    assert_eq!(fx.db().accounts().expect("reads").len(), 1);
}

#[test]
fn stopping_releases_the_port_quickly() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (bridge, events) = start_in(dir.path(), 0).expect("starts");
    let port = bridge.port();
    assert_ne!(port, 0);
    assert_eq!(http(port, "GET", api::PING, &[(api::TOKEN_HEADER, TOKEN)], b"").status, 200);

    // A second bridge cannot take the same port.
    let other = tempfile::tempdir().expect("a temp dir");
    assert!(matches!(start_in(other.path(), port), Err(BridgeError::Bind { port: p, .. }) if p == port));

    let started = Instant::now();
    drop(bridge);
    assert!(started.elapsed() < Duration::from_secs(2), "stopping took {:?}", started.elapsed());
    assert!(events.try_iter().any(|e| e == BridgeEvent::Stopped { error: None }));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => break,
            Err(error) => assert!(Instant::now() < deadline, "the port is still held: {error}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
