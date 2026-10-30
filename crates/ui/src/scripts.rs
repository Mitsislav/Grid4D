//! The user's script library and the Script Manager's state (see
//! `docs/reference/extend-scripting.md`): `.rhai` files in a folder of the
//! user data folder, made with example scripts the first time; the script
//! being edited; the console. Only this module touches the library's files.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::i18n::key;

/// Script files' extension.
pub const EXTENSION: &str = "rhai";

/// The example scripts written into a new library.
const EXAMPLES: &[(&str, &str)] = &[
    ("Grid of Cubes", include_str!("../scripts/Grid of Cubes.rhai")),
    (
        "Number Selected Objects",
        include_str!("../scripts/Number Selected Objects.rhai"),
    ),
    ("Random Colours", include_str!("../scripts/Random Colours.rhai")),
    (
        "Scatter on the Last Selected",
        include_str!("../scripts/Scatter on the Last Selected.rhai"),
    ),
    (
        "Spread Selected Objects",
        include_str!("../scripts/Spread Selected Objects.rhai"),
    ),
];

/// What a new script starts with.
const NEW_SCRIPT: &str = "// A new script. Ctrl+Enter runs it; the manual's Scripts chapter lists\n\
// what it can use.\nprint(`${objects().len()} objects`);\n";

/// A line of the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub text: String,
    /// An error (shown in the warning colour).
    pub error: bool,
}

/// The script library, the Script Manager's script and its console.
#[derive(Debug)]
pub struct Scripts {
    /// The library's folder; `None` lists nothing (tests, no data folder).
    folder: Option<PathBuf>,
    /// The library's scripts by name (file names without `.rhai`), sorted.
    names: Vec<String>,
    /// The Script Manager window is open.
    pub show: bool,
    /// The edited script's name (empty: not saved yet).
    pub name: String,
    pub text: String,
    /// The text as last saved or opened (unsaved changes: it differs).
    saved: String,
    console: Vec<ConsoleLine>,
}

impl Default for Scripts {
    fn default() -> Self {
        Self {
            folder: None,
            names: Vec::new(),
            show: false,
            name: String::new(),
            text: NEW_SCRIPT.to_owned(),
            saved: NEW_SCRIPT.to_owned(),
            console: Vec::new(),
        }
    }
}

/// The console keeps this many lines.
const CONSOLE_LINES: usize = 1000;

impl Scripts {
    /// The library in `folder`; made, with the example scripts, if it does
    /// not exist yet.
    #[must_use]
    pub fn new(folder: PathBuf) -> Self {
        if !folder.exists() {
            match fs::create_dir_all(&folder) {
                Ok(()) => {
                    for (name, text) in EXAMPLES {
                        let path = folder.join(format!("{name}.{EXTENSION}"));
                        if let Err(err) = fs::write(&path, text) {
                            log::warn!("Scripts: could not write {}: {err}", path.display());
                        }
                    }
                }
                Err(err) => log::warn!("Scripts: no library folder {}: {err}", folder.display()),
            }
        }
        let mut scripts = Self {
            folder: Some(folder),
            ..Self::default()
        };
        scripts.refresh();
        scripts
    }

    /// The library's folder, if it has one.
    #[must_use]
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The library's scripts by name, sorted.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Lists the library's folder again.
    pub fn refresh(&mut self) {
        self.names.clear();
        let Some(entries) = self.folder.as_ref().and_then(|f| fs::read_dir(f).ok()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
                && path.is_file()
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                self.names.push(stem.to_owned());
            }
        }
        self.names.sort_by_key(|n| n.to_lowercase());
    }

    /// The path of the library script `name`.
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        let folder = self
            .folder
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no script library"))?;
        if !valid_name(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a valid script name"));
        }
        Ok(folder.join(format!("{name}.{EXTENSION}")))
    }

    /// The text of the library script `name`.
    pub(crate) fn read(&self, name: &str) -> io::Result<String> {
        fs::read_to_string(self.path(name)?)
    }

    /// Opens the library script `name` in the Script Manager.
    pub(crate) fn open(&mut self, name: &str) -> io::Result<()> {
        let text = self.read(name)?;
        self.name = name.to_owned();
        self.text.clone_from(&text);
        self.saved = text;
        Ok(())
    }

    /// Starts a new, unnamed script.
    pub(crate) fn new_script(&mut self) {
        self.name.clear();
        self.text = NEW_SCRIPT.to_owned();
        self.saved = NEW_SCRIPT.to_owned();
    }

    /// Saves the edited script into the library under its name.
    pub(crate) fn save(&mut self) -> io::Result<()> {
        let name = self.name.trim().to_owned();
        fs::write(self.path(&name)?, &self.text)?;
        self.name = name;
        self.saved.clone_from(&self.text);
        self.refresh();
        Ok(())
    }

    /// The edited script has changes that are not saved.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    /// The console's lines, oldest first.
    #[must_use]
    pub fn console(&self) -> &[ConsoleLine] {
        &self.console
    }

    /// Adds lines to the console.
    pub fn log(&mut self, lines: impl IntoIterator<Item = String>, error: bool) {
        self.console
            .extend(lines.into_iter().map(|text| ConsoleLine { text, error }));
        let excess = self.console.len().saturating_sub(CONSOLE_LINES);
        self.console.drain(..excess);
    }

    pub fn clear_console(&mut self) {
        self.console.clear();
    }
}

/// `true` if `name` can be a library script's file name: not empty, no path
/// separators or characters file systems refuse.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

/// Why a library script could not be read or saved (a message's title).
pub(crate) const COULD_NOT_SAVE: &str = key("Could Not Save Script");

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test (not made yet).
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grid4d-scripts-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_library_gets_the_examples_once() {
        let dir = folder("examples");
        let scripts = Scripts::new(dir.clone());
        let wanted: Vec<&str> = EXAMPLES.iter().map(|(n, _)| *n).collect();
        assert_eq!(scripts.names(), wanted, "sorted by name");
        // Deleted by the user: not put back.
        fs::remove_file(dir.join("Grid of Cubes.rhai")).unwrap();
        fs::write(dir.join("notes.txt"), "not a script").unwrap();
        let again = Scripts::new(dir.clone());
        assert_eq!(again.names().len(), EXAMPLES.len() - 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scripts_are_saved_opened_and_named_safely() {
        let dir = folder("save");
        let mut scripts = Scripts::new(dir.clone());
        assert!(!scripts.is_dirty());
        scripts.text = "print(1);".to_owned();
        assert!(scripts.is_dirty());
        scripts.name = "../outside".to_owned();
        assert!(scripts.save().is_err(), "no paths");
        scripts.name = "  Mine ".to_owned();
        scripts.save().unwrap();
        assert_eq!(scripts.name, "Mine");
        assert!(!scripts.is_dirty());
        assert!(scripts.names().contains(&"Mine".to_owned()));
        scripts.new_script();
        assert!(scripts.name.is_empty());
        scripts.open("Mine").unwrap();
        assert_eq!((scripts.name.as_str(), scripts.text.as_str()), ("Mine", "print(1);"));
        assert!(scripts.open("Missing").is_err());
        assert!(!valid_name("") && !valid_name("a:b") && valid_name("Grid 2"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_console_keeps_the_newest_lines() {
        let mut scripts = Scripts::default();
        scripts.log((0..CONSOLE_LINES + 5).map(|i| i.to_string()), false);
        assert_eq!(scripts.console().len(), CONSOLE_LINES);
        assert_eq!(scripts.console()[0].text, "5");
        scripts.log(["oops".to_owned()], true);
        assert!(scripts.console().last().is_some_and(|l| l.error));
        scripts.clear_console();
        assert!(scripts.console().is_empty());
    }

    #[test]
    fn every_example_runs_on_the_demo_scene() {
        crate::test_plugins();
        let doc = grid4d_scene::Document::with_demo_scene();
        let id = |name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        // The Sphere and the Cube selected: the Cube last (the surface).
        let selection = [id("Sphere"), id("Cube")];
        for (name, text) in EXAMPLES {
            let outcome = grid4d_script::run(&doc, &selection, 0.0, crate::commands::shape_of, text);
            let outcome = outcome.unwrap_or_else(|err| panic!("{name}: {err}"));
            assert!(!outcome.inverses.is_empty(), "{name} changes something");
            assert_eq!(outcome.output.len(), 1, "{name} says what it did");
        }
    }
}
