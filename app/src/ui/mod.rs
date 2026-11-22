//! The app's look: tokens, motion and styled components. Pages build only from
//! these; `nsh_base` controls are wrapped here once.
//!
//! The modules that are `pub` hold the shared building blocks the screens of v0.2 compose
//! (icons, form parts, galleries, property lists). They are public so that a component no
//! screen uses yet is not dead code; the rest stay `pub(crate)`.

pub(crate) mod breakdown;
pub(crate) mod chart;
pub(crate) mod charts;
pub mod checklist_editor;
pub(crate) mod chips;
pub(crate) mod controls;
pub mod files;
pub mod forms;
pub mod gallery;
pub mod icons;
pub(crate) mod layout;
pub(crate) mod month_grid;
pub(crate) mod motion;
pub(crate) mod parts;
pub(crate) mod plan_vs_actual;
pub mod properties;
pub(crate) mod r_chart;
pub mod reorder;
pub mod rule_row;
pub mod selectable_card;
pub(crate) mod tab_bar;
pub(crate) mod table;
pub(crate) mod theme;
