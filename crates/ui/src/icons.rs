//! Vector icons painted with egui shapes.
//!
//! Painting icons (instead of using font glyphs or bitmaps) keeps them crisp at
//! any DPI, tintable from the palette, and free of third-party artwork.
//! Coordinates below are in a unit square: (0, 0) top-left, (1, 1) bottom-right.

use std::f32::consts::{PI, TAU};

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, vec2};
use grid4d_scene::ObjectKindTag;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Object(ObjectKindTag),
    // Tools
    LiveSelection,
    RectangleSelection,
    LoopSelection,
    RingSelection,
    Move,
    Scale,
    Rotate,
    // Component modes
    MakeEditable,
    ModelMode,
    PointsMode,
    EdgesMode,
    PolygonsMode,
    // Edit
    Undo,
    Redo,
    CoordWorld,
    CoordObject,
    Snap,
    // Render
    RenderView,
    RenderPictureViewer,
    RenderSettings,
    // Transport
    GoToStart,
    StepBack,
    PlayBackward,
    PlayForward,
    Pause,
    StepForward,
    GoToEnd,
    Record,
    Loop,
    // Viewport navigation
    NavZoom,
    // Small glyphs
    Check,
    Cross,
    FoldRight,
    FoldDown,
}

/// Paints `icon` centred in `rect` (the largest square that fits).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let c = Canvas {
        painter,
        rect: Rect::from_center_size(rect.center(), Vec2::splat(side)),
        color,
        stroke: Stroke::new((side / 11.0).clamp(1.0, 1.8), color),
    };
    let soft = color.gamma_multiply(0.3);

    match icon {
        Icon::Object(tag) => paint_object(&c, tag, soft),

        Icon::LiveSelection => {
            c.circle((0.42, 0.42), 0.3);
            c.fill(&[(0.55, 0.52), (0.95, 0.72), (0.72, 0.95)]);
        }
        Icon::RectangleSelection => {
            // A dashed frame and the pointer dragging it.
            for (a, b) in [
                ((0.08, 0.12), (0.32, 0.12)),
                ((0.48, 0.12), (0.72, 0.12)),
                ((0.08, 0.12), (0.08, 0.34)),
                ((0.08, 0.5), (0.08, 0.72)),
                ((0.08, 0.72), (0.3, 0.72)),
                ((0.72, 0.12), (0.72, 0.32)),
            ] {
                c.line(a, b);
            }
            c.fill(&[(0.55, 0.52), (0.95, 0.72), (0.72, 0.95)]);
        }
        Icon::LoopSelection => {
            // A quad grid with one row of edges picked out along it.
            let faint = color.gamma_multiply(0.4);
            for x in [0.1, 0.5, 0.9] {
                c.polyline_stroke(&[(x, 0.1), (x, 0.9)], false, Stroke::new(c.stroke.width, faint));
            }
            c.polyline_stroke(&[(0.1, 0.1), (0.9, 0.1)], false, Stroke::new(c.stroke.width, faint));
            c.polyline_stroke(&[(0.1, 0.9), (0.9, 0.9)], false, Stroke::new(c.stroke.width, faint));
            let thick = Stroke::new(c.stroke.width * 1.7, color);
            c.polyline_stroke(&[(0.04, 0.5), (0.96, 0.5)], false, thick);
        }
        Icon::RingSelection => {
            // The same grid with the parallel edges across a row picked out.
            let faint = color.gamma_multiply(0.4);
            for y in [0.1, 0.5, 0.9] {
                c.polyline_stroke(&[(0.1, y), (0.9, y)], false, Stroke::new(c.stroke.width, faint));
            }
            let thick = Stroke::new(c.stroke.width * 1.7, color);
            for x in [0.1, 0.5, 0.9] {
                c.polyline_stroke(&[(x, 0.08), (x, 0.52)], false, thick);
            }
        }
        Icon::Move => {
            c.line((0.5, 0.14), (0.5, 0.86));
            c.line((0.14, 0.5), (0.86, 0.5));
            for (tip, dir) in [
                ((0.5, 0.04), (0.0, -1.0)),
                ((0.5, 0.96), (0.0, 1.0)),
                ((0.04, 0.5), (-1.0, 0.0)),
                ((0.96, 0.5), (1.0, 0.0)),
            ] {
                c.arrow_head(tip, dir, 0.17);
            }
        }
        Icon::Scale => {
            c.rect_stroke((0.08, 0.08), (0.92, 0.92), color.gamma_multiply(0.45));
            c.rect_fill((0.08, 0.6), (0.4, 0.92), color);
            c.line((0.4, 0.6), (0.78, 0.22));
            c.arrow_head((0.9, 0.1), (1.0, -1.0), 0.22);
        }
        Icon::Rotate => {
            let end = c.arc_points((0.5, 0.5), 0.36, 0.2 * PI, 1.8 * PI);
            c.arrow_head_at(end, 0.2);
        }

        Icon::MakeEditable => {
            // The cube's wireframe with its points showing: now it can be edited.
            paint_object(&c, ObjectKindTag::Cube, Color32::TRANSPARENT);
            for p in [
                (0.5, 0.08),
                (0.9, 0.29),
                (0.9, 0.71),
                (0.5, 0.92),
                (0.1, 0.71),
                (0.1, 0.29),
                (0.5, 0.5),
            ] {
                c.disc(p, 0.08);
            }
        }
        Icon::ModelMode => paint_object(&c, ObjectKindTag::Cube, Color32::TRANSPARENT),
        Icon::PointsMode => {
            c.rect_stroke((0.18, 0.18), (0.82, 0.82), color.gamma_multiply(0.35));
            for p in [(0.18, 0.18), (0.82, 0.18), (0.82, 0.82), (0.18, 0.82)] {
                c.disc(p, 0.11);
            }
        }
        Icon::EdgesMode => {
            let thick = Stroke::new(c.stroke.width * 1.6, color);
            c.polyline_stroke(&[(0.15, 0.15), (0.85, 0.15), (0.85, 0.85), (0.15, 0.85)], true, thick);
            c.line((0.15, 0.85), (0.85, 0.15));
        }
        Icon::PolygonsMode => {
            c.fill(&[(0.15, 0.15), (0.85, 0.15), (0.85, 0.85), (0.15, 0.85)]);
        }

        Icon::Undo => {
            let end = c.arc_points((0.55, 0.62), 0.32, 0.0, -PI);
            c.arrow_head_at(end, 0.22);
        }
        Icon::Redo => {
            let end = c.arc_points((0.45, 0.62), 0.32, PI, TAU);
            c.arrow_head_at(end, 0.22);
        }
        Icon::CoordWorld => {
            c.circle((0.5, 0.5), 0.4);
            c.ellipse((0.5, 0.5), (0.4, 0.14));
            c.ellipse((0.5, 0.5), (0.15, 0.4));
        }
        Icon::CoordObject => {
            let o = (0.22, 0.78);
            c.line(o, (0.22, 0.14));
            c.line(o, (0.86, 0.78));
            c.line(o, (0.6, 0.42));
            c.arrow_head((0.22, 0.06), (0.0, -1.0), 0.16);
            c.arrow_head((0.94, 0.78), (1.0, 0.0), 0.16);
        }
        Icon::Snap => {
            let thick = Stroke::new(c.stroke.width * 2.0, color);
            c.polyline_stroke(&[(0.24, 0.2), (0.24, 0.55)], false, thick);
            c.polyline_stroke(&[(0.76, 0.2), (0.76, 0.55)], false, thick);
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = PI - PI * i as f32 / 16.0;
                    c.p(0.5 + 0.26 * a.cos(), 0.55 + 0.3 * a.sin())
                })
                .collect();
            painter.add(Shape::line(pts, thick));
            c.rect_fill((0.12, 0.08), (0.36, 0.22), color);
            c.rect_fill((0.64, 0.08), (0.88, 0.22), color);
        }

        Icon::RenderView => {
            c.rect_stroke((0.06, 0.16), (0.94, 0.84), color);
            c.fill(&[(0.4, 0.32), (0.68, 0.5), (0.4, 0.68)]);
        }
        Icon::RenderPictureViewer => {
            c.rect_stroke((0.26, 0.06), (0.96, 0.66), color.gamma_multiply(0.5));
            c.rect_fill((0.04, 0.3), (0.76, 0.94), soft);
            c.rect_stroke((0.04, 0.3), (0.76, 0.94), color);
            c.fill(&[(0.3, 0.46), (0.54, 0.62), (0.3, 0.78)]);
        }
        Icon::RenderSettings => {
            c.circle((0.5, 0.5), 0.2);
            let teeth = Stroke::new(c.stroke.width * 1.9, color);
            for i in 0..8 {
                let a = TAU * i as f32 / 8.0;
                let (s, co) = a.sin_cos();
                painter.line_segment(
                    [
                        c.p(0.5 + co * 0.27, 0.5 + s * 0.27),
                        c.p(0.5 + co * 0.42, 0.5 + s * 0.42),
                    ],
                    teeth,
                );
            }
        }

        Icon::GoToStart => {
            c.rect_fill((0.14, 0.22), (0.24, 0.78), color);
            c.fill(&[(0.56, 0.22), (0.56, 0.78), (0.26, 0.5)]);
            c.fill(&[(0.88, 0.22), (0.88, 0.78), (0.58, 0.5)]);
        }
        Icon::StepBack => {
            c.rect_fill((0.24, 0.22), (0.34, 0.78), color);
            c.fill(&[(0.76, 0.22), (0.76, 0.78), (0.38, 0.5)]);
        }
        Icon::PlayBackward => c.fill(&[(0.76, 0.18), (0.76, 0.82), (0.24, 0.5)]),
        Icon::PlayForward => c.fill(&[(0.26, 0.18), (0.26, 0.82), (0.78, 0.5)]),
        Icon::Pause => {
            c.rect_fill((0.26, 0.2), (0.42, 0.8), color);
            c.rect_fill((0.58, 0.2), (0.74, 0.8), color);
        }
        Icon::StepForward => {
            c.fill(&[(0.24, 0.22), (0.24, 0.78), (0.62, 0.5)]);
            c.rect_fill((0.66, 0.22), (0.76, 0.78), color);
        }
        Icon::GoToEnd => {
            c.fill(&[(0.12, 0.22), (0.12, 0.78), (0.42, 0.5)]);
            c.fill(&[(0.44, 0.22), (0.44, 0.78), (0.74, 0.5)]);
            c.rect_fill((0.76, 0.22), (0.86, 0.78), color);
        }
        Icon::Record => c.disc((0.5, 0.5), 0.28),
        Icon::Loop => {
            let top = c.arc_points((0.5, 0.5), 0.32, PI + 0.35, TAU - 0.35);
            c.arrow_head_at(top, 0.2);
            let bottom = c.arc_points((0.5, 0.5), 0.32, 0.35, PI - 0.35);
            c.arrow_head_at(bottom, 0.2);
        }

        Icon::NavZoom => {
            c.circle((0.42, 0.42), 0.27);
            let thick = Stroke::new(c.stroke.width * 1.8, color);
            c.polyline_stroke(&[(0.62, 0.62), (0.9, 0.9)], false, thick);
        }

        Icon::Check => {
            let thick = Stroke::new(c.stroke.width * 1.4, color);
            c.polyline_stroke(&[(0.18, 0.52), (0.42, 0.76), (0.84, 0.26)], false, thick);
        }
        Icon::Cross => {
            let thick = Stroke::new(c.stroke.width * 1.4, color);
            c.polyline_stroke(&[(0.24, 0.24), (0.76, 0.76)], false, thick);
            c.polyline_stroke(&[(0.76, 0.24), (0.24, 0.76)], false, thick);
        }
        Icon::FoldRight => c.fill(&[(0.34, 0.22), (0.34, 0.78), (0.74, 0.5)]),
        Icon::FoldDown => c.fill(&[(0.22, 0.34), (0.78, 0.34), (0.5, 0.74)]),
    }
}

fn paint_object(c: &Canvas<'_>, tag: ObjectKindTag, soft: Color32) {
    match tag {
        ObjectKindTag::Cube => {
            c.fill_with(&[(0.5, 0.08), (0.9, 0.29), (0.5, 0.5), (0.1, 0.29)], soft);
            c.polyline(
                &[
                    (0.5, 0.08),
                    (0.9, 0.29),
                    (0.9, 0.71),
                    (0.5, 0.92),
                    (0.1, 0.71),
                    (0.1, 0.29),
                ],
                true,
            );
            c.line((0.1, 0.29), (0.5, 0.5));
            c.line((0.9, 0.29), (0.5, 0.5));
            c.line((0.5, 0.5), (0.5, 0.92));
        }
        ObjectKindTag::Sphere => {
            c.disc_with((0.5, 0.5), 0.41, soft);
            c.circle((0.5, 0.5), 0.41);
            c.ellipse((0.5, 0.5), (0.41, 0.14));
            c.ellipse((0.5, 0.5), (0.14, 0.41));
        }
        ObjectKindTag::Cylinder => {
            c.painter
                .add(Shape::ellipse_filled(c.p(0.5, 0.24), c.v(0.34, 0.11), soft));
            c.ellipse((0.5, 0.24), (0.34, 0.11));
            c.line((0.16, 0.24), (0.16, 0.76));
            c.line((0.84, 0.24), (0.84, 0.76));
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = PI * i as f32 / 16.0;
                    c.p(0.5 + 0.34 * a.cos(), 0.76 + 0.11 * a.sin())
                })
                .collect();
            c.painter.add(Shape::line(pts, c.stroke));
        }
        ObjectKindTag::Plane => {
            let quad = [(0.28, 0.28), (0.96, 0.28), (0.72, 0.74), (0.04, 0.74)];
            c.fill_with(&quad, soft);
            c.polyline(&quad, true);
        }
        ObjectKindTag::Cone => {
            // Apex plus the front half of the base ellipse: a convex outline.
            let mut outline: Vec<P> = vec![(0.5, 0.1)];
            outline.extend((0..=16).map(|i| {
                let a = PI * i as f32 / 16.0;
                (0.5 + 0.36 * a.cos(), 0.78 + 0.11 * a.sin())
            }));
            c.fill_with(&outline, soft);
            c.ellipse((0.5, 0.78), (0.36, 0.11));
            c.line((0.5, 0.1), (0.14, 0.78));
            c.line((0.5, 0.1), (0.86, 0.78));
        }
        ObjectKindTag::Torus => {
            c.painter
                .add(Shape::ellipse_filled(c.p(0.5, 0.52), c.v(0.43, 0.27), soft));
            c.ellipse((0.5, 0.52), (0.43, 0.27));
            c.ellipse((0.5, 0.47), (0.17, 0.08));
        }
        ObjectKindTag::Disc => {
            c.painter
                .add(Shape::ellipse_filled(c.p(0.5, 0.55), c.v(0.43, 0.2), soft));
            c.ellipse((0.5, 0.55), (0.43, 0.2));
            c.disc((0.5, 0.55), 0.05);
        }
        ObjectKindTag::Capsule => {
            let (r, top, bottom) = (0.2, 0.28, 0.72);
            // Upper half circle left to right, then the lower one right to left.
            let mut outline: Vec<P> = (0..=12)
                .map(|i| {
                    let a = PI + PI * i as f32 / 12.0;
                    (0.5 + r * a.cos(), top + r * a.sin())
                })
                .collect();
            outline.extend((0..=12).map(|i| {
                let a = PI * i as f32 / 12.0;
                (0.5 + r * a.cos(), bottom + r * a.sin())
            }));
            c.fill_with(&outline, soft);
            c.polyline(&outline, true);
            c.ellipse((0.5, 0.5), (0.2, 0.06));
        }
        ObjectKindTag::Null => {
            c.circle((0.5, 0.5), 0.2);
            c.line((0.5, 0.06), (0.5, 0.3));
            c.line((0.5, 0.7), (0.5, 0.94));
            c.line((0.06, 0.5), (0.3, 0.5));
            c.line((0.7, 0.5), (0.94, 0.5));
        }
        ObjectKindTag::Camera => {
            c.rect_fill((0.06, 0.34), (0.66, 0.78), soft);
            c.rect_stroke((0.06, 0.34), (0.66, 0.78), c.color);
            c.fill(&[(0.66, 0.56), (0.95, 0.36), (0.95, 0.76)]);
            c.circle((0.22, 0.2), 0.1);
            c.circle((0.48, 0.2), 0.1);
        }
        ObjectKindTag::Polygon => {
            // A triangle split into four: a mesh of polygons.
            let outline = [(0.5, 0.1), (0.92, 0.86), (0.08, 0.86)];
            c.fill_with(&outline, soft);
            c.polyline(&outline, true);
            c.polyline(&[(0.71, 0.48), (0.5, 0.86), (0.29, 0.48)], true);
        }
        ObjectKindTag::Light => {
            c.disc((0.5, 0.5), 0.19);
            for i in 0..8 {
                let a = TAU * i as f32 / 8.0;
                let (s, co) = a.sin_cos();
                c.line((0.5 + co * 0.29, 0.5 + s * 0.29), (0.5 + co * 0.44, 0.5 + s * 0.44));
            }
        }
    }
}

/// Maps unit-square coordinates into the icon rect.
struct Canvas<'a> {
    painter: &'a Painter,
    rect: Rect,
    color: Color32,
    stroke: Stroke,
}

type P = (f32, f32);

impl Canvas<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.rect.min + vec2(x * self.rect.width(), y * self.rect.height())
    }

    fn v(&self, x: f32, y: f32) -> Vec2 {
        vec2(x * self.rect.width(), y * self.rect.height())
    }

    fn pts(&self, points: &[P]) -> Vec<Pos2> {
        points.iter().map(|&(x, y)| self.p(x, y)).collect()
    }

    fn line(&self, a: P, b: P) {
        self.painter
            .line_segment([self.p(a.0, a.1), self.p(b.0, b.1)], self.stroke);
    }

    fn polyline(&self, points: &[P], closed: bool) {
        self.polyline_stroke(points, closed, self.stroke);
    }

    fn polyline_stroke(&self, points: &[P], closed: bool, stroke: Stroke) {
        let pts = self.pts(points);
        self.painter.add(if closed {
            Shape::closed_line(pts, stroke)
        } else {
            Shape::line(pts, stroke)
        });
    }

    /// Filled convex polygon in the icon colour.
    fn fill(&self, points: &[P]) {
        self.fill_with(points, self.color);
    }

    fn fill_with(&self, points: &[P], color: Color32) {
        self.painter
            .add(Shape::convex_polygon(self.pts(points), color, Stroke::NONE));
    }

    fn circle(&self, center: P, r: f32) {
        self.painter
            .circle_stroke(self.p(center.0, center.1), r * self.rect.width(), self.stroke);
    }

    fn disc(&self, center: P, r: f32) {
        self.disc_with(center, r, self.color);
    }

    fn disc_with(&self, center: P, r: f32, color: Color32) {
        self.painter
            .circle_filled(self.p(center.0, center.1), r * self.rect.width(), color);
    }

    fn ellipse(&self, center: P, radius: P) {
        let thin = Stroke::new(self.stroke.width * 0.8, self.stroke.color);
        self.painter.add(Shape::ellipse_stroke(
            self.p(center.0, center.1),
            self.v(radius.0, radius.1),
            thin,
        ));
    }

    fn rect_fill(&self, min: P, max: P, color: Color32) {
        let r = Rect::from_min_max(self.p(min.0, min.1), self.p(max.0, max.1));
        self.painter.rect_filled(r, CornerRadius::same(1), color);
    }

    fn rect_stroke(&self, min: P, max: P, color: Color32) {
        let r = Rect::from_min_max(self.p(min.0, min.1), self.p(max.0, max.1));
        self.painter.rect_stroke(
            r,
            CornerRadius::same(1),
            Stroke::new(self.stroke.width, color),
            StrokeKind::Inside,
        );
    }

    /// Arc from angle `a0` to `a1` (radians, screen space: y points down).
    /// Returns the end point and the direction of travel there.
    fn arc_points(&self, center: P, r: f32, a0: f32, a1: f32) -> (Pos2, Vec2) {
        const SEGMENTS: usize = 24;
        let pts: Vec<Pos2> = (0..=SEGMENTS)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / SEGMENTS as f32;
                self.p(center.0 + r * a.cos(), center.1 + r * a.sin())
            })
            .collect();
        let end = *pts.last().unwrap_or(&self.rect.center());
        let sign = (a1 - a0).signum();
        let dir = vec2(-a1.sin(), a1.cos()) * sign;
        self.painter.add(Shape::line(pts, self.stroke));
        (end, dir)
    }

    /// Filled arrow head with its tip at `tip` (unit coords), pointing along `dir`.
    fn arrow_head(&self, tip: P, dir: P, size: f32) {
        self.arrow_head_at((self.p(tip.0, tip.1), vec2(dir.0, dir.1)), size);
    }

    fn arrow_head_at(&self, (tip, dir): (Pos2, Vec2), size: f32) {
        let d = dir.normalized() * size * self.rect.width();
        let n = d.rot90() * 0.6;
        let base = tip - d;
        self.painter.add(Shape::convex_polygon(
            vec![tip, base + n, base - n],
            self.color,
            Stroke::NONE,
        ));
    }
}

/// Paints an icon into a `size × size` square centred on `center`.
pub fn paint_centered(painter: &Painter, center: Pos2, size: f32, icon: Icon, color: Color32) {
    paint(painter, Rect::from_center_size(center, Vec2::splat(size)), icon, color);
}
