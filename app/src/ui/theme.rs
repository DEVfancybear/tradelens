//! The theme (`theme.json`, shadcn `cssVars`) and the app's own tokens.

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

/// The monospaced family numbers are drawn in (`font-mono` of `theme.json`).
pub(crate) fn mono() -> SharedString {
    theme().font_mono.clone().unwrap_or_else(|| SharedString::from("monospace"))
}
