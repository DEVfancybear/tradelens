//! `tradelens-cloud` against a local server that plays Google, Microsoft and Notion.
//! No test here touches the network or the OS credential store (apart from the one marked
//! `#[ignore]`).

mod auth;
mod drives;
mod mock;
mod notion;
mod store;
