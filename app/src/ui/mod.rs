//! The app's look: tokens, motion and styled components. Pages build only from
//! these; `nsh_base` controls are wrapped here once.
//!
//! The modules that are `pub` hold the shared building blocks the screens of v0.2 compose
//! (icons, form parts, galleries, property lists). They are public so that a component no
//! screen uses yet is not dead code; the rest stay `pub(crate)`.

pub(crate) mod chart;
pub(crate) mod chips;
pub(crate) mod controls;
pub mod files;
pub mod forms;
pub mod gallery;
pub mod icons;
pub(crate) mod motion;
pub(crate) mod parts;
pub mod properties;
pub mod reorder;
pub(crate) mod table;
pub(crate) mod theme;
