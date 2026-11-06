//! # grid4d-io
//!
//! Reading and writing Grid4D documents (`.g4d` files; `.oc4d` files from before
//! the project was renamed from OpenC4D open too).
//!
//! ## Architecture rule
//!
//! The file format is a set of versioned DTOs, separate from the in-memory
//! types of `grid4d-scene`, so the scene can change without breaking files. Each
//! format version has its own module; older versions are migrated on load.
//! This crate never depends on `wgpu`, `egui` or `grid4d-ui`.
//!
//! ## Format, version 4 (current)
//!
//! A zip archive (deflate) with fixed timestamps, so the same document always
//! gives the same bytes:
//!
//! * `document.json` — UTF-8 JSON:
//!
//!   ```text
//!   { "format": "grid4d", "version": 4,
//!     "document": { "name": …, "time": { "fps", "start", "end", "looping" },
//!                   "objects": [ { "name", "parent"?, "kind": { "type": "cube", … },
//!                                  "position", "rotation_hpb", "scale", "display_color",
//!                                  "visible_in_editor", "visible_in_render", "enabled" } ] },
//!     "meshes": [ { "points", "faces", "corners", "uvs" } ] }
//!   ```
//!
//! * `meshes/<index>.bin` — each polygon mesh's arrays, little-endian (see `v4.rs`);
//!   a polygon object's kind is `{ "type": "polygon", "mesh": <index> }`, and objects
//!   sharing a mesh share the file.
//!
//! * `objects` is flat, in Object Manager pre-order; `parent` is the index of an
//!   *earlier* object (absent for top-level objects). Hierarchies of any depth load
//!   without recursion, and cycles cannot be expressed.
//! * Units are centimetres, rotations are HPB Euler angles in **radians**, colours
//!   are linear RGB — the scene's own conventions, so a round trip is exact.
//! * Primitive kinds carry their segment counts (`"segments"`, `"rotation_segments"`, …).
//! * Detached (deleted but undoable) nodes and editor state are never saved.
//!
//! Version 3 (Grid4D 1.0) is the same archive, frozen; version 4 only adds to it,
//! so a version 3 `document.json` is upgraded as JSON (`v3.rs`) and read through
//! version 4's types. Versions 1 and 2 were plain JSON documents (tag `"oc4d"`,
//! no polygon objects); they are still read and migrated on load. Files are
//! always written in the current version. The format is specified in
//! `docs/FILE_FORMAT.md`.

mod common;
mod gltf_export;
mod gltf_import;
mod import;
mod obj;
mod picture;
mod texture;
mod v1;
mod v2;
mod v3;
mod v4;

use std::{
    fmt, fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
};

use grid4d_scene::{Document, SimulationCache};
use serde::Deserialize;

use crate::common::Archive;
pub use crate::{
    gltf_export::{export_gltf, glb_bytes, gltf_document},
    gltf_import::import_gltf,
    import::{
        EXPORT_EXTENSIONS, IMPORT_EXTENSIONS, ObjectEntry, Objects, export, import, is_importable, remap_materials,
        remap_objects,
    },
    obj::{export_obj, import_obj, obj_text},
    picture::{Channels, exr_bytes, png_bytes, save_exr, save_png},
    texture::{
        Level, MAX_TEXTURE_SIDE, Pixels, TEXTURE_EXTENSIONS, TextureImage, decode_texture, is_texture_extension,
    },
};

/// File extension, without the dot.
pub const EXTENSION: &str = "g4d";
/// Every extension that opens as a document: [`EXTENSION`], and `oc4d` from before
/// the project was renamed from OpenC4D (same format). Files are saved as `EXTENSION`.
pub const OPEN_EXTENSIONS: [&str; 2] = [EXTENSION, "oc4d"];
/// The format version this build writes (and the newest it reads).
pub const FORMAT_VERSION: u32 = v4::VERSION;
/// The `"format"` tag of archives (version 3 and later).
const FORMAT_TAG: &str = "grid4d";
/// The tag of the JSON files of versions 1 and 2, from the project's former name.
const LEGACY_FORMAT_TAG: &str = "oc4d";
/// Every zip archive starts with a local file header.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// Upper bound for `document.json` once decompressed (against zip bombs).
const MAX_DOCUMENT_BYTES: u64 = 256 << 20;

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
            // Line 0: found in JSON already parsed (an older version's, upgraded).
            Self::Parse { line: 0, message, .. } => f.write_str(message),
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

impl From<zip::result::ZipError> for IoError {
    fn from(err: zip::result::ZipError) -> Self {
        match err {
            zip::result::ZipError::Io(err) => Self::Io(err),
            // Everything else: a damaged archive, or one using zip features we never write.
            other => Self::Invalid(format!("damaged or unsupported archive: {other}")),
        }
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

/// Serialises `doc` in the current format (linked textures' paths as they
/// are; see [`to_bytes_at`]).
pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, IoError> {
    v4::write(doc, None, None)
}

/// [`to_bytes`], with a simulation's frames saved beside the document.
pub fn to_bytes_with_cache(doc: &Document, cache: Option<&SimulationCache>) -> Result<Vec<u8>, IoError> {
    v4::write(doc, cache, None)
}

/// [`to_bytes_with_cache`] for a file in `folder`: linked textures inside it
/// are stored relative to it, so the folder can move with its pictures.
pub fn to_bytes_at(doc: &Document, cache: Option<&SimulationCache>, folder: &Path) -> Result<Vec<u8>, IoError> {
    v4::write(doc, cache, Some(folder))
}

/// Reads a document written by this or an older format version: an archive
/// (version 3 and later) or a JSON text file (versions 1 and 2).
pub fn from_bytes(bytes: &[u8]) -> Result<Document, IoError> {
    from_bytes_with_cache(bytes).map(|(doc, _)| doc)
}

/// [`from_bytes`], with the simulation's frames saved beside the document, if
/// any.
pub fn from_bytes_with_cache(bytes: &[u8]) -> Result<(Document, Option<SimulationCache>), IoError> {
    read_bytes(bytes, None)
}

/// [`from_bytes_with_cache`] for a file in `folder`: relative texture paths
/// are taken from it.
pub fn from_bytes_at(bytes: &[u8], folder: &Path) -> Result<(Document, Option<SimulationCache>), IoError> {
    read_bytes(bytes, Some(folder))
}

fn read_bytes(bytes: &[u8], folder: Option<&Path>) -> Result<(Document, Option<SimulationCache>), IoError> {
    if bytes.starts_with(ZIP_MAGIC) {
        let mut archive = Archive::new(bytes)?;
        let json = archive.read(v4::DOCUMENT_ENTRY, MAX_DOCUMENT_BYTES)?;
        let text = std::str::from_utf8(&json).map_err(|_| IoError::NotADocument)?;
        return match Header::parse(text)? {
            (FORMAT_TAG, v4::VERSION) => v4::File::parse(text)?.into_document(&mut archive, folder),
            (FORMAT_TAG, v3::VERSION) => {
                v4::File::from_value(v3::upgrade(serde_json::from_str(text)?)?)?.into_document(&mut archive, folder)
            }
            (FORMAT_TAG, found) if found > FORMAT_VERSION => Err(IoError::UnsupportedVersion {
                found,
                newest: FORMAT_VERSION,
            }),
            _ => Err(IoError::NotADocument),
        };
    }
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::NotADocument)?;
    from_json(text).map(|doc| (doc, None))
}

/// Reads a JSON document of format version 1 or 2.
fn from_json(text: &str) -> Result<Document, IoError> {
    match Header::parse(text)? {
        (LEGACY_FORMAT_TAG, 1) => v1::File::parse(text)?.into_document(),
        (LEGACY_FORMAT_TAG, 2) => v2::File::parse(text)?.into_document(),
        (LEGACY_FORMAT_TAG | FORMAT_TAG, found) if found > FORMAT_VERSION => Err(IoError::UnsupportedVersion {
            found,
            newest: FORMAT_VERSION,
        }),
        _ => Err(IoError::NotADocument),
    }
}

/// Just enough of a document to dispatch on its version before parsing the rest.
#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

impl Header {
    /// The format tag (one of ours, or `""`) and the version.
    fn parse(text: &str) -> Result<(&'static str, u32), IoError> {
        let header: Self = serde_json::from_str(text)?;
        let tag = match header.format.as_deref() {
            Some(FORMAT_TAG) => FORMAT_TAG,
            Some(LEGACY_FORMAT_TAG) => LEGACY_FORMAT_TAG,
            _ => "",
        };
        header
            .version
            .map(|version| (tag, version))
            .ok_or(IoError::NotADocument)
    }
}

/// Saves `doc` to `path`.
///
/// Writes a temporary file next to `path` and renames it over the target, so a
/// crash or a full disk never leaves a half-written document behind.
pub fn save(doc: &Document, path: &Path) -> Result<(), IoError> {
    save_with_cache(doc, None, path)
}

/// [`save`], with a simulation's frames saved beside the document. Linked
/// textures inside the file's folder are stored relative to it.
pub fn save_with_cache(doc: &Document, cache: Option<&SimulationCache>, path: &Path) -> Result<(), IoError> {
    write_atomic(path, &v4::write(doc, cache, Some(folder_of(path)))?)
}

/// The folder a file lies in (the current one for a bare name).
fn folder_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// Writes `bytes` to a temporary file next to `path` and renames it over `path`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IoError> {
    let temp = temp_path(path);
    if let Err(err) = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, path)) {
        // Best effort: the temporary file may not even exist.
        let _ = fs::remove_file(&temp);
        return Err(err.into());
    }
    Ok(())
}

/// Loads the document at `path`.
pub fn load(path: &Path) -> Result<Document, IoError> {
    load_with_cache(path).map(|(doc, _)| doc)
}

/// [`load`], with the simulation's frames saved beside the document, if any.
/// Relative texture paths are taken from the file's folder.
pub fn load_with_cache(path: &Path) -> Result<(Document, Option<SimulationCache>), IoError> {
    read_bytes(&read_file(path)?, Some(folder_of(path)))
}

/// Reads the whole file at `path`, reserving the memory it needs first: a
/// file larger than the memory available is refused with an error
/// (`ErrorKind::OutOfMemory`) instead of stopping the program.
pub fn read_file(path: &Path) -> Result<Vec<u8>, IoError> {
    let mut file = fs::File::open(path)?;
    let mut bytes = reserve(file.metadata()?.len())?;
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// An empty buffer with room for `size` bytes, or an error when that much
/// memory cannot be had.
fn reserve(size: u64) -> Result<Vec<u8>, IoError> {
    let mut bytes = Vec::new();
    match usize::try_from(size).map(|size| bytes.try_reserve_exact(size)) {
        Ok(Ok(())) => Ok(bytes),
        _ => Err(IoError::Io(io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!("not enough memory to read this file ({} MB)", size.div_ceil(1 << 20)),
        ))),
    }
}

/// `scene.g4d` → `scene.g4d.tmp`, in the same directory (so the rename is atomic).
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_larger_than_memory_is_refused_with_a_message() {
        for size in [u64::MAX, 1 << 62] {
            let Err(IoError::Io(err)) = reserve(size) else {
                panic!("{size} bytes reserved");
            };
            assert_eq!(err.kind(), io::ErrorKind::OutOfMemory);
            assert!(
                err.to_string().starts_with("not enough memory to read this file ("),
                "{err}"
            );
        }
        assert!(reserve(1 << 20).unwrap().capacity() >= 1 << 20);
    }

    #[test]
    fn a_save_that_cannot_be_written_keeps_the_old_file() {
        let dir = std::env::temp_dir().join(format!("grid4d-io-failed-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.g4d");
        let old = Document::with_demo_scene();
        save(&old, &path).unwrap();
        let before = fs::read(&path).unwrap();

        // What a full disk does to the temporary file, made certain: it
        // cannot be written (a folder is in its place).
        fs::create_dir(temp_path(&path)).unwrap();
        let err = save(&Document::new("Newer"), &path).unwrap_err();
        assert!(matches!(err, IoError::Io(_)), "{err}");
        assert_eq!(fs::read(&path).unwrap(), before, "the old file is untouched");
        assert_eq!(load(&path).unwrap().name, old.name);

        // A temporary file left half written is removed.
        fs::remove_dir(temp_path(&path)).unwrap();
        fs::write(temp_path(&path), b"PK half").unwrap();
        save(&Document::new("Newer"), &path).unwrap();
        assert!(!temp_path(&path).exists());
        assert_eq!(read_file(&path).unwrap(), fs::read(&path).unwrap());
        assert_eq!(load(&path).unwrap().name, "Newer");
        let _ = fs::remove_dir_all(dir);
    }
}
