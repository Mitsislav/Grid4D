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
//! | `Ctrl+F` / `Ctrl+G` | Previous / next key   |
//! | `F9`           | Record Active Objects      |
//! | `F1`–`F4` / `F5` | View 1–4 alone / all views |
//! | `Tab` (over the bottom panel) | Timeline / F-Curve ([`take_tab`]) |
//! | `H` / `S` (over the F-Curve graph) | Frame all / selected keys |
//! | `Ctrl+F9`      | Autokeying on / off        |
//! | `Shift+F5` / `Shift+F7` | Attribute / Coordinate Manager |
//! | `Shift+F2`     | Material Manager (bottom tab) |
//! | `Ctrl+N` (over the Material Manager) | New material |
//! | `Ctrl+B`       | Render Settings window open / closed |
//! | `Shift+R`      | Render to Picture Viewer   |
//! | `Ctrl+R` / `Esc` | Render View / clear it   |
//!
//! Two-key chords (press the first key, then the second) are listed in
//! [`Chord::complete`]: `U~L` Loop Selection, `K~L` Loop/Path Cut, `M~M` Connect,
//! `N~A` Gouraud Shading, …

use egui::{Event, Key};
use grid4d_render::{DisplayStyle, ShadingMode};
use grid4d_scene::Document;

use crate::{EditorCommand, EditorState, FrameCurves, Tool, workspace::Pane};

/// The first key of a two-key shortcut, Cinema 4D style (`U~L`: U, then L).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    /// Selection commands.
    U,
    /// Knife tools.
    K,
    /// Modelling commands.
    M,
    /// The active view's display mode.
    N,
}

/// What a completed chord does.
#[expect(clippy::large_enum_variant, reason = "one per key press, never stored")]
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
            Key::N => Some(Self::N),
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
            Self::N => {
                "N…  A Gouraud · B Gouraud (Lines) · C Quick · D Quick (Lines) · E Constant · F Hidden Line · G Lines · H Wireframe · K Box · Esc cancels"
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
            (Self::N, Key::A) => Command(EditorCommand::SetShading(ShadingMode::Gouraud)),
            (Self::N, Key::B) => Command(EditorCommand::SetShading(ShadingMode::GouraudLines)),
            (Self::N, Key::C) => Command(EditorCommand::SetShading(ShadingMode::Quick)),
            (Self::N, Key::D) => Command(EditorCommand::SetShading(ShadingMode::QuickLines)),
            (Self::N, Key::E) => Command(EditorCommand::SetShading(ShadingMode::Constant)),
            (Self::N, Key::F) => Command(EditorCommand::SetShading(ShadingMode::HiddenLine)),
            (Self::N, Key::G) => Command(EditorCommand::SetShading(ShadingMode::Lines)),
            (Self::N, Key::H) => Command(EditorCommand::SetDisplayStyle(DisplayStyle::Wireframe)),
            (Self::N, Key::K) => Command(EditorCommand::SetDisplayStyle(DisplayStyle::Box)),
            _ => return None,
        })
    }
}

/// Tab over the bottom panel switches between the Timeline and the F-Curve
/// editor. egui would move keyboard focus to the next widget on Tab, so the key
/// is taken out of the input before egui sees it (call from
/// `eframe::App::raw_input_hook`). Nothing happens while a widget has focus.
pub fn take_tab(ctx: &egui::Context, editor: &mut EditorState, input: &mut egui::RawInput) {
    let other = match editor.panes.hovered {
        Some(Pane::Timeline) => Pane::FCurve,
        Some(Pane::FCurve) => Pane::Timeline,
        _ => return,
    };
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    let is_tab = |event: &Event| matches!(event, Event::Key { key: Key::Tab, modifiers, .. } if modifiers.is_none());
    let pressed = input
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Key {
                    pressed: true,
                    repeat: false,
                    ..
                }
            ) && is_tab(e)
        })
        .count();
    if pressed == 0 {
        return;
    }
    input.events.retain(|e| !is_tab(e));
    if pressed % 2 == 1 {
        editor.panes.show = Some(other);
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
            // Over the F-Curve graph, H and S frame the curves.
            let curves = editor.panes.is_shown(Pane::FCurve) && editor.curves.hovered;
            if i.key_pressed(Key::H) {
                if curves {
                    editor.curves.frame = Some(FrameCurves::All);
                } else {
                    editor.commands.push(EditorCommand::FrameScene);
                }
            }
            if curves && i.key_pressed(Key::S) {
                editor.curves.frame = Some(FrameCurves::Selected);
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
                // After a click in the timeline's key strip or the F-Curve graph,
                // Delete removes keys; in the Material Manager, the material.
                let shown = |pane| editor.panes.is_shown(pane);
                let material = editor
                    .materials
                    .selected
                    .filter(|_| shown(Pane::Materials) && editor.materials.focused);
                let command = if shown(Pane::FCurve) && editor.curves.focused && !editor.curves.keys.is_empty() {
                    EditorCommand::EditCurveKeys {
                        op: crate::keys::CurveOp::Delete,
                    }
                } else if shown(Pane::Timeline) && editor.keys.focused && !editor.keys.frames.is_empty() {
                    EditorCommand::DeleteKeys
                } else if let Some(material) = material {
                    EditorCommand::DeleteMaterial(material)
                } else {
                    EditorCommand::DeleteSelected
                };
                editor.commands.push(command);
            }
            if i.key_pressed(Key::F9) {
                editor.commands.push(EditorCommand::RecordKeys { at: time.frame() });
            }
            for (key, view) in [
                (Key::F1, Some(0)),
                (Key::F2, Some(1)),
                (Key::F3, Some(2)),
                (Key::F4, Some(3)),
                (Key::F5, None),
            ] {
                if i.key_pressed(key) {
                    editor.commands.push(EditorCommand::ShowView(view));
                }
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
        if i.modifiers.shift_only() && i.key_pressed(Key::R) {
            editor.commands.push(EditorCommand::RenderToPictureViewer);
        }
        for (key, pane) in [
            (Key::F2, Pane::Materials),
            (Key::F5, Pane::Attributes),
            (Key::F7, Pane::Coordinates),
        ] {
            if i.modifiers.shift_only() && i.key_pressed(key) {
                editor.panes.show = Some(pane);
            }
        }
        let ctrl_only = i.modifiers.command && !i.modifiers.shift && !i.modifiers.alt;
        if ctrl_only && i.key_pressed(Key::F9) {
            editor.animation.autokey = !editor.animation.autokey;
        }
        if ctrl_only && i.key_pressed(Key::B) {
            editor.show_render_settings = !editor.show_render_settings;
        }
        if ctrl_only && i.key_pressed(Key::R) {
            editor.commands.push(EditorCommand::RenderView);
        }
        // Esc stops baking, else clears the Render View; a drag in progress
        // takes it for itself.
        if plain && i.key_pressed(Key::Escape) && !i.pointer.any_down() && !editor.simulation.stop_baking() {
            editor.renders.clear_view();
        }
        for (key, forward) in [(Key::F, false), (Key::G, true)] {
            if ctrl_only
                && i.key_pressed(key)
                && let Some(frame) = crate::keys::adjacent_key(&doc.scene, &editor.selection, time.frame(), forward)
            {
                time.stop();
                time.set_frame(frame);
            }
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
                // Over the Material Manager, Ctrl+N makes a material.
                editor.commands.push(if editor.panes.hovered == Some(Pane::Materials) {
                    EditorCommand::NewMaterial
                } else {
                    EditorCommand::NewDocument
                });
            } else if i.key_pressed(Key::O) {
                editor.commands.push(if i.modifiers.shift {
                    EditorCommand::Merge
                } else {
                    EditorCommand::Open
                });
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
        assert_eq!(press(Key::O, ctrl | Modifiers::SHIFT, false), [EditorCommand::Merge]);
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
        assert_eq!(
            keys(&[(Key::N, none), (Key::B, none)]).1,
            [EditorCommand::SetShading(ShadingMode::GouraudLines)]
        );
        assert_eq!(
            keys(&[(Key::N, none), (Key::K, none)]).1,
            [EditorCommand::SetDisplayStyle(DisplayStyle::Box)]
        );
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

    /// Presses `key` once with `modifiers` on the given document and editor.
    fn press_on(doc: &mut Document, editor: &mut EditorState, key: Key, modifiers: Modifiers) {
        let ctx = egui::Context::default();
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
            ],
            ..RawInput::default()
        };
        ctx.run_ui(input, |ui| handle(ui.ctx(), doc, editor))
            .drop_without_applying_deltas();
    }

    #[test]
    fn ctrl_r_renders_the_view_and_esc_clears_it() {
        assert_eq!(press(Key::R, Modifiers::COMMAND, false), [EditorCommand::RenderView]);
        let mut doc = Document::with_demo_scene();
        crate::rendering::tests::small(&mut doc, 1);
        let mut editor = EditorState::default();
        let key = crate::ViewportPanel::new(None).view_key(&doc);
        editor
            .renders
            .start_view(&doc, &grid4d_render::OrbitCamera::default(), key, None);
        assert!(editor.renders.view_render().is_some());
        press_on(&mut doc, &mut editor, Key::Escape, Modifiers::NONE);
        assert!(editor.renders.view_render().is_none(), "Esc clears it");
    }

    #[test]
    fn shift_r_renders_to_the_picture_viewer() {
        assert_eq!(
            press(Key::R, Modifiers::SHIFT, false),
            [EditorCommand::RenderToPictureViewer]
        );
        assert!(press(Key::R, Modifiers::NONE, false).is_empty(), "R is the Rotate tool");
    }

    #[test]
    fn ctrl_b_opens_and_closes_the_render_settings() {
        let (mut doc, mut editor) = (Document::with_demo_scene(), EditorState::default());
        press_on(&mut doc, &mut editor, Key::B, Modifiers::COMMAND);
        assert!(editor.show_render_settings);
        assert_ne!(editor.tool, Tool::Bridge, "not the Bridge tool (B)");
        press_on(&mut doc, &mut editor, Key::B, Modifiers::COMMAND);
        assert!(!editor.show_render_settings);
    }

    #[test]
    fn animation_keys_follow_maxons_list() {
        assert_eq!(
            press(Key::F9, Modifiers::NONE, false),
            [EditorCommand::RecordKeys { at: 0 }]
        );
        let (mut doc, mut editor) = (Document::with_demo_scene(), EditorState::default());
        press_on(&mut doc, &mut editor, Key::F9, Modifiers::COMMAND);
        assert!(editor.animation.autokey, "Ctrl+F9 switches Autokeying on");
        press_on(&mut doc, &mut editor, Key::F9, Modifiers::COMMAND);
        assert!(!editor.animation.autokey);

        // Ctrl+G / Ctrl+F jump between the selected object's keys.
        let cube = doc.scene.roots()[0];
        editor.selection.set_single(cube);
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        data.record([grid4d_scene::Param::Scale(grid4d_scene::Axis::X)], 0.0, 12.0);
        data.record([grid4d_scene::Param::Scale(grid4d_scene::Axis::X)], 0.0, 40.0);
        press_on(&mut doc, &mut editor, Key::G, Modifiers::COMMAND);
        assert_eq!(doc.time.frame(), 12);
        press_on(&mut doc, &mut editor, Key::G, Modifiers::COMMAND);
        assert_eq!(doc.time.frame(), 40);
        press_on(&mut doc, &mut editor, Key::G, Modifiers::COMMAND);
        assert_eq!(doc.time.frame(), 40, "no key after the last");
        press_on(&mut doc, &mut editor, Key::F, Modifiers::COMMAND);
        assert_eq!(doc.time.frame(), 12);
    }

    #[test]
    fn delete_removes_keys_after_a_click_in_the_key_strip() {
        let (mut doc, mut editor) = (Document::default(), EditorState::default());
        editor.keys.frames = vec![5.0];
        press_on(&mut doc, &mut editor, Key::Delete, Modifiers::NONE);
        assert_eq!(
            editor.commands.pending(),
            [EditorCommand::DeleteSelected],
            "not focused"
        );
        let mut editor = EditorState::default();
        editor.keys.frames = vec![5.0];
        editor.keys.focused = true;
        press_on(&mut doc, &mut editor, Key::Delete, Modifiers::NONE);
        assert_eq!(editor.commands.pending(), [EditorCommand::DeleteKeys]);
    }

    #[test]
    fn over_the_f_curve_graph_keys_act_on_curves() {
        let (mut doc, mut editor) = (Document::default(), EditorState::default());
        editor.panes.mark_shown(Pane::FCurve);
        editor.curves.hovered = true;
        press_on(&mut doc, &mut editor, Key::H, Modifiers::NONE);
        press_on(&mut doc, &mut editor, Key::S, Modifiers::NONE);
        assert_eq!(editor.curves.frame, Some(FrameCurves::Selected));
        assert!(
            editor.commands.pending().is_empty(),
            "H framed the curves, not the scene"
        );

        editor.curves.focused = true;
        editor.curves.keys.push(crate::CurveKey {
            node: grid4d_scene::NodeId::default(),
            param: grid4d_scene::Param::Position(grid4d_scene::Axis::X),
            frame: 0.0,
        });
        press_on(&mut doc, &mut editor, Key::Delete, Modifiers::NONE);
        assert_eq!(
            editor.commands.pending(),
            [EditorCommand::EditCurveKeys {
                op: crate::keys::CurveOp::Delete
            }]
        );
        // On the Timeline tab the curve selection does not matter.
        let mut editor = EditorState {
            curves: editor.curves.clone(),
            ..EditorState::default()
        };
        press_on(&mut doc, &mut editor, Key::Delete, Modifiers::NONE);
        assert_eq!(editor.commands.pending(), [EditorCommand::DeleteSelected]);
    }

    #[test]
    fn tab_over_the_bottom_panel_switches_it_before_egui_sees_it() {
        let ctx = egui::Context::default();
        let mut editor = EditorState::default();
        let tab = |pressed| Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut input = RawInput {
            events: vec![tab(true), tab(false)],
            ..RawInput::default()
        };
        take_tab(&ctx, &mut editor, &mut input);
        assert_eq!(editor.panes.show, None, "not over the panel");
        assert_eq!(input.events.len(), 2, "left for egui");
        editor.panes.hovered = Some(Pane::Timeline);
        take_tab(&ctx, &mut editor, &mut input);
        assert_eq!(editor.panes.show, Some(Pane::FCurve));
        assert!(input.events.is_empty(), "taken out");
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
