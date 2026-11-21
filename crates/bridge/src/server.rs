//! The server loop: one thread, `tiny_http`, a [`tradelens_core::Db`] of its own (SQLite in WAL mode
//! lets it write while the UI reads). This file speaks HTTP; [`crate::routes`] decides what a
//! request does.

use std::{
    io::{self, Read},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use tiny_http::{Header, Method, Request, Response, Server};
use tradelens_core::Db;

use crate::{
    BridgeConfig, BridgeError, BridgeEvent, Revision,
    api::{self, ApiError, MAX_SCREENSHOT_BYTES},
    routes::{Answer, Failure, Routes, ScreenshotHeaders},
};

pub(crate) type OnEvent = Box<dyn Fn(BridgeEvent) + Send>;

/// How often the loop looks at the stop flag; the longest a stop waits for an idle server.
const POLL: Duration = Duration::from_millis(250);

/// The largest JSON body accepted, in bytes. A batch of 200 deals is about 60 KB.
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// How much of a refused body is read and thrown away so the client sees the answer
/// instead of a reset connection.
const MAX_DRAIN_BYTES: u64 = 64 * 1024 * 1024;

/// A started server: its thread and the port it really listens on.
#[derive(Debug)]
pub(crate) struct Running {
    pub thread: JoinHandle<()>,
    pub port: u16,
}

/// The routes of the contract, `docs/sync-api.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Ping,
    Deals,
    Account,
    Screenshot,
}

impl Route {
    fn of(path: &str) -> Option<Self> {
        match path {
            api::PING => Some(Self::Ping),
            api::DEALS => Some(Self::Deals),
            api::ACCOUNT => Some(Self::Account),
            api::SCREENSHOT => Some(Self::Screenshot),
            _ => None,
        }
    }

    fn method(self) -> Method {
        match self {
            Self::Ping => Method::Get,
            Self::Deals | Self::Account | Self::Screenshot => Method::Post,
        }
    }

    fn allow(self) -> &'static str {
        match self {
            Self::Ping => "GET",
            Self::Deals | Self::Account | Self::Screenshot => "POST",
        }
    }

    fn body_limit(self) -> usize {
        match self {
            Self::Ping => 0,
            Self::Deals | Self::Account => MAX_JSON_BYTES,
            Self::Screenshot => MAX_SCREENSHOT_BYTES,
        }
    }
}

/// What goes back on the wire.
#[derive(Debug)]
struct Reply {
    status: u16,
    body: String,
    allow: Option<&'static str>,
}

impl Reply {
    fn error(status: u16, message: &str) -> Self {
        let body = serde_json::to_string(&ApiError { error: message.to_owned() }).unwrap_or_else(|_| "{}".to_owned());
        Self { status, body, allow: None }
    }

    fn from_result(result: Result<Answer, Failure>) -> Self {
        match result {
            Ok(Answer(body)) => Self { status: 200, body, allow: None },
            Err(Failure { status, message }) => Self::error(status, &message),
        }
    }
}

/// Binds `127.0.0.1:port` (0 picks a free one), opens the database and starts the loop.
pub(crate) fn spawn(
    config: BridgeConfig,
    revision: Revision,
    stop: Arc<AtomicBool>,
    on_event: OnEvent,
) -> Result<Running, BridgeError> {
    let db = Db::open(&config.db_path)?;
    let bind = |message: String| BridgeError::Bind { port: config.port, message };
    let server = Server::http(("127.0.0.1", config.port)).map_err(|e| bind(e.to_string()))?;
    let port = server.server_addr().to_ip().map_or(config.port, |addr| addr.port());
    let data_dir = config.db_path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let routes = Routes::new(db, data_dir.join("attachments"), revision, on_event);
    let token = config.token;
    let thread = std::thread::Builder::new()
        .name("tradelens-bridge".to_owned())
        .spawn(move || run(server, routes, &token, &stop, port))
        .map_err(|e| bind(e.to_string()))?;
    Ok(Running { thread, port })
}

fn run(server: Server, mut routes: Routes, token: &str, stop: &AtomicBool, port: u16) {
    routes.emit(BridgeEvent::Listening { port });
    log::info!("bridge nghe tại 127.0.0.1:{port}");
    let error = loop {
        if stop.load(Ordering::Acquire) {
            break None;
        }
        match server.recv_timeout(POLL) {
            Ok(Some(request)) => serve(request, token, &mut routes),
            Ok(None) => {}
            Err(error) => {
                log::error!("bridge dừng vì lỗi: {error}");
                break Some(error.to_string());
            }
        }
    };
    // Dropping the server closes the listener, so the port is free before anyone hears of the stop.
    drop(server);
    routes.emit(BridgeEvent::Stopped { error });
}

/// Answers one request. Auth comes first, so a client without the token is refused
/// before anything of its body is read.
fn serve(mut request: Request, token: &str, routes: &mut Routes) {
    let reply = answer(&mut request, token, routes);
    if reply.status >= 400 {
        log::warn!("{} {} -> {}: {}", request.method(), request.url(), reply.status, reply.body);
        routes.emit(BridgeEvent::Rejected { reason: rejection(&reply) });
    }
    send(request, reply);
}

fn answer(request: &mut Request, token: &str, routes: &mut Routes) -> Reply {
    if !authorized(request, token) {
        return Reply::error(401, "Sai hoặc thiếu token (header X-TradeLens-Token)");
    }
    let path = request.url().split(['?', '#']).next().unwrap_or_default();
    let Some(route) = Route::of(path) else {
        return Reply::error(404, "Không có đường dẫn này");
    };
    if *request.method() != route.method() {
        return Reply { allow: Some(route.allow()), ..Reply::error(405, "Sai phương thức HTTP") };
    }
    let headers = ScreenshotHeaders {
        login: header(request, api::LOGIN_HEADER),
        server: header(request, api::SERVER_HEADER),
        position: header(request, api::POSITION_HEADER),
        moment: header(request, api::MOMENT_HEADER),
    };
    let body = match read_body(request, route) {
        Ok(body) => body,
        Err(reply) => return reply,
    };
    let result = match route {
        Route::Ping => Ok(Routes::ping()),
        Route::Deals => routes.deals(&body),
        Route::Account => routes.account(&body),
        Route::Screenshot => routes.screenshot(&headers, &body),
    };
    Reply::from_result(result)
}

/// The body of a POST, at most the route's limit; a GET has none to read.
fn read_body(request: &mut Request, route: Route) -> Result<Vec<u8>, Reply> {
    let limit = route.body_limit();
    if route.method() == Method::Get {
        return Ok(Vec::new());
    }
    let too_large = || Reply::error(413, &format!("Nội dung gửi lên lớn hơn {} KB", limit / 1024));
    if request.body_length().is_some_and(|length| length > limit) {
        drain(request);
        return Err(too_large());
    }
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut body = Vec::new();
    if let Err(error) = request.as_reader().take(cap).read_to_end(&mut body) {
        log::warn!("không đọc hết nội dung gửi lên: {error}");
        return Err(Reply::error(400, "Không đọc được nội dung gửi lên"));
    }
    if body.len() > limit {
        drain(request);
        return Err(too_large());
    }
    if body.is_empty() {
        return Err(Reply::error(400, "Nội dung gửi lên trống"));
    }
    Ok(body)
}

/// Reads and drops what is left of a refused body, up to [`MAX_DRAIN_BYTES`].
fn drain(request: &mut Request) {
    // A failed drain only means the client sees a closed connection instead of the 413.
    if let Err(error) = io::copy(&mut request.as_reader().take(MAX_DRAIN_BYTES), &mut io::sink()) {
        log::debug!("không bỏ được phần còn lại của nội dung bị từ chối: {error}");
    }
}

fn send(request: Request, reply: Reply) {
    let mut response = Response::from_string(reply.body)
        .with_status_code(reply.status)
        .with_header(ascii_header("Content-Type", "application/json; charset=utf-8"));
    if let Some(allow) = reply.allow {
        response = response.with_header(ascii_header("Allow", allow));
    }
    if let Err(error) = request.respond(response) {
        log::debug!("không gửi được trả lời: {error}");
    }
}

fn ascii_header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("header names and values here are plain ASCII")
}

/// The error message of a reply, for the data sources screen.
fn rejection(reply: &Reply) -> String {
    serde_json::from_str::<ApiError>(&reply.body).map_or_else(|_| format!("Lỗi {}", reply.status), |e| e.error)
}

fn header(request: &Request, name: &'static str) -> Option<String> {
    request.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_owned())
}

fn authorized(request: &Request, token: &str) -> bool {
    !token.is_empty() && header(request, api::TOKEN_HEADER).is_some_and(|given| tokens_match(given.trim(), token))
}

/// Compares in time that depends only on the expected token's length, so response
/// timing does not tell a guess how many leading bytes were right.
fn tokens_match(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    let mut difference = given.len() ^ expected.len();
    for (index, byte) in expected.iter().enumerate() {
        difference |= usize::from(given.get(index).copied().unwrap_or(0) ^ byte);
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_only_when_equal() {
        assert!(tokens_match("abc123", "abc123"));
        assert!(!tokens_match("abc124", "abc123"));
        assert!(!tokens_match("abc12", "abc123"));
        assert!(!tokens_match("abc1234", "abc123"));
        assert!(!tokens_match("", "abc123"));
    }

    #[test]
    fn routes_know_their_paths() {
        assert_eq!(Route::of("/api/v1/deals"), Some(Route::Deals));
        assert_eq!(Route::of("/api/v1/deals/"), None);
        assert_eq!(Route::of("/"), None);
    }
}
