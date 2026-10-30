//! "About & shortcuts" modal.

use egui::{Grid, Id, Modal, RichText, ScrollArea};

use crate::i18n::{key, tr, trf};
use crate::{EditorState, theme::palette as p};

const SHORTCUTS: [(&str, &str); 43] = [
    ("Alt + Left drag", key("Orbit")),
    ("Alt + Middle drag / Middle drag", key("Pan")),
    ("Alt + Right drag / Wheel", key("Zoom")),
    ("H", key("Frame scene")),
    ("O", key("Frame selected objects")),
    ("E / T / R", key("Move / Scale / Rotate tool")),
    ("9 / 0", key("Live / Rectangle Selection tool")),
    ("U~L / U~B", key("Loop / Ring Selection tool (U, then L)")),
    ("C", key("Make editable (primitive → polygon object)")),
    ("Q", key("Enable / disable the (parent) generator")),
    (
        "Shift+S",
        key("Snapping on / off (right-click the snap button: settings)"),
    ),
    ("D / I", key("Extrude / Inset tool (drag left or right)")),
    ("K~L / K~K", key("Loop/Path Cut / Line Cut tool")),
    ("M~M / M~N / U~Z", key("Connect / Dissolve / Melt")),
    ("M~S / B (M~B)", key("Bevel / Bridge tool")),
    ("U~O", key("Optimize…")),
    ("Click / Shift / Ctrl", key("Select / add / toggle")),
    ("Delete / Backspace", key("Delete selection (objects or components)")),
    ("Ctrl+Backspace / Alt+Backspace", key("Dissolve / Melt")),
    ("Ctrl+A / Ctrl+Shift+A", key("Select all / deselect all")),
    ("Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z)", key("Undo / redo")),
    ("Ctrl+N / Ctrl+O", key("New / open document (also glTF, OBJ)")),
    ("Ctrl+Shift+O", key("Merge a .g4d, glTF or OBJ file into the document")),
    ("Ctrl+S / Ctrl+Shift+S", key("Save / save as")),
    ("F8 / F6 / F7", key("Play / play backwards / stop")),
    ("F / G", key("Previous / next frame")),
    ("F1–F4 / F5", key("View 1–4 alone / all four views")),
    (
        "N~A … N~G",
        key("Gouraud, Gouraud (Lines), Quick, Quick (Lines), Constant, Hidden Line, Lines"),
    ),
    ("N~H / N~K", key("Wireframe / Box")),
    ("Middle click on a view", key("Show it alone / back in the arrangement")),
    ("Ctrl+F / Ctrl+G", key("Previous / next key")),
    ("F9", key("Record Active Objects")),
    ("Ctrl+F9", key("Autokeying on / off")),
    ("Tab (over the timeline)", key("Timeline / F-Curve tab")),
    ("H / S (over the F-Curve graph)", key("Frame all / selected keys")),
    ("Shift+F5 / Shift+F7", key("Attribute / Coordinate Manager")),
    ("Shift+F2", key("Material Manager")),
    ("Ctrl+N (over the Material Manager)", key("New material")),
    ("Ctrl+B", key("Render Settings")),
    ("Shift+R", key("Render to Picture Viewer")),
    ("Ctrl+R / Esc", key("Render View / clear it")),
    ("Double-click name", key("Rename (Object Manager)")),
    ("Drag row onto row", key("Re-parent (Object Manager)")),
];

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    if !editor.show_about {
        return;
    }
    let response = Modal::new(Id::unique("grid4d.about")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(tr("Grid4D")).size(20.0).strong().color(p().text_strong));
        ui.label(
            RichText::new(trf(
                "Version {0} · open-source 3D content creation suite",
                &[&env!("CARGO_PKG_VERSION")],
            ))
            .color(p().text_dim),
        );
        ui.label(
            RichText::new(tr(
                "Interface in English and Greek (Preferences ▸ Interface ▸ Language)",
            ))
            .color(p().text_dim),
        );
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Keyboard & mouse")).strong());
        ui.add_space(4.0);
        // The list scrolls, so the dialog (and its Close button) fits small windows.
        let room = (ctx.content_rect().height() - 260.0).max(120.0);
        ScrollArea::vertical().max_height(room).show(ui, |ui| {
            Grid::new("grid4d.shortcuts")
                .num_columns(2)
                .spacing([24.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for (keys, action) in SHORTCUTS {
                        ui.label(RichText::new(keys).monospace().color(p().text_strong));
                        ui.label(tr(action));
                        ui.end_row();
                    }
                });
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(tr(
                "Grid4D is an independent project inspired by the Cinema 4D workflow. \
                 It is not affiliated with or endorsed by Maxon. Cinema 4D is a trademark of Maxon Computer GmbH.",
            ))
            .size(11.0)
            .color(p().text_faint),
        );
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.button(tr("Close")).clicked()).inner
    });
    if response.inner || response.should_close() {
        editor.show_about = false;
    }
}
