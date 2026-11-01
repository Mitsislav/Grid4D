//! What Grid4D says when it cannot start, or starts without a usable data
//! folder (spec `docs/reference/app-release.md`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::file::Alert;
use crate::i18n::{tr, trf};

/// Why the data folder cannot be used.
#[derive(Debug)]
pub struct DataFolderProblem {
    /// The folder, if the system named one.
    dir: Option<PathBuf>,
    /// What went wrong with it.
    error: String,
}

impl DataFolderProblem {
    /// The "Data Folder Unavailable" alert, in the interface's language.
    #[must_use]
    pub fn alert(&self) -> Alert {
        let reason = match &self.dir {
            Some(dir) => format!("{}\n{}", dir.display(), self.error),
            None => tr("The system gave Grid4D no folder for its data.").to_owned(),
        };
        Alert {
            title: "Data Folder Unavailable",
            message: format!(
                "{}\n\n{reason}",
                tr("Settings, recovery copies, scripts and crash reports cannot be kept in this session.")
            ),
        }
    }
}

/// Whether Grid4D can keep its files in `dir` (settings, recovery copies,
/// scripts, crash reports): made when missing, then a file written into it
/// and removed. `dir` `None`: the system gave no place for it.
///
/// # Errors
/// What went wrong, to tell the user ([`DataFolderProblem::alert`]).
pub fn check_data_folder(dir: Option<&Path>) -> Result<(), DataFolderProblem> {
    let Some(dir) = dir else {
        return Err(DataFolderProblem {
            dir: None,
            error: String::new(),
        });
    };
    let probe = dir.join(".grid4d-write-test");
    fs::create_dir_all(dir)
        .and_then(|()| fs::write(&probe, b"ok"))
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|err| DataFolderProblem {
            dir: Some(dir.to_path_buf()),
            error: err.to_string(),
        })
}

/// Tells the user, in a system message box, that Grid4D cannot start: its
/// graphics could not be set up (`reason`, from the system). Also logged.
pub fn cannot_start(reason: &str) {
    let text = cannot_start_text(reason);
    log::error!("{text}");
    let dialog = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(tr("Grid4D Cannot Start"))
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok);
    // On a thread of its own: the failed event loop may have left a quit
    // message in this thread's queue, which would close the box at once.
    let shown = std::thread::spawn(move || {
        dialog.show();
    });
    let _ = shown.join();
}

/// The message [`cannot_start`] shows.
#[must_use]
pub fn cannot_start_text(reason: &str) -> String {
    trf(
        "Grid4D could not start its graphics:\n\n{0}\n\nGrid4D needs a graphics card with Vulkan, Direct3D 12 or Metal support. Updating the graphics driver often helps.",
        &[&reason],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_folder_is_made_and_must_be_writable() {
        let dir = crate::autosave::tests::folder("data-folder");
        let data = dir.join("grid4d").join("data");
        assert!(check_data_folder(Some(&data)).is_ok());
        assert!(data.is_dir(), "made");
        assert_eq!(fs::read_dir(&data).unwrap().count(), 0, "the test file is gone");

        let missing = check_data_folder(None).unwrap_err().alert();
        assert_eq!(missing.title, "Data Folder Unavailable");
        assert!(missing.message.contains("cannot be kept in this session"));
        assert!(missing.message.contains("no folder for its data"));

        // A file where the folder should be: it cannot be made.
        let blocked = dir.join("blocked");
        fs::write(&blocked, "").unwrap();
        let alert = check_data_folder(Some(&blocked.join("data"))).unwrap_err().alert();
        assert!(
            alert.message.contains(&blocked.display().to_string()),
            "{}",
            alert.message
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_start_up_failure_says_why_and_what_helps() {
        let text = cannot_start_text("No suitable graphics adapter found");
        assert!(text.contains("No suitable graphics adapter found"));
        assert!(text.contains("Vulkan, Direct3D 12 or Metal"));
    }
}
