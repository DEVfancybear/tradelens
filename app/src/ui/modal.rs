//! The one modal of the window: a panel over a scrim, drawn inside the window itself.
//!
//! `nsh_base::dialog()` presents its children in a native window layer of its own, which does not
//! paint on Windows (the window goes dark and nothing shows), so no screen uses it. A modal here is
//! a layer of the window's root, as the search palette is: [`host`] is mounted once in the shell,
//! [`open`] queues a [`Modal`], and only the first one in line is on screen.
//!
//! What the layer gives a modal:
//! - a scrim (the theme's `background`, mostly opaque) that covers the page, so nothing behind it can
//!   be pressed, and that dismisses the modal when pressed unless the modal [keeps it open](Modal::keeps_open_on_scrim);
//! - a centred panel (`popover`, border, radius, shadow) with a title, a close button and a body that
//!   scrolls when it is taller than the window;
//! - Escape and the close button, which both hand the decision to the modal's `on_dismiss`: the
//!   modal closes itself with [`close`] once it has done what dismissing means (a form that must
//!   write something first closes when the write is done);
//! - the keyboard focus moved onto the panel when it opens, and kept in the layer by two sentinels
//!   at its ends, so that Tab and Shift+Tab do not go on into the page behind.
//!
//! Escape is bound with the layer's own owner: the binding goes with the layer when the modal closes,
//! and nothing is bound while no modal is open.

use std::{cell::Cell, rc::Rc};

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use super::{
    controls::{Variant, icon_action},
    icons::Icon,
};

/// How opaque the scrim is.
const SCRIM_OPACITY: f32 = 0.72;

/// The width of a panel that does not ask for one.
const DEFAULT_WIDTH: f32 = 560.0;

/// The size of the focus sentinels at the ends of the layer.
const SENTINEL: f32 = 1.0;

/// A modal to show: what it says, how it looks and what dismissing it does.
#[derive(Clone)]
pub(crate) struct Modal {
    id: String,
    title: SharedString,
    close_label: SharedString,
    width: f32,
    scrim_dismisses: bool,
    body: Rc<dyn Fn() -> AnyElement>,
    on_dismiss: Rc<dyn Fn()>,
}

impl Modal {
    /// A modal named `id` (two modals with the same id are one), titled `title`. `body` builds
    /// its content each time it is shown; `on_dismiss` is what Escape, the close button and (by
    /// default) a press on the scrim ask for: it decides what happens, and calls [`close`].
    pub(crate) fn new<E: IntoElement>(
        id: impl Into<String>,
        title: impl Into<SharedString>,
        body: impl Fn() -> E + 'static,
        on_dismiss: impl Fn() + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            close_label: SharedString::from("Đóng"),
            width: DEFAULT_WIDTH,
            scrim_dismisses: true,
            body: Rc::new(move || body().into_element()),
            on_dismiss: Rc::new(on_dismiss),
        }
    }

    /// The panel's width in points.
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// What the close button is called to a screen reader (and a test).
    pub(crate) fn close_label(mut self, label: impl Into<SharedString>) -> Self {
        self.close_label = label.into();
        self
    }

    /// A press on the scrim does nothing: for a modal with unsaved edits, which a stray click
    /// outside the panel must not throw away. Escape and the close button still dismiss it.
    pub(crate) fn keeps_open_on_scrim(mut self) -> Self {
        self.scrim_dismisses = false;
        self
    }
}

/// The modals waiting, the first of them on screen.
#[derive(Default, Clone)]
struct Queue(Vec<Modal>);

/// The modal on screen; equal to another when it has the same id, so that a change elsewhere in
/// the queue does not build it again.
#[derive(Clone)]
struct Shown(Modal);

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

/// Puts `modal` in line: it shows at once when none is on screen. One that is already in line
/// (or on screen) under the same id is not added twice.
pub(crate) fn open(modal: Modal) {
    global::<Queue>().update(|queue| {
        if !queue.0.iter().any(|waiting| waiting.id == modal.id) {
            queue.0.push(modal);
        }
    });
}

/// Takes the modal named `id` out of line, from the screen if it is the one shown: the next one
/// shows in its place.
pub(crate) fn close(id: &str) {
    global::<Queue>().update(|queue| queue.0.retain(|waiting| waiting.id != id));
}

/// The layer, mounted once as a child of the window's root, after the screens and before the
/// search palette.
pub(crate) fn host() -> impl IntoElement {
    let queue = global::<Queue>();
    let head = computed(move || queue.with(|queue| queue.0.first().cloned().map(Shown)));
    dynamic(move || head.get().map(|Shown(modal)| untracked(move || layer(&modal))))
}

/// The scrim, the panel and the sentinels of one modal.
fn layer(modal: &Modal) -> impl IntoElement + use<> {
    let dismiss = Rc::clone(&modal.on_dismiss);
    on_key("escape", {
        let dismiss = Rc::clone(&dismiss);
        move |_| dismiss()
    });

    let panel_ref = element_ref();
    let end_ref = element_ref();
    // Set while the focus is moved to the end sentinel on purpose (Shift+Tab from the start), so
    // that the sentinel does not send it straight back.
    let wrapping = Rc::new(Cell::new(false));
    spawn(async move {
        next_frame().await;
        panel_ref.focus();
    });

    let scrim_press = {
        let dismiss = Rc::clone(&dismiss);
        let dismisses = modal.scrim_dismisses;
        move || {
            if dismisses {
                dismiss();
            }
        }
    };
    let sentinel = || view().absolute().size(px(SENTINEL)).focusable(true).accessibility_elements_hidden(true);
    view()
        .absolute()
        .inset_0()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            button()
                .cursor_pointer()
                .on_press(scrim_press)
                .accessibility_label("Đóng hộp thoại")
                .absolute()
                .inset_0()
                .bg(move || theme().background)
                .opacity(SCRIM_OPACITY),
        )
        .child(sentinel().on_focus({
            let wrapping = Rc::clone(&wrapping);
            move || {
                wrapping.set(true);
                end_ref.focus();
            }
        }))
        .child(panel(modal, &dismiss, panel_ref))
        .child(sentinel().element_ref(&end_ref).on_focus(move || {
            if !wrapping.replace(false) {
                panel_ref.focus();
            }
        }))
}

fn panel(modal: &Modal, dismiss: &Rc<dyn Fn()>, panel_ref: ElementRef) -> impl IntoElement + use<> {
    let close = Rc::clone(dismiss);
    view()
        .element_ref(&panel_ref)
        .focusable(true)
        .role(AccessibilityRole::Dialog)
        .label(modal.title.clone())
        .w(px(modal.width))
        .max_w(relative(1.0))
        .max_h(relative(1.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .shadow_lg()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .px_5()
                .pt_4()
                .pb_2()
                .child(
                    text(modal.title.clone())
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(icon_action(Icon::Close, modal.close_label.clone(), Variant::Ghost, move || close())),
        )
        .child(scroll().min_h(px(0.0)).child(view().px_5().pb_5().child((modal.body)())))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;
    use crate::ui::controls::{action, commit_field};

    /// A page with a field, two buttons that open modals, and the layer on top; what the modals
    /// were asked is shown as text.
    #[component]
    fn Harness() -> impl IntoElement {
        crate::ui::theme::install();
        let log = signal(Vec::<&'static str>::new());
        let note = move |what: &'static str| log.update(|all| all.push(what));
        let first = move || {
            open(Modal::new(
                "first",
                "Hộp thứ nhất",
                || {
                    view().gap_2().child(text("nội dung một")).child(commit_field(
                        "Ô trong hộp",
                        "Ô trong hộp",
                        String::new,
                        |_| {},
                    ))
                },
                move || {
                    note("first dismissed");
                    close("first");
                },
            ));
        };
        let second = move || {
            open(
                Modal::new(
                    "second",
                    "Hộp thứ hai",
                    || text("nội dung hai"),
                    move || {
                        note("second asked");
                    },
                )
                .keeps_open_on_scrim(),
            );
        };
        view()
            .flex_1()
            .child(
                view()
                    .gap_2()
                    // First on the page: where Tab goes if it leaves the end of the modal.
                    .child(commit_field("Ô sau hộp", "Ô sau hộp", String::new, |_| {}))
                    .child(action("Mở một", Variant::Secondary, first))
                    .child(action("Mở hai", Variant::Secondary, second))
                    .child(action("Mở cả hai", Variant::Secondary, move || {
                        first();
                        second();
                    }))
                    .child(action("Đóng hai", Variant::Secondary, || close("second")))
                    .child(text(move || format!("log {}", log.get().join(",")))),
            )
            .child(host())
    }

    /// A point of the scrim that the panel does not cover.
    fn corner() -> nsh_core::Point<nsh_core::Points> {
        nsh_core::Point { x: nsh_core::Points(4.0), y: nsh_core::Points(4.0) }
    }

    fn start() -> Headless {
        // A desktop window, as the app has: the panel is wider than a phone.
        let mut config = EngineConfig {
            viewport: nsh_core::Size::new(px(1000.0), px(900.0)),
            ..EngineConfig::new(Root::of::<Harness>())
        };
        config.environment.os = nsh_core::platform::Os::Windows;
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("log ").expect("the harness shows");
        app
    }

    #[test]
    fn a_modal_shows_its_title_and_body_and_the_close_button_dismisses_it() {
        let mut app = start();
        app.press("Mở một").expect("opens");
        app.wait_for("nội dung một").expect("the body shows");
        assert!(app.screen().shows("Hộp thứ nhất"), "the title shows");
        app.press("role:button[Đóng]").expect("the close button");
        app.wait_for("log first dismissed").expect("the close button asked to dismiss");
        app.wait_for_gone("nội dung một", Duration::from_secs(5)).expect("the modal closed itself");
    }

    #[test]
    fn escape_dismisses_the_modal_and_only_while_one_is_open() {
        let mut app = start();
        app.press_key("escape").expect("Escape with nothing open");
        assert!(!app.screen().shows("dismissed"), "nothing was dismissed");
        app.press("Mở một").expect("opens");
        app.wait_for("nội dung một").expect("the body shows");
        app.press_key("escape").expect("Escape");
        app.wait_for("log first dismissed").expect("Escape asked to dismiss");
        app.wait_for_gone("nội dung một", Duration::from_secs(5)).expect("closed");
        // The key is bound to the layer: once it is gone, Escape does nothing again.
        app.press_key("escape").expect("Escape after it closed");
        app.wait(Duration::from_millis(300)).expect("the app runs");
        assert!(app.screen().shows("log first dismissed"), "nothing more was asked");
    }

    #[test]
    fn the_scrim_dismisses_a_modal_unless_it_keeps_its_edits() {
        let mut app = start();
        app.press("Mở một").expect("opens");
        app.wait_for("nội dung một").expect("the body shows");
        app.press_at("label:Đóng hộp thoại", corner()).expect("a press outside the panel");
        app.wait_for("log first dismissed").expect("the scrim dismissed it");
        app.wait_for_gone("nội dung một", Duration::from_secs(5)).expect("closed");

        app.press("Mở hai").expect("opens the one that keeps its edits");
        app.wait_for("nội dung hai").expect("the body shows");
        app.press_at("label:Đóng hộp thoại", corner()).expect("a press outside the panel");
        app.wait(Duration::from_millis(300)).expect("the app runs");
        assert!(app.screen().shows("nội dung hai"), "the scrim left it open");
        assert!(!app.screen().matches("text:/second asked/").expect("selector"), "and asked nothing");
        app.press("role:button[Đóng]").expect("the close button still asks");
        app.wait_for("text:/second asked/").expect("the close button asked to dismiss");
    }

    #[test]
    fn one_modal_is_on_screen_at_a_time_and_the_next_one_follows() {
        let mut app = start();
        app.press("Mở cả hai").expect("opens both");
        app.wait_for("nội dung một").expect("the first shows");
        assert!(!app.screen().shows("nội dung hai"), "the second waits");
        app.press("role:button[Đóng]").expect("dismisses the first");
        app.wait_for("nội dung hai").expect("the second takes its place");
        assert!(!app.screen().shows("nội dung một"), "the first is gone");
    }

    /// The tab stops of the page are not reached from the modal: the focus goes round inside it.
    #[test]
    fn tab_does_not_leave_the_modal() {
        let mut app = start();
        app.press("Mở một").expect("opens");
        app.wait_for("nội dung một").expect("the body shows");
        // Every stop the focus reaches takes a letter if it is a field: the field of the page
        // behind the modal must stay empty, however far Tab and Shift+Tab go.
        for key in ["tab", "tab", "tab", "tab", "tab", "tab", "shift+tab", "shift+tab", "shift+tab", "shift+tab"] {
            app.press_key(key).expect("moves the focus");
            app.press_key("x").expect("types where the focus is");
            eprintln!(
                "DBG {key} in={:?} behind={:?}",
                app.screen().input_text("label:Ô trong hộp"),
                app.screen().input_text("label:Ô sau hộp")
            );
        }
        assert_eq!(app.screen().input_text("label:Ô sau hộp").expect("the page's field"), Some(""));
        assert!(
            app.screen().input_text("label:Ô trong hộp").expect("the modal's field").is_some_and(|t| t.contains('x')),
            "the focus did reach the modal's own field"
        );
    }
}
