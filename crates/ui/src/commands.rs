//! Structural editor operations, queued by panels and applied once per frame.
//!
//! This is the single choke point for undo/redo: every document change a
//! command makes is applied as [`SceneEdit`]s and recorded as one undo step in
//! [`EditorState::history`]. Selection and view changes are not undo steps, but
//! each step restores the selection it was made with.

use glam::Vec3;
use grid4d_geom::{
    BitSet, Component, MeshError, PolyMesh, Topology, ops,
    selection::{self, corners_from_edges, edges_from_corners},
};
use grid4d_render::{DisplayStyle, OrbitCamera, ShadingMode};
use grid4d_scene::{
    ComponentState, Components, Document, MeshHandle, Node, NodeData, NodeId, ObjectKind, ObjectKindTag, Param, Scene,
    SceneEdit, SplineHandle, Transform,
};

use crate::i18n::key;
use crate::{
    EditMode, EditorState, Selection, ViewportPanel,
    file::{self, FileAction, UnsavedChoice},
    theme::palette,
};

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click.
    Replace,
    /// Ctrl/Cmd + click.
    Toggle,
    /// Shift + click.
    Add,
    /// Ctrl/Cmd + click in a component mode (C4D removes there, rather than toggling).
    Remove,
}

impl SelectMode {
    /// Maps keyboard modifiers to a selection mode, C4D-style.
    #[must_use]
    pub fn from_modifiers(m: egui::Modifiers) -> Self {
        if m.command {
            Self::Toggle
        } else if m.shift {
            Self::Add
        } else {
            Self::Replace
        }
    }

    /// The same for components: Shift adds, Ctrl removes.
    #[must_use]
    pub fn for_components(m: egui::Modifiers) -> Self {
        match Self::from_modifiers(m) {
            Self::Toggle => Self::Remove,
            other => other,
        }
    }
}

/// Mesh ▸ Spline: commands on the active editable spline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineOp {
    /// The selected points lose their tangents.
    Hard,
    /// The selected points get smooth tangents.
    Soft,
    /// The segments with selected points (all without a selection) run backwards.
    Reverse,
    /// In closed segments, the selected point becomes the first.
    SetFirstPoint,
}

impl SplineOp {
    pub const ALL: [Self; 4] = [Self::Hard, Self::Soft, Self::Reverse, Self::SetFirstPoint];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hard => "Hard Interpolation",
            Self::Soft => "Soft Interpolation",
            Self::Reverse => "Reverse Sequence",
            Self::SetFirstPoint => "Set First Point",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    /// Replaces the document with an empty one (asks to save unsaved changes first).
    NewDocument,
    /// Asks for a document to open (asks to save unsaved changes first).
    Open,
    /// Opens this document: File ▸ Open Recent, or a file dropped on the window
    /// (asks to save unsaved changes first).
    OpenFile(std::path::PathBuf),
    /// Asks for a glTF file or Grid4D document and adds its objects (File ▸ Merge).
    Merge,
    /// Adds the objects of this file (a dropped glTF file). One undo step.
    MergeFile(std::path::PathBuf),
    /// Asks where, then writes the document as glTF or OBJ (File ▸ Export).
    Export,
    /// Asks where renders are saved and sets the render settings' File (one
    /// undo step).
    BrowseRenderOutput,
    /// A Mesh ▸ Spline command on the active editable spline (one undo step).
    SplineCommand(SplineOp),
    /// The Pen: a point at world `position` with outgoing tangent `tangent`
    /// (world, zero for a corner), added to the active editable spline, or to a
    /// new spline. One undo step.
    PenPoint {
        position: Vec3,
        tangent: Vec3,
    },
    /// The Pen: closes the segment being drawn and finishes (one undo step).
    PenClose,
    /// The Pen: finishes the spline being drawn (the next click starts a new one).
    PenFinish,
    /// Renders the scene at the current frame through the active view, in the
    /// background, and opens the Picture Viewer (Shift+R). Stops a render that runs.
    RenderToPictureViewer,
    /// Renders the active view into the viewport (Render View, Ctrl+R),
    /// replacing one that shows. Not saved.
    RenderView,
    /// Stops the running render, keeping what it rendered.
    StopRender,
    /// Asks where, then saves the picture the Picture Viewer shows as PNG.
    SaveRenderAs,
    /// Adds `tag` (a kind's default, or a preset such as Collider Body) to
    /// the selected objects that have no tag of its kind — Material tags are
    /// added to every one, over those there — (one undo step) and selects it
    /// on the active object.
    AddTag(grid4d_scene::Tag),
    /// Removes one tag from the object (one undo step).
    DeleteTag(crate::TagRef),
    /// Writes a Material tag's projection into its polygon object's UVs and
    /// switches the tag to UV (Bake to UVs; one undo step).
    BakeMaterialTag(crate::TagRef),
    /// Saves to the current file, asking for a path if there is none yet.
    Save,
    /// Asks for a path and saves there.
    SaveAs,
    /// Closes the window (asks to save unsaved changes first).
    Quit,
    /// The answer to a pending "Save changes?" prompt.
    AnswerUnsaved(UnsavedChoice),
    /// The answer to "Recover Unsaved Work?" after a crash.
    Recover(crate::autosave::RecoveryChoice),
    /// Shows this crash report in the system's file manager.
    ShowCrashReport(std::path::PathBuf),
    /// Gives every view the preferred shading, grid, axis and HUD.
    ApplyViewDefaults,
    /// Puts `objects` on `layer` (`None`: on no layer of their own), one undo
    /// step; objects already there are left alone.
    AssignLayer {
        layer: Option<grid4d_scene::LayerId>,
        objects: Vec<NodeId>,
    },
    /// Selects every object whose own layer is this one.
    SelectLayerObjects(grid4d_scene::LayerId),
    /// Creates an object of the plugin with this type id (Create ▸ Plugins),
    /// selected; nothing if it is not installed. `child`: a deformer goes
    /// under the active object, fitted to it (Shift + click).
    CreatePlugin {
        type_id: String,
        child: bool,
    },
    /// Runs the command plugin with this type id on the document and the
    /// selection with these options: its changes as one undo step, or its
    /// message when it cannot run.
    RunPlugin {
        type_id: String,
        options: grid4d_scene::PluginObject,
    },
    /// Runs a script on the document: its changes as one undo step, its
    /// output or error shown.
    RunScript(String),
    /// Script ▸ Run Script File…: asks for a `.rhai` file and runs it.
    RunScriptFile,
    /// Runs the library script of this name (Script ▸ a script).
    RunLibraryScript(String),
    /// The Script Manager's New: an empty script.
    NewScript,
    /// The Script Manager's Open: the library script of this name.
    OpenScript(String),
    /// The Script Manager's Save: its script into the library, under its name.
    SaveScript,
    /// Makes a take active (`None`: Main); the view looks through its camera,
    /// if it has one.
    SwitchTake(Option<grid4d_scene::TakeId>),
    /// Adds a take and makes it active.
    NewTake,
    /// Removes a take (going back to Main first if it is the active one).
    DeleteTake(grid4d_scene::TakeId),
    /// Drops one override of a take (`group` `None`: its render settings); the
    /// active take's object shows Main's value again.
    ClearOverride {
        take: grid4d_scene::TakeId,
        node: Option<NodeId>,
        group: Option<grid4d_scene::OverrideGroup>,
    },
    CreateObject(ObjectKindTag),
    /// Creates an object under the active one (Shift + create for deformers),
    /// a deformer fitted to it; at the top without an active object.
    CreateChild(ObjectKindTag),
    /// Sizes a deformer's box to its parent and centres it there.
    FitToParent(NodeId),
    DeleteSelected,
    /// Turns the selected primitives into polygon objects that look the same
    /// (C4D's "Make Editable", `C`), and the selected generators into polygon
    /// objects holding their result (their cage goes); others are left alone.
    MakeEditable,
    /// Simulate ▸ Bake Simulation: the whole range now (Esc stops).
    BakeSimulation,
    /// Simulate ▸ Clear Simulation Cache.
    ClearSimulation,
    /// Adds copies of the selected objects as the viewport shows them at the
    /// current frame (deformed, generated, animated) as plain objects, each
    /// after its original, and selects them (C4D's "Current State to Object").
    /// One undo step.
    CurrentStateToObject,
    /// A new generator of kind `tag` that takes the active object's place in the
    /// hierarchy, with the object as its child (Alt + create). One undo step.
    WrapInGenerator(ObjectKindTag),
    /// Enables or disables the selected generators, or the generator above each
    /// selected object (C4D's "Toggle Parent Generator", `Q`). One undo step.
    ToggleParentGenerator,
    /// Component modes: merges the selected points, edges or polygons into the
    /// polygons around them (Melt, U~Z / Alt+Backspace). One undo step.
    MeltSelected,
    /// Like Melt; for edges also removes the points left between two edges
    /// (Dissolve, M~N / Ctrl+Backspace). One undo step.
    DissolveSelected,
    /// Joins the selected points, or the middles of the selected edges, that share
    /// a polygon (Connect Points/Edges, M~M). One undo step.
    ConnectSelected,
    /// Loop/Path Cut on polygon object `node` across unique edge `edge` at
    /// `fractions` along it (from the edge's first point). One undo step.
    LoopCut {
        node: NodeId,
        edge: u32,
        fractions: Vec<f32>,
    },
    /// Line Cut on polygon object `node`: the plane through `eye` and the world
    /// directions `from` and `to` (the rays through the line's ends), with the Line
    /// Cut tool's options. One undo step.
    LineCut {
        node: NodeId,
        eye: Vec3,
        from: Vec3,
        to: Vec3,
    },
    /// Bridge on polygon object `node` from component `from` to `to` (faces, or
    /// unique edges in Edges mode) with the Bridge tool's options. One undo step.
    Bridge {
        node: NodeId,
        component: Component,
        from: u32,
        to: u32,
    },
    /// Bridges the two selected polygons or edges of the active object (the
    /// Bridge tool's Apply). One undo step.
    BridgeSelected,
    /// Welds the active object's selected points (those of the selected edges or
    /// polygons) into one: onto point `onto` if it is one of them, else at their
    /// centre. One undo step.
    Weld {
        node: NodeId,
        onto: Option<u32>,
    },
    /// Optimize with its options: the active polygon object, limited to the
    /// selection's points in a component mode. One undo step.
    Optimize,
    /// Runs the current modelling tool (Extrude, Inset) once with its options on
    /// the active polygon object's selection in the current mode (its Apply
    /// button). One undo step if anything changed.
    ApplyMeshTool,
    /// A UV tool on the active polygon object's selected polygons (all its
    /// visible ones when none), with the editor's UV settings: its UVs or its
    /// seams change. One undo step if anything changed.
    UvCommand(crate::uv_tools::UvOp),
    /// A click, frame or loop in a component mode on polygon object `node`:
    /// `hits` are the picked points, unique edges or polygons — none for empty
    /// space, which clears the selection on a plain click. One undo step if
    /// anything changed.
    SelectComponents {
        node: NodeId,
        component: Component,
        hits: Vec<u32>,
        mode: SelectMode,
    },
    /// Select ▸ Grow Selection (neighbours of the selection, current mode).
    GrowSelection,
    /// Select ▸ Shrink Selection (drops the selection's border).
    ShrinkSelection,
    /// Select ▸ Invert (every visible component not selected).
    InvertSelection,
    /// Select ▸ Select Connected (everything reachable from the selection).
    SelectConnected,
    /// Select ▸ Hide Selected / Hide Unselected / Unhide All.
    HideSelected,
    HideUnselected,
    UnhideAll,
    /// Switches to mode `to`, converting the current mode's component selection
    /// of the active object (Shift+click on a mode: `tolerant`; Ctrl+click:
    /// strict). One undo step if the selection changed.
    ConvertSelection {
        to: EditMode,
        tolerant: bool,
    },
    Select {
        id: NodeId,
        mode: SelectMode,
    },
    SelectAll,
    DeselectAll,
    /// Moves `node` under `new_parent` (last child), keeping its world placement.
    Reparent {
        node: NodeId,
        new_parent: Option<NodeId>,
    },
    Undo,
    Redo,
    /// Applies one discrete document change (a rename, a visibility toggle) as
    /// an undo step.
    Edit {
        edit: SceneEdit,
        label: &'static str,
    },
    /// Records a change that was already written live while the user interacted
    /// (a gizmo drag, a value field): `inverse` restores the state from before
    /// the interaction, e.g. `SetData` with the node's old data. Nothing is
    /// recorded if the live value equals it, i.e. nothing changed.
    CommitLive {
        inverse: SceneEdit,
        label: &'static str,
    },
    /// Record Active Objects (F9): keys at frame `at` for the enabled channels of
    /// the selected objects, with the values shown at the current frame. One undo
    /// step.
    RecordKeys {
        at: i32,
    },
    /// Removes the keys selected on the timeline from the selected objects. One
    /// undo step.
    DeleteKeys,
    /// Moves the keys selected on the timeline by `by` frames. One undo step.
    MoveKeys {
        by: i32,
    },
    /// A new material with the defaults, selected and shown in the Attribute
    /// Manager. One undo step.
    NewMaterial,
    /// Removes the material; Material tags that used it stay, empty. One undo step.
    DeleteMaterial(grid4d_scene::MaterialId),
    /// Links `material` to each of `nodes`: their last Material tag gets it
    /// (keeping its projection), or a new Material tag is added; with `layer`
    /// a new tag always goes on top. One undo step.
    AssignMaterial {
        material: grid4d_scene::MaterialId,
        nodes: Vec<NodeId>,
        layer: bool,
    },
    /// Asks for pictures and adds them as textures (Texture Manager ▸ Add…).
    /// One undo step.
    AddTextures,
    /// Adds these pictures as textures (dropped on the window); a picture
    /// already linked is selected, not added again. One undo step.
    AddTextureFiles(Vec<std::path::PathBuf>),
    /// Removes the texture. One undo step.
    DeleteTexture(grid4d_scene::TextureId),
    /// Asks for another picture for the texture. One undo step.
    RelinkTexture(grid4d_scene::TextureId),
    /// Asks for a folder and links every missing texture to the picture of
    /// the same name in it. One undo step.
    RelinkTextureFolder,
    /// Keeps the linked picture's bytes in the document. One undo step.
    EmbedTexture(grid4d_scene::TextureId),
    /// Asks where, writes the embedded picture there and links it. One undo
    /// step.
    UnembedTexture(grid4d_scene::TextureId),
    /// Shows a linked texture's file in the system's file browser.
    RevealTexture(grid4d_scene::TextureId),
    /// Removes the textures no material uses. One undo step.
    RemoveUnusedTextures,
    /// Asks for a picture and puts it on `channel` of `material` (added as a
    /// texture, linear for data channels, unless it is one already). One undo
    /// step.
    LoadChannelTexture {
        material: grid4d_scene::MaterialId,
        channel: grid4d_scene::Channel,
    },
    /// Changes or deletes the keys selected in the F-Curve editor. One undo step.
    EditCurveKeys {
        op: crate::keys::CurveOp,
    },
    /// Like [`EditorCommand::CommitLive`] for a change of several objects (an
    /// F-Curve drag): one undo step restoring them all.
    CommitLiveAll {
        inverses: Vec<SceneEdit>,
        label: &'static str,
    },
    /// Shows view `Some(i)` alone and makes it active (F1–F4), or all four views
    /// (`None`, F5).
    ShowView(Option<usize>),
    /// The active view's shading mode (Display menu, N~A … N~G).
    SetShading(ShadingMode),
    /// The active view's display style: Wireframe (N~H) or Box (N~K).
    SetDisplayStyle(DisplayStyle),
    FrameScene,
    FrameSelected,
    ResetView,
}

/// FIFO of pending commands. Draining keeps the vector's capacity.
#[derive(Debug, Default)]
pub struct CommandQueue {
    pending: Vec<EditorCommand>,
}

impl CommandQueue {
    pub fn push(&mut self, command: EditorCommand) {
        self.pending.push(command);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The queued commands, oldest first.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> &[EditorCommand] {
        &self.pending
    }
}

/// Applies every queued command. Returns `true` if anything was applied, so the
/// caller can request a repaint and show the result immediately.
pub fn apply_pending(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) -> bool {
    // A new frame: temporary values give way to the curves (not an undo step).
    let frame = doc.time.frame();
    let mut changed = false;
    // Preferences ▸ Undo Depth.
    if editor.history.limit() != editor.preferences.undo_depth {
        editor.history.set_limit(doc, editor.preferences.undo_depth);
    }
    if editor.seen_frame != Some(frame) {
        editor.seen_frame = Some(frame);
        changed = doc.scene.discard_unkeyed(f64::from(frame));
    }
    // Texture pictures loading in the background.
    changed |= editor.textures.update(doc.scene.textures());
    // Material previews rendered in the background.
    changed |= editor
        .previews
        .update(doc.scene.materials(), doc.scene.textures(), &editor.textures);
    // News from renders in the background; finished frames are saved.
    let (rendered, finished) = editor.renders.poll();
    changed |= rendered;
    for frame in &finished {
        file::save_rendered_frame(editor, frame);
    }
    if editor.commands.is_empty() {
        return changed;
    }
    let mut pending = std::mem::take(&mut editor.commands.pending);
    for command in pending.drain(..) {
        apply(command, doc, editor, viewport);
    }
    // Hand the (now empty) buffer back so its capacity is reused next frame.
    editor.commands.pending = pending;
    // Another object chosen: the Pen adds to the active spline again.
    if editor
        .pen_finished
        .is_some_and(|id| editor.selection.active() != Some(id))
    {
        editor.pen_finished = None;
    }
    true
}

fn apply(command: EditorCommand, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    match command {
        EditorCommand::NewDocument => file::request(FileAction::New, doc, editor, viewport),
        EditorCommand::Open => file::request(FileAction::Open, doc, editor, viewport),
        EditorCommand::OpenFile(path) => file::request(FileAction::OpenPath(path), doc, editor, viewport),
        EditorCommand::Merge => {
            let directory = editor
                .file
                .path()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf);
            if let Some(path) = editor.file.dialogs.pick_merge(directory.as_deref()) {
                file::merge(&path, doc, editor, viewport);
            }
        }
        EditorCommand::MergeFile(path) => file::merge(&path, doc, editor, viewport),
        EditorCommand::Export => file::export(doc, editor),
        EditorCommand::RenderToPictureViewer => {
            // Render Settings ▸ Take: another take renders from a copy
            // switched to it; a take's camera is the one renders look through.
            let take = doc.takes.take_to_render();
            let camera = take
                .and_then(|t| doc.takes.get(t))
                .and_then(|t| t.camera)
                .filter(|&c| doc.scene.contains(c))
                .or(viewport.view_object());
            let other = (take != doc.takes.active()).then(|| {
                let mut copy = doc.clone();
                copy.switch_take(take);
                copy
            });
            let shown = other.as_ref().unwrap_or(doc);
            let replay = editor.simulation.replay(shown);
            let pictures = editor.textures.pictures(shown.scene.textures());
            editor
                .renders
                .start(shown, pictures, viewport.camera(), camera, editor.file.path(), replay);
            editor.renders.viewer_open = true;
        }
        EditorCommand::RenderView => {
            let key = viewport.view_key(doc);
            let replay = editor.simulation.replay(doc);
            let pictures = editor.textures.pictures(doc.scene.textures());
            editor.renders.start_view(doc, pictures, viewport.camera(), key, replay);
        }
        EditorCommand::BakeSimulation => editor.simulation.bake(doc),
        EditorCommand::ClearSimulation => editor.simulation.clear(),
        EditorCommand::StopRender => editor.renders.stop(),
        EditorCommand::SaveRenderAs => file::save_render_as(doc, editor),
        EditorCommand::SplineCommand(op) => spline_command(doc, editor, op),
        EditorCommand::PenPoint { position, tangent } => pen_point(doc, editor, position, tangent),
        EditorCommand::PenClose => {
            if let Some(id) = pen_spline(doc, editor) {
                let data = doc.scene.get(id).and_then(|node| {
                    let ObjectKind::Spline { spline, .. } = &node.data.kind else {
                        return None;
                    };
                    let closed = spline.edited(|s| {
                        if let Some(last) = s.segments.last_mut() {
                            last.closed = true;
                        }
                    });
                    let mut data = node.data.clone();
                    if let ObjectKind::Spline { spline, .. } = &mut data.kind {
                        *spline = closed;
                    }
                    Some(data)
                });
                if let Some(data) = data {
                    commit(
                        doc,
                        editor,
                        "Close Spline",
                        vec![SceneEdit::SetData { id, data }],
                        |_, _| {},
                    );
                }
                editor.pen_finished = Some(id);
            }
        }
        EditorCommand::PenFinish => editor.pen_finished = editor.selection.active(),
        EditorCommand::BrowseRenderOutput => {
            if let Some(output) = file::pick_render_output(doc, editor) {
                let mut settings = doc.render.clone();
                settings.output = output;
                if settings != doc.render {
                    commit(
                        doc,
                        editor,
                        "Render Settings",
                        vec![SceneEdit::SetRender(settings)],
                        |_, _| {},
                    );
                }
            }
        }
        EditorCommand::AddTag(tag) => {
            let kind = tag.kind();
            let edits = editor
                .selection
                .as_slice()
                .iter()
                .filter_map(|&id| {
                    let data = &doc.scene.get(id)?.data;
                    if kind != grid4d_scene::TagKind::Material && data.tags.iter().any(|t| t.kind() == kind) {
                        return None;
                    }
                    let mut data = data.clone();
                    data.tags.push(tag.clone());
                    Some(SceneEdit::SetData { id, data })
                })
                .collect();
            commit(doc, editor, "Add Tag", edits, |_, _| {});
            editor.tag_selection = editor.selection.active().and_then(|node| {
                let tags = &doc.scene.get(node)?.data.tags;
                let index = tags.iter().rposition(|t| t.kind() == kind)?;
                crate::TagRef::at(node, tags, index)
            });
        }
        EditorCommand::DeleteTag(tag) => delete_tag(doc, editor, tag),
        EditorCommand::BakeMaterialTag(tag) => bake_material_tag(doc, editor, tag),
        EditorCommand::Save => {
            file::save(doc, editor, false);
        }
        EditorCommand::SaveAs => {
            file::save(doc, editor, true);
        }
        EditorCommand::Quit => file::request(FileAction::Quit, doc, editor, viewport),
        EditorCommand::AnswerUnsaved(choice) => file::answer(choice, doc, editor, viewport),
        EditorCommand::Recover(choice) => file::recover(choice, doc, editor, viewport),
        EditorCommand::ShowCrashReport(path) => file::reveal(&path),
        EditorCommand::ApplyViewDefaults => viewport.apply_defaults(&editor.preferences.views),
        EditorCommand::CreateObject(tag) if ObjectKindTag::EFFECTORS.contains(&tag) => {
            create_effector(doc, editor, viewport, tag);
        }
        EditorCommand::CreateObject(tag) => {
            let edit = SceneEdit::Insert {
                parent: None,
                index: usize::MAX,
                data: new_object_data(tag, &doc.scene, viewport.camera()),
            };
            commit(doc, editor, "Create Object", vec![edit], |inverses, selection| {
                // The inverse of an insert is the detach of the new node.
                if let Some(id) = inverses.first().and_then(SceneEdit::node) {
                    selection.set_single(id);
                }
            });
        }
        EditorCommand::RunPlugin { type_id, options } => run_plugin(doc, editor, &type_id, options),
        EditorCommand::RunScript(source) => run_script(doc, editor, &source),
        EditorCommand::RunLibraryScript(name) => match editor.scripts.read(&name) {
            Ok(source) => run_script(doc, editor, &source),
            Err(err) => {
                editor.file.alert = Some(file::Alert {
                    title: "Could Not Open Script",
                    message: format!("{name}\n\n{err}"),
                });
            }
        },
        EditorCommand::NewScript => editor.scripts.new_script(),
        EditorCommand::OpenScript(name) => {
            if let Err(err) = editor.scripts.open(&name) {
                editor.file.alert = Some(file::Alert {
                    title: "Could Not Open Script",
                    message: format!("{name}\n\n{err}"),
                });
            }
        }
        EditorCommand::SaveScript => {
            let message = if crate::scripts::valid_name(&editor.scripts.name) {
                editor.scripts.save().err().map(|err| err.to_string())
            } else {
                Some(crate::i18n::tr("Type a name for the script first (without \\ / : * ? \" < > |).").to_owned())
            };
            if let Some(message) = message {
                editor.file.alert = Some(file::Alert {
                    title: crate::scripts::COULD_NOT_SAVE,
                    message,
                });
            }
        }
        EditorCommand::RunScriptFile => {
            let directory = editor
                .file
                .path()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf);
            if let Some(path) = editor.file.dialogs.pick_script(directory.as_deref()) {
                file::run_script_file(&path, doc, editor);
            }
        }
        EditorCommand::CreatePlugin { type_id, child } => {
            let registry = grid4d_plugin::registry();
            let Some(plugin) = registry.plugin(&type_id) else {
                log::warn!("no plugin {type_id} to create");
                return;
            };
            let kind = ObjectKind::Plugin(grid4d_plugin::Registry::new_object(plugin));
            let mut data = NodeData::new(doc.scene.unique_name(plugin.name()), kind);
            // A deformer under the active object, its box around it.
            let parent = editor
                .selection
                .active()
                .filter(|&id| child && doc.scene.contains(id) && registry.deformer(&type_id).is_some());
            if let Some(parent) = parent {
                fit_to_parent(doc, parent, &mut data);
            }
            let edit = SceneEdit::Insert {
                parent,
                index: usize::MAX,
                data,
            };
            commit(doc, editor, key("Create Object"), vec![edit], |inverses, selection| {
                if let Some(id) = inverses.first().and_then(SceneEdit::node) {
                    selection.set_single(id);
                }
            });
        }
        EditorCommand::CreateChild(tag) => {
            let Some(parent) = editor.selection.active().filter(|&id| doc.scene.contains(id)) else {
                apply(EditorCommand::CreateObject(tag), doc, editor, viewport);
                return;
            };
            let mut data = new_object_data(tag, &doc.scene, viewport.camera());
            fit_to_parent(doc, parent, &mut data);
            let edit = SceneEdit::Insert {
                parent: Some(parent),
                index: usize::MAX,
                data,
            };
            commit(doc, editor, "Create Object", vec![edit], |inverses, selection| {
                if let Some(id) = inverses.first().and_then(SceneEdit::node) {
                    selection.set_single(id);
                }
            });
        }
        EditorCommand::FitToParent(id) => {
            let Some(node) = doc.scene.get(id) else { return };
            let Some(parent) = node.parent() else { return };
            let mut data = node.data.clone();
            if fit_to_parent(doc, parent, &mut data) {
                commit(
                    doc,
                    editor,
                    "Fit to Parent",
                    vec![SceneEdit::SetData { id, data }],
                    |_, _| {},
                );
            }
        }
        // A selected tag goes first: Delete removes it, not its object.
        EditorCommand::DeleteSelected if editor.mode == EditMode::Model && editor.selected_tag(doc).is_some() => {
            if let Some((tag, _)) = editor.selected_tag(doc) {
                delete_tag(doc, editor, tag);
            }
        }
        // On an editable spline in Points mode: its selected points.
        EditorCommand::DeleteSelected if spline_target(doc, editor).is_some() => {
            let lattice = spline_target(doc, editor)
                .and_then(|id| doc.scene.get(id))
                .is_some_and(|n| matches!(n.data.kind, ObjectKind::Ffd { .. }));
            if !lattice {
                delete_spline_points(doc, editor);
            }
        }
        // In a component mode Delete removes components, never objects.
        EditorCommand::DeleteSelected if editor.mode != EditMode::Model => {
            let label = match editor.mode {
                EditMode::Points => "Delete Points",
                EditMode::Edges => key("Delete Edges"),
                EditMode::Polygons | EditMode::Model => key("Delete Polygons"),
            };
            rebuild_selection(doc, editor, label, ops::delete, false);
        }
        EditorCommand::MeltSelected => rebuild_selection(doc, editor, "Melt", ops::melt, false),
        EditorCommand::DissolveSelected => rebuild_selection(doc, editor, "Dissolve", ops::dissolve, false),
        EditorCommand::ConnectSelected => rebuild_selection(doc, editor, key("Connect"), ops::connect, true),
        EditorCommand::LoopCut { node, edge, fractions } => {
            let select_cuts = editor.modeling_settings.loop_cut.select_cuts;
            let data = doc.scene.get(node).and_then(|n| {
                let ObjectKind::Polygon { mesh, .. } = &n.data.kind else {
                    return None;
                };
                let rebuilt = crate::modeling::loop_cut(mesh, idx(edge), &fractions)?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::edges_if(select_cuts))
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Loop Cut",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::LineCut { node, eye, from, to } => {
            let settings = editor.modeling_settings.line_cut;
            let world = doc.scene.world_matrix(node);
            let data = doc.scene.get(node).and_then(|n| {
                let ObjectKind::Polygon { mesh, .. } = &n.data.kind else {
                    return None;
                };
                // The cut runs in the object's space.
                let local = world.inverse();
                let cut = ops::LineCut {
                    eye: local.transform_point3(eye),
                    from: local.transform_vector3(from),
                    to: local.transform_vector3(to),
                    infinite: settings.infinite,
                    connect: settings.connect,
                };
                let visible = |x: Vec3| {
                    !settings.visible_only || {
                        let to_x = x - cut.eye;
                        let distance = to_x.length();
                        // Seen if nothing is hit clearly before the crossing.
                        mesh.raycast(cut.eye, to_x / distance.max(f32::EPSILON))
                            .is_none_or(|hit| hit.t >= distance * (1.0 - 1e-4) - 1e-4)
                    }
                };
                let rebuilt = ops::line_cut(mesh.mesh(), mesh.topology(), &cut, visible).ok()?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::edges_if(settings.select_cuts))
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Line Cut",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::DeleteSelected => {
            // Detaching a node takes its descendants along, so only the topmost
            // selected nodes are detached.
            let edits = editor
                .selection
                .as_slice()
                .iter()
                .filter(|&&id| !has_selected_ancestor(&doc.scene, &editor.selection, id))
                .map(|&root| SceneEdit::Detach { root })
                .collect();
            commit(doc, editor, "Delete", edits, |_, selection| selection.clear());
        }
        EditorCommand::MakeEditable => {
            // Simulated clones as they are at this frame.
            let mut simulated = grid4d_eval::Overrides::default();
            editor.simulation.overrides(doc.time.frame(), &mut simulated);
            let edits = make_editable_edits(doc, editor.selection.as_slice(), &simulated);
            commit(doc, editor, "Make Editable", edits, |_, _| {});
        }
        EditorCommand::CurrentStateToObject => current_state_to_object(doc, editor),
        EditorCommand::WrapInGenerator(tag) => wrap_in_generator(doc, editor, viewport, tag),
        EditorCommand::ToggleParentGenerator => {
            let mut generators: Vec<NodeId> = Vec::new();
            for &id in editor.selection.as_slice() {
                // The object itself if it is a generator, else the nearest one above it.
                let mut current = Some(id);
                while let Some(at) = current {
                    let Some(node) = doc.scene.get(at) else { break };
                    if node.data.kind.is_generator() {
                        if !generators.contains(&at) {
                            generators.push(at);
                        }
                        break;
                    }
                    current = node.parent();
                }
            }
            let edits = generators
                .into_iter()
                .filter_map(|id| {
                    let mut data = doc.scene.get(id)?.data.clone();
                    data.enabled = !data.enabled;
                    Some(SceneEdit::SetData { id, data })
                })
                .collect();
            commit(doc, editor, "Toggle Generator", edits, |_, _| {});
        }
        EditorCommand::ApplyMeshTool => {
            let tool = editor.tool;
            let mut settings = editor.modeling_settings;
            let offset = crate::modeling::offset_mut(tool, &mut settings).map_or(0.0, |o| *o);
            if let Some((id, component)) = component_target(doc, editor)
                && let Some(data) = doc
                    .scene
                    .get(id)
                    .and_then(|n| crate::modeling::apply(&n.data, tool, component, &settings, offset))
            {
                commit(
                    doc,
                    editor,
                    tool.label(),
                    vec![SceneEdit::SetData { id, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::Bridge {
            node,
            component,
            from,
            to,
        } => {
            let options = editor.modeling_settings.bridge;
            let data = doc.scene.get(node).and_then(|n| {
                let ObjectKind::Polygon { mesh, .. } = &n.data.kind else {
                    return None;
                };
                bridge(mesh, component, idx(from), idx(to), &options).and_then(|rebuilt| {
                    crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::PolygonsSelected)
                })
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Bridge",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::BridgeSelected => {
            let options = editor.modeling_settings.bridge;
            let target = component_target(doc, editor);
            let data = target.and_then(|(id, component)| {
                let n = doc.scene.get(id)?;
                let ObjectKind::Polygon { mesh, components } = &n.data.kind else {
                    return None;
                };
                let pair: Vec<usize> = match component {
                    Component::Polygon => components.selected().polygons.iter().collect(),
                    Component::Edge => edges_from_corners(&components.selected().edges, mesh.topology())
                        .iter()
                        .collect(),
                    Component::Point => Vec::new(),
                };
                let &[from, to] = pair.as_slice() else { return None };
                let rebuilt = bridge(mesh, component, from, to, &options)?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::PolygonsSelected)
                    .map(|d| (id, d))
            });
            if let Some((id, data)) = data {
                commit(doc, editor, "Bridge", vec![SceneEdit::SetData { id, data }], |_, _| {});
            }
        }
        EditorCommand::Weld { node, onto } => {
            let component = editor.mode.component();
            let data = doc.scene.get(node).zip(component).and_then(|(n, component)| {
                let ObjectKind::Polygon { mesh, components } = &n.data.kind else {
                    return None;
                };
                let points = selection::points_of(mesh.mesh(), components.selected().get(component), component);
                if points.len() < 2 {
                    return None;
                }
                let at = |p: usize| mesh.mesh().points()[p];
                let to = match onto.map(idx).filter(|&p| points.contains(p)) {
                    Some(p) => at(p),
                    None => {
                        // The middle of the selection's bounding box.
                        let (min, max) = points
                            .iter()
                            .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(at(p)), hi.max(at(p))));
                        (min + max) * 0.5
                    }
                };
                let rebuilt = ops::weld(mesh.mesh(), &points, to).ok()?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::Unselected)
            });
            if let Some(data) = data {
                commit(
                    doc,
                    editor,
                    "Weld",
                    vec![SceneEdit::SetData { id: node, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::UvCommand(op) => {
            let edges_cut = editor.mode == EditMode::Edges;
            let data = editor.selection.active().and_then(|id| {
                let node = doc.scene.get(id)?;
                crate::uv_tools::apply(&node.data, op, &editor.uv_settings, edges_cut).map(|d| (id, d))
            });
            if let Some((id, data)) = data {
                commit(
                    doc,
                    editor,
                    op.label(),
                    vec![SceneEdit::SetData { id, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::Optimize => {
            let options = editor.modeling_settings.optimize;
            let component = editor.mode.component();
            let data = editor.selection.active().and_then(|id| {
                let n = doc.scene.get(id)?;
                let ObjectKind::Polygon { mesh, components } = &n.data.kind else {
                    return None;
                };
                // In a component mode, only the selection's points (all if none).
                let scope = component
                    .map(|c| selection::points_of(mesh.mesh(), components.selected().get(c), c))
                    .filter(|points| !points.is_empty());
                let rebuilt = ops::optimize(mesh.mesh(), scope.as_ref(), &options).ok()?;
                crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::Unselected).map(|d| (id, d))
            });
            if let Some((id, data)) = data {
                commit(
                    doc,
                    editor,
                    "Optimize",
                    vec![SceneEdit::SetData { id, data }],
                    |_, _| {},
                );
            }
        }
        EditorCommand::Select { id, mode } => {
            if doc.scene.contains(id) {
                match mode {
                    SelectMode::Replace => editor.selection.set_single(id),
                    SelectMode::Toggle => editor.selection.toggle(id),
                    SelectMode::Add => editor.selection.add(id),
                    SelectMode::Remove => editor.selection.remove(id),
                }
            }
        }
        EditorCommand::SelectComponents { node, hits, mode, .. } if is_spline(doc, node) => {
            edit_spline_points(doc, editor, node, key("Select Points"), |state, _| {
                let picked: Vec<usize> = hits
                    .iter()
                    .map(|&i| idx(i))
                    .filter(|&i| !state.hidden.points.contains(i))
                    .collect();
                let bits = &mut state.selected.points;
                match mode {
                    SelectMode::Replace => {
                        bits.clear();
                        bits.extend(picked);
                    }
                    SelectMode::Add => bits.extend(picked),
                    SelectMode::Remove => {
                        for i in picked {
                            bits.remove(i);
                        }
                    }
                    SelectMode::Toggle => {
                        let on = picked.iter().any(|&i| bits.contains(i));
                        for i in picked {
                            if on {
                                bits.remove(i);
                            } else {
                                bits.insert(i);
                            }
                        }
                    }
                }
            });
        }
        EditorCommand::SelectComponents {
            node,
            component,
            hits,
            mode,
        } => edit_components(doc, editor, node, select_label(component), |state, mesh| {
            let picked: BitSet = hits.iter().map(|&i| idx(i)).collect();
            // An edge is stored as all of its corners; hidden components stay unselected.
            let mut picked = stored(picked, component, mesh);
            picked.remove_all(&hidden_mask(state, mesh, component));
            let indices: Vec<usize> = picked.iter().collect();
            let bits = state.selected.get_mut(component);
            match mode {
                SelectMode::Replace => {
                    bits.clear();
                    bits.extend(indices);
                }
                SelectMode::Add => bits.extend(indices),
                SelectMode::Remove => {
                    for i in indices {
                        bits.remove(i);
                    }
                }
                SelectMode::Toggle => {
                    let on = indices.iter().any(|&i| bits.contains(i));
                    for i in indices {
                        if on {
                            bits.remove(i);
                        } else {
                            bits.insert(i);
                        }
                    }
                }
            }
        }),
        EditorCommand::GrowSelection => {
            let edges_only = editor.selection_settings.grow_edges_only;
            in_component_mode(doc, editor, key("Grow Selection"), |state, mesh, component| {
                let grown = selection::grow(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    edges_only,
                );
                set_selected(state, mesh, component, grown);
            });
        }
        EditorCommand::ShrinkSelection => {
            let edges_only = editor.selection_settings.grow_edges_only;
            in_component_mode(doc, editor, key("Shrink Selection"), |state, mesh, component| {
                let shrunk = selection::shrink(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    edges_only,
                );
                set_selected(state, mesh, component, shrunk);
            });
        }
        EditorCommand::InvertSelection if spline_target(doc, editor).is_some() => {
            on_spline_target(doc, editor, key("Invert Selection"), |state, count| {
                let mut inverted = BitSet::new();
                inverted.extend((0..count).filter(|&i| !state.selected.points.contains(i)));
                inverted.remove_all(&state.hidden.points);
                state.selected.points = inverted;
            });
        }
        EditorCommand::InvertSelection => {
            in_component_mode(doc, editor, key("Invert Selection"), |state, mesh, component| {
                let hidden = hidden_mask(state, mesh, component);
                let inverted = selection::invert(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    &hidden,
                );
                set_selected(state, mesh, component, inverted);
            })
        }
        EditorCommand::SelectConnected => {
            in_component_mode(doc, editor, key("Select Connected"), |state, mesh, component| {
                let reached =
                    selection::connected(mesh.mesh(), mesh.topology(), state.selected.get(component), component);
                set_selected(state, mesh, component, reached);
            })
        }
        EditorCommand::HideSelected => {
            in_component_mode(doc, editor, key("Hide Selected"), |state, mesh, component| {
                let selected = stored(
                    elements(state.selected.get(component), component, mesh),
                    component,
                    mesh,
                );
                state.hidden.get_mut(component).extend(selected.iter());
                state.selected.get_mut(component).clear();
            })
        }
        EditorCommand::HideUnselected => {
            in_component_mode(doc, editor, key("Hide Unselected"), |state, mesh, component| {
                let hidden = hidden_mask(state, mesh, component);
                let others = selection::invert(
                    mesh.mesh(),
                    mesh.topology(),
                    state.selected.get(component),
                    component,
                    &hidden,
                );
                state.hidden.get_mut(component).extend(others.iter());
            })
        }
        EditorCommand::UnhideAll => in_component_mode(doc, editor, key("Unhide All"), |state, _, _| {
            state.hidden = grid4d_geom::ComponentSet::default();
        }),
        EditorCommand::ConvertSelection { to, tolerant } => {
            if let (Some(from), Some(target), Some(id)) =
                (editor.mode.component(), to.component(), editor.selection.active())
                && from != target
            {
                edit_components(doc, editor, id, key("Convert Selection"), |state, mesh| {
                    let mut converted =
                        selection::convert(mesh.mesh(), mesh.topology(), &state.selected, from, target, tolerant);
                    converted.remove_all(state.hidden.get(target)); // hidden components stay unselected
                    *state.selected.get_mut(target) = converted;
                });
            }
            editor.mode = to;
        }
        EditorCommand::SelectAll if spline_target(doc, editor).is_some() => {
            on_spline_target(doc, editor, "Select All", |state, count| {
                let mut all = BitSet::new();
                all.insert_range(0..count);
                all.remove_all(&state.hidden.points);
                state.selected.points = all;
            });
        }
        EditorCommand::DeselectAll if spline_target(doc, editor).is_some() => {
            on_spline_target(doc, editor, "Deselect All", |state, _| state.selected.points.clear());
        }
        EditorCommand::SelectAll if editor.mode != EditMode::Model => {
            in_component_mode(doc, editor, "Select All", |state, mesh, component| {
                let count = match component {
                    Component::Point => mesh.mesh().point_count(),
                    Component::Edge => mesh.mesh().corner_count(),
                    Component::Polygon => mesh.mesh().face_count(),
                };
                let mut all = BitSet::new();
                all.insert_range(0..count);
                set_selected(state, mesh, component, all);
            });
        }
        EditorCommand::DeselectAll if editor.mode != EditMode::Model => {
            if let (Some(component), Some(id)) = (editor.mode.component(), editor.selection.active()) {
                edit_components(doc, editor, id, "Deselect All", |state, _| {
                    state.selected.get_mut(component).clear();
                });
            }
        }
        EditorCommand::SelectAll => {
            let mut ids = Vec::with_capacity(doc.scene.len());
            let mut stack = Vec::new();
            doc.scene.walk(&mut stack, |id, _, _, _| {
                ids.push(id);
                true
            });
            editor.selection.set_all(ids);
        }
        EditorCommand::DeselectAll => editor.selection.clear(),
        EditorCommand::AssignLayer { layer, objects } => {
            let layer = layer.filter(|&l| doc.scene.layers().contains(l));
            let edits: Vec<SceneEdit> = objects
                .iter()
                .filter_map(|&id| {
                    let node = doc.scene.get(id).filter(|n| n.data.layer != layer)?;
                    let mut data = node.data.clone();
                    data.layer = layer;
                    Some(SceneEdit::SetData { id, data })
                })
                .collect();
            if !edits.is_empty() {
                let label = if layer.is_some() {
                    key("Add to Layer")
                } else {
                    key("Remove from Layer")
                };
                commit(doc, editor, label, edits, |_, _| {});
            }
        }
        EditorCommand::SwitchTake(take) => {
            commit(doc, editor, "Switch Take", vec![SceneEdit::SwitchTake(take)], |_, _| {});
            if let Some(camera) = take
                .and_then(|t| doc.takes.get(t))
                .and_then(|t| t.camera)
                .filter(|&c| doc.scene.contains(c))
            {
                editor.view_camera = Some(camera);
            }
        }
        EditorCommand::NewTake => {
            let mut takes = doc.takes.clone();
            let take = takes.insert(grid4d_scene::Take::new(takes.unique_name("Take")));
            commit(
                doc,
                editor,
                "New Take",
                vec![SceneEdit::SetTakes(takes), SceneEdit::SwitchTake(Some(take))],
                |_, _| {},
            );
        }
        EditorCommand::DeleteTake(take) => {
            let mut inverses = Vec::new();
            if doc.takes.active() == Some(take)
                && let Ok(inverse) = doc.apply(SceneEdit::SwitchTake(None))
            {
                inverses.push(inverse);
            }
            let mut takes = doc.takes.clone();
            if takes.remove(take).is_some()
                && let Ok(inverse) = doc.apply(SceneEdit::SetTakes(takes))
            {
                inverses.insert(0, inverse);
            }
            let selection = editor.selection.clone();
            editor
                .history
                .record_as_is(doc, "Delete Take", inverses, selection.clone(), selection);
        }
        EditorCommand::ClearOverride { take, node, group } => {
            let mut takes = doc.takes.clone();
            let mut edits = Vec::new();
            match (node, group) {
                (Some(node), Some(group)) => {
                    let main = takes.clear_override(take, node, group);
                    edits.push(SceneEdit::SetTakes(takes));
                    if let (Some(main), Some(current)) = (main, doc.scene.get(node)) {
                        let mut data = current.data.clone();
                        main.write(&mut data);
                        edits.push(SceneEdit::SetData { id: node, data });
                    }
                }
                _ => {
                    let main = takes.clear_render(take);
                    edits.push(SceneEdit::SetTakes(takes));
                    if let Some(main) = main {
                        edits.push(SceneEdit::SetRender(main));
                    }
                }
            }
            let selection = editor.selection.clone();
            match doc.apply_all(edits) {
                Ok(inverses) => {
                    editor
                        .history
                        .record_as_is(doc, "Clear Override", inverses, selection.clone(), selection);
                }
                Err(err) => log::warn!("Clear Override rejected: {err}"),
            }
        }
        EditorCommand::SelectLayerObjects(layer) => {
            let mut ids = Vec::new();
            doc.scene.walk(&mut Vec::new(), |id, node, _, _| {
                if node.data.layer == Some(layer) {
                    ids.push(id);
                }
                true
            });
            editor.selection.set_all(ids);
        }
        EditorCommand::Reparent { node, new_parent } => {
            let current_parent = doc.scene.get(node).map(Node::parent);
            if current_parent.is_some_and(|parent| parent != new_parent)
                && let Some(transform) = doc.scene.transform_keeping_world(node, new_parent)
            {
                let edit = SceneEdit::Move {
                    id: node,
                    parent: new_parent,
                    index: usize::MAX,
                    transform,
                };
                commit(doc, editor, "Reparent", vec![edit], |_, _| {});
            }
        }
        EditorCommand::Undo => {
            editor.history.undo(doc, &mut editor.selection);
            doc.scene.discard_unkeyed(f64::from(doc.time.frame()));
        }
        EditorCommand::Redo => {
            editor.history.redo(doc, &mut editor.selection);
            doc.scene.discard_unkeyed(f64::from(doc.time.frame()));
        }
        EditorCommand::Edit { edit, label } => commit(doc, editor, label, vec![edit], |_, _| {}),
        EditorCommand::CommitLive { inverse, label } => {
            if !is_current_state(doc, &inverse) {
                let selection = editor.selection.clone();
                editor
                    .history
                    .record(doc, label, vec![inverse], selection.clone(), selection);
            }
        }
        EditorCommand::RecordKeys { at } => {
            let edits = crate::keys::record(doc, &editor.selection, &editor.animation, f64::from(at));
            commit(doc, editor, "Record Keys", edits, |_, _| {});
        }
        EditorCommand::DeleteKeys => {
            let edits = crate::keys::delete(doc, &editor.selection, &editor.keys.frames);
            commit(doc, editor, "Delete Keys", edits, |_, _| {});
            editor.keys.frames.clear();
        }
        EditorCommand::MoveKeys { by } => {
            let by = f64::from(by);
            let edits = crate::keys::shift(doc, &editor.selection, &editor.keys.frames, by);
            commit(doc, editor, "Move Keys", edits, |_, _| {});
            for frame in &mut editor.keys.frames {
                *frame += by;
            }
        }
        EditorCommand::NewMaterial => {
            let mut materials = doc.scene.materials().clone();
            let name = materials.unique_name("Mat");
            let id = materials.insert(grid4d_scene::Material::new(name));
            commit(
                doc,
                editor,
                "New Material",
                vec![SceneEdit::SetMaterials(materials)],
                |_, _| {},
            );
            editor.materials.selected = Some(id);
            editor.materials.shown = true;
        }
        EditorCommand::DeleteMaterial(id) => {
            let mut materials = doc.scene.materials().clone();
            if materials.remove(id).is_some() {
                commit(
                    doc,
                    editor,
                    "Delete Material",
                    vec![SceneEdit::SetMaterials(materials)],
                    |_, _| {},
                );
            }
            if editor.materials.selected == Some(id) {
                editor.materials.selected = None;
                editor.materials.shown = false;
            }
        }
        EditorCommand::AddTextures
        | EditorCommand::AddTextureFiles(_)
        | EditorCommand::DeleteTexture(_)
        | EditorCommand::RelinkTexture(_)
        | EditorCommand::RelinkTextureFolder
        | EditorCommand::EmbedTexture(_)
        | EditorCommand::UnembedTexture(_)
        | EditorCommand::RevealTexture(_)
        | EditorCommand::RemoveUnusedTextures
        | EditorCommand::LoadChannelTexture { .. } => texture_command(command, doc, editor),
        EditorCommand::AssignMaterial { material, nodes, layer } => {
            let edits: Vec<SceneEdit> = nodes
                .iter()
                .filter_map(|&id| {
                    let before = &doc.scene.get(id)?.data;
                    let mut data = before.clone();
                    let last = data
                        .tags
                        .iter_mut()
                        .rev()
                        .find_map(|t| match t {
                            grid4d_scene::Tag::Material(tag) => Some(tag),
                            _ => None,
                        })
                        .filter(|_| !layer);
                    match last {
                        Some(tag) => tag.material = Some(material),
                        None => data
                            .tags
                            .push(grid4d_scene::Tag::Material(grid4d_scene::MaterialTag::new(Some(
                                material,
                            )))),
                    }
                    (data != *before).then_some(SceneEdit::SetData { id, data })
                })
                .collect();
            if doc.scene.materials().contains(material) {
                commit(doc, editor, "Assign Material", edits, |_, _| {});
            }
        }
        EditorCommand::EditCurveKeys { op } => {
            let edits = crate::keys::curve_edits(doc, &editor.curves.keys, op);
            commit(doc, editor, op.label(), edits, |_, _| {});
            if op == crate::keys::CurveOp::Delete {
                editor.curves.keys.clear();
            }
        }
        EditorCommand::CommitLiveAll { inverses, label } => {
            let inverses: Vec<SceneEdit> = inverses
                .into_iter()
                .filter(|inverse| !is_current_state(doc, inverse))
                .collect();
            let selection = editor.selection.clone();
            editor
                .history
                .record(doc, label, inverses, selection.clone(), selection);
        }
        EditorCommand::ShowView(view) => viewport.show_views(view),
        EditorCommand::SetShading(shading) => viewport.set_shading(shading),
        EditorCommand::SetDisplayStyle(style) => viewport.set_display_style(style),
        EditorCommand::FrameScene => viewport.frame_scene(&doc.scene),
        EditorCommand::FrameSelected => viewport.frame_selection(&doc.scene, &editor.selection),
        EditorCommand::ResetView => viewport.reset_view(),
    }
    editor.selection.retain_existing(&doc.scene);
}

/// Applies `edits` as one undo step named `label`, all or nothing.
///
/// `select` updates the selection from the inverses (e.g. to select a created
/// object); the step remembers the selection before and after.
/// The folder of the document's file, where dialogs start.
fn document_folder(editor: &EditorState) -> Option<std::path::PathBuf> {
    editor
        .file
        .path()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
}

/// Commits `change` to a copy of the document's textures as one undo step
/// `label`; nothing if it changed nothing.
fn edit_textures(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    change: impl FnOnce(&mut grid4d_scene::Textures),
) {
    let mut textures = doc.scene.textures().clone();
    change(&mut textures);
    if &textures != doc.scene.textures() {
        commit(doc, editor, label, vec![SceneEdit::SetTextures(textures)], |_, _| {});
    }
}

/// The textures some material's channels use.
#[must_use]
pub fn used_textures(doc: &Document) -> std::collections::HashSet<grid4d_scene::TextureId> {
    doc.scene
        .materials()
        .iter()
        .flat_map(|(_, material)| material.maps().map(|(_, slot)| slot.texture))
        .collect()
}

/// The Texture Manager's commands (see `docs/reference/material-textures.md`).
#[allow(clippy::too_many_lines, reason = "one arm per command, each short")]
fn texture_command(command: EditorCommand, doc: &mut Document, editor: &mut EditorState) {
    use grid4d_scene::{Texture, TextureSource};

    let folder = document_folder(editor);
    let linked = |doc: &Document, id| match doc.scene.textures().get(id).map(|t| &t.source) {
        Some(TextureSource::Linked(path)) => Some(path.clone()),
        _ => None,
    };
    match command {
        EditorCommand::AddTextures | EditorCommand::AddTextureFiles(_) => {
            let paths = match command {
                EditorCommand::AddTextureFiles(paths) => paths,
                _ => editor.file.dialogs.pick_textures(folder.as_deref()),
            };
            let mut chosen = None;
            edit_textures(doc, editor, "Add Textures", |textures| {
                for path in paths {
                    let known = path
                        .extension()
                        .is_some_and(|ext| grid4d_io::is_texture_extension(&ext.to_string_lossy()));
                    if !known {
                        continue;
                    }
                    chosen = Some(textures.linked_to(&path).unwrap_or_else(|| {
                        let mut texture = Texture::linked(path);
                        texture.name = textures.unique_name(&texture.name);
                        textures.insert(texture)
                    }));
                }
            });
            if chosen.is_some() {
                editor.texture_selection = chosen;
                editor.panes.show = Some(crate::workspace::Pane::Textures);
            }
        }
        EditorCommand::DeleteTexture(id) => {
            edit_textures(doc, editor, "Delete Texture", |textures| {
                textures.remove(id);
            });
        }
        EditorCommand::RelinkTexture(id) => {
            let Some(texture) = doc.scene.textures().get(id) else {
                return;
            };
            let start = match &texture.source {
                TextureSource::Linked(path) => path
                    .parent()
                    .filter(|dir| dir.is_dir())
                    .map(std::path::Path::to_path_buf)
                    .or(folder),
                TextureSource::Embedded { .. } => folder,
            };
            if let Some(path) = editor.file.dialogs.pick_texture(start.as_deref()) {
                edit_textures(doc, editor, "Relink Texture", |textures| {
                    if let Some(texture) = textures.get_mut(id) {
                        texture.source = TextureSource::Linked(path);
                    }
                });
            }
        }
        EditorCommand::RelinkTextureFolder => {
            let Some(dir) = editor.file.dialogs.pick_folder(folder.as_deref()) else {
                return;
            };
            let found: Vec<(grid4d_scene::TextureId, std::path::PathBuf)> = doc
                .scene
                .textures()
                .iter()
                .filter(|&(id, t)| matches!(editor.textures.status(id, t), crate::textures::Status::Missing))
                .filter_map(|(id, t)| {
                    let TextureSource::Linked(path) = &t.source else {
                        return None;
                    };
                    let candidate = dir.join(path.file_name()?);
                    candidate.is_file().then_some((id, candidate))
                })
                .collect();
            if found.is_empty() {
                editor.file.alert = Some(file::Alert {
                    title: "Relink Folder",
                    message: crate::i18n::trf("None of the missing pictures is in {0}.", &[&dir.display().to_string()]),
                });
                return;
            }
            edit_textures(doc, editor, "Relink Textures", |textures| {
                for (id, path) in found {
                    if let Some(texture) = textures.get_mut(id) {
                        texture.source = TextureSource::Linked(path);
                    }
                }
            });
        }
        EditorCommand::EmbedTexture(id) => {
            let Some(path) = linked(doc, id) else {
                return;
            };
            match grid4d_io::read_file(&path) {
                Ok(bytes) => {
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    edit_textures(doc, editor, "Embed Texture", |textures| {
                        if let Some(texture) = textures.get_mut(id) {
                            texture.source = TextureSource::Embedded {
                                file_name,
                                bytes: bytes.into(),
                            };
                        }
                    });
                }
                Err(err) => {
                    editor.file.alert = Some(file::Alert {
                        title: "Cannot Embed Texture",
                        message: format!("{}\n\n{err}", path.display()),
                    });
                }
            }
        }
        EditorCommand::UnembedTexture(id) => {
            let Some(TextureSource::Embedded { file_name, bytes }) =
                doc.scene.textures().get(id).map(|t| t.source.clone())
            else {
                return;
            };
            let Some(path) = editor.file.dialogs.pick_texture_save(folder.as_deref(), &file_name) else {
                return;
            };
            match std::fs::write(&path, &bytes) {
                Ok(()) => edit_textures(doc, editor, "Unembed Texture", |textures| {
                    if let Some(texture) = textures.get_mut(id) {
                        texture.source = TextureSource::Linked(path);
                    }
                }),
                Err(err) => {
                    editor.file.alert = Some(file::Alert {
                        title: "Cannot Unembed Texture",
                        message: format!("{}\n\n{err}", path.display()),
                    });
                }
            }
        }
        EditorCommand::RevealTexture(id) => {
            if let Some(path) = linked(doc, id) {
                file::reveal(&path);
            }
        }
        EditorCommand::LoadChannelTexture { material, channel } => {
            if !doc.scene.materials().contains(material) {
                return;
            }
            let Some(path) = editor.file.dialogs.pick_texture(folder.as_deref()) else {
                return;
            };
            let mut textures = doc.scene.textures().clone();
            let texture = textures.linked_to(&path).unwrap_or_else(|| {
                let mut texture = Texture::linked(path);
                texture.name = textures.unique_name(&texture.name);
                if !channel.is_colour() {
                    texture.color_space = grid4d_scene::ColorSpace::Linear;
                }
                textures.insert(texture)
            });
            let mut materials = doc.scene.materials().clone();
            if let Some(m) = materials.get_mut(material) {
                let slot = m.map_mut(channel);
                *slot = Some(match *slot {
                    Some(old) => grid4d_scene::TextureSlot { texture, ..old },
                    None => grid4d_scene::TextureSlot::new(texture),
                });
            }
            commit(
                doc,
                editor,
                "Load Texture",
                vec![SceneEdit::SetTextures(textures), SceneEdit::SetMaterials(materials)],
                |_, _| {},
            );
        }
        EditorCommand::RemoveUnusedTextures => {
            let used = used_textures(doc);
            edit_textures(doc, editor, "Remove Unused Textures", |textures| {
                let unused: Vec<_> = textures.ids().iter().copied().filter(|id| !used.contains(id)).collect();
                for id in unused {
                    textures.remove(id);
                }
            });
        }
        _ => {}
    }
    if editor
        .texture_selection
        .is_some_and(|id| !doc.scene.textures().contains(id))
    {
        editor.texture_selection = None;
    }
}

fn commit(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    edits: Vec<SceneEdit>,
    select: impl FnOnce(&[SceneEdit], &mut Selection),
) {
    if edits.is_empty() {
        return;
    }
    let selection_before = editor.selection.clone();
    match doc.apply_all(edits) {
        Ok(inverses) => {
            select(&inverses, &mut editor.selection);
            editor.selection.retain_existing(&doc.scene);
            let selection_after = editor.selection.clone();
            editor
                .history
                .record(doc, label, inverses, selection_before, selection_after);
        }
        Err(err) => log::warn!("{label} rejected: {err}"),
    }
}

/// The active object and the current mode's component kind, when the mode is a
/// component mode and the active object a polygon object.
#[must_use]
pub fn component_target(doc: &Document, editor: &EditorState) -> Option<(NodeId, Component)> {
    let component = editor.mode.component()?;
    let id = editor.selection.active()?;
    matches!(doc.scene.get(id)?.data.kind, ObjectKind::Polygon { .. }).then_some((id, component))
}

/// Runs a rebuilding operation (Delete, Melt, Dissolve, Connect) on the active
/// polygon object's selection in the current component mode, carrying its
/// selection and hidden flags across: one undo step named `label` if the mesh
/// changed. With `select_cuts` the new edges become the edge selection.
fn rebuild_selection(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    operation: impl FnOnce(&PolyMesh, &Topology, Component, &BitSet) -> Result<ops::Rebuilt, MeshError>,
    select_cuts: bool,
) {
    let Some((id, component)) = component_target(doc, editor) else {
        return;
    };
    let data = doc.scene.get(id).and_then(|n| {
        let ObjectKind::Polygon { mesh, components } = &n.data.kind else {
            return None;
        };
        let topology = mesh.topology();
        let selected = match component {
            Component::Point => components.selected().points.clone(),
            Component::Edge => edges_from_corners(&components.selected().edges, topology),
            Component::Polygon => components.selected().polygons.clone(),
        };
        if selected.is_empty() {
            return None;
        }
        let rebuilt = operation(mesh.mesh(), topology, component, &selected).ok()?;
        crate::modeling::rebuilt_data(&n.data, rebuilt, crate::modeling::Fresh::edges_if(select_cuts))
    });
    if let Some(data) = data {
        commit(doc, editor, label, vec![SceneEdit::SetData { id, data }], |_, _| {});
    }
}

/// Removes `node`'s tag of `kind` as one undo step and clears the tag selection.
fn delete_tag(doc: &mut Document, editor: &mut EditorState, tag: crate::TagRef) {
    let edit = doc.scene.get(tag.node).and_then(|n| {
        let mut data = n.data.clone();
        let index = tag.index_in(&data.tags)?;
        data.tags.remove(index);
        Some(SceneEdit::SetData { id: tag.node, data })
    });
    commit(doc, editor, "Delete Tag", edit.into_iter().collect(), |_, _| {});
    editor.tag_selection = None;
}

/// Bake to UVs: the Material tag's projection, fitted or placed as the tag
/// says, becomes its polygon object's UVs, and the tag projects by UV (its
/// turn, tiling and offset still apply, so the picture stays where it was).
/// Nothing for UV and Camera projections or other objects.
fn bake_material_tag(doc: &mut Document, editor: &mut EditorState, tag: crate::TagRef) {
    let Some(node) = doc.scene.get(tag.node) else { return };
    let mut data = node.data.clone();
    let Some(index) = tag.index_in(&data.tags) else { return };
    let Some(grid4d_scene::Tag::Material(material_tag)) = data.tags.get_mut(index) else {
        return;
    };
    let ObjectKind::Polygon { mesh, .. } = &mut data.kind else {
        return;
    };
    let polygons = mesh.mesh();
    let Some(to_unit) = material_tag.to_unit(polygons.bounds()) else {
        return;
    };
    let mut baked = polygons.clone();
    let uvs = grid4d_geom::uv::project(material_tag.projection, polygons, to_unit);
    if baked.set_uvs(Some(uvs)).is_err() {
        return;
    }
    *mesh = MeshHandle::new(baked);
    material_tag.projection = grid4d_scene::Projection::Uv;
    material_tag.placement = None;
    let edits = vec![SceneEdit::SetData { id: tag.node, data }];
    commit(doc, editor, "Bake to UVs", edits, |_, _| {});
}

/// Alt + create: a new generator takes the active object's place (same parent and
/// position among its siblings) and the object moves under it, keeping its
/// transform (the generator sits at the parent's origin). Without an active
/// object it is an ordinary create. One undo step, the generator selected.
fn wrap_in_generator(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel, tag: ObjectKindTag) {
    let Some(active) = editor.selection.active().filter(|&id| doc.scene.contains(id)) else {
        apply(EditorCommand::CreateObject(tag), doc, editor, viewport);
        return;
    };
    // The objects that become its inputs: the active one, or for generators
    // with several inputs (Sweep: profile, path; Loft: sections) the selected
    // objects in selection order, none inside another.
    let mut targets = vec![active];
    let wanted = tag.default_kind().generator_inputs();
    if wanted >= 2 {
        let mut chosen: Vec<NodeId> = Vec::new();
        for &id in editor.selection.as_slice() {
            if chosen.len() < wanted
                && doc.scene.contains(id)
                && !chosen
                    .iter()
                    .any(|&other| is_inside(&doc.scene, id, other) || is_inside(&doc.scene, other, id))
            {
                chosen.push(id);
            }
        }
        if chosen.len() >= 2 {
            targets = chosen;
        }
    }
    let target = targets[0];
    let parent = doc.scene.get(target).and_then(Node::parent);
    let index = doc.scene.index_in_parent(target).unwrap_or(usize::MAX);
    let selection_before = editor.selection.clone();
    let insert = SceneEdit::Insert {
        parent,
        index,
        data: new_object_data(tag, &doc.scene, viewport.camera()),
    };
    let Ok(undo_insert) = doc.apply(insert) else { return };
    let Some(generator) = undo_insert.node() else { return };
    let mut undo = Vec::with_capacity(targets.len() + 1);
    for (index, &id) in targets.iter().enumerate() {
        let transform = doc.scene.get(id).map(|n| n.data.transform).unwrap_or_default();
        let mov = SceneEdit::Move {
            id,
            parent: Some(generator),
            index,
            transform,
        };
        match doc.apply(mov) {
            Ok(undo_move) => undo.push(undo_move),
            Err(err) => {
                log::warn!("Wrapping in a generator rejected: {err}");
                for edit in undo.into_iter().rev() {
                    let _ = doc.apply(edit);
                }
                let _ = doc.apply(undo_insert);
                return;
            }
        }
    }
    // Undone in reverse: the moves back, then the generator removed.
    undo.reverse();
    undo.push(undo_insert);
    editor.selection.set_single(generator);
    let selection_after = editor.selection.clone();
    editor
        .history
        .record(doc, "Create Generator", undo, selection_before, selection_after);
}

/// A new effector, added to the Effectors list of every selected Cloner (as
/// the reference does), all as one undo step; the effector becomes the
/// selection.
fn create_effector(doc: &mut Document, editor: &mut EditorState, viewport: &ViewportPanel, tag: ObjectKindTag) {
    let selection_before = editor.selection.clone();
    let cloners: Vec<NodeId> = editor
        .selection
        .as_slice()
        .iter()
        .copied()
        .filter(|&id| {
            doc.scene
                .get(id)
                .is_some_and(|n| matches!(n.data.kind, ObjectKind::Cloner { .. }))
        })
        .collect();
    let insert = SceneEdit::Insert {
        parent: None,
        index: usize::MAX,
        data: new_object_data(tag, &doc.scene, viewport.camera()),
    };
    let Ok(undo_insert) = doc.apply(insert) else { return };
    let Some(effector) = undo_insert.node() else { return };
    let mut undo = Vec::with_capacity(cloners.len() + 1);
    for id in cloners {
        let Some(mut data) = doc.scene.get(id).map(|n| n.data.clone()) else {
            continue;
        };
        if let ObjectKind::Cloner { effectors, .. } = &mut data.kind {
            effectors.push(effector);
        }
        if let Ok(inverse) = doc.apply(SceneEdit::SetData { id, data }) {
            undo.push(inverse);
        }
    }
    // Undone in reverse: the lists back, then the effector removed.
    undo.reverse();
    undo.push(undo_insert);
    editor.selection.set_single(effector);
    let selection_after = editor.selection.clone();
    editor
        .history
        .record(doc, "Create Object", undo, selection_before, selection_after);
}

/// `inner` is `outer` or below it.
fn is_inside(scene: &Scene, inner: NodeId, outer: NodeId) -> bool {
    let mut at = Some(inner);
    while let Some(id) = at {
        if id == outer {
            return true;
        }
        at = scene.get(id).and_then(Node::parent);
    }
    false
}

/// A Bridge between two faces (Polygons) or two unique edges (Edges) of `mesh`.
fn bridge(
    mesh: &MeshHandle,
    component: Component,
    from: usize,
    to: usize,
    options: &ops::BridgeOptions,
) -> Option<ops::Rebuilt> {
    match component {
        Component::Polygon => ops::bridge_polygons(mesh.mesh(), from, to, options).ok(),
        Component::Edge => ops::bridge_edges(mesh.mesh(), mesh.topology(), from, to, options).ok(),
        Component::Point => None,
    }
}

/// Runs `change` on the active polygon object's components in the current
/// component mode, as one undo step named `label` (nothing outside such a mode).
fn in_component_mode(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, &MeshHandle, Component),
) {
    if let Some((id, component)) = component_target(doc, editor) {
        edit_components(doc, editor, id, label, |state, mesh| change(state, mesh, component));
    }
}

/// Every stored flag that hides a component of kind `component`: hidden ones,
/// the points/edges of hidden polygons, and every corner of a hidden edge.
fn hidden_mask(state: &ComponentState, mesh: &MeshHandle, component: Component) -> BitSet {
    let closure = selection::hidden_closure(mesh.mesh(), mesh.topology(), &state.hidden);
    stored(elements(closure.get(component), component, mesh), component, mesh)
}

/// Replaces the selection of `component` with `bits` (stored form), minus hidden ones.
fn set_selected(state: &mut ComponentState, mesh: &MeshHandle, component: Component, mut bits: BitSet) {
    bits.remove_all(&hidden_mask(state, mesh, component));
    *state.selected.get_mut(component) = bits;
}

/// A stored selection as elements: edges as unique edges, the rest as is.
fn elements(bits: &BitSet, component: Component, mesh: &MeshHandle) -> BitSet {
    match component {
        Component::Edge => edges_from_corners(bits, mesh.topology()),
        Component::Point | Component::Polygon => bits.clone(),
    }
}

/// Elements in stored form: an edge as all of its corners.
fn stored(bits: BitSet, component: Component, mesh: &MeshHandle) -> BitSet {
    match component {
        Component::Edge => corners_from_edges(&bits, mesh.topology()),
        Component::Point | Component::Polygon => bits,
    }
}

/// The active object when it is an editable spline (any mode).
pub(crate) fn active_spline(doc: &Document, editor: &EditorState) -> Option<NodeId> {
    editor.selection.active().filter(|&id| is_spline(doc, id))
}

/// The spline the Pen adds to: the active editable spline, unless the Pen
/// finished it.
pub(crate) fn pen_spline(doc: &Document, editor: &EditorState) -> Option<NodeId> {
    active_spline(doc, editor).filter(|&id| editor.pen_finished != Some(id))
}

/// Maps a component state's point flags through `map` (old index → new).
fn remap_points(state: &ComponentState, map: &[u32]) -> ComponentState {
    let through = |bits: &BitSet| -> BitSet { bits.iter().filter_map(|old| map.get(old).copied()).map(idx).collect() };
    let mut out = ComponentState::default();
    out.selected.points = through(&state.selected.points);
    out.hidden.points = through(&state.hidden.points);
    out
}

/// Mesh ▸ Spline commands on the active editable spline: one undo step if the
/// spline changed; the selection follows its points.
fn spline_command(doc: &mut Document, editor: &mut EditorState, op: SplineOp) {
    let Some(id) = active_spline(doc, editor) else { return };
    let Some(node) = doc.scene.get(id) else { return };
    let ObjectKind::Spline {
        spline,
        interpolation,
        components,
    } = &node.data.kind
    else {
        return;
    };
    let selected = &components.selected().points;
    let current = spline.spline();
    let (changed, state) = match op {
        SplineOp::Hard | SplineOp::Soft => {
            let mut changed = current.clone();
            if op == SplineOp::Hard {
                changed.hard(selected);
            } else {
                changed.soft(selected);
            }
            (changed, components.state().clone())
        }
        SplineOp::Reverse => {
            let ranges: Vec<(usize, usize)> = current.segment_ranges().collect();
            let picked = |segment: usize| {
                selected.is_empty()
                    || ranges
                        .get(segment)
                        .is_some_and(|&(start, end)| (start..end).any(|i| selected.contains(i)))
            };
            let (changed, map) = current.reverse_segments(picked);
            (changed, remap_points(components.state(), &map))
        }
        SplineOp::SetFirstPoint => {
            let (changed, map) = current.set_first_points(selected);
            (changed, remap_points(components.state(), &map))
        }
    };
    if changed == *current && state == *components.state() {
        return;
    }
    let mut data = node.data.clone();
    data.kind = ObjectKind::Spline {
        spline: SplineHandle::new(changed),
        interpolation: *interpolation,
        components: Components::new(state),
    };
    commit(
        doc,
        editor,
        op.label(),
        vec![SceneEdit::SetData { id, data }],
        |_, _| {},
    );
}

/// The Pen's point: appended to the spline being drawn (a new segment if its
/// last is closed), or the first point of a new spline, which becomes active.
fn pen_point(doc: &mut Document, editor: &mut EditorState, position: Vec3, tangent: Vec3) {
    if let Some(id) = pen_spline(doc, editor) {
        let world = doc.scene.world_matrix_at(id, f64::from(doc.time.frame()));
        if world.determinant().abs() <= f32::EPSILON {
            return;
        }
        let local = world.inverse();
        let (point, tangent) = (local.transform_point3(position), local.transform_vector3(tangent));
        let Some(node) = doc.scene.get(id) else { return };
        let mut data = node.data.clone();
        if let ObjectKind::Spline { spline, .. } = &mut data.kind {
            *spline = spline.edited(|s| {
                s.points.push(point);
                s.tangents.push([-tangent, tangent]);
                match s.segments.last_mut() {
                    Some(last) if !last.closed => last.count += 1,
                    _ => s.segments.push(grid4d_geom::spline::Segment {
                        count: 1,
                        closed: false,
                    }),
                }
            });
        }
        commit(
            doc,
            editor,
            "Add Point",
            vec![SceneEdit::SetData { id, data }],
            |_, _| {},
        );
        return;
    }
    let spline = grid4d_geom::spline::Spline {
        kind: editor.pen.kind,
        points: vec![position],
        tangents: vec![[-tangent, tangent]],
        segments: vec![grid4d_geom::spline::Segment {
            count: 1,
            closed: false,
        }],
    };
    let mut data = NodeData::new(
        doc.scene.unique_name(ObjectKindTag::Spline.label()),
        ObjectKind::Spline {
            spline: SplineHandle::new(spline),
            interpolation: grid4d_geom::spline::Interpolation::default(),
            components: Components::default(),
        },
    );
    data.display_color = [0.9, 0.9, 0.92];
    let edit = SceneEdit::Insert {
        parent: None,
        index: usize::MAX,
        data,
    };
    commit(doc, editor, "Pen", vec![edit], |inverses, selection| {
        if let Some(SceneEdit::Detach { root }) = inverses.first() {
            selection.set_single(*root);
        }
    });
    editor.pen_finished = None;
}

/// The active object in Points mode, when its points are edited as a
/// spline's: an editable spline or an FFD's lattice.
#[must_use]
pub fn spline_target(doc: &Document, editor: &EditorState) -> Option<NodeId> {
    (editor.mode == EditMode::Points)
        .then(|| editor.selection.active())
        .flatten()
        .filter(|&id| is_spline(doc, id))
}

/// Whether `id`'s points are edited as a spline's (an editable spline, an FFD).
fn is_spline(doc: &Document, id: NodeId) -> bool {
    doc.scene.get(id).is_some_and(|n| n.data.kind.points().is_some())
}

/// Runs `change` on the spline target's components (with its point count) as
/// one undo step named `label`.
fn on_spline_target(
    doc: &mut Document,
    editor: &mut EditorState,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, usize),
) {
    if let Some(id) = spline_target(doc, editor) {
        edit_spline_points(doc, editor, id, label, |state, spline| {
            change(state, spline.points.len())
        });
    }
}

/// Changes the components of editable spline `id` with `change` as one undo
/// step named `label`; records nothing if they end up unchanged.
fn edit_spline_points(
    doc: &mut Document,
    editor: &mut EditorState,
    id: NodeId,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, &grid4d_geom::spline::Spline),
) {
    let Some(node) = doc.scene.get(id) else { return };
    let mut data = node.data.clone();
    let Some((spline, components)) = data.kind.points_mut() else {
        return;
    };
    let before = components.clone();
    change(components.make_mut(), spline.spline());
    if *components == before {
        return;
    }
    commit(doc, editor, label, vec![SceneEdit::SetData { id, data }], |_, _| {});
}

/// Delete in Points mode on an editable spline: its selected points go, hidden
/// flags follow the points that stay. One undo step.
fn delete_spline_points(doc: &mut Document, editor: &mut EditorState) {
    // An FFD's lattice points are never deleted (the grid would break).
    let Some(id) = spline_target(doc, editor) else { return };
    let Some(node) = doc.scene.get(id) else { return };
    let ObjectKind::Spline {
        spline,
        interpolation,
        components,
    } = &node.data.kind
    else {
        return;
    };
    let selected = &components.selected().points;
    if selected.is_empty() {
        return;
    }
    let (kept, map) = spline.spline().without_points(selected);
    let mut state = ComponentState::default();
    state.hidden.points.extend(
        components
            .hidden()
            .points
            .iter()
            .filter_map(|old| map.get(old).copied().flatten())
            .map(idx),
    );
    let mut data = node.data.clone();
    data.kind = ObjectKind::Spline {
        spline: SplineHandle::new(kept),
        interpolation: *interpolation,
        components: Components::new(state),
    };
    commit(
        doc,
        editor,
        "Delete Points",
        vec![SceneEdit::SetData { id, data }],
        |_, _| {},
    );
}

/// Changes the components of polygon object `id` with `change` as one undo step
/// named `label`; records nothing if they end up unchanged.
fn edit_components(
    doc: &mut Document,
    editor: &mut EditorState,
    id: NodeId,
    label: &'static str,
    change: impl FnOnce(&mut ComponentState, &MeshHandle),
) {
    let Some(node) = doc.scene.get(id) else { return };
    let ObjectKind::Polygon { mesh, components } = &node.data.kind else {
        return;
    };
    let mut updated = components.clone();
    change(updated.make_mut(), mesh);
    if updated == *components {
        return;
    }
    let mut data = node.data.clone();
    data.kind = ObjectKind::Polygon {
        mesh: mesh.clone(),
        components: updated,
    };
    commit(doc, editor, label, vec![SceneEdit::SetData { id, data }], |_, _| {});
}

const fn select_label(component: Component) -> &'static str {
    match component {
        Component::Point => key("Select Points"),
        Component::Edge => key("Select Edges"),
        Component::Polygon => key("Select Polygons"),
    }
}

/// `u32` index → `usize` (lossless on every supported platform).
fn idx(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `true` if [`EditorCommand::MakeEditable`] has a primitive or generator to convert in
/// `selection` (enables the menu entry and the palette button).
#[must_use]
pub fn can_make_editable(doc: &Document, selection: &Selection) -> bool {
    // Cheap (called every frame): no mesh is built here.
    selection.as_slice().iter().any(|&id| {
        doc.scene.get(id).is_some_and(|node| {
            let kind = &node.data.kind;
            ObjectKindTag::PRIMITIVES.contains(&kind.tag())
                || (kind.is_generator() && node.data.enabled && !node.children().is_empty())
        })
    })
}

/// `true` if [`EditorCommand::CurrentStateToObject`] has something to copy in
/// `selection` (enables the menu entry): an object other than a deformer.
#[must_use]
pub fn can_bake_current_state(doc: &Document, selection: &Selection) -> bool {
    selection.as_slice().iter().any(|&id| {
        doc.scene
            .get(id)
            .is_some_and(|node| !grid4d_eval::is_deformer(&node.data.kind))
    })
}

/// An object's shape in its own space at `frame`, for command plugins: a
/// polygon object's mesh, a generator's result, a primitive's or a
/// generator plugin's mesh.
pub(crate) fn shape_of(scene: &Scene, id: NodeId, frame: f64) -> Option<PolyMesh> {
    let kind = scene.get(id)?.data.kind_at(frame);
    match &*kind {
        ObjectKind::Polygon { mesh, .. } => Some(mesh.mesh().clone()),
        kind if kind.is_generator() => grid4d_eval::generator_mesh(scene, id, frame),
        kind => grid4d_eval::editable_mesh(kind),
    }
}

/// Runs a command plugin: its changes become one undo step named after it,
/// the new top-level objects selected; its message, when it cannot run, is
/// shown. The options are kept for its dialog.
fn run_plugin(doc: &mut Document, editor: &mut EditorState, type_id: &str, options: grid4d_scene::PluginObject) {
    let Some(plugin) = grid4d_plugin::registry().command(type_id) else {
        log::warn!("no command plugin {type_id}");
        return;
    };
    *editor.plugin_commands.options_mut(plugin) = options.clone();
    let frame = f64::from(doc.time.frame());
    let result = {
        let shapes = |id: NodeId| shape_of(&doc.scene, id, frame);
        let context = grid4d_plugin::Context::new(
            doc,
            editor.selection.as_slice(),
            editor.selection.active(),
            frame,
            &shapes,
        );
        plugin.run(&context, &grid4d_plugin::Values::new(plugin.params(), &options))
    };
    match result {
        Ok(changes) => apply_changes(doc, editor, plugin.name(), changes),
        Err(message) => {
            editor.file.alert = Some(file::Alert {
                title: plugin.name(),
                message: crate::i18n::tr(&message).to_owned(),
            });
        }
    }
}

/// The last lines of a script's output, for a message.
fn output_text(output: &[String]) -> String {
    const SHOWN: usize = 40;
    let skipped = output.len().saturating_sub(SHOWN);
    let mut text = if skipped > 0 {
        String::from("…\n")
    } else {
        String::new()
    };
    text.push_str(&output[skipped..].join("\n"));
    text
}

/// Runs a script (see `docs/reference/extend-scripting.md`): on a copy of the
/// document, which becomes the document when it finishes, its edits one undo
/// step ("Run Script"), the selection it asked for. Its output and error go to
/// the Script Manager's console, and into a message while the Script Manager
/// is closed. A failing script changes nothing.
pub(crate) fn run_script(doc: &mut Document, editor: &mut EditorState, source: &str) {
    let frame = f64::from(doc.time.frame());
    match grid4d_script::run(doc, editor.selection.as_slice(), frame, shape_of, source) {
        Ok(outcome) => {
            let selection_before = editor.selection.clone();
            let changed = !outcome.inverses.is_empty();
            if changed {
                *doc = outcome.document;
            }
            if let Some(selection) = outcome.selection {
                editor.selection.set_all(selection);
            }
            editor.selection.retain_existing(&doc.scene);
            if changed {
                let selection_after = editor.selection.clone();
                editor
                    .history
                    .record(doc, "Run Script", outcome.inverses, selection_before, selection_after);
            }
            if !outcome.output.is_empty() && !editor.scripts.show {
                editor.file.alert = Some(file::Alert {
                    title: "Script Output",
                    message: output_text(&outcome.output),
                });
            }
            editor.scripts.log(outcome.output, false);
        }
        Err(err) => {
            if !editor.scripts.show {
                let mut message = err.to_string();
                if !err.output.is_empty() {
                    message.push_str("\n\n");
                    message.push_str(&output_text(&err.output));
                }
                editor.file.alert = Some(file::Alert {
                    title: "Script Error",
                    message,
                });
            }
            let line = err.to_string();
            editor.scripts.log(err.output, false);
            editor.scripts.log([line], true);
        }
    }
}

/// Applies a command plugin's changes as one undo step named `label` (all or
/// nothing) and selects the new top-level objects.
fn apply_changes(doc: &mut Document, editor: &mut EditorState, label: &'static str, changes: grid4d_plugin::Changes) {
    if changes.is_empty() {
        return;
    }
    let selection_before = editor.selection.clone();
    match changes.apply(doc) {
        Ok(applied) => {
            if !applied.tops.is_empty() {
                editor.selection.set_all(applied.tops);
            }
            editor.selection.retain_existing(&doc.scene);
            let selection_after = editor.selection.clone();
            editor
                .history
                .record(doc, label, applied.inverses, selection_before, selection_after);
        }
        Err(err) => log::warn!("{label} rejected: {err}"),
    }
}

/// Inserts the copies [`crate::current_state::bake`] makes as one undo step and
/// selects the top ones.
fn current_state_to_object(doc: &mut Document, editor: &mut EditorState) {
    // Simulated objects as they are at this frame.
    let mut simulated = grid4d_eval::Overrides::default();
    editor.simulation.overrides(doc.time.frame(), &mut simulated);
    let baked = crate::current_state::bake(doc, editor.selection.as_slice(), &simulated);
    if baked.is_empty() {
        return;
    }
    let selection_before = editor.selection.clone();
    let mut ids: Vec<NodeId> = Vec::with_capacity(baked.len());
    let mut roots = Vec::new();
    let mut inverses = Vec::with_capacity(baked.len());
    for copy in baked {
        let (parent, index) = match copy.parent {
            Some(parent) => (ids.get(parent).copied(), usize::MAX),
            // Right after its original.
            None => (
                doc.scene.get(copy.original).and_then(Node::parent),
                doc.scene.index_in_parent(copy.original).map_or(usize::MAX, |i| i + 1),
            ),
        };
        let inserted = doc
            .apply(SceneEdit::Insert {
                parent,
                index,
                data: copy.data,
            })
            .map_err(|err| err.to_string())
            .and_then(|inverse| inverse.node().map(|id| (id, inverse)).ok_or_else(String::new));
        let Ok((id, inverse)) = inserted else {
            // All or nothing: take back what was inserted.
            for inverse in inverses.into_iter().rev() {
                let _ = doc.apply(inverse);
            }
            log::warn!("Current State to Object rejected");
            return;
        };
        if copy.parent.is_none() {
            roots.push(id);
        }
        ids.push(id);
        inverses.push(inverse);
    }
    inverses.reverse(); // children before parents
    editor.selection.set_all(roots);
    let selection_after = editor.selection.clone();
    editor.history.record(
        doc,
        "Current State to Object",
        inverses,
        selection_before,
        selection_after,
    );
}

/// Make Editable as document edits: each selected primitive becomes a polygon
/// object; each selected, enabled generator a polygon object holding its result
/// at the viewport level, its cage removed. Objects inside such a cage are not
/// baked on their own (they go with it); everything else is left alone.
fn make_editable_edits(doc: &Document, selection: &[NodeId], simulated: &grid4d_eval::Overrides) -> Vec<SceneEdit> {
    let enabled_generator = |id: NodeId| {
        doc.scene
            .get(id)
            .filter(|node| node.data.kind.is_generator() && node.data.enabled)
    };
    // A generator's inputs: its first child, or the Sweep's profile and path.
    let inputs = |node: &Node| -> Vec<NodeId> {
        node.children()
            .iter()
            .take(node.data.kind.generator_inputs())
            .copied()
            .collect()
    };
    let all_cages: Vec<NodeId> = selection
        .iter()
        .filter_map(|&id| enabled_generator(id))
        .flat_map(inputs)
        .collect();
    let in_cage = |id: NodeId| {
        let mut at = Some(id);
        while let Some(current) = at {
            if all_cages.contains(&current) {
                return true;
            }
            at = doc.scene.get(current).and_then(Node::parent);
        }
        false
    };
    let mut edits = Vec::new();
    let mut cages = Vec::new();
    // Shapes are baked as they are at the current frame.
    let frame = f64::from(doc.time.frame());
    // Clones exist only in an evaluation (made when a Cloner needs it).
    let mut evaluated: Option<grid4d_eval::EvaluatedScene> = None;
    for &id in selection {
        let Some(node) = doc.scene.get(id).filter(|_| !in_cage(id)) else {
            continue;
        };
        let kind = node.data.kind_at(frame);
        // A spline primitive becomes an editable spline: the same curve, its
        // plane and direction baked into the points.
        if let ObjectKind::SplinePrimitive(primitive) = &*kind {
            let mut data = node.data.clone();
            data.kind = ObjectKind::Spline {
                spline: SplineHandle::new(primitive.spline()),
                interpolation: primitive.interpolation,
                components: Components::default(),
            };
            edits.push(SceneEdit::SetData {
                id,
                data: without_lost_tracks(data),
            });
            continue;
        }
        // A text becomes an editable spline of its outlines.
        if let ObjectKind::Text(text) = &*kind {
            let mut data = node.data.clone();
            data.kind = ObjectKind::Spline {
                spline: SplineHandle::new(grid4d_text::text_spline(text)),
                interpolation: text.interpolation,
                components: Components::default(),
            };
            edits.push(SceneEdit::SetData {
                id,
                data: without_lost_tracks(data),
            });
            continue;
        }
        // A Cloner becomes a Null holding its clones as polygon objects.
        if let ObjectKind::Cloner { .. } = &*kind {
            if enabled_generator(id).is_none() {
                continue;
            }
            let eval = evaluated.get_or_insert_with(|| {
                let mut eval = grid4d_eval::EvaluatedScene::new();
                eval.set_frame(frame);
                eval.set_overrides(simulated);
                eval.update(&doc.scene);
                eval
            });
            let mut data = node.data.clone();
            data.kind = ObjectKind::Null;
            edits.push(SceneEdit::SetData {
                id,
                data: without_lost_tracks(data),
            });
            edits.extend(
                crate::current_state::clone_objects(eval, &doc.scene, id, frame)
                    .into_iter()
                    .map(|data| SceneEdit::Insert {
                        parent: Some(id),
                        index: usize::MAX,
                        data,
                    }),
            );
            cages.extend(inputs(node));
            continue;
        }
        let mesh = if node.data.kind.is_generator() {
            // A disabled generator shows its children as they are: nothing to bake.
            if enabled_generator(id).is_none() {
                continue;
            }
            let Some(mesh) = grid4d_eval::generator_mesh(&doc.scene, id, frame) else {
                continue;
            };
            cages.extend(inputs(node));
            mesh
        } else {
            let Some(mesh) = grid4d_eval::editable_mesh(&kind) else {
                continue;
            };
            mesh
        };
        let mut data = node.data.clone();
        data.kind = ObjectKind::polygon(MeshHandle::new(mesh));
        edits.push(SceneEdit::SetData {
            id,
            data: without_lost_tracks(data),
        });
    }
    edits.extend(cages.into_iter().map(|root| SceneEdit::Detach { root }));
    edits
}

/// Sizes the deformer in `data` to `parent`'s extent at the current frame and
/// centres it there, unrotated. `false` (nothing changed) for other objects
/// or a parent without an extent (a Null).
fn fit_to_parent(doc: &Document, parent: NodeId, data: &mut NodeData) -> bool {
    let frame = f64::from(doc.time.frame());
    let Some(node) = doc.scene.get(parent) else {
        return false;
    };
    let bounds = node.data.kind_at(frame).local_bounds().or_else(|| {
        let (min, max) = grid4d_eval::generator_mesh(&doc.scene, parent, frame)?.bounds()?;
        Some(grid4d_scene::Aabb { min, max })
    });
    let Some(bounds) = bounds.filter(|b| !b.is_empty()) else {
        return false;
    };
    // A deformer plugin's box is the parameter it names.
    if let ObjectKind::Plugin(object) = &mut data.kind {
        let Some(plugin) = grid4d_plugin::registry().deformer(&object.type_id) else {
            return false;
        };
        plugin.set_size(object, (bounds.max - bounds.min).max(Vec3::splat(1.0)));
        data.transform = Transform {
            position: bounds.center(),
            ..Transform::IDENTITY
        };
        return true;
    }
    let Some(size) = data.kind.deformer_size_mut() else {
        return false;
    };
    let before = *size;
    *size = (bounds.max - bounds.min).max(Vec3::splat(1.0));
    let after = *size;
    // An FFD's lattice scales with its box.
    if let ObjectKind::Ffd { lattice, .. } = &mut data.kind {
        let points = grid4d_geom::deform::scale_lattice(&lattice.spline().points, before, after);
        *lattice = lattice.edited(|s| s.points = points);
    }
    data.transform = Transform {
        position: bounds.center(),
        ..Transform::IDENTITY
    };
    true
}

/// `data` without the tracks of parameters its (new) kind does not have.
fn without_lost_tracks(mut data: NodeData) -> NodeData {
    let kept = data.kind.params();
    data.tracks.retain(|t| match t.param {
        Param::Object(p) => kept.contains(&p),
        Param::Position(_) | Param::Rotation(_) | Param::Scale(_) => true,
    });
    data
}

/// Default data for a new object, named uniquely, C4D-style.
fn new_object_data(tag: ObjectKindTag, scene: &Scene, camera: &OrbitCamera) -> NodeData {
    let mut data = NodeData::new(scene.unique_name(tag.label()), tag.default_kind());
    match tag {
        // Like C4D: a new camera clones the current view (place, bank, field of view).
        ObjectKindTag::Camera => {
            data.transform = Transform::from_matrix(&camera.eye_matrix());
            data.kind = ObjectKind::Camera {
                fov_y_deg: camera.fov_y.to_degrees(),
            };
            data.display_color = [0.3, 0.3, 0.32];
        }
        ObjectKindTag::Light => {
            data.transform.position = Vec3::new(0.0, 400.0, 0.0);
            data.display_color = [1.0, 0.95, 0.85];
        }
        // At infinity: where it stands does not matter.
        ObjectKindTag::Sky | ObjectKindTag::Environment => data.display_color = [0.6, 0.8, 1.0],
        // Light lines show on the dark viewport.
        ObjectKindTag::Circle
        | ObjectKindTag::Rectangle
        | ObjectKindTag::Arc
        | ObjectKindTag::NSide
        | ObjectKindTag::Star
        | ObjectKindTag::Helix
        | ObjectKindTag::Flower
        | ObjectKindTag::Text
        | ObjectKindTag::Spline => data.display_color = [0.9, 0.9, 0.92],
        // A light violet box, apart from splines and selections.
        ObjectKindTag::Bend
        | ObjectKindTag::Twist
        | ObjectKindTag::Taper
        | ObjectKindTag::Bulge
        | ObjectKindTag::Ffd => {
            data.display_color = [0.55, 0.4, 0.9];
        }
        // A light orange marker and falloff.
        ObjectKindTag::PlainEffector | ObjectKindTag::RandomEffector => {
            data.display_color = [0.95, 0.6, 0.35];
        }
        ObjectKindTag::Null
        | ObjectKindTag::Cube
        | ObjectKindTag::Sphere
        | ObjectKindTag::Cylinder
        | ObjectKindTag::Plane
        | ObjectKindTag::Cone
        | ObjectKindTag::Torus
        | ObjectKindTag::Disc
        | ObjectKindTag::Capsule
        | ObjectKindTag::Plugin
        | ObjectKindTag::Polygon
        | ObjectKindTag::SubdivisionSurface
        | ObjectKindTag::Extrude
        | ObjectKindTag::Lathe
        | ObjectKindTag::Sweep
        | ObjectKindTag::Loft
        | ObjectKindTag::Cloner => {}
    }
    data
}

/// `true` if applying the value edit `edit` would change nothing, i.e. a live
/// interaction ended where it started. A missing node counts as unchanged, so a
/// commit for an object deleted mid-drag records nothing.
fn is_current_state(doc: &Document, edit: &SceneEdit) -> bool {
    match edit {
        SceneEdit::SetData { id, data } => doc.scene.get(*id).is_none_or(|node| node.data == *data),
        SceneEdit::SetTime(settings) => doc.time.settings() == *settings,
        SceneEdit::SetName(name) => doc.name == *name,
        SceneEdit::SetMaterials(materials) => doc.scene.materials() == materials,
        SceneEdit::SetTextures(textures) => doc.scene.textures() == textures,
        SceneEdit::SetLayers(layers) => doc.scene.layers() == layers,
        SceneEdit::SetTakes(takes) => doc.takes == *takes,
        SceneEdit::SwitchTake(take) => doc.takes.active() == *take,
        SceneEdit::SetRender(settings) => doc.render == *settings,
        SceneEdit::SetSimulation(settings) => doc.simulation == *settings,
        // Structural changes are never made live; record them as given.
        SceneEdit::Insert { .. } | SceneEdit::Detach { .. } | SceneEdit::Attach { .. } | SceneEdit::Move { .. } => {
            false
        }
    }
}

fn has_selected_ancestor(scene: &Scene, selection: &Selection, id: NodeId) -> bool {
    let mut current = scene.get(id).and_then(Node::parent);
    while let Some(parent) = current {
        if selection.contains(parent) {
            return true;
        }
        current = scene.get(parent).and_then(Node::parent);
    }
    false
}

/// Icon tint for an object type (shared by Object Manager, menus and toolbar).
#[must_use]
pub fn tag_color(tag: ObjectKindTag) -> egui::Color32 {
    match tag {
        ObjectKindTag::Null => palette().icon_null,
        ObjectKindTag::Camera => palette().icon_camera,
        ObjectKindTag::Light | ObjectKindTag::Sky | ObjectKindTag::Environment => palette().icon_light,
        ObjectKindTag::SubdivisionSurface
        | ObjectKindTag::Extrude
        | ObjectKindTag::Lathe
        | ObjectKindTag::Sweep
        | ObjectKindTag::Loft
        | ObjectKindTag::Cloner => palette().icon_generator,
        ObjectKindTag::Circle
        | ObjectKindTag::Rectangle
        | ObjectKindTag::Arc
        | ObjectKindTag::NSide
        | ObjectKindTag::Star
        | ObjectKindTag::Helix
        | ObjectKindTag::Flower
        | ObjectKindTag::Text
        | ObjectKindTag::Spline => palette().icon_spline,
        ObjectKindTag::Plugin => palette().icon_generator,
        ObjectKindTag::Bend
        | ObjectKindTag::Twist
        | ObjectKindTag::Taper
        | ObjectKindTag::Bulge
        | ObjectKindTag::Ffd => palette().icon_deformer,
        ObjectKindTag::PlainEffector | ObjectKindTag::RandomEffector => palette().icon_effector,
        ObjectKindTag::Cube
        | ObjectKindTag::Sphere
        | ObjectKindTag::Cylinder
        | ObjectKindTag::Plane
        | ObjectKindTag::Cone
        | ObjectKindTag::Torus
        | ObjectKindTag::Disc
        | ObjectKindTag::Capsule
        | ObjectKindTag::Polygon => palette().icon_primitive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid4d_render::FilterItem;
    use grid4d_scene::{Axis, Param};

    /// A headless viewport: without a wgpu render state it only keeps the camera.
    fn viewport() -> ViewportPanel {
        ViewportPanel::new(None)
    }

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene
            .iter()
            .find(|(_, n)| n.data.name == name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no node named {name}"))
    }

    fn run(doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel, commands: &[EditorCommand]) {
        for command in commands {
            editor.commands.push(command.clone());
        }
        assert!(apply_pending(doc, editor, viewport));
        assert!(editor.commands.is_empty());
    }

    #[test]
    fn apply_pending_reports_whether_anything_ran() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        assert!(!apply_pending(&mut doc, &mut editor, &mut vp));
        editor.commands.push(EditorCommand::DeselectAll);
        assert!(apply_pending(&mut doc, &mut editor, &mut vp));
        assert!(!apply_pending(&mut doc, &mut editor, &mut vp));
    }

    fn key_frames_of(doc: &Document, id: NodeId) -> Vec<f64> {
        let mut frames = Vec::new();
        let mut selection = Selection::default();
        selection.set_single(id);
        crate::keys::key_frames(&doc.scene, &selection, &mut frames);
        frames
    }

    #[test]
    fn recording_moving_and_deleting_keys_are_one_step_each() {
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_all([cube, sphere]);
        let before = doc.scene.get(cube).unwrap().data.clone();

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RecordKeys { at: 0 }]);
        doc.time.set_frame(20);
        // Move the cube at frame 20 (temporary), then record it there.
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        let mut shown = data.transform_at(20.0);
        shown.position.y = 300.0;
        data.set_transform_at(&shown, 20.0, false);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RecordKeys { at: 20 }]);
        let data = &doc.scene.get(cube).unwrap().data;
        assert_eq!(data.tracks.len(), 9);
        assert_eq!(data.value_at(Param::Position(Axis::Y), 20.0), 300.0);
        assert_eq!(data.value_at(Param::Position(Axis::Y), 0.0), 100.0);
        assert_eq!(key_frames_of(&doc, sphere), [0.0, 20.0]);
        assert_eq!(editor.history.undo_label(), Some("Record Keys"));

        // Move the keys at 20 to 30, then delete those at 0.
        editor.keys.frames = vec![20.0];
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MoveKeys { by: 10 }]);
        assert_eq!(key_frames_of(&doc, cube), [0.0, 30.0]);
        assert_eq!(editor.keys.frames, [30.0], "the selection moves with its keys");
        editor.keys.frames = vec![0.0];
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteKeys]);
        assert_eq!(key_frames_of(&doc, cube), [30.0]);
        assert!(editor.keys.frames.is_empty());

        // One step each: undoing the delete and the move brings back both keys,
        // undoing both recordings the cube as it was.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(key_frames_of(&doc, cube), [0.0, 20.0]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(doc.scene.get(cube).unwrap().data, before);
    }

    #[test]
    fn record_follows_the_channel_toggles_and_needs_a_selection() {
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        let revision = doc.scene.revision();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RecordKeys { at: 0 }]);
        assert_eq!(doc.scene.revision(), revision, "nothing selected, nothing recorded");
        editor.selection.set_single(cube);
        editor.animation.position = false;
        editor.animation.scale = false;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RecordKeys { at: 5 }]);
        let tracks = &doc.scene.get(cube).unwrap().data.tracks;
        assert_eq!(tracks.len(), 3);
        assert!(tracks.iter().all(|t| matches!(t.param, Param::Rotation(_))));
    }

    #[test]
    fn material_tags_layer_and_bake_their_projection_one_step_each() {
        use grid4d_scene::{MaterialTag, Projection, Tag, TagKind, Tags};
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::NewMaterial, EditorCommand::NewMaterial],
        );
        let ids = doc.scene.materials().ids().to_vec();
        editor.selection.set_single(cube);
        let assign = |material, layer| EditorCommand::AssignMaterial {
            material,
            nodes: vec![cube],
            layer,
        };
        let materials = |doc: &Document| {
            doc.scene
                .get(cube)
                .unwrap()
                .data
                .tags
                .material_tags()
                .map(|t| (t.material, t.projection))
                .collect::<Vec<_>>()
        };
        run(&mut doc, &mut editor, &mut vp, &[assign(ids[0], false)]);
        // A second tag on top (Shift), then a plain drop replaces the top one's material.
        run(&mut doc, &mut editor, &mut vp, &[assign(ids[1], true)]);
        assert_eq!(
            materials(&doc),
            [(Some(ids[0]), Projection::Uv), (Some(ids[1]), Projection::Uv)]
        );
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        let Some(Tag::Material(top)) = data.tags.last_mut() else {
            panic!("the new tag is last")
        };
        top.projection = Projection::Box;
        run(&mut doc, &mut editor, &mut vp, &[assign(ids[0], false)]);
        assert_eq!(
            materials(&doc),
            [(Some(ids[0]), Projection::Uv), (Some(ids[0]), Projection::Box)],
            "the projection stays"
        );

        // Adding a Material tag adds another and selects it; deleting removes only it.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Material.default_tag())],
        );
        assert_eq!(materials(&doc).len(), 3);
        let (tag, index) = editor.selected_tag(&doc).unwrap();
        assert_eq!((tag.kind, tag.nth, index), (TagKind::Material, 2, 2));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::DeleteTag(crate::TagRef { nth: 0, ..tag })],
        );
        assert_eq!(
            materials(&doc),
            [(Some(ids[0]), Projection::Box), (None, Projection::Uv)]
        );

        // Bake to UVs needs a polygon object: nothing on the primitive.
        let boxed = crate::TagRef {
            node: cube,
            kind: TagKind::Material,
            nth: 0,
        };
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::BakeMaterialTag(boxed)]);
        assert_eq!(editor.history.undo_len(), steps);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let before = doc.scene.get(cube).unwrap().data.clone();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::BakeMaterialTag(boxed)]);
        assert_eq!(editor.history.undo_label(), Some("Bake to UVs"));
        let data = &doc.scene.get(cube).unwrap().data;
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!("a polygon object")
        };
        let ObjectKind::Polygon { mesh: old, .. } = &before.kind else {
            unreachable!()
        };
        let tag = MaterialTag {
            projection: Projection::Box,
            ..MaterialTag::new(None)
        };
        let expected = grid4d_geom::uv::project(Projection::Box, old.mesh(), tag.to_unit(old.mesh().bounds()).unwrap());
        assert_eq!(mesh.mesh().uvs().unwrap(), expected);
        assert_eq!(
            materials(&doc)[0],
            (Some(ids[0]), Projection::Uv),
            "the tag reads the UVs now"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(cube).unwrap().data, before);
    }

    #[test]
    fn materials_are_made_assigned_and_deleted_one_step_each() {
        use grid4d_scene::{Tag, Tags};
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::NewMaterial, EditorCommand::NewMaterial],
        );
        let ids = doc.scene.materials().ids().to_vec();
        assert_eq!(ids.len(), 2);
        let names: Vec<&str> = doc.scene.materials().iter().map(|(_, m)| m.name.as_str()).collect();
        assert_eq!(names, ["Mat", "Mat.1"]);
        assert_eq!(editor.materials.selected, Some(ids[1]), "the new one is selected");
        assert!(editor.materials.shown, "and shown in the Attribute Manager");

        // Assigning adds a tag, or changes the one there.
        let assign = |material| EditorCommand::AssignMaterial {
            material,
            nodes: vec![cube, sphere],
            layer: false,
        };
        run(&mut doc, &mut editor, &mut vp, &[assign(ids[0])]);
        run(&mut doc, &mut editor, &mut vp, &[assign(ids[1])]);
        for id in [cube, sphere] {
            let tags = &doc.scene.get(id).unwrap().data.tags;
            assert_eq!(tags.len(), 1, "one Material tag");
            assert_eq!(tags.material(), Some(ids[1]));
        }
        assert_eq!(editor.history.undo_label(), Some("Assign Material"));

        // Deleting leaves the tag empty-handed; undo brings the link back.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteMaterial(ids[1])]);
        assert!(!doc.scene.materials().contains(ids[1]));
        assert_eq!(editor.materials.selected, None);
        assert_eq!(doc.scene.effective_material(cube), None);
        assert!(matches!(doc.scene.get(cube).unwrap().data.tags[..], [Tag::Material(_)]));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.effective_material(cube), Some(ids[1]));

        let steps = editor.history.undo_len();
        assert_eq!(steps, 4, "two new materials and two assignments");
        for _ in 0..steps {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        }
        assert!(doc.scene.materials().is_empty());
        assert!(doc.scene.get(cube).unwrap().data.tags.is_empty());
    }

    #[test]
    fn a_live_change_of_several_objects_is_one_undo_step() {
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        let before: Vec<(NodeId, NodeData)> = [cube, sphere]
            .into_iter()
            .map(|id| (id, doc.scene.get(id).unwrap().data.clone()))
            .collect();
        for &(id, _) in &before {
            doc.scene.get_mut(id).unwrap().data.transform.position.x += 10.0;
        }
        let inverses = before
            .iter()
            .map(|(id, data)| SceneEdit::SetData {
                id: *id,
                data: data.clone(),
            })
            .collect();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLiveAll {
                inverses,
                label: "Move Keys",
            }],
        );
        assert_eq!(editor.history.undo_len(), 1);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        for (id, data) in &before {
            assert_eq!(&doc.scene.get(*id).unwrap().data, data);
        }
        // Nothing changed: nothing recorded.
        let inverses = vec![SceneEdit::SetData {
            id: cube,
            data: doc.scene.get(cube).unwrap().data.clone(),
        }];
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLiveAll {
                inverses,
                label: "Move Keys",
            }],
        );
        assert_eq!(editor.history.undo_len(), 0);
    }

    #[test]
    fn temporary_values_are_dropped_when_the_frame_changes() {
        let mut doc = Document::with_demo_scene();
        let (mut editor, mut vp) = (EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RecordKeys { at: 0 }]);
        let data = &mut doc.scene.get_mut(cube).unwrap().data;
        let mut shown = data.transform_at(0.0);
        shown.position.x = 55.0;
        data.set_transform_at(&shown, 0.0, false);
        assert!(!apply_pending(&mut doc, &mut editor, &mut vp), "same frame: kept");
        assert!(doc.scene.get(cube).unwrap().data.unkeyed.is_some());
        doc.time.set_frame(1);
        assert!(apply_pending(&mut doc, &mut editor, &mut vp), "a new frame drops it");
        assert_eq!(doc.scene.get(cube).unwrap().data.unkeyed, None);
        doc.time.set_frame(0);
        assert_eq!(
            doc.scene
                .get(cube)
                .unwrap()
                .data
                .value_at(Param::Position(Axis::X), 0.0),
            0.0
        );
    }

    #[test]
    fn create_object_names_uniquely_and_selects_it() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::CreateObject(ObjectKindTag::Cube),
            ],
        );
        assert_eq!(doc.scene.len(), 2);
        let second = id_of(&doc, "Cube.1");
        assert!(doc.scene.contains(id_of(&doc, "Cube")));
        assert_eq!(editor.selection.as_slice(), &[second]);
        assert_eq!(
            doc.scene.get(second).unwrap().data.kind,
            ObjectKindTag::Cube.default_kind()
        );
    }

    #[test]
    fn apply_mesh_tool_is_one_undo_step_in_a_component_mode() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let cube = id_of(&doc, "Cube");
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let faces = |doc: &Document| match &doc.scene.get(cube).unwrap().data.kind {
            ObjectKind::Polygon { mesh, .. } => mesh.mesh().face_count(),
            _ => 0,
        };
        editor.tool = crate::Tool::Extrude;
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(
            (faces(&doc), editor.history.undo_len()),
            (6, steps),
            "Model mode: nothing"
        );

        editor.mode = EditMode::Polygons;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(faces(&doc), 6 + 6 * 4, "every face is its own piece at 90°");
        assert_eq!(editor.history.undo_label(), Some("Extrude"));
        assert_eq!(editor.history.undo_len(), steps + 1);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(faces(&doc), 6);
    }

    fn mesh_of(doc: &Document, id: NodeId) -> MeshHandle {
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("not a polygon object")
        };
        mesh.clone()
    }

    fn select_in(
        doc: &mut Document,
        editor: &mut EditorState,
        vp: &mut ViewportPanel,
        id: NodeId,
        component: Component,
        hits: Vec<u32>,
    ) {
        run(
            doc,
            editor,
            vp,
            &[EditorCommand::SelectComponents {
                node: id,
                component,
                hits,
                mode: SelectMode::Replace,
            }],
        );
    }

    #[test]
    fn deleting_components_rebuilds_the_mesh_as_one_step() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let before = mesh_of(&doc, id);
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Point, vec![0]);
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        let after = mesh_of(&doc, id);
        assert_eq!((after.mesh().point_count(), after.mesh().face_count()), (7, 3));
        assert!(
            selected(&doc, id, Component::Point).is_empty(),
            "the deleted point was the selection"
        );
        assert_eq!(editor.history.undo_len(), steps + 1);
        assert_eq!(editor.history.undo_label(), Some("Delete Points"));
        assert!(doc.scene.contains(id), "the object stays");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(mesh_of(&doc, id).id(), before.id());
        assert_eq!(
            selected(&doc, id, Component::Point),
            [0],
            "undo brings the selection back too"
        );

        // Nothing selected: nothing happens.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(
            (mesh_of(&doc, id).mesh().face_count(), editor.history.undo_len()),
            (6, steps)
        );
    }

    #[test]
    fn melt_dissolve_and_connect_are_one_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let faces = |doc: &Document| mesh_of(doc, id).mesh().face_count();
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Polygon, vec![0, 1]);
        let adjacent = {
            let mesh = mesh_of(&doc, id);
            let shared = mesh
                .mesh()
                .face(0)
                .iter()
                .filter(|p| mesh.mesh().face(1).contains(p))
                .count();
            shared == 2
        };
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MeltSelected]);
        assert_eq!(faces(&doc), if adjacent { 5 } else { 6 });
        if adjacent {
            assert_eq!(editor.history.undo_label(), Some("Melt"));
            assert_eq!(
                selected(&doc, id, Component::Polygon).len(),
                1,
                "the merged polygon stays selected"
            );
        }

        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        // Two opposite corners of one face.
        let mesh = mesh_of(&doc, id);
        let face = mesh.mesh().face(0).to_vec();
        select_in(
            &mut doc,
            &mut editor,
            &mut vp,
            id,
            Component::Point,
            vec![face[0], face[2]],
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ConnectSelected]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 7, "the face split in two");
        assert_eq!(editor.history.undo_label(), Some("Connect"));
        assert_eq!(mesh_of(&doc, id).topology().edge_count(), 12 + 1, "one new edge");
        assert!(
            selected(&doc, id, Component::Point).len() == 2,
            "the points stay selected"
        );

        editor.mode = EditMode::Edges;
        let topology = mesh_of(&doc, id).topology().clone();
        let new_edge = topology.edge_count() - 1;
        let corner = topology.edge_corners(new_edge)[0];
        select_in(
            &mut doc,
            &mut editor,
            &mut vp,
            id,
            Component::Edge,
            vec![u32::try_from(new_edge).unwrap()],
        );
        assert!(!selected(&doc, id, Component::Edge).is_empty(), "{corner}");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DissolveSelected]);
        assert_eq!(
            mesh_of(&doc, id).mesh().face_count(),
            6,
            "dissolving the new edge undoes the split"
        );
        assert_eq!(editor.history.undo_label(), Some("Dissolve"));
    }

    #[test]
    fn loop_and_line_cuts_are_one_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Edges);
        let mesh = mesh_of(&doc, id);
        // A vertical edge: its ring runs round the four sides.
        let vertical = (0..mesh.topology().edge_count())
            .find(|&e| {
                let [a, b] = mesh.topology().edges()[e];
                let (pa, pb) = (mesh.mesh().points()[a as usize], mesh.mesh().points()[b as usize]);
                (pa.x - pb.x).abs() < 1e-3 && (pa.z - pb.z).abs() < 1e-3
            })
            .unwrap();
        let cut = EditorCommand::LoopCut {
            node: id,
            edge: u32::try_from(vertical).unwrap(),
            fractions: vec![0.5],
        };
        run(&mut doc, &mut editor, &mut vp, &[cut]);
        let after = mesh_of(&doc, id);
        assert_eq!((after.mesh().point_count(), after.mesh().face_count()), (12, 10));
        assert_eq!(editor.history.undo_label(), Some("Loop Cut"));
        assert_eq!(
            selected(&doc, id, Component::Edge).len(),
            8,
            "the 4 new edges, from both sides"
        );

        // A line straight down through the moved cube, in world space.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        doc.scene.get_mut(id).unwrap().data.transform.position = Vec3::new(500.0, 0.0, 0.0);
        let eye = Vec3::new(500.0, 1000.0, 0.0);
        let line = EditorCommand::LineCut {
            node: id,
            eye,
            from: Vec3::new(500.0, 0.0, -300.0) - eye,
            to: Vec3::new(500.0, 0.0, 300.0) - eye,
        };
        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&line));
        assert_eq!(
            mesh_of(&doc, id).mesh().face_count(),
            6 + 1,
            "visible only: the top face splits, the sides are seen edge-on, the bottom is hidden"
        );
        assert_eq!(editor.history.undo_label(), Some("Line Cut"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        editor.modeling_settings.line_cut.visible_only = false;
        run(&mut doc, &mut editor, &mut vp, &[line]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 6 + 4, "all the way round");
    }

    /// A polygon object made from a cube with 3 segments per side (54 quads).
    fn segmented_cube(mode: EditMode) -> (Document, EditorState, ViewportPanel, NodeId) {
        let mesh = grid4d_geom::primitives::cube(Vec3::splat(300.0), [3, 3, 3]);
        let mut doc = Document::new("segmented");
        let id = doc
            .scene
            .insert(Node::new("Cube", ObjectKind::polygon(MeshHandle::new(mesh))));
        let mut editor = EditorState {
            mode,
            ..EditorState::default()
        };
        editor.selection.set_single(id);
        (doc, editor, viewport(), id)
    }

    /// The face of `id` with `normal` whose centre lies on the axis through it.
    fn centre_face(doc: &Document, id: NodeId, normal: Vec3) -> u32 {
        let mesh = mesh_of(doc, id);
        let f = (0..mesh.mesh().face_count())
            .find(|&f| {
                let c = mesh.mesh().face_center(f) * (Vec3::ONE - normal.abs());
                mesh.mesh().face_normal(f).abs_diff_eq(normal, 1e-4) && c.length() < 1e-3
            })
            .unwrap();
        u32::try_from(f).unwrap()
    }

    #[test]
    fn bevel_bridge_weld_and_optimize_are_one_step_each() {
        // Bevel: Apply on one selected edge.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Edges);
        editor.tool = crate::Tool::Bevel;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(
            mesh_of(&doc, id).mesh().face_count(),
            6,
            "nothing selected: nothing to bevel"
        );
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Edge, vec![0]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 7);
        assert_eq!(editor.history.undo_label(), Some("Bevel"));
        assert_eq!(
            selected(&doc, id, Component::Edge).len(),
            4,
            "the strip's two long edges, from both sides"
        );

        // Bridge: the middles of top and bottom, through the cube.
        let (mut doc, mut editor, mut vp, id) = segmented_cube(EditMode::Polygons);
        let (top, bottom) = (centre_face(&doc, id, Vec3::Y), centre_face(&doc, id, -Vec3::Y));
        let bridge = EditorCommand::Bridge {
            node: id,
            component: Component::Polygon,
            from: top,
            to: bottom,
        };
        run(&mut doc, &mut editor, &mut vp, &[bridge]);
        let mesh = mesh_of(&doc, id);
        assert_eq!(mesh.mesh().face_count(), 54 - 2 + 4);
        assert!(mesh.topology().is_closed());
        assert_eq!(editor.history.undo_label(), Some("Bridge"));
        assert_eq!(
            selected(&doc, id, Component::Polygon).len(),
            4,
            "the new walls are selected"
        );
        // The same through Apply on the two selected polygons.
        let (mut doc, mut editor, mut vp, id) = segmented_cube(EditMode::Polygons);
        let pair = vec![centre_face(&doc, id, Vec3::Y), centre_face(&doc, id, -Vec3::Y)];
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Polygon, pair);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::BridgeSelected]);
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 56);

        // Weld two points of one edge onto the first: the cube loses a point.
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let [a, b] = mesh_of(&doc, id).topology().edges()[0];
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Point, vec![a, b]);
        let kept = mesh_of(&doc, id).mesh().points()[usize::try_from(a).unwrap()];
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Weld {
                node: id,
                onto: Some(a),
            }],
        );
        let welded = mesh_of(&doc, id);
        assert_eq!(welded.mesh().point_count(), 7);
        assert!(welded.mesh().points().contains(&kept), "onto the clicked point");
        assert_eq!(editor.history.undo_label(), Some("Weld"));

        // Optimize closes an editable cylinder's separate caps.
        let mut doc = Document::new("cylinder");
        let id = doc
            .scene
            .insert(Node::new("Cylinder", ObjectKindTag::Cylinder.default_kind()));
        let mut editor = EditorState::default();
        let mut vp = viewport();
        editor.selection.set_single(id);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let before = mesh_of(&doc, id).mesh().point_count();
        assert!(!mesh_of(&doc, id).topology().is_closed(), "separate caps");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Optimize]);
        let after = mesh_of(&doc, id);
        assert!(after.mesh().point_count() < before);
        assert!(after.topology().is_closed());
        assert_eq!(editor.history.undo_label(), Some("Optimize"));
    }

    #[test]
    fn uv_tools_are_one_undo_step_each_and_seams_survive_modelling() {
        use crate::uv_tools::UvOp;
        let mut doc = Document::new("cube");
        let id = doc.scene.insert(Node::new("Cube", ObjectKindTag::Cube.default_kind()));
        let mut editor = EditorState::default();
        let mut vp = viewport();
        editor.selection.set_single(id);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let before = mesh_of(&doc, id);

        // Unwrap: new UVs, unmirrored, in the square; undo gives the old mesh back.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::UvCommand(UvOp::Unwrap)],
        );
        assert_eq!(editor.history.undo_label(), Some("Unwrap UVs"));
        let unwrapped = mesh_of(&doc, id);
        assert_ne!(unwrapped.id(), before.id());
        assert_eq!(grid4d_geom::uv::flipped_faces(unwrapped.mesh()), 0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(mesh_of(&doc, id).id(), before.id());
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(mesh_of(&doc, id).id(), unwrapped.id());

        // Seams from the selected edges, kept through an Extrude.
        editor.mode = EditMode::Edges;
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Edge, vec![0, 1]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::UvCommand(UvOp::MarkSeams)],
        );
        assert_eq!(editor.history.undo_label(), Some("Mark Seams"));
        let seams = |doc: &Document| {
            let ObjectKind::Polygon { components, .. } = &doc.scene.get(id).unwrap().data.kind else {
                unreachable!()
            };
            components.state().seams.len()
        };
        assert_eq!(seams(&doc), 4, "two edges, both sides");
        editor.mode = EditMode::Polygons;
        select_in(&mut doc, &mut editor, &mut vp, id, Component::Polygon, vec![5]);
        editor.tool = crate::Tool::Extrude;
        editor.modeling_settings.extrude.offset = 20.0;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ApplyMeshTool]);
        assert!(seams(&doc) >= 2, "the seams away from the extruded side stay");
        // Undo the Extrude, the polygon selection, then the seams.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(editor.history.undo_label(), Some("Mark Seams"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(seams(&doc), 0);

        // A tool with nothing to change adds no step.
        let steps = editor.history.undo_len();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::UvCommand(UvOp::ClearSeams)],
        );
        assert_eq!(editor.history.undo_len(), steps);
    }

    #[test]
    fn new_camera_is_placed_at_the_current_view() {
        let (mut doc, mut editor, mut vp) = (Document::default(), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Camera)],
        );
        let camera = doc.scene.get(id_of(&doc, "Camera")).unwrap();
        assert!(camera.data.transform.position.abs_diff_eq(vp.camera().eye(), 1e-3));
    }

    #[test]
    fn delete_removes_subtrees_and_clears_selection() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let before = doc.scene.len();
        let (group, child) = (id_of(&doc, "Group"), id_of(&doc, "Box"));
        // Parent and child both selected: the child goes with the parent, no double delete.
        editor.selection.set_all([child, group]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(doc.scene.len(), before - 3, "Group, Cylinder and Box");
        assert!(!doc.scene.contains(group) && !doc.scene.contains(child));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn make_editable_bakes_primitives_into_meshes_as_one_undo_step() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, light, group) = (id_of(&doc, "Cube"), id_of(&doc, "Light"), id_of(&doc, "Group"));
        let cube_before = doc.scene.get(cube).unwrap().data.clone();
        let light_before = doc.scene.get(light).unwrap().data.clone();
        editor.selection.set_all([cube, light, group]);
        assert!(can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);

        let data = doc.scene.get(cube).unwrap().data.clone();
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!("not converted: {:?}", data.kind)
        };
        assert_eq!((mesh.mesh().point_count(), mesh.mesh().face_count()), (8, 6));
        let topology = Topology::build(mesh.mesh());
        assert!(topology.is_closed() && topology.is_manifold(), "a closed, manifold box");
        let bounds = mesh.bounds().unwrap();
        assert!(
            bounds.max.abs_diff_eq(Vec3::splat(100.0), 1e-4),
            "the 200 cm size is in the points"
        );
        assert_eq!((&data.name, data.transform), (&cube_before.name, cube_before.transform));
        assert_eq!(doc.scene.get(light).unwrap().data, light_before, "lights stay lights");
        assert_eq!(editor.history.undo_label(), Some("Make Editable"));
        assert!(!can_make_editable(&doc, &editor.selection), "nothing left to convert");

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(cube).unwrap().data, cube_before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        let ObjectKind::Polygon { mesh: again, .. } = &doc.scene.get(cube).unwrap().data.kind else {
            panic!("redo did not restore the mesh")
        };
        assert_eq!(
            again.id(),
            mesh.id(),
            "the same mesh comes back, so its GPU copy is reused"
        );
    }

    /// A document with one polygon cube (8 points, 6 quads), active, in `mode`.
    fn polygon_cube(mode: EditMode) -> (Document, EditorState, ViewportPanel, NodeId) {
        let mesh = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        let mut doc = Document::new("components");
        let id = doc
            .scene
            .insert(Node::new("Cube", ObjectKind::polygon(MeshHandle::new(mesh))));
        let mut editor = EditorState {
            mode,
            ..EditorState::default()
        };
        editor.selection.set_single(id);
        (doc, editor, viewport(), id)
    }

    fn selected(doc: &Document, id: NodeId, component: Component) -> Vec<usize> {
        let ObjectKind::Polygon { components, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("not a polygon object")
        };
        components.selected().get(component).iter().collect()
    }

    #[test]
    fn component_clicks_follow_the_modifiers_one_undo_step_each() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let click = |hits: &[u32], mode| EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits: hits.to_vec(),
            mode,
        };
        run(&mut doc, &mut editor, &mut vp, &[click(&[2], SelectMode::Replace)]);
        assert_eq!(selected(&doc, id, Component::Point), [2]);
        assert_eq!(editor.history.undo_label(), Some("Select Points"));
        run(&mut doc, &mut editor, &mut vp, &[click(&[5], SelectMode::Add)]);
        run(&mut doc, &mut editor, &mut vp, &[click(&[2], SelectMode::Remove)]);
        assert_eq!(selected(&doc, id, Component::Point), [5]);
        run(&mut doc, &mut editor, &mut vp, &[click(&[], SelectMode::Replace)]);
        assert!(selected(&doc, id, Component::Point).is_empty(), "empty space deselects");
        assert_eq!(editor.history.undo_len(), 4);
        run(&mut doc, &mut editor, &mut vp, &[click(&[], SelectMode::Replace)]);
        assert_eq!(editor.history.undo_len(), 4, "no change, no step");

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(selected(&doc, id, Component::Point), [2, 5]);
    }

    #[test]
    fn an_edge_is_selected_through_all_of_its_corners() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Edges);
        let pick = |e, mode| EditorCommand::SelectComponents {
            node: id,
            component: Component::Edge,
            hits: vec![e],
            mode,
        };
        run(&mut doc, &mut editor, &mut vp, &[pick(3, SelectMode::Replace)]);
        assert_eq!(
            selected(&doc, id, Component::Edge).len(),
            2,
            "one corner in each of its two faces"
        );
        run(&mut doc, &mut editor, &mut vp, &[pick(3, SelectMode::Toggle)]);
        assert!(selected(&doc, id, Component::Edge).is_empty());
    }

    #[test]
    fn shift_and_ctrl_on_a_mode_convert_the_selection() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        let select = EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits: vec![0],
            mode: SelectMode::Replace,
        };
        let convert = |tolerant| EditorCommand::ConvertSelection {
            to: EditMode::Polygons,
            tolerant,
        };
        run(&mut doc, &mut editor, &mut vp, &[select.clone(), convert(true)]);
        assert_eq!(editor.mode, EditMode::Polygons);
        assert_eq!(
            selected(&doc, id, Component::Polygon).len(),
            3,
            "every face using the point"
        );
        assert_eq!(editor.history.undo_label(), Some("Convert Selection"));

        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Points);
        run(&mut doc, &mut editor, &mut vp, &[select, convert(false)]);
        assert_eq!(editor.mode, EditMode::Polygons);
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "strict: no face is fully selected"
        );
        assert_eq!(
            editor.history.undo_label(),
            Some("Select Points"),
            "nothing changed, no step"
        );
    }

    #[test]
    fn select_all_and_delete_respect_the_component_mode() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 2, 3, 4, 5]);
        assert_eq!(editor.selection.as_slice(), &[id], "the object selection is untouched");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeselectAll]);
        assert!(selected(&doc, id, Component::Polygon).is_empty());
        assert_eq!(editor.history.undo_label(), Some("Deselect All"));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SelectAll, EditorCommand::DeleteSelected],
        );
        assert!(
            doc.scene.contains(id),
            "Delete removes the polygons, not the object, in a component mode"
        );
        assert_eq!(mesh_of(&doc, id).mesh().face_count(), 0);
        assert_eq!(editor.history.undo_label(), Some("Delete Polygons"));
    }

    #[test]
    fn grow_shrink_invert_and_connected_work_on_the_current_mode() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let select = |f: u32| EditorCommand::SelectComponents {
            node: id,
            component: Component::Polygon,
            hits: vec![f],
            mode: SelectMode::Replace,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(0), EditorCommand::GrowSelection],
        );
        assert_eq!(
            selected(&doc, id, Component::Polygon).len(),
            5,
            "the face and its 4 neighbours"
        );
        assert_eq!(editor.history.undo_label(), Some("Grow Selection"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::InvertSelection]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 1, "the opposite face");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectConnected]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6, "the whole cube");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ShrinkSelection]);
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6, "no border, no shrink");
    }

    #[test]
    fn hidden_components_drop_out_of_select_all_and_invert() {
        let (mut doc, mut editor, mut vp, id) = polygon_cube(EditMode::Polygons);
        let select = |f: u32| EditorCommand::SelectComponents {
            node: id,
            component: Component::Polygon,
            hits: vec![f],
            mode: SelectMode::Replace,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(2), EditorCommand::HideSelected],
        );
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "hidden ones are not selected"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 3, 4, 5]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(0), EditorCommand::InvertSelection],
        );
        assert_eq!(selected(&doc, id, Component::Polygon), [1, 3, 4, 5]);
        run(&mut doc, &mut editor, &mut vp, &[select(2)]);
        assert!(
            selected(&doc, id, Component::Polygon).is_empty(),
            "a hidden face cannot be picked"
        );
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::UnhideAll, EditorCommand::SelectAll],
        );
        assert_eq!(selected(&doc, id, Component::Polygon).len(), 6);
        // Undo walks back through every step, the hide included.
        for _ in 0..5 {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        }
        assert_eq!(selected(&doc, id, Component::Polygon), [0, 1, 3, 4, 5]);
    }

    #[test]
    fn make_editable_without_primitives_records_nothing() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        editor.selection.set_all([id_of(&doc, "Light"), id_of(&doc, "Group")]);
        assert!(!can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        assert_eq!(editor.history.undo_label(), None);
    }

    #[test]
    fn reparent_keeps_world_position_and_rejects_cycles() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, group, cylinder) = (id_of(&doc, "Cube"), id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let world_before = doc.scene.world_matrix(cube).w_axis.truncate();

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::Reparent {
                    node: cube,
                    new_parent: Some(group),
                },
                // Group under its own child: rejected, the hierarchy is unchanged.
                EditorCommand::Reparent {
                    node: group,
                    new_parent: Some(cylinder),
                },
            ],
        );
        assert_eq!(doc.scene.get(cube).unwrap().parent(), Some(group));
        assert_eq!(doc.scene.get(group).unwrap().parent(), None);
        let world_after = doc.scene.world_matrix(cube).w_axis.truncate();
        assert!(world_after.abs_diff_eq(world_before, 1e-3), "{world_after:?}");
    }

    #[test]
    fn select_modes_follow_c4d() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        let select = |id, mode| EditorCommand::Select { id, mode };

        run(&mut doc, &mut editor, &mut vp, &[select(cube, SelectMode::Replace)]);
        assert_eq!(editor.selection.as_slice(), &[cube]);

        run(&mut doc, &mut editor, &mut vp, &[select(sphere, SelectMode::Add)]);
        assert_eq!(editor.selection.as_slice(), &[cube, sphere]);
        assert_eq!(editor.selection.active(), Some(sphere));

        run(&mut doc, &mut editor, &mut vp, &[select(cube, SelectMode::Toggle)]);
        assert_eq!(editor.selection.as_slice(), &[sphere]);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(editor.selection.len(), doc.scene.len());

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeselectAll]);
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn commands_on_deleted_nodes_are_ignored() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::DeleteSelected,
                EditorCommand::Select {
                    id: sphere,
                    mode: SelectMode::Add,
                },
                EditorCommand::Reparent {
                    node: sphere,
                    new_parent: None,
                },
            ],
        );
        assert!(!doc.scene.contains(sphere));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn new_document_resets_scene_selection_and_view() {
        let (mut doc, mut editor, mut vp) = (Document::with_demo_scene(), EditorState::default(), viewport());
        editor.selection.set_single(id_of(&doc, "Cube"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::FrameSelected]);
        assert_ne!(*vp.camera(), OrbitCamera::default());

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewDocument]);
        assert!(doc.scene.is_empty());
        assert!(editor.selection.is_empty());
        assert_eq!(*vp.camera(), OrbitCamera::default());
    }

    // ── Undo / redo ─────────────────────────────────────────────────────────

    /// The user-visible document: hierarchy (pre-order ids, parents, depths) and data.
    fn snapshot(doc: &Document) -> Vec<(NodeId, Option<NodeId>, u32, NodeData)> {
        let mut out = Vec::new();
        doc.scene.walk(&mut Vec::new(), |id, node, _, depth| {
            out.push((id, node.parent(), depth, node.data.clone()));
            true
        });
        out
    }

    fn demo() -> (Document, EditorState, ViewportPanel) {
        (Document::with_demo_scene(), EditorState::default(), viewport())
    }

    #[test]
    fn create_undo_redo_reuses_the_node_and_restores_selection() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        let before = snapshot(&doc);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let cube = id_of(&doc, "Cube.1");
        assert_eq!(editor.selection.as_slice(), &[cube]);
        assert_eq!(editor.history.undo_label(), Some("Create Object"));
        let after = snapshot(&doc);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
        assert_eq!(editor.selection.as_slice(), &[sphere], "selection from before the step");
        assert_eq!(editor.history.redo_label(), Some("Create Object"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(snapshot(&doc), after);
        assert_eq!(editor.selection.as_slice(), &[cube], "same NodeId after redo");
    }

    #[test]
    fn delete_is_one_step_and_undo_restores_ids_order_and_selection() {
        let (mut doc, mut editor, mut vp) = demo();
        let (group, boxed, cube) = (id_of(&doc, "Group"), id_of(&doc, "Box"), id_of(&doc, "Cube"));
        // Box is inside Group: it goes with its parent instead of failing the batch.
        editor.selection.set_all([boxed, cube, group]);
        let before = snapshot(&doc);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(doc.scene.len(), before.len() - 4, "Cube, Group, Cylinder, Box");
        assert_eq!(editor.history.undo_len(), 1);
        assert_eq!(editor.history.undo_label(), Some("Delete"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
        assert_eq!(editor.selection.as_slice(), &[boxed, cube, group]);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert!(!doc.scene.contains(group) && !doc.scene.contains(cube));
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn reparent_undo_restores_parent_order_and_transform() {
        let (mut doc, mut editor, mut vp) = demo();
        let (cube, group) = (id_of(&doc, "Cube"), id_of(&doc, "Group"));
        let before = snapshot(&doc);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Reparent {
                node: cube,
                new_parent: Some(group),
            }],
        );
        assert_eq!(doc.scene.get(cube).unwrap().parent(), Some(group));
        assert_eq!(editor.history.undo_label(), Some("Reparent"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(snapshot(&doc), before);
    }

    #[test]
    fn rejected_and_no_op_commands_record_nothing() {
        let (mut doc, mut editor, mut vp) = demo();
        let (group, cylinder) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let before = snapshot(&doc);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::Undo, // nothing to undo yet
                EditorCommand::Redo,
                EditorCommand::DeleteSelected, // nothing selected
                EditorCommand::Reparent {
                    node: group,
                    new_parent: Some(cylinder), // cycle
                },
                EditorCommand::Reparent {
                    node: cylinder,
                    new_parent: Some(group), // already its parent
                },
                EditorCommand::SelectAll,
                EditorCommand::FrameSelected,
                EditorCommand::DeselectAll,
            ],
        );
        assert_eq!(snapshot(&doc), before);
        assert!(!editor.history.can_undo() && !editor.history.can_redo());
        assert_eq!(editor.history.state_id(), 0);
    }

    #[test]
    fn commit_live_records_live_edits() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        let before = doc.scene.get(sphere).unwrap().data.clone();
        let commit = EditorCommand::CommitLive {
            inverse: SceneEdit::SetData {
                id: sphere,
                data: before.clone(),
            },
            label: "Move",
        };

        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&commit));
        assert!(!editor.history.can_undo(), "unchanged data records nothing");

        // A live edit, as the gizmo makes while dragging, then the commit on release.
        doc.scene.get_mut(sphere).unwrap().data.transform.position.x += 25.0;
        let after = doc.scene.get(sphere).unwrap().data.clone();
        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&commit));
        assert_eq!(editor.history.undo_label(), Some("Move"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(sphere).unwrap().data, before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.scene.get(sphere).unwrap().data, after);
    }

    #[test]
    fn mixed_steps_undo_and_redo_exactly() {
        let (mut doc, mut editor, mut vp) = demo();
        let (sphere, group, light) = (id_of(&doc, "Sphere"), id_of(&doc, "Group"), id_of(&doc, "Light"));
        let mut states = vec![snapshot(&doc)];

        let mut step = |doc: &mut Document, editor: &mut EditorState, commands: &[EditorCommand]| {
            run(doc, editor, &mut vp, commands);
            states.push(snapshot(doc));
        };
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::CreateObject(ObjectKindTag::Null)],
        );
        let null = id_of(&doc, "Null");
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::Reparent {
                node: sphere,
                new_parent: Some(null),
            }],
        );
        step(
            &mut doc,
            &mut editor,
            &[
                EditorCommand::Select {
                    id: group,
                    mode: SelectMode::Replace,
                },
                EditorCommand::Select {
                    id: light,
                    mode: SelectMode::Add,
                },
                EditorCommand::DeleteSelected,
            ],
        );
        let before = doc.scene.get(null).unwrap().data.clone();
        doc.scene.get_mut(null).unwrap().data.name = "Rig".to_owned();
        step(
            &mut doc,
            &mut editor,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetData { id: null, data: before },
                label: "Rename",
            }],
        );
        assert_eq!(editor.history.undo_len(), 4);

        for expected in states.iter().rev().skip(1) {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
            assert_eq!(&snapshot(&doc), expected);
        }
        assert!(!editor.history.can_undo());
        for expected in states.iter().skip(1) {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
            assert_eq!(&snapshot(&doc), expected);
        }
        assert!(!editor.history.can_redo());
    }

    #[test]
    fn a_new_step_discards_redo_and_purges_only_unreachable_nodes() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        editor.selection.set_single(sphere);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::DeleteSelected,                    // undo step 1 can bring Sphere back
                EditorCommand::CreateObject(ObjectKindTag::Cube), // step 2
                EditorCommand::Undo,                              // Cube only reachable by redo
            ],
        );
        assert_eq!(doc.scene.detached_roots().len(), 2);
        let undone_cube = *doc.scene.detached_roots().last().unwrap();

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Plane)],
        );
        assert!(!editor.history.can_redo());
        assert_eq!(
            doc.scene.detached_roots(),
            &[sphere],
            "the cube was purged, the sphere kept"
        );

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert!(doc.scene.contains(sphere), "undo of the delete still works");
        assert!(!doc.scene.contains(undone_cube));
        assert!(doc.scene.detached_roots().iter().all(|&root| root != undone_cube));
    }

    #[test]
    fn the_history_limit_purges_what_falls_off() {
        let (mut doc, mut editor, mut vp) = demo();
        editor.preferences.undo_depth = 2;
        let ids = ["Cube", "Sphere", "Light"].map(|name| id_of(&doc, name));
        for id in ids {
            editor.selection.set_single(id);
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        }
        assert_eq!(editor.history.undo_len(), 2);
        assert_eq!(
            doc.scene.detached_roots(),
            &ids[1..],
            "the oldest delete can no longer be undone, so its node is freed"
        );

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo, EditorCommand::Undo],
        );
        assert!(!doc.scene.contains(ids[0]));
        assert!(doc.scene.contains(ids[1]) && doc.scene.contains(ids[2]));
    }

    #[test]
    fn rendering_a_take_renders_its_changes_without_making_it_active() {
        use crate::rendering::tests::{small, wait};
        let (mut doc, mut editor, mut vp) = demo();
        small(&mut doc, 2);
        let picture = |doc: &mut Document, editor: &mut EditorState, vp: &mut ViewportPanel| {
            run(doc, editor, vp, &[EditorCommand::RenderToPictureViewer]);
            wait(&mut editor.renders);
            editor.renders.history().last().unwrap().rgba.clone()
        };
        let main = picture(&mut doc, &mut editor, &mut vp);

        // A take that hides every object from the renderer, made while it is
        // active, then Main made active again.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewTake]);
        let take = doc.takes.ids()[0];
        for &root in doc.scene.roots().to_vec().iter() {
            let before = doc.scene.get(root).unwrap().data.clone();
            doc.scene.get_mut(root).unwrap().data.visible_in_render = false;
            run(
                &mut doc,
                &mut editor,
                &mut vp,
                &[EditorCommand::CommitLive {
                    inverse: SceneEdit::SetData { id: root, data: before },
                    label: "Hide",
                }],
            );
        }
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(None)]);
        assert_eq!(picture(&mut doc, &mut editor, &mut vp), main, "Main renders as before");

        let mut takes = doc.takes.clone();
        takes.render_take = Some(take);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Edit {
                edit: SceneEdit::SetTakes(takes),
                label: "Render Take",
            }],
        );
        let empty = picture(&mut doc, &mut editor, &mut vp);
        assert_ne!(empty, main, "the take's hidden objects are left out");
        assert_eq!(doc.takes.active(), None, "Main stays active");
        assert!(
            doc.scene
                .roots()
                .iter()
                .all(|&r| doc.scene.get(r).unwrap().data.visible_in_render)
        );
    }

    #[test]
    fn takes_record_edits_switch_clear_and_delete_as_undo_steps() {
        use grid4d_scene::OverrideGroup;
        let (mut doc, mut editor, mut vp) = demo();
        let cube = id_of(&doc, "Cube");
        let main = doc.scene.get(cube).unwrap().data.clone();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewTake]);
        let take = doc.takes.ids()[0];
        assert_eq!(doc.takes.active(), Some(take), "a new take is active");
        assert_eq!(doc.takes.get(take).unwrap().name, "Take");

        // A live drag, as the gizmo makes it, then its commit: one step that
        // also records the take's override.
        doc.scene.get_mut(cube).unwrap().data.transform.position.x = 250.0;
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: cube,
                    data: main.clone(),
                },
                label: "Move",
            }],
        );
        let overrides = |doc: &Document| doc.takes.get(take).map_or(0, grid4d_scene::Take::override_count);
        assert_eq!(overrides(&doc), 1);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(overrides(&doc), 0, "undo takes back the recording too");
        assert_eq!(doc.scene.get(cube).unwrap().data.transform, main.transform);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(overrides(&doc), 1);

        // Main shows Main's values; the take its own.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(None)]);
        assert_eq!(doc.scene.get(cube).unwrap().data.transform, main.transform);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(Some(take))]);
        assert_eq!(doc.scene.get(cube).unwrap().data.transform.position.x, 250.0);

        // Clearing the override shows Main's value again, without recording it.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::ClearOverride {
                take,
                node: Some(cube),
                group: Some(OverrideGroup::Transform),
            }],
        );
        assert_eq!(overrides(&doc), 0);
        assert_eq!(doc.scene.get(cube).unwrap().data.transform, main.transform);
        assert!(doc.takes.main().is_empty());
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(cube).unwrap().data.transform.position.x, 250.0);

        // Rendering another take leaves the active one as it is.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(None)]);
        let mut chosen = doc.takes.clone();
        chosen.render_take = Some(take);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::Edit {
                    edit: SceneEdit::SetTakes(chosen),
                    label: "Render Take",
                },
                EditorCommand::RenderToPictureViewer,
                EditorCommand::StopRender,
            ],
        );
        assert_eq!(doc.takes.active(), None);
        assert_eq!(doc.scene.get(cube).unwrap().data.transform, main.transform);

        // Deleting the active take goes back to Main in the same step.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(Some(take))]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteTake(take)]);
        assert!(doc.takes.is_empty());
        assert_eq!(doc.scene.get(cube).unwrap().data.transform, main.transform);
        assert_eq!(editor.history.undo_label(), Some("Delete Take"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.takes.active(), Some(take));
        assert_eq!(doc.scene.get(cube).unwrap().data.transform.position.x, 250.0);
    }

    #[test]
    fn a_deformer_plugin_goes_under_the_selection_fitted_and_a_take_overrides_it() {
        use grid4d_scene::{ObjectParam, PluginValue};
        crate::test_plugins();
        let (mut doc, mut editor, mut vp) = demo();
        let cube = id_of(&doc, "Cube");
        editor.selection.set_single(cube);
        let wave = |type_id: &str, child| EditorCommand::CreatePlugin {
            type_id: type_id.to_owned(),
            child,
        };
        run(&mut doc, &mut editor, &mut vp, &[wave("grid4d.wave", true)]);
        let id = editor.selection.active().unwrap();
        assert_ne!(id, cube);
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.parent(), Some(cube), "Shift: under the selection");
        let ObjectKind::Plugin(object) = &node.data.kind else {
            panic!("a plugin object")
        };
        let size = doc.scene.get(cube).unwrap().data.kind.local_bounds().unwrap();
        assert_eq!(
            object.get("Size"),
            Some(PluginValue::Vector(size.max - size.min)),
            "fitted to it"
        );
        assert!(
            !can_bake_current_state(&doc, &editor.selection),
            "a deformer has no state to copy"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(!doc.scene.contains(id));
        // Without Shift, or a generator with it: at the top.
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[wave("grid4d.gear", true)]);
        let gear = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(gear).unwrap().parent(), None);

        // A take changes its Amplitude; Main keeps its own.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[wave("grid4d.wave", true)]);
        let id = editor.selection.active().unwrap();
        let amplitude = |doc: &Document| match &doc.scene.get(id).unwrap().data.kind {
            ObjectKind::Plugin(o) => o.get("Amplitude"),
            _ => None,
        };
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewTake]);
        let take = doc.takes.ids()[0];
        let mut data = doc.scene.get(id).unwrap().data.clone();
        data.kind.set_param(ObjectParam::Plugin(1), 55.0);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Edit {
                edit: SceneEdit::SetData { id, data },
                label: "Amplitude",
            }],
        );
        assert_eq!(amplitude(&doc), Some(PluginValue::Float(55.0)));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(None)]);
        assert_eq!(amplitude(&doc), Some(PluginValue::Float(20.0)), "Main's value");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(Some(take))]);
        assert_eq!(amplitude(&doc), Some(PluginValue::Float(55.0)), "the take's");
    }

    #[test]
    fn a_command_plugin_runs_as_one_step_or_says_why_it_cannot() {
        use grid4d_scene::PluginValue;
        crate::test_plugins();
        let (mut doc, mut editor, mut vp) = demo();
        let scatter = |count: i32| {
            let mut options = grid4d_scene::PluginObject::new("grid4d.scatter");
            options.set("Count", PluginValue::Int(count));
            EditorCommand::RunPlugin {
                type_id: "grid4d.scatter".to_owned(),
                options,
            }
        };
        // Nothing selected: a message, no change.
        editor.selection.clear();
        let (len, steps) = (doc.scene.len(), editor.history.undo_len());
        run(&mut doc, &mut editor, &mut vp, &[scatter(5)]);
        assert!(
            editor
                .file
                .alert
                .take()
                .is_some_and(|a| a.message.contains("surface last"))
        );
        assert_eq!((doc.scene.len(), editor.history.undo_len()), (len, steps));

        // The Sphere over the Cube: a group and five copies, one step.
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        editor.selection.set_all(vec![sphere, cube]);
        assert_eq!(editor.selection.active(), Some(cube));
        run(&mut doc, &mut editor, &mut vp, &[scatter(5)]);
        assert!(editor.file.alert.is_none());
        assert_eq!(doc.scene.len(), len + 6);
        assert_eq!(editor.history.undo_len(), steps + 1);
        let group = editor.selection.active().unwrap();
        let node = doc.scene.get(group).unwrap();
        assert_eq!((node.data.name.as_str(), node.children().len()), ("Scatter", 5));
        assert_eq!(editor.selection.as_slice(), [group], "the new group selected");
        // Copies sit on the cube's faces (in its own space).
        let bounds = doc.scene.get(cube).unwrap().data.kind.local_bounds().unwrap();
        let into_cube = doc.scene.world_matrix(cube).inverse();
        for &copy in node.children() {
            let p = into_cube.transform_point3(doc.scene.world_matrix(copy).w_axis.truncate());
            let on_face = (0..3).any(|a| (p[a].abs() - bounds.max[a]).abs() < 1e-2);
            assert!(on_face && p.abs().max_element() <= 100.0 + 1e-2, "{p}");
        }
        let options = editor
            .plugin_commands
            .options_mut(grid4d_plugin::registry().command("grid4d.scatter").unwrap());
        assert_eq!(
            options.get("Count"),
            Some(PluginValue::Int(5)),
            "remembered for the dialog"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.len(), len, "one undo takes it all back");
        assert_eq!(editor.selection.as_slice(), [sphere, cube]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.scene.len(), len + 6);
    }

    #[test]
    fn a_script_is_one_undo_step_and_a_failing_one_changes_nothing() {
        let (mut doc, mut editor, mut vp) = demo();
        let (len, steps) = (doc.scene.len(), editor.history.undo_len());
        let grid = r#"
            for i in 0..4 {
                let cube = create("Cube");
                cube.position = [i * 300, 0, 0];
                cube.color = [1, 0, 0];
            }
            select(find("Sphere"));
            print("done");
        "#;
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::RunScript(grid.to_owned())],
        );
        assert_eq!(doc.scene.len(), len + 4);
        assert_eq!(editor.history.undo_len(), steps + 1, "one step");
        assert_eq!(editor.selection.as_slice(), [id_of(&doc, "Sphere")]);
        let alert = editor.file.alert.take().unwrap();
        assert_eq!((alert.title, alert.message.as_str()), ("Script Output", "done"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.len(), len, "all of it undone");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.scene.len(), len + 4);

        // An error: nothing changes, the message says where.
        let before = doc.scene.len();
        let steps = editor.history.undo_len();
        let failing = "create(\"Cube\");\nlet o = find(\"Nobody\");\no.name = \"x\";".to_owned();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::RunScript(failing)]);
        assert_eq!((doc.scene.len(), editor.history.undo_len()), (before, steps));
        let alert = editor.file.alert.take().unwrap();
        assert_eq!(alert.title, "Script Error");
        assert!(alert.message.starts_with("Line 3, column"), "{}", alert.message);

        // A script that only reads: no step, no change.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::RunScript("let n = objects().len();".to_owned())],
        );
        assert_eq!(editor.history.undo_len(), steps);
        assert!(editor.file.alert.is_none());
    }

    #[test]
    fn with_the_script_manager_open_output_and_errors_go_to_its_console() {
        let (mut doc, mut editor, mut vp) = demo();
        editor.scripts.show = true;
        let script = |s: &str| EditorCommand::RunScript(s.to_owned());
        run(&mut doc, &mut editor, &mut vp, &[script("print(\"hello\");")]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[script("print(\"before\");\nlet o = find(\"Nobody\");\no.name = \"x\";")],
        );
        assert!(editor.file.alert.is_none(), "no messages");
        let console: Vec<(&str, bool)> = editor
            .scripts
            .console()
            .iter()
            .map(|l| (l.text.as_str(), l.error))
            .collect();
        assert_eq!(console[..2], [("hello", false), ("before", false)][..]);
        assert!(console[2].1 && console[2].0.starts_with("Line 3"), "{console:?}");

        // Saving needs a name; the library runs by name.
        editor.scripts.name = "a/b".to_owned();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SaveScript]);
        assert!(
            editor
                .file
                .alert
                .take()
                .is_some_and(|a| a.title == crate::scripts::COULD_NOT_SAVE)
        );
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::RunLibraryScript("Missing".to_owned())],
        );
        assert!(editor.file.alert.take().is_some(), "no library: cannot open");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewScript]);
        assert!(editor.scripts.name.is_empty() && !editor.scripts.is_dirty());
    }

    /// Phase 13's acceptance (`docs/IMPLEMENTATION_PLAN.md`) for the Gear: its
    /// parameters edit, undo, animate, override in a take and save; the file
    /// round-trips as it is, whether or not the plugin is installed (the file
    /// layer never asks). The Wave, Scatter and scripts have their own tests.
    #[test]
    fn phase_13_acceptance_a_gear_edits_animates_overrides_and_saves() {
        use grid4d_scene::{ObjectParam, PluginValue};
        crate::test_plugins();
        let (mut doc, mut editor, mut vp) = demo();
        let create = EditorCommand::CreatePlugin {
            type_id: "grid4d.gear".to_owned(),
            child: false,
        };
        run(&mut doc, &mut editor, &mut vp, &[create]);
        let gear = editor.selection.active().unwrap();
        let value = |doc: &Document, name: &str, frame: f64| match &*doc.scene.get(gear).unwrap().data.kind_at(frame) {
            ObjectKind::Plugin(o) => o.get(name),
            _ => None,
        };
        let edit = |doc: &Document, change: &dyn Fn(&mut NodeData)| {
            let mut data = doc.scene.get(gear).unwrap().data.clone();
            change(&mut data);
            EditorCommand::Edit {
                edit: SceneEdit::SetData { id: gear, data },
                label: "Edit",
            }
        };
        let set = |name: &'static str, v: PluginValue| {
            move |data: &mut NodeData| {
                if let ObjectKind::Plugin(o) = &mut data.kind {
                    o.set(name, v);
                }
            }
        };

        // Edited, undone, redone.
        let teeth = edit(&doc, &set("Teeth", PluginValue::Int(20)));
        run(&mut doc, &mut editor, &mut vp, &[teeth]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(value(&doc, "Teeth", 0.0), Some(PluginValue::Int(12)));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(value(&doc, "Teeth", 0.0), Some(PluginValue::Int(20)));

        // Its Radius keyed from 100 to 150 over ten frames.
        let radius = {
            let ObjectKind::Plugin(o) = &doc.scene.get(gear).unwrap().data.kind else {
                unreachable!()
            };
            let index = o.values.iter().position(|(n, _)| n == "Radius").unwrap();
            Param::Object(ObjectParam::Plugin(u8::try_from(index).unwrap()))
        };
        let keys = edit(&doc, &|data: &mut NodeData| {
            data.toggle_key(radius, 0.0); // a key at 0 with its 100
            data.set_value_at(radius, 150.0, 10.0, true);
        });
        run(&mut doc, &mut editor, &mut vp, &[keys]);
        let Some(PluginValue::Float(halfway)) = value(&doc, "Radius", 5.0) else {
            panic!("a radius")
        };
        assert!(halfway > 100.0 && halfway < 150.0, "{halfway}");

        // A take with a thicker gear; Main keeps its own.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::NewTake]);
        let take = doc.takes.ids()[0];
        let thicker = edit(&doc, &set("Thickness", PluginValue::Float(40.0)));
        run(&mut doc, &mut editor, &mut vp, &[thicker]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(None)]);
        assert_eq!(value(&doc, "Thickness", 0.0), Some(PluginValue::Float(20.0)));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SwitchTake(Some(take))]);
        assert_eq!(value(&doc, "Thickness", 0.0), Some(PluginValue::Float(40.0)));

        // Saved and opened again: the same object, its keys found by name.
        let bytes = grid4d_io::to_bytes(&doc).unwrap();
        let loaded = grid4d_io::from_bytes(&bytes).unwrap();
        let find = |doc: &Document| {
            doc.scene
                .iter()
                .find(|(_, n)| n.data.name == "Gear")
                .unwrap()
                .1
                .data
                .clone()
        };
        let (before, after) = (find(&doc), find(&loaded));
        assert_eq!((&after.kind, &after.tracks), (&before.kind, &before.tracks));
        assert_eq!(grid4d_io::to_bytes(&loaded).unwrap(), bytes, "saved back unchanged");
    }

    #[test]
    fn a_plugin_object_is_created_undone_and_made_editable() {
        crate::test_plugins();
        let (mut doc, mut editor, mut vp) = demo();
        let before = doc.scene.len();
        let create = |type_id: &str, child: bool| EditorCommand::CreatePlugin {
            type_id: type_id.to_owned(),
            child,
        };
        run(&mut doc, &mut editor, &mut vp, &[create("grid4d.gear", false)]);
        let id = editor.selection.active().unwrap();
        let data = &doc.scene.get(id).unwrap().data;
        assert_eq!(data.name, "Gear");
        assert!(matches!(&data.kind, ObjectKind::Plugin(o) if o.type_id == "grid4d.gear" && o.values.len() == 6));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.len(), before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert!(doc.scene.contains(id));

        // A plugin that is not installed: nothing happens.
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[create("nobody.nothing", false)]);
        assert_eq!((doc.scene.len(), editor.history.undo_len()), (before + 1, steps));

        editor.selection.set_single(id);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("made editable")
        };
        assert_eq!(mesh.mesh().face_count(), 12 * 4 * 4, "the gear's faces, hole included");
    }

    #[test]
    fn objects_go_on_and_off_layers_in_one_undoable_step() {
        let (mut doc, mut editor, mut vp) = demo();
        let layer = doc.scene.layers_mut().insert(grid4d_scene::Layer::new("Set", [1.0; 3]));
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AssignLayer {
                layer: Some(layer),
                objects: vec![cube, sphere],
            }],
        );
        let on = |doc: &Document, id| doc.scene.get(id).unwrap().data.layer;
        assert_eq!((on(&doc, cube), on(&doc, sphere)), (Some(layer), Some(layer)));
        assert_eq!(editor.history.undo_label(), Some("Add to Layer"));
        assert_eq!(editor.history.undo_len(), 1);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SelectLayerObjects(layer)],
        );
        assert_eq!(editor.selection.as_slice(), &[cube, sphere]);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AssignLayer {
                layer: None,
                objects: vec![sphere],
            }],
        );
        assert_eq!(on(&doc, sphere), None);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!((on(&doc, cube), on(&doc, sphere)), (None, None));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(on(&doc, cube), Some(layer));

        // Nothing to change: no step.
        let steps = editor.history.undo_len();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AssignLayer {
                layer: Some(layer),
                objects: vec![cube],
            }],
        );
        assert_eq!(editor.history.undo_len(), steps);

        // A locked layer's object keeps its place under the gizmo.
        doc.scene.layers_mut().get_mut(layer).unwrap().switches.locked = true;
        let protection = doc.scene.protection(cube);
        assert!(protection.position && protection.rotation && protection.scale);
        assert!(!doc.scene.protection(id_of(&doc, "Light")).position);
    }

    #[test]
    fn lowering_the_undo_depth_drops_the_oldest_steps_at_once() {
        let (mut doc, mut editor, mut vp) = demo();
        let ids = ["Cube", "Sphere", "Light"].map(|name| id_of(&doc, name));
        for id in ids {
            editor.selection.set_single(id);
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        }
        assert_eq!(editor.history.undo_len(), 3);
        editor.preferences.undo_depth = 1;
        apply_pending(&mut doc, &mut editor, &mut vp);
        assert_eq!((editor.history.limit(), editor.history.undo_len()), (1, 1));
        assert_eq!(
            doc.scene.detached_roots(),
            &ids[2..],
            "only the newest delete can come back"
        );
        editor.preferences.undo_depth = 50;
        apply_pending(&mut doc, &mut editor, &mut vp);
        assert_eq!((editor.history.limit(), editor.history.undo_len()), (50, 1));
    }

    #[test]
    fn apply_to_views_gives_every_view_the_preferred_look() {
        let (mut doc, mut editor, mut vp) = demo();
        editor.preferences.views = crate::preferences::ViewDefaults {
            shading: ShadingMode::QuickLines,
            grid: false,
            world_axis: true,
            hud: false,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::ShowView(Some(3)), EditorCommand::ApplyViewDefaults],
        );
        for view in [3, 0] {
            run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ShowView(Some(view))]);
            let filter = vp.filter();
            assert_eq!(vp.display().shading, ShadingMode::QuickLines, "view {view}");
            assert!(!filter.shows(FilterItem::Grid) && !filter.shows(FilterItem::Hud));
            assert!(filter.shows(FilterItem::WorldAxis) && filter.shows(FilterItem::Camera));
        }
    }

    #[test]
    fn recovering_opens_the_copy_unsaved_and_linked_to_its_file() {
        use crate::autosave::{Autosave, RecoveryChoice};
        let (mut doc, mut editor, mut vp) = demo();
        let dir = crate::autosave::tests::folder("recover-command");
        grid4d_io::save(&Document::new("Crashed Work"), &dir.join("7-7.g4d")).unwrap();
        std::fs::write(dir.join("7-7.txt"), "D:/shots/work.g4d").unwrap();
        editor.autosave = Autosave::new(dir.clone());
        assert!(editor.autosave.offer.is_some());

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Recover(RecoveryChoice::Recover)],
        );
        assert_eq!(doc.name, "Crashed Work");
        assert_eq!(editor.file.path(), Some(std::path::Path::new("D:/shots/work.g4d")));
        assert!(editor.file.is_dirty(&editor.history), "recovered work is unsaved");
        assert_eq!(editor.autosave.offer, None);
        editor.autosave.finish();
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "nothing left after a normal exit"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn state_id_identifies_document_states() {
        let (mut doc, mut editor, mut vp) = demo();
        let s0 = editor.history.state_id();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let s1 = editor.history.state_id();
        assert_ne!(s1, s0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(editor.history.state_id(), s0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(editor.history.state_id(), s1);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::CreateObject(ObjectKindTag::Sphere)],
        );
        let s2 = editor.history.state_id();
        assert!(s2 != s0 && s2 != s1, "a different edit is a different state");
    }

    #[test]
    fn new_document_clears_the_history() {
        let (mut doc, mut editor, mut vp) = demo();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::CreateObject(ObjectKindTag::Cube),
                EditorCommand::Undo,
                EditorCommand::NewDocument,
            ],
        );
        // The document has unsaved changes: New asks first.
        assert_eq!(editor.file.pending(), Some(&FileAction::New));
        assert!(editor.history.can_undo());

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)],
        );
        assert!(!editor.history.can_undo() && !editor.history.can_redo());
        assert_eq!(editor.history.state_id(), 0);
        assert!(doc.scene.detached_roots().is_empty());
        assert!(doc.scene.is_empty());
        assert!(!editor.file.is_dirty(&editor.history));
    }

    #[test]
    fn edit_and_commit_live_cover_document_settings() {
        let (mut doc, mut editor, mut vp) = demo();
        let original = doc.time.settings();

        // A live change (the fps field while dragging), committed on release.
        doc.time.set_fps(48);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetTime(original),
                label: "Project Settings",
            }],
        );
        // A discrete change, applied by the command itself.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Edit {
                edit: SceneEdit::SetName("Shot 010".to_owned()),
                label: "Rename Project",
            }],
        );
        assert_eq!(doc.time.fps(), 48);
        assert_eq!(doc.name, "Shot 010");
        assert_eq!(editor.history.undo_len(), 2);

        // Committing the current state again records nothing.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetName("Shot 010".to_owned()),
                label: "Rename Project",
            }],
        );
        assert_eq!(editor.history.undo_len(), 2);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(doc.name, "Untitled 1");
        assert_eq!(doc.time.settings(), original);
    }

    #[test]
    fn a_commit_for_a_deleted_node_records_nothing() {
        let (mut doc, mut editor, mut vp) = demo();
        let sphere = id_of(&doc, "Sphere");
        let before = doc.scene.get(sphere).unwrap().data.clone();
        editor.selection.set_single(sphere);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetData {
                    id: sphere,
                    data: before,
                },
                label: "Move",
            }],
        );
        assert_eq!(editor.history.undo_label(), Some("Delete"));
        assert_eq!(editor.history.undo_len(), 1);
    }

    #[test]
    fn alt_create_wraps_the_active_object_in_one_undo_step() {
        let (mut doc, mut editor, mut vp) = demo();
        let (group, boxed) = (id_of(&doc, "Group"), id_of(&doc, "Box"));
        let world_before = doc.scene.world_matrix(boxed);
        let transform_before = doc.scene.get(boxed).unwrap().data.transform;
        editor.selection.set_single(boxed);
        let wrap = EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface);
        run(&mut doc, &mut editor, &mut vp, std::slice::from_ref(&wrap));

        let sds = editor.selection.active().unwrap();
        assert_eq!(
            doc.scene.get(sds).unwrap().data.kind.tag(),
            ObjectKindTag::SubdivisionSurface
        );
        assert_eq!(
            doc.scene.get(sds).unwrap().parent(),
            Some(group),
            "takes the object's place"
        );
        assert_eq!(doc.scene.index_in_parent(sds), Some(1));
        assert_eq!(doc.scene.get(boxed).unwrap().parent(), Some(sds));
        assert_eq!(doc.scene.get(boxed).unwrap().data.transform, transform_before);
        assert!(
            doc.scene.world_matrix(boxed).abs_diff_eq(world_before, 1e-4),
            "stays where it was"
        );
        assert_eq!(
            (editor.history.undo_len(), editor.history.undo_label()),
            (1, Some("Create Generator"))
        );

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(!doc.scene.contains(sds));
        assert_eq!(doc.scene.get(boxed).unwrap().parent(), Some(group));
        assert_eq!(doc.scene.index_in_parent(boxed), Some(1));
        assert_eq!(editor.selection.active(), Some(boxed));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.scene.get(boxed).unwrap().parent(), Some(sds), "same generator back");

        // Nothing selected: an ordinary create at the top level.
        editor.selection.clear();
        run(&mut doc, &mut editor, &mut vp, &[wrap]);
        let empty = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(empty).unwrap().parent(), None);
        assert!(doc.scene.children(empty).is_empty());
        assert_eq!(doc.scene.get(empty).unwrap().data.name, "Subdivision Surface.1");
    }

    #[test]
    fn make_editable_on_a_generator_bakes_its_result_and_removes_the_cage() {
        let (mut doc, mut editor, mut vp) = demo();
        let cube = id_of(&doc, "Cube");
        editor.selection.set_single(cube);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface)],
        );
        let sds = editor.selection.active().unwrap();
        let before = doc.scene.get(sds).unwrap().data.clone();
        let expected = grid4d_eval::generator_mesh(&doc.scene, sds, 0.0).unwrap();
        assert_eq!(expected.face_count(), 6 * 4 * 4, "level 2 of a one-segment cube");

        // The cage selected too: it goes with the generator, not baked on its own.
        editor.selection.set_all([sds, cube]);
        assert!(can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(sds).unwrap().data.kind else {
            panic!("not baked")
        };
        assert_eq!(mesh.mesh(), &expected, "the viewport level, as shown");
        assert_eq!(doc.scene.get(sds).unwrap().data.name, before.name);
        assert!(!doc.scene.contains(cube), "the cage is gone");
        assert_eq!(editor.history.undo_label(), Some("Make Editable"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(sds).unwrap().data, before);
        assert_eq!(doc.scene.get(cube).unwrap().parent(), Some(sds), "the cage is back");

        // A disabled generator shows its cage as it is: nothing to bake.
        let mut disabled = before;
        disabled.enabled = false;
        doc.scene.get_mut(sds).unwrap().data = disabled;
        editor.selection.set_single(sds);
        assert!(!can_make_editable(&doc, &editor.selection));
    }

    #[test]
    fn q_toggles_the_generator_of_the_selection() {
        let (mut doc, mut editor, mut vp) = demo();
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        editor.selection.set_single(cube);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface)],
        );
        let sds = editor.selection.active().unwrap();
        let enabled = |doc: &Document| doc.scene.get(sds).unwrap().data.enabled;

        // The cage selected: its parent generator toggles.
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ToggleParentGenerator]);
        assert!(!enabled(&doc));
        assert_eq!(editor.history.undo_label(), Some("Toggle Generator"));
        // The generator and its cage selected: toggled once, not twice.
        editor.selection.set_all([sds, cube]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ToggleParentGenerator]);
        assert!(enabled(&doc));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(!enabled(&doc));

        // No generator above: nothing changes, nothing is recorded.
        let steps = editor.history.undo_len();
        editor.selection.set_single(sphere);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::ToggleParentGenerator]);
        assert_eq!(editor.history.undo_len(), steps);
    }

    #[test]
    fn tags_are_added_selected_and_deleted_as_undo_steps() {
        use grid4d_scene::{Tag, TagKind};
        let (mut doc, mut editor, mut vp) = demo();
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        editor.selection.set_all([sphere, cube]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Phong.default_tag())],
        );
        for id in [cube, sphere] {
            assert_eq!(doc.scene.get(id).unwrap().data.tags, [TagKind::Phong.default_tag()]);
        }
        assert_eq!(editor.history.undo_label(), Some("Add Tag"));
        let (tag, index) = editor
            .selected_tag(&doc)
            .expect("the active object's new tag is selected");
        assert_eq!((tag.node, tag.kind, index), (cube, TagKind::Phong, 0));

        // A second Phong tag is not added; another kind is.
        let steps = editor.history.undo_len();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Phong.default_tag())],
        );
        assert_eq!(editor.history.undo_len(), steps, "nothing to add");
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Protection.default_tag())],
        );
        assert_eq!(doc.scene.get(cube).unwrap().data.tags.len(), 2);

        // Delete removes the selected tag, not the object.
        assert_eq!(editor.selected_tag(&doc).unwrap().0.kind, TagKind::Protection);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert!(doc.scene.contains(cube), "the object stays");
        assert_eq!(doc.scene.get(cube).unwrap().data.tags, [TagKind::Phong.default_tag()]);
        assert_eq!(editor.history.undo_label(), Some("Delete Tag"));
        assert_eq!(editor.tag_selection, None);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(cube).unwrap().data.tags.len(), 2, "undo brings it back");

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::DeleteTag(crate::TagRef {
                node: sphere,
                kind: TagKind::Phong,
                nth: 0,
            })],
        );
        assert!(
            doc.scene
                .get(sphere)
                .unwrap()
                .data
                .tags
                .iter()
                .all(|t| t.kind() != TagKind::Phong),
            "the Sphere's Phong tag is gone (its Protection tag stays)"
        );
        // With no tag selected, Delete deletes objects again.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert!(!doc.scene.contains(cube));
        let _ = Tag::Phong {
            angle_limit: true,
            angle: 0.0,
        };
    }

    #[test]
    fn a_tag_selection_only_counts_while_its_object_is_active() {
        use grid4d_scene::TagKind;
        let (mut doc, mut editor, mut vp) = demo();
        let (cube, sphere) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"));
        editor.selection.set_single(cube);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Target.default_tag())],
        );
        assert!(editor.selected_tag(&doc).is_some());
        editor.selection.set_single(sphere);
        assert_eq!(editor.selected_tag(&doc), None, "another object is active");
        editor.selection.set_single(cube);
        assert!(editor.selected_tag(&doc).is_some());
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(editor.selected_tag(&doc), None, "the tag is gone");
    }

    /// The spline object's control points and selected points.
    fn spline_of(doc: &Document, id: NodeId) -> (grid4d_geom::spline::Spline, Vec<usize>) {
        let ObjectKind::Spline { spline, components, .. } = &doc.scene.get(id).unwrap().data.kind else {
            panic!("not an editable spline");
        };
        (spline.spline().clone(), components.selected().points.iter().collect())
    }

    #[test]
    fn make_editable_turns_a_spline_primitive_into_the_same_spline() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Star)],
        );
        let id = editor.selection.active().unwrap();
        let ObjectKind::SplinePrimitive(primitive) = doc.scene.get(id).unwrap().data.kind.clone() else {
            panic!()
        };
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let ObjectKind::Spline {
            spline, interpolation, ..
        } = &doc.scene.get(id).unwrap().data.kind
        else {
            panic!("an editable spline")
        };
        assert_eq!(
            spline.spline().sample(*interpolation),
            primitive.polyline(),
            "the same curve"
        );
        assert_eq!(editor.history.undo_label(), Some("Make Editable"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(matches!(
            doc.scene.get(id).unwrap().data.kind,
            ObjectKind::SplinePrimitive(_)
        ));
    }

    #[test]
    fn spline_points_are_selected_inverted_and_deleted_as_undo_steps() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::NSide),
                EditorCommand::MakeEditable,
            ],
        );
        let id = editor.selection.active().unwrap();
        editor.mode = EditMode::Points;
        let select = |hits: Vec<u32>, mode| EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits,
            mode,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(vec![1, 3], SelectMode::Replace)],
        );
        assert_eq!(spline_of(&doc, id).1, [1, 3]);
        assert_eq!(editor.history.undo_label(), Some("Select Points"));
        run(&mut doc, &mut editor, &mut vp, &[select(vec![3], SelectMode::Remove)]);
        assert_eq!(spline_of(&doc, id).1, [1]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::InvertSelection]);
        assert_eq!(spline_of(&doc, id).1, [0, 2, 3, 4, 5]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeselectAll]);
        assert!(spline_of(&doc, id).1.is_empty());
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(spline_of(&doc, id).1.len(), 6);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[select(vec![0, 2], SelectMode::Replace)],
        );
        let before = spline_of(&doc, id).0;
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        let (after, selected) = spline_of(&doc, id);
        assert_eq!(
            after.points,
            [before.points[1], before.points[3], before.points[4], before.points[5]]
        );
        assert!(selected.is_empty());
        assert!(doc.scene.get(id).is_some(), "Delete in Points mode keeps the object");
        assert_eq!(editor.history.undo_label(), Some("Delete Points"));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(spline_of(&doc, id).0, before);
    }

    #[test]
    fn spline_commands_change_the_active_spline_one_step_each() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::CreateObject(ObjectKindTag::Circle),
                EditorCommand::MakeEditable,
            ],
        );
        let id = editor.selection.active().unwrap();
        editor.mode = EditMode::Points;
        let select = |hits: Vec<u32>| EditorCommand::SelectComponents {
            node: id,
            component: Component::Point,
            hits,
            mode: SelectMode::Replace,
        };
        run(&mut doc, &mut editor, &mut vp, &[select(vec![1])]);
        let (before, _) = spline_of(&doc, id);

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SplineCommand(SplineOp::Hard)],
        );
        let (hard, selected) = spline_of(&doc, id);
        assert_eq!(hard.tangents[1], [Vec3::ZERO; 2], "a corner");
        assert_eq!(hard.tangents[0], before.tangents[0], "others keep theirs");
        assert_eq!(selected, [1]);
        assert_eq!(editor.history.undo_label(), Some("Hard Interpolation"));

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SplineCommand(SplineOp::Soft)],
        );
        let (soft, _) = spline_of(&doc, id);
        assert!(soft.tangents[1][1].length() > 1.0, "smooth again");
        assert!(soft.tangents[1][0].dot(soft.tangents[1][1]) < 0.0, "in line");
        assert_eq!(editor.history.undo_label(), Some("Soft Interpolation"));

        // Set First Point: point 1 becomes the first; the selection follows it.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SplineCommand(SplineOp::SetFirstPoint)],
        );
        let (first, selected) = spline_of(&doc, id);
        assert_eq!(first.points[0], soft.points[1]);
        assert_eq!(selected, [0]);
        assert_eq!(editor.history.undo_label(), Some("Set First Point"));
        // Again: point 0 is already the first, so nothing changes and no step is made.
        let steps = editor.history.state_id();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SplineCommand(SplineOp::SetFirstPoint)],
        );
        assert_eq!(editor.history.state_id(), steps);

        // Reverse with nothing selected runs every segment backwards.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::DeselectAll,
                EditorCommand::SplineCommand(SplineOp::Reverse),
            ],
        );
        let (reversed, _) = spline_of(&doc, id);
        assert_eq!(reversed.points.len(), 4);
        assert_eq!(reversed.points[0], first.points[3]);
        assert_eq!(reversed.points[3], first.points[0]);
        assert_eq!(editor.history.undo_label(), Some("Reverse Sequence"));

        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(spline_of(&doc, id).0, first);
    }

    #[test]
    fn the_pen_adds_points_closes_and_starts_new_splines() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let pen = |x: f32, z: f32, tangent: Vec3| EditorCommand::PenPoint {
            position: Vec3::new(x, 0.0, z),
            tangent,
        };
        run(&mut doc, &mut editor, &mut vp, &[pen(0.0, 0.0, Vec3::ZERO)]);
        let first = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(first).unwrap().data.name, "Spline");
        assert_eq!(spline_of(&doc, first).0.kind, grid4d_geom::spline::SplineType::Bezier);
        assert_eq!(editor.history.undo_label(), Some("Pen"));

        let pull = Vec3::new(0.0, 0.0, 50.0);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[pen(100.0, 0.0, pull), pen(100.0, 100.0, Vec3::ZERO)],
        );
        let (spline, _) = spline_of(&doc, first);
        assert_eq!(spline.points.len(), 3);
        assert_eq!(
            spline.tangents[1],
            [-pull, pull],
            "the dragged tangent is the outgoing one"
        );
        assert_eq!(spline.tangents[2], [Vec3::ZERO; 2], "a click makes a corner");
        assert_eq!(editor.history.undo_label(), Some("Add Point"));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::PenClose]);
        assert!(spline_of(&doc, first).0.segments[0].closed);
        assert_eq!(editor.history.undo_label(), Some("Close Spline"));

        // Finished: the next point starts a new spline, linear this time.
        editor.pen.kind = grid4d_geom::spline::SplineType::Linear;
        run(&mut doc, &mut editor, &mut vp, &[pen(500.0, 0.0, Vec3::ZERO)]);
        let second = editor.selection.active().unwrap();
        assert_ne!(second, first);
        assert_eq!(spline_of(&doc, second).0.kind, grid4d_geom::spline::SplineType::Linear);

        // Choosing the first spline again: the Pen adds a new segment to it.
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::PenFinish]);
        editor.selection.set_single(first);
        run(&mut doc, &mut editor, &mut vp, &[pen(0.0, -100.0, Vec3::ZERO)]);
        let (spline, _) = spline_of(&doc, first);
        assert_eq!(spline.points.len(), 4);
        assert_eq!(spline.segments.len(), 2);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(spline_of(&doc, first).0.points.len(), 3);
    }

    #[test]
    fn an_extrude_wraps_a_spline_and_make_editable_bakes_the_solid() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Rectangle)],
        );
        let rectangle = editor.selection.active().unwrap();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::WrapInGenerator(ObjectKindTag::Extrude)],
        );
        let extrude = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(extrude).unwrap().data.name, "Extrude");
        assert_eq!(doc.scene.get(rectangle).unwrap().parent(), Some(extrude));
        let solid = grid4d_eval::generator_mesh(&doc.scene, extrude, 0.0).unwrap();
        assert_eq!(solid.face_count(), 4 + 2, "four walls and two caps");
        assert!(can_make_editable(&doc, &editor.selection));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(extrude).unwrap().data.kind else {
            panic!("not baked")
        };
        assert_eq!(mesh.mesh(), &solid);
        assert!(!doc.scene.contains(rectangle), "the spline went with it");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(extrude).unwrap().data.kind.tag(), ObjectKindTag::Extrude);
        assert_eq!(doc.scene.get(rectangle).unwrap().parent(), Some(extrude));
    }

    #[test]
    fn a_lathe_wraps_a_spline_and_bakes_into_a_closed_body() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Circle)],
        );
        let circle = editor.selection.active().unwrap();
        // Beside the axis: a torus.
        doc.scene.get_mut(circle).unwrap().data.transform.position.x = 400.0;
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[
                EditorCommand::WrapInGenerator(ObjectKindTag::Lathe),
                EditorCommand::MakeEditable,
            ],
        );
        let lathe = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(lathe).unwrap().data.name, "Lathe");
        let ObjectKind::Polygon { mesh, .. } = &doc.scene.get(lathe).unwrap().data.kind else {
            panic!("not baked")
        };
        assert_eq!(mesh.mesh().face_count() % 24, 0, "24 steps round");
        assert!(!doc.scene.contains(circle));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(doc.scene.get(circle).unwrap().parent(), None, "back where it was");
    }

    #[test]
    fn a_sweep_wraps_two_selected_splines_and_bakes_without_them() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Circle)],
        );
        let profile = editor.selection.active().unwrap();
        let ObjectKind::SplinePrimitive(primitive) = &mut doc.scene.get_mut(profile).unwrap().data.kind else {
            unreachable!()
        };
        primitive.shape = grid4d_geom::spline::SplineShape::Circle {
            radius: 10.0,
            ellipse: None,
            ring: None,
        };
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Rectangle)],
        );
        let path = editor.selection.active().unwrap();
        editor.selection.set_all([profile, path]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::WrapInGenerator(ObjectKindTag::Sweep)],
        );
        let sweep = editor.selection.active().unwrap();
        assert_eq!(doc.scene.children(sweep), [profile, path], "profile first, path second");
        assert_eq!(editor.history.undo_label(), Some("Create Generator"));
        let solid = grid4d_eval::generator_mesh(&doc.scene, sweep, 0.0).unwrap();
        assert!(solid.face_count() > 0);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        assert!(matches!(
            doc.scene.get(sweep).unwrap().data.kind,
            ObjectKind::Polygon { .. }
        ));
        assert!(
            !doc.scene.contains(profile) && !doc.scene.contains(path),
            "both inputs go"
        );
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert!(!doc.scene.contains(sweep));
        for id in [profile, path] {
            assert_eq!(doc.scene.get(id).unwrap().parent(), None, "back at the top");
        }
        assert_eq!(doc.scene.roots(), [profile, path], "in their places");
    }

    #[test]
    fn a_loft_wraps_every_selected_spline_and_bakes_without_them() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let mut sections = Vec::new();
        for z in [0.0, 200.0, 400.0] {
            run(
                &mut doc,
                &mut editor,
                &mut vp,
                &[EditorCommand::CreateObject(ObjectKindTag::Circle)],
            );
            let id = editor.selection.active().unwrap();
            doc.scene.get_mut(id).unwrap().data.transform.position.z = z;
            sections.push(id);
        }
        // Selected out of their order: the selection's order decides.
        editor.selection.set_all([sections[2], sections[0], sections[1]]);
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::WrapInGenerator(ObjectKindTag::Loft)],
        );
        let loft = editor.selection.active().unwrap();
        assert_eq!(doc.scene.children(loft), [sections[2], sections[0], sections[1]]);
        assert!(grid4d_eval::generator_mesh(&doc.scene, loft, 0.0).unwrap().face_count() > 0);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        assert!(sections.iter().all(|&id| !doc.scene.contains(id)), "every section goes");
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::Undo, EditorCommand::Undo],
        );
        assert_eq!(doc.scene.roots(), sections, "back in their places");
    }

    #[test]
    fn make_editable_turns_a_text_into_its_outline_spline() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Text)],
        );
        let id = editor.selection.active().unwrap();
        let ObjectKind::Text(text) = doc.scene.get(id).unwrap().data.kind.clone() else {
            panic!("not a text")
        };
        assert_eq!(doc.scene.get(id).unwrap().data.name, "Text");
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let (spline, _) = spline_of(&doc, id);
        assert_eq!(spline, grid4d_text::text_spline(&text), "the same curve");
        assert_eq!(
            spline.segments.len(),
            5,
            "T, x and t one contour each, e two (its hole)"
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.get(id).unwrap().data.kind, ObjectKind::Text(text));
    }

    #[test]
    fn make_editable_bakes_animated_parameters_at_the_frame_and_drops_their_tracks() {
        use grid4d_scene::{Axis, Key, ObjectParam, Track};
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Sphere)],
        );
        let id = editor.selection.active().unwrap();
        let tracks = &mut doc.scene.get_mut(id).unwrap().data.tracks;
        tracks.push(Track {
            param: Param::Object(ObjectParam::Radius),
            curve: grid4d_scene::Curve::from_keys([Key::new(0.0, 10.0), Key::new(10.0, 50.0)]),
        });
        tracks.push(Track {
            param: Param::Position(Axis::X),
            curve: grid4d_scene::Curve::from_keys([Key::new(0.0, 0.0), Key::new(10.0, 5.0)]),
        });
        doc.time.set_frame(10);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        let data = &doc.scene.get(id).unwrap().data;
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!("not baked")
        };
        let (_, max) = mesh.mesh().bounds().unwrap();
        assert!((max.y - 50.0).abs() < 0.5, "the radius at frame 10: {max}");
        assert_eq!(data.tracks.len(), 1, "the radius track goes, the position track stays");
    }

    #[test]
    fn shift_create_puts_a_deformer_under_the_selection_fitted_to_it() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cylinder)],
        );
        let cylinder = editor.selection.active().unwrap();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateChild(ObjectKindTag::Bend)],
        );
        let bend = editor.selection.active().unwrap();
        let node = doc.scene.get(bend).unwrap();
        assert_eq!(node.parent(), Some(cylinder));
        let ObjectKind::Bend(b) = node.data.kind else {
            panic!("not a bend")
        };
        assert!(b.size.abs_diff_eq(Vec3::new(100.0, 200.0, 100.0), 1e-3), "{}", b.size);
        assert_eq!(node.data.transform.position, Vec3::ZERO, "the cylinder is centred");
        assert_eq!(editor.history.undo_label(), Some("Create Object"));

        // Moved and resized by hand, then fitted again: one step.
        let data = &mut doc.scene.get_mut(bend).unwrap().data;
        data.transform.position.x = 77.0;
        if let ObjectKind::Bend(b) = &mut data.kind {
            b.size = Vec3::ONE;
        }
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::FitToParent(bend)]);
        assert_eq!(doc.scene.get(bend).unwrap().data.transform.position.x, 0.0);
        assert_eq!(editor.history.undo_label(), Some("Fit to Parent"));
        // Without a selection Shift + create adds at the top.
        editor.selection.clear();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateChild(ObjectKindTag::Bend)],
        );
        let top = editor.selection.active().unwrap();
        assert_eq!(doc.scene.get(top).unwrap().parent(), None);
    }

    #[test]
    fn points_mode_selects_an_ffds_lattice_and_delete_leaves_it() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Ffd)],
        );
        let id = editor.selection.active().unwrap();
        editor.mode = EditMode::Points;
        assert_eq!(spline_target(&doc, &editor), Some(id));
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::SelectComponents {
                node: id,
                component: Component::Point,
                hits: vec![26],
                mode: SelectMode::Replace,
            }],
        );
        let points = |doc: &Document| match &doc.scene.get(id).unwrap().data.kind {
            ObjectKind::Ffd {
                lattice, components, ..
            } => (
                lattice.spline().points.len(),
                components.selected().points.iter().collect::<Vec<_>>(),
            ),
            other => panic!("{other:?}"),
        };
        assert_eq!(points(&doc), (27, vec![26]));
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::DeleteSelected]);
        assert_eq!(points(&doc), (27, vec![26]), "lattice points are not deleted");
        assert_eq!(editor.history.undo_len(), steps);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::SelectAll]);
        assert_eq!(points(&doc).1.len(), 27);
    }

    #[test]
    fn current_state_adds_a_bent_cube_after_it_in_one_step() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let cube = editor.selection.active().unwrap();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateChild(ObjectKindTag::Bend)],
        );
        let bend = editor.selection.active().unwrap();
        if let ObjectKind::Bend(b) = &mut doc.scene.get_mut(bend).unwrap().data.kind {
            b.strength = 1.5;
        }
        editor.selection.set_single(cube);
        let steps = editor.history.undo_len();
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::CurrentStateToObject]);
        let roots = doc.scene.roots().to_vec();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], cube, "the copy follows its original");
        let copy = roots[1];
        assert_eq!(editor.selection.as_slice(), [copy]);
        assert_eq!(editor.history.undo_len(), steps + 1);
        let node = doc.scene.get(copy).unwrap();
        assert_eq!(node.data.name, "Cube");
        assert!(node.children().is_empty(), "the Bend is not copied");
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("not a polygon object: {:?}", node.data.kind)
        };
        let original = &doc.scene.get(cube).unwrap().data.kind;
        assert!(matches!(original, ObjectKind::Cube { .. }), "the original is untouched");
        let plain = grid4d_eval::editable_mesh(original).unwrap();
        assert_eq!(mesh.mesh().point_count(), plain.point_count());
        assert_ne!(mesh.mesh().points(), plain.points(), "the bent shape");
        assert_eq!(doc.scene.children(cube), [bend]);

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.scene.roots(), [cube]);
        assert_eq!(editor.selection.as_slice(), [cube]);
    }

    #[test]
    fn current_state_copies_hierarchies_bakes_generators_and_freezes_animation() {
        use grid4d_scene::{Axis, Key, Track};
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let scene = &mut doc.scene;
        let group = scene.insert(Node::new("Group", ObjectKind::Null));
        let child = |scene: &mut Scene, parent, name: &str, tag: ObjectKindTag| {
            scene.insert_child(parent, Node::new(name, tag.default_kind())).unwrap()
        };
        let sds = child(scene, group, "SDS", ObjectKindTag::SubdivisionSurface);
        child(scene, sds, "Cage", ObjectKindTag::Cube);
        let extrude = child(scene, group, "Extrude", ObjectKindTag::Extrude);
        child(scene, extrude, "Rectangle", ObjectKindTag::Rectangle);
        let hidden = child(scene, group, "Hidden", ObjectKindTag::Sphere);
        child(scene, hidden, "Under Hidden", ObjectKindTag::Cube);
        scene.get_mut(hidden).unwrap().data.visible_in_editor = false;
        child(scene, group, "Light", ObjectKindTag::Light);
        scene.get_mut(sds).unwrap().data.tracks.push(Track {
            param: Param::Position(Axis::X),
            curve: grid4d_scene::Curve::from_keys([Key::new(0.0, 0.0), Key::new(10.0, 100.0)]),
        });
        doc.time.set_frame(5);
        editor.selection.set_all([group, sds]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::CurrentStateToObject]);

        let roots = doc.scene.roots().to_vec();
        assert_eq!(roots.len(), 2, "the SDS goes with its selected group");
        let copy = roots[1];
        assert_eq!(editor.selection.as_slice(), [copy]);
        assert!(matches!(doc.scene.get(copy).unwrap().data.kind, ObjectKind::Null));
        let names: Vec<&str> = doc
            .scene
            .children(copy)
            .iter()
            .map(|&id| doc.scene.get(id).unwrap().data.name.as_str())
            .collect();
        assert_eq!(names, ["SDS", "Extrude", "Light"], "hidden objects are left out");
        for &id in doc.scene.children(copy) {
            assert!(doc.scene.children(id).is_empty(), "consumed inputs are not copied");
        }
        let baked = &doc.scene.get(doc.scene.children(copy)[0]).unwrap().data;
        let ObjectKind::Polygon { mesh, .. } = &baked.kind else {
            panic!("the SDS is not baked")
        };
        assert!(mesh.mesh().face_count() > 6, "subdivided");
        assert!(baked.tracks.is_empty());
        assert!((baked.transform.position.x - 50.0).abs() < 1e-3, "frozen at frame 5");
        let extruded = &doc.scene.get(doc.scene.children(copy)[1]).unwrap().data.kind;
        assert!(matches!(extruded, ObjectKind::Polygon { .. }));
        assert!(matches!(
            doc.scene.get(doc.scene.children(copy)[2]).unwrap().data.kind,
            ObjectKind::Light { .. }
        ));
        assert_eq!(
            doc.scene.get(sds).unwrap().data.tracks.len(),
            1,
            "the original keeps its animation"
        );
    }

    fn cloner_of_cubes(doc: &mut Document) -> (NodeId, NodeId) {
        let cloner = doc
            .scene
            .insert(Node::new("Cloner", ObjectKindTag::Cloner.default_kind()));
        let cube = doc
            .scene
            .insert_child(cloner, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        (cloner, cube)
    }

    #[test]
    fn make_editable_turns_a_cloner_into_a_null_of_its_clones() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let (cloner, cube) = cloner_of_cubes(&mut doc);
        editor.selection.set_single(cloner);
        assert!(can_make_editable(&doc, &editor.selection));
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::MakeEditable]);
        assert!(matches!(doc.scene.get(cloner).unwrap().data.kind, ObjectKind::Null));
        assert!(!doc.scene.contains(cube), "the original child goes");
        let clones = doc.scene.children(cloner).to_vec();
        let names: Vec<&str> = clones
            .iter()
            .map(|&id| doc.scene.get(id).unwrap().data.name.as_str())
            .collect();
        assert_eq!(names, ["Cube.0", "Cube.1", "Cube.2"]);
        let last = &doc.scene.get(clones[2]).unwrap().data;
        assert!(matches!(last.kind, ObjectKind::Polygon { .. }));
        assert_eq!(last.transform.position, Vec3::new(0.0, 500.0, 0.0));

        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(matches!(
            doc.scene.get(cloner).unwrap().data.kind,
            ObjectKind::Cloner { .. }
        ));
        assert_eq!(doc.scene.children(cloner), [cube]);
    }

    #[test]
    fn current_state_and_export_write_clones_as_objects() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let (cloner, _) = cloner_of_cubes(&mut doc);
        editor.selection.set_single(cloner);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::CurrentStateToObject]);
        let copy = doc.scene.roots()[1];
        assert!(matches!(doc.scene.get(copy).unwrap().data.kind, ObjectKind::Null));
        assert_eq!(doc.scene.children(copy).len(), 3);
        assert!(
            doc.scene
                .children(copy)
                .iter()
                .all(|&id| matches!(doc.scene.get(id).unwrap().data.kind, ObjectKind::Polygon { .. }))
        );

        let objects = file::exportable(&doc);
        let index = objects.nodes.iter().position(|e| e.data.name == "Cloner").unwrap();
        let clones: Vec<&str> = objects
            .nodes
            .iter()
            .filter(|e| e.parent == Some(index))
            .map(|e| e.data.name.as_str())
            .collect();
        assert_eq!(
            clones,
            ["Cube.0", "Cube.1", "Cube.2"],
            "the Cloner's clones, not its child"
        );
    }

    #[test]
    fn an_effector_created_with_a_cloner_selected_joins_its_list_in_one_step() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let (cloner, _) = cloner_of_cubes(&mut doc);
        editor.selection.set_single(cloner);
        let steps = editor.history.undo_len();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::PlainEffector)],
        );
        let effector = editor.selection.active().unwrap();
        assert!(doc.scene.get(effector).unwrap().data.kind.is_effector());
        let list = |doc: &Document| match &doc.scene.get(cloner).unwrap().data.kind {
            ObjectKind::Cloner { effectors, .. } => effectors.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(list(&doc), [effector]);
        assert_eq!(editor.history.undo_len(), steps + 1);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(list(&doc).is_empty());
        assert!(!doc.scene.contains(effector));
        assert_eq!(editor.selection.as_slice(), [cloner]);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(list(&doc), [effector]);

        // Without a Cloner selected it is just a new object.
        editor.selection.clear();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::RandomEffector)],
        );
        assert_eq!(list(&doc), [effector]);
    }

    #[test]
    fn rigid_and_collider_bodies_are_one_tag_kind_added_in_one_step() {
        use grid4d_scene::{Body, Motion, Tag, TagKind};
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CreateObject(ObjectKindTag::Cube)],
        );
        let cube = editor.selection.active().unwrap();
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(Tag::Dynamics(Body::collider()))],
        );
        let tags = |doc: &Document| doc.scene.get(cube).unwrap().data.tags.clone();
        assert_eq!(tags(&doc), [Tag::Dynamics(Body::collider())]);
        assert_eq!(editor.tag_selection.map(|t| t.kind), Some(TagKind::Dynamics));
        // A Rigid Body is the same kind: the object keeps its one Dynamics tag.
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::AddTag(TagKind::Dynamics.default_tag())],
        );
        let [Tag::Dynamics(body)] = tags(&doc)[..] else {
            panic!("{:?}", tags(&doc))
        };
        assert_eq!(body.motion, Motion::Collider);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert!(tags(&doc).is_empty());
    }

    #[test]
    fn simulation_settings_are_one_undo_step() {
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let before = doc.simulation;
        doc.simulation.steps_per_frame = 20;
        run(
            &mut doc,
            &mut editor,
            &mut vp,
            &[EditorCommand::CommitLive {
                inverse: SceneEdit::SetSimulation(before),
                label: "Simulation Settings",
            }],
        );
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Undo]);
        assert_eq!(doc.simulation, before);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::Redo]);
        assert_eq!(doc.simulation.steps_per_frame, 20);
    }

    #[test]
    fn current_state_and_export_take_the_simulated_pose() {
        use grid4d_scene::{Body, NodeData, Tag};
        let (mut doc, mut editor, mut vp) = (Document::new("t"), EditorState::default(), viewport());
        let mut floor = NodeData::new("Floor", ObjectKindTag::Plane.default_kind());
        floor.tags.push(Tag::Dynamics(Body::collider()));
        doc.scene.insert(Node::from_data(floor));
        let mut cube = NodeData::new("Cube", ObjectKindTag::Cube.default_kind());
        cube.transform.position.y = 600.0;
        cube.tags.push(Tag::Dynamics(Body::rigid()));
        let cube = doc.scene.insert(Node::from_data(cube));
        doc.time.set_frame(20);
        while editor.simulation.advance(&doc, 20) {}
        editor.selection.set_single(cube);
        run(&mut doc, &mut editor, &mut vp, &[EditorCommand::CurrentStateToObject]);
        let copy = doc.scene.roots()[2];
        let y = doc.scene.get(copy).unwrap().data.transform.position.y;
        assert!(y < 500.0, "where it has fallen to at frame 20: {y}");
        assert_eq!(doc.scene.get(cube).unwrap().data.transform.position.y, 600.0);

        let mut simulated = grid4d_eval::Overrides::default();
        editor.simulation.overrides(20, &mut simulated);
        let objects = file::exportable_at(&doc, &simulated);
        let exported = objects.nodes.iter().find(|e| e.data.name == "Cube").unwrap();
        assert!((exported.data.transform.position.y - y).abs() < 1e-3);
    }
}
