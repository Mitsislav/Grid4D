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
//! | `Ctrl+N` / `Ctrl+O` | New / open document   |
//! | `Ctrl+S` / `Ctrl+Shift+S` | Save / save as  |
//! | `Ctrl+Z`       | Undo                       |
//! | `Ctrl+Y` / `Ctrl+Shift+Z` | Redo            |
//! | `F6` / `F7` / `F8` | Play backwards / stop / play forwards |
//! | `F` / `G`      | Previous / next frame      |

use egui::Key;
use grid4d_scene::Document;

use crate::{EditorCommand, EditorState, Tool};

/// Handles global shortcuts. Ignored while a text field has keyboard focus
/// (text fields have their own undo).
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
                (Key::D, Tool::Extrude),
                (Key::I, Tool::Inset),
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
            if i.key_pressed(Key::C) {
                editor.commands.push(EditorCommand::MakeEditable);
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
        // Not in the middle of a drag: the drag records its own step on release,
        // starting from the document it began with.
        if i.modifiers.command && !i.pointer.any_down() {
            if i.key_pressed(Key::N) {
                editor.commands.push(EditorCommand::NewDocument);
            } else if i.key_pressed(Key::O) {
                editor.commands.push(EditorCommand::Open);
            } else if i.key_pressed(Key::S) {
                editor.commands.push(if i.modifiers.shift {
                    EditorCommand::SaveAs
                } else {
                    EditorCommand::Save
                });
            } else if i.key_pressed(Key::Z) {
                editor.commands.push(if i.modifiers.shift {
                    EditorCommand::Redo
                } else {
                    EditorCommand::Undo
                });
            } else if i.key_pressed(Key::Y) {
                editor.commands.push(EditorCommand::Redo);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    use super::*;

    /// Presses `key` with `modifiers` in one headless frame and returns the
    /// queued commands. With `dragging`, the primary mouse button is held.
    fn press(key: Key, modifiers: Modifiers, dragging: bool) -> Vec<EditorCommand> {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::default(), EditorState::default());
        let mut events = vec![Event::ModifiersChanged(modifiers)];
        if dragging {
            let pos = Pos2::new(100.0, 100.0);
            events.push(Event::PointerMoved(pos));
            events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            });
        }
        events.push(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        let input = RawInput {
            events,
            ..RawInput::default()
        };
        ctx.run_ui(input, |ui| handle(ui.ctx(), &mut doc, &mut editor))
            .drop_without_applying_deltas();
        editor.commands.pending().to_vec()
    }

    #[test]
    fn undo_and_redo_follow_c4d() {
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        assert_eq!(press(Key::Z, ctrl, false), [EditorCommand::Undo]);
        assert_eq!(press(Key::Y, ctrl, false), [EditorCommand::Redo]);
        assert_eq!(press(Key::Z, ctrl_shift, false), [EditorCommand::Redo]);
    }

    #[test]
    fn file_shortcuts() {
        let ctrl = Modifiers::COMMAND;
        assert_eq!(press(Key::N, ctrl, false), [EditorCommand::NewDocument]);
        assert_eq!(press(Key::O, ctrl, false), [EditorCommand::Open]);
        assert_eq!(press(Key::S, ctrl, false), [EditorCommand::Save]);
        assert_eq!(press(Key::S, ctrl | Modifiers::SHIFT, false), [EditorCommand::SaveAs]);
        assert!(press(Key::S, Modifiers::NONE, false).is_empty());
    }

    #[test]
    fn c_makes_editable_like_c4d() {
        assert_eq!(press(Key::C, Modifiers::NONE, false), [EditorCommand::MakeEditable]);
        assert!(
            press(Key::C, Modifiers::COMMAND, false).is_empty(),
            "Ctrl+C is not Make Editable"
        );
    }

    #[test]
    fn d_and_i_pick_extrude_and_inset() {
        for (key, tool) in [(Key::D, Tool::Extrude), (Key::I, Tool::Inset)] {
            let ctx = egui::Context::default();
            let (mut doc, mut editor) = (Document::default(), EditorState::default());
            let input = RawInput {
                events: vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..RawInput::default()
            };
            ctx.run_ui(input, |ui| handle(ui.ctx(), &mut doc, &mut editor))
                .drop_without_applying_deltas();
            assert_eq!(editor.tool, tool);
        }
    }

    #[test]
    fn plain_keys_and_drags_never_undo() {
        assert!(press(Key::Z, Modifiers::NONE, false).is_empty());
        assert!(press(Key::Y, Modifiers::NONE, false).is_empty());
        assert!(
            press(Key::Z, Modifiers::COMMAND, true).is_empty(),
            "a drag in progress must finish first"
        );
    }
}
