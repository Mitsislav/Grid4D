/// Transport state of the document timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Stopped,
    Forward,
    Backward,
}

/// Document time: frame rate, preview range and the current time.
///
/// Time is kept as a fractional frame (`f64`) so playback stays smooth and
/// drift-free at any display refresh rate; [`TimeState::frame`] is the integer
/// frame shown in the UI. Defaults match Cinema 4D: 30 fps, frames 0–90.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeState {
    fps: u32,
    start: i32,
    end: i32,
    current: f64,
    playback: Playback,
    pub looping: bool,
}

impl Default for TimeState {
    fn default() -> Self {
        Self {
            fps: 30,
            start: 0,
            end: 90,
            current: 0.0,
            playback: Playback::Stopped,
            looping: true,
        }
    }
}

impl TimeState {
    pub const MAX_FPS: u32 = 240;

    #[must_use]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.fps = fps.clamp(1, Self::MAX_FPS);
    }

    #[must_use]
    pub fn start(&self) -> i32 {
        self.start
    }

    #[must_use]
    pub fn end(&self) -> i32 {
        self.end
    }

    /// Sets the preview range. `end` is forced to be at least `start + 1`.
    pub fn set_range(&mut self, start: i32, end: i32) {
        self.start = start;
        self.end = end.max(start.saturating_add(1));
        self.current = self.current.clamp(f64::from(self.start), f64::from(self.end));
    }

    /// Number of frames in the range (inclusive).
    #[must_use]
    pub fn frame_count(&self) -> i32 {
        self.end - self.start + 1
    }

    /// The current integer frame.
    #[must_use]
    pub fn frame(&self) -> i32 {
        // Truncation is the intent: frame N lasts from N.0 up to N.999….
        #[expect(clippy::cast_possible_truncation, reason = "clamped to the i32 frame range")]
        let frame = self.current.floor() as i32;
        frame
    }

    /// The current time in fractional frames.
    #[must_use]
    pub fn frame_f64(&self) -> f64 {
        self.current
    }

    pub fn set_frame(&mut self, frame: i32) {
        self.current = f64::from(frame.clamp(self.start, self.end));
    }

    /// Current time in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.current / f64::from(self.fps)
    }

    #[must_use]
    pub fn playback(&self) -> Playback {
        self.playback
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playback != Playback::Stopped
    }

    pub fn play_forward(&mut self) {
        self.playback = Playback::Forward;
    }

    pub fn play_backward(&mut self) {
        self.playback = Playback::Backward;
    }

    pub fn stop(&mut self) {
        self.playback = Playback::Stopped;
        // Land on a whole frame so the UI shows a stable value.
        self.current = self.current.floor();
    }

    /// Play/pause toggle (forward).
    pub fn toggle_play(&mut self) {
        if self.is_playing() {
            self.stop();
        } else {
            self.play_forward();
        }
    }

    /// Steps by `delta` frames, clamped to the range.
    pub fn step(&mut self, delta: i32) {
        self.set_frame(self.frame().saturating_add(delta));
    }

    pub fn go_to_start(&mut self) {
        self.current = f64::from(self.start);
    }

    pub fn go_to_end(&mut self) {
        self.current = f64::from(self.end);
    }

    /// Advances playback by `dt_seconds` of wall-clock time.
    /// Returns `true` if the integer frame changed.
    pub fn advance(&mut self, dt_seconds: f64) -> bool {
        let direction = match self.playback {
            Playback::Stopped => return false,
            Playback::Forward => 1.0,
            Playback::Backward => -1.0,
        };
        let before = self.frame();
        let start = f64::from(self.start);
        // The last frame is displayed for a full frame duration before wrapping.
        let end_exclusive = f64::from(self.end) + 1.0;
        let len = end_exclusive - start;

        self.current += direction * dt_seconds.max(0.0) * f64::from(self.fps);

        if self.current >= end_exclusive || self.current < start {
            if self.looping {
                self.current = start + (self.current - start).rem_euclid(len);
            } else {
                self.current = self.current.clamp(start, f64::from(self.end));
                self.playback = Playback::Stopped;
            }
        }
        self.frame() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_at_fps() {
        let mut t = TimeState::default();
        t.play_forward();
        assert!(t.advance(1.0));
        assert_eq!(t.frame(), 30);
    }

    #[test]
    fn loops_forward_and_backward() {
        let mut t = TimeState::default();
        t.set_range(0, 9); // 10 frames
        t.set_fps(10);
        t.play_forward();
        t.advance(1.25); // 12.5 frames → wraps to 2.5
        assert_eq!(t.frame(), 2);

        t.play_backward();
        t.advance(0.5); // 2.5 - 5 = -2.5 → wraps to 7.5
        assert_eq!(t.frame(), 7);
    }

    #[test]
    fn stops_at_end_without_loop() {
        let mut t = TimeState {
            looping: false,
            ..TimeState::default()
        };
        t.play_forward();
        t.advance(100.0);
        assert_eq!(t.frame(), 90);
        assert!(!t.is_playing());
    }

    #[test]
    fn range_is_always_valid() {
        let mut t = TimeState::default();
        t.set_frame(50);
        t.set_range(10, 5);
        assert_eq!((t.start(), t.end()), (10, 11));
        assert_eq!(t.frame(), 11);
    }
}
