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
    // It may be narrower than its text (a long setup name in a narrow column): the text ends in an ellipsis.
    view().flex_row().items_center().h(px(22.0)).min_w(px(0.0)).px_2().rounded(px(6.0)).bg(move || tone.fill()).child(
        text(label.into())
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .number_of_lines(1)
            .min_w(px(0.0))
            .text_color(move || tone.ink()),
    )
}

/// A profit or loss figure in the monospaced font: green with a plus, red with a minus.
/// Colour and sign always go together, so the result never depends on colour alone.
pub(crate) fn pnl_text(value: impl Fn() -> String + Clone + 'static) -> Text {
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
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;

    #[component]
    fn Figures() -> impl IntoElement {
        super::super::theme::install();
        crate::i18n::install();
        view()
            .child(pnl_text(|| "+34.4R".to_owned()))
            .child(figure_text(|| "47.1%".to_owned()))
            .child(super::super::parts::stat_tile("Win rate", || ("62.5%".to_owned(), Tone::Neutral), String::new))
            .child(text("Inter ở đây"))
    }

    fn family_of(app: &Headless, shown: &str) -> Option<String> {
        let style = app.screen().span_style(&format!("text:{shown}")).expect("the text is on screen");
        style.font_family.map(|family| family.name().to_string())
    }

    #[test]
    fn a_figure_is_drawn_in_the_monospaced_family_of_the_theme_with_tabular_digits() {
        let theme = super::super::theme::tokens().expect("theme.json parses");
        let mut app =
            Headless::start(EngineConfig::new(Root::of::<Figures>().tokens(theme))).expect("the engine starts");
        app.wait_for("+34.4R").expect("the figures show");
        for figure in ["+34.4R", "47.1%", "62.5%"] {
            assert_eq!(family_of(&app, figure).as_deref(), Some("JetBrains Mono"), "{figure} is a figure");
        }
        assert_ne!(
            family_of(&app, "Inter ở đây").as_deref(),
            Some("JetBrains Mono"),
            "what is not a figure is not set in mono"
        );
    }

    #[test]
    fn tone_follows_the_sign_of_an_amount() {
        assert_eq!(Tone::of_amount("+$24.46"), Tone::Profit);
        assert_eq!(Tone::of_amount("\u{2212}$5.64"), Tone::Loss);
        assert_eq!(Tone::of_amount("$0.00"), Tone::Neutral);
    }
}
