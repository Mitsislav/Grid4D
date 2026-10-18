//! # grid4d-io
//!
//! Reading and writing Grid4D documents (`.g4d` files; `.oc4d` files from before
//! the project was renamed from OpenC4D are the same format and open too).
//!
//! ## Architecture rule
//!
//! The file format is a set of versioned DTOs, separate from the in-memory
//! types of `grid4d-scene`, so the scene can change without breaking files. Each
//! format version has its own module; older versions are migrated on load.
//! This crate never depends on `wgpu`, `egui` or `grid4d-ui`.
//!
//! ## Format, version 2 (current)
//!
//! UTF-8 JSON:
//!
//! ```text
//! { "format": "oc4d", "version": 2,
//!   "document": { "name": …, "time": { "fps", "start", "end", "looping" },
//!                 "objects": [ { "name", "parent"?, "kind": { "type": "cube", … },
//!                                "position", "rotation_hpb", "scale", "display_color",
//!                                "visible_in_editor", "visible_in_render", "enabled" } ] } }
//! ```
//!
//! * `objects` is flat, in Object Manager pre-order; `parent` is the index of an
//!   *earlier* object (absent for top-level objects). Hierarchies of any depth load
//!   without recursion, and cycles cannot be expressed.
//! * Units are centimetres, rotations are HPB Euler angles in **radians**, colours
//!   are linear RGB — the scene's own conventions, so a round trip is exact.
//! * Primitive kinds carry their segment counts (`"segments"`, `"rotation_segments"`, …).
//! * Detached (deleted but undoable) nodes and editor state are never saved.
//!
//! Version 1 (no segment counts; no cone, torus, disc or capsule) is still read and
//! migrated on load; files are always written in the current version.

mod common;
mod v1;
mod v2;

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use grid4d_scene::Document;
use serde::Deserialize;

/// File extension, without the dot.
pub const EXTENSION: &str = "g4d";
/// Every extension that opens as a document: [`EXTENSION`], and `oc4d` from before
/// the project was renamed from OpenC4D (same format). Files are saved as `EXTENSION`.
pub const OPEN_EXTENSIONS: [&str; 2] = [EXTENSION, "oc4d"];
/// The format version this build writes (and the newest it reads).
pub const FORMAT_VERSION: u32 = 2;
/// The `"format"` tag inside every file. It keeps the project's former name because
/// released format versions are frozen; only a new version may change it.
const FORMAT_TAG: &str = "oc4d";

/// Why a document could not be saved or loaded.
#[derive(Debug)]
pub enum IoError {
    Io(io::Error),
    /// Not valid JSON, or JSON that does not match the format.
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    /// Valid JSON, but not a Grid4D document.
    NotADocument,
    /// Written by a newer Grid4D (or an unknown version).
    UnsupportedVersion {
        found: u32,
        newest: u32,
    },
    /// Well-formed, but inconsistent or unrepresentable content.
    Invalid(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Parse { line, column, message } => write!(f, "line {line}, column {column}: {message}"),
            Self::NotADocument => f.write_str("not a Grid4D document"),
            Self::UnsupportedVersion { found, newest } => write!(
                f,
                "file format version {found} is not supported (this Grid4D reads up to version {newest})"
            ),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Parse { .. } | Self::NotADocument | Self::UnsupportedVersion { .. } | Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for IoError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for IoError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse {
            line: err.line(),
            column: err.column(),
            message: err.to_string(),
        }
    }
}

/// Serialises `doc` in the current format (pretty-printed, trailing newline).
pub fn to_string(doc: &Document) -> Result<String, IoError> {
    let mut text = serde_json::to_string_pretty(&v2::File::from_document(doc)?)?;
    text.push('\n');
    Ok(text)
}

/// Parses a document written by this or an older format version.
pub fn from_str(text: &str) -> Result<Document, IoError> {
    /// Just enough to dispatch on the version before parsing the rest.
    #[derive(Deserialize)]
    struct Header {
        format: Option<String>,
        version: Option<u32>,
    }

    let header: Header = serde_json::from_str(text)?;
    let (Some(FORMAT_TAG), Some(version)) = (header.format.as_deref(), header.version) else {
        return Err(IoError::NotADocument);
    };
    match version {
        1 => v1::File::parse(text)?.into_document(),
        2 => v2::File::parse(text)?.into_document(),
        found => Err(IoError::UnsupportedVersion {
            found,
            newest: FORMAT_VERSION,
        }),
    }
}

/// Saves `doc` to `path`.
///
/// Writes a temporary file next to `path` and renames it over the target, so a
/// crash or a full disk never leaves a half-written document behind.
pub fn save(doc: &Document, path: &Path) -> Result<(), IoError> {
    let text = to_string(doc)?;
    let temp = temp_path(path);
    if let Err(err) = fs::write(&temp, text).and_then(|()| fs::rename(&temp, path)) {
        // Best effort: the temporary file may not even exist.
        let _ = fs::remove_file(&temp);
        return Err(err.into());
    }
    Ok(())
}

/// Loads the document at `path`.
pub fn load(path: &Path) -> Result<Document, IoError> {
    from_str(&fs::read_to_string(path)?)
}

/// `scene.g4d` → `scene.g4d.tmp`, in the same directory (so the rename is atomic).
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}
