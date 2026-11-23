//! A local HTTP server that plays the providers: token, userinfo, Drive, Graph and Notion.
//! Each test hands it a function from request to reply and reads back what was asked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use percent_encoding::percent_decode_str;
use serde_json::Value;
use tiny_http::{Header, Response, Server, StatusCode};

/// A request the server received.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) method: String,
    /// The path and the query, as sent.
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Seen {
    pub(crate) fn path(&self) -> &str {
        self.url.split('?').next().unwrap_or_default()
    }

    /// The decoded value of a query parameter.
    pub(crate) fn param(&self, name: &str) -> Option<String> {
        let query = self.url.split_once('?')?.1;
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| percent_decode_str(&value.replace('+', " ")).decode_utf8_lossy().into_owned())
        })
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as a form: `a=1&b=2`, decoded.
    pub(crate) fn form(&self) -> Vec<(String, String)> {
        let decode = |s: &str| percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
        self.text()
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (decode(k), decode(v))
            })
            .collect()
    }

    pub(crate) fn form_value(&self, name: &str) -> Option<String> {
        self.form().into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }
}

/// What the server answers.
#[derive(Debug, Clone)]
pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    pub(crate) fn status(status: u16) -> Self {
        Self { status, headers: Vec::new(), body: Vec::new() }
    }

    pub(crate) fn json(status: u16, body: &Value) -> Self {
        Self::status(status).header("Content-Type", "application/json").body(body.to_string().into_bytes())
    }

    pub(crate) fn ok(body: &Value) -> Self {
        Self::json(200, body)
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
}

/// A server on a free port, stopped when dropped.
#[derive(Debug)]
pub(crate) struct Mock {
    pub(crate) base: String,
    log: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Mock {
    /// Starts the server; `handler` gets every request and the server's own base URL.
    pub(crate) fn start(handler: impl Fn(&Seen, &str) -> Reply + Send + 'static) -> Self {
        let server = Server::http("127.0.0.1:0").expect("a free port");
        let base = format!("http://{}", server.server_addr().to_ip().expect("an IP address"));
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (log, stop, base) = (Arc::clone(&log), Arc::clone(&stop), base.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50)) else { continue };
                    let mut body = Vec::new();
                    request.as_reader().read_to_end(&mut body).expect("the request body is readable");
                    let seen = Seen {
                        method: request.method().as_str().to_owned(),
                        url: request.url().to_owned(),
                        headers: request
                            .headers()
                            .iter()
                            .map(|h| (h.field.as_str().as_str().to_owned(), h.value.as_str().to_owned()))
                            .collect(),
                        body,
                    };
                    log.lock().expect("lock poisoned").push(seen.clone());
                    let reply = handler(&seen, &base);
                    let mut response = Response::from_data(reply.body).with_status_code(StatusCode(reply.status));
                    for (name, value) in &reply.headers {
                        response
                            .add_header(Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a valid header"));
                    }
                    request.respond(response).expect("the client is still there");
                }
            })
        };
        Self { base, log, stop, thread: Some(thread) }
    }

    /// Every request so far, oldest first.
    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.log.lock().expect("lock poisoned").clone()
    }

    /// The requests to `path` (query ignored), oldest first.
    pub(crate) fn requests_to(&self, path: &str) -> Vec<Seen> {
        self.requests().into_iter().filter(|r| r.path() == path).collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The server thread only panics on a broken test; that is reported by the test itself.
            thread.join().ok();
        }
    }
}
