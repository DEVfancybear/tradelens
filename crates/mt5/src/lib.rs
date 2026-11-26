//! The direct MT5 connection: for each account the trader signs in to, a portable copy of their
//! MetaTrader 5 terminal runs hidden and a Python worker drives it through the official
//! `MetaTrader5` package. No EA is installed in the terminal.
//!
//! - [`Terminal`] is what a connection asks of a terminal; [`python::Worker`] is the real one,
//!   [`memory::Broker`] a broker in memory for tests and the demo.
//! - [`Link`] keeps one account connected on a thread of its own: it logs in, reads the deals and
//!   the open positions every few seconds and tells the app through [`Event`]s, and it carries the
//!   trader's orders (close, move the stop or the target).
//! - [`install`] finds the MT5 terminals installed on the machine and makes an account's portable
//!   copy; [`python`] finds a Python that has the package. [`installer`] installs MetaTrader 5
//!   itself with MetaQuotes' installer when the machine has none.
//!
//! Nothing here touches the database or the UI: the app writes what a poll brings.

pub mod install;
pub mod installer;
mod link;
pub mod memory;
mod protocol;
pub mod python;
pub mod runtime;

pub use link::{Event, Link, LinkSettings, LinkState, Listener, Opener};
pub use protocol::{AccountRef, Direction, Levels, Login, Mt5Error, Password, Poll, Position, Terminal};
