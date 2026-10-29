//! Attribute Manager: parameters of the active object, organised in C4D's
//! Basic / Coord. / Object tabs. With nothing selected it shows the project
//! settings, like Cinema 4D does. In Tool mode it shows the active tool's
//! options instead; choosing a selection tool switches to Tool mode, changing
//! the object selection switches back.
//!
//! Fields write to the document live; each finished interaction (a drag, a
//! typed value, a click) is recorded as one undo step through [`LiveEdit`].

use egui::{Button, DragValue, Grid, Margin, RichText, ScrollArea, Sense, TextEdit, Ui, Vec2};
use grid4d_geom::cloner::{CloneOrder, Cloner, Distribution, Layout, StepMode};
use grid4d_geom::deform::{Bend, BendOrigin, DeformerMode, Ffd, Shape};
use grid4d_geom::effector::{Effector, FalloffShape};
use grid4d_geom::loops::WalkOptions;
use grid4d_geom::primitives::MAX_SEGMENTS;
use grid4d_geom::spline::{
    ArcType, Interpolation, SplinePlane, SplinePrimitive, SplineShape, SplineType, Text, TextAlign,
};
use grid4d_geom::surface::{CapType, ExtrudeDirection, Extrusion, Lathe, Loft, Sweep};
use grid4d_scene::{
    Axis, Document, Falloff, KeyState, LightDetails, LightShadow, LightType, Material, MaterialId, NodeData, NodeId,
    ObjectKind, ObjectParam, Param, Scene, SceneEdit, SplineHandle, Tag, TagKind, TimeSettings,
};

use crate::{
    CommandQueue, EditorCommand, EditorState, Tool,
    commands::tag_color,
    icons::{self, Icon},
    keys::Keying,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::palette as p,
    widgets::{attr_label, manager_area_tabs, section_heading},
};

const LABEL_WIDTH: f32 = 128.0;
const FIELD_WIDTH: f32 = 96.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Tab {
    Basic,
    #[default]
    Coord,
    Object,
}

impl Tab {
    /// Undo label for edits made on this tab.
    const fn edit_label(self) -> &'static str {
        match self {
            Self::Basic => "Edit Basic Properties",
            Self::Coord => "Edit Coordinates",
            Self::Object => "Edit Object Properties",
        }
    }
}

/// What the Attribute Manager shows (its Mode menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum View {
    #[default]
    Objects,
    Tool,
}

#[derive(Debug, Default)]
pub struct AttributesPanel {
    view: View,
    /// The tool and active object last frame, to switch views when they change.
    seen_tool: Option<Tool>,
    seen_active: Option<NodeId>,
    tab: Tab,
    node_edit: LiveEdit<NodeId, NodeData>,
    material_edit: LiveEdit<MaterialId, Material>,
    name_edit: LiveEdit<(), String>,
    time_edit: LiveEdit<(), TimeSettings>,
}

impl AttributesPanel {
    pub fn show(&mut self, ui: &mut Ui, doc: &mut Document, editor: &mut EditorState) {
        // Like C4D: picking a selection tool shows its options, selecting objects shows them.
        if self.seen_tool != Some(editor.tool) {
            if self.seen_tool.is_some() {
                self.view = if editor.tool.shows_options() {
                    View::Tool
                } else {
                    View::Objects
                };
            }
            self.seen_tool = Some(editor.tool);
            editor.materials.shown = false;
        }
        if self.seen_active != editor.selection.active() {
            self.view = View::Objects;
            self.seen_active = editor.selection.active();
            editor.materials.shown = false;
        }

        manager_area_tabs(ui, editor);
        egui::Frame::new().inner_margin(Margin::symmetric(4, 1)).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Mode", |ui| {
                    for (view, label) in [(View::Objects, "Objects"), (View::Tool, "Tool")] {
                        if ui.add(Button::new(label).selected(self.view == view)).clicked() {
                            self.view = view;
                            ui.close();
                        }
                    }
                    ui.add_enabled(false, Button::new("Tags"));
                    ui.add_enabled(false, Button::new("Materials"));
                });
                ui.menu_button("Edit", |ui| {
                    ui.add_enabled(false, Button::new("Copy"));
                    ui.add_enabled(false, Button::new("Paste"));
                });
                ui.menu_button("User Data", |ui| {
                    ui.add_enabled(false, Button::new("Add User Data…"));
                });
            });
        });

        ScrollArea::vertical()
            .id_salt("grid4d.attributes.scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                    // A material picked in the Material Manager comes first.
                    if editor.materials.shown
                        && let Some(id) = editor.materials.selected.filter(|&m| doc.scene.materials().contains(m))
                    {
                        self.material_ui(ui, doc, id, &mut editor.commands);
                        return;
                    }
                    match (self.view, editor.selection.active()) {
                        (View::Tool, _) => tool_ui(ui, doc, editor),
                        (View::Objects, Some(id)) => {
                            let shown = Shown {
                                tag: editor.selected_tag(doc).map(|(tag, index)| (tag.kind, index)),
                                selection_len: editor.selection.len(),
                                keying: Keying::of(doc, editor),
                            };
                            self.node_ui(ui, doc, id, shown, &mut editor.commands);
                        }
                        (View::Objects, None) => self.project_ui(ui, doc, &mut editor.commands),
                    }
                });
            });
    }

    /// A material's settings, edited live; one undo step per interaction.
    fn material_ui(&mut self, ui: &mut Ui, doc: &mut Document, id: MaterialId, commands: &mut CommandQueue) {
        let Some(mut material) = doc.scene.materials().get(id).cloned() else {
            return;
        };
        self.material_edit.begin(id, &material);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            icons::paint_material_ball(ui.painter(), rect, material.color);
            ui.label(
                RichText::new(format!("Material [{}]", material.name))
                    .strong()
                    .color(p::TEXT_STRONG),
            );
        });
        section_heading(ui, "Material");
        Grid::new("grid4d.attr.material")
            .num_columns(2)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                attr_label(ui, "Name", LABEL_WIDTH);
                ui.add(TextEdit::singleline(&mut material.name).desired_width(f32::INFINITY));
                ui.end_row();
                attr_label(ui, "Color", LABEL_WIDTH);
                colour_button(ui, &mut material.color);
                ui.end_row();
                for (label, value) in [
                    ("Roughness", &mut material.roughness),
                    ("Metallic", &mut material.metallic),
                ] {
                    attr_label(ui, label, LABEL_WIDTH);
                    let mut percent = *value * 100.0;
                    if ui
                        .add_sized(
                            [FIELD_WIDTH, 19.0],
                            DragValue::new(&mut percent).speed(0.5).range(0.0..=100.0).suffix(" %"),
                        )
                        .changed()
                    {
                        *value = percent / 100.0;
                    }
                    ui.end_row();
                }
                attr_label(ui, "Luminance", LABEL_WIDTH);
                colour_button(ui, &mut material.emission);
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.label(
            RichText::new("The viewport shows the colour; roughness, metallic and luminance shade it from Phase 7")
                .color(p::TEXT_DIM),
        );
        if doc.scene.materials().get(id).is_some_and(|m| *m != material)
            && let Some(m) = doc.scene.materials_mut().get_mut(id)
        {
            m.clone_from(&material);
        }
        if let Some((id, before)) = self.material_edit.end(id, &material, interaction_ongoing(ui)) {
            let mut table = doc.scene.materials().clone();
            if let Some(m) = table.get_mut(id) {
                *m = before;
            }
            commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetMaterials(table),
                label: "Edit Material",
            });
        }
    }

    /// The active object's attributes, or those of its selected tag.
    fn node_ui(&mut self, ui: &mut Ui, doc: &mut Document, id: NodeId, shown: Shown, commands: &mut CommandQueue) {
        let Shown {
            tag,
            selection_len,
            keying,
        } = shown;
        // Widgets edit a copy; the scene is only written (and its revision only
        // bumped) when a value actually changed, so idle frames stay idle.
        let Some(mut data) = doc.scene.get(id).map(|node| node.data.clone()) else {
            return;
        };
        self.node_edit.begin(id, &data);

        if let Some((kind, index)) = tag {
            // Title: "Phong Tag [Cube]"
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                icons::paint(ui.painter(), rect, Icon::Tag(kind), p::TAG);
                ui.label(
                    RichText::new(format!("{} Tag [{}]", kind.label(), data.name))
                        .strong()
                        .color(p::TEXT_STRONG),
                );
            });
            ui.add_space(4.0);
            if let Some(tag) = data.tags.get_mut(index) {
                tag_ui(ui, tag, &doc.scene, id);
            }
            self.finish_node_edit(ui, doc, id, &data, "Edit Tag", commands);
            return;
        }

        // Title: "Cube Object [Cube]"
        ui.horizontal(|ui| {
            let tag = data.kind.tag();
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Object(tag), tag_color(tag));
            ui.label(
                RichText::new(format!("{} Object [{}]", data.kind.label(), data.name))
                    .strong()
                    .color(p::TEXT_STRONG),
            );
            if selection_len > 1 {
                ui.label(RichText::new(format!("+{} more", selection_len - 1)).color(p::TEXT_DIM));
            }
        });
        ui.add_space(4.0);

        // Tab buttons.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, label) in [(Tab::Basic, "Basic"), (Tab::Coord, "Coord."), (Tab::Object, "Object")] {
                let button = Button::new(label)
                    .selected(self.tab == tab)
                    .min_size(Vec2::new(64.0, 20.0));
                if ui.add(button).clicked() {
                    self.tab = tab;
                }
            }
        });

        let mut label = self.tab.edit_label();
        match self.tab {
            Tab::Basic => basic_tab(ui, &mut data),
            Tab::Coord => label = coord_tab(ui, &mut data, keying).unwrap_or(label),
            Tab::Object => {
                let mut fit = false;
                label = object_tab_at(ui, &mut data, keying, &mut fit, Some((&doc.scene, id))).unwrap_or(label);
                if fit {
                    commands.push(EditorCommand::FitToParent(id));
                }
            }
        }
        self.finish_node_edit(ui, doc, id, &data, label, commands);
    }

    /// Writes the edited copy back when it changed, and records one undo step when
    /// the interaction ends.
    fn finish_node_edit(
        &mut self,
        ui: &Ui,
        doc: &mut Document,
        id: NodeId,
        data: &NodeData,
        label: &'static str,
        commands: &mut CommandQueue,
    ) {
        if doc.scene.get(id).is_some_and(|node| node.data != *data)
            && let Some(node) = doc.scene.get_mut(id)
        {
            node.data.clone_from(data);
        }
        if let Some((id, before)) = self.node_edit.end(id, data, interaction_ongoing(ui)) {
            commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData { id, data: before },
                label,
            });
        }
    }

    fn project_ui(&mut self, ui: &mut Ui, doc: &mut Document, commands: &mut CommandQueue) {
        self.name_edit.begin((), &doc.name);
        self.time_edit.begin((), &doc.time.settings());
        project_settings(ui, doc);

        let ongoing = interaction_ongoing(ui);
        if let Some(((), before)) = self.name_edit.end((), &doc.name, ongoing) {
            commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetName(before),
                label: "Rename Project",
            });
        }
        if let Some(((), before)) = self.time_edit.end((), &doc.time.settings(), ongoing) {
            commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetTime(before),
                label: "Project Settings",
            });
        }
    }
}

fn basic_tab(ui: &mut Ui, data: &mut NodeData) {
    section_heading(ui, "Basic Properties");
    Grid::new("grid4d.attr.basic")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Name", LABEL_WIDTH);
            ui.add(TextEdit::singleline(&mut data.name).desired_width(f32::INFINITY));
            ui.end_row();

            attr_label(ui, "Visible in Editor", LABEL_WIDTH);
            ui.checkbox(&mut data.visible_in_editor, "");
            ui.end_row();

            attr_label(ui, "Visible in Renderer", LABEL_WIDTH);
            ui.checkbox(&mut data.visible_in_render, "");
            ui.end_row();

            attr_label(ui, "Display Color", LABEL_WIDTH);
            colour_button(ui, &mut data.display_color);
            ui.end_row();

            attr_label(ui, "Enabled", LABEL_WIDTH);
            ui.checkbox(&mut data.enabled, "");
            ui.end_row();
        });
}

/// The Coordinates fields show and edit the values at the current frame
/// (`keying`); the circle before each is its key button. Returns an undo label
/// when a key button was used.
fn coord_tab(ui: &mut Ui, data: &mut NodeData, keying: Keying) -> Option<&'static str> {
    section_heading(ui, "Coordinates");
    const ROWS: [[&str; 3]; 3] = [
        ["P . X", "S . X", "R . H"],
        ["P . Y", "S . Y", "R . P"],
        ["P . Z", "S . Z", "R . B"],
    ];
    // A Protection tag makes its channels read-only.
    let protection = grid4d_scene::Tags::protection(data.tags.as_slice());
    let frame = keying.frame;
    let before = data.transform_at(frame);
    let mut shown = before;
    let t = &mut shown;
    let mut clicked: Option<(Param, KeyClick)> = None;
    Grid::new("grid4d.attr.coord")
        .num_columns(6)
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (axis, [pl, sl, rl]) in ROWS.into_iter().enumerate() {
                let axis_id = Axis::ALL[axis];
                let mut label = |ui: &mut Ui, param: Param, text: &str| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        if let Some(click) = key_button(ui, data.key_state(param, frame)) {
                            clicked = Some((param, click));
                        }
                        small_label(ui, text);
                    });
                };
                label(ui, Param::Position(axis_id), pl);
                ui.add_enabled_ui(!protection.position, |ui| {
                    ui.add_sized(
                        [64.0, 19.0],
                        DragValue::new(&mut t.position[axis])
                            .speed(1.0)
                            .suffix(" cm")
                            .max_decimals(2),
                    )
                });
                label(ui, Param::Scale(axis_id), sl);
                ui.add_enabled_ui(!protection.scale, |ui| {
                    ui.add_sized(
                        [48.0, 19.0],
                        DragValue::new(&mut t.scale[axis])
                            .speed(0.01)
                            .range(0.001..=10_000.0)
                            .max_decimals(3),
                    )
                });
                label(ui, Param::Rotation(axis_id), rl);
                let mut degrees = t.rotation[axis].to_degrees();
                let rotated = ui
                    .add_enabled_ui(!protection.rotation, |ui| {
                        ui.add_sized(
                            [56.0, 19.0],
                            DragValue::new(&mut degrees).speed(0.5).suffix("°").max_decimals(2),
                        )
                    })
                    .inner
                    .changed();
                if rotated {
                    t.rotation[axis] = degrees.to_radians();
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    let unprotected = !(protection.position || protection.scale || protection.rotation);
    if ui
        .add_enabled(unprotected, Button::new("Reset Transform"))
        .on_disabled_hover_text("A Protection tag guards this object's transform")
        .clicked()
    {
        *t = grid4d_scene::Transform::IDENTITY;
    }
    if shown != before {
        data.set_transform_at(&shown, frame, keying.autokey);
    }
    let (param, click) = clicked?;
    Some(match click {
        KeyClick::Toggle => {
            data.toggle_key(param, frame);
            "Set Key"
        }
        KeyClick::DeleteTrack => {
            data.delete_track(param, frame);
            "Delete Track"
        }
    })
}

/// What a click on a key button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyClick {
    /// Plain click: set a key, or remove the one shown.
    Toggle,
    /// Shift+Ctrl+click: remove the whole track.
    DeleteTrack,
}

/// A parameter's key button: grey ring (not animated), red ring (animated),
/// red disc (a key here), yellow (a temporary value).
fn key_button(ui: &mut Ui, state: KeyState) -> Option<KeyClick> {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(11.0), Sense::click());
    let (color, filled) = match state {
        KeyState::Unanimated => (p::TEXT_FAINT, false),
        KeyState::Track => (p::RECORD, false),
        KeyState::Key => (p::RECORD, true),
        KeyState::Changed { key } => (p::KEY_CHANGED, key),
    };
    let color = if response.hovered() {
        color.gamma_multiply(1.4)
    } else {
        color
    };
    let (center, radius) = (rect.center(), 4.0);
    if filled {
        ui.painter().circle_filled(center, radius, color);
    } else {
        ui.painter()
            .circle_stroke(center, radius - 0.6, egui::Stroke::new(1.3, color));
    }
    let tip = match state {
        KeyState::Unanimated => "Click: set a key at the current frame",
        KeyState::Track => "Animated · click: set a key here · Shift+Ctrl+click: delete the track",
        KeyState::Key => "Key at this frame · click: remove it · Shift+Ctrl+click: delete the track",
        KeyState::Changed { .. } => {
            "Changed but not recorded: dropped when the frame changes · click: set a key · Shift+Ctrl+click: delete the track"
        }
    };
    if !response.on_hover_text(tip).clicked() {
        return None;
    }
    let m = ui.input(|i| i.modifiers);
    Some(if m.shift && m.command {
        KeyClick::DeleteTrack
    } else {
        KeyClick::Toggle
    })
}

/// What the Attribute Manager shows for the active object.
#[derive(Clone, Copy, Debug)]
struct Shown {
    /// The selected tag: its kind and index in the object's tags.
    tag: Option<(TagKind, usize)>,
    selection_len: usize,
    keying: Keying,
}

/// The Object tab at the current frame (`keying`): the parameters as they are
/// there, edits written back as keys, temporary values or base values (like
/// the Coordinates fields), and the key buttons. Returns an undo label when a
/// key button was used.
fn object_tab_at(
    ui: &mut Ui,
    data: &mut NodeData,
    keying: Keying,
    fit: &mut bool,
    links: Option<(&Scene, NodeId)>,
) -> Option<&'static str> {
    let frame = keying.frame;
    let shown = data.kind_at(frame).into_owned();
    let mut edited = shown.clone();
    let mut keys = KeyRows::new(data, &shown, frame);
    object_tab(ui, &mut edited, &mut keys, links);
    *fit = keys.fit_to_parent;
    if edited != shown {
        data.set_kind_at(&edited, frame, keying.autokey);
    }
    let (param, click) = keys.clicked?;
    let param = Param::Object(param);
    Some(match click {
        KeyClick::Toggle => {
            data.toggle_key(param, frame);
            "Set Key"
        }
        KeyClick::DeleteTrack => {
            data.delete_track(param, frame);
            "Delete Track"
        }
    })
}

/// The key buttons of the Object tab: which of the shown object's parameters a
/// row edits (found by the row's label), their states, the buttons drawn and
/// the one clicked. Off (`default`) on other tabs.
#[derive(Debug, Default)]
struct KeyRows {
    states: Vec<(ObjectParam, KeyState)>,
    drawn: Vec<ObjectParam>,
    clicked: Option<(ObjectParam, KeyClick)>,
    /// A deformer's Fit to Parent button was clicked.
    fit_to_parent: bool,
}

impl KeyRows {
    fn new(data: &NodeData, shown: &ObjectKind, frame: f64) -> Self {
        Self {
            states: shown
                .params()
                .into_iter()
                .map(|p| (p, data.key_state(Param::Object(p), frame)))
                .collect(),
            ..Self::default()
        }
    }

    /// The key button of the row labelled `label`, after its field; nothing
    /// for rows that are not an animatable parameter.
    fn button(&mut self, ui: &mut Ui, label: &str) {
        let Some(&(param, state)) = self.states.iter().find(|(p, _)| p.label() == label) else {
            return;
        };
        self.drawn.push(param);
        if let Some(click) = key_button(ui, state) {
            self.clicked = Some((param, click));
        }
    }
}

/// `links`: the document and the object shown, for rows that link another
/// object (none: such rows are left out).
fn object_tab(ui: &mut Ui, kind: &mut ObjectKind, k: &mut KeyRows, links: Option<(&Scene, NodeId)>) {
    section_heading(ui, "Object Properties");
    Grid::new("grid4d.attr.object")
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| match kind {
            ObjectKind::Null => {
                ui.label(
                    RichText::new("A Null groups objects. Drag objects onto it in the Object Manager.")
                        .color(p::TEXT_DIM),
                );
                ui.end_row();
            }
            ObjectKind::Cube { size, segments } => {
                for (axis, label) in ["Size . X", "Size . Y", "Size . Z"].into_iter().enumerate() {
                    length_row(ui, k, label, &mut size[axis]);
                }
                for (axis, label) in ["Segments X", "Segments Y", "Segments Z"].into_iter().enumerate() {
                    segments_row(ui, label, &mut segments[axis], 1);
                }
            }
            ObjectKind::Sphere { radius, segments } => {
                length_row(ui, k, "Radius", radius);
                segments_row(ui, "Segments", segments, 3);
            }
            ObjectKind::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            } => {
                length_row(ui, k, "Radius", radius);
                length_row(ui, k, "Height", height);
                segments_row(ui, "Height Segments", height_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            } => {
                length_row(ui, k, "Width", width);
                length_row(ui, k, "Height", depth);
                segments_row(ui, "Width Segments", width_segments, 1);
                segments_row(ui, "Height Segments", depth_segments, 1);
            }
            ObjectKind::Cone {
                bottom_radius,
                top_radius,
                height,
                rotation_segments,
                height_segments,
            } => {
                length_row(ui, k, "Top Radius", top_radius);
                length_row(ui, k, "Bottom Radius", bottom_radius);
                length_row(ui, k, "Height", height);
                segments_row(ui, "Height Segments", height_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            } => {
                length_row(ui, k, "Ring Radius", ring_radius);
                segments_row(ui, "Ring Segments", ring_segments, 3);
                length_row(ui, k, "Pipe Radius", pipe_radius);
                segments_row(ui, "Pipe Segments", pipe_segments, 3);
            }
            ObjectKind::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            } => {
                length_row(ui, k, "Inner Radius", inner_radius);
                length_row(ui, k, "Outer Radius", outer_radius);
                segments_row(ui, "Disc Segments", disc_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Capsule {
                radius,
                height,
                rotation_segments,
                height_segments,
                cap_segments,
            } => {
                length_row(ui, k, "Radius", radius);
                length_row(ui, k, "Height", height);
                segments_row(ui, "Height Segments", height_segments, 1);
                segments_row(ui, "Cap Segments", cap_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Camera { fov_y_deg } => {
                attr_label(ui, "Field of View (Vertical)", LABEL_WIDTH);
                ui.add_sized(
                    [FIELD_WIDTH, 19.0],
                    DragValue::new(fov_y_deg).speed(0.25).range(1.0..=170.0).suffix("°"),
                );
                k.button(ui, "Field of View (Vertical)");
                ui.end_row();
            }
            ObjectKind::Light {
                color,
                intensity,
                light_type,
                details,
                shadow,
            } => {
                attr_label(ui, "Color", LABEL_WIDTH);
                colour_button(ui, color);
                ui.end_row();
                attr_label(ui, "Intensity", LABEL_WIDTH);
                let mut percent = *intensity * 100.0;
                if ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut percent)
                            .speed(1.0)
                            .range(0.0..=10_000.0)
                            .suffix(" %"),
                    )
                    .changed()
                {
                    *intensity = percent / 100.0;
                }
                k.button(ui, "Intensity");
                ui.end_row();
                attr_label(ui, "Type", LABEL_WIDTH);
                egui::ComboBox::from_id_salt("grid4d.attr.light.type")
                    .selected_text(light_type.label())
                    .show_ui(ui, |ui| {
                        for option in LightType::ALL {
                            ui.selectable_value(light_type, option, option.label());
                        }
                    });
                ui.end_row();
                attr_label(ui, "Shadow", LABEL_WIDTH);
                egui::ComboBox::from_id_salt("grid4d.attr.light.shadow")
                    .selected_text(shadow.label())
                    .show_ui(ui, |ui| {
                        for option in LightShadow::ALL {
                            ui.selectable_value(shadow, option, option.label());
                        }
                    });
                ui.end_row();
                light_details(ui, k, *light_type, details);
            }
            ObjectKind::SubdivisionSurface {
                viewport_levels,
                render_levels,
                linear_pre,
                boundary,
            } => {
                attr_label(ui, "Type", LABEL_WIDTH);
                ui.label("Catmull-Clark (N-Gons)");
                ui.end_row();
                for (label, value) in [
                    ("Subdivision Viewport", viewport_levels),
                    ("Subdivision Renderer", render_levels),
                    ("Linear Pre-Subdivision", linear_pre),
                ] {
                    attr_label(ui, label, LABEL_WIDTH);
                    ui.add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(value)
                            .speed(0.05)
                            .range(0..=grid4d_geom::subdivision::MAX_LEVEL),
                    );
                    ui.end_row();
                }
                attr_label(ui, "Boundary Interpolation", LABEL_WIDTH);
                egui::ComboBox::from_id_salt("grid4d.attr.sds.boundary")
                    .selected_text(match boundary {
                        grid4d_geom::subdivision::Boundary::Edge => "Edge",
                        grid4d_geom::subdivision::Boundary::EdgeAndCorner => "Edge and Corner",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(boundary, grid4d_geom::subdivision::Boundary::Edge, "Edge");
                        ui.selectable_value(
                            boundary,
                            grid4d_geom::subdivision::Boundary::EdgeAndCorner,
                            "Edge and Corner",
                        );
                    });
                ui.end_row();
                ui.label("");
                ui.label(RichText::new("Smooths its first child (the cage)").color(p::TEXT_DIM));
                ui.end_row();
            }
            ObjectKind::Extrude {
                extrusion,
                hierarchical,
            } => extrude_rows(ui, k, extrusion, hierarchical),
            ObjectKind::Lathe(lathe) => lathe_rows(ui, k, lathe),
            ObjectKind::Bend(bend) => bend_rows(ui, k, bend),
            ObjectKind::Ffd {
                ffd,
                lattice,
                components,
            } => ffd_rows(ui, k, ffd, lattice, components),
            ObjectKind::Twist(twist) => {
                box_rows(ui, k, &mut twist.size, &mut twist.mode);
                angle_row(ui, k, "Angle", &mut twist.angle, -36_000.0..=36_000.0);
                fit_row(ui, k, "Twists its parent's subtree");
            }
            ObjectKind::Taper(taper) => {
                box_rows(ui, k, &mut taper.size, &mut taper.mode);
                strength_row(ui, k, &mut taper.strength);
                shape_rows(ui, k, &mut taper.shape);
                fit_row(ui, k, "Tapers its parent's subtree");
            }
            ObjectKind::Bulge(bulge) => {
                box_rows(ui, k, &mut bulge.size, &mut bulge.mode);
                strength_row(ui, k, &mut bulge.strength);
                shape_rows(ui, k, &mut bulge.shape);
                fit_row(ui, k, "Bulges its parent's subtree");
            }
            ObjectKind::Sweep(sweep) => sweep_rows(ui, k, sweep),
            ObjectKind::Loft(loft) => loft_rows(ui, loft),
            ObjectKind::Cloner {
                cloner,
                object,
                effectors,
            } => {
                cloner_rows(ui, k, cloner, object, links);
                if let Some((scene, _)) = links {
                    effector_list_rows(ui, effectors, scene);
                }
            }
            ObjectKind::PlainEffector(effector) => effector_rows(ui, k, effector, false),
            ObjectKind::RandomEffector(effector) => effector_rows(ui, k, effector, true),
            ObjectKind::SplinePrimitive(primitive) => spline_rows(ui, k, primitive),
            ObjectKind::Text(text) => text_rows(ui, k, text),
            ObjectKind::Spline {
                spline, interpolation, ..
            } => editable_spline_rows(ui, spline, interpolation),
            ObjectKind::Polygon { mesh, .. } => {
                // The geometry is edited in the viewport (Points / Edges / Polygons).
                let mesh = mesh.mesh();
                for (label, count) in [("Points", mesh.point_count()), ("Polygons", mesh.face_count())] {
                    attr_label(ui, label, LABEL_WIDTH);
                    ui.label(count.to_string());
                    ui.end_row();
                }
            }
        });
}

/// The active tool's options (editor state: no undo, not saved).
fn tool_ui(ui: &mut Ui, doc: &Document, editor: &mut EditorState) {
    let k = &mut KeyRows::default();
    let tool = editor.tool;
    ui.label(
        RichText::new(format!("{} [Tool]", tool.label()))
            .strong()
            .color(p::TEXT_STRONG),
    );
    section_heading(ui, "Options");
    let s = &mut editor.selection_settings;
    Grid::new("grid4d.attr.tool")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| match tool {
            Tool::LiveSelection => {
                attr_label(ui, "Radius", LABEL_WIDTH);
                ui.add_sized(
                    [FIELD_WIDTH, 19.0],
                    DragValue::new(&mut s.radius)
                        .speed(0.5)
                        .range(1.0..=1000.0)
                        .suffix(" px"),
                )
                .on_hover_text("Middle-drag in the viewport changes it too");
                ui.end_row();
                check_row(ui, "Only Select Visible Elements", &mut s.live_visible_only);
                check_row(ui, "Tolerant Selection", &mut s.live_tolerant);
            }
            Tool::RectangleSelection => {
                check_row(ui, "Only Select Visible Elements", &mut s.rectangle_visible_only);
                check_row(ui, "Tolerant Selection", &mut s.rectangle_tolerant);
            }
            Tool::LoopSelection => walk_rows(ui, &mut s.loop_options),
            Tool::RingSelection => walk_rows(ui, &mut s.ring_options),
            Tool::Move => {
                check_row(ui, "Tweak Mode", &mut editor.transform_settings.tweak);
                ui.label("");
                // Wrapped: a long label in a grid would widen the whole panel.
                ui.add(
                    egui::Label::new(
                        RichText::new("With nothing selected, drag a point, edge or polygon to move just it")
                            .color(p::TEXT_DIM),
                    )
                    .wrap(),
                );
                ui.end_row();
            }
            Tool::Scale | Tool::Rotate => {
                ui.label(RichText::new("This tool has no options yet.").color(p::TEXT_DIM));
                ui.end_row();
            }
            Tool::Extrude => {
                let m = &mut editor.modeling_settings;
                offset_row(ui, k, "Offset", &mut m.extrude.offset);
                count_row(ui, "Subdivision", &mut m.extrude.subdivision, 0);
                check_row(ui, "Create N-gons", &mut m.extrude.create_ngons);
                check_row(ui, "Create Caps", &mut m.extrude.create_caps);
                check_row(ui, "Preserve Groups", &mut m.extrude.preserve_groups);
                angle_row(ui, k, "Maximum Angle", &mut m.extrude.max_angle, 0.0..=180.0);
                angle_row(ui, k, "Rotation (Edges)", &mut m.edge_rotation, -180.0..=180.0);
            }
            Tool::Inset => {
                let m = &mut editor.modeling_settings;
                offset_row(ui, k, "Offset", &mut m.inset.offset);
                count_row(ui, "Subdivision", &mut m.inset.subdivision, 1);
                check_row(ui, "Create N-gons", &mut m.inset.create_ngons);
                check_row(ui, "Preserve Groups", &mut m.inset.preserve_groups);
                angle_row(ui, k, "Maximum Angle", &mut m.inset.max_angle, 0.0..=180.0);
            }
            Tool::LoopCut => {
                let m = &mut editor.modeling_settings.loop_cut;
                attr_label(ui, "Offset", LABEL_WIDTH);
                let mut percent = m.offset * 100.0;
                if ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut percent).speed(0.5).range(1.0..=99.0).suffix(" %"),
                    )
                    .on_hover_text("A click sets it from where the cursor is on the edge")
                    .changed()
                {
                    m.offset = percent / 100.0;
                }
                ui.end_row();
                count_row(ui, "Number of Cuts", &mut m.cuts, 1);
                check_row(ui, "Select Cuts", &mut m.select_cuts);
            }
            Tool::Bevel => {
                let m = &mut editor.modeling_settings.bevel;
                offset_row(ui, k, "Offset", &mut m.offset);
                count_row(ui, "Subdivision", &mut m.subdivision, 0);
                attr_label(ui, "Depth", LABEL_WIDTH);
                let mut percent = m.depth * 100.0;
                if ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut percent)
                            .speed(1.0)
                            .range(-100.0..=100.0)
                            .suffix(" %"),
                    )
                    .changed()
                {
                    m.depth = percent / 100.0;
                }
                ui.end_row();
                check_row(ui, "Limit", &mut m.limit);
            }
            Tool::Bridge => {
                let m = &mut editor.modeling_settings.bridge;
                count_row(ui, "Subdivision", &mut m.subdivision, 0);
                attr_label(ui, "Spin Steps", LABEL_WIDTH);
                ui.add_sized(
                    [FIELD_WIDTH, 19.0],
                    DragValue::new(&mut m.spin).speed(0.05).range(-64..=64),
                );
                ui.end_row();
                check_row(ui, "Delete Original Polygons", &mut m.delete_originals);
            }
            Tool::Weld => {
                ui.label(RichText::new("No options").color(p::TEXT_DIM));
                ui.end_row();
            }
            Tool::Pen => {
                attr_label(ui, "Type", LABEL_WIDTH);
                let kind = &mut editor.pen.kind;
                egui::ComboBox::from_id_salt("grid4d.attr.pen.type")
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for option in SplineType::ALL {
                            ui.selectable_value(kind, option, option.label());
                        }
                    })
                    .response
                    .on_hover_text("The type of a new spline; points added to a spline keep its type");
                ui.end_row();
            }
            Tool::LineCut => {
                let m = &mut editor.modeling_settings.line_cut;
                check_row(ui, "Visible Only", &mut m.visible_only);
                check_row(ui, "Infinite Cut", &mut m.infinite);
                check_row(ui, "Connect Cut Edges", &mut m.connect);
                check_row(ui, "Select Cuts", &mut m.select_cuts);
            }
        });
    if tool == Tool::Move {
        snap_section(ui, &mut editor.snap);
    }
    if tool == Tool::Bridge {
        ui.add_space(6.0);
        let target = crate::commands::component_target(doc, editor)
            .is_some_and(|(_, c)| matches!(c, grid4d_geom::Component::Polygon | grid4d_geom::Component::Edge));
        if ui
            .add_enabled(target, Button::new("Apply"))
            .on_hover_text("Bridge the two selected polygons or border edges")
            .on_disabled_hover_text(tool.hint())
            .clicked()
        {
            editor.commands.push(EditorCommand::BridgeSelected);
        }
        ui.label(RichText::new("Or drag from one polygon or border edge to another").color(p::TEXT_DIM));
    }
    if tool == Tool::Weld {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Click one of the selected points to weld onto it, anywhere else for their centre")
                .color(p::TEXT_DIM),
        );
    }
    if tool.is_cut_tool() {
        ui.add_space(6.0);
        let how = match tool {
            Tool::LoopCut => "Hover an edge to see the loop, click to cut it",
            _ => "Drag a line across the object; release cuts, Esc cancels",
        };
        ui.label(RichText::new(how).color(p::TEXT_DIM));
    }
    if tool.is_mesh_tool() {
        ui.add_space(6.0);
        let target = crate::commands::component_target(doc, editor)
            .is_some_and(|(_, component)| crate::modeling::works_in(tool, component));
        if ui
            .add_enabled(target, Button::new("Apply"))
            .on_hover_text("Run the tool once with these options on the selection (or on everything)")
            .on_disabled_hover_text(tool.hint())
            .clicked()
        {
            editor.commands.push(EditorCommand::ApplyMeshTool);
        }
        ui.label(RichText::new("Or drag left/right in the viewport: every drag is a new operation").color(p::TEXT_DIM));
    }
}

/// A tag's settings (Attribute Manager, Mode ▸ Tags); `owner` is the tagged object.
fn tag_ui(ui: &mut Ui, tag: &mut Tag, scene: &Scene, owner: NodeId) {
    let k = &mut KeyRows::default();
    section_heading(ui, "Tag Properties");
    Grid::new("grid4d.attr.tag")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| match tag {
            Tag::Phong { angle_limit, angle } => {
                check_row(ui, "Angle Limit", angle_limit);
                if *angle_limit {
                    angle_row(ui, k, "Phong Angle", angle, 0.0..=180.0);
                } else {
                    attr_label(ui, "Phong Angle", LABEL_WIDTH);
                    ui.label(RichText::new("no limit (180°)").color(p::TEXT_DIM));
                    ui.end_row();
                }
            }
            Tag::Protection {
                position,
                scale,
                rotation,
            } => {
                check_row(ui, "Position", position);
                check_row(ui, "Scale", scale);
                check_row(ui, "Rotation", rotation);
            }
            Tag::Target { target, up, pitch } => {
                object_row(ui, "Target Object", target, scene, owner);
                object_row(ui, "Up Vector", up, scene, owner);
                check_row(ui, "Pitch", pitch);
            }
            Tag::Material { material } => material_row(ui, material, scene),
        });
    let hint = match tag {
        Tag::Phong { .. } => "Edges sharper than the angle stay hard; with Angle Limit off everything is smooth",
        Tag::Protection { .. } => "Protected channels cannot be changed with the gizmo or the Coordinates fields",
        Tag::Target { .. } => "The object turns its forward axis (−Z) towards the target; Pitch off keeps it level",
        Tag::Material { .. } => {
            "The object and its children without a Material tag of their own show this material; drag a material onto an object to assign it"
        }
    };
    ui.add_space(4.0);
    ui.label(RichText::new(hint).color(p::TEXT_DIM));
}

/// A Material tag's link: a list of the document's materials, `None` first.
fn material_row(ui: &mut Ui, value: &mut Option<MaterialId>, scene: &Scene) {
    attr_label(ui, "Material", LABEL_WIDTH);
    let materials = scene.materials();
    let name = value
        .and_then(|id| materials.get(id))
        .map_or("None", |m| m.name.as_str());
    egui::ComboBox::from_id_salt("grid4d.attr.tag.material")
        .selected_text(name)
        .width(FIELD_WIDTH + 40.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "None");
            for (id, material) in materials.iter() {
                ui.selectable_value(value, Some(id), &material.name);
            }
        });
    ui.end_row();
}

/// A link to another object: a list of the document's objects, `None` first.
fn object_row(ui: &mut Ui, label: &str, value: &mut Option<NodeId>, scene: &Scene, owner: NodeId) {
    attr_label(ui, label, LABEL_WIDTH);
    let name = value
        .and_then(|id| scene.get(id))
        .map_or("None", |n| n.data.name.as_str());
    egui::ComboBox::from_id_salt(("grid4d.attr.tag.link", label))
        .selected_text(name)
        .width(FIELD_WIDTH + 40.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "None");
            // Built only while the list is open.
            scene.walk(&mut Vec::new(), |id, node, _, depth| {
                if id != owner {
                    let indent = "   ".repeat(usize::try_from(depth).unwrap_or(0));
                    ui.selectable_value(value, Some(id), format!("{indent}{}", node.data.name));
                }
                true
            });
        });
    ui.end_row();
}

/// Snapping options (Move tool; the same settings as the snap button's menu).
fn snap_section(ui: &mut Ui, snap: &mut crate::SnapSettings) {
    section_heading(ui, "Snap");
    Grid::new("grid4d.attr.snap")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            check_row(ui, "Enable Snapping", &mut snap.enabled);
            attr_label(ui, "Type", LABEL_WIDTH);
            egui::ComboBox::from_id_salt("grid4d.attr.snap.type")
                .selected_text(snap.kind.label())
                .show_ui(ui, |ui| {
                    for kind in crate::SnapType::ALL {
                        ui.selectable_value(&mut snap.kind, kind, kind.label());
                    }
                });
            ui.end_row();
            attr_label(ui, "Snap Radius", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut snap.radius)
                    .speed(0.2)
                    .range(0.1..=100.0)
                    .suffix(" px"),
            );
            ui.end_row();
            for kind in grid4d_render::SnapKind::ALL {
                check_row(ui, kind.label(), snap.targets.flag_mut(kind));
            }
        });
    ui.label(RichText::new("The gizmo's centre (or the tweaked element) snaps while you move it").color(p::TEXT_DIM));
}

/// A length that may be negative (a tool's offset).
fn offset_row(ui: &mut Ui, k: &mut KeyRows, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value).speed(0.5).suffix(" cm").max_decimals(2),
    );
    k.button(ui, label);
    ui.end_row();
}

fn count_row(ui: &mut Ui, label: &str, value: &mut u32, min: u32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized([FIELD_WIDTH, 19.0], DragValue::new(value).speed(0.05).range(min..=64));
    ui.end_row();
}

/// An angle stored in radians, edited in degrees.
fn angle_row(ui: &mut Ui, k: &mut KeyRows, label: &str, radians: &mut f32, degrees: std::ops::RangeInclusive<f32>) {
    attr_label(ui, label, LABEL_WIDTH);
    let mut value = radians.to_degrees();
    if ui
        .add_sized(
            [FIELD_WIDTH, 19.0],
            DragValue::new(&mut value).speed(0.5).range(degrees).suffix("°"),
        )
        .changed()
    {
        *radians = value.to_radians();
    }
    k.button(ui, label);
    ui.end_row();
}

/// A spline primitive's shape, then its plane, direction and intermediate points.
fn spline_rows(ui: &mut Ui, k: &mut KeyRows, primitive: &mut SplinePrimitive) {
    match &mut primitive.shape {
        SplineShape::Circle { radius, ellipse, ring } => {
            length_row(ui, k, "Radius", radius);
            let r = *radius;
            optional_length_row(ui, k, "Ellipse", "Radius Y", ellipse, r);
            optional_length_row(ui, k, "Ring", "Inner Radius", ring, r * 0.5);
        }
        SplineShape::Rectangle {
            width,
            height,
            rounding,
        } => {
            length_row(ui, k, "Width", width);
            length_row(ui, k, "Height", height);
            optional_length_row(ui, k, "Rounding", "Rounding Radius", rounding, 50.0);
        }
        SplineShape::Arc {
            kind,
            radius,
            inner_radius,
            start,
            end,
        } => {
            attr_label(ui, "Type", LABEL_WIDTH);
            egui::ComboBox::from_id_salt("grid4d.attr.arc.type")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for option in ArcType::ALL {
                        ui.selectable_value(kind, option, option.label());
                    }
                });
            ui.end_row();
            length_row(ui, k, "Radius", radius);
            if *kind == ArcType::Ring {
                length_row(ui, k, "Inner Radius", inner_radius);
            }
            angle_row(ui, k, "Start Angle", start, -360.0..=360.0);
            angle_row(ui, k, "End Angle", end, -360.0..=360.0);
        }
        SplineShape::NSide {
            radius,
            sides,
            rounding,
        } => {
            length_row(ui, k, "Radius", radius);
            bounded_count_row(ui, "Sides", sides, 3, SplineShape::MAX_SIDES);
            optional_length_row(ui, k, "Rounding", "Rounding Radius", rounding, 50.0);
        }
        SplineShape::Star {
            inner_radius,
            outer_radius,
            twist,
            points,
        } => {
            length_row(ui, k, "Inner Radius", inner_radius);
            length_row(ui, k, "Outer Radius", outer_radius);
            angle_row(ui, k, "Twist", twist, -360.0..=360.0);
            bounded_count_row(ui, "Points", points, 3, SplineShape::MAX_SIDES);
        }
        SplineShape::Helix {
            start_radius,
            start_angle,
            end_radius,
            end_angle,
            radial_bias,
            height,
            height_bias,
            subdivision,
        } => {
            length_row(ui, k, "Start Radius", start_radius);
            angle_row(ui, k, "Start Angle", start_angle, -36_000.0..=36_000.0);
            length_row(ui, k, "End Radius", end_radius);
            angle_row(ui, k, "End Angle", end_angle, -36_000.0..=36_000.0);
            percent_row(ui, k, "Radial Bias", radial_bias);
            length_row(ui, k, "Height", height);
            percent_row(ui, k, "Height Bias", height_bias);
            bounded_count_row(ui, "Subdivision", subdivision, 3, SplineShape::MAX_SUBDIVISION);
        }
        SplineShape::Flower {
            inner_radius,
            outer_radius,
            petals,
        } => {
            length_row(ui, k, "Inner Radius", inner_radius);
            length_row(ui, k, "Outer Radius", outer_radius);
            bounded_count_row(ui, "Petals", petals, 1, SplineShape::MAX_SIDES);
        }
    }
    placement_rows(
        ui,
        &mut primitive.plane,
        &mut primitive.reverse,
        &mut primitive.interpolation,
    );
}

/// A Text spline's parameters: the text itself (several lines), its size,
/// alignment and spacing, then plane, direction and sampling.
fn text_rows(ui: &mut Ui, k: &mut KeyRows, text: &mut Text) {
    attr_label(ui, "Text", LABEL_WIDTH);
    let mut written = text.text.to_string();
    if ui
        .add(
            TextEdit::multiline(&mut written)
                .id_salt("grid4d.attr.text.text")
                .desired_rows(2)
                .desired_width(FIELD_WIDTH + 40.0),
        )
        .changed()
    {
        text.text = written.chars().take(Text::MAX_CHARS).collect::<String>().into();
    }
    ui.end_row();
    attr_label(ui, "Font", LABEL_WIDTH);
    ui.label(grid4d_text::FONT_NAME);
    ui.end_row();
    attr_label(ui, "Align", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.text.align")
        .selected_text(text.align.label())
        .show_ui(ui, |ui| {
            for align in TextAlign::ALL {
                ui.selectable_value(&mut text.align, align, align.label());
            }
        });
    ui.end_row();
    length_row(ui, k, "Height", &mut text.height);
    offset_row(ui, k, "Horizontal Spacing", &mut text.horizontal_spacing);
    offset_row(ui, k, "Vertical Spacing", &mut text.vertical_spacing);
    placement_rows(ui, &mut text.plane, &mut text.reverse, &mut text.interpolation);
}

/// Plane, Reverse and the intermediate points of a spline primitive or text.
fn placement_rows(ui: &mut Ui, plane: &mut SplinePlane, reverse: &mut bool, interpolation: &mut Interpolation) {
    attr_label(ui, "Plane", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.spline.plane")
        .selected_text(plane.label())
        .show_ui(ui, |ui| {
            for option in SplinePlane::ALL {
                ui.selectable_value(plane, option, option.label());
            }
        });
    ui.end_row();
    check_row(ui, "Reverse", reverse);
    interpolation_rows(ui, interpolation);
}

/// An editable spline: its type, closing and sampling; the points themselves are
/// edited in the viewport (Points mode).
fn editable_spline_rows(ui: &mut Ui, spline: &mut SplineHandle, interpolation: &mut Interpolation) {
    let current = spline.spline().kind;
    attr_label(ui, "Type", LABEL_WIDTH);
    let mut kind = current;
    egui::ComboBox::from_id_salt("grid4d.attr.spline.type")
        .selected_text(kind.label())
        .show_ui(ui, |ui| {
            for option in SplineType::ALL {
                ui.selectable_value(&mut kind, option, option.label());
            }
        });
    ui.end_row();
    if kind != current {
        *spline = spline.edited(|s| {
            s.kind = kind;
            // A spline turned into Bezier gets smooth handles where it has none.
            if kind == SplineType::Bezier {
                s.smooth_tangents();
            }
        });
    }
    let segments = &spline.spline().segments;
    let all_closed = !segments.is_empty() && segments.iter().all(|s| s.closed);
    let mut closed = all_closed;
    attr_label(ui, "Close Spline", LABEL_WIDTH);
    if ui.checkbox(&mut closed, "").changed() {
        *spline = spline.edited(|s| {
            for segment in &mut s.segments {
                segment.closed = closed;
            }
        });
    }
    ui.end_row();
    interpolation_rows(ui, interpolation);
    let s = spline.spline();
    for (label, count) in [("Points", s.points.len()), ("Segments", s.segments.len())] {
        attr_label(ui, label, LABEL_WIDTH);
        ui.label(count.to_string());
        ui.end_row();
    }
}

/// Intermediate Points and the value its mode uses.
fn interpolation_rows(ui: &mut Ui, interpolation: &mut Interpolation) {
    // Sampling settings are not animatable: no key buttons.
    let k = &mut KeyRows::default();
    const MODES: [&str; 5] = ["None", "Natural", "Uniform", "Adaptive", "Subdivided"];
    let current = match interpolation {
        Interpolation::None => 0,
        Interpolation::Natural { .. } => 1,
        Interpolation::Uniform { .. } => 2,
        Interpolation::Adaptive { .. } => 3,
        Interpolation::Subdivided { .. } => 4,
    };
    let (number, angle, length) = match *interpolation {
        Interpolation::Natural { number } | Interpolation::Uniform { number } => (number, 5f32.to_radians(), 5.0),
        Interpolation::Adaptive { angle } => (8, angle, 5.0),
        Interpolation::Subdivided { angle, max_length } => (8, angle, max_length),
        Interpolation::None => (8, 5f32.to_radians(), 5.0),
    };
    attr_label(ui, "Intermediate Points", LABEL_WIDTH);
    let mut chosen = current;
    egui::ComboBox::from_id_salt("grid4d.attr.spline.points")
        .selected_text(MODES[current])
        .show_ui(ui, |ui| {
            for (index, label) in MODES.into_iter().enumerate() {
                ui.selectable_value(&mut chosen, index, label);
            }
        });
    ui.end_row();
    if chosen != current {
        *interpolation = match chosen {
            0 => Interpolation::None,
            1 => Interpolation::Natural { number },
            2 => Interpolation::Uniform { number },
            3 => Interpolation::Adaptive { angle },
            _ => Interpolation::Subdivided {
                angle,
                max_length: length,
            },
        };
    }
    match interpolation {
        Interpolation::None => {}
        Interpolation::Natural { number } | Interpolation::Uniform { number } => {
            bounded_count_row(ui, "Number", number, 0, Interpolation::MAX_NUMBER);
        }
        Interpolation::Adaptive { angle } => angle_row(ui, k, "Angle", angle, 0.1..=90.0),
        Interpolation::Subdivided { angle, max_length } => {
            angle_row(ui, k, "Angle", angle, 0.1..=90.0);
            attr_label(ui, "Maximum Length", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(max_length)
                    .speed(0.5)
                    .range(Interpolation::MIN_LENGTH..=1_000_000.0)
                    .suffix(" cm")
                    .max_decimals(2),
            );
            ui.end_row();
        }
    }
}

/// A checkbox turning an optional length on (at `fallback`) or off, and the
/// length's own row while on.
fn optional_length_row(
    ui: &mut Ui,
    k: &mut KeyRows,
    label: &str,
    length_label: &str,
    value: &mut Option<f32>,
    fallback: f32,
) {
    let mut on = value.is_some();
    attr_label(ui, label, LABEL_WIDTH);
    if ui.checkbox(&mut on, "").changed() {
        *value = on.then_some(fallback);
    }
    ui.end_row();
    if let Some(length) = value {
        length_row(ui, k, length_label, length);
    }
}

fn bounded_count_row(ui: &mut Ui, label: &str, value: &mut u32, min: u32, max: u32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized([FIELD_WIDTH, 19.0], DragValue::new(value).speed(0.1).range(min..=max));
    ui.end_row();
}

/// A 0…1 value shown as 0…100 %.
fn percent_row(ui: &mut Ui, k: &mut KeyRows, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    let mut percent = *value * 100.0;
    if ui
        .add_sized(
            [FIELD_WIDTH, 19.0],
            DragValue::new(&mut percent).speed(0.5).range(0.0..=100.0).suffix(" %"),
        )
        .changed()
    {
        *value = percent / 100.0;
    }
    k.button(ui, label);
    ui.end_row();
}

/// The Extrude generator's Object and Caps parameters.
fn extrude_rows(ui: &mut Ui, k: &mut KeyRows, extrusion: &mut Extrusion, hierarchical: &mut bool) {
    attr_label(ui, "Direction", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.extrude.direction")
        .selected_text(extrusion.direction.label())
        .show_ui(ui, |ui| {
            for option in ExtrudeDirection::ALL {
                ui.selectable_value(&mut extrusion.direction, option, option.label());
            }
        });
    ui.end_row();
    if extrusion.direction != ExtrudeDirection::Absolute {
        offset_row(ui, k, "Offset", &mut extrusion.offset);
    }
    if matches!(
        extrusion.direction,
        ExtrudeDirection::Custom | ExtrudeDirection::Absolute
    ) {
        for (axis, label) in ["Movement . X", "Movement . Y", "Movement . Z"].into_iter().enumerate() {
            offset_row(ui, k, label, &mut extrusion.movement[axis]);
        }
    }
    attr_label(ui, "Subdivision", LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(&mut extrusion.subdivision)
            .speed(0.1)
            .range(1..=Extrusion::MAX_SUBDIVISION),
    );
    ui.end_row();
    check_row(ui, "Reverse Normals", &mut extrusion.reverse_normals);
    check_row(ui, "Hierarchical", hierarchical);
    ui.label("");
    ui.label(RichText::new("Caps").strong());
    ui.end_row();
    cap_rows(
        ui,
        &mut extrusion.start_cap,
        &mut extrusion.end_cap,
        &mut extrusion.cap_type,
    );
    ui.label("");
    ui.label(RichText::new("Extrudes its first child spline").color(p::TEXT_DIM));
    ui.end_row();
}

/// The Lathe generator's Object and Caps parameters.
fn lathe_rows(ui: &mut Ui, k: &mut KeyRows, lathe: &mut Lathe) {
    angle_row(ui, k, "Angle", &mut lathe.angle, -36_000.0..=36_000.0);
    attr_label(ui, "Subdivision", LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(&mut lathe.subdivision)
            .speed(0.1)
            .range(Lathe::MIN_SUBDIVISION..=Lathe::MAX_SUBDIVISION),
    );
    ui.end_row();
    offset_row(ui, k, "Movement", &mut lathe.movement);
    scale_row(ui, k, "Scaling", &mut lathe.scaling);
    check_row(ui, "Reverse Normals", &mut lathe.reverse_normals);
    ui.label("");
    ui.label(RichText::new("Caps").strong());
    ui.end_row();
    cap_rows(ui, &mut lathe.start_cap, &mut lathe.end_cap, &mut lathe.cap_type);
    ui.label("");
    ui.label(RichText::new("Turns its first child spline around Y").color(p::TEXT_DIM));
    ui.end_row();
}

/// The Sweep generator's Object and Caps parameters.
fn sweep_rows(ui: &mut Ui, k: &mut KeyRows, sweep: &mut Sweep) {
    scale_row(ui, k, "End Scale", &mut sweep.end_scale);
    angle_row(ui, k, "End Rotation", &mut sweep.end_rotation, -36_000.0..=36_000.0);
    percent_row(ui, k, "Start Growth", &mut sweep.start_growth);
    percent_row(ui, k, "End Growth", &mut sweep.end_growth);
    check_row(ui, "Parallel Movement", &mut sweep.parallel_movement);
    check_row(ui, "Constant Cross Section", &mut sweep.constant_cross_section);
    check_row(ui, "Banking", &mut sweep.banking);
    check_row(ui, "Reverse Normals", &mut sweep.reverse_normals);
    ui.label("");
    ui.label(RichText::new("Caps").strong());
    ui.end_row();
    cap_rows(ui, &mut sweep.start_cap, &mut sweep.end_cap, &mut sweep.cap_type);
    ui.label("");
    ui.label(RichText::new("Child 1: profile; child 2: path").color(p::TEXT_DIM));
    ui.end_row();
}

/// A Cloner's mode and its layout's values, which child each clone repeats and
/// Reset Coordinates.
fn cloner_rows(
    ui: &mut Ui,
    k: &mut KeyRows,
    cloner: &mut Cloner,
    object: &mut Option<NodeId>,
    links: Option<(&Scene, NodeId)>,
) {
    attr_label(ui, "Mode", LABEL_WIDTH);
    let mut index = cloner.layout.index();
    egui::ComboBox::from_id_salt("grid4d.attr.cloner.mode")
        .selected_text(cloner.layout.label())
        .show_ui(ui, |ui| {
            for (i, label) in Layout::LABELS.into_iter().enumerate() {
                ui.selectable_value(&mut index, i, label);
            }
        });
    ui.end_row();
    if index != cloner.layout.index() {
        cloner.layout = Layout::of_index(index);
    }
    attr_label(ui, "Clones", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.cloner.clones")
        .selected_text(cloner.clones.label())
        .show_ui(ui, |ui| {
            for option in CloneOrder::ALL {
                ui.selectable_value(&mut cloner.clones, option, option.label());
            }
        });
    ui.end_row();
    if cloner.clones == CloneOrder::Random {
        bounded_count_row(ui, "Seed", &mut cloner.seed, 0, 9_999_999);
    }
    check_row(ui, "Reset Coordinates", &mut cloner.reset_coordinates);
    let max = Cloner::MAX_COUNT;
    match &mut cloner.layout {
        Layout::Linear(linear) => {
            bounded_count_row(ui, "Count", &mut linear.count, 0, max);
            bounded_count_row(ui, "Offset", &mut linear.offset, 0, max);
            step_mode_row(ui, &mut linear.mode);
            for (axis, label) in ["P . X", "P . Y", "P . Z"].into_iter().enumerate() {
                offset_row(ui, k, label, &mut linear.position[axis]);
            }
            for (axis, label) in ["S . X", "S . Y", "S . Z"].into_iter().enumerate() {
                scale_row(ui, k, label, &mut linear.scale[axis]);
            }
            for (axis, label) in ["R . H", "R . P", "R . B"].into_iter().enumerate() {
                angle_row(ui, k, label, &mut linear.rotation[axis], -36_000.0..=36_000.0);
            }
        }
        Layout::Radial(radial) => {
            bounded_count_row(ui, "Count", &mut radial.count, 0, max);
            length_row(ui, k, "Radius", &mut radial.radius);
            attr_label(ui, "Plane", LABEL_WIDTH);
            egui::ComboBox::from_id_salt("grid4d.attr.cloner.plane")
                .selected_text(radial.plane.label())
                .show_ui(ui, |ui| {
                    for option in SplinePlane::ALL {
                        ui.selectable_value(&mut radial.plane, option, option.label());
                    }
                });
            ui.end_row();
            check_row(ui, "Align", &mut radial.align);
            angle_row(ui, k, "Start Angle", &mut radial.start_angle, -36_000.0..=36_000.0);
            angle_row(ui, k, "End Angle", &mut radial.end_angle, -36_000.0..=36_000.0);
            angle_row(ui, k, "Offset", &mut radial.offset, -36_000.0..=36_000.0);
        }
        Layout::Grid(grid) => {
            for (axis, label) in ["Count . X", "Count . Y", "Count . Z"].into_iter().enumerate() {
                bounded_count_row(ui, label, &mut grid.count[axis], 1, max);
            }
            step_mode_row(ui, &mut grid.mode);
            for (axis, label) in ["Size . X", "Size . Y", "Size . Z"].into_iter().enumerate() {
                length_row(ui, k, label, &mut grid.size[axis]);
            }
        }
        Layout::Object(surface) => {
            if let Some((scene, owner)) = links {
                object_row(ui, "Object", object, scene, owner);
            }
            attr_label(ui, "Distribution", LABEL_WIDTH);
            egui::ComboBox::from_id_salt("grid4d.attr.cloner.distribution")
                .selected_text(surface.distribution.label())
                .show_ui(ui, |ui| {
                    for option in Distribution::ALL {
                        ui.selectable_value(&mut surface.distribution, option, option.label());
                    }
                });
            ui.end_row();
            check_row(ui, "Align Clone", &mut surface.align);
        }
    }
    ui.label("");
    ui.label(RichText::new("Clones repeat its children").color(p::TEXT_DIM));
    ui.end_row();
}

/// A Cloner's Effectors: the linked ones (each with a remove button) and a
/// list of the document's other effectors to add.
fn effector_list_rows(ui: &mut Ui, effectors: &mut Vec<NodeId>, scene: &Scene) {
    let mut remove = None;
    for (index, &id) in effectors.iter().enumerate() {
        attr_label(ui, if index == 0 { "Effectors" } else { "" }, LABEL_WIDTH);
        let name = scene.get(id).map_or("(deleted)", |n| n.data.name.as_str());
        ui.horizontal(|ui| {
            ui.label(name);
            if ui.small_button("✕").on_hover_text("Remove from the list").clicked() {
                remove = Some(index);
            }
        });
        ui.end_row();
    }
    if let Some(index) = remove {
        effectors.remove(index);
    }
    attr_label(ui, if effectors.is_empty() { "Effectors" } else { "" }, LABEL_WIDTH);
    let mut added = None;
    egui::ComboBox::from_id_salt("grid4d.attr.cloner.add_effector")
        .selected_text("Add…")
        .width(FIELD_WIDTH + 40.0)
        .show_ui(ui, |ui| {
            // Built only while the list is open.
            scene.walk(&mut Vec::new(), |id, node, _, _| {
                if node.data.kind.is_effector() && !effectors.contains(&id) && ui.button(&node.data.name).clicked() {
                    added = Some(id);
                }
                true
            });
        });
    ui.end_row();
    if let Some(id) = added {
        effectors.push(id);
    }
}

/// A Plain or Random effector: Strength, the changes it makes, its colour and
/// falloff.
fn effector_rows(ui: &mut Ui, k: &mut KeyRows, effector: &mut Effector, random: bool) {
    strength_row(ui, k, &mut effector.strength);
    if random {
        bounded_count_row(ui, "Seed", &mut effector.seed, 0, 9_999_999);
    }
    check_row(ui, "Position", &mut effector.use_position);
    if effector.use_position {
        for (axis, label) in ["P . X", "P . Y", "P . Z"].into_iter().enumerate() {
            offset_row(ui, k, label, &mut effector.position[axis]);
        }
    }
    check_row(ui, "Scale", &mut effector.use_scale);
    if effector.use_scale {
        check_row(ui, "Uniform Scale", &mut effector.uniform_scale);
        let axes: &[(usize, &str)] = if effector.uniform_scale {
            &[(0, "Scale")]
        } else {
            &[(0, "S . X"), (1, "S . Y"), (2, "S . Z")]
        };
        for &(axis, label) in axes {
            signed_percent_row(ui, label, &mut effector.scale[axis]);
        }
    }
    check_row(ui, "Rotation", &mut effector.use_rotation);
    if effector.use_rotation {
        for (axis, label) in ["R . H", "R . P", "R . B"].into_iter().enumerate() {
            angle_row(ui, k, label, &mut effector.rotation[axis], -36_000.0..=36_000.0);
        }
    }
    check_row(ui, "Color", &mut effector.use_color);
    if effector.use_color && !random {
        attr_label(ui, "Effector Color", LABEL_WIDTH);
        colour_button(ui, &mut effector.color);
        ui.end_row();
    }
    let falloff = &mut effector.falloff;
    attr_label(ui, "Falloff", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.effector.falloff")
        .selected_text(falloff.shape.label())
        .show_ui(ui, |ui| {
            for option in FalloffShape::ALL {
                ui.selectable_value(&mut falloff.shape, option, option.label());
            }
        });
    ui.end_row();
    if falloff.shape != FalloffShape::Infinite {
        for (axis, label) in ["Size . X", "Size . Y", "Size . Z"].into_iter().enumerate() {
            length_row(ui, k, label, &mut falloff.size[axis]);
        }
        if falloff.shape != FalloffShape::Linear {
            percent_row(ui, k, "Soft Edge", &mut falloff.soft);
        }
    }
    ui.label("");
    ui.label(RichText::new("Add it to a Cloner's Effectors").color(p::TEXT_DIM));
    ui.end_row();
}

/// A change in percent that may be negative (a scale change: −50 % halves).
fn signed_percent_row(ui: &mut Ui, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    let mut percent = *value * 100.0;
    if ui
        .add_sized(
            [FIELD_WIDTH, 19.0],
            DragValue::new(&mut percent)
                .speed(0.5)
                .range(-100.0..=10_000.0)
                .suffix(" %"),
        )
        .changed()
    {
        *value = percent / 100.0;
    }
    ui.end_row();
}

/// Whether Linear and Grid Array values count per step or to the end.
fn step_mode_row(ui: &mut Ui, mode: &mut StepMode) {
    attr_label(ui, "Spacing", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.cloner.step")
        .selected_text(mode.label())
        .show_ui(ui, |ui| {
            for option in StepMode::ALL {
                ui.selectable_value(mode, option, option.label());
            }
        });
    ui.end_row();
}

/// Every deformer's box: its Size and Mode.
fn box_rows(ui: &mut Ui, k: &mut KeyRows, size: &mut glam::Vec3, mode: &mut DeformerMode) {
    for (axis, label) in ["Size . X", "Size . Y", "Size . Z"].into_iter().enumerate() {
        length_row(ui, k, label, &mut size[axis]);
    }
    attr_label(ui, "Mode", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.deformer.mode")
        .selected_text(mode.label())
        .show_ui(ui, |ui| {
            for option in DeformerMode::ALL {
                ui.selectable_value(mode, option, option.label());
            }
        });
    ui.end_row();
}

/// A Taper's or Bulge's strength, in percent (negative values allowed).
fn strength_row(ui: &mut Ui, k: &mut KeyRows, value: &mut f32) {
    attr_label(ui, "Strength", LABEL_WIDTH);
    let mut shown = *value * 100.0;
    if ui
        .add_sized(
            [FIELD_WIDTH, 19.0],
            DragValue::new(&mut shown)
                .speed(0.5)
                .range(-10_000.0..=10_000.0)
                .suffix(" %"),
        )
        .changed()
    {
        *value = shown / 100.0;
    }
    k.button(ui, "Strength");
    ui.end_row();
}

/// A Taper's or Bulge's Curvature and Fillet.
fn shape_rows(ui: &mut Ui, k: &mut KeyRows, shape: &mut Shape) {
    scale_row(ui, k, "Curvature", &mut shape.curvature);
    check_row(ui, "Fillet", &mut shape.fillet);
}

/// The Fit to Parent button and a short hint.
fn fit_row(ui: &mut Ui, k: &mut KeyRows, hint: &str) {
    ui.label("");
    if ui
        .button("Fit to Parent")
        .on_hover_text("Size the box to the parent and centre it there")
        .clicked()
    {
        k.fit_to_parent = true;
    }
    ui.end_row();
    ui.label("");
    ui.label(RichText::new(hint).color(p::TEXT_DIM));
    ui.end_row();
}

/// The FFD's box, grid and Reset Lattice: Size scales the lattice, a new grid
/// (or Reset) puts it back on its regular grid, its selection cleared.
fn ffd_rows(
    ui: &mut Ui,
    k: &mut KeyRows,
    ffd: &mut Ffd,
    lattice: &mut SplineHandle,
    components: &mut grid4d_scene::Components,
) {
    let (size, grid) = (ffd.size, ffd.grid);
    box_rows(ui, k, &mut ffd.size, &mut ffd.mode);
    for (axis, label) in ["Grid Points X", "Grid Points Y", "Grid Points Z"]
        .into_iter()
        .enumerate()
    {
        bounded_count_row(ui, label, &mut ffd.grid[axis], Ffd::MIN_GRID, Ffd::MAX_GRID);
    }
    ui.label("");
    let reset = ui
        .button("Reset Lattice")
        .on_hover_text("Put every lattice point back on its grid")
        .clicked();
    ui.end_row();
    if reset || ffd.grid != grid {
        *lattice = SplineHandle::new(grid4d_geom::spline::Spline::new(
            SplineType::Linear,
            ffd.grid_points(),
            false,
        ));
        *components = grid4d_scene::Components::default();
    } else if ffd.size != size {
        let points = grid4d_geom::deform::scale_lattice(&lattice.spline().points, size, ffd.size);
        *lattice = lattice.edited(|s| s.points = points);
    }
    fit_row(ui, k, "Points mode edits the lattice");
}

/// The Bend deformer's parameters and its Fit to Parent button.
fn bend_rows(ui: &mut Ui, k: &mut KeyRows, bend: &mut Bend) {
    box_rows(ui, k, &mut bend.size, &mut bend.mode);
    angle_row(ui, k, "Strength", &mut bend.strength, -36_000.0..=36_000.0);
    angle_row(ui, k, "Angle", &mut bend.angle, -36_000.0..=36_000.0);
    check_row(ui, "Keep Y-Axis Length", &mut bend.keep_length);
    attr_label(ui, "Origin", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.bend.origin")
        .selected_text(bend.origin.label())
        .show_ui(ui, |ui| {
            for origin in BendOrigin::ALL {
                ui.selectable_value(&mut bend.origin, origin, origin.label());
            }
        });
    ui.end_row();
    fit_row(ui, k, "Bends its parent's subtree");
}

/// The Loft generator's Object and Caps parameters.
fn loft_rows(ui: &mut Ui, loft: &mut Loft) {
    for (label, value) in [
        ("Subdivision U", &mut loft.subdivision_u),
        ("Subdivision V", &mut loft.subdivision_v),
    ] {
        bounded_count_row(ui, label, value, Loft::MIN_SUBDIVISION, Loft::MAX_SUBDIVISION);
    }
    check_row(ui, "Organic Form", &mut loft.organic);
    check_row(ui, "Per Segment", &mut loft.per_segment);
    check_row(ui, "Loop", &mut loft.looped);
    check_row(ui, "Linear Interpolation", &mut loft.linear);
    check_row(ui, "Reverse Normals", &mut loft.reverse_normals);
    ui.label("");
    ui.label(RichText::new("Caps").strong());
    ui.end_row();
    cap_rows(ui, &mut loft.start_cap, &mut loft.end_cap, &mut loft.cap_type);
    ui.label("");
    ui.label(RichText::new("A skin over its child splines").color(p::TEXT_DIM));
    ui.end_row();
}

/// A scale factor (1.0 = 100 %, never negative) edited in percent.
fn scale_row(ui: &mut Ui, k: &mut KeyRows, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    let mut shown = *value * 100.0;
    if ui
        .add_sized(
            [FIELD_WIDTH, 19.0],
            DragValue::new(&mut shown)
                .speed(0.5)
                .range(0.0..=100_000.0)
                .suffix(" %"),
        )
        .changed()
    {
        *value = shown / 100.0;
    }
    k.button(ui, label);
    ui.end_row();
}

/// Start / End Cap and Caps Type (Extrude, Lathe, Sweep, Loft).
fn cap_rows(ui: &mut Ui, start: &mut bool, end: &mut bool, cap_type: &mut CapType) {
    check_row(ui, "Start Cap", start);
    check_row(ui, "End Cap", end);
    attr_label(ui, "Caps Type", LABEL_WIDTH);
    egui::ComboBox::from_id_salt("grid4d.attr.caps.type")
        .selected_text(cap_type.label())
        .show_ui(ui, |ui| {
            for option in CapType::ALL {
                ui.selectable_value(cap_type, option, option.label());
            }
        });
    ui.end_row();
}

fn check_row(ui: &mut Ui, label: &str, value: &mut bool) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.checkbox(value, "");
    ui.end_row();
}

fn walk_rows(ui: &mut Ui, options: &mut WalkOptions) {
    check_row(ui, "Stop at Selections", &mut options.stop_at_selections);
    check_row(ui, "Stop at Non-Quads", &mut options.stop_at_non_quads);
    check_row(ui, "Stop at Poles", &mut options.stop_at_poles);
}

fn project_settings(ui: &mut Ui, doc: &mut Document) {
    ui.label(
        RichText::new(format!("Project [{}]", doc.name))
            .strong()
            .color(p::TEXT_STRONG),
    );
    section_heading(ui, "Project Settings");
    let time = &mut doc.time;
    Grid::new("grid4d.attr.project")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Name", LABEL_WIDTH);
            ui.add(TextEdit::singleline(&mut doc.name).desired_width(f32::INFINITY));
            ui.end_row();

            attr_label(ui, "FPS", LABEL_WIDTH);
            let mut fps = time.fps();
            if ui
                .add_sized([FIELD_WIDTH, 19.0], DragValue::new(&mut fps).range(1..=240))
                .changed()
            {
                time.set_fps(fps);
            }
            ui.end_row();

            let (mut start, mut end) = (time.start(), time.end());
            attr_label(ui, "Minimum Time", LABEL_WIDTH);
            let changed_start = ui
                .add_sized([FIELD_WIDTH, 19.0], DragValue::new(&mut start).suffix(" F"))
                .changed();
            ui.end_row();
            attr_label(ui, "Maximum Time", LABEL_WIDTH);
            let changed_end = ui
                .add_sized([FIELD_WIDTH, 19.0], DragValue::new(&mut end).suffix(" F"))
                .changed();
            ui.end_row();
            if changed_start || changed_end {
                time.set_range(start, end);
            }

            attr_label(ui, "Loop Playback", LABEL_WIDTH);
            ui.checkbox(&mut time.looping, "");
            ui.end_row();

            attr_label(ui, "Objects", LABEL_WIDTH);
            ui.label(doc.scene.len().to_string());
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.label(RichText::new("Select an object to edit its attributes.").color(p::TEXT_DIM));
}

/// A segment count, from `min` up to what the geometry generators accept.
/// A colour swatch with its picker. The picker converts the colour on every
/// frame (0.9 comes back as 0.90000004), so the value is written back only when
/// the user changes it: showing a colour must not edit the document.
fn colour_button(ui: &mut Ui, colour: &mut [f32; 3]) -> egui::Response {
    let mut shown = *colour;
    let response = ui.color_edit_button_rgb(&mut shown);
    if response.changed() {
        *colour = shown;
    }
    response
}

/// A light's Details: the cone of a spot, the falloff of a light with a position.
fn light_details(ui: &mut Ui, k: &mut KeyRows, light_type: LightType, details: &mut LightDetails) {
    if light_type == LightType::Spot {
        attr_label(ui, "Use Inner", LABEL_WIDTH);
        ui.checkbox(&mut details.use_inner, "");
        ui.end_row();
        let use_inner = details.use_inner;
        for (label, value, enabled) in [
            ("Inner Angle", &mut details.inner_angle_deg, use_inner),
            ("Outer Angle", &mut details.outer_angle_deg, true),
        ] {
            attr_label(ui, label, LABEL_WIDTH);
            ui.add_enabled(
                enabled,
                DragValue::new(value)
                    .speed(0.25)
                    .range(0.0..=LightDetails::MAX_ANGLE_DEG)
                    .suffix("°"),
            );
            k.button(ui, label);
            ui.end_row();
        }
    }
    if light_type.has_position() {
        attr_label(ui, "Falloff", LABEL_WIDTH);
        egui::ComboBox::from_id_salt("grid4d.attr.light.falloff")
            .selected_text(details.falloff.label())
            .show_ui(ui, |ui| {
                for option in Falloff::ALL {
                    ui.selectable_value(&mut details.falloff, option, option.label());
                }
            });
        ui.end_row();
        if details.falloff != Falloff::None {
            attr_label(ui, "Radius/Decay", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut details.radius)
                    .speed(1.0)
                    .range(LightDetails::MIN_RADIUS..=1_000_000.0)
                    .suffix(" cm"),
            );
            k.button(ui, "Radius/Decay");
            ui.end_row();
        }
    }
}

fn segments_row(ui: &mut Ui, label: &str, value: &mut u32, min: u32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value).speed(0.1).range(min..=MAX_SEGMENTS),
    );
    ui.end_row();
}

fn length_row(ui: &mut Ui, k: &mut KeyRows, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value)
            .speed(1.0)
            .range(0.0..=1_000_000.0)
            .suffix(" cm")
            .max_decimals(2),
    );
    k.button(ui, label);
    ui.end_row();
}

fn small_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(p::TEXT_DIM).size(11.5));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Showing an object's parameters must not change them.
    #[test]
    fn showing_light_parameters_changes_nothing() {
        let ctx = egui::Context::default();
        for light_type in LightType::ALL {
            for falloff in Falloff::ALL {
                let mut data = NodeData::new(
                    "Light",
                    ObjectKind::Light {
                        color: [1.0, 0.9, 0.75],
                        intensity: 1.6,
                        light_type,
                        details: LightDetails {
                            inner_angle_deg: 20.0,
                            outer_angle_deg: 70.0,
                            falloff,
                            ..LightDetails::default()
                        },
                        shadow: LightShadow::ShadowMaps,
                    },
                );
                let before = data.clone();
                for _ in 0..3 {
                    ctx.run_ui(egui::RawInput::default(), |ui| {
                        object_tab(ui, &mut data.kind, &mut KeyRows::default(), None);
                    })
                    .drop_without_applying_deltas();
                }
                assert_eq!(data, before, "{light_type:?} with {falloff:?}");
            }
        }
    }

    #[test]
    fn showing_a_colour_keeps_it_exactly() {
        let ctx = egui::Context::default();
        let mut colour = [0.9, 0.62, 0.64];
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                colour_button(ui, &mut colour);
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(colour, [0.9, 0.62, 0.64]);
    }

    /// Every animatable parameter of every object has exactly one key button
    /// on the Object tab (labels map rows to parameters, so this guards them).
    #[test]
    fn every_animatable_parameter_has_one_key_button() {
        use grid4d_geom::spline::SplineShape;
        let mut kinds: Vec<ObjectKind> = grid4d_scene::ObjectKindTag::ALL
            .into_iter()
            .map(grid4d_scene::ObjectKindTag::default_kind)
            .collect();
        // Optional values switched on, and a spot light's cone.
        for shape in [
            SplineShape::Circle {
                radius: 100.0,
                ellipse: Some(50.0),
                ring: Some(20.0),
            },
            SplineShape::Rectangle {
                width: 100.0,
                height: 50.0,
                rounding: Some(10.0),
            },
            SplineShape::NSide {
                radius: 100.0,
                sides: 6,
                rounding: Some(10.0),
            },
        ] {
            kinds.push(ObjectKind::SplinePrimitive(SplinePrimitive::new(shape)));
        }
        if let ObjectKind::Light { light_type, .. } = kinds
            .iter_mut()
            .find(|k| matches!(k, ObjectKind::Light { .. }))
            .unwrap()
        {
            *light_type = LightType::Spot;
        }
        let ctx = egui::Context::default();
        for kind in kinds {
            let data = NodeData::new("o", kind.clone());
            let mut keys = KeyRows::new(&data, &kind, 0.0);
            let mut shown = kind.clone();
            ctx.run_ui(egui::RawInput::default(), |ui| {
                object_tab(ui, &mut shown, &mut keys, None)
            })
            .drop_without_applying_deltas();
            let mut drawn = keys.drawn.clone();
            drawn.sort_by_key(|p| p.index());
            let mut wanted = kind.params();
            wanted.sort_by_key(|p| p.index());
            assert_eq!(drawn, wanted, "{:?}", kind.tag());
            assert_eq!(shown, kind, "showing changes nothing");
        }
    }

    #[test]
    fn the_object_tab_shows_animated_values_and_idles_without_writing() {
        use grid4d_scene::{Key, ObjectParam, Track};
        let mut data = NodeData::new("s", grid4d_scene::ObjectKindTag::Sphere.default_kind());
        data.tracks.push(Track {
            param: Param::Object(ObjectParam::Radius),
            curve: grid4d_scene::Curve::from_keys([Key::new(0.0, 10.0), Key::new(10.0, 30.0)]),
        });
        let before = data.clone();
        let ctx = egui::Context::default();
        let keying = Keying {
            frame: 10.0,
            autokey: false,
        };
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(object_tab_at(ui, &mut data, keying, &mut false, None), None);
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(data, before);
    }
}
