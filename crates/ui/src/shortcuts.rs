//! Global keyboard shortcuts (Cinema 4D defaults where they exist).
//!
//! | Key            | Action                     |
//! |----------------|----------------------------|
//! | `E` / `T` / `R`| Move / Scale / Rotate tool |
//! | `9` / `0`      | Live / Rectangle Selection tool |
//! | `D` / `I`      | Extrude / Inset tool       |
//! | `C`            | Make Editable              |
//! | `Ctrl+⌫` / `Alt+⌫` | Dissolve / Melt        |
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
//!
//! Two-key chords (press the first key, then the second) are listed in
//! [`Chord::complete`]: `U~L` Loop Selection, `K~L` Loop/Path Cut, `M~M` Connect, …

use egui::{Event, Key};
use grid4d_scene::Document;

use crate::{EditorCommand, EditorState, Tool};

/// The first key of a two-key shortcut, Cinema 4D style (`U~L`: U, then L).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    /// Selection commands.
    U,
    /// Knife tools.
    K,
    /// Modelling commands.
    M,
}

/// What a completed chord does.
enum Action {
    Tool(Tool),
    Command(EditorCommand),
    /// Open the Optimize dialog.
    Optimize,
}

impl Chord {
    fn of(key: Key) -> Option<Self> {
        match key {
            Key::U => Some(Self::U),
            Key::K => Some(Self::K),
            Key::M => Some(Self::M),
            _ => None,
        }
    }

    /// Status-bar help while the chord waits for its second key.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::U => {
                "U…  L Loop · B Ring · W Connected · Y Grow · K Shrink · I Invert · Z Melt · O Optimize · Esc cancels"
            }
            Self::K => "K…  L Loop/Path Cut · K Line Cut · Esc cancels",
            Self::M => {
                "M…  M Connect · N Dissolve · T Extrude · W Inset · S Bevel · B Bridge · L Loop/Path Cut · K Line Cut · Esc cancels"
            }
        }
    }

    /// The chord's second key, from Maxon's shortcut list.
    fn complete(self, key: Key) -> Option<Action> {
        use Action::{Command, Tool as Pick};
        Some(match (self, key) {
            (Self::U, Key::L) => Pick(Tool::LoopSelection),
            (Self::U, Key::B) => Pick(Tool::RingSelection),
            (Self::U, Key::W) => Command(EditorCommand::SelectConnected),
            (Self::U, Key::Y) => Command(EditorCommand::GrowSelection),
            (Self::U, Key::K) => Command(EditorCommand::ShrinkSelection),
            (Self::U, Key::I) => Command(EditorCommand::InvertSelection),
            (Self::U, Key::Z) => Command(EditorCommand::MeltSelected),
            (Self::U, Key::O) => Action::Optimize,
            (Self::K | Self::M, Key::L) => Pick(Tool::LoopCut),
            (Self::K | Self::M, Key::K) => Pick(Tool::LineCut),
            (Self::M, Key::M) => Command(EditorCommand::ConnectSelected),
            (Self::M, Key::N) => Command(EditorCommand::DissolveSelected),
            (Self::M, Key::T) => Pick(Tool::Extrude),
            (Self::M, Key::W) => Pick(Tool::Inset),
            (Self::M, Key::S) => Pick(Tool::Bevel),
            (Self::M, Key::B) => Pick(Tool::Bridge),
            _ => return None,
        })
    }
}

/// Handles global shortcuts. Ignored while a text field has keyboard focus
/// (text fields have their own undo).
pub fn handle(ctx: &egui::Context, doc: &mut Document, editor: &mut EditorState) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    ctx.input(|i| {
        let plain = i.modifiers.is_none();
        let time = &mut doc.time;

        // Chords first: a pending first key takes the next plain key (any other
        // key, Esc included, just cancels it); U, K or M start one.
        let mut chorded = false;
        for event in &i.events {
            let Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if let Some(chord) = editor.chord.take() {
                chorded = true;
                if modifiers.is_none() {
                    match chord.complete(*key) {
                        Some(Action::Tool(tool)) => editor.tool = tool,
                        Some(Action::Command(command)) => editor.commands.push(command),
                        Some(Action::Optimize) => editor.show_optimize = true,
                        None => {}
                    }
                }
            } else if modifiers.is_none()
                && let Some(chord) = Chord::of(*key)
            {
                editor.chord = Some(chord);
                chorded = true;
            }
        }
        if i.pointer.any_pressed() {
            editor.chord = None;
        }
        if chorded {
            return;
        }

        if plain {
            for (key, tool) in [
                (Key::E, Tool::Move),
                (Key::T, Tool::Scale),
                (Key::R, Tool::Rotate),
                (Key::Num9, Tool::LiveSelection),
                (Key::Num0, Tool::RectangleSelection),
                (Key::D, Tool::Extrude),
                (Key::I, Tool::Inset),
                (Key::B, Tool::Bridge),
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
            if i.key_pressed(Key::Q) {
                editor.commands.push(EditorCommand::ToggleParentGenerator);
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
        if i.modifiers.shift_only() && i.key_pressed(Key::S) {
            editor.snap.enabled = !editor.snap.enabled;
        }
        let backspace = i.key_pressed(Key::Backspace) || i.key_pressed(Key::Delete);
        if backspace && i.modifiers.command && !i.modifiers.alt && !i.modifiers.shift {
            editor.commands.push(EditorCommand::DissolveSelected);
        }
        if backspace && i.modifiers.alt && !i.modifiers.command && !i.modifiers.shift {
            editor.commands.push(EditorCommand::MeltSelected);
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
    fn shift_s_toggles_snapping_like_c4d() {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::default(), EditorState::default());
        let shift_s = |editor: &mut EditorState, doc: &mut Document| {
            let modifiers = Modifiers::SHIFT;
            let input = RawInput {
                events: vec![
                    Event::ModifiersChanged(modifiers),
                    Event::Key {
                        key: Key::S,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..RawInput::default()
            };
            ctx.run_ui(input, |ui| handle(ui.ctx(), doc, editor))
                .drop_without_applying_deltas();
        };
        shift_s(&mut editor, &mut doc);
        assert!(editor.snap.enabled);
        shift_s(&mut editor, &mut doc);
        assert!(!editor.snap.enabled);
        assert!(press(Key::S, Modifiers::COMMAND | Modifiers::SHIFT, false) == [EditorCommand::SaveAs]);
    }

    #[test]
    fn q_toggles_the_parent_generator_like_c4d() {
        assert_eq!(
            press(Key::Q, Modifiers::NONE, false),
            [EditorCommand::ToggleParentGenerator]
        );
        assert!(press(Key::Q, Modifiers::COMMAND, false).is_empty(), "Ctrl+Q quits");
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

    fn keys(sequence: &[(Key, Modifiers)]) -> (EditorState, Vec<EditorCommand>) {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::default(), EditorState::default());
        for &(key, modifiers) in sequence {
            let input = RawInput {
                events: vec![
                    Event::ModifiersChanged(modifiers),
                    Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    // Released again, as real keys are (a key still down repeats).
                    Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..RawInput::default()
            };
            ctx.run_ui(input, |ui| handle(ui.ctx(), &mut doc, &mut editor))
                .drop_without_applying_deltas();
        }
        let commands = editor.commands.pending().to_vec();
        (editor, commands)
    }

    #[test]
    fn chords_follow_maxons_list() {
        let none = Modifiers::NONE;
        let (editor, _) = keys(&[(Key::U, none)]);
        assert_eq!(editor.chord, Some(Chord::U), "U waits for the second key");
        let (editor, commands) = keys(&[(Key::U, none), (Key::L, none)]);
        assert_eq!(
            (editor.tool, editor.chord, commands.len()),
            (Tool::LoopSelection, None, 0)
        );
        assert_eq!(keys(&[(Key::K, none), (Key::K, none)]).0.tool, Tool::LineCut);
        assert_eq!(keys(&[(Key::M, none), (Key::L, none)]).0.tool, Tool::LoopCut);
        assert_eq!(
            keys(&[(Key::M, none), (Key::M, none)]).1,
            [EditorCommand::ConnectSelected]
        );
        assert_eq!(
            keys(&[(Key::M, none), (Key::N, none)]).1,
            [EditorCommand::DissolveSelected]
        );
        assert_eq!(keys(&[(Key::U, none), (Key::Z, none)]).1, [EditorCommand::MeltSelected]);
        // An unbound second key does nothing, not even its own shortcut.
        let (editor, commands) = keys(&[(Key::U, none), (Key::E, none)]);
        assert_eq!((editor.tool, editor.chord, commands.len()), (Tool::Move, None, 0));
        let (editor, _) = keys(&[(Key::U, none), (Key::Escape, none), (Key::T, none)]);
        assert_eq!(editor.tool, Tool::Scale, "Esc cancels the chord, then T works again");
    }

    #[test]
    fn single_keys_for_rectangle_selection_dissolve_and_melt() {
        assert_eq!(keys(&[(Key::Num0, Modifiers::NONE)]).0.tool, Tool::RectangleSelection);
        assert_eq!(
            keys(&[(Key::Backspace, Modifiers::COMMAND)]).1,
            [EditorCommand::DissolveSelected]
        );
        assert_eq!(
            keys(&[(Key::Backspace, Modifiers::ALT)]).1,
            [EditorCommand::MeltSelected]
        );
        assert_eq!(
            keys(&[(Key::Backspace, Modifiers::NONE)]).1,
            [EditorCommand::DeleteSelected]
        );
    }

    #[test]
    fn bevel_bridge_and_optimize_keys() {
        let none = Modifiers::NONE;
        assert_eq!(keys(&[(Key::M, none), (Key::S, none)]).0.tool, Tool::Bevel);
        assert_eq!(keys(&[(Key::M, none), (Key::B, none)]).0.tool, Tool::Bridge);
        assert_eq!(keys(&[(Key::B, none)]).0.tool, Tool::Bridge);
        let (editor, commands) = keys(&[(Key::U, none), (Key::O, none)]);
        assert!(editor.show_optimize && commands.is_empty(), "U~O opens the dialog");
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
