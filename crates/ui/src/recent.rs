//! The recently used documents (File ▸ Open Recent), newest first.
//!
//! Persisted by the application as a plain string (one path per line) under
//! [`STORAGE_KEY`], so this crate needs no storage backend.

use std::path::{Path, PathBuf};

/// Key under which the application stores [`RecentFiles::to_storage`].
pub const STORAGE_KEY: &str = "grid4d.recent_files";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// How many documents are remembered.
    pub const LIMIT: usize = 10;

    /// Parses the stored list; blank lines and duplicates are ignored.
    #[must_use]
    pub fn from_storage(text: &str) -> Self {
        let mut recent = Self::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let path = PathBuf::from(line);
            if !recent.paths.contains(&path) && recent.paths.len() < Self::LIMIT {
                recent.paths.push(path);
            }
        }
        recent
    }

    /// One path per line, newest first. Paths that are not valid UTF-8 cannot be
    /// stored faithfully as text and are left out.
    #[must_use]
    pub fn to_storage(&self) -> String {
        let mut text = String::new();
        for path in self.paths.iter().filter_map(|p| p.to_str()) {
            text.push_str(path);
            text.push('\n');
        }
        text
    }

    /// Moves `path` to the top, adding it if new and dropping the oldest beyond the limit.
    pub fn touch(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(Self::LIMIT);
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Newest first.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(recent: &RecentFiles) -> Vec<&str> {
        recent.paths().iter().map(|p| p.to_str().unwrap()).collect()
    }

    #[test]
    fn touch_moves_to_the_top_without_duplicates() {
        let mut recent = RecentFiles::default();
        for name in ["a.g4d", "b.g4d", "c.g4d", "a.g4d"] {
            recent.touch(Path::new(name));
        }
        assert_eq!(paths(&recent), ["a.g4d", "c.g4d", "b.g4d"]);
        recent.remove(Path::new("c.g4d"));
        assert_eq!(paths(&recent), ["a.g4d", "b.g4d"]);
    }

    #[test]
    fn only_the_newest_are_kept() {
        let mut recent = RecentFiles::default();
        for i in 0..15 {
            recent.touch(Path::new(&format!("{i}.g4d")));
        }
        assert_eq!(recent.paths().len(), RecentFiles::LIMIT);
        assert_eq!(recent.paths()[0], Path::new("14.g4d"));
        assert_eq!(recent.paths()[RecentFiles::LIMIT - 1], Path::new("5.g4d"));
    }

    #[test]
    fn storage_round_trips_and_tolerates_junk() {
        let mut recent = RecentFiles::default();
        recent.touch(Path::new("C:/scenes/old shot.g4d"));
        recent.touch(Path::new("/home/me/ünïcode.g4d"));
        assert_eq!(RecentFiles::from_storage(&recent.to_storage()), recent);

        let loaded = RecentFiles::from_storage("\n  a.g4d  \n\nb.g4d\na.g4d\n");
        assert_eq!(paths(&loaded), ["a.g4d", "b.g4d"]);
        assert!(RecentFiles::from_storage("").is_empty());
    }
}
