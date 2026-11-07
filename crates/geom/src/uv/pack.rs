//! Packing islands into the UV square (see
//! `docs/reference/manager-uv-editor.md`): every island keeps its shape and
//! all are scaled alike; their boxes are laid in rows, tallest first, with a
//! margin between them, and the whole fitted into 0 – 1.

use glam::Vec2;

/// How [`pack`] lays islands out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackOptions {
    /// The gap around each island, in UV units (4 pixels of a 1024 picture:
    /// 4 / 1024).
    pub margin: f32,
    /// Islands taller than wide are turned a quarter, so rows fill better.
    pub rotate: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            margin: 4.0 / 1024.0,
            rotate: true,
        }
    }
}

/// An island's box: its size and where its corners' UVs start.
#[derive(Clone, Copy, Debug)]
struct Island {
    min: Vec2,
    size: Vec2,
    turned: bool,
}

/// Lays the islands `islands` (each a list of corners of `uvs`) into the UV
/// square without overlap: each moved (and turned a quarter when
/// `options.rotate` and taller than wide), all scaled alike so the layout
/// fits 0 – 1 with `options.margin` around each island.
pub fn pack(uvs: &mut [Vec2], islands: &[Vec<usize>], options: &PackOptions) {
    let mut boxes: Vec<(usize, Island)> = Vec::with_capacity(islands.len());
    for (i, corners) in islands.iter().enumerate() {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &c in corners {
            if let Some(&uv) = uvs.get(c) {
                min = min.min(uv);
                max = max.max(uv);
            }
        }
        if !(min.is_finite() && max.is_finite()) {
            continue;
        }
        let size = max - min;
        let turned = options.rotate && size.y > size.x;
        boxes.push((
            i,
            Island {
                min,
                size: if turned { Vec2::new(size.y, size.x) } else { size },
                turned,
            },
        ));
    }
    if boxes.is_empty() {
        return;
    }
    // Tallest first, so rows waste little height.
    boxes.sort_by(|a, b| b.1.size.y.total_cmp(&a.1.size.y).then(a.0.cmp(&b.0)));
    let sizes: Vec<Vec2> = boxes.iter().map(|(_, b)| b.size).collect();
    let area: f32 = sizes.iter().map(|s| s.x * s.y).sum();
    let widest = sizes.iter().map(|s| s.x).fold(0.0, f32::max);

    // The margin is in the final units: the gap before scaling depends on the
    // scale, found by a few rounds.
    let mut best: Option<(f32, Vec<Vec2>)> = None;
    for factor in [1.0f32, 1.1, 1.2, 1.35, 1.5, 1.7, 2.0] {
        let width_for = |gap: f32| (area.sqrt() * factor).max(widest + 2.0 * gap);
        let mut gap = 0.0f32;
        for _ in 0..6 {
            gap = options.margin.max(0.0) * rows(&sizes, width_for(gap), gap).0;
        }
        let (extent, at) = rows(&sizes, width_for(gap), gap);
        if best.as_ref().is_none_or(|(e, _)| extent < *e) {
            best = Some((extent, at));
        }
    }
    let Some((extent, at)) = best else {
        return;
    };
    let scale = if extent > 0.0 { 1.0 / extent } else { 1.0 };
    for ((i, island), place) in boxes.iter().zip(at) {
        for &c in &islands[*i] {
            let Some(uv) = uvs.get_mut(c) else {
                continue;
            };
            let d = *uv - island.min;
            // A quarter turn (keeps the picture unmirrored): (u, v) → (v, w − u).
            let local = if island.turned {
                Vec2::new(d.y, island.size.y - d.x)
            } else {
                d
            };
            *uv = (place + local) * scale;
        }
    }
}

/// Boxes of `sizes` laid in rows `width` wide with `gap` around each: where
/// each goes, and the larger of the layout's width and height.
fn rows(sizes: &[Vec2], width: f32, gap: f32) -> (f32, Vec<Vec2>) {
    let mut at = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut row_height, mut used_width) = (gap, gap, 0.0f32, 0.0f32);
    for size in sizes {
        if x > gap && x + size.x + gap > width {
            y += row_height + gap;
            x = gap;
            row_height = 0.0;
        }
        at.push(Vec2::new(x, y));
        x += size.x + gap;
        row_height = row_height.max(size.y);
        used_width = used_width.max(x);
    }
    let height = y + row_height + gap;
    (used_width.max(height), at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PolyMesh, Topology, primitives,
        uv::{flipped_faces, islands::layout},
    };

    /// The UV boxes of `mesh`'s islands.
    fn boxes(mesh: &PolyMesh) -> Vec<(Vec2, Vec2)> {
        let topology = Topology::build(mesh);
        let uvs = mesh.uvs().unwrap();
        layout(mesh, &topology)
            .island_corners(mesh)
            .iter()
            .map(|corners| {
                let min = corners.iter().map(|&c| uvs[c]).fold(Vec2::INFINITY, Vec2::min);
                let max = corners.iter().map(|&c| uvs[c]).fold(Vec2::NEG_INFINITY, Vec2::max);
                (min, max)
            })
            .collect()
    }

    #[test]
    fn packed_islands_never_overlap_and_lie_in_the_square() {
        // A cube's six sides, made very uneven first: stretched and spread out.
        let mut cube = primitives::cube(glam::Vec3::splat(2.0), [1, 1, 1]);
        let topology = Topology::build(&cube);
        let layout = layout(&cube, &topology);
        let islands = layout.island_corners(&cube);
        let mut uvs = cube.uvs().unwrap().to_vec();
        for (i, corners) in islands.iter().enumerate() {
            let k = i as f32;
            for &c in corners {
                uvs[c] = uvs[c] * Vec2::new(1.0 + k, 0.5 + k * 0.3) + Vec2::new(k * 3.0, -k);
            }
        }
        let options = PackOptions {
            margin: 0.01,
            rotate: true,
        };
        pack(&mut uvs, &islands, &options);
        cube.set_uvs(Some(uvs.clone())).unwrap();
        assert!(
            uvs.iter()
                .all(|uv| (-1e-5..=1.00001).contains(&uv.x) && (-1e-5..=1.00001).contains(&uv.y))
        );
        assert_eq!(flipped_faces(&cube), 0, "turning never mirrors");
        let boxes = boxes(&cube);
        assert_eq!(boxes.len(), 6);
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let apart = a.1.x + 0.009 <= b.0.x
                    || b.1.x + 0.009 <= a.0.x
                    || a.1.y + 0.009 <= b.0.y
                    || b.1.y + 0.009 <= a.0.y;
                assert!(apart, "{a:?} and {b:?} overlap or touch");
            }
        }
        // The layout fills the square in one direction at least.
        let reach = boxes.iter().fold(Vec2::ZERO, |m, b| m.max(b.1));
        assert!(reach.x > 0.95 || reach.y > 0.95, "{reach}");
    }

    #[test]
    fn nothing_to_pack_changes_nothing() {
        let mut uvs = vec![Vec2::new(3.0, 4.0)];
        pack(&mut uvs, &[], &PackOptions::default());
        assert_eq!(uvs, [Vec2::new(3.0, 4.0)]);
        // A single point island lands inside the square.
        pack(&mut uvs, &[vec![0]], &PackOptions::default());
        assert!(uvs[0].x >= 0.0 && uvs[0].x <= 1.0);
    }
}
