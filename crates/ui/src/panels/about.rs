//! "About & shortcuts" modal.

use egui::{Grid, Id, Modal, RichText, ScrollArea};

use crate::{EditorState, theme::palette as p};

const SHORTCUTS: [(&str, &str); 28] = [
    ("Alt + Left drag", "Orbit"),
    ("Alt + Middle drag / Middle drag", "Pan"),
    ("Alt + Right drag / Wheel", "Zoom"),
    ("H", "Frame scene"),
    ("O", "Frame selected objects"),
    ("E / T / R", "Move / Scale / Rotate tool"),
    ("9 / 0", "Live / Rectangle Selection tool"),
    ("U~L / U~B", "Loop / Ring Selection tool (U, then L)"),
    ("C", "Make editable (primitive → polygon object)"),
    ("Q", "Enable / disable the (parent) generator"),
    ("Shift+S", "Snapping on / off (right-click the snap button: settings)"),
    ("D / I", "Extrude / Inset tool (drag left or right)"),
    ("K~L / K~K", "Loop/Path Cut / Line Cut tool"),
    ("M~M / M~N / U~Z", "Connect / Dissolve / Melt"),
    ("M~S / B (M~B)", "Bevel / Bridge tool"),
    ("U~O", "Optimize…"),
    ("Click / Shift / Ctrl", "Select / add / toggle"),
    ("Delete / Backspace", "Delete selection (objects or components)"),
    ("Ctrl+Backspace / Alt+Backspace", "Dissolve / Melt"),
    ("Ctrl+A / Ctrl+Shift+A", "Select all / deselect all"),
    ("Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z)", "Undo / redo"),
    ("Ctrl+N / Ctrl+O", "New / open document (also glTF)"),
    ("Ctrl+Shift+O", "Merge a .g4d or glTF file into the document"),
    ("Ctrl+S / Ctrl+Shift+S", "Save / save as"),
    ("F8 / F6 / F7", "Play / play backwards / stop"),
    ("F / G", "Previous / next frame"),
    ("Double-click name", "Rename (Object Manager)"),
    ("Drag row onto row", "Re-parent (Object Manager)"),
];

pub fn show(ctx: &egui::Context, editor: &mut EditorState) {
    if !editor.show_about {
        return;
    }
    let response = Modal::new(Id::unique("grid4d.about")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Grid4D").size(20.0).strong().color(p::TEXT_STRONG));
        ui.label(
            RichText::new(concat!(
                "Version ",
                env!("CARGO_PKG_VERSION"),
                " · open-source 3D content creation suite"
            ))
            .color(p::TEXT_DIM),
        );
        ui.add_space(10.0);
        ui.label(RichText::new("Keyboard & mouse").strong());
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
                        ui.label(RichText::new(keys).monospace().color(p::TEXT_STRONG));
                        ui.label(action);
                        ui.end_row();
                    }
                });
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "Grid4D is an independent project inspired by the Cinema 4D workflow. \
                 It is not affiliated with or endorsed by Maxon. Cinema 4D is a trademark of Maxon Computer GmbH.",
            )
            .size(11.0)
            .color(p::TEXT_FAINT),
        );
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.button("Close").clicked()).inner
    });
    if response.inner || response.should_close() {
        editor.show_about = false;
    }
}
