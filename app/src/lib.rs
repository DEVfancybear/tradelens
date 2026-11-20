//! TradeLens's UI, in NSH's Rust mode: `nsh.json` `"root"` names [`App`].
//!
//! - `shell`  the window: sidebar and the page of the current route
//! - `nav`    the routes
//! - `store`  the app's state over `tradelens-core`
//! - `ui`     the app's styled components and tokens; pages use only these
//! - `pages`  one module per screen

mod nav;
mod pages;
mod shell;
mod store;
mod ui;

use nsh_base::theme;
use nsh_core::{element::desktop::window as app_window, prelude::*};

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("tradelens/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.register::<Workspace>();
}

/// The root: the main window, showing [`Workspace`].
#[component]
pub fn App() -> impl IntoElement {
    app_window().id("main").content(workspace)
}

/// The main window's content: theme, store and shell. A component of its own so
/// headless tests mount it without a desktop window.
#[component]
pub fn Workspace() -> impl IntoElement {
    ui::theme::install();
    store::provide();
    view().flex_1().bg(move || theme().background).text_color(move || theme().foreground).child(shell::shell())
}
