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

pub mod autosave;
pub mod commands;
pub mod crash;
mod current_state;
pub mod editor;
pub mod file;
pub mod history;
pub mod i18n;
pub mod icons;
pub mod keys;
mod live_edit;
mod modeling;
pub mod panels;
pub mod preferences;
#[cfg(test)]
mod random_commands;
pub mod recent;
pub mod rendering;
pub mod scripts;
pub mod shell;
pub mod shortcuts;
pub mod simulation;
pub mod startup;
pub mod textures;
pub mod theme;
mod widgets;
pub mod window;
pub mod workspace;

pub use commands::{CommandQueue, EditorCommand, SelectMode, SplineOp};
pub use editor::{
    AnimationSettings, BuildInfo, CurveKey, CurveSelection, EditMode, EditorState, FrameCurves, KeySelection,
    MaterialDrag, MaterialSelection, ModelingSettings, Selection, SelectionSettings, SnapSettings, SnapType, TagRef,
    Tool, TransformSettings,
};
pub use file::{DocumentFile, FileAction, FileDialogs, NativeDialogs, UnsavedChoice};
pub use history::{History, UndoGroup};
pub use panels::{
    AttributesPanel, CoordinatesPanel, FCurvePanel, MaterialsPanel, OutlinerPanel, PictureViewerPanel,
    RenderSettingsPanel, TimelinePanel, ViewportPanel,
};
pub use recent::RecentFiles;

/// The test plugins (the Gear), installed once for this test binary.
#[cfg(test)]
pub(crate) fn test_plugins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut registry = grid4d_plugin::Registry::default();
        registry.add_generator(grid4d_plugin_gear::Gear);
        registry.add_deformer(grid4d_plugin_wave::Wave);
        registry.add_command(grid4d_plugin_scatter::Scatter);
        grid4d_plugin::install(registry);
    });
}
