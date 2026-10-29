//! The document's file on disk: where it lives, whether it has unsaved changes,
//! the "Save changes?" flow, native file dialogs and the window title.
//!
//! `commands::apply` calls the operations here; panels only queue commands and
//! show [`DocumentFile`] state. File I/O never happens inside a panel.

use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
};

use egui::ViewportCommand;
use grid4d_io::{ObjectEntry, Objects};
use grid4d_raytrace::Image;
use grid4d_scene::{Document, ImageFormat, MeshHandle, NodeData, NodeId, ObjectKind, SceneEdit, Tags};

use crate::{
    EditorCommand, EditorState, History, ViewportPanel,
    recent::RecentFiles,
    rendering::{FinishedFrame, Output, SaveTarget},
};

/// Every extension File ▸ Open and File ▸ Merge accept.
const ANY_EXTENSIONS: [&str; 5] = ["g4d", "oc4d", "gltf", "glb", "obj"];

/// Native open/save dialogs, behind a trait so tests never open a window.
pub trait FileDialogs: std::fmt::Debug {
    /// A document to open, or `None` if the user cancelled.
    fn pick_open(&mut self, directory: Option<&Path>) -> Option<PathBuf>;
    /// Where to save, or `None` if the user cancelled. `suggested` is a bare
    /// file name such as `Untitled 1.g4d`.
    fn pick_save(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf>;
    /// A file to merge into the document (File ▸ Merge), or `None` if cancelled.
    fn pick_merge(&mut self, directory: Option<&Path>) -> Option<PathBuf> {
        self.pick_open(directory)
    }
    /// Where to export (File ▸ Export): a `.glb` or `.gltf` path, or `None` if
    /// cancelled. `suggested` is a bare file name such as `Untitled 1.glb`.
    fn pick_export(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        self.pick_save(directory, suggested)
    }
    /// Where renders are saved (Render Settings ▸ Save ▸ File), or `None` if
    /// cancelled. `suggested` is a bare file name such as `Untitled 1.png`.
    fn pick_render_output(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        self.pick_save(directory, suggested)
    }
}

/// The platform dialogs (Win32, Cocoa, XDG desktop portal), via `rfd`.
#[derive(Debug, Default)]
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn pick_open(&mut self, directory: Option<&Path>) -> Option<PathBuf> {
        with_filters(with_directory(rfd::FileDialog::new(), directory))
            .set_title("Open")
            .pick_file()
    }

    fn pick_merge(&mut self, directory: Option<&Path>) -> Option<PathBuf> {
        with_filters(with_directory(rfd::FileDialog::new(), directory))
            .set_title("Merge")
            .pick_file()
    }

    fn pick_save(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        with_directory(rfd::FileDialog::new(), directory)
            .set_title("Save As")
            .add_filter("Grid4D Document", &[grid4d_io::EXTENSION])
            .set_file_name(suggested)
            .save_file()
            .map(with_extension)
    }

    fn pick_export(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        with_directory(rfd::FileDialog::new(), directory)
            .set_title("Export")
            .add_filter("glTF Binary (.glb)", &["glb"])
            .add_filter("glTF (.gltf + .bin)", &["gltf"])
            .add_filter("Wavefront OBJ (.obj + .mtl)", &["obj"])
            .set_file_name(suggested)
            .save_file()
            .map(with_export_extension)
    }

    fn pick_render_output(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        with_directory(rfd::FileDialog::new(), directory)
            .set_title("Save Render As")
            .add_filter("Pictures (PNG, OpenEXR)", &["png", "exr"])
            .set_file_name(suggested)
            .save_file()
    }
}

/// A typed export name without `.glb`, `.gltf` or `.obj` gets `.glb`.
fn with_export_extension(path: PathBuf) -> PathBuf {
    let known = path.extension().is_some_and(|ext| {
        grid4d_io::EXPORT_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    });
    if known {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".glb");
    PathBuf::from(name)
}

/// Grid4D documents and models (glTF, OBJ), together and each on its own.
fn with_filters(dialog: rfd::FileDialog) -> rfd::FileDialog {
    dialog
        .add_filter("Grid4D Document or Model", &ANY_EXTENSIONS)
        .add_filter("Grid4D Document", &grid4d_io::OPEN_EXTENSIONS)
        .add_filter("glTF 2.0", &["gltf", "glb"])
        .add_filter("Wavefront OBJ", &["obj"])
}

fn with_directory(dialog: rfd::FileDialog, directory: Option<&Path>) -> rfd::FileDialog {
    match directory {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    }
}

/// Some dialogs return the typed name without the extension: append it (rather
/// than replacing, so `shot.v2` becomes `shot.v2.g4d`). A typed legacy `.oc4d`
/// name is kept.
fn with_extension(path: PathBuf) -> PathBuf {
    if is_document(&path) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(grid4d_io::EXTENSION);
    PathBuf::from(name)
}

/// An action that would replace or close the current document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    /// Ask for a document, then open it.
    Open,
    /// Open this document (File ▸ Open Recent).
    OpenPath(PathBuf),
    Quit,
}

/// The answer to "Save changes?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// A message for the user, shown as a modal by `panels::alert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub title: &'static str,
    pub message: String,
}

/// Session state of the document's file. Lives in [`EditorState::file`].
#[derive(Debug)]
pub struct DocumentFile {
    path: Option<PathBuf>,
    /// [`History::state_id`] when the document was last saved, opened or created.
    saved_state: u64,
    /// The action waiting for an answer to "Save changes?".
    pending: Option<FileAction>,
    /// Quit was confirmed: close the window and let the close request through.
    quitting: bool,
    pub dialogs: Box<dyn FileDialogs>,
    /// An error to show; the alert panel clears it when dismissed.
    pub alert: Option<Alert>,
    /// File ▸ Open Recent; the application loads and stores it.
    pub recent: RecentFiles,
    /// What the window title was last set to (to update it only on change).
    title_name: String,
    title_dirty: bool,
    title_set: bool,
}

impl Default for DocumentFile {
    fn default() -> Self {
        Self::with_dialogs(Box::new(NativeDialogs))
    }
}

impl DocumentFile {
    /// A file state for a new, unsaved document, using `dialogs` to ask for paths.
    #[must_use]
    pub fn with_dialogs(dialogs: Box<dyn FileDialogs>) -> Self {
        Self {
            path: None,
            saved_state: 0,
            pending: None,
            quitting: false,
            dialogs,
            alert: None,
            recent: RecentFiles::default(),
            title_name: String::new(),
            title_dirty: false,
            title_set: false,
        }
    }

    /// Where the document was last saved or opened from.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// `true` if the document changed since it was last saved, opened or created.
    /// Undoing back to that state makes it clean again.
    #[must_use]
    pub fn is_dirty(&self, history: &History) -> bool {
        history.state_id() != self.saved_state
    }

    /// The action waiting for an answer to "Save changes?", if any.
    #[must_use]
    pub fn pending(&self) -> Option<&FileAction> {
        self.pending.as_ref()
    }

    #[must_use]
    pub fn is_quitting(&self) -> bool {
        self.quitting
    }

    /// The file name once saved, otherwise the document's name.
    #[must_use]
    pub fn display_name<'a>(&'a self, doc: &'a Document) -> Cow<'a, str> {
        match self.path.as_deref().and_then(Path::file_name) {
            Some(name) => name.to_string_lossy(),
            None => Cow::Borrowed(doc.name.as_str()),
        }
    }

    /// The new window title if it changed since the last call ("shot.g4d * — Grid4D").
    /// Allocates only when the title changes.
    pub fn title_update(&mut self, doc: &Document, history: &History) -> Option<String> {
        let dirty = self.is_dirty(history);
        let unchanged = {
            let name = self.display_name(doc);
            self.title_set && dirty == self.title_dirty && name == self.title_name.as_str()
        };
        if unchanged {
            return None;
        }
        self.title_name = self.display_name(doc).into_owned();
        self.title_dirty = dirty;
        self.title_set = true;
        Some(format!("{}{} — Grid4D", self.title_name, if dirty { " *" } else { "" }))
    }

    fn directory(&self) -> Option<&Path> {
        self.path.as_deref().and_then(Path::parent)
    }
}

/// Runs `action` now, or first asks "Save changes?" if the document is dirty.
pub(crate) fn request(action: FileAction, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    if editor.file.is_dirty(&editor.history) {
        editor.file.pending = Some(action);
    } else {
        perform(action, doc, editor, viewport);
    }
}

/// Resolves a pending "Save changes?" prompt.
pub(crate) fn answer(
    choice: UnsavedChoice,
    doc: &mut Document,
    editor: &mut EditorState,
    viewport: &mut ViewportPanel,
) {
    let Some(action) = editor.file.pending.take() else {
        return;
    };
    match choice {
        // A cancelled or failed save keeps the document: the action is dropped.
        UnsavedChoice::Save => {
            if save(doc, editor, false) {
                perform(action, doc, editor, viewport);
            }
        }
        UnsavedChoice::Discard => perform(action, doc, editor, viewport),
        UnsavedChoice::Cancel => {}
    }
}

fn perform(action: FileAction, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    match action {
        FileAction::New => replace(doc, editor, viewport, Document::new("Untitled 1"), None),
        FileAction::Open => {
            let directory = editor.file.directory().map(Path::to_path_buf);
            if let Some(path) = editor.file.dialogs.pick_open(directory.as_deref()) {
                open_path(path, doc, editor, viewport);
            }
        }
        FileAction::OpenPath(path) => open_path(path, doc, editor, viewport),
        FileAction::Quit => editor.file.quitting = true,
    }
}

/// Loads `path` and makes it the current document. A glTF file becomes a new,
/// untitled document named after it (saving asks for a `.g4d` path). On error
/// the current document stays and an alert explains why.
pub(crate) fn open_path(path: PathBuf, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    let loaded = if grid4d_io::is_importable(&path) {
        let name = path
            .file_stem()
            .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy());
        grid4d_io::import(&path).map(|imported| (imported.into_document(&name), None))
    } else {
        grid4d_io::load(&path).map(|doc| (doc, Some(path.clone())))
    };
    match loaded {
        Ok((loaded, saved_path)) => {
            editor.file.recent.touch(&path);
            replace(doc, editor, viewport, loaded, saved_path);
            viewport.frame_scene(&doc.scene);
        }
        Err(err) => {
            // A recent entry whose file is gone is of no further use.
            if matches!(&err, grid4d_io::IoError::Io(io) if io.kind() == std::io::ErrorKind::NotFound) {
                editor.file.recent.remove(&path);
            }
            editor.file.alert = Some(Alert {
                title: "Could Not Open Document",
                message: format!("{}\n\n{err}", path.display()),
            });
        }
    }
}

fn replace(
    doc: &mut Document,
    editor: &mut EditorState,
    viewport: &mut ViewportPanel,
    new: Document,
    path: Option<PathBuf>,
) {
    editor.history.clear(doc);
    *doc = new;
    editor.selection.clear();
    viewport.reset_view();
    editor.file.path = path;
    editor.file.saved_state = editor.history.state_id();
}

/// File ▸ Merge: adds the objects of `path` (a glTF file or a Grid4D document)
/// to the document at the top level, as one undo step, and selects them. On
/// error nothing changes and an alert explains why.
pub(crate) fn merge(path: &Path, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    let failed = |editor: &mut EditorState, message: String| {
        editor.file.alert = Some(Alert {
            title: "Could Not Merge File",
            message: format!("{}\n\n{message}", path.display()),
        });
    };
    let imported = match grid4d_io::import(path) {
        Ok(imported) => imported,
        Err(err) => return failed(editor, err.to_string()),
    };
    let selection_before = editor.selection.clone();
    let mut ids: Vec<NodeId> = Vec::with_capacity(imported.nodes.len());
    let mut roots: Vec<NodeId> = Vec::new();
    let mut inverses = Vec::with_capacity(imported.nodes.len() + 2);
    // The file's materials join the document's (with unique names); its Material
    // tags are pointed at them.
    let mut material_of: HashMap<grid4d_scene::MaterialId, grid4d_scene::MaterialId> = HashMap::new();
    if !imported.materials.is_empty() {
        let mut materials = doc.scene.materials().clone();
        for (old, material) in imported.materials.iter() {
            let mut material = material.clone();
            material.name = materials.unique_name(&material.name);
            material_of.insert(old, materials.insert(material));
        }
        match doc.apply(SceneEdit::SetMaterials(materials)) {
            Ok(inverse) => inverses.push(inverse),
            Err(err) => return failed(editor, err.to_string()),
        }
    }
    // Object references (Target tags) are resolved once every object has its id.
    let mut later: Vec<(usize, NodeData)> = Vec::new();
    for (index, node) in imported.nodes.into_iter().enumerate() {
        let parent = node.parent.and_then(|p| ids.get(p).copied());
        let mut data = node.data;
        grid4d_io::remap_materials(&mut data, |m| material_of.get(&m).copied());
        let refers = data.tags.iter().any(|t| {
            matches!(
                t,
                grid4d_scene::Tag::Target { target: Some(_), .. } | grid4d_scene::Tag::Target { up: Some(_), .. }
            )
        }) || matches!(data.kind, ObjectKind::Cloner { object: Some(_), .. });
        if refers {
            later.push((index, data.clone()));
            grid4d_io::remap_objects(&mut data, |_| None);
        }
        let insert = SceneEdit::Insert {
            parent,
            index: usize::MAX,
            data,
        };
        match doc.apply(insert) {
            Ok(inverse) => {
                let Some(id) = inverse.node() else { continue };
                if parent.is_none() {
                    roots.push(id);
                }
                ids.push(id);
                inverses.push(inverse);
            }
            Err(err) => {
                // All or nothing: take back what was inserted.
                for inverse in inverses.into_iter().rev() {
                    let _ = doc.apply(inverse);
                }
                return failed(editor, err.to_string());
            }
        }
    }
    if roots.is_empty() {
        for inverse in inverses.into_iter().rev() {
            let _ = doc.apply(inverse);
        }
        return failed(editor, "The file contains no objects.".to_owned());
    }
    let node_of: HashMap<NodeId, NodeId> = imported.sources.iter().copied().zip(ids.iter().copied()).collect();
    for (index, mut data) in later {
        let Some(&id) = ids.get(index) else { continue };
        grid4d_io::remap_objects(&mut data, |old| node_of.get(&old).copied());
        if let Ok(inverse) = doc.apply(SceneEdit::SetData { id, data }) {
            inverses.push(inverse);
        }
    }
    inverses.reverse(); // newest first: references, then children before parents, then materials
    editor.selection.set_all(roots);
    let selection_after = editor.selection.clone();
    editor
        .history
        .record(doc, "Merge", inverses, selection_before, selection_after);
    viewport.frame_selection(&doc.scene, &editor.selection);
}

/// File ▸ Export: asks where, then writes the document as glTF (`.glb`, or
/// `.gltf` + `.bin`) or OBJ (`.obj` + `.mtl`), by the chosen extension. The
/// document does not change; a failure shows an alert.
pub(crate) fn export(doc: &Document, editor: &mut EditorState) {
    let suggested = format!("{}.glb", file_stem_for(&editor.file.display_name(doc)));
    let directory = editor.file.directory().map(Path::to_path_buf);
    let Some(path) = editor.file.dialogs.pick_export(directory.as_deref(), &suggested) else {
        return;
    };
    if let Err(err) = grid4d_io::export(&exportable(doc), &path) {
        editor.file.alert = Some(Alert {
            title: "Could Not Export",
            message: format!("{}\n\n{err}", path.display()),
        });
    }
}

/// Saves a frame a render has just finished where its Render Settings said
/// (its folder made if needed; a numbered file for an animation, PNG or
/// OpenEXR), and notes the outcome on the picture. The first failure of a
/// render also shows an alert.
pub(crate) fn save_rendered_frame(editor: &mut EditorState, done: &FinishedFrame) {
    let Some(picture) = editor.renders.picture(done.number) else {
        return;
    };
    let output = match &picture.target {
        SaveTarget::File(output) => output,
        SaveTarget::Nowhere(why) => {
            let why = (*why).to_owned();
            editor.renders.set_note(done.number, why);
            return;
        }
    };
    let Some(path) = output.file(done.frame) else {
        return;
    };
    match write_frame(&done.image, output, &path) {
        Ok(()) => editor.renders.record_saved(done.number, &path),
        Err(err) => {
            let first_failure = !picture.note.starts_with("Not saved");
            if first_failure {
                editor.file.alert = Some(Alert {
                    title: "Could Not Save the Render",
                    message: format!("{}\n\n{err}", path.display()),
                });
            }
            editor.renders.set_note(done.number, format!("Not saved: {err}"));
        }
    }
}

/// Writes a finished frame in `output`'s format and alpha setting, making its
/// folder if needed: sRGB PNG, or OpenEXR in linear light.
fn write_frame(image: &Image, output: &Output, path: &Path) -> Result<(), grid4d_io::IoError> {
    make_folder(path)?;
    let channels = if output.alpha() {
        grid4d_io::Channels::Rgba
    } else {
        grid4d_io::Channels::Rgb
    };
    match output.format() {
        ImageFormat::Png => {
            let pixels = if output.alpha() {
                image.to_rgba8()
            } else {
                image.to_rgb8()
            };
            grid4d_io::save_png(path, image.width, image.height, channels, &pixels)
        }
        ImageFormat::Exr => grid4d_io::save_exr(path, image.width, image.height, channels, &image.pixels),
    }
}

/// The Picture Viewer's Save As…: asks where, then saves the shown picture as
/// PNG (with transparency if it was rendered with Alpha Channel).
pub(crate) fn save_render_as(doc: &Document, editor: &mut EditorState) {
    let (renders, file) = (&mut editor.renders, &mut editor.file);
    let Some(picture) = renders.shown_picture().filter(|p| p.has_pixels()) else {
        return;
    };
    let png = ImageFormat::Png.extension();
    let current = match &picture.target {
        SaveTarget::File(output) => output.file(picture.frame),
        SaveTarget::Nowhere(_) => None,
    };
    let directory = current
        .as_deref()
        .and_then(Path::parent)
        .or_else(|| file.directory())
        .map(Path::to_path_buf);
    let suggested = current.as_deref().and_then(Path::file_stem).map_or_else(
        || format!("{} {}", file_stem_for(&file.display_name(doc)), picture.number),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let Some(mut path) = file
        .dialogs
        .pick_render_output(directory.as_deref(), &format!("{suggested}.{png}"))
    else {
        return;
    };
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(png)) {
        path.as_mut_os_string().push(".png");
    }
    let number = picture.number;
    let (pixels, channels) = picture.pixels();
    let written =
        make_folder(&path).and_then(|()| grid4d_io::save_png(&path, picture.width, picture.height, channels, pixels));
    match written {
        Ok(()) => renders.set_note(number, format!("Saved to {}", path.display())),
        Err(err) => {
            file.alert = Some(Alert {
                title: "Could Not Save the Render",
                message: format!("{}\n\n{err}", path.display()),
            });
        }
    }
}

/// Makes the folder `path` goes in, if it has one.
fn make_folder(path: &Path) -> Result<(), grid4d_io::IoError> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)?;
    }
    Ok(())
}

/// Render Settings ▸ Save ▸ File ▸ …: asks where renders go and returns the
/// File setting for it — relative to the document's folder (with `/`) when the
/// file is inside it, else the full path — or `None` if cancelled.
pub(crate) fn pick_render_output(doc: &Document, editor: &mut EditorState) -> Option<String> {
    let current = doc.render.output_path(editor.file.path());
    let directory = current
        .as_deref()
        .and_then(Path::parent)
        .or_else(|| editor.file.directory())
        .map(Path::to_path_buf);
    let suggested = current.as_deref().and_then(Path::file_name).map_or_else(
        || {
            format!(
                "{}.{}",
                file_stem_for(&editor.file.display_name(doc)),
                doc.render.format.extension()
            )
        },
        |name| name.to_string_lossy().into_owned(),
    );
    let path = editor
        .file
        .dialogs
        .pick_render_output(directory.as_deref(), &suggested)?;
    let relative = editor
        .file
        .directory()
        .and_then(|folder| path.strip_prefix(folder).ok())
        .map(|inside| {
            inside
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        });
    Some(relative.unwrap_or_else(|| path.to_string_lossy().into_owned()))
}

/// The document's objects as an export writes them: every primitive as a polygon
/// object of its current shape, every switched-on generator as one holding its
/// result in the colour it shows (its inputs left out: the cage, profile and
/// path, sections), a switched-off generator as a Null over its children.
#[must_use]
pub fn exportable(doc: &Document) -> Objects {
    let mut nodes = Vec::new();
    let mut index_of: HashMap<NodeId, usize> = HashMap::new();
    let mut stack: Vec<NodeId> = doc.scene.roots().iter().rev().copied().collect();
    // Deformed shapes as the viewport shows them at the current frame.
    let mut evaluated = grid4d_eval::EvaluatedScene::new();
    evaluated.set_frame(f64::from(doc.time.frame()));
    evaluated.update(&doc.scene);
    while let Some(id) = stack.pop() {
        let Some(node) = doc.scene.get(id) else { continue };
        let mut data = node.data.clone();
        let mut cage = None;
        let mut inputs: &[NodeId] = &[];
        if data.kind.is_generator() {
            match grid4d_eval::generator_mesh(&doc.scene, id, f64::from(doc.time.frame())).filter(|_| data.enabled) {
                Some(mesh) => {
                    let taken = node.children().len().min(data.kind.generator_inputs());
                    inputs = &node.children()[..taken];
                    cage = inputs.first().copied();
                    if let Some(colour) = grid4d_eval::generator_color(&doc.scene, id) {
                        data.display_color = colour;
                    }
                    data.kind = ObjectKind::polygon(MeshHandle::new(mesh));
                }
                None => {
                    // A Cloner's children are written as its clones (below).
                    if data.enabled && matches!(data.kind, ObjectKind::Cloner { .. }) {
                        inputs = node.children();
                    }
                    data.kind = ObjectKind::Null;
                }
            }
        } else if let Some(mesh) = grid4d_eval::editable_mesh(&data.kind_at(f64::from(doc.time.frame()))) {
            data.kind = ObjectKind::polygon(MeshHandle::new(mesh));
        }
        if let Some(mesh) = evaluated.deformed(id) {
            data.kind = ObjectKind::polygon(mesh.clone());
        }
        // A generator without a material shows its cage's.
        if let Some(cage) = cage.and_then(|c| doc.scene.get(c))
            && data.tags.material().is_none()
            && let Some(material) = cage.data.tags.material()
        {
            data.tags.push(grid4d_scene::Tag::Material {
                material: Some(material),
            });
        }
        let parent = node.parent().and_then(|p| index_of.get(&p).copied());
        let index = nodes.len();
        index_of.insert(id, index);
        let cloner = node.data.enabled && matches!(node.data.kind, ObjectKind::Cloner { .. });
        nodes.push(ObjectEntry { parent, data });
        // A Cloner is written as a group of its clones.
        if cloner {
            let frame = f64::from(doc.time.frame());
            nodes.extend(
                crate::current_state::clone_objects(&evaluated, &doc.scene, id, frame)
                    .into_iter()
                    .map(|data| ObjectEntry {
                        parent: Some(index),
                        data,
                    }),
            );
        }
        stack.extend(node.children().iter().rev().filter(|child| !inputs.contains(child)));
    }
    Objects {
        nodes,
        materials: doc.scene.materials().clone(),
        sources: Vec::new(),
    }
}

/// Saves to the current file, or asks where (always with `ask_path`, i.e. Save As).
/// Returns `true` if the document was written.
pub(crate) fn save(doc: &Document, editor: &mut EditorState, ask_path: bool) -> bool {
    let path = match (&editor.file.path, ask_path) {
        (Some(path), false) => path.clone(),
        _ => {
            let suggested = format!(
                "{}.{}",
                file_stem_for(&editor.file.display_name(doc)),
                grid4d_io::EXTENSION
            );
            let directory = editor.file.directory().map(Path::to_path_buf);
            let Some(path) = editor.file.dialogs.pick_save(directory.as_deref(), &suggested) else {
                return false;
            };
            path
        }
    };
    match grid4d_io::save(doc, &path) {
        Ok(()) => {
            editor.file.recent.touch(&path);
            editor.file.path = Some(path);
            editor.file.saved_state = editor.history.state_id();
            true
        }
        Err(err) => {
            editor.file.alert = Some(Alert {
                title: "Could Not Save Document",
                message: format!("{}\n\n{err}", path.display()),
            });
            false
        }
    }
}

/// A name usable as a file stem: drops a trailing `.g4d` (or legacy `.oc4d`) and
/// replaces characters that are invalid in file names on some platform.
fn file_stem_for(name: &str) -> String {
    let stem = grid4d_io::OPEN_EXTENSIONS
        .iter()
        .find_map(|ext| name.strip_suffix(ext).and_then(|s| s.strip_suffix('.')))
        .unwrap_or(name);
    let cleaned: String = stem
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Untitled".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// `true` for paths with a document extension, `.g4d` or legacy `.oc4d` (any letter case).
#[must_use]
pub fn is_document(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        grid4d_io::OPEN_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

/// Opens a document dropped onto the window (asking "Save changes?" first),
/// merges a dropped glTF file, or explains why the dropped files cannot be used.
/// Call once per frame, before the commands are applied.
pub fn accept_dropped_files(ctx: &egui::Context, editor: &mut EditorState) {
    // Allocates only on the frame something was dropped.
    let choice = ctx.input(|i| {
        let paths: Vec<&Path> = i.raw.dropped_files.iter().map(|file| file.path()).collect();
        choose_dropped(&paths)
    });
    match choice {
        None => {}
        Some(Ok(DropAction::Open(path))) => editor.commands.push(EditorCommand::OpenFile(path)),
        Some(Ok(DropAction::Merge(path))) => editor.commands.push(EditorCommand::MergeFile(path)),
        Some(Err(alert)) => editor.file.alert = Some(alert),
    }
}

/// What a drop does.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DropAction {
    /// A Grid4D document: open it.
    Open(PathBuf),
    /// A glTF file: merge it (the reference's "Merge on Drag & Drop").
    Merge(PathBuf),
}

/// The first `.g4d` among `paths` (opened), else the first glTF file (merged);
/// `None` if nothing was dropped, an alert if none of them can be used. (One
/// document is open at a time, so further files in the same drop are ignored.)
fn choose_dropped(paths: &[&Path]) -> Option<Result<DropAction, Alert>> {
    if paths.is_empty() {
        return None;
    }
    if let Some(path) = paths.iter().find(|path| is_document(path)) {
        return Some(Ok(DropAction::Open(path.to_path_buf())));
    }
    Some(match paths.iter().find(|path| grid4d_io::is_importable(path)) {
        Some(path) => Ok(DropAction::Merge(path.to_path_buf())),
        None => {
            let mut message = format!(
                "None of the dropped files is a Grid4D document (.{}) or a model (.gltf, .glb, .obj):\n",
                grid4d_io::EXTENSION
            );
            for path in paths {
                message.push('\n');
                message.push_str(&path.file_name().unwrap_or(path.as_os_str()).to_string_lossy());
            }
            Err(Alert {
                title: "Cannot Open Dropped Files",
                message,
            })
        }
    })
}

/// Keeps the window in step with the document file, once per frame after the
/// commands ran: the close button asks "Save changes?" first, a confirmed quit
/// closes the window, and the title shows the name and a `*` for unsaved changes.
pub fn sync_window(ctx: &egui::Context, doc: &Document, editor: &mut EditorState) {
    let close_requested = ctx.input(|i| i.viewport().close_requested());
    if close_requested && !editor.file.quitting && editor.file.is_dirty(&editor.history) {
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        editor.commands.push(EditorCommand::Quit);
    }
    if editor.file.quitting {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    if let Some(title) = editor.file.title_update(doc, &editor.history) {
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_appended_not_replaced() {
        assert_eq!(with_extension(PathBuf::from("a/shot")), PathBuf::from("a/shot.g4d"));
        assert_eq!(with_extension(PathBuf::from("a/shot.g4d")), PathBuf::from("a/shot.g4d"));
        assert_eq!(with_extension(PathBuf::from("shot.v2")), PathBuf::from("shot.v2.g4d"));
        assert_eq!(with_extension(PathBuf::from("old.oc4d")), PathBuf::from("old.oc4d"));
    }

    #[test]
    fn legacy_oc4d_files_are_documents() {
        for path in ["shot.g4d", "SHOT.G4D", "old.oc4d", "a/Old.OC4D"] {
            assert!(is_document(Path::new(path)), "{path}");
        }
        for path in ["notes.txt", "g4d", "shot.g4d.bak", "shot.c4d"] {
            assert!(!is_document(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn suggested_names_are_valid_file_names() {
        assert_eq!(file_stem_for("Untitled 1"), "Untitled 1");
        assert_eq!(file_stem_for("shot.g4d"), "shot");
        assert_eq!(file_stem_for("old.oc4d"), "old");
        assert_eq!(file_stem_for("a/b:c*?"), "a_b_c__");
        assert_eq!(file_stem_for("   "), "Untitled");
    }

    #[test]
    fn the_title_changes_only_when_needed() {
        let doc = Document::new("Untitled 1");
        let mut history = History::default();
        let mut file = DocumentFile::with_dialogs(Box::new(NativeDialogs));
        assert_eq!(
            file.title_update(&doc, &history).as_deref(),
            Some("Untitled 1 — Grid4D")
        );
        assert_eq!(file.title_update(&doc, &history), None);

        let mut scratch = Document::new("x");
        history.record(
            &mut scratch,
            "Edit",
            vec![SceneEdit::SetName("x".to_owned())],
            crate::Selection::default(),
            crate::Selection::default(),
        );
        assert_eq!(
            file.title_update(&doc, &history).as_deref(),
            Some("Untitled 1 * — Grid4D")
        );
        file.path = Some(PathBuf::from("renders/shot.g4d"));
        assert_eq!(
            file.title_update(&doc, &history).as_deref(),
            Some("shot.g4d * — Grid4D")
        );
    }

    // ── The New / Open / Save / Quit workflow ───────────────────────────────

    use std::{cell::RefCell, rc::Rc};

    use egui::{RawInput, ViewportEvent, ViewportId, ViewportInfo};
    use grid4d_scene::ObjectKindTag;

    use crate::{EditorCommand, commands};

    /// What the fake dialogs were asked, shared with the test.
    #[derive(Debug, Default)]
    struct DialogLog {
        open_calls: usize,
        save_suggestions: Vec<String>,
    }

    /// Answers dialogs with preset paths; never opens a window.
    #[derive(Debug)]
    struct FakeDialogs {
        open: Option<PathBuf>,
        save: Option<PathBuf>,
        /// The answer to Render Settings ▸ File ▸ ….
        render: Option<PathBuf>,
        log: Rc<RefCell<DialogLog>>,
    }

    impl FileDialogs for FakeDialogs {
        fn pick_open(&mut self, _directory: Option<&Path>) -> Option<PathBuf> {
            self.log.borrow_mut().open_calls += 1;
            self.open.clone()
        }

        fn pick_save(&mut self, _directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
            self.log.borrow_mut().save_suggestions.push(suggested.to_owned());
            self.save.clone()
        }

        fn pick_render_output(&mut self, _directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
            self.log.borrow_mut().save_suggestions.push(suggested.to_owned());
            self.render.clone()
        }
    }

    struct Fixture {
        doc: Document,
        editor: EditorState,
        viewport: ViewportPanel,
        log: Rc<RefCell<DialogLog>>,
        dir: PathBuf,
    }

    impl Fixture {
        /// The demo scene, with dialogs answering `open` / `save` (file names in a
        /// fresh temp directory unique to `test`).
        fn new(test: &str, open: Option<&str>, save: Option<&str>) -> Self {
            let dir = std::env::temp_dir().join(format!("grid4d-ui-{}-{test}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let log = Rc::new(RefCell::new(DialogLog::default()));
            let dialogs = FakeDialogs {
                open: open.map(|name| dir.join(name)),
                save: save.map(|name| dir.join(name)),
                render: Some(dir.join("renders").join("still.png")),
                log: Rc::clone(&log),
            };
            let editor = EditorState {
                file: DocumentFile::with_dialogs(Box::new(dialogs)),
                ..EditorState::default()
            };
            Self {
                doc: Document::with_demo_scene(),
                editor,
                viewport: ViewportPanel::new(None),
                log,
                dir,
            }
        }

        fn run(&mut self, commands: &[EditorCommand]) {
            for command in commands {
                self.editor.commands.push(command.clone());
            }
            commands::apply_pending(&mut self.doc, &mut self.editor, &mut self.viewport);
        }

        fn dirty(&self) -> bool {
            self.editor.file.is_dirty(&self.editor.history)
        }

        fn make_dirty(&mut self) {
            self.run(&[EditorCommand::CreateObject(ObjectKindTag::Cube)]);
            assert!(self.dirty());
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn the_render_output_is_stored_relative_to_the_document() {
        let mut f = Fixture::new("render_output", None, Some("shot.g4d"));
        // Not saved yet: the full path, and the document's name is suggested.
        f.run(&[EditorCommand::BrowseRenderOutput]);
        let full = f.dir.join("renders").join("still.png");
        assert_eq!(f.doc.render.output, full.to_string_lossy());
        assert_eq!(f.log.borrow().save_suggestions, ["Untitled 1.png"]);
        assert_eq!(f.editor.history.undo_label(), Some("Render Settings"));

        // Saved: inside its folder, relative with `/`; the file's name is suggested.
        f.run(&[EditorCommand::SaveAs, EditorCommand::BrowseRenderOutput]);
        assert_eq!(f.doc.render.output, "renders/still.png");
        assert_eq!(f.doc.render.output_path(f.editor.file.path()), Some(full));
        assert_eq!(
            f.log.borrow().save_suggestions.last().map(String::as_str),
            Some("still.png")
        );

        f.run(&[EditorCommand::Undo]);
        assert_eq!(
            f.doc.render.output,
            f.dir.join("renders").join("still.png").to_string_lossy()
        );
    }

    #[test]
    fn an_animation_is_saved_as_numbered_files() {
        let mut f = Fixture::new("render_animation", None, Some("shot.g4d"));
        crate::rendering::tests::small(&mut f.doc, 1);
        f.doc.render.frames = grid4d_scene::FrameRange::Manual { from: 3, to: 5 };
        f.doc.render.format = ImageFormat::Exr;
        f.run(&[EditorCommand::SaveAs, EditorCommand::BrowseRenderOutput]);
        f.run(&[EditorCommand::RenderToPictureViewer]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while f.editor.renders.is_rendering() {
            assert!(std::time::Instant::now() < deadline, "the render did not end");
            std::thread::sleep(std::time::Duration::from_millis(2));
            f.run(&[]);
        }
        let folder = f.dir.join("renders");
        for frame in ["0003", "0004", "0005"] {
            let file = folder.join(format!("still{frame}.exr"));
            let bytes = std::fs::read(&file).unwrap();
            assert!(
                bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]),
                "an OpenEXR file: {}",
                file.display()
            );
        }
        assert!(!folder.join("still.png").exists() && !folder.join("still.exr").exists());
        let picture = f.editor.renders.shown_picture().unwrap();
        assert_eq!((picture.frames, picture.saved), (3, 3));
        assert!(picture.note.starts_with("Saved 3 of 3 frames"), "{}", picture.note);
    }

    #[test]
    fn a_finished_render_is_saved_where_the_settings_say() {
        let mut f = Fixture::new("render_save", None, Some("shot.g4d"));
        crate::rendering::tests::small(&mut f.doc, 2);
        f.doc.render.alpha = true;
        f.run(&[EditorCommand::SaveAs, EditorCommand::BrowseRenderOutput]);
        f.run(&[EditorCommand::RenderToPictureViewer]);
        assert!(f.editor.renders.viewer_open);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while f.editor.renders.is_rendering() {
            assert!(std::time::Instant::now() < deadline, "the render did not end");
            std::thread::sleep(std::time::Duration::from_millis(2));
            f.run(&[]);
        }
        let file = f.dir.join("renders").join("still.png");
        let picture = f.editor.renders.shown_picture().unwrap();
        assert_eq!(picture.note, format!("Saved to {}", file.display()));
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&file).unwrap()));
        let info = decoder.read_info().unwrap().info().clone();
        assert_eq!((info.width, info.height), (24, 14));
        assert_eq!(info.color_type, png::ColorType::Rgba, "Alpha Channel on");

        // Save As writes the shown picture again (the fake dialog answers the same file).
        std::fs::remove_file(&file).unwrap();
        f.run(&[EditorCommand::SaveRenderAs]);
        assert!(file.exists());
        assert!(f.editor.file.alert.is_none());
    }

    #[test]
    fn save_as_writes_the_file_and_marks_the_document_clean() {
        let mut f = Fixture::new("save_as", None, Some("shot.g4d"));
        f.make_dirty();
        f.run(&[EditorCommand::SaveAs]);

        let path = f.dir.join("shot.g4d");
        assert_eq!(f.editor.file.path(), Some(path.as_path()));
        assert!(!f.dirty());
        assert_eq!(f.log.borrow().save_suggestions, ["Untitled 1.g4d"]);
        assert_eq!(grid4d_io::load(&path).unwrap().scene.len(), f.doc.scene.len());
        assert_eq!(f.editor.file.display_name(&f.doc), "shot.g4d");
    }

    #[test]
    fn save_asks_for_a_path_only_once() {
        let mut f = Fixture::new("save_twice", None, Some("shot.g4d"));
        f.run(&[EditorCommand::Save]);
        f.make_dirty();
        f.run(&[EditorCommand::Save]);
        assert_eq!(
            f.log.borrow().save_suggestions.len(),
            1,
            "the second save reuses the path"
        );
        assert!(!f.dirty());
        let saved = grid4d_io::load(&f.dir.join("shot.g4d")).unwrap();
        assert_eq!(
            saved.scene.len(),
            f.doc.scene.len(),
            "the second save wrote the new cube"
        );
    }

    #[test]
    fn a_cancelled_save_dialog_changes_nothing() {
        let mut f = Fixture::new("cancel_save", None, None);
        f.make_dirty();
        f.run(&[EditorCommand::SaveAs]);
        assert!(f.dirty());
        assert_eq!(f.editor.file.path(), None);
        assert!(f.editor.file.alert.is_none());
    }

    #[test]
    fn undoing_back_to_the_saved_state_is_clean_again() {
        let mut f = Fixture::new("undo_clean", None, Some("shot.g4d"));
        f.run(&[EditorCommand::Save]);
        f.make_dirty();
        f.run(&[EditorCommand::Undo]);
        assert!(!f.dirty());
        f.run(&[EditorCommand::Redo]);
        assert!(f.dirty());
    }

    #[test]
    fn open_asks_about_unsaved_changes_then_replaces_the_document() {
        let mut f = Fixture::new("open", Some("other.g4d"), None);
        let mut other = Document::new("Other");
        other.scene.insert(grid4d_scene::Node::new("Lonely", ObjectKind::Null));
        grid4d_io::save(&other, &f.dir.join("other.g4d")).unwrap();
        f.make_dirty();
        let before = f.doc.scene.len();

        f.run(&[EditorCommand::Open]);
        assert_eq!(f.editor.file.pending(), Some(&FileAction::Open));
        assert_eq!(
            f.log.borrow().open_calls,
            0,
            "no dialog before the question is answered"
        );

        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Cancel)]);
        assert_eq!(f.editor.file.pending(), None);
        assert_eq!(f.doc.scene.len(), before, "cancel keeps the document");
        assert!(f.dirty());

        f.run(&[
            EditorCommand::Open,
            EditorCommand::AnswerUnsaved(UnsavedChoice::Discard),
        ]);
        assert_eq!(f.log.borrow().open_calls, 1);
        assert_eq!(f.doc.name, "Other");
        assert_eq!(f.doc.scene.len(), 1);
        assert!(!f.dirty());
        assert!(!f.editor.history.can_undo(), "a loaded document starts a new history");
        assert!(f.editor.selection.is_empty());
        assert_eq!(f.editor.file.path(), Some(f.dir.join("other.g4d").as_path()));
    }

    #[test]
    fn saving_from_the_prompt_then_continues() {
        let mut f = Fixture::new("save_then_new", None, Some("kept.g4d"));
        f.make_dirty();
        let edited = f.doc.scene.len();
        f.run(&[
            EditorCommand::NewDocument,
            EditorCommand::AnswerUnsaved(UnsavedChoice::Save),
        ]);
        assert_eq!(grid4d_io::load(&f.dir.join("kept.g4d")).unwrap().scene.len(), edited);
        assert!(f.doc.scene.is_empty(), "then the new document");
        assert_eq!(f.editor.file.path(), None);
        assert!(!f.dirty());
    }

    #[test]
    fn a_failed_save_keeps_the_document_and_explains() {
        let mut f = Fixture::new("failed_save", None, Some("missing-dir/shot.g4d"));
        f.make_dirty();
        let edited = f.doc.scene.len();
        f.run(&[
            EditorCommand::NewDocument,
            EditorCommand::AnswerUnsaved(UnsavedChoice::Save),
        ]);
        assert_eq!(f.doc.scene.len(), edited, "New did not happen");
        assert!(f.dirty());
        assert_eq!(f.editor.file.pending(), None);
        let alert = f.editor.file.alert.as_ref().unwrap();
        assert_eq!(alert.title, "Could Not Save Document");
    }

    #[test]
    fn a_broken_file_shows_an_alert_and_keeps_the_document() {
        let mut f = Fixture::new("broken", Some("broken.g4d"), None);
        std::fs::write(f.dir.join("broken.g4d"), "{ not json").unwrap();
        let before = f.doc.scene.len();
        f.run(&[EditorCommand::Open]);
        assert_eq!(f.doc.scene.len(), before);
        let alert = f.editor.file.alert.as_ref().unwrap();
        assert_eq!(alert.title, "Could Not Open Document");
        assert!(alert.message.contains("broken.g4d"), "{}", alert.message);
    }

    #[test]
    fn quit_asks_only_when_there_are_unsaved_changes() {
        let mut f = Fixture::new("quit_clean", None, None);
        f.run(&[EditorCommand::Quit]);
        assert!(f.editor.file.is_quitting());

        let mut f = Fixture::new("quit_dirty", None, None);
        f.make_dirty();
        f.run(&[EditorCommand::Quit]);
        assert!(!f.editor.file.is_quitting());
        assert_eq!(f.editor.file.pending(), Some(&FileAction::Quit));
        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)]);
        assert!(f.editor.file.is_quitting());
    }

    /// Runs `sync_window` in a headless frame, optionally with the window's close
    /// button pressed, and returns the viewport commands it sent.
    fn sync_frame(f: &mut Fixture, close_button: bool) -> Vec<ViewportCommand> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        if close_button {
            input.viewports.insert(
                ViewportId::ROOT,
                ViewportInfo {
                    events: vec![ViewportEvent::Close],
                    ..ViewportInfo::default()
                },
            );
        }
        let (doc, editor) = (&f.doc, &mut f.editor);
        let output = ctx.run_ui(input, |ui| sync_window(ui.ctx(), doc, editor));
        let commands = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default();
        output.drop_without_applying_deltas();
        commands
    }

    #[test]
    fn the_close_button_asks_first_when_there_are_unsaved_changes() {
        let mut f = Fixture::new("close_button", None, None);
        let sent = sync_frame(&mut f, true);
        assert!(
            !sent.contains(&ViewportCommand::CancelClose),
            "a clean document just closes"
        );

        f.make_dirty();
        let sent = sync_frame(&mut f, true);
        assert!(sent.contains(&ViewportCommand::CancelClose));
        assert!(
            sent.iter()
                .any(|c| matches!(c, ViewportCommand::Title(t) if t.ends_with("* — Grid4D")))
        );
        f.run(&[]); // the queued Quit
        assert_eq!(f.editor.file.pending(), Some(&FileAction::Quit));

        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)]);
        let sent = sync_frame(&mut f, true);
        assert!(sent.contains(&ViewportCommand::Close));
        assert!(
            !sent.contains(&ViewportCommand::CancelClose),
            "a confirmed quit goes through"
        );
    }

    #[test]
    fn saving_and_opening_update_the_recent_list() {
        let mut f = Fixture::new("recent_touch", Some("a.g4d"), Some("b.g4d"));
        grid4d_io::save(&Document::new("A"), &f.dir.join("a.g4d")).unwrap();
        f.run(&[EditorCommand::SaveAs]); // b.g4d
        f.run(&[EditorCommand::Open]); // a.g4d (the document is clean, no prompt)
        let recent: Vec<_> = f.editor.file.recent.paths().to_vec();
        assert_eq!(recent, [f.dir.join("a.g4d"), f.dir.join("b.g4d")], "newest first");
    }

    #[test]
    fn open_recent_asks_first_and_opens_the_file() {
        let mut f = Fixture::new("recent_open", None, None);
        let path = f.dir.join("old.g4d");
        grid4d_io::save(&Document::new("Old shot"), &path).unwrap();
        f.make_dirty();

        f.run(&[EditorCommand::OpenFile(path.clone())]);
        assert_eq!(f.editor.file.pending(), Some(&FileAction::OpenPath(path.clone())));
        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)]);
        assert_eq!(f.doc.name, "Old shot");
        assert_eq!(f.editor.file.path(), Some(path.as_path()));
        assert_eq!(f.log.borrow().open_calls, 0, "no dialog for a recent file");
    }

    #[test]
    fn a_vanished_recent_file_is_dropped_from_the_list() {
        let mut f = Fixture::new("recent_gone", None, None);
        let gone = f.dir.join("gone.g4d");
        f.editor.file.recent.touch(&gone);
        f.run(&[EditorCommand::OpenFile(gone)]);
        assert!(f.editor.file.recent.is_empty());
        assert_eq!(f.editor.file.alert.as_ref().unwrap().title, "Could Not Open Document");
    }

    // ── Merge and glTF ──────────────────────────────────────────────────────

    /// `model.gltf` (+ `tri.bin`) in `dir`: Model > Tri, a one-triangle mesh.
    fn write_gltf(dir: &Path) -> PathBuf {
        let mut bin = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend(v.to_le_bytes());
        }
        std::fs::write(dir.join("tri.bin"), &bin).unwrap();
        let json = r#"{
            "asset": { "version": "2.0" },
            "scene": 0, "scenes": [ { "nodes": [0] } ],
            "nodes": [ { "name": "Model", "children": [1], "translation": [0, 1, 0] },
                       { "name": "Tri", "mesh": 0 } ],
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 } } ] } ],
            "buffers": [ { "byteLength": 36, "uri": "tri.bin" } ],
            "bufferViews": [ { "buffer": 0, "byteLength": 36 } ],
            "accessors": [ { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                             "min": [0, 0, 0], "max": [1, 1, 0] } ]
        }"#;
        let path = dir.join("model.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }

    fn names(doc: &Document) -> Vec<String> {
        Objects::from_document(doc)
            .nodes
            .into_iter()
            .map(|n| n.data.name)
            .collect()
    }

    #[test]
    fn merge_adds_a_gltf_model_as_one_undo_step_and_selects_it() {
        let mut f = Fixture::new("merge_gltf", Some("model.gltf"), None);
        let path = write_gltf(&f.dir);
        let before = names(&f.doc);
        let selection_before = f.editor.selection.clone();
        f.run(&[EditorCommand::Merge]);
        assert_eq!(f.log.borrow().open_calls, 1, "asked for the file");

        let after = names(&f.doc);
        assert_eq!(after.len(), before.len() + 2);
        assert_eq!(
            &after[before.len()..],
            ["Model", "Tri"],
            "added at the end, at the top level"
        );
        let model = f.doc.scene.iter().find(|(_, n)| n.data.name == "Model").unwrap().0;
        assert_eq!(f.editor.selection.as_slice(), &[model], "the merged object is selected");
        assert!(
            f.doc
                .scene
                .get(model)
                .unwrap()
                .data
                .transform
                .position
                .abs_diff_eq(glam::Vec3::new(0.0, 100.0, 0.0), 1e-4),
            "metres → centimetres"
        );
        assert_eq!(f.editor.history.undo_label(), Some("Merge"));
        assert!(f.dirty());
        assert_eq!(f.editor.file.path(), None, "merging keeps the document's own file");

        f.run(&[EditorCommand::Undo]);
        assert_eq!(names(&f.doc), before, "undo removes all of it");
        assert_eq!(f.editor.selection, selection_before);
        f.run(&[EditorCommand::Redo]);
        assert_eq!(names(&f.doc), after);
        assert!(f.doc.scene.contains(model), "the same objects come back");

        // A dropped glTF file merges straight away.
        f.run(&[EditorCommand::MergeFile(path)]);
        assert_eq!(names(&f.doc).len(), after.len() + 2);
    }

    #[test]
    fn merging_a_document_brings_its_materials_and_keeps_its_references() {
        use grid4d_scene::{Material, Tag, Tags};
        let mut f = Fixture::new("merge_materials", None, None);
        // The open document already has a material called "Red".
        let mine = f.doc.scene.materials_mut().insert(Material::new("Red"));
        let mut other = Document::with_demo_scene();
        let mut red = Material::new("Red");
        red.color = [1.0, 0.0, 0.0];
        let red = other.scene.materials_mut().insert(red);
        let find = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let (cube, boxed) = (find(&other, "Cube"), find(&other, "Box"));
        other.scene.get_mut(cube).unwrap().data.tags = vec![
            Tag::Material { material: Some(red) },
            Tag::Target {
                target: Some(boxed),
                up: None,
                pitch: true,
            },
        ];
        let path = f.dir.join("other.g4d");
        grid4d_io::save(&other, &path).unwrap();
        let before = f.doc.scene.len();
        f.run(&[EditorCommand::MergeFile(path)]);

        let materials = f.doc.scene.materials();
        assert_eq!(materials.len(), 2);
        let (merged, material) = materials.iter().find(|&(id, _)| id != mine).unwrap();
        assert_eq!(material.name, "Red.1", "a unique name");
        assert_eq!(material.color, [1.0, 0.0, 0.0]);
        // The merged objects are the selected roots; the merged Cube is one of them.
        let new_cube = *f
            .editor
            .selection
            .as_slice()
            .iter()
            .find(|&&id| f.doc.scene.get(id).unwrap().data.name == "Cube")
            .unwrap();
        let data = &f.doc.scene.get(new_cube).unwrap().data;
        assert_eq!(
            data.tags.material(),
            Some(merged),
            "its tag points at the merged material"
        );
        let target = data.tags.aim().map(|a| a.target).unwrap();
        assert_eq!(f.doc.scene.get(target).unwrap().data.name, "Box");
        assert_eq!(f.editor.history.undo_len(), 1, "one step");

        f.run(&[EditorCommand::Undo]);
        assert_eq!(f.doc.scene.materials().len(), 1, "undo takes the material back too");
        assert_eq!(f.doc.scene.len(), before);
        f.run(&[EditorCommand::Redo]);
        assert_eq!(f.doc.scene.materials().len(), 2);
    }

    #[test]
    fn a_document_merges_too_and_a_broken_file_changes_nothing() {
        let mut f = Fixture::new("merge_g4d", None, None);
        let other = f.dir.join("other.g4d");
        grid4d_io::save(&Document::with_demo_scene(), &other).unwrap();
        let count = names(&f.doc).len();
        f.run(&[EditorCommand::MergeFile(other)]);
        assert_eq!(names(&f.doc).len(), count * 2);
        assert_eq!(f.editor.history.undo_len(), 1);

        let broken = f.dir.join("broken.gltf");
        std::fs::write(&broken, "{ not json").unwrap();
        f.run(&[EditorCommand::MergeFile(broken)]);
        assert_eq!(names(&f.doc).len(), count * 2, "nothing added");
        assert_eq!(f.editor.history.undo_len(), 1, "nothing recorded");
        assert_eq!(f.editor.file.alert.as_ref().unwrap().title, "Could Not Merge File");
    }

    #[test]
    fn opening_a_gltf_file_makes_an_untitled_document_named_after_it() {
        let mut f = Fixture::new("open_gltf", None, Some("saved.g4d"));
        let path = write_gltf(&f.dir);
        f.run(&[EditorCommand::OpenFile(path.clone())]);
        assert_eq!(names(&f.doc), ["Model", "Tri"]);
        assert_eq!(f.doc.name, "model");
        assert_eq!(f.editor.file.path(), None, "not a .g4d: saving asks where");
        assert!(!f.dirty());
        assert!(f.editor.file.recent.paths().contains(&path));

        f.run(&[EditorCommand::Save]);
        assert_eq!(f.log.borrow().save_suggestions, ["model.g4d"]);
        assert_eq!(f.editor.file.path(), Some(f.dir.join("saved.g4d").as_path()));
    }

    // ── Export ──────────────────────────────────────────────────────────────

    /// Wraps the demo's Cube in a Subdivision Surface (the Cube becomes its cage).
    fn with_subdivision_surface(f: &mut Fixture) -> NodeId {
        let cube = f.doc.scene.iter().find(|(_, n)| n.data.name == "Cube").unwrap().0;
        f.editor.selection.set_single(cube);
        f.run(&[EditorCommand::WrapInGenerator(ObjectKindTag::SubdivisionSurface)]);
        f.editor.selection.active().unwrap()
    }

    #[test]
    fn exportable_leaves_a_sweeps_profile_and_path_out_in_its_own_colour() {
        let mut doc = Document::new("sweep");
        let mut sweep = grid4d_scene::Node::new("Sweep", ObjectKindTag::Sweep.default_kind());
        sweep.data.display_color = [0.1, 0.2, 0.3];
        let sweep = doc.scene.insert(sweep);
        for (name, tag) in [("Circle", ObjectKindTag::Circle), ("Helix", ObjectKindTag::Helix)] {
            doc.scene
                .insert_child(sweep, grid4d_scene::Node::new(name, tag.default_kind()))
                .unwrap();
        }
        let extra = doc
            .scene
            .insert_child(
                sweep,
                grid4d_scene::Node::new("Cube", ObjectKindTag::Cube.default_kind()),
            )
            .unwrap();
        let objects = exportable(&doc);
        let names: Vec<&str> = objects.nodes.iter().map(|n| n.data.name.as_str()).collect();
        assert_eq!(
            names,
            ["Sweep", "Cube"],
            "profile and path baked in; other children kept"
        );
        assert_eq!(objects.nodes[0].data.display_color, [0.1, 0.2, 0.3], "its own colour");
        assert_eq!(objects.nodes[1].parent, Some(0));
        assert!(doc.scene.contains(extra));
    }

    #[test]
    fn exportable_bakes_primitives_and_generators_and_leaves_cages_out() {
        let mut f = Fixture::new("exportable", None, None);
        let sds = with_subdivision_surface(&mut f);
        let objects = exportable(&f.doc);
        let entry = |name: &str| objects.nodes.iter().find(|n| n.data.name == name);
        assert!(entry("Cube").is_none(), "the cage goes into the generator's mesh");
        let smoothed = grid4d_eval::generator_mesh(&f.doc.scene, sds, 0.0).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &entry("Subdivision Surface").unwrap().data.kind else {
            panic!("the generator is written as its result")
        };
        assert_eq!(mesh.mesh(), &smoothed);
        let cage_colour = f
            .doc
            .scene
            .get(f.doc.scene.children(sds)[0])
            .unwrap()
            .data
            .display_color;
        assert_eq!(
            entry("Subdivision Surface").unwrap().data.display_color,
            cage_colour,
            "in the colour the viewport shows it in"
        );
        for name in ["Sphere", "Cylinder", "Box"] {
            assert_eq!(
                entry(name).unwrap().data.kind.tag(),
                ObjectKindTag::Polygon,
                "{name} baked"
            );
        }
        assert_eq!(entry("Group").unwrap().data.kind, ObjectKind::Null);
        assert_eq!(entry("Light").unwrap().data.kind.tag(), ObjectKindTag::Light);
        assert_eq!(objects.nodes.len(), f.doc.scene.len() - 1);

        // Switched off: an empty node over its cage, which is written as it is.
        f.doc.scene.get_mut(sds).unwrap().data.enabled = false;
        let objects = exportable(&f.doc);
        let generator = objects
            .nodes
            .iter()
            .position(|n| n.data.name == "Subdivision Surface")
            .unwrap();
        assert_eq!(objects.nodes[generator].data.kind, ObjectKind::Null);
        let cube = objects.nodes.iter().find(|n| n.data.name == "Cube").unwrap();
        assert_eq!(
            (cube.parent, cube.data.kind.tag()),
            (Some(generator), ObjectKindTag::Polygon)
        );
    }

    #[test]
    fn export_writes_glb_or_gltf_and_leaves_the_document_alone() {
        let mut f = Fixture::new("export", None, Some("scene.glb"));
        with_subdivision_surface(&mut f);
        let (steps, dirty) = (f.editor.history.undo_len(), f.dirty());
        f.run(&[EditorCommand::Export]);
        assert_eq!(f.log.borrow().save_suggestions, ["Untitled 1.glb"]);
        assert_eq!(
            (f.editor.history.undo_len(), f.dirty()),
            (steps, dirty),
            "nothing recorded"
        );
        assert_eq!(f.editor.file.path(), None, "the document keeps its own file");
        assert!(f.editor.file.alert.is_none());

        let back = grid4d_io::import(&f.dir.join("scene.glb")).unwrap();
        let expected: Vec<String> = exportable(&f.doc).nodes.into_iter().map(|n| n.data.name).collect();
        let got: Vec<String> = back.nodes.iter().map(|n| n.data.name.clone()).collect();
        assert_eq!(got, expected);

        // The .gltf flavour writes its .bin next to it.
        let mut f = Fixture::new("export_gltf", None, Some("scene.gltf"));
        f.run(&[EditorCommand::Export]);
        assert!(f.dir.join("scene.gltf").is_file() && f.dir.join("scene.bin").is_file());
        assert_eq!(
            grid4d_io::import(&f.dir.join("scene.gltf")).unwrap().nodes.len(),
            f.doc.scene.len()
        );

        // Cancelled: nothing written, nothing said.
        let mut f = Fixture::new("export_cancel", None, None);
        f.run(&[EditorCommand::Export]);
        assert!(f.editor.file.alert.is_none());
        assert_eq!(std::fs::read_dir(&f.dir).unwrap().count(), 0);

        // An unwritable place: an alert.
        let mut f = Fixture::new("export_fail", None, Some("missing/folder/scene.glb"));
        f.run(&[EditorCommand::Export]);
        assert_eq!(f.editor.file.alert.as_ref().unwrap().title, "Could Not Export");
    }

    #[test]
    fn obj_export_writes_the_meshes_and_opens_again() {
        let mut f = Fixture::new("export_obj", None, Some("scene.obj"));
        f.run(&[EditorCommand::Export]);
        assert!(f.dir.join("scene.obj").is_file() && f.dir.join("scene.mtl").is_file());
        assert!(f.editor.file.alert.is_none());

        // OBJ holds meshes only: every primitive, baked; no Nulls, cameras or lights.
        let meshes: Vec<String> = exportable(&f.doc)
            .nodes
            .into_iter()
            .filter(|n| matches!(&n.data.kind, ObjectKind::Polygon { mesh, .. } if mesh.mesh().face_count() > 0))
            .map(|n| n.data.name)
            .collect();
        f.run(&[EditorCommand::OpenFile(f.dir.join("scene.obj"))]);
        assert!(f.editor.file.alert.is_none(), "{:?}", f.editor.file.alert);
        assert_eq!(names(&f.doc), meshes);
        assert_eq!(f.doc.name, "scene");
        assert!(
            f.doc
                .scene
                .iter()
                .all(|(_, n)| n.data.kind.tag() == ObjectKindTag::Polygon && n.parent().is_none())
        );
    }

    // ── Drag and drop ───────────────────────────────────────────────────────

    #[test]
    fn the_first_document_in_a_drop_opens_else_a_gltf_file_merges() {
        assert_eq!(choose_dropped(&[]), None);
        let picked = choose_dropped(&[
            Path::new("notes.txt"),
            Path::new("duck.glb"),
            Path::new("a/shot.OC4D"),
            Path::new("b.g4d"),
        ]);
        assert_eq!(picked, Some(Ok(DropAction::Open(PathBuf::from("a/shot.OC4D")))));
        let picked = choose_dropped(&[Path::new("notes.txt"), Path::new("model.GLTF")]);
        assert_eq!(picked, Some(Ok(DropAction::Merge(PathBuf::from("model.GLTF")))));

        assert_eq!(
            choose_dropped(&[Path::new("notes.txt"), Path::new("model.obj")]),
            Some(Ok(DropAction::Merge(PathBuf::from("model.obj"))))
        );
        let Some(Err(alert)) = choose_dropped(&[Path::new("a/notes.txt"), Path::new("model.fbx")]) else {
            panic!("nothing usable was dropped");
        };
        assert_eq!(alert.title, "Cannot Open Dropped Files");
        assert!(alert.message.contains("notes.txt") && alert.message.contains("model.fbx"));
    }

    /// A file dropped onto the window, as the windowing integration reports it.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|err| err.to_string())
        }
    }

    #[test]
    fn dropping_a_document_on_the_window_opens_it_after_the_prompt() {
        let mut f = Fixture::new("drop", None, None);
        let path = f.dir.join("dropped.g4d");
        grid4d_io::save(&Document::new("Dropped"), &path).unwrap();
        f.make_dirty();

        let ctx = egui::Context::default();
        let input = RawInput {
            dropped_files: vec![std::sync::Arc::new(Dropped(path.clone()))],
            ..RawInput::default()
        };
        let editor = &mut f.editor;
        ctx.run_ui(input, |ui| accept_dropped_files(ui.ctx(), editor))
            .drop_without_applying_deltas();

        f.run(&[]); // the queued OpenFile
        assert_eq!(f.editor.file.pending(), Some(&FileAction::OpenPath(path.clone())));
        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)]);
        assert_eq!(f.doc.name, "Dropped");
        assert_eq!(f.editor.file.path(), Some(path.as_path()));
        assert_eq!(f.editor.file.recent.paths().first(), Some(&path));
    }

    #[test]
    fn export_writes_deformed_shapes() {
        use grid4d_geom::deform::Bend;
        let mut doc = Document::new("t");
        let cube = doc
            .scene
            .insert(grid4d_scene::Node::new("Cube", ObjectKindTag::Cube.default_kind()));
        let bend = Bend {
            size: glam::Vec3::splat(200.0),
            strength: 1.5,
            ..Bend::default()
        };
        doc.scene
            .insert_child(cube, grid4d_scene::Node::new("Bend", ObjectKind::Bend(bend)))
            .unwrap();
        let objects = exportable(&doc);
        let entry = objects.nodes.iter().find(|e| e.data.name == "Cube").unwrap();
        let ObjectKind::Polygon { mesh, .. } = &entry.data.kind else {
            panic!("not baked")
        };
        let plain = grid4d_eval::editable_mesh(&ObjectKindTag::Cube.default_kind()).unwrap();
        assert_ne!(mesh.mesh().points(), plain.points(), "the bent shape is written");
    }
}
