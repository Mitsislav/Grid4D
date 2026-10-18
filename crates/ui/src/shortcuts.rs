//! Global keyboard shortcuts (Cinema 4D defaults where they exist).
//!
//! | Key            | Action                     |
//! |----------------|----------------------------|
//! | `E` / `T` / `R`| Move / Scale / Rotate tool |
//! | `9`            | Live Selection tool        |
//! | `H`            | Frame scene                |
//! | `O`            | Frame selected objects     |
//! | `Delete`/`⌫`   | Delete selection           |
//! | `Ctrl+A`       | Select all                 |
//! | `Ctrl+Shift+A` | Deselect all               |
//! | `F6` / `F7` / `F8` | Play backwards / stop / play forwards |
//! | `F` / `G`      | Previous / next frame      |

use egui::Key;
use oc4d_scene::Document;

use crate::{EditorCommand, EditorState, Tool};

/// Handles global shortcuts. Ignored while a text field has keyboard focus.
pub fn handle(ctx: &egui::Context, doc: &mut Document, editor: &mut EditorState) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    ctx.input(|i| {
        let plain = i.modifiers.is_none();
        let time = &mut doc.time;

        if plain {
            for (key, tool) in [
                (Key::E, Tool::Move),
                (Key::T, Tool::Scale),
                (Key::R, Tool::Rotate),
                (Key::Num9, Tool::LiveSelection),
            ] {
                if i.key_pressed(key) {
                    editor.tool = tool;
                }
            }
            if i.key_pressed(Key::H) {
                editor.commands.push(EditorCommand::FrameScene);
            }
            if i.key_pressed(Key::O) {
                editor.commands.push(EditorCommand::FrameSelected);
            }
            if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                editor.commands.push(EditorCommand::DeleteSelected);
            }
            if i.key_pressed(Key::F6) {
                time.play_backward();
            }
            if i.key_pressed(Key::F7) {
                time.stop();
            }
            if i.key_pressed(Key::F8) {
                time.play_forward();
            }
            if i.key_pressed(Key::F) {
                time.stop();
                time.step(-1);
            }
            if i.key_pressed(Key::G) {
                time.stop();
                time.step(1);
            }
        }
        if i.modifiers.command && i.key_pressed(Key::A) {
            editor.commands.push(if i.modifiers.shift {
                EditorCommand::DeselectAll
            } else {
                EditorCommand::SelectAll
            });
        }
    });
}
