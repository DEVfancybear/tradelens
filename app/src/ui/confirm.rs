//! The question the app asks before it destroys data, in the app's own look.
//!
//! The system's alert cannot take the theme (its buttons come out in the platform's blue), so the
//! question is a [`dialog`] of the window: a title, what will happen, a secondary "Huỷ" and the
//! destructive action. Escape cancels and Enter confirms (a button that has the keyboard focus
//! takes the key for itself, so Tab to "Huỷ" and Enter cancels); focus stays inside the dialog
//! while it is open, as the window layer of a dialog keeps it.
//!
//! [`confirm_then`] asks; [`host`] is mounted once over the whole window and shows the questions
//! one at a time.

use std::{cell::RefCell, rc::Rc};

use nsh_base::{dialog, theme};
use nsh_core::prelude::*;

use super::controls::{Variant, action};

/// How opaque the veil behind the question is.
const SCRIM_OPACITY: f32 = 0.72;

/// The width of the question's panel.
const PANEL_WIDTH: f32 = 420.0;

/// One question and what happens when it is confirmed.
struct Question {
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    /// Taken when the question is confirmed.
    then: RefCell<Option<Box<dyn FnOnce()>>>,
    /// The owner `then` runs under: one that lives as long as the page.
    owner: Owner,
}

/// The questions waiting to be answered, the first of them on screen.
#[derive(Default, Clone)]
struct Asking(Vec<Rc<Question>>);

/// The question on screen; equal to another only when it is the same one.
#[derive(Clone)]
struct Shown(Rc<Question>);

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Asks the trader to confirm something that destroys data and runs `then` when they choose
/// `confirm_label`; "Huỷ", Escape and a press outside the dialog do nothing.
///
/// `then` runs as a task of `owner`, not of the element that asked: a press handler's own owner is
/// the button's, and when the part of the page the button is in is built again (the cloud card is,
/// at every change of the sign-in state) what it owns is cancelled and the question would never be
/// answered. `owner` is one that lives as long as the page (the root's, `root_owner`); call this
/// from the handler, take `owner` while building.
pub(crate) fn confirm_then(
    owner: Owner,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
    then: impl FnOnce() + 'static,
) {
    let question = Rc::new(Question {
        title: title.into(),
        description: description.into(),
        confirm_label: confirm_label.into(),
        then: RefCell::new(Some(Box::new(then))),
        owner,
    });
    global::<Asking>().update(|asking| asking.0.push(question));
}

/// What the trader chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    Confirm,
    Cancel,
}

/// Closes `question` and, when it is confirmed, runs what it was asked for.
fn answer(question: &Rc<Question>, reply: Reply) {
    global::<Asking>().update(|asking| asking.0.retain(|queued| !Rc::ptr_eq(queued, question)));
    if reply == Reply::Cancel {
        return;
    }
    let then = question.then.borrow_mut().take();
    if let Some(then) = then {
        question.owner.run(then);
    }
}

/// The dialog of the question that is waiting, mounted once as a child of the window's root.
pub(crate) fn host() -> impl IntoElement {
    let asking = global::<Asking>();
    let head = computed(move || asking.with(|asking| asking.0.first().cloned().map(Shown)));
    dynamic(move || {
        head.get().map(|Shown(question)| {
            let cancelled = Rc::clone(&question);
            dialog()
                .role(AccessibilityRole::AlertDialog)
                .label(question.title.clone())
                .on_cancel(move || answer(&cancelled, Reply::Cancel))
                .flex_1()
                .child(untracked(|| scrim(&question)))
        })
    })
}

/// The veil over the window with the panel in its middle.
fn scrim(question: &Rc<Question>) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .p_6()
        .bg(move || theme().background.opacity(SCRIM_OPACITY))
        .child(panel(question))
}

fn panel(question: &Rc<Question>) -> impl IntoElement {
    let (cancel, confirm, entered) = (Rc::clone(question), Rc::clone(question), Rc::clone(question));
    // Enter confirms while the question is open. A button that has the keyboard focus (Tab moved it
    // to "Huỷ") takes the key for its own press first, so Enter on "Huỷ" cancels.
    on_key("enter", move |_| answer(&entered, Reply::Confirm));
    view()
        .w(px(PANEL_WIDTH))
        .p_5()
        .gap_4()
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .shadow_lg()
        .child(
            view()
                .gap_2()
                .child(
                    text(question.title.clone())
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(text(question.description.clone()).text_sm().text_color(move || theme().muted_foreground)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(action("Huỷ", Variant::Secondary, move || answer(&cancel, Reply::Cancel)))
                .child(action(question.confirm_label.clone(), Variant::Destructive, move || {
                    answer(&confirm, Reply::Confirm);
                })),
        )
}
