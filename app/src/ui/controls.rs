//! Styled controls: buttons, the sidebar's rows, segmented switches, selects, text fields,
//! tag pickers and the star rating. Pages compose these and never style a control themselves.

use std::rc::Rc;

use nsh_base::{
    ButtonElement, SelectEntry, SelectItem, alert_dialog, button, input, select, theme, toggle, toggle_group,
};
use nsh_core::prelude::*;

use super::{
    icons::{Icon, icon},
    motion,
    theme::destructive_foreground,
};

/// The size of the icon in a sidebar row.
const SIDEBAR_ICON: f32 = 18.0;

/// How much a button stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    /// The page's main action.
    Primary,
    /// Other actions.
    Secondary,
    /// Actions in toolbars and rows.
    Ghost,
    /// An action that deletes or throws something away.
    Destructive,
}

/// Whether a button can be pressed, read again whenever it is asked.
type Enabled = Rc<dyn Fn() -> bool>;

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
    let enabled: Enabled = Rc::new(enabled);
    let ink = variant.ink_when(&enabled);
    button_shell(variant, enabled, on_press)
        .h(px(34.0))
        .px_4()
        .child(text(label).text_sm().number_of_lines(1).font_weight(FontWeight::MEDIUM).text_color(ink))
}

/// A link: a word or two in the `primary` colour that go to another page, as the design draws
/// "Xem tất cả" and "Mở lịch". The pointing hand and an underline show on hover; the focus ring on
/// the keyboard.
pub(crate) fn link(label: impl IntoReactive<SharedString>, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .rounded(px(4.0))
        .hover(nsh_core::element::Styled::underline)
        .active(|s| s.opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_xs()
                .number_of_lines(1)
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().primary),
        )
}

/// A button with an icon before its label.
pub(crate) fn action_icon(
    glyph: Icon,
    label: impl IntoReactive<SharedString>,
    variant: Variant,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    action_icon_when(glyph, label, variant, || true, on_press)
}

/// [`action_icon`] that can be switched off while `enabled` is false.
pub(crate) fn action_icon_when(
    glyph: Icon,
    label: impl IntoReactive<SharedString>,
    variant: Variant,
    enabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled: Enabled = Rc::new(enabled);
    let (icon_ink, label_ink) = (variant.ink_when(&enabled), variant.ink_when(&enabled));
    button_shell(variant, enabled, on_press)
        .h(px(34.0))
        .px_3()
        .gap_2()
        .child(icon(glyph, ICON_SIZE).text_color(icon_ink))
        .child(text(label).text_sm().number_of_lines(1).font_weight(FontWeight::MEDIUM).text_color(label_ink))
}

/// A square button that holds only an icon. `accessibility_label` is what screen readers say
/// and what tests press, so it is required.
pub(crate) fn icon_action(
    glyph: Icon,
    accessibility_label: impl Into<SharedString>,
    variant: Variant,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let enabled: Enabled = Rc::new(|| true);
    let ink = variant.ink_when(&enabled);
    button_shell(variant, enabled, on_press)
        .accessibility_label(accessibility_label.into())
        .size(px(34.0))
        .child(icon(glyph, ICON_SIZE).text_color(ink))
}

/// Asks the trader to confirm something that destroys data, with the system's alert: `true`
/// only when `confirm_label` is chosen; Cancel, Escape and a dismissed dialog answer `false`.
/// Await it in a task, never block on it.
///
/// The alert is the platform's own, so its buttons wear the platform's colours and the app's theme
/// cannot change them. An in-app `dialog()` was tried and dropped: on Windows its window layer did
/// not paint, and the question was answered without the trader's input.
pub(crate) async fn confirm(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
) -> bool {
    let answer = alert_dialog()
        .title(title)
        .description(description)
        .cancel("cancel", "Huỷ")
        .destructive("confirm", confirm_label)
        .show()
        .await;
    matches!(answer, Ok(result) if result.action.as_deref() == Some("confirm"))
}

/// Asks as [`confirm`] does and runs `then` when the trader confirms, as a task of `owner`, not of
/// the element that asked.
///
/// A task started in a press handler belongs to the owner of the button: when the part of the page the
/// button is in is built again (the cloud card is, at every change of the sign-in state), the task is
/// cancelled and the question never appears. `owner` is one that lives as long as the page (the
/// root's, [`root_owner`]); call this from the handler, take `owner` while building. `then` runs
/// only when the trader chose `confirm_label`.
pub(crate) fn confirm_then(
    owner: Owner,
    title: impl Into<SharedString> + 'static,
    description: impl Into<SharedString> + 'static,
    confirm_label: impl Into<SharedString> + 'static,
    then: impl FnOnce() + 'static,
) {
    owner.run(move || {
        spawn(async move {
            if confirm(title, description, confirm_label).await {
                then();
            }
        });
    });
}

/// The owner of the whole app, which no rebuilt part of a page disposes; the one a question that must
/// outlive its button is asked under ([`confirm_then`]). Call while building.
pub(crate) fn root_owner() -> Owner {
    use_context::<RootOwner>().map_or_else(Owner::current, |root| root.0)
}

/// The app's root owner, provided once by the root ([`provide_root_owner`]).
#[derive(Clone, Copy)]
struct RootOwner(Owner);

/// Makes the owner the root has the one [`root_owner`] gives. Call once, in the root.
pub(crate) fn provide_root_owner() {
    provide_context(RootOwner(Owner::current()));
}

/// The size of an icon inside a button.
const ICON_SIZE: f32 = 16.0;

impl Variant {
    /// The colour of the content on an enabled button of this variant.
    fn ink(self) -> AdaptiveColor {
        match self {
            Self::Primary => theme().primary_foreground,
            Self::Secondary => theme().secondary_foreground,
            Self::Ghost => theme().foreground,
            Self::Destructive => destructive_foreground(),
        }
    }

    /// The colour of the content: [`Variant::ink`], or the muted ink of a button that is switched off.
    fn ink_when(self, enabled: &Enabled) -> impl Fn() -> AdaptiveColor + Clone + 'static {
        let enabled = Rc::clone(enabled);
        move || if enabled() { self.ink() } else { theme().muted_foreground }
    }

    fn fill(self) -> AdaptiveColor {
        match self {
            Self::Primary => theme().primary,
            Self::Secondary => theme().secondary,
            Self::Ghost => AdaptiveColor::TRANSPARENT,
            Self::Destructive => theme().destructive,
        }
    }
}

/// How much of its colour a button that is switched off keeps.
const DISABLED_OPACITY: f32 = 0.6;

/// What every button shares: fill, shape, states; its size and content are the caller's.
///
/// A button that is switched off looks the same in every variant: the muted fill (none for a
/// ghost button) with the muted ink of [`Variant::ink_when`], dimmed, with no hover and no pressed
/// state; a variant's own fill would otherwise hide that it cannot be pressed.
fn button_shell(variant: Variant, enabled: Enabled, on_press: impl Fn() + 'static) -> ButtonElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .disabled(move || !enabled())
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        // Read as the theme changes, not once here: a read here would tie a `dynamic` holding the
        // button to the theme, and make it again (and drop the button half-drawn) when it changes.
        .bg(move || variant.fill())
        // A button with no fill of its own shows the hover as a fill.
        .hover(move |s| match variant {
            Variant::Ghost => s.bg(move || theme().accent),
            Variant::Primary | Variant::Secondary | Variant::Destructive => s.opacity(0.9),
        })
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(move |s| {
            let s = s.opacity(DISABLED_OPACITY).cursor(nsh_core::Cursor::Default);
            match variant {
                Variant::Ghost => s.bg(AdaptiveColor::TRANSPARENT),
                Variant::Primary | Variant::Secondary | Variant::Destructive => s.bg(move || theme().muted),
            }
        })
}

/// A row of the sidebar: an icon, a label, and the selection's fill and bar.
pub(crate) fn sidebar_row(
    glyph: Icon,
    label: &'static str,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .cursor_pointer()
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
            icon(glyph, SIDEBAR_ICON)
                .text_color(move || if selected() { theme().sidebar_primary } else { theme().muted_foreground }),
        )
        .child(
            text(label)
                .number_of_lines(1)
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
                .cursor_pointer()
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
    drop_down(Caption::Shown, label.into(), choices, value, on_change)
}

/// A drop-down for a row that already names it (a property list): only the chosen title and
/// a chevron; `label` is what screen readers say.
pub(crate) fn select_value(
    label: impl Into<SharedString>,
    choices: impl Fn() -> Vec<Choice> + 'static,
    value: impl Fn() -> SharedString + Copy + 'static,
    on_change: impl Fn(SharedString) + 'static,
) -> impl IntoElement {
    drop_down(Caption::Hidden, label.into(), choices, value, on_change)
}

fn drop_down(
    caption: Caption,
    label: SharedString,
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
        .accessibility_label(label.clone())
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
                .cursor_pointer()
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
    label: impl Into<SharedString>,
    glyph: Option<&'static str>,
    placeholder: &'static str,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    framed(glyph, text_input(label.into(), placeholder, value).on_change(move |text: &str| on_change(text)))
}

/// A one-line field for a name or a short value that is kept when the trader presses Enter or
/// leaves the field, not on every key: what is typed stays a draft until then.
///
/// The draft follows `value` while the trader has not written over it. `value` often reads a
/// [`Reading`](crate::store::Reading) or the snapshot, which announce themselves at every read of the
/// database whether or not what they hold changed, so it is made a memo: only a new text reaches the
/// draft, and then only if the draft still is the text `value` last said. What is being typed is
/// never replaced, by a read that brings nothing new or by one that brings another text.
pub(crate) fn commit_field(
    label: impl Into<SharedString>,
    placeholder: &'static str,
    value: impl Fn() -> String + 'static,
    on_commit: impl Fn(String) + 'static,
) -> impl IntoElement {
    let value = Rc::new(value);
    let draft = signal(String::new());
    // What `value` said last: the draft is the trader's own once it differs from it.
    let said = stored(None::<String>);
    let followed = {
        let value = Rc::clone(&value);
        computed(move || value())
    };
    effect(move || {
        let next = followed.get();
        untracked(|| {
            let before = said.replace(Some(next.clone()));
            if before.is_none_or(|before| before == draft.get_untracked()) {
                draft.set(next);
            }
        });
    });
    let keep: Rc<dyn Fn()> = Rc::new(move || {
        let typed = draft.get_untracked();
        if typed.trim() != value().trim() {
            on_commit(typed);
        }
    });
    let on_enter = Rc::clone(&keep);
    framed(
        None,
        text_input(label.into(), placeholder, move || draft.get())
            .on_change(move |typed: &str| draft.set(typed.to_owned()))
            .on_submit(move |_: &str| on_enter())
            .on_blur(move || keep()),
    )
}

/// A one-line field that adds what is typed: Enter, or leaving the field, hands the text to `on_add`
/// and empties the field, ready for the next one; Enter keeps the focus in it. (A [`commit_field`] over a
/// value that is always empty would keep the text it added, and add it again when the field is left.)
pub(crate) fn add_field(
    label: impl Into<SharedString>,
    placeholder: &'static str,
    on_add: impl Fn(String) + 'static,
) -> impl IntoElement {
    let draft = signal(String::new());
    let add: Rc<dyn Fn()> = Rc::new(move || {
        let typed = draft.get_untracked();
        if !typed.trim().is_empty() {
            draft.set(String::new());
            on_add(typed);
        }
    });
    let on_enter = Rc::clone(&add);
    framed(
        None,
        text_input(label.into(), placeholder, move || draft.get())
            .on_change(move |typed: &str| draft.set(typed.to_owned()))
            .on_submit(move |_: &str| on_enter())
            // Enter keeps the focus (the default unfocuses): the next one is typed straight away.
            .submit_behavior(SubmitBehavior::Submit)
            .on_blur(move || add()),
    )
}

/// The input of a text field, before its frame.
fn text_input(label: SharedString, placeholder: &'static str, value: impl Fn() -> String + 'static) -> Input {
    input()
        .flex_1()
        .label(label)
        .text_sm()
        .text_color(move || theme().foreground)
        .placeholder(placeholder)
        .placeholder_color(move || color(theme().muted_foreground))
        .caret_color(move || color(theme().primary))
        .value(value)
}

/// The frame of a one-line field: its border, fill and the mark before the text.
fn framed(glyph: Option<&'static str>, field: Input) -> impl IntoElement {
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
        .child(field)
}

/// A box for several lines of text, `lines` tall.
pub(crate) fn textarea(
    label: impl Into<SharedString>,
    placeholder: &'static str,
    lines: u32,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    const LINE_HEIGHT: f32 = 20.0;
    let label: SharedString = label.into();
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
    group: impl Into<SharedString>,
    options: impl Fn() -> Vec<(String, String)> + 'static,
    selected: impl Fn() -> Vec<String> + Copy + 'static,
    on_toggle: impl Fn(&str, bool) + Copy + 'static,
) -> impl IntoElement {
    let group: SharedString = group.into();
    // `options` often reads the snapshot or a `Reading`, which announce themselves at every read of
    // the database: the toggles follow the options, not the announcements (a toggle made again loses
    // the keyboard focus it has).
    let options = computed(options);
    view().flex_row().flex_wrap().gap_2().child(dynamic(move || {
        let options = options.get();
        untracked(|| {
            options
                .into_iter()
                .map(|(key, label)| {
                    let is_on = {
                        let key = key.clone();
                        move || selected().contains(&key)
                    };
                    let toggled = key;
                    toggle()
                        .cursor_pointer()
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
                        .style_when(is_on.clone(), |s| {
                            s.bg(move || theme().accent).border_color(move || theme().primary)
                        })
                        .child(text(label).text_xs().font_weight(FontWeight::MEDIUM).text_color(move || {
                            if is_on() { theme().accent_foreground } else { theme().muted_foreground }
                        }))
                })
                .collect::<Vec<_>>()
        })
    }))
}

/// Five stars to grade by; pressing the grade already given clears it. `0` is not graded.
pub(crate) fn rating(
    label: impl Into<SharedString>,
    value: impl Fn() -> u8 + Copy + 'static,
    on_change: impl Fn(u8) + Copy + 'static,
) -> impl IntoElement {
    rating_of(label, 5, value, on_change)
}

/// [`rating`] with `max` stars (a scale field of the trader's own that tops at 3 or 5).
pub(crate) fn rating_of(
    label: impl Into<SharedString>,
    max: u8,
    value: impl Fn() -> u8 + Copy + 'static,
    on_change: impl Fn(u8) + Copy + 'static,
) -> impl IntoElement {
    let label: SharedString = label.into();
    view().flex_row().gap_1().children((1_u8..=max).map(move |star| {
        let filled = move || value() >= star;
        button()
            .cursor_pointer()
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::{DialogAnswer, Headless};

    use super::*;

    #[component]
    fn Harness() -> impl IntoElement {
        crate::ui::theme::install();
        let pressed = signal(Vec::<&'static str>::new());
        let note = move |what: &'static str| pressed.update(|all| all.push(what));
        let committed = signal(String::from("Tên cũ"));
        // What a read of the database gives: it announces itself at every read, changed or not.
        let read = signal_with(String::from("Mã 1"), SignalEquals::Never);
        let added = signal(Vec::<String>::new());
        let confirmed = signal(String::from("chưa hỏi"));
        let owner = Owner::current();
        let rebuilt = signal(0_u32);
        view()
            .gap_2()
            .child(icon_action(Icon::Trash, "Xoá dòng", Variant::Ghost, move || note("icon")))
            .child(action_icon(Icon::Plus, "Thêm dòng", Variant::Secondary, move || note("labelled")))
            .child(action_icon_when(Icon::Check, "Lưu", Variant::Primary, || false, move || note("disabled")))
            .child(action_when("Tắt", Variant::Secondary, || false, move || note("sent")))
            .child(action_when("Bật", Variant::Secondary, || true, move || note("resent")))
            .child(link("Xem tất cả", move || note("link")))
            .child(commit_field("Tên", "Tên", move || committed.get(), move |name| committed.set(name)))
            .child(commit_field("Mã", "Mã", move || read.get(), |_| {}))
            .child(action("Đọc lại", Variant::Secondary, move || read.update(|_| {})))
            .child(action("Đọc mã 2", Variant::Secondary, move || read.set("Mã 2".to_owned())))
            .child(add_field("Thêm", "Thêm", move |typed| added.update(|all| all.push(typed))))
            .child(text(move || format!("added {}", added.get().join("|"))))
            .child(action("Hỏi", Variant::Secondary, move || {
                confirmed.set("đã hỏi".to_owned());
                confirm_then(owner, "Xoá?", "Không hoàn tác được.", "Xoá hẳn", move || {
                    confirmed.set("đã xác nhận".to_owned());
                });
            }))
            // A button made again every time `rebuilt` changes, and which asks as it goes: the
            // question outlives it, as the cloud card's does when the sign-in state changes.
            .child(dynamic(move || {
                let _ = rebuilt.get();
                action("Hỏi rồi dựng lại", Variant::Secondary, move || {
                    confirm_then(owner, "Thay?", "Dữ liệu sẽ bị thay.", "Thay hẳn", move || {
                        confirmed.set("đã thay".to_owned());
                    });
                    rebuilt.update(|n| *n += 1);
                })
            }))
            .child(text(move || format!("pressed {}", pressed.get().join(","))))
            .child(text(move || format!("name {}", committed.get())))
            .child(text(move || confirmed.get()))
    }

    fn start() -> Headless {
        let mut config = EngineConfig::new(Root::of::<Harness>());
        config.environment.os = nsh_core::platform::Os::Linux;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("pressed ").expect("the harness shows");
        app
    }

    #[test]
    fn an_icon_only_button_is_found_by_its_accessibility_label() {
        let mut app = start();
        app.press("label:Xoá dòng").expect("the label names the button");
        app.press("Thêm dòng").expect("the labelled button");
        app.wait_for("pressed icon,labelled").expect("both presses arrived");
    }

    #[test]
    fn a_disabled_button_takes_no_press() {
        let mut app = start();
        assert!(app.press("Lưu").is_err() || !app.screen().shows("pressed disabled"));
        assert!(!app.screen().shows("pressed disabled"));
    }

    #[test]
    fn a_commit_field_keeps_its_draft_until_enter() {
        let mut app = start();
        app.type_text("label:Tên", " mới").expect("types");
        assert!(app.screen().shows("name Tên cũ"), "still the old name while typing");
        app.submit("label:Tên").expect("Enter");
        app.wait_for("name Tên cũ mới").expect("committed on Enter");
    }

    #[test]
    fn a_commit_field_follows_what_is_read_until_the_trader_writes_over_it() {
        let mut app = start();
        let held = |app: &Headless| app.screen().input_text("label:Mã").expect("the field").map(str::to_owned);
        assert_eq!(held(&app).as_deref(), Some("Mã 1"));
        // A read that brings nothing new changes nothing, and neither it nor the press of its button
        // (which takes the focus) puts back what the trader is writing.
        app.type_text("label:Mã", "a").expect("types");
        app.press("Đọc lại").expect("reads again");
        assert_eq!(held(&app).as_deref(), Some("Mã 1a"), "a read with nothing new leaves the draft");
        // A read that brings another text leaves the draft too: it is the trader's now.
        app.press("Đọc mã 2").expect("reads another text");
        assert_eq!(held(&app).as_deref(), Some("Mã 1a"), "another text does not replace what is being written");
    }

    #[test]
    fn a_commit_field_that_was_not_written_in_shows_the_new_text() {
        let mut app = start();
        app.press("Đọc mã 2").expect("reads another text");
        app.wait_until("the new text in the field", Duration::from_secs(5), |screen| {
            screen.input_text("label:Mã") == Ok(Some("Mã 2"))
        })
        .expect("a field nobody wrote in follows what is read");
    }

    #[test]
    fn an_add_field_hands_over_its_text_once_and_empties() {
        let mut app = start();
        app.type_text("label:Thêm", "một").expect("types");
        app.submit("label:Thêm").expect("Enter adds it");
        app.wait_for("added một").expect("it was handed over");
        assert_eq!(app.screen().input_text("label:Thêm"), Ok(Some("")), "and the field is empty again");
        // Enter keeps the focus: the next keys go to the field, with no click in between.
        app.press_key("h").expect("typed on the focused field");
        app.press_key("a").expect("typed on the focused field");
        assert_eq!(app.screen().input_text("label:Thêm"), Ok(Some("ha")), "the field still has the focus");
        app.submit("label:Thêm").expect("Enter adds the next");
        app.wait_for("added một|ha").expect("both were handed over, in order");
        // Leaving the field adds nothing a second time.
        app.press("Đọc lại").expect("takes the focus away");
        app.wait(Duration::from_millis(200)).expect("the app runs");
        assert!(app.screen().shows("added một|ha"), "each added once: {:?}", app.screen().texts());
    }

    #[test]
    fn a_button_that_is_switched_off_has_the_muted_ink_whatever_its_variant() {
        let app = start();
        let ink =
            |label: &str| app.screen().span_style(&format!("text:{label}")).expect("the label is on screen").color;
        assert!(is_token("muted-foreground", ink("Tắt")), "a switched-off secondary button");
        assert!(is_token("muted-foreground", ink("Lưu")), "a switched-off primary button");
        assert!(is_token("secondary-foreground", ink("Bật")), "the same button, on");
        assert!(!app.screen().enabled("role:button[Tắt]").expect("the button"));
        assert!(app.screen().enabled("role:button[Bật]").expect("the button"));
    }

    #[test]
    fn a_link_is_a_primary_coloured_press_target() {
        let mut app = start();
        let style = app.screen().span_style("text:Xem tất cả").expect("the link's text");
        assert!(is_token("primary", style.color), "the link is teal");
        app.press("Xem tất cả").expect("the link");
        app.wait_for("pressed link").expect("the press arrived");
    }

    /// Whether `got` is the token `name` of `theme.json`, in the light or the dark appearance.
    fn is_token(name: &str, got: nsh_core::Color) -> bool {
        let tokens = nsh_base::tokens_of(Some(include_str!("../../theme.json"))).expect("theme.json parses");
        let token = tokens.color(name).unwrap_or_else(|| panic!("theme.json has `{name}`"));
        got == nsh_core::Color::from(token.light) || got == nsh_core::Color::from(token.dark)
    }

    /// The question is the system's alert: tests answer it through the headless dialog API, never
    /// by pressing a view.
    #[test]
    fn a_confirmation_runs_what_it_guards_only_for_its_destructive_button() {
        let mut app = start();
        app.on_dialog(|_| DialogAnswer::Press("Huỷ".to_owned()));
        app.press("Hỏi").expect("asks");
        app.wait_for("đã hỏi").expect("the handler ran");
        app.wait(Duration::from_millis(400)).expect("the app runs");
        assert!(!app.screen().shows("đã xác nhận"), "cancelling confirms nothing");

        app.on_dialog(|dialog| {
            assert_eq!(dialog.title.as_deref(), Some("Xoá?"));
            DialogAnswer::Press("Xoá hẳn".to_owned())
        });
        app.press("Hỏi").expect("asks again");
        app.wait_for("đã xác nhận").expect("the guarded action ran");
    }

    #[test]
    fn a_confirmation_nobody_answers_runs_nothing() {
        let mut app = start();
        app.answer_dialogs_myself();
        app.press("Hỏi").expect("asks");
        app.wait(Duration::from_millis(800)).expect("the app runs");
        assert!(app.pending_dialog().is_some(), "the question waits for an answer");
        assert!(!app.screen().shows("đã xác nhận"), "an unanswered question confirms nothing");
    }

    #[test]
    fn a_question_outlives_the_button_that_asked_it() {
        let mut app = start();
        app.answer_dialogs_myself();
        app.press("Hỏi rồi dựng lại").expect("asks, and the button is made again");
        app.wait(Duration::from_millis(400)).expect("the app runs");
        assert!(app.pending_dialog().is_some(), "the question is open though its asker was rebuilt");
        app.answer_dialog(DialogAnswer::Press("Thay hẳn".to_owned())).expect("answers");
        app.wait_for("đã thay").expect("the answer acts, though the button that asked is gone");
    }

    #[test]
    fn cancelling_a_question_whose_asker_was_rebuilt_runs_nothing() {
        let mut app = start();
        app.answer_dialogs_myself();
        app.press("Hỏi rồi dựng lại").expect("asks, and the button is made again");
        app.wait(Duration::from_millis(400)).expect("the app runs");
        assert!(app.pending_dialog().is_some(), "the question is open");
        app.answer_dialog(DialogAnswer::Press("Huỷ".to_owned())).expect("answers");
        app.wait(Duration::from_millis(400)).expect("the app runs");
        assert!(!app.screen().shows("đã thay"), "cancelling acts on nothing");
    }

    /// How many times [`ThemedHolder`]'s `dynamic` has made its button.
    static HOLDER_BUILDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// A button inside a `dynamic`, with the theme set after the first frame starts, as every test
    /// harness (and a theme switch) does.
    #[component]
    fn ThemedHolder() -> impl IntoElement {
        crate::ui::theme::install();
        let shown = signal(0_u32);
        view()
            .child(dynamic(move || {
                let _ = shown.get();
                HOLDER_BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                action("Trong dynamic", Variant::Secondary, || {})
            }))
            .child(text("ready"))
    }

    #[test]
    fn a_button_does_not_tie_the_dynamic_holding_it_to_the_theme() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<ThemedHolder>())).expect("the engine starts");
        app.wait_for("Trong dynamic").expect("the button shows");
        app.wait(Duration::from_millis(300)).expect("the app runs");
        // Made twice, the first button was dropped before it was drawn ("a prop read or written
        // after its owner was disposed" on the engine thread).
        assert_eq!(HOLDER_BUILDS.load(std::sync::atomic::Ordering::SeqCst), 1, "the button is made once");
    }

    /// How many times [`ErrorHolder`]'s `dynamic` has made its error line.
    static ERROR_LINE_BUILDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// The error line pages draw under a form: a `dynamic` that shows a failure in the `loss`
    /// colour, with the theme set in the same component.
    #[component]
    fn ErrorHolder() -> impl IntoElement {
        crate::ui::theme::install();
        let failure = signal(Some(String::from("Không lưu được")));
        view()
            .child(dynamic(move || {
                ERROR_LINE_BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                failure.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss))
            }))
            .child(text("ready"))
    }

    #[test]
    fn an_error_line_does_not_tie_the_dynamic_holding_it_to_the_theme() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<ErrorHolder>())).expect("the engine starts");
        app.wait_for("Không lưu được").expect("the error shows");
        app.wait(Duration::from_millis(300)).expect("the app runs");
        assert_eq!(ERROR_LINE_BUILDS.load(std::sync::atomic::Ordering::SeqCst), 1, "the error line is made once");
    }
}
