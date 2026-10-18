//! # oc4d-ui
//!
//! The egui front-end of OpenC4D: a Cinema 4D-style workspace made of
//! independent panels, a dark industrial theme and viewport interaction.
//!
//! ## Architecture rules
//!
//! * Panels are self-contained: one file per panel, each a struct (or function)
//!   with a `show(ui, doc, editor)` entry point. Panels never call each other.
//! * Simple value edits (a DragValue on a position) write to the scene directly.
//!   Structural edits (create, delete, select, reparent, framing) are pushed as
//!   [`EditorCommand`]s and applied once per frame, after layout, by
//!   [`commands::apply_pending`]. This keeps borrow scopes small and gives undo/redo
//!   a single choke point later.
//! * GPU work is delegated to `oc4d-render`; this crate only adapts it to egui's
//!   paint-callback mechanism.

pub mod commands;
pub mod editor;
pub mod icons;
pub mod panels;
pub mod shortcuts;
pub mod theme;
mod widgets;

pub use commands::{CommandQueue, EditorCommand, SelectMode};
pub use editor::{EditMode, EditorState, Selection, Tool};
pub use panels::{AttributesPanel, OutlinerPanel, TimelinePanel, ViewportPanel};
