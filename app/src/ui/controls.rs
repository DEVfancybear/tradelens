//! Styled controls: buttons, the sidebar's rows, segmented switches, selects, text fields,
//! tag pickers and the star rating. Pages compose these and never style a control themselves.

use std::rc::Rc;

use nsh_base::{SelectEntry, SelectItem, button, input, select, theme, toggle, toggle_group};
use nsh_core::prelude::*;

use super::motion;

/// How much a button stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    /// The page's main action.
    Primary,
    /// Other actions.
    Secondary,
    /// Actions in toolbars and rows.
    Ghost,
}

/// The app's button.
pub(crate) fn action(
    label: impl IntoReactive<SharedString>,
    variant: Variant,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    action_when(label, variant, || true, on_press)
}

/// A button that can be switched off while `enabled` is false.
pub(crate) fn action_when(
    label: impl IntoReactive<SharedString>,
    variant: Variant,
    enabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let (fill, ink) = match variant {
        Variant::Primary => (theme().primary, theme().primary_foreground),
        Variant::Secondary => (theme().secondary, theme().secondary_foreground),
        Variant::Ghost => (AdaptiveColor::TRANSPARENT, theme().foreground),
    };
    button()
        .on_press(on_press)
        .disabled(move || !enabled())
        .h(px(34.0))
        .px_4()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(fill)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .child(text(label).text_sm().font_weight(FontWeight::MEDIUM).text_color(ink))
}

/// A row of the sidebar: a mark, a label, and the selection's fill and bar.
pub(crate) fn sidebar_row(
    glyph: &'static str,
    label: &'static str,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label(label)
        .h(px(34.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_3()
        .rounded(move || theme().radius)
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().sidebar_ring))
        .style_when(selected, |s| s.bg(move || theme().sidebar_accent))
        .child(
            view()
                .absolute()
                .left_0()
                .top(px(9.0))
                .w(px(3.0))
                .h(px(16.0))
                .rounded(px(1.5))
                .bg(move || theme().sidebar_primary)
                .opacity(move || if selected() { 1.0_f32 } else { 0.0 }),
        )
        .child(
            text(glyph)
                .w(px(18.0))
                .text_color(move || if selected() { theme().sidebar_primary } else { theme().muted_foreground }),
        )
        .child(
            text(label)
                .flex_1()
                .number_of_lines(1)
                .text_sm()
                .style_when(selected, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || theme().sidebar_foreground),
        )
}

/// A set of options of which exactly one is on (a period, Buy / Sell / both). `group` names
/// the set for screen readers: each option reads "group: label".
pub(crate) fn segmented<T: Copy + PartialEq + 'static>(
    group: &'static str,
    options: &'static [T],
    label: fn(T) -> &'static str,
    value: impl Fn() -> T + Copy + 'static,
    on_change: impl Fn(T) + Copy + 'static,
) -> impl IntoElement {
    toggle_group()
        .flex_row()
        .p_1()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().secondary)
        .children(options.iter().map(move |&option| {
            let on = move || value() == option;
            toggle()
                .pressed(on)
                .on_change(move |_: &bool| on_change(option))
                .accessibility_label(format!("{group}: {}", label(option)))
                .h(px(26.0))
                .px_3()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(|s| s.opacity(0.85))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .style_when(on, |s| s.bg(move || theme().card))
                .child(
                    text(label(option))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .style_when(on, |s| s.font_weight(FontWeight::SEMIBOLD))
                        .text_color(move || if on() { theme().foreground } else { theme().muted_foreground }),
                )
        }))
}

/// One option of a [`select_field`]: what it stores and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    pub(crate) value: SharedString,
    pub(crate) title: SharedString,
}

impl Choice {
    pub(crate) fn new(value: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self { value: value.into(), title: title.into() }
    }
}

/// Whether a drop-down's trigger repeats its label before the chosen title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caption {
    Shown,
    Hidden,
}

/// A labelled drop-down: the label, the chosen title and a chevron open a list of `choices`.
/// `value` is the stored value of the chosen one; a value no choice has shows as the first.
pub(crate) fn select_field(
    label: &'static str,
    choices: impl Fn() -> Vec<Choice> + 'static,
    value: impl Fn() -> SharedString + Copy + 'static,
    on_change: impl Fn(SharedString) + 'static,
) -> impl IntoElement {
    drop_down(Caption::Shown, label, choices, value, on_change)
}

/// A drop-down for a row that already names it (a property list): only the chosen title and
/// a chevron; `label` is what screen readers say.
pub(crate) fn select_value(
    label: &'static str,
    choices: impl Fn() -> Vec<Choice> + 'static,
    value: impl Fn() -> SharedString + Copy + 'static,
    on_change: impl Fn(SharedString) + 'static,
) -> impl IntoElement {
    drop_down(Caption::Hidden, label, choices, value, on_change)
}

fn drop_down(
    caption: Caption,
    label: &'static str,
    choices: impl Fn() -> Vec<Choice> + 'static,
    value: impl Fn() -> SharedString + Copy + 'static,
    on_change: impl Fn(SharedString) + 'static,
) -> impl IntoElement {
    let choices = Rc::new(choices);
    let listed = Rc::clone(&choices);
    let title = move || {
        let wanted = value();
        let all = choices();
        all.iter().find(|c| c.value == wanted).or(all.first()).map_or_default(|c| c.title.clone())
    };
    select()
        .items(move || listed().into_iter().map(|c| SelectItem::new(c.value, c.title)).collect::<Vec<_>>())
        .value(move || Some(value()))
        .on_change(move |next: &Option<SharedString>| {
            if let Some(next) = next {
                on_change(next.clone());
            }
        })
        .accessibility_label(label)
        .offset(px(4.0))
        .popup_style(|s| {
            s.p_1()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().popover)
                .text_color(move || theme().popover_foreground)
                .shadow_md()
        })
        .item_class("rounded-md focus:bg-accent focus:text-accent-foreground selected:font-medium disabled:opacity-50")
        .render_item(|item: &SelectEntry| {
            view().flex_row().items_center().px_2().h(px(30.0)).child(text(item.title.clone()).text_sm())
        })
        .trigger(
            view()
                .h(px(34.0))
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().card)
                .hover(|s| s.bg(move || theme().accent))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .when(caption == Caption::Shown, |v| {
                    v.child(text(label).text_sm().text_color(move || theme().muted_foreground))
                })
                .child(text(title).text_sm().font_weight(FontWeight::MEDIUM).number_of_lines(1))
                .child(text("▾").text_xs().text_color(move || theme().muted_foreground)),
        )
}

/// A one-line text field, with a mark before the text when `glyph` is given.
pub(crate) fn text_field(
    label: &'static str,
    glyph: Option<&'static str>,
    placeholder: &'static str,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(34.0))
        .px_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().card)
        .when_some(glyph, |v, glyph| v.child(text(glyph).text_color(move || theme().muted_foreground)))
        .child(
            input()
                .flex_1()
                .label(label)
                .text_sm()
                .text_color(move || theme().foreground)
                .placeholder(placeholder)
                .placeholder_color(move || color(theme().muted_foreground))
                .caret_color(move || color(theme().primary))
                .value(value)
                .on_change(move |text: &str| on_change(text)),
        )
}

/// A box for several lines of text, `lines` tall.
pub(crate) fn textarea(
    label: &'static str,
    placeholder: &'static str,
    lines: u32,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    const LINE_HEIGHT: f32 = 20.0;
    view()
        .p_3()
        .h(px(LINE_HEIGHT * f32::from(u16::try_from(lines).unwrap_or(u16::MAX)) + 24.0))
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().background)
        .child(
            input()
                .multiline(true)
                .flex_1()
                .label(label)
                .text_sm()
                .text_color(move || theme().foreground)
                .placeholder(placeholder)
                .placeholder_color(move || color(theme().muted_foreground))
                .caret_color(move || color(theme().primary))
                .value(value)
                .on_change(move |text: &str| on_change(text)),
        )
}

/// A group of toggles to pick several of: each of `options` is `(key, label)`; `selected`
/// holds the keys that are on and `on_toggle` hears `(key, on)`. The toggles are made again
/// only when the options change, never when the selection does.
pub(crate) fn tag_picker(
    group: &'static str,
    options: impl Fn() -> Vec<(String, String)> + 'static,
    selected: impl Fn() -> Vec<String> + Copy + 'static,
    on_toggle: impl Fn(&str, bool) + Copy + 'static,
) -> impl IntoElement {
    view().flex_row().flex_wrap().gap_2().child(dynamic(move || {
        options()
            .into_iter()
            .map(|(key, label)| {
                let is_on = {
                    let key = key.clone();
                    move || selected().contains(&key)
                };
                let toggled = key;
                toggle()
                    .pressed(is_on.clone())
                    .on_change(move |next: &bool| on_toggle(&toggled, *next))
                    .accessibility_label(format!("{group}: {label}"))
                    .h(px(26.0))
                    .px_3()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .hover(|s| s.opacity(0.85))
                    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                    .style_when(is_on.clone(), |s| s.bg(move || theme().accent).border_color(move || theme().primary))
                    .child(
                        text(label).text_xs().font_weight(FontWeight::MEDIUM).text_color(move || {
                            if is_on() { theme().accent_foreground } else { theme().muted_foreground }
                        }),
                    )
            })
            .collect::<Vec<_>>()
    }))
}

/// Five stars to grade by; pressing the grade already given clears it. `0` is not graded.
pub(crate) fn rating(
    label: &'static str,
    value: impl Fn() -> u8 + Copy + 'static,
    on_change: impl Fn(u8) + Copy + 'static,
) -> impl IntoElement {
    view().flex_row().gap_1().children((1_u8..=5).map(move |star| {
        let filled = move || value() >= star;
        button()
            .on_press(move || on_change(if value() == star { 0 } else { star }))
            .accessibility_label(format!("{label}: {star} sao"))
            .size(px(28.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(|s| s.bg(move || theme().accent))
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
            .child(
                text(move || if filled() { "★" } else { "☆" })
                    .text_color(move || if filled() { theme().chart_3 } else { theme().muted_foreground }),
            )
    }))
}
