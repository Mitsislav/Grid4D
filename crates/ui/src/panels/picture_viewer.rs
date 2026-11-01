//! Render ▸ Render to Picture Viewer (Shift+R): the window that shows renders
//! as they sharpen, the session's history, zoom, Stop and Save As (see
//! `docs/reference/render-picture-viewer.md`). The renders themselves run in
//! [`crate::rendering`]; this panel only shows them and pushes commands.

use std::time::Duration;

use egui::{
    Button, Color32, ColorImage, Id, Pos2, ProgressBar, Rect, RichText, ScrollArea, Sense, TextureHandle,
    TextureOptions, Ui, Vec2, Window, pos2, vec2,
};

use crate::i18n::{tr, trf};
use crate::{
    EditorCommand, EditorState,
    rendering::{Picture, RenderState, Renders},
    theme::palette as p,
};

const HISTORY_WIDTH: f32 = 170.0;
const STATUS_HEIGHT: f32 = 22.0;
/// Zoom limits, in screen pixels per picture pixel.
const MIN_ZOOM: f32 = 1.0 / 16.0;
const MAX_ZOOM: f32 = 16.0;
/// How often the editor looks for news while a render runs.
const POLL: Duration = Duration::from_millis(100);

/// The picture on the GPU, re-uploaded when another picture or pass is shown.
struct Shown {
    number: u32,
    version: u64,
    texture: TextureHandle,
}

#[derive(Default)]
pub struct PictureViewerPanel {
    shown: Option<Shown>,
    /// Screen pixels per picture pixel; `None`: fit the window.
    zoom: Option<f32>,
    /// The picture's centre, from the area's centre, in points.
    pan: Vec2,
    /// The picture area's size last frame (for the zoom Fit gives).
    last_area: Vec2,
}

impl std::fmt::Debug for PictureViewerPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureViewerPanel")
            .field("zoom", &self.zoom)
            .field("pan", &self.pan)
            .finish_non_exhaustive()
    }
}

impl PictureViewerPanel {
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut EditorState) {
        if editor.renders.is_busy() {
            // The editor polls renders once a frame, also with the window closed.
            ctx.request_repaint_after(POLL);
        }
        if !editor.renders.viewer_open {
            return;
        }
        let mut open = true;
        let screen = ctx.content_rect();
        Window::new(tr("Picture Viewer"))
            .id(Id::unique("grid4d.picture_viewer"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 460.0])
            .default_pos(screen.center() - vec2(380.0, 250.0))
            .min_size([420.0, 260.0])
            .show(ctx, |ui| self.contents(ui, editor));
        editor.renders.viewer_open = open;
    }

    fn contents(&mut self, ui: &mut Ui, editor: &mut EditorState) {
        let pixels_per_point = ui.ctx().pixels_per_point();
        self.toolbar(ui, editor, pixels_per_point);
        ui.separator();
        let size = vec2(ui.available_width(), (ui.available_height() - STATUS_HEIGHT).max(120.0));
        ui.horizontal_top(|ui| {
            let area = vec2((size.x - HISTORY_WIDTH - 8.0).max(120.0), size.y);
            self.picture_area(ui, &editor.renders, area, pixels_per_point);
            history(ui, &mut editor.renders, size.y);
        });
        status_line(ui, &editor.renders);
    }

    fn toolbar(&mut self, ui: &mut Ui, editor: &mut EditorState, pixels_per_point: f32) {
        ui.horizontal(|ui| {
            let rendering = editor.renders.is_rendering();
            if ui
                .button(tr("Render"))
                .on_hover_text(tr("Render the active view (Shift+R)"))
                .clicked()
            {
                editor.commands.push(EditorCommand::RenderToPictureViewer);
            }
            if ui.add_enabled(rendering, Button::new(tr("Stop"))).clicked() {
                editor.commands.push(EditorCommand::StopRender);
            }
            ui.separator();
            let savable = editor
                .renders
                .shown_picture()
                .is_some_and(|picture| picture.has_pixels() && picture.state != RenderState::Rendering);
            if ui
                .add_enabled(savable, Button::new(tr("Save As…")))
                .on_hover_text(tr("Save the shown picture as PNG"))
                .clicked()
            {
                editor.commands.push(EditorCommand::SaveRenderAs);
            }
            ui.separator();
            if ui.add(Button::selectable(self.zoom.is_none(), tr("Fit"))).clicked() {
                (self.zoom, self.pan) = (None, Vec2::ZERO);
            }
            if ui.add(Button::selectable(self.zoom == Some(1.0), "100 %")).clicked() {
                (self.zoom, self.pan) = (Some(1.0), Vec2::ZERO);
            }
            let current = self.zoom.unwrap_or(self.fitted(&editor.renders, pixels_per_point));
            if ui.button(tr("−")).on_hover_text(tr("Zoom out")).clicked() {
                self.zoom_by(current, 0.5);
            }
            if ui.button(tr("+")).on_hover_text(tr("Zoom in")).clicked() {
                self.zoom_by(current, 2.0);
            }
            ui.label(RichText::new(format!("{:.0} %", current * 100.0)).color(p().text_dim));
        });
    }

    /// The zoom Fit gives (for the area's size last frame).
    fn fitted(&self, renders: &Renders, pixels_per_point: f32) -> f32 {
        renders
            .shown_picture()
            .map_or(1.0, |picture| fit_zoom(picture, self.last_area, pixels_per_point))
    }

    fn zoom_by(&mut self, current: f32, factor: f32) {
        let zoom = (current * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= zoom / current;
        self.zoom = Some(zoom);
    }

    fn picture_area(&mut self, ui: &mut Ui, renders: &Renders, size: Vec2, pixels_per_point: f32) {
        let (rect, response) = ui.allocate_exact_size(size, Sense::drag());
        self.last_area = rect.size();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, p().bg_well);
        let Some(picture) = renders.shown_picture() else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                tr("Render › Render to Picture Viewer (Shift+R)"),
                egui::FontId::proportional(13.0),
                p().text_dim,
            );
            return;
        };
        if !picture.has_pixels() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                tr("Preparing the scene…"),
                egui::FontId::proportional(13.0),
                p().text_dim,
            );
            return;
        }
        let texture = self.texture(ui.ctx(), picture);

        // Wheel: zoom about the pointer; drag: pan.
        let zoom = self
            .zoom
            .unwrap_or_else(|| fit_zoom(picture, rect.size(), pixels_per_point));
        if response.dragged() {
            self.pan += response.drag_delta();
            self.zoom = Some(zoom);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if response.hovered()
            && scroll != 0.0
            && let Some(pointer) = response.hover_pos()
        {
            let new = (zoom * (scroll * 0.002).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
            let from_centre = pointer - rect.center() - self.pan;
            self.pan += from_centre - from_centre * (new / zoom);
            self.zoom = Some(new);
        }
        let zoom = self.zoom.unwrap_or(zoom);

        let picture_size = vec2(to_f32(picture.width), to_f32(picture.height)) * zoom / pixels_per_point;
        let image = Rect::from_center_size(rect.center() + self.pan, picture_size);
        if picture.alpha {
            checkerboard(&painter, image.intersect(rect));
        } else {
            painter.rect_filled(image, 0.0, Color32::BLACK);
        }
        painter.image(
            texture,
            image,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    /// The shown picture's texture, uploaded again when it changed.
    fn texture(&mut self, ctx: &egui::Context, picture: &Picture) -> egui::TextureId {
        let current = self
            .shown
            .as_ref()
            .is_some_and(|s| s.number == picture.number && s.version == picture.version);
        if !current {
            let size = [
                usize::try_from(picture.width).unwrap_or(0),
                usize::try_from(picture.height).unwrap_or(0),
            ];
            let image = if picture.alpha {
                ColorImage::from_rgba_unmultiplied(size, &picture.rgba)
            } else {
                ColorImage::from_rgb(size, &picture.rgb)
            };
            // Enlarged pictures show their pixels; reduced ones are smoothed.
            let options = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Linear,
                ..TextureOptions::default()
            };
            match &mut self.shown {
                Some(shown) => {
                    shown.texture.set(image, options);
                    (shown.number, shown.version) = (picture.number, picture.version);
                }
                None => {
                    self.shown = Some(Shown {
                        number: picture.number,
                        version: picture.version,
                        texture: ctx.load_texture("grid4d.picture_viewer", image, options),
                    });
                }
            }
        }
        self.shown
            .as_ref()
            .map_or_else(egui::TextureId::default, |s| s.texture.id())
    }
}

/// The session's pictures, newest first; a click shows one.
fn history(ui: &mut Ui, renders: &mut Renders, height: f32) {
    ui.vertical(|ui| {
        ui.set_width(HISTORY_WIDTH);
        ui.label(RichText::new(tr("History")).size(11.0).color(p().text_dim));
        let shown = renders.shown_picture().map(|picture| picture.number);
        let mut chosen = None;
        ScrollArea::vertical()
            .id_salt("grid4d.picture_viewer.history")
            .max_height(height - 18.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for picture in renders.history().iter().rev() {
                    let state = match picture.state {
                        RenderState::Rendering => tr(" · rendering"),
                        RenderState::Finished => "",
                        RenderState::Stopped => tr(" · stopped"),
                    };
                    let frames = if picture.frames > 1 {
                        trf(" · {0} frames", &[&picture.frames])
                    } else {
                        String::new()
                    };
                    let text = trf(
                        "Render {0}{1}\n{2} × {3} · {4} spp{5} · {6}",
                        &[
                            &picture.number,
                            &state,
                            &picture.width,
                            &picture.height,
                            &picture.total_samples,
                            &frames,
                            &seconds(picture.elapsed),
                        ],
                    );
                    let button = Button::selectable(shown == Some(picture.number), text)
                        .min_size(vec2(HISTORY_WIDTH - 12.0, 0.0));
                    if ui.add(button).clicked() {
                        chosen = Some(picture.number);
                    }
                }
            });
        if let Some(number) = chosen {
            renders.shown = Some(number);
        }
    });
}

/// Progress of the shown render, or what happened to it.
fn status_line(ui: &mut Ui, renders: &Renders) {
    let Some(picture) = renders.shown_picture() else {
        return;
    };
    let frames = if picture.frames > 1 {
        trf(
            " · frame {0} ({1} of {2} done)",
            &[&picture.frame, &picture.frames_done, &picture.frames],
        )
    } else {
        String::new()
    };
    let summary = trf(
        "Render {0} · {1} × {2}{3} · {4} of {5} samples · {6}",
        &[
            &picture.number,
            &picture.width,
            &picture.height,
            &frames,
            &picture.samples,
            &picture.total_samples,
            &seconds(picture.elapsed),
        ],
    );
    match renders.progress().filter(|&(number, _)| number == picture.number) {
        Some((_, fraction)) => {
            ui.add(
                ProgressBar::new(fraction)
                    .desired_height(16.0)
                    .text(trf("Rendering · {0}", &[&summary])),
            );
        }
        None => {
            let text = if picture.note.is_empty() {
                summary
            } else {
                format!("{summary} · {}", picture.note)
            };
            ui.label(RichText::new(text).size(11.5).color(p().text_dim));
        }
    }
}

/// The zoom that shows all of `picture` in an area of `area` points.
fn fit_zoom(picture: &Picture, area: Vec2, pixels_per_point: f32) -> f32 {
    let (w, h) = (to_f32(picture.width).max(1.0), to_f32(picture.height).max(1.0));
    let fit = (area.x / w).min(area.y / h) * pixels_per_point;
    fit.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Grey squares behind transparent pictures.
fn checkerboard(painter: &egui::Painter, rect: Rect) {
    const CELL: f32 = 8.0;
    painter.rect_filled(rect, 0.0, Color32::from_gray(0x66));
    let columns = (rect.width() / CELL).ceil();
    let rows = (rect.height() / CELL).ceil();
    let mut y = 0.0;
    while y < rows {
        let mut x = (y % 2.0).floor();
        while x < columns {
            let min = Pos2::new(rect.left() + x * CELL, rect.top() + y * CELL);
            let cell = Rect::from_min_size(min, Vec2::splat(CELL)).intersect(rect);
            painter.rect_filled(cell, 0.0, Color32::from_gray(0x99));
            x += 2.0;
        }
        y += 1.0;
    }
}

fn seconds(duration: Duration) -> String {
    format!("{:.1} s", duration.as_secs_f32())
}

/// Picture sizes (at most 16384) as points.
fn to_f32(n: u32) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

#[cfg(test)]
mod tests {
    use egui::{Event, MouseWheelSource, MouseWheelUnit, RawInput, TouchPhase};
    use grid4d_scene::Document;

    use super::*;

    fn frame(ctx: &egui::Context, panel: &mut PictureViewerPanel, editor: &mut EditorState, events: Vec<Event>) {
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            ..RawInput::default()
        };
        ctx.run_ui(input, |ui| panel.show(ui.ctx(), editor))
            .drop_without_applying_deltas();
    }

    #[test]
    fn the_wheel_zooms_about_the_pointer() {
        let mut doc = Document::with_demo_scene();
        crate::rendering::tests::small(&mut doc, 1);
        (doc.render.width, doc.render.height) = (160, 90);
        let mut editor = EditorState::default();
        editor
            .renders
            .start(&doc, &grid4d_render::OrbitCamera::default(), None, None, None);
        crate::rendering::tests::wait(&mut editor.renders);
        editor.renders.viewer_open = true;

        let ctx = egui::Context::default();
        let mut panel = PictureViewerPanel::default();
        for _ in 0..3 {
            frame(&ctx, &mut panel, &mut editor, Vec::new());
        }
        assert_eq!(panel.zoom, None, "fits the window");
        let fit = panel.fitted(&editor.renders, 1.0);
        assert!(fit > 1.0, "a small picture is enlarged to fit: {fit}");

        let over = Pos2::new(540.0, 380.0);
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut panel, &mut editor, vec![Event::PointerMoved(over), wheel]);
        for _ in 0..30 {
            frame(&ctx, &mut panel, &mut editor, vec![Event::PointerMoved(over)]);
        }
        let zoom = panel.zoom.expect("zoomed");
        assert!(zoom > fit * 1.1, "wheel up zooms in: {zoom} vs {fit}");
        assert!(panel.pan != Vec2::ZERO, "about the pointer, not the centre");
    }
}
