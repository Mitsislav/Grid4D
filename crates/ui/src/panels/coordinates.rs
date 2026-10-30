//! The Coordinate Manager (see `docs/reference/manager-coordinate.md`): exact
//! position, scale or size, and rotation of the active object — or, in a
//! component mode, of the selected points, edges or polygons — in object or world
//! space. A tab beside the Attribute Manager.
//!
//! Fields write the document live; each finished interaction is one undo step
//! (`EditorCommand::CommitLive`). Values are those at the current frame, written
//! back through `NodeData::set_transform_at` (Autokeying, temporary values).

use egui::{Button, ComboBox, DragValue, Grid, Margin, RichText, Ui, Vec2};
use glam::{Mat4, Quat, Vec3};
use grid4d_geom::{Component, selection};
use grid4d_scene::{Aabb, Document, NodeData, NodeId, ObjectKind, Scene, SceneEdit, Tags, Transform};

use crate::{EditorCommand, EditorState, keys::Keying, live_edit::interaction_ongoing, theme::palette as p, widgets};

/// How positions and rotations are read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    /// In the object's parent (the Coord. tab's values).
    #[default]
    Object,
    World,
}

impl Space {
    const ALL: [Self; 2] = [Self::Object, Self::World];

    const fn label(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::World => "World",
        }
    }
}

/// What the middle column shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Middle {
    #[default]
    Scale,
    /// The object's extent along its axes, scale included.
    Size,
    /// The same with its children.
    SizePlus,
}

impl Middle {
    const ALL: [Self; 3] = [Self::Scale, Self::Size, Self::SizePlus];

    const fn label(self) -> &'static str {
        match self {
            Self::Scale => "Scale",
            Self::Size => "Size",
            Self::SizePlus => "Size+",
        }
    }
}

/// How the fields are read: the space and the middle column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub space: Space,
    pub middle: Middle,
}

/// Position, middle column, rotation (degrees), each X / Y / Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fields {
    pub position: Vec3,
    /// `None` when there is nothing to measure (a Null's size).
    pub middle: Option<Vec3>,
    pub rotation: Vec3,
}

/// What the manager edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Object(NodeId),
    /// Selected components of a polygon object.
    Components(NodeId, Component),
}

impl Target {
    const fn node(self) -> NodeId {
        match self {
            Self::Object(id) | Self::Components(id, _) => id,
        }
    }
}

/// An interaction in progress: the node's data and the fields when it began.
#[derive(Clone, Debug)]
struct Edit {
    target: Target,
    before: NodeData,
    fields: Fields,
}

#[derive(Debug, Default)]
pub struct CoordinatesPanel {
    mode: Mode,
    /// What the fields show (and what the user types into).
    fields: Option<Fields>,
    edit: Option<Edit>,
}

impl CoordinatesPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        let keying = Keying::of(doc, editor);
        let target = target(doc, editor);
        // The selection or mode changed during an edit: finish it.
        if self.edit.as_ref().is_some_and(|e| Some(e.target) != target) {
            self.finish(editor);
        }
        let current = target.and_then(|t| read(&doc.scene, t, keying.frame, self.mode));
        if self.edit.is_none() {
            self.fields = current;
        }

        let scroll = egui::ScrollArea::vertical()
            .id_salt("grid4d.coordinates.scroll")
            .auto_shrink([false; 2]);
        scroll.show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                let Some(target) = target else {
                    ui.label(RichText::new("Select an object to edit its coordinates.").color(p().text_dim));
                    return;
                };
                let title = match target {
                    Target::Object(id) => doc.scene.get(id).map(|n| n.data.name.clone()),
                    Target::Components(id, component) => doc.scene.get(id).map(|n| {
                        let what = match component {
                            Component::Point => "Points",
                            Component::Edge => "Edges",
                            Component::Polygon => "Polygons",
                        };
                        format!("{what} of {}", n.data.name)
                    }),
                };
                ui.label(RichText::new(title.unwrap_or_default()).strong().color(p().text_strong));
                ui.add_space(4.0);
                let Some(mut fields) = self.fields else {
                    ui.label(RichText::new("Nothing selected").color(p().text_dim));
                    return;
                };
                let locks = match target {
                    Target::Object(id) => doc
                        .scene
                        .get(id)
                        .map_or_else(Default::default, |n| n.data.tags.protection()),
                    Target::Components(..) => grid4d_scene::Protection::default(),
                };
                let middle = match target {
                    Target::Object(_) => self.mode.middle,
                    Target::Components(..) => Middle::Size,
                };
                let changed = fields_grid(ui, &mut fields, middle, locks);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ComboBox::from_id_salt("grid4d.coordinates.space")
                        .width(76.0)
                        .selected_text(self.mode.space.label())
                        .show_ui(ui, |ui| {
                            for space in Space::ALL {
                                ui.selectable_value(&mut self.mode.space, space, space.label());
                            }
                        });
                    ui.add_enabled_ui(matches!(target, Target::Object(_)), |ui| {
                        ComboBox::from_id_salt("grid4d.coordinates.middle")
                            .width(64.0)
                            .selected_text(middle.label())
                            .show_ui(ui, |ui| {
                                for m in Middle::ALL {
                                    ui.selectable_value(&mut self.mode.middle, m, m.label());
                                }
                            });
                    });
                    let apply = ui.add(Button::new("Apply")).on_hover_text("Writes the fields as shown");
                    if apply.clicked()
                        && let (Some(current), Some(node)) = (current, doc.scene.get(target.node()))
                        && let Some(data) = write(&doc.scene, target, &node.data, current, fields, keying, self.mode)
                        && data != node.data
                    {
                        editor.commands.push(EditorCommand::Edit {
                            edit: SceneEdit::SetData {
                                id: target.node(),
                                data,
                            },
                            label: "Apply Coordinates",
                        });
                    }
                });
                if let Target::Components(..) = target {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Rotation is relative: it turns the selection by that much around its centre")
                            .color(p().text_dim),
                    );
                }

                if changed {
                    if self.edit.is_none()
                        && let (Some(node), Some(start)) = (doc.scene.get(target.node()), current)
                    {
                        self.edit = Some(Edit {
                            target,
                            before: node.data.clone(),
                            fields: start,
                        });
                    }
                    self.fields = Some(fields);
                    if let Some(edit) = &self.edit
                        && let Some(data) =
                            write(&doc.scene, target, &edit.before, edit.fields, fields, keying, self.mode)
                        && let Some(node) = doc.scene.get_mut(target.node())
                        && node.data != data
                    {
                        node.data = data;
                    }
                }
            })
        });
        if self.edit.is_some() && !interaction_ongoing(ui) {
            self.finish(editor);
        }
    }

    /// Records the interaction in progress as one undo step.
    fn finish(&mut self, editor: &mut EditorState) {
        if let Some(edit) = self.edit.take() {
            let label = match edit.target {
                Target::Object(_) => "Edit Coordinates",
                Target::Components(..) => "Transform Components",
            };
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: edit.target.node(),
                    data: edit.before,
                },
                label,
            });
        }
    }
}

/// What the manager works on: the selected components of the active polygon
/// object in a component mode, else the active object.
fn target(doc: &Document, editor: &EditorState) -> Option<Target> {
    let id = editor.selection.active()?;
    let node = doc.scene.get(id)?;
    match (editor.mode.component(), &node.data.kind) {
        (Some(component), ObjectKind::Polygon { .. }) => Some(Target::Components(id, component)),
        (Some(_), _) | (None, _) => Some(Target::Object(id)),
    }
}

/// The three columns of fields; returns whether a value changed.
fn fields_grid(ui: &mut Ui, fields: &mut Fields, middle: Middle, locks: grid4d_scene::Protection) -> bool {
    let mut changed = false;
    let (middle_title, middle_is_length, middle_speed) = match middle {
        Middle::Scale => ("Scale", false, 0.01),
        Middle::Size | Middle::SizePlus => ("Size", true, 1.0),
    };
    Grid::new("grid4d.coordinates.fields")
        .num_columns(6)
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for title in ["", "Position", "", middle_title, "", "Rotation"] {
                ui.label(RichText::new(title).color(p().text_dim).size(11.5));
            }
            ui.end_row();
            for (axis, [pl, ml, rl]) in [["X", "X", "H"], ["Y", "Y", "P"], ["Z", "Z", "B"]]
                .into_iter()
                .enumerate()
            {
                small(ui, pl);
                changed |= ui
                    .add_enabled_ui(!locks.position, |ui| {
                        ui.add_sized(
                            Vec2::new(70.0, 19.0),
                            widgets::length(ui, &mut fields.position[axis])
                                .speed(1.0)
                                .max_decimals(2),
                        )
                    })
                    .inner
                    .changed();
                small(ui, ml);
                match &mut fields.middle {
                    Some(m) => {
                        changed |= ui
                            .add_enabled_ui(!locks.scale, |ui| {
                                let field = if middle_is_length {
                                    widgets::length(ui, &mut m[axis])
                                } else {
                                    DragValue::new(&mut m[axis])
                                };
                                ui.add_sized(
                                    Vec2::new(64.0, 19.0),
                                    field.speed(middle_speed).range(0.001..=1.0e7).max_decimals(3),
                                )
                            })
                            .inner
                            .changed();
                    }
                    None => {
                        ui.label(RichText::new("—").color(p().text_faint));
                    }
                }
                small(ui, rl);
                changed |= ui
                    .add_enabled_ui(!locks.rotation, |ui| {
                        ui.add_sized(
                            Vec2::new(60.0, 19.0),
                            DragValue::new(&mut fields.rotation[axis])
                                .speed(0.5)
                                .suffix("°")
                                .max_decimals(2),
                        )
                    })
                    .inner
                    .changed();
                ui.end_row();
            }
        });
    changed
}

fn small(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(p().text_dim).size(11.5));
}

/// The fields for `target` at `frame`.
fn read(scene: &Scene, target: Target, frame: f64, mode: Mode) -> Option<Fields> {
    match target {
        Target::Object(id) => read_object(scene, id, frame, mode),
        Target::Components(id, component) => read_components(scene, id, component, frame, mode.space),
    }
}

fn read_object(scene: &Scene, id: NodeId, frame: f64, Mode { space, middle }: Mode) -> Option<Fields> {
    let data = &scene.get(id)?.data;
    let shown = data.transform_at(frame);
    let world = scene.world_matrix_at(id, frame);
    let (world_scale, world_rotation, world_position) = world.to_scale_rotation_translation();
    let (position, rotation, scale) = match space {
        Space::Object => (shown.position, shown.rotation, shown.scale),
        Space::World => (world_position, hpb(world_rotation), world_scale),
    };
    let middle = match middle {
        Middle::Scale => Some(scale),
        Middle::Size => data
            .kind_at(frame)
            .local_bounds()
            .map(|b| (b.max - b.min) * world_scale.abs()),
        Middle::SizePlus => subtree_bounds(scene, id, frame).map(|b| (b.max - b.min) * world_scale.abs()),
    };
    Some(Fields {
        position,
        middle,
        rotation: degrees(rotation),
    })
}

/// The bounds of `id` and everything below it, in `id`'s own space.
fn subtree_bounds(scene: &Scene, id: NodeId, frame: f64) -> Option<Aabb> {
    let into = scene.world_matrix_at(id, frame).inverse();
    let mut bounds = Aabb::EMPTY;
    let mut pending = vec![id];
    while let Some(current) = pending.pop() {
        if let Some(node) = scene.get(current)
            && let Some(local) = node.data.kind_at(frame).local_bounds()
        {
            let relative = into * scene.world_matrix_at(current, frame);
            bounds = bounds.union(local.transformed(&relative));
        }
        pending.extend_from_slice(scene.children(current));
    }
    (!bounds.is_empty()).then_some(bounds)
}

fn read_components(scene: &Scene, id: NodeId, component: Component, frame: f64, space: Space) -> Option<Fields> {
    let ObjectKind::Polygon { mesh, components } = &scene.get(id)?.data.kind else {
        return None;
    };
    let points = selection::points_of(mesh.mesh(), components.selected().get(component), component);
    let into = space_matrix(scene, id, frame, space);
    let positions = mesh.mesh().points();
    let mut found = points
        .iter()
        .filter_map(|i| positions.get(i))
        .map(|&p| into.transform_point3(p));
    let first = found.next()?;
    let (min, max) = found.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
    Some(Fields {
        position: (min + max) * 0.5,
        middle: Some(max - min),
        rotation: Vec3::ZERO,
    })
}

/// Object space → the space values are read in, for components.
fn space_matrix(scene: &Scene, id: NodeId, frame: f64, space: Space) -> Mat4 {
    match space {
        Space::Object => Mat4::IDENTITY,
        Space::World => scene.world_matrix_at(id, frame),
    }
}

/// `before` (the node's data when the interaction began, whose fields were
/// `from`) changed so that its fields read `to`. Only the columns that differ
/// are applied, so world values that do not round-trip exactly change nothing.
fn write(
    scene: &Scene,
    target: Target,
    before: &NodeData,
    from: Fields,
    to: Fields,
    keying: Keying,
    mode: Mode,
) -> Option<NodeData> {
    match target {
        Target::Object(id) => Some(write_object(scene, id, before, from, to, keying, mode)),
        Target::Components(id, component) => {
            write_components(scene, id, component, before, from, to, keying.frame, mode.space)
        }
    }
}

fn write_object(
    scene: &Scene,
    id: NodeId,
    before: &NodeData,
    from: Fields,
    to: Fields,
    keying: Keying,
    Mode { space, middle }: Mode,
) -> NodeData {
    let frame = keying.frame;
    let mut shown = before.transform_at(frame);
    let parent = scene.parent_world_matrix_at(id, frame);
    let (parent_scale, parent_rotation, _) = parent.to_scale_rotation_translation();
    if to.position != from.position {
        shown.position = match space {
            Space::Object => to.position,
            Space::World => parent.inverse().transform_point3(to.position),
        };
    }
    if to.rotation != from.rotation {
        let previous = shown.rotation;
        let rotation = radians(to.rotation);
        match space {
            Space::Object => shown.rotation = rotation,
            Space::World => {
                let world = Transform {
                    rotation,
                    ..Transform::IDENTITY
                }
                .rotation_quat();
                shown.set_rotation_quat(parent_rotation.inverse() * world);
                // Continuous angles, as with the gizmo.
                for i in 0..3 {
                    let d = shown.rotation[i] - previous[i];
                    shown.rotation[i] = previous[i] + (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                }
            }
        }
    }
    if let (Some(old), Some(new)) = (from.middle, to.middle)
        && old != new
    {
        // Scale in the world: divide the parent's out. Sizes: scale by the ratio.
        shown.scale = match (middle, space) {
            (Middle::Scale, Space::Object) => new,
            (Middle::Scale, Space::World) => new / nonzero(parent_scale),
            (Middle::Size | Middle::SizePlus, _) => shown.scale * ratio(new, old),
        };
    }
    let mut data = before.clone();
    data.set_transform_at(&shown, frame, keying.autokey);
    data
}

#[expect(clippy::too_many_arguments, reason = "the pieces of one component edit")]
fn write_components(
    scene: &Scene,
    id: NodeId,
    component: Component,
    before: &NodeData,
    from: Fields,
    to: Fields,
    frame: f64,
    space: Space,
) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &before.kind else {
        return None;
    };
    let points = selection::points_of(mesh.mesh(), components.selected().get(component), component);
    let into = space_matrix(scene, id, frame, space);
    if into.determinant().abs() <= f32::EPSILON {
        return None;
    }
    let scale = match (from.middle, to.middle) {
        (Some(old), Some(new)) => ratio(new, old),
        _ => Vec3::ONE,
    };
    let rotation = Transform {
        rotation: radians(to.rotation - from.rotation),
        ..Transform::IDENTITY
    }
    .rotation_quat();
    // In the chosen space: around the old centre, scale, turn, then move to the new centre.
    let change = Mat4::from_translation(to.position)
        * Mat4::from_quat(rotation)
        * Mat4::from_scale(scale)
        * Mat4::from_translation(-from.position);
    let local = into.inverse() * change * into;
    let mut moved = mesh.mesh().points().to_vec();
    for i in points.iter() {
        if let Some(point) = moved.get_mut(i) {
            *point = local.transform_point3(*point);
        }
    }
    let mut data = before.clone();
    if let ObjectKind::Polygon { mesh, .. } = &mut data.kind {
        *mesh = mesh.with_points(&moved);
    }
    Some(data)
}

/// `new / old` per axis; 1 where `old` is zero.
fn ratio(new: Vec3, old: Vec3) -> Vec3 {
    Vec3::new(
        if old.x.abs() > f32::EPSILON { new.x / old.x } else { 1.0 },
        if old.y.abs() > f32::EPSILON { new.y / old.y } else { 1.0 },
        if old.z.abs() > f32::EPSILON { new.z / old.z } else { 1.0 },
    )
}

fn nonzero(v: Vec3) -> Vec3 {
    v.map(|c| if c.abs() > f32::EPSILON { c } else { 1.0 })
}

fn hpb(q: Quat) -> Vec3 {
    let mut t = Transform::IDENTITY;
    t.set_rotation_quat(q);
    t.rotation
}

fn degrees(v: Vec3) -> Vec3 {
    Vec3::new(v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees())
}

fn radians(v: Vec3) -> Vec3 {
    Vec3::new(v.x.to_radians(), v.y.to_radians(), v.z.to_radians())
}

#[cfg(test)]
mod tests {
    use grid4d_geom::BitSet;
    use grid4d_scene::{MeshHandle, Node, ObjectKindTag};

    use super::*;

    const KEYING: Keying = Keying {
        frame: 0.0,
        autokey: false,
    };

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
    }

    fn mode(space: Space, middle: Middle) -> Mode {
        Mode { space, middle }
    }

    /// Fields of `id`, changed by `change`, written back to the document.
    fn set(doc: &mut Document, target: Target, mode: Mode, change: impl FnOnce(&mut Fields)) {
        let from = read(&doc.scene, target, 0.0, mode).unwrap();
        let mut to = from;
        change(&mut to);
        let before = doc.scene.get(target.node()).unwrap().data.clone();
        let data = write(&doc.scene, target, &before, from, to, KEYING, mode).unwrap();
        doc.scene.get_mut(target.node()).unwrap().data = data;
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.abs_diff_eq(b, 1e-3)
    }

    #[test]
    fn entering_the_values_shown_changes_nothing() {
        let doc = Document::with_demo_scene();
        let boxed = Target::Object(id_of(&doc, "Box"));
        for space in Space::ALL {
            for middle in Middle::ALL {
                let m = mode(space, middle);
                let fields = read(&doc.scene, boxed, 0.0, m).unwrap();
                let before = &doc.scene.get(boxed.node()).unwrap().data;
                let data = write(&doc.scene, boxed, before, fields, fields, KEYING, m).unwrap();
                assert_eq!(&data, before, "{space:?} {middle:?}");
            }
        }
    }

    #[test]
    fn world_values_place_an_object_whatever_its_parent() {
        let mut doc = Document::with_demo_scene();
        let boxed = id_of(&doc, "Box");
        let group = id_of(&doc, "Group");
        doc.scene.get_mut(group).unwrap().data.transform =
            Transform::from_position(Vec3::new(320.0, 0.0, -80.0)).with_rotation_deg(Vec3::new(45.0, 10.0, 0.0));
        let world = mode(Space::World, Middle::Scale);
        set(&mut doc, Target::Object(boxed), world, |f| {
            f.position = Vec3::new(10.0, 20.0, 30.0);
            f.rotation = Vec3::new(90.0, 0.0, 0.0);
        });
        let m = doc.scene.world_matrix(boxed);
        let (_, rotation, position) = m.to_scale_rotation_translation();
        assert!(close(position, Vec3::new(10.0, 20.0, 30.0)), "{position}");
        let expected = Quat::from_rotation_y(90f32.to_radians());
        assert!(rotation.angle_between(expected) < 1e-3, "world heading 90°");
        let shown = read(&doc.scene, Target::Object(boxed), 0.0, world).unwrap();
        assert!(close(shown.rotation, Vec3::new(90.0, 0.0, 0.0)), "{}", shown.rotation);
    }

    #[test]
    fn scale_size_and_size_plus() {
        let mut doc = Document::with_demo_scene();
        let cube = Target::Object(id_of(&doc, "Cube"));
        let size = mode(Space::Object, Middle::Size);
        assert_eq!(
            read(&doc.scene, cube, 0.0, size).unwrap().middle,
            Some(Vec3::splat(200.0))
        );
        set(&mut doc, cube, size, |f| {
            f.middle = Some(Vec3::new(400.0, 200.0, 100.0))
        });
        let scale = doc.scene.get(cube.node()).unwrap().data.transform.scale;
        assert!(close(scale, Vec3::new(2.0, 1.0, 0.5)), "a new size scales: {scale}");

        // A Null has no size of its own; Size+ measures its children.
        let group = Target::Object(id_of(&doc, "Group"));
        assert_eq!(read(&doc.scene, group, 0.0, size).unwrap().middle, None);
        let plus = read(&doc.scene, group, 0.0, mode(Space::Object, Middle::SizePlus))
            .unwrap()
            .middle
            .unwrap();
        assert!(plus.x > 100.0 && plus.y >= 200.0, "{plus}");

        // World scale divides the parent's out.
        let boxed = id_of(&doc, "Box");
        doc.scene.get_mut(group.node()).unwrap().data.transform.scale = Vec3::splat(2.0);
        set(
            &mut doc,
            Target::Object(boxed),
            mode(Space::World, Middle::Scale),
            |f| {
                f.middle = Some(Vec3::splat(3.0));
            },
        );
        let scale = doc.scene.get(boxed).unwrap().data.transform.scale;
        assert!(close(scale, Vec3::splat(1.5)), "{scale}");
    }

    #[test]
    fn animated_values_with_autokeying() {
        let mut doc = Document::with_demo_scene();
        let cube = id_of(&doc, "Cube");
        let px = grid4d_scene::Param::Position(grid4d_scene::Axis::X);
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        data.record([px], 0.0, 0.0);
        data.set_value_at(px, 100.0, 10.0, true);
        let target = Target::Object(cube);
        let object = mode(Space::Object, Middle::Scale);
        let at10 = read(&doc.scene, target, 10.0, object).unwrap();
        assert_eq!(at10.position.x, 100.0, "the value at the frame");
        let mut to = at10;
        to.position.x = 150.0;
        let before = doc.scene.get(cube).unwrap().data.clone();
        let keying = Keying {
            frame: 10.0,
            autokey: true,
        };
        let data = write(&doc.scene, target, &before, at10, to, keying, object).unwrap();
        let key = data.tracks[0].curve.key_at(10.0).unwrap();
        assert_eq!(key.value, 150.0, "a key with Autokeying");
    }

    fn polygon_cube(doc: &mut Document, points: &[usize]) -> NodeId {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let mut kind = ObjectKind::polygon(MeshHandle::new(mesh));
        if let ObjectKind::Polygon { components, .. } = &mut kind {
            let selected: BitSet = points.iter().copied().collect();
            components.make_mut().selected.points = selected;
        }
        doc.scene
            .insert(Node::new("Poly", kind).with_position(Vec3::new(0.0, 0.0, 500.0)))
    }

    fn points(doc: &Document, id: NodeId) -> Vec<Vec3> {
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("polygon object");
        };
        mesh.mesh().points().to_vec()
    }

    #[test]
    fn components_move_scale_and_turn_around_their_centre() {
        let mut doc = Document::default();
        let id = polygon_cube(&mut doc, &[0, 1, 2, 3, 4, 5, 6, 7]);
        let target = Target::Components(id, Component::Point);
        let object = mode(Space::Object, Middle::Size);
        let fields = read(&doc.scene, target, 0.0, object).unwrap();
        assert!(close(fields.position, Vec3::ZERO));
        assert_eq!(fields.middle, Some(Vec3::splat(200.0)));
        assert_eq!(fields.rotation, Vec3::ZERO);
        let world = read(&doc.scene, target, 0.0, mode(Space::World, Middle::Size)).unwrap();
        assert!(close(world.position, Vec3::new(0.0, 0.0, 500.0)), "world space");

        set(&mut doc, target, object, |f| {
            f.position.y = 50.0;
            f.middle = Some(Vec3::new(100.0, 200.0, 200.0));
        });
        let after = read(&doc.scene, target, 0.0, object).unwrap();
        assert!(close(after.position, Vec3::new(0.0, 50.0, 0.0)), "{}", after.position);
        assert!(close(after.middle.unwrap(), Vec3::new(100.0, 200.0, 200.0)));

        // A quarter turn about Y swaps the box's X and Z extents.
        set(&mut doc, target, object, |f| f.rotation.x = 90.0);
        let turned = read(&doc.scene, target, 0.0, object).unwrap();
        assert!(
            close(turned.middle.unwrap(), Vec3::new(200.0, 200.0, 100.0)),
            "{:?}",
            turned.middle
        );
        assert_eq!(turned.rotation, Vec3::ZERO, "rotation reads 0 again");
        assert!(close(turned.position, Vec3::new(0.0, 50.0, 0.0)), "around the centre");
    }

    #[test]
    fn only_the_selected_points_move() {
        let mut doc = Document::default();
        let id = polygon_cube(&mut doc, &[0]);
        let before = points(&doc, id);
        set(
            &mut doc,
            Target::Components(id, Component::Point),
            mode(Space::Object, Middle::Size),
            |f| {
                f.position.x += 10.0;
            },
        );
        let after = points(&doc, id);
        assert!(close(after[0], before[0] + Vec3::X * 10.0));
        assert_eq!(after[1..], before[1..]);
    }
}
