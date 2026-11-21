//! Small marks that carry a state or a result: chips and profit/loss text.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::theme::{self as tokens, loss, loss_soft, profit, profit_soft};
use crate::format;

/// What a chip or a figure says about its subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Neutral,
    Profit,
    Loss,
    Primary,
}

impl Tone {
    /// The tone of a displayed amount: its sign decides.
    pub(crate) fn of_amount(text: &str) -> Self {
        if format::is_gain(text) {
            Self::Profit
        } else if format::is_loss(text) {
            Self::Loss
        } else {
            Self::Neutral
        }
    }

    fn ink(self) -> AdaptiveColor {
        match self {
            Self::Neutral => theme().foreground,
            Self::Profit => profit(),
            Self::Loss => loss(),
            Self::Primary => theme().primary,
        }
    }

    fn fill(self) -> AdaptiveColor {
        match self {
            Self::Neutral => theme().secondary,
            Self::Profit => profit_soft(),
            Self::Loss => loss_soft(),
            Self::Primary => theme().accent,
        }
    }

    /// The colour of a figure: muted where nothing is gained or lost.
    fn figure(self) -> AdaptiveColor {
        match self {
            Self::Neutral => theme().muted_foreground,
            other => other.ink(),
        }
    }
}

/// A small rounded label: Buy / Sell, a setup, a state.
pub(crate) fn chip(label: impl Into<SharedString>, tone: Tone) -> impl IntoElement {
    view().flex_row().items_center().h(px(22.0)).px_2().rounded(px(6.0)).bg(move || tone.fill()).child(
        text(label.into()).text_xs().font_weight(FontWeight::MEDIUM).number_of_lines(1).text_color(move || tone.ink()),
    )
}

/// A profit or loss figure in the monospaced font: green with a plus, red with a minus.
/// Colour and sign always go together, so the result never depends on colour alone.
pub(crate) fn pnl_text(value: impl Fn() -> String + Clone + 'static) -> impl IntoElement {
    let shown = value.clone();
    text(shown)
        .font_family(tokens::mono)
        .tabular_nums()
        .number_of_lines(1)
        .text_color(move || Tone::of_amount(&value()).figure())
}

/// A figure in the monospaced font, in the default ink.
pub(crate) fn figure_text(value: impl Fn() -> String + 'static) -> Text {
    text(value).font_family(tokens::mono).tabular_nums().number_of_lines(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_follows_the_sign_of_an_amount() {
        assert_eq!(Tone::of_amount("+$24.46"), Tone::Profit);
        assert_eq!(Tone::of_amount("\u{2212}$5.64"), Tone::Loss);
        assert_eq!(Tone::of_amount("$0.00"), Tone::Neutral);
    }
}
