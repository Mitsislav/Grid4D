//! # grid4d-text
//!
//! Text splines (see `docs/reference/object-text.md`): the outlines of a
//! text's letters, read from a font, as one Bezier [`Spline`] with a closed
//! segment per contour (letters with holes have several).
//!
//! ## Architecture rule
//!
//! Depends on `grid4d-geom` (the spline types and [`Text`], plain data),
//! `ttf-parser` and the typeface egui already embeds in Grid4D; no scene, no
//! GPU, no UI. The scene stores only a text's parameters; evaluation turns them
//! into curves through [`text_spline`].

use std::sync::OnceLock;

use glam::{Vec2, Vec3};
use grid4d_geom::spline::{Segment, Spline, SplineType, Text, TextAlign};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// The typeface text splines are written in (the one egui uses for its
/// interface, already part of the program).
pub const FONT_NAME: &str = "Ubuntu Light";

/// The built-in font, parsed once.
fn face() -> Option<&'static Face<'static>> {
    static FACE: OnceLock<Option<Face<'static>>> = OnceLock::new();
    FACE.get_or_init(|| Face::parse(epaint_default_fonts::UBUNTU_LIGHT, 0).ok())
        .as_ref()
}

/// The outline of `text`, laid out line by line from the origin (the first
/// baseline on the X axis, lines running downwards), in its plane and
/// direction. Empty without letters that draw anything.
#[must_use]
pub fn text_spline(text: &Text) -> Spline {
    let mut out = Spline {
        kind: SplineType::Bezier,
        points: Vec::new(),
        tangents: Vec::new(),
        segments: Vec::new(),
    };
    let Some(face) = face() else {
        return out;
    };
    let scale = text.height.max(0.0) / f32::from(face.units_per_em());
    let line_height = (f32::from(face.ascender()) - f32::from(face.descender()) + f32::from(face.line_gap())) * scale
        + text.vertical_spacing;
    let mut placed: Vec<(Option<GlyphId>, f32)> = Vec::new();
    for (row, line) in text.text.split('\n').enumerate() {
        // Where each letter starts, and how wide the line is.
        placed.clear();
        let mut pen = 0.0;
        for (k, ch) in line.chars().filter(|&c| c != '\r').enumerate() {
            let ch = if ch == '\t' { ' ' } else { ch };
            let glyph = face.glyph_index(ch);
            if k > 0 {
                pen += text.horizontal_spacing;
            }
            placed.push((glyph, pen));
            let advance = face.glyph_hor_advance(glyph.unwrap_or(GlyphId(0))).unwrap_or(0);
            pen += f32::from(advance) * scale;
        }
        let shift = match text.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => -pen * 0.5,
            TextAlign::Right => -pen,
        };
        let baseline = -line_height * f32::from(u16::try_from(row).unwrap_or(u16::MAX));
        for &(glyph, x) in &placed {
            // Characters the font lacks draw nothing.
            let Some(glyph) = glyph else { continue };
            let mut builder = Builder {
                out: &mut out,
                offset: Vec2::new(shift + x, baseline),
                scale,
                start: None,
            };
            face.outline_glyph(glyph, &mut builder);
            builder.finish();
        }
    }
    let out = out.mapped(|p| text.plane.place(p));
    if text.reverse { out.reversed() } else { out }
}

/// Collects a glyph's contours into the spline as closed Bezier segments.
struct Builder<'a> {
    out: &'a mut Spline,
    offset: Vec2,
    scale: f32,
    /// The current contour's first point.
    start: Option<usize>,
}

impl Builder<'_> {
    fn at(&self, x: f32, y: f32) -> Vec3 {
        Vec3::new(self.offset.x + x * self.scale, self.offset.y + y * self.scale, 0.0)
    }

    fn push(&mut self, point: Vec3, incoming: Vec3) {
        self.out.points.push(point);
        self.out.tangents.push([incoming, Vec3::ZERO]);
    }

    /// Sets the outgoing tangent of the last point.
    fn leave(&mut self, outgoing: Vec3) {
        if let Some(last) = self.out.tangents.last_mut() {
            last[1] = outgoing;
        }
    }

    fn last(&self) -> Vec3 {
        self.out.points.last().copied().unwrap_or(Vec3::ZERO)
    }

    /// Ends the current contour: a closing point on top of the first one is
    /// merged into it; contours of fewer than two points are dropped.
    fn finish(&mut self) {
        let Some(start) = self.start.take() else { return };
        let mut count = self.out.points.len() - start;
        if count >= 2 && self.out.points[start].distance_squared(self.last()) < 1e-10 {
            if let Some([incoming, _]) = self.out.tangents.pop() {
                self.out.tangents[start][0] = incoming;
            }
            self.out.points.pop();
            count -= 1;
        }
        if count < 2 {
            self.out.points.truncate(start);
            self.out.tangents.truncate(start);
            return;
        }
        self.out.segments.push(Segment {
            count: u32::try_from(count).unwrap_or(u32::MAX),
            closed: true,
        });
    }
}

impl OutlineBuilder for Builder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        self.start = Some(self.out.points.len());
        let point = self.at(x, y);
        self.push(point, Vec3::ZERO);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let point = self.at(x, y);
        self.push(point, Vec3::ZERO);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // The cubic Bezier equal to the quadratic one.
        let (from, control, to) = (self.last(), self.at(x1, y1), self.at(x, y));
        self.leave((control - from) * (2.0 / 3.0));
        self.push(to, (control - to) * (2.0 / 3.0));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (from, to) = (self.last(), self.at(x, y));
        self.leave(self.at(x1, y1) - from);
        self.push(to, self.at(x2, y2) - to);
    }

    fn close(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use grid4d_geom::spline::{Interpolation, SplinePlane};

    use super::*;

    fn spline(text: &str) -> Spline {
        text_spline(&Text::new(text))
    }

    fn bounds(spline: &Spline) -> (Vec3, Vec3) {
        spline.sample(Interpolation::default()).bounds().unwrap()
    }

    #[test]
    fn letters_have_their_contours_and_spaces_none() {
        assert_eq!(spline("o").segments.len(), 2, "outline and hole");
        assert_eq!(spline("B").segments.len(), 3, "outline and two holes");
        assert_eq!(spline("l").segments.len(), 1);
        for empty in ["", " ", "\t \n "] {
            let s = spline(empty);
            assert!(s.points.is_empty() && s.segments.is_empty(), "{empty:?}");
        }
        let s = spline("Hello");
        assert!(s.segments.iter().all(|segment| segment.closed));
        assert_eq!(s.points.len(), s.tangents.len());
        let counted: u32 = s.segments.iter().map(|segment| segment.count).sum();
        assert_eq!(counted as usize, s.points.len());
    }

    #[test]
    fn height_lines_alignment_and_spacing() {
        let (min, max) = bounds(&spline("H"));
        assert!(
            min.x >= 0.0 && min.y.abs() < 1.0,
            "on the baseline, after the origin: {min}"
        );
        let big = text_spline(&Text {
            height: 400.0,
            ..Text::new("H")
        });
        let (_, big_max) = bounds(&big);
        assert!((big_max.y - max.y * 2.0).abs() < 0.1, "twice the size");

        let (two_min, _) = bounds(&spline("H\nH"));
        assert!(two_min.y < -max.y, "the second line is below");

        let right = bounds(&text_spline(&Text {
            align: TextAlign::Right,
            ..Text::new("HH")
        }));
        assert!(right.1.x <= 0.0 && right.0.x < 0.0, "ends at the origin: {right:?}");
        let centred = bounds(&text_spline(&Text {
            align: TextAlign::Center,
            ..Text::new("HH")
        }));
        assert!((centred.0.x + centred.1.x).abs() < 5.0, "centred: {centred:?}");

        let wide = bounds(&text_spline(&Text {
            horizontal_spacing: 50.0,
            ..Text::new("HHH")
        }));
        let narrow = bounds(&spline("HHH"));
        assert!((wide.1.x - narrow.1.x - 100.0).abs() < 1e-2, "two gaps of 50");
    }

    #[test]
    fn plane_and_reverse_like_other_splines() {
        let flat = text_spline(&Text {
            plane: SplinePlane::XZ,
            ..Text::new("H")
        });
        let (min, max) = bounds(&flat);
        assert!(min.y.abs() < 1e-4 && max.y.abs() < 1e-4, "lies in XZ");
        let forwards = spline("o");
        let backwards = text_spline(&Text {
            reverse: true,
            ..Text::new("o")
        });
        assert_eq!(backwards, forwards.reversed());
        assert_eq!(FONT_NAME, "Ubuntu Light");
    }
}
