//! # grid4d-ui
//!
//! The egui front-end of Grid4D: a Cinema 4D-style workspace made of
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
//! * GPU work is delegated to `grid4d-render`, which draws the 3D view into its own
//!   offscreen target; this crate only shows that target as an egui texture.

pub mod commands;
pub mod editor;
pub mod file;
pub mod history;
pub mod icons;
pub mod keys;
mod live_edit;
mod modeling;
pub mod panels;
pub mod recent;
pub mod shortcuts;
pub mod theme;
mod widgets;
pub mod window;

pub use commands::{CommandQueue, EditorCommand, SelectMode};
pub use editor::{
    AnimationSettings, BottomPanel, CurveKey, CurveSelection, EditMode, EditorState, FrameCurves, KeySelection,
    ManagerTab, MaterialDrag, MaterialSelection, ModelingSettings, Selection, SelectionSettings, SnapSettings,
    SnapType, TagRef, Tool, TransformSettings,
};
pub use file::{DocumentFile, FileAction, FileDialogs, NativeDialogs, UnsavedChoice};
pub use history::{History, UndoGroup};
pub use panels::{
    AttributesPanel, CoordinatesPanel, FCurvePanel, MaterialsPanel, OutlinerPanel, TimelinePanel, ViewportPanel,
};
pub use recent::RecentFiles;
