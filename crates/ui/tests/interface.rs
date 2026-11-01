//! The interface driven headless with `egui_kittest`: clicks, drags, typing
//! and modifier keys go through egui exactly as a user's would, on the whole
//! interface (`Shell`) with the demo scene — what posted window messages
//! cannot reach on screen (spec `docs/reference/app-release.md`, 14.5).

#![allow(clippy::unwrap_used, reason = "test code")]

use egui::{Key, Modifiers, PointerButton, Pos2, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use glam::Vec3;
use grid4d_scene::{Document, NodeId, ObjectKind, ObjectKindTag};
use grid4d_ui::{EditorCommand, EditorState, ViewportPanel, shell::Shell, workspace::Workspace};

/// The interface with the demo scene, the first object active, as Grid4D
/// starts (without a GPU: the viewport takes input but draws no picture).
fn harness() -> Harness<'static, Shell> {
    let doc = Document::with_demo_scene();
    let mut editor = EditorState::default();
    if let Some(&first) = doc.scene.roots().first() {
        editor.selection.set_single(first);
    }
    let shell = Shell::new(doc, editor, ViewportPanel::new(None), Workspace::default());
    let mut harness = Harness::builder()
        .with_size([1600.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| grid4d_ui::theme::apply(ctx, grid4d_ui::theme::ColorTheme::Dark))
        .build_ui_state(|ui, shell: &mut Shell| shell.ui(ui), shell);
    harness.run_steps(3);
    harness
}

fn id_of(shell: &Shell, name: &str) -> NodeId {
    shell
        .doc
        .scene
        .iter()
        .find(|(_, n)| n.data.name == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no object named {name}"))
}

/// The Object Manager's row of the object `name`.
fn row(harness: &Harness<'_, Shell>, name: &str) -> egui::Rect {
    harness.get_by_role_and_label(ROW, name).rect()
}

/// The role of an Object Manager row.
const ROW: Role = Role::TreeItem;

/// `count` clicks at `pos` with `modifiers` held, a frame apart (two make a
/// double click).
fn click_at(harness: &mut Harness<'_, Shell>, pos: Pos2, modifiers: Modifiers, count: usize) {
    harness.hover_at(pos);
    harness.run_steps(1);
    for _ in 0..count {
        for pressed in [true, false] {
            harness.event_modifiers(
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers,
                },
                modifiers,
            );
        }
        harness.run_steps(1);
    }
    harness.run_steps(2);
}

fn selected(harness: &Harness<'_, Shell>) -> Vec<String> {
    let shell = harness.state();
    shell
        .editor
        .selection
        .as_slice()
        .iter()
        .map(|&id| shell.doc.scene.get(id).unwrap().data.name.clone())
        .collect()
}

fn undo(harness: &mut Harness<'_, Shell>) {
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run_steps(2);
}

/// Presses at `from`, moves to `to` in steps, releases there.
fn drag(harness: &mut Harness<'_, Shell>, from: Pos2, to: Pos2, modifiers: Modifiers) {
    let press = |harness: &mut Harness<'_, Shell>, pos: Pos2, pressed: bool| {
        harness.event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
    };
    harness.hover_at(from);
    harness.run_steps(1);
    press(harness, from, true);
    harness.run_steps(1);
    for k in 1..=8u8 {
        harness.hover_at(from + (to - from) * (f32::from(k) / 8.0));
        harness.run_steps(1);
    }
    press(harness, to, false);
    harness.run_steps(3);
}

#[test]
fn shift_adds_and_ctrl_toggles_in_the_object_manager() {
    let mut harness = harness();
    let cube = row(&harness, "Cube").center();
    click_at(&mut harness, cube, Modifiers::NONE, 1);
    assert_eq!(selected(&harness), ["Cube"]);

    let sphere = row(&harness, "Sphere").center();
    click_at(&mut harness, sphere, Modifiers::SHIFT, 1);
    assert_eq!(selected(&harness), ["Cube", "Sphere"]);

    click_at(&mut harness, cube, Modifiers::COMMAND, 1);
    assert_eq!(selected(&harness), ["Sphere"], "Ctrl toggles the cube off");
}

#[test]
fn dragging_a_row_onto_another_reparents_it_as_one_undo_step() {
    let mut harness = harness();
    let (sphere, group) = (id_of(harness.state(), "Sphere"), id_of(harness.state(), "Group"));
    let world = harness.state().doc.scene.world_matrix(sphere);
    let from = row(&harness, "Sphere").center();
    let onto = row(&harness, "Group").center();
    drag(&mut harness, from, onto, Modifiers::NONE);
    let scene = &harness.state().doc.scene;
    assert_eq!(scene.get(sphere).unwrap().parent(), Some(group));
    assert!(
        scene.world_matrix(sphere).abs_diff_eq(world, 1e-3),
        "it stays where it was"
    );
    undo(&mut harness);
    assert_eq!(harness.state().doc.scene.get(sphere).unwrap().parent(), None);
}

#[test]
fn renaming_in_the_object_manager_and_undo() {
    let mut harness = harness();
    let cube = id_of(harness.state(), "Cube");
    let at = row(&harness, "Cube").center();
    click_at(&mut harness, at, Modifiers::NONE, 2);
    // The name field takes the focus (the search field is a text input too).
    let field = harness
        .get_all_by_role(Role::TextInput)
        .find(egui_kittest::Node::is_focused)
        .unwrap();
    field.type_text(" A");
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().doc.scene.get(cube).unwrap().data.name, "Cube A");
    assert_eq!(harness.state().editor.history.undo_label(), Some("Rename"));
    undo(&mut harness);
    assert_eq!(harness.state().doc.scene.get(cube).unwrap().data.name, "Cube");
}

#[test]
fn a_typed_value_is_one_undo_step() {
    let mut harness = harness();
    let cube = id_of(harness.state(), "Cube");
    let before = harness.state().doc.scene.get(cube).unwrap().data.transform;
    let field = harness.get_by_role_and_label(Role::SpinButton, "P . X");
    field.click();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.get_by_role(Role::TextInput).type_text("250");
    harness.key_press(Key::Enter);
    harness.run_steps(3);
    let after = harness.state().doc.scene.get(cube).unwrap().data.transform;
    assert!((after.position.x - 250.0).abs() < 1e-3, "{after:?}");
    undo(&mut harness);
    assert_eq!(harness.state().doc.scene.get(cube).unwrap().data.transform, before);
}

#[test]
fn dragging_the_gizmo_moves_the_object_in_one_undo_step() {
    let mut harness = harness();
    let cube = id_of(harness.state(), "Cube");
    let shell = harness.state();
    let centre = shell.doc.scene.world_matrix(cube).w_axis.truncate();
    let viewport = &shell.panels.viewport;
    let (Some(origin), Some(along)) = (
        viewport.screen_position(centre),
        viewport.screen_position(centre + Vec3::X * 10.0),
    ) else {
        panic!("the cube is in view");
    };
    // The X handle reaches about 90 points from the object; grab it there.
    let direction = (along - origin).normalized();
    let grab = origin + direction * 70.0;
    let before = shell.doc.scene.get(cube).unwrap().data.transform.position;
    drag(&mut harness, grab, grab + direction * 120.0, Modifiers::NONE);
    let after = harness.state().doc.scene.get(cube).unwrap().data.transform.position;
    assert!(after.x > before.x + 10.0, "moved along X: {before} → {after}");
    assert!((after.y - before.y).abs() < 1e-3 && (after.z - before.z).abs() < 1e-3);
    undo(&mut harness);
    assert_eq!(
        harness.state().doc.scene.get(cube).unwrap().data.transform.position,
        before
    );
}

#[test]
fn dragging_a_bend_handle_bends_in_one_undo_step() {
    let mut harness = harness();
    let cube = id_of(harness.state(), "Cube");
    harness.state_mut().editor.selection.set_single(cube);
    harness
        .state_mut()
        .editor
        .commands
        .push(EditorCommand::CreateChild(ObjectKindTag::Bend));
    harness.run_steps(3);
    let shell = harness.state();
    let bend = shell.editor.selection.active().unwrap();
    let ObjectKind::Bend(deformer) = &shell.doc.scene.get(bend).unwrap().data.kind else {
        panic!("a Bend was made");
    };
    let handle = shell.doc.scene.world_matrix(bend).transform_point3(deformer.handle());
    let Some(grab) = shell.panels.viewport.screen_position(handle) else {
        panic!("the handle is in view");
    };
    let strength = |harness: &Harness<'_, Shell>| match &harness.state().doc.scene.get(bend).unwrap().data.kind {
        ObjectKind::Bend(bend) => bend.strength,
        _ => unreachable!(),
    };
    let before = strength(&harness);
    drag(&mut harness, grab, grab + egui::vec2(80.0, 0.0), Modifiers::NONE);
    assert!(
        (strength(&harness) - before).abs() > 0.01,
        "bent: {}",
        strength(&harness)
    );
    undo(&mut harness);
    assert_eq!(strength(&harness), before);
}

#[test]
fn a_menu_command_creates_an_object() {
    let mut harness = harness();
    let count = harness.state().doc.scene.len();
    let create = harness.get_by_role_and_label(Role::Button, "Create").rect();
    harness.get_by_role_and_label(Role::Button, "Create").click();
    harness.run_steps(2);
    // Create ▸ Object (the Object Manager has an "Object" menu too).
    let object = harness
        .get_all_by_label("Object")
        .map(|node| node.rect())
        .filter(|rect| rect.top() >= create.bottom() && (rect.left() - create.left()).abs() < 40.0)
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    harness.hover_at(object.center());
    harness.run_steps(2);
    harness.get_by_label("Torus").click();
    harness.run_steps(3);
    let shell = harness.state();
    assert_eq!(shell.doc.scene.len(), count + 1);
    let active = shell.editor.selection.active().unwrap();
    assert_eq!(
        shell.doc.scene.get(active).unwrap().data.kind.tag(),
        ObjectKindTag::Torus
    );
    assert_eq!(shell.editor.history.undo_label(), Some("Create Object"));
}
