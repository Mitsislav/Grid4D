//! Texture coordinates (UVs): what primitives and generators make, and the
//! checks the UV tools rely on (see `docs/reference/tag-material.md` and
//! `manager-uv-editor.md`).
//!
//! ## Conventions
//!
//! A mesh holds one UV per corner ([`PolyMesh::uvs`]), so a point may sit in
//! several places of the picture (a seam). `u` runs to the right of the
//! picture and `v` **down** it (`v = 0` is the top row, as image files and
//! glTF store rows). A face seen from its front shows the picture unmirrored
//! when its UVs wind counter-clockwise in `(u, −v)`: [`winding`] is positive.
//! Projections keep the picture upright: "up" is world +Y, and for faces
//! looking up (down) it is −Z (+Z), so a top face reads like a plan seen from
//! above.

use glam::{Vec2, Vec3};

use crate::PolyMesh;

/// The picture's right and up directions on a flat face looking along
/// `normal` (seen from its front, `right × up = normal`); `None` for a zero
/// normal.
#[must_use]
pub fn planar_axes(normal: Vec3) -> Option<(Vec3, Vec3)> {
    let n = normal.try_normalize()?;
    let up = if n.y > 0.9 {
        Vec3::NEG_Z
    } else if n.y < -0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = up.cross(n).try_normalize()?;
    Some((right, n.cross(right)))
}

/// UVs for flat `polygons` (point indices into `points`) that face along
/// `normal`: projected onto their plane, upright, and scaled together so the
/// larger side of their common bounds spans 0 – 1 (no stretching). One list
/// per polygon, in its corners' order.
#[must_use]
pub fn planar(points: &[Vec3], polygons: &[Vec<u32>], normal: Vec3) -> Vec<Vec<Vec2>> {
    let Some((right, up)) = planar_axes(normal) else {
        return polygons.iter().map(|p| vec![Vec2::ZERO; p.len()]).collect();
    };
    let flat = |i: u32| {
        let p = points.get(crate::us(i)).copied().unwrap_or(Vec3::ZERO);
        Vec2::new(p.dot(right), p.dot(up))
    };
    let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for &i in polygons.iter().flatten() {
        let q = flat(i);
        min = min.min(q);
        max = max.max(q);
    }
    let extent = (max - min).max_element();
    let scale = if extent.is_finite() && extent > f32::EPSILON {
        extent.recip()
    } else {
        0.0
    };
    polygons
        .iter()
        .map(|polygon| {
            polygon
                .iter()
                .map(|&i| {
                    let q = flat(i);
                    Vec2::new((q.x - min.x) * scale, (max.y - q.y) * scale)
                })
                .collect()
        })
        .collect()
}

/// Box projection: each face takes the side of `mesh`'s bounding box its
/// normal faces most, and that side spans the whole picture (stretched to
/// it). One UV per corner. A cube gets the picture once on each side.
#[must_use]
pub fn box_uvs(mesh: &PolyMesh) -> Vec<Vec2> {
    let mut uvs = Vec::with_capacity(mesh.corner_count());
    let Some((min, max)) = mesh.bounds() else {
        return uvs;
    };
    let (centre, size) = ((min + max) * 0.5, max - min);
    for f in 0..mesh.face_count() {
        let normal = mesh.face_vector(f);
        let a = normal.abs();
        let side = if a.x >= a.y && a.x >= a.z {
            Vec3::X * normal.x.signum()
        } else if a.y >= a.z {
            Vec3::Y * normal.y.signum()
        } else {
            Vec3::Z * normal.z.signum()
        };
        let (right, up) = planar_axes(side).unwrap_or((Vec3::X, Vec3::Y));
        // The box's extent along an axis direction (axes are ±X, ±Y or ±Z).
        let extent = |axis: Vec3| {
            let e = size.dot(axis.abs());
            if e > f32::EPSILON { e.recip() } else { 0.0 }
        };
        let (sx, sy) = (extent(right), extent(up));
        for &p in mesh.face(f) {
            let q = mesh.points()[crate::us(p)] - centre;
            uvs.push(Vec2::new(0.5 + q.dot(right) * sx, 0.5 - q.dot(up) * sy));
        }
    }
    uvs
}

/// Gives `mesh` [`box_uvs`] if it has no UVs of its own.
pub fn ensure_uvs(mesh: &mut PolyMesh) {
    if mesh.uvs().is_none() {
        let uvs = box_uvs(mesh);
        // One per corner by construction; a mismatch leaves the mesh as it was.
        let _ = mesh.set_uvs(Some(uvs));
    }
}

/// Twice the signed area of the UV polygon `uvs` in `(u, −v)`: positive when
/// the picture shows unmirrored on the face's front (see the conventions).
#[must_use]
pub fn winding(uvs: &[Vec2]) -> f32 {
    let n = uvs.len();
    (0..n)
        .map(|i| {
            let (a, b) = (uvs[i], uvs[(i + 1) % n]);
            Vec2::new(a.x, -a.y).perp_dot(Vec2::new(b.x, -b.y))
        })
        .sum()
}

/// How many faces of `mesh` show the picture mirrored (their UVs wind the
/// wrong way); 0 without UVs. Faces whose UVs have no area count as fine.
#[must_use]
pub fn flipped_faces(mesh: &PolyMesh) -> usize {
    let Some(uvs) = mesh.uvs() else {
        return 0;
    };
    (0..mesh.face_count())
        .filter(|&f| winding(&uvs[mesh.corner_range(f)]) < -1e-7)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    #[test]
    fn axes_keep_the_picture_upright_and_unmirrored() {
        for normal in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z, Vec3::ONE] {
            let (right, up) = planar_axes(normal).unwrap();
            assert!(right.cross(up).abs_diff_eq(normal.normalize(), 1e-6), "{normal}");
        }
        assert_eq!(
            planar_axes(Vec3::Z),
            Some((Vec3::X, Vec3::Y)),
            "a front face: X right, Y up"
        );
        assert_eq!(planar_axes(Vec3::Y), Some((Vec3::X, Vec3::NEG_Z)), "a top face: a plan");
        assert_eq!(planar_axes(Vec3::ZERO), None);
    }

    #[test]
    fn planar_uvs_fill_the_square_without_stretching() {
        // A 4 × 2 rectangle facing +Z.
        let points = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(4.0, 2.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
        ];
        let uvs = planar(&points, &[vec![0, 1, 2, 3]], Vec3::Z);
        assert_eq!(
            uvs[0],
            [
                Vec2::new(0.0, 0.5),
                Vec2::new(1.0, 0.5),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 0.0)
            ]
        );
        assert!(winding(&uvs[0]) > 0.0);
        assert!(
            planar(&points, &[vec![0, 1, 2]], Vec3::ZERO)[0]
                .iter()
                .all(|uv| *uv == Vec2::ZERO)
        );
    }

    #[test]
    fn a_box_projection_puts_the_whole_picture_on_each_side_of_a_cube() {
        let mut cube = shapes::cube(2.0);
        ensure_uvs(&mut cube);
        let uvs = cube.uvs().unwrap();
        assert_eq!(uvs.len(), cube.corner_count());
        for f in 0..cube.face_count() {
            let face = &uvs[cube.corner_range(f)];
            let (min, max) = face.iter().fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(a, b), &uv| {
                (a.min(uv), b.max(uv))
            });
            assert!(
                min.abs_diff_eq(Vec2::ZERO, 1e-6) && max.abs_diff_eq(Vec2::ONE, 1e-6),
                "face {f}"
            );
        }
        assert_eq!(flipped_faces(&cube), 0);

        // Existing UVs are kept.
        let before = cube.uvs().unwrap().to_vec();
        ensure_uvs(&mut cube);
        assert_eq!(cube.uvs().unwrap(), before);
    }

    #[test]
    fn mirrored_uvs_are_found() {
        let mut quad = PolyMesh::from_faces(
            vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
            [[0u32, 1, 2, 3]],
        )
        .unwrap();
        assert_eq!(flipped_faces(&quad), 0, "no UVs, nothing flipped");
        ensure_uvs(&mut quad);
        assert_eq!(flipped_faces(&quad), 0);
        let mirrored: Vec<Vec2> = quad
            .uvs()
            .unwrap()
            .iter()
            .map(|uv| Vec2::new(1.0 - uv.x, uv.y))
            .collect();
        quad.set_uvs(Some(mirrored)).unwrap();
        assert_eq!(flipped_faces(&quad), 1);
    }
}
