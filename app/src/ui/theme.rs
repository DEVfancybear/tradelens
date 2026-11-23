//! The theme (`theme.json`, shadcn `cssVars`) and the app's own tokens.

use nsh_base::{theme, tokens_of};
use nsh_core::{
    element::{Tokens, set_tokens},
    prelude::*,
};

/// The theme file, also named by `nsh.json` for `nsh run` and `nsh package`.
const THEME: &str = include_str!("../../theme.json");

/// The tokens of `theme.json`; what cannot be read is logged and leaves the default theme.
pub(crate) fn tokens() -> Option<Tokens> {
    match tokens_of(Some(THEME)) {
        Ok(tokens) => Some(tokens),
        Err(error) => {
            log::error!("theme.json: {error}");
            None
        }
    }
}

/// Draws the app with its theme under `cargo run` too, where `nsh.json` is not read. The root
/// carries the same tokens ([`crate::root`]) so that the first frame already has them, with the
/// fonts they name; this makes a component mounted without that root (a test's) have them too.
pub(crate) fn install() {
    if let Some(tokens) = tokens() {
        set_tokens(tokens);
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
