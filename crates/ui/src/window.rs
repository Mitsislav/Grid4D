//! The application window itself (as opposed to the document's file, see
//! `file.rs`): making sure it fits the screen it opens on.

use egui::{Vec2, ViewportCommand};

/// One-time start-up check. A window restored (or created) larger than its
/// monitor — e.g. the old 1600 × 940 default on a 1080p screen at 125 % scaling —
/// would hide the timeline and status bar behind the taskbar; it is maximized
/// instead. Checked once, so it never fights the user's own resizing.
#[derive(Debug, Default)]
pub struct WindowFit {
    checked: bool,
}

impl WindowFit {
    /// Call every frame; does its work on the first frame the monitor size is known.
    pub fn check(&mut self, ctx: &egui::Context) {
        if self.checked {
            return;
        }
        let (monitor, outer, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.monitor_size, v.outer_rect, v.maximized)
        });
        let (Some(monitor), Some(outer)) = (monitor, outer) else {
            return; // not reported yet
        };
        self.checked = true;
        if maximized != Some(true) && !fits(outer.size(), monitor) {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
        }
    }
}

/// `true` if a window of outer size `window` fits on a monitor of size `monitor`
/// (both in points; one point of slack for rounding).
fn fits(window: Vec2, monitor: Vec2) -> bool {
    window.x <= monitor.x + 1.0 && window.y <= monitor.y + 1.0
}

#[cfg(test)]
mod tests {
    use egui::{RawInput, Rect, ViewportId, ViewportInfo, pos2, vec2};

    use super::*;

    #[test]
    fn fitting_allows_rounding_slack() {
        let monitor = vec2(1536.0, 864.0);
        assert!(fits(vec2(1280.0, 800.0), monitor));
        assert!(fits(vec2(1536.4, 864.0), monitor));
        assert!(
            !fits(vec2(1536.0, 888.0), monitor),
            "the title bar pushed it past the screen"
        );
    }

    /// Runs one headless frame with the given window/monitor info and tells whether
    /// the window was maximized (egui sends other viewport commands of its own).
    fn maximizes(fit: &mut WindowFit, info: ViewportInfo) -> bool {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.viewports.insert(ViewportId::ROOT, info);
        let output = ctx.run_ui(input, |ui| fit.check(ui.ctx()));
        let sent = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default();
        output.drop_without_applying_deltas();
        sent.contains(&ViewportCommand::Maximized(true))
    }

    fn info(outer: Vec2, maximized: bool) -> ViewportInfo {
        ViewportInfo {
            monitor_size: Some(vec2(1536.0, 864.0)),
            outer_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), outer)),
            maximized: Some(maximized),
            ..ViewportInfo::default()
        }
    }

    #[test]
    fn an_oversized_window_is_maximized_once() {
        let mut fit = WindowFit::default();
        assert!(maximizes(&mut fit, info(vec2(1550.0, 890.0), false)));
        // Later frames leave the user's window alone.
        assert!(!maximizes(&mut fit, info(vec2(1550.0, 890.0), false)));
    }

    #[test]
    fn a_fitting_or_maximized_window_is_left_alone() {
        assert!(!maximizes(&mut WindowFit::default(), info(vec2(1280.0, 800.0), false)));
        assert!(!maximizes(&mut WindowFit::default(), info(vec2(1550.0, 890.0), true)));
    }

    #[test]
    fn it_waits_until_the_monitor_is_known() {
        let mut fit = WindowFit::default();
        assert!(!maximizes(&mut fit, ViewportInfo::default()));
        assert!(!fit.checked, "nothing reported yet: try again next frame");
    }
}
