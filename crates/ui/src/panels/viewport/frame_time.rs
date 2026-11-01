//! View ▸ Frame Time (spec `docs/reference/app-release.md`): the time between
//! frames and the slowest part of the viewport's own work, averaged over the
//! last frames, drawn in the active view's top-right corner.

use std::time::Instant;

use egui::{Align2, FontId, Painter, Rect, vec2};

use crate::i18n::{key, tr, trf};
use crate::theme::view_palette as p;

/// The parts of the viewport's work that are timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    /// Animation, generators, deformers, clones.
    Evaluation,
    /// What to draw, batched (and the component overlay).
    Extraction,
    /// The commands sent to the graphics card.
    Drawing,
}

impl Part {
    const ALL: [Self; 3] = [Self::Evaluation, Self::Extraction, Self::Drawing];

    const fn index(self) -> usize {
        match self {
            Self::Evaluation => 0,
            Self::Extraction => 1,
            Self::Drawing => 2,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Evaluation => key("Evaluation"),
            Self::Extraction => key("Extraction"),
            Self::Drawing => key("Drawing"),
        }
    }
}

/// The viewport's frame times. Timing costs a clock read per part; nothing
/// allocates until the readout is drawn.
#[derive(Debug)]
pub(super) struct FrameTimes {
    /// The readout is on (View ▸ Frame Time); not saved.
    pub(super) shown: bool,
    /// This frame's parts so far, seconds.
    current: [f64; 3],
    /// Averages: the time between frames and each part, seconds.
    interval: f64,
    parts: [f64; 3],
    /// The last frames' times between frames (a ring), for the worst one.
    recent: [f32; Self::RECENT],
    next: usize,
}

impl Default for FrameTimes {
    fn default() -> Self {
        Self {
            shown: false,
            current: [0.0; 3],
            interval: 0.0,
            parts: [0.0; 3],
            recent: [0.0; Self::RECENT],
            next: 0,
        }
    }
}

impl FrameTimes {
    /// How much of a new frame goes into the averages.
    const SMOOTHING: f64 = 0.1;
    /// A longer gap between frames is a pause, not a slow frame.
    const PAUSE: f64 = 0.25;
    /// Frames the worst one is taken from (about two seconds at 60 fps).
    const RECENT: usize = 128;

    /// A new frame, `dt` seconds after the last one: the last frame's parts
    /// go into the averages (unless the gap was a pause).
    pub(super) fn begin(&mut self, dt: f32) {
        let (dt, seconds) = (f64::from(dt), dt);
        if dt > 0.0 && dt <= Self::PAUSE {
            self.interval = blend(self.interval, dt);
            self.recent[self.next] = seconds;
            self.next = (self.next + 1) % Self::RECENT;
            for (average, now) in self.parts.iter_mut().zip(self.current) {
                *average = blend(*average, now);
            }
        }
        self.current = [0.0; 3];
    }

    /// Adds the time since `start` to `part`.
    pub(super) fn add(&mut self, part: Part, start: Instant) {
        self.current[part.index()] += start.elapsed().as_secs_f64();
    }

    /// The slowest part on average, and its time in seconds.
    fn slowest(&self) -> (Part, f64) {
        Part::ALL.into_iter().map(|part| (part, self.parts[part.index()])).fold(
            (Part::Evaluation, 0.0),
            |best, next| if next.1 > best.1 { next } else { best },
        )
    }

    /// The slowest of the last frames, seconds.
    fn worst(&self) -> f32 {
        self.recent.iter().copied().fold(0.0, f32::max)
    }

    /// The two lines of the readout: "16.7 ms · 60 fps · worst 18.0 ms",
    /// "slowest: …".
    fn lines(&self) -> [String; 2] {
        let fps = if self.interval > 0.0 {
            (1.0 / self.interval).round()
        } else {
            0.0
        };
        let (part, time) = self.slowest();
        [
            trf(
                "{0} ms · {1} fps · worst {2} ms",
                &[
                    &format!("{:.1}", self.interval * 1000.0),
                    &fps,
                    &format!("{:.1}", self.worst() * 1000.0),
                ],
            ),
            trf(
                "slowest: {0} {1} ms",
                &[&tr(part.label()), &format!("{:.1}", time * 1000.0)],
            ),
        ]
    }

    /// Draws the readout in `rect`'s top-right corner.
    pub(super) fn paint(&self, painter: &Painter, rect: Rect) {
        let [first, second] = self.lines();
        let at = rect.right_top() + vec2(-12.0, 10.0);
        painter.text(at, Align2::RIGHT_TOP, first, FontId::monospace(11.5), p().text);
        painter.text(
            at + vec2(0.0, 16.0),
            Align2::RIGHT_TOP,
            second,
            FontId::monospace(11.5),
            p().text_dim,
        );
    }
}

/// `average` moved towards `now`; the first value is taken as it is.
fn blend(average: f64, now: f64) -> f64 {
    if average == 0.0 {
        now
    } else {
        average + (now - average) * FrameTimes::SMOOTHING
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A clock read `duration` ago.
    fn after(duration: Duration) -> Instant {
        Instant::now().checked_sub(duration).unwrap()
    }

    #[test]
    fn averages_skip_pauses_and_name_the_slowest_part() {
        let mut times = FrameTimes::default();
        for _ in 0..100 {
            times.begin(1.0 / 60.0);
            times.add(Part::Evaluation, after(Duration::from_millis(1)));
            times.add(Part::Drawing, after(Duration::from_millis(3)));
            times.add(Part::Drawing, after(Duration::from_millis(1)));
        }
        times.begin(1.0 / 60.0);
        assert!((times.interval - 1.0 / 60.0).abs() < 1e-6);
        let (part, time) = times.slowest();
        assert_eq!(part, Part::Drawing);
        assert!((0.0039..0.006).contains(&time), "{time}");

        // A pause of two seconds changes nothing.
        let before = times.interval;
        times.begin(2.0);
        assert!((times.interval - before).abs() < 1e-12);

        let [first, second] = times.lines();
        assert_eq!(first, "16.7 ms · 60 fps · worst 16.7 ms");
        times.begin(0.1);
        assert!(times.lines()[0].ends_with("worst 100.0 ms"), "a slow frame shows");
        assert!(second.starts_with("slowest: Drawing "), "{second}");
    }
}
