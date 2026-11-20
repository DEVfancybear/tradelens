//! The theme (`theme.json`, shadcn `cssVars`) and the app's own tokens.

#![expect(dead_code, reason = "the skeleton's pages read these from v0.1")]

use nsh_base::{theme, tokens_of};
use nsh_core::{element::set_tokens, prelude::*};

/// The theme file, also named by `nsh.json` for `nsh run` and `nsh package`.
const THEME: &str = include_str!("../../theme.json");

/// Draws the app with its theme under `cargo run` too, where `nsh.json` is not read.
pub(crate) fn install() {
    match tokens_of(Some(THEME)) {
        Ok(tokens) => set_tokens(tokens),
        Err(error) => log::error!("theme.json: {error}"),
    }
}

/// A custom token; a misspelt name shows as the foreground, which review catches.
fn token(name: &str) -> AdaptiveColor {
    theme().color(name).unwrap_or(theme().foreground)
}

/// Money made: P/L above zero, green days.
pub(crate) fn profit() -> AdaptiveColor {
    token("profit")
}

pub(crate) fn profit_soft() -> AdaptiveColor {
    token("profit-soft")
}

/// Money lost.
pub(crate) fn loss() -> AdaptiveColor {
    token("loss")
}

pub(crate) fn loss_soft() -> AdaptiveColor {
    token("loss-soft")
}

/// A rule close to its limit.
pub(crate) fn warning() -> AdaptiveColor {
    token("warning")
}
