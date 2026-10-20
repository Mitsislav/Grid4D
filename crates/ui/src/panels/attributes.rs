//! Attribute Manager: parameters of the active object, organised in C4D's
//! Basic / Coord. / Object tabs. With nothing selected it shows the project
//! settings, like Cinema 4D does. In Tool mode it shows the active tool's
//! options instead; choosing a selection tool switches to Tool mode, changing
//! the object selection switches back.
//!
//! Fields write to the document live; each finished interaction (a drag, a
//! typed value, a click) is recorded as one undo step through [`LiveEdit`].

use egui::{Button, DragValue, Grid, Margin, RichText, ScrollArea, Sense, TextEdit, Ui, Vec2};
use grid4d_geom::loops::WalkOptions;
use grid4d_geom::primitives::MAX_SEGMENTS;
use grid4d_scene::{Document, NodeData, NodeId, ObjectKind, SceneEdit, TimeSettings};

use crate::{
    CommandQueue, EditorCommand, EditorState, Tool,
    commands::tag_color,
    icons::{self, Icon},
    live_edit::{LiveEdit, interaction_ongoing},
    theme::palette as p,
    widgets::{attr_label, manager_tabs, section_heading},
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
        }
        if self.seen_active != editor.selection.active() {
            self.view = View::Objects;
            self.seen_active = editor.selection.active();
        }

        let _ = manager_tabs(ui, &["Attributes", "Layers"], 0);
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
                    match (self.view, editor.selection.active()) {
                        (View::Tool, _) => tool_ui(ui, doc, editor),
                        (View::Objects, Some(id)) => {
                            self.node_ui(ui, doc, id, editor.selection.len(), &mut editor.commands);
                        }
                        (View::Objects, None) => self.project_ui(ui, doc, &mut editor.commands),
                    }
                });
            });
    }

    fn node_ui(
        &mut self,
        ui: &mut Ui,
        doc: &mut Document,
        id: NodeId,
        selection_len: usize,
        commands: &mut CommandQueue,
    ) {
        // Widgets edit a copy; the scene is only written (and its revision only
        // bumped) when a value actually changed, so idle frames stay idle.
        let Some(mut data) = doc.scene.get(id).map(|node| node.data.clone()) else {
            return;
        };
        self.node_edit.begin(id, &data);

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

        match self.tab {
            Tab::Basic => basic_tab(ui, &mut data),
            Tab::Coord => coord_tab(ui, &mut data),
            Tab::Object => object_tab(ui, &mut data),
        }
        if doc.scene.get(id).is_some_and(|node| node.data != data)
            && let Some(node) = doc.scene.get_mut(id)
        {
            node.data.clone_from(&data);
        }

        if let Some((id, before)) = self.node_edit.end(id, &data, interaction_ongoing(ui)) {
            commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetData { id, data: before },
                label: self.tab.edit_label(),
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
            ui.color_edit_button_rgb(&mut data.display_color);
            ui.end_row();

            attr_label(ui, "Enabled", LABEL_WIDTH);
            ui.checkbox(&mut data.enabled, "");
            ui.end_row();
        });
}

fn coord_tab(ui: &mut Ui, data: &mut NodeData) {
    section_heading(ui, "Coordinates");
    const ROWS: [[&str; 3]; 3] = [
        ["P . X", "S . X", "R . H"],
        ["P . Y", "S . Y", "R . P"],
        ["P . Z", "S . Z", "R . B"],
    ];
    let t = &mut data.transform;
    Grid::new("grid4d.attr.coord")
        .num_columns(6)
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (axis, [pl, sl, rl]) in ROWS.into_iter().enumerate() {
                small_label(ui, pl);
                ui.add_sized(
                    [70.0, 19.0],
                    DragValue::new(&mut t.position[axis])
                        .speed(1.0)
                        .suffix(" cm")
                        .max_decimals(2),
                );
                small_label(ui, sl);
                ui.add_sized(
                    [52.0, 19.0],
                    DragValue::new(&mut t.scale[axis])
                        .speed(0.01)
                        .range(0.001..=10_000.0)
                        .max_decimals(3),
                );
                small_label(ui, rl);
                let mut degrees = t.rotation[axis].to_degrees();
                if ui
                    .add_sized(
                        [60.0, 19.0],
                        DragValue::new(&mut degrees).speed(0.5).suffix("°").max_decimals(2),
                    )
                    .changed()
                {
                    t.rotation[axis] = degrees.to_radians();
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    if ui.button("Reset Transform").clicked() {
        *t = grid4d_scene::Transform::IDENTITY;
    }
}

fn object_tab(ui: &mut Ui, data: &mut NodeData) {
    section_heading(ui, "Object Properties");
    Grid::new("grid4d.attr.object")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| match &mut data.kind {
            ObjectKind::Null => {
                ui.label(
                    RichText::new("A Null groups objects. Drag objects onto it in the Object Manager.")
                        .color(p::TEXT_DIM),
                );
                ui.end_row();
            }
            ObjectKind::Cube { size, segments } => {
                for (axis, label) in ["Size . X", "Size . Y", "Size . Z"].into_iter().enumerate() {
                    length_row(ui, label, &mut size[axis]);
                }
                for (axis, label) in ["Segments X", "Segments Y", "Segments Z"].into_iter().enumerate() {
                    segments_row(ui, label, &mut segments[axis], 1);
                }
            }
            ObjectKind::Sphere { radius, segments } => {
                length_row(ui, "Radius", radius);
                segments_row(ui, "Segments", segments, 3);
            }
            ObjectKind::Cylinder {
                radius,
                height,
                rotation_segments,
                height_segments,
            } => {
                length_row(ui, "Radius", radius);
                length_row(ui, "Height", height);
                segments_row(ui, "Height Segments", height_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Plane {
                width,
                depth,
                width_segments,
                depth_segments,
            } => {
                length_row(ui, "Width", width);
                length_row(ui, "Height", depth);
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
                length_row(ui, "Top Radius", top_radius);
                length_row(ui, "Bottom Radius", bottom_radius);
                length_row(ui, "Height", height);
                segments_row(ui, "Height Segments", height_segments, 1);
                segments_row(ui, "Rotation Segments", rotation_segments, 3);
            }
            ObjectKind::Torus {
                ring_radius,
                pipe_radius,
                ring_segments,
                pipe_segments,
            } => {
                length_row(ui, "Ring Radius", ring_radius);
                segments_row(ui, "Ring Segments", ring_segments, 3);
                length_row(ui, "Pipe Radius", pipe_radius);
                segments_row(ui, "Pipe Segments", pipe_segments, 3);
            }
            ObjectKind::Disc {
                inner_radius,
                outer_radius,
                rotation_segments,
                disc_segments,
            } => {
                length_row(ui, "Inner Radius", inner_radius);
                length_row(ui, "Outer Radius", outer_radius);
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
                length_row(ui, "Radius", radius);
                length_row(ui, "Height", height);
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
                ui.end_row();
            }
            ObjectKind::Light { color, intensity } => {
                attr_label(ui, "Color", LABEL_WIDTH);
                ui.color_edit_button_rgb(color);
                ui.end_row();
                attr_label(ui, "Intensity", LABEL_WIDTH);
                let mut percent = *intensity * 100.0;
                if ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut percent).speed(1.0).range(0.0..=1000.0).suffix(" %"),
                    )
                    .changed()
                {
                    *intensity = percent / 100.0;
                }
                ui.end_row();
            }
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
                ui.label(
                    RichText::new("With nothing selected, drag a point, edge or polygon to move just it")
                        .color(p::TEXT_DIM),
                );
                ui.end_row();
            }
            Tool::Scale | Tool::Rotate => {
                ui.label(RichText::new("This tool has no options yet.").color(p::TEXT_DIM));
                ui.end_row();
            }
            Tool::Extrude => {
                let m = &mut editor.modeling_settings;
                offset_row(ui, "Offset", &mut m.extrude.offset);
                count_row(ui, "Subdivision", &mut m.extrude.subdivision, 0);
                check_row(ui, "Create N-gons", &mut m.extrude.create_ngons);
                check_row(ui, "Create Caps", &mut m.extrude.create_caps);
                check_row(ui, "Preserve Groups", &mut m.extrude.preserve_groups);
                angle_row(ui, "Maximum Angle", &mut m.extrude.max_angle, 0.0..=180.0);
                angle_row(ui, "Rotation (Edges)", &mut m.edge_rotation, -180.0..=180.0);
            }
            Tool::Inset => {
                let m = &mut editor.modeling_settings;
                offset_row(ui, "Offset", &mut m.inset.offset);
                count_row(ui, "Subdivision", &mut m.inset.subdivision, 1);
                check_row(ui, "Create N-gons", &mut m.inset.create_ngons);
                check_row(ui, "Preserve Groups", &mut m.inset.preserve_groups);
                angle_row(ui, "Maximum Angle", &mut m.inset.max_angle, 0.0..=180.0);
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
                offset_row(ui, "Offset", &mut m.offset);
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
            Tool::LineCut => {
                let m = &mut editor.modeling_settings.line_cut;
                check_row(ui, "Visible Only", &mut m.visible_only);
                check_row(ui, "Infinite Cut", &mut m.infinite);
                check_row(ui, "Connect Cut Edges", &mut m.connect);
                check_row(ui, "Select Cuts", &mut m.select_cuts);
            }
        });
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

/// A length that may be negative (a tool's offset).
fn offset_row(ui: &mut Ui, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value).speed(0.5).suffix(" cm").max_decimals(2),
    );
    ui.end_row();
}

fn count_row(ui: &mut Ui, label: &str, value: &mut u32, min: u32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized([FIELD_WIDTH, 19.0], DragValue::new(value).speed(0.05).range(min..=64));
    ui.end_row();
}

/// An angle stored in radians, edited in degrees.
fn angle_row(ui: &mut Ui, label: &str, radians: &mut f32, degrees: std::ops::RangeInclusive<f32>) {
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
fn segments_row(ui: &mut Ui, label: &str, value: &mut u32, min: u32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value).speed(0.1).range(min..=MAX_SEGMENTS),
    );
    ui.end_row();
}

fn length_row(ui: &mut Ui, label: &str, value: &mut f32) {
    attr_label(ui, label, LABEL_WIDTH);
    ui.add_sized(
        [FIELD_WIDTH, 19.0],
        DragValue::new(value)
            .speed(1.0)
            .range(0.0..=1_000_000.0)
            .suffix(" cm")
            .max_decimals(2),
    );
    ui.end_row();
}

fn small_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(p::TEXT_DIM).size(11.5));
}
