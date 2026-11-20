//! The server loop: one thread, `tiny_http`, a `Db` of its own (SQLite in WAL mode
//! lets it write while the UI reads).

use std::{
    sync::{Arc, atomic::AtomicBool},
    thread::JoinHandle,
};

use crate::{BridgeConfig, BridgeError, BridgeEvent, Revision};

pub(crate) type OnEvent = Box<dyn Fn(BridgeEvent) + Send>;

pub(crate) fn spawn(
    _config: BridgeConfig,
    _revision: Revision,
    _stop: Arc<AtomicBool>,
    _on_event: OnEvent,
) -> Result<JoinHandle<()>, BridgeError> {
    // TODO(v0.1): bind 127.0.0.1:port, loop on recv_timeout until `stop`,
    // route PING / DEALS / ACCOUNT, check TOKEN_HEADER, upsert, bump revision.
    todo!("v0.1")
}
