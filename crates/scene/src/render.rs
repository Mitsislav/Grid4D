//! How the document is rendered (see `docs/reference/render-settings.md`):
//! picture size, quality, frames and where the picture is saved.

use std::{
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

use crate::TimeState;

/// Which frames an animation render renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameRange {
    /// The frame at the time slider.
    #[default]
    Current,
    /// The document's range (Minimum to Maximum Time).
    All,
    /// `from` to `to`, both included (`to` is never before `from`).
    Manual { from: i32, to: i32 },
}

impl FrameRange {
    /// The choices of the Frame Range list, Manual with its range.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "Current Frame",
            Self::All => "All Frames",
            Self::Manual { .. } => "Manual",
        }
    }
}

/// The document's render settings. Change them through
/// [`SceneEdit::SetRender`](crate::SceneEdit::SetRender), which keeps them
/// [`validated`](Self::validated).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderSettings {
    /// Picture size in pixels (1…[`Self::MAX_SIZE`] each).
    pub width: u32,
    pub height: u32,
    /// Changing one side of the picture changes the other with it (the panel's
    /// behaviour; stored so it stays on).
    pub lock_ratio: bool,
    pub frames: FrameRange,
    /// Every n-th frame of the range (1…[`Self::MAX_FRAME_STEP`]).
    pub frame_step: u32,
    /// Samples per pixel (1…[`Self::MAX_SAMPLES`]).
    pub samples: u32,
    /// Light bounces between surfaces (0…[`Self::MAX_BOUNCES`]).
    pub bounces: u32,
    /// A scene without lights is lit by the automatic light.
    pub auto_light: bool,
    /// A finished render is saved to [`Self::output`].
    pub save: bool,
    /// Where the picture is saved, as typed: absolute, or relative to the
    /// document's folder (see [`Self::output_path`]). Empty: nowhere.
    pub output: String,
    /// The saved picture keeps its transparency; otherwise the background is black.
    pub alpha: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            lock_ratio: false,
            frames: FrameRange::Current,
            frame_step: 1,
            samples: 16,
            bounces: 2,
            auto_light: true,
            save: true,
            output: String::new(),
            alpha: false,
        }
    }
}

impl RenderSettings {
    pub const MAX_SIZE: u32 = 16_384;
    pub const MAX_SAMPLES: u32 = 4096;
    pub const MAX_BOUNCES: u32 = 64;
    pub const MAX_FRAME_STEP: u32 = 10_000;
    /// The extension of saved pictures (PNG; OpenEXR is planned).
    pub const EXTENSION: &str = "png";

    /// The settings with every value in its range.
    #[must_use]
    pub fn validated(mut self) -> Self {
        self.width = self.width.clamp(1, Self::MAX_SIZE);
        self.height = self.height.clamp(1, Self::MAX_SIZE);
        self.frame_step = self.frame_step.clamp(1, Self::MAX_FRAME_STEP);
        self.samples = self.samples.clamp(1, Self::MAX_SAMPLES);
        self.bounces = self.bounces.min(Self::MAX_BOUNCES);
        if let FrameRange::Manual { from, to } = &mut self.frames {
            *to = (*to).max(*from);
        }
        self
    }

    /// The first and last frame of the range for a document with `time`.
    #[must_use]
    pub fn frame_span(&self, time: &TimeState) -> RangeInclusive<i32> {
        match self.frames {
            FrameRange::Current => time.frame()..=time.frame(),
            FrameRange::All => time.start()..=time.end(),
            FrameRange::Manual { from, to } => from..=to.max(from),
        }
    }

    /// The frames to render, in order: the span, every [`Self::frame_step`]-th.
    pub fn frames(&self, time: &TimeState) -> impl Iterator<Item = i32> {
        let step = usize::try_from(self.frame_step.max(1)).unwrap_or(1);
        self.frame_span(time).step_by(step)
    }

    /// How many pictures [`Self::frames`] gives.
    #[must_use]
    pub fn frame_count(&self, time: &TimeState) -> u64 {
        let span = self.frame_span(time);
        let length = i64::from(*span.end()) - i64::from(*span.start());
        u64::try_from(length).map_or(1, |length| length / u64::from(self.frame_step.max(1)) + 1)
    }

    /// The file a finished render is saved to, for a document saved at
    /// `document` (`None`: not saved yet). `None` when Save is off, File is
    /// empty, or File is relative and the document has no folder yet. `.png` is
    /// added unless the name ends with it.
    #[must_use]
    pub fn output_path(&self, document: Option<&Path>) -> Option<PathBuf> {
        let typed = self.output.trim();
        if !self.save || typed.is_empty() {
            return None;
        }
        // Rebuilt from its parts: `renders/shot` shows with the platform's separators.
        let typed: PathBuf = Path::new(typed).components().collect();
        let mut path = if typed.is_absolute() {
            typed
        } else {
            document?.parent()?.join(typed)
        };
        let has_extension = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(Self::EXTENSION));
        if !has_extension {
            let mut name = path.into_os_string();
            name.push(".");
            name.push(Self::EXTENSION);
            path = PathBuf::from(name);
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_clamps_every_value() {
        let settings = RenderSettings {
            width: 0,
            height: 99_999,
            frame_step: 0,
            samples: 100_000,
            bounces: 1000,
            frames: FrameRange::Manual { from: 10, to: 3 },
            ..RenderSettings::default()
        }
        .validated();
        assert_eq!((settings.width, settings.height), (1, RenderSettings::MAX_SIZE));
        assert_eq!(settings.frame_step, 1);
        assert_eq!(settings.samples, RenderSettings::MAX_SAMPLES);
        assert_eq!(settings.bounces, RenderSettings::MAX_BOUNCES);
        assert_eq!(settings.frames, FrameRange::Manual { from: 10, to: 10 });
        assert_eq!(RenderSettings::default().validated(), RenderSettings::default());
    }

    #[test]
    fn frames_follow_the_range_and_step() {
        let mut time = TimeState::default();
        time.set_range(0, 90);
        time.set_frame(12);
        let mut settings = RenderSettings::default();
        assert_eq!(settings.frames(&time).collect::<Vec<_>>(), [12]);
        assert_eq!(settings.frame_count(&time), 1);

        settings.frames = FrameRange::All;
        assert_eq!(settings.frame_count(&time), 91);
        settings.frame_step = 10;
        assert_eq!(
            settings.frames(&time).collect::<Vec<_>>(),
            [0, 10, 20, 30, 40, 50, 60, 70, 80, 90]
        );
        assert_eq!(settings.frame_count(&time), 10);

        settings.frames = FrameRange::Manual { from: -5, to: 7 };
        settings.frame_step = 4;
        assert_eq!(settings.frames(&time).collect::<Vec<_>>(), [-5, -1, 3, 7]);
        assert_eq!(settings.frame_count(&time), 4);
    }

    #[test]
    fn output_paths_resolve_against_the_document() {
        let document = Path::new("/projects/shot/scene.g4d");
        let mut settings = RenderSettings::default();
        assert_eq!(settings.output_path(Some(document)), None, "empty: nowhere");

        settings.output = "render".to_owned();
        assert_eq!(
            settings.output_path(Some(document)),
            Some(PathBuf::from("/projects/shot/render.png"))
        );
        assert_eq!(settings.output_path(None), None, "relative needs a saved document");

        settings.output = "out/final.PNG".to_owned();
        assert_eq!(
            settings.output_path(Some(document)),
            Some(PathBuf::from("/projects/shot/out/final.PNG"))
        );
        settings.output = "take.v2".to_owned();
        assert_eq!(
            settings.output_path(Some(document)),
            Some(PathBuf::from("/projects/shot/take.v2.png")),
            "appended, not replaced"
        );

        let absolute = std::env::temp_dir().join("still");
        settings.output = absolute.to_string_lossy().into_owned();
        assert_eq!(settings.output_path(None), Some(absolute.with_extension("png")));

        settings.save = false;
        assert_eq!(settings.output_path(None), None, "Save off");
    }
}
