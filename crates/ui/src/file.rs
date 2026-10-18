//! The document's file on disk: where it lives, whether it has unsaved changes,
//! the "Save changes?" flow, native file dialogs and the window title.
//!
//! `commands::apply` calls the operations here; panels only queue commands and
//! show [`DocumentFile`] state. File I/O never happens inside a panel.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use egui::ViewportCommand;
use oc4d_scene::Document;

use crate::{EditorCommand, EditorState, History, ViewportPanel, recent::RecentFiles};

/// Native open/save dialogs, behind a trait so tests never open a window.
pub trait FileDialogs: std::fmt::Debug {
    /// A document to open, or `None` if the user cancelled.
    fn pick_open(&mut self, directory: Option<&Path>) -> Option<PathBuf>;
    /// Where to save, or `None` if the user cancelled. `suggested` is a bare
    /// file name such as `Untitled 1.oc4d`.
    fn pick_save(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf>;
}

/// The platform dialogs (Win32, Cocoa, XDG desktop portal), via `rfd`.
#[derive(Debug, Default)]
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn pick_open(&mut self, directory: Option<&Path>) -> Option<PathBuf> {
        with_directory(rfd::FileDialog::new(), directory)
            .set_title("Open")
            .add_filter("OpenC4D Document", &[oc4d_io::EXTENSION])
            .pick_file()
    }

    fn pick_save(&mut self, directory: Option<&Path>, suggested: &str) -> Option<PathBuf> {
        with_directory(rfd::FileDialog::new(), directory)
            .set_title("Save As")
            .add_filter("OpenC4D Document", &[oc4d_io::EXTENSION])
            .set_file_name(suggested)
            .save_file()
            .map(with_extension)
    }
}

fn with_directory(dialog: rfd::FileDialog, directory: Option<&Path>) -> rfd::FileDialog {
    match directory {
        Some(dir) => dialog.set_directory(dir),
        None => dialog,
    }
}

/// Some dialogs return the typed name without the extension: append it (rather
/// than replacing, so `shot.v2` becomes `shot.v2.oc4d`).
fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|ext| ext == oc4d_io::EXTENSION) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(oc4d_io::EXTENSION);
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

    /// The new window title if it changed since the last call ("shot.oc4d * — OpenC4D").
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
        Some(format!(
            "{}{} — OpenC4D",
            self.title_name,
            if dirty { " *" } else { "" }
        ))
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

/// Loads `path` and makes it the current document. On error the current
/// document stays and an alert explains why.
pub(crate) fn open_path(path: PathBuf, doc: &mut Document, editor: &mut EditorState, viewport: &mut ViewportPanel) {
    match oc4d_io::load(&path) {
        Ok(loaded) => {
            editor.file.recent.touch(&path);
            replace(doc, editor, viewport, loaded, Some(path));
            viewport.frame_scene(&doc.scene);
        }
        Err(err) => {
            // A recent entry whose file is gone is of no further use.
            if matches!(&err, oc4d_io::IoError::Io(io) if io.kind() == std::io::ErrorKind::NotFound) {
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

/// Saves to the current file, or asks where (always with `ask_path`, i.e. Save As).
/// Returns `true` if the document was written.
pub(crate) fn save(doc: &Document, editor: &mut EditorState, ask_path: bool) -> bool {
    let path = match (&editor.file.path, ask_path) {
        (Some(path), false) => path.clone(),
        _ => {
            let suggested = format!(
                "{}.{}",
                file_stem_for(&editor.file.display_name(doc)),
                oc4d_io::EXTENSION
            );
            let directory = editor.file.directory().map(Path::to_path_buf);
            let Some(path) = editor.file.dialogs.pick_save(directory.as_deref(), &suggested) else {
                return false;
            };
            path
        }
    };
    match oc4d_io::save(doc, &path) {
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

/// A name usable as a file stem: drops a trailing `.oc4d` and replaces
/// characters that are invalid in file names on some platform.
fn file_stem_for(name: &str) -> String {
    let stem = name
        .strip_suffix(oc4d_io::EXTENSION)
        .and_then(|s| s.strip_suffix('.'))
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
        assert_eq!(with_extension(PathBuf::from("a/shot")), PathBuf::from("a/shot.oc4d"));
        assert_eq!(
            with_extension(PathBuf::from("a/shot.oc4d")),
            PathBuf::from("a/shot.oc4d")
        );
        assert_eq!(with_extension(PathBuf::from("shot.v2")), PathBuf::from("shot.v2.oc4d"));
    }

    #[test]
    fn suggested_names_are_valid_file_names() {
        assert_eq!(file_stem_for("Untitled 1"), "Untitled 1");
        assert_eq!(file_stem_for("shot.oc4d"), "shot");
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
            Some("Untitled 1 — OpenC4D")
        );
        assert_eq!(file.title_update(&doc, &history), None);

        let mut scratch = Document::new("x");
        history.record(
            &mut scratch,
            "Edit",
            vec![oc4d_scene::SceneEdit::SetName("x".to_owned())],
            crate::Selection::default(),
            crate::Selection::default(),
        );
        assert_eq!(
            file.title_update(&doc, &history).as_deref(),
            Some("Untitled 1 * — OpenC4D")
        );
        file.path = Some(PathBuf::from("renders/shot.oc4d"));
        assert_eq!(
            file.title_update(&doc, &history).as_deref(),
            Some("shot.oc4d * — OpenC4D")
        );
    }

    // ── The New / Open / Save / Quit workflow ───────────────────────────────

    use std::{cell::RefCell, rc::Rc};

    use egui::{RawInput, ViewportEvent, ViewportId, ViewportInfo};
    use oc4d_scene::ObjectKindTag;

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
            let dir = std::env::temp_dir().join(format!("oc4d-ui-{}-{test}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let log = Rc::new(RefCell::new(DialogLog::default()));
            let dialogs = FakeDialogs {
                open: open.map(|name| dir.join(name)),
                save: save.map(|name| dir.join(name)),
                log: Rc::clone(&log),
            };
            let editor = EditorState {
                file: DocumentFile::with_dialogs(Box::new(dialogs)),
                ..EditorState::default()
            };
            Self {
                doc: Document::with_demo_scene(),
                editor,
                viewport: ViewportPanel::new(None, None, 1),
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
    fn save_as_writes_the_file_and_marks_the_document_clean() {
        let mut f = Fixture::new("save_as", None, Some("shot.oc4d"));
        f.make_dirty();
        f.run(&[EditorCommand::SaveAs]);

        let path = f.dir.join("shot.oc4d");
        assert_eq!(f.editor.file.path(), Some(path.as_path()));
        assert!(!f.dirty());
        assert_eq!(f.log.borrow().save_suggestions, ["Untitled 1.oc4d"]);
        assert_eq!(oc4d_io::load(&path).unwrap().scene.len(), f.doc.scene.len());
        assert_eq!(f.editor.file.display_name(&f.doc), "shot.oc4d");
    }

    #[test]
    fn save_asks_for_a_path_only_once() {
        let mut f = Fixture::new("save_twice", None, Some("shot.oc4d"));
        f.run(&[EditorCommand::Save]);
        f.make_dirty();
        f.run(&[EditorCommand::Save]);
        assert_eq!(
            f.log.borrow().save_suggestions.len(),
            1,
            "the second save reuses the path"
        );
        assert!(!f.dirty());
        let saved = oc4d_io::load(&f.dir.join("shot.oc4d")).unwrap();
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
        let mut f = Fixture::new("undo_clean", None, Some("shot.oc4d"));
        f.run(&[EditorCommand::Save]);
        f.make_dirty();
        f.run(&[EditorCommand::Undo]);
        assert!(!f.dirty());
        f.run(&[EditorCommand::Redo]);
        assert!(f.dirty());
    }

    #[test]
    fn open_asks_about_unsaved_changes_then_replaces_the_document() {
        let mut f = Fixture::new("open", Some("other.oc4d"), None);
        let mut other = Document::new("Other");
        other
            .scene
            .insert(oc4d_scene::Node::new("Lonely", oc4d_scene::ObjectKind::Null));
        oc4d_io::save(&other, &f.dir.join("other.oc4d")).unwrap();
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
        assert_eq!(f.editor.file.path(), Some(f.dir.join("other.oc4d").as_path()));
    }

    #[test]
    fn saving_from_the_prompt_then_continues() {
        let mut f = Fixture::new("save_then_new", None, Some("kept.oc4d"));
        f.make_dirty();
        let edited = f.doc.scene.len();
        f.run(&[
            EditorCommand::NewDocument,
            EditorCommand::AnswerUnsaved(UnsavedChoice::Save),
        ]);
        assert_eq!(oc4d_io::load(&f.dir.join("kept.oc4d")).unwrap().scene.len(), edited);
        assert!(f.doc.scene.is_empty(), "then the new document");
        assert_eq!(f.editor.file.path(), None);
        assert!(!f.dirty());
    }

    #[test]
    fn a_failed_save_keeps_the_document_and_explains() {
        let mut f = Fixture::new("failed_save", None, Some("missing-dir/shot.oc4d"));
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
        let mut f = Fixture::new("broken", Some("broken.oc4d"), None);
        std::fs::write(f.dir.join("broken.oc4d"), "{ not json").unwrap();
        let before = f.doc.scene.len();
        f.run(&[EditorCommand::Open]);
        assert_eq!(f.doc.scene.len(), before);
        let alert = f.editor.file.alert.as_ref().unwrap();
        assert_eq!(alert.title, "Could Not Open Document");
        assert!(alert.message.contains("broken.oc4d"), "{}", alert.message);
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
                .any(|c| matches!(c, ViewportCommand::Title(t) if t.ends_with("* — OpenC4D")))
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
        let mut f = Fixture::new("recent_touch", Some("a.oc4d"), Some("b.oc4d"));
        oc4d_io::save(&Document::new("A"), &f.dir.join("a.oc4d")).unwrap();
        f.run(&[EditorCommand::SaveAs]); // b.oc4d
        f.run(&[EditorCommand::Open]); // a.oc4d (the document is clean, no prompt)
        let recent: Vec<_> = f.editor.file.recent.paths().to_vec();
        assert_eq!(recent, [f.dir.join("a.oc4d"), f.dir.join("b.oc4d")], "newest first");
    }

    #[test]
    fn open_recent_asks_first_and_opens_the_file() {
        let mut f = Fixture::new("recent_open", None, None);
        let path = f.dir.join("old.oc4d");
        oc4d_io::save(&Document::new("Old shot"), &path).unwrap();
        f.make_dirty();

        f.run(&[EditorCommand::OpenRecent(path.clone())]);
        assert_eq!(f.editor.file.pending(), Some(&FileAction::OpenPath(path.clone())));
        f.run(&[EditorCommand::AnswerUnsaved(UnsavedChoice::Discard)]);
        assert_eq!(f.doc.name, "Old shot");
        assert_eq!(f.editor.file.path(), Some(path.as_path()));
        assert_eq!(f.log.borrow().open_calls, 0, "no dialog for a recent file");
    }

    #[test]
    fn a_vanished_recent_file_is_dropped_from_the_list() {
        let mut f = Fixture::new("recent_gone", None, None);
        let gone = f.dir.join("gone.oc4d");
        f.editor.file.recent.touch(&gone);
        f.run(&[EditorCommand::OpenRecent(gone)]);
        assert!(f.editor.file.recent.is_empty());
        assert_eq!(f.editor.file.alert.as_ref().unwrap().title, "Could Not Open Document");
    }
}
