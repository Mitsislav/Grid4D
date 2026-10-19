use std::{fmt, ops::Range};

use glam::{Vec2, Vec3};

use crate::{u32_index, us};

/// Why a mesh edit was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// A face refers to a point that does not exist.
    PointOutOfRange { point: u32, point_count: usize },
    /// A face needs at least three corners.
    TooFewCorners(usize),
    /// A face visits the same point twice.
    RepeatedPoint(u32),
    /// More points or corners than a `u32` index can address.
    TooLarge,
    /// UVs are per corner: their count must equal the number of corners.
    UvCountMismatch { uvs: usize, corners: usize },
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::PointOutOfRange { point, point_count } => {
                write!(f, "face refers to point {point}, but the mesh has {point_count} points")
            }
            Self::TooFewCorners(n) => write!(f, "a face needs at least 3 corners, got {n}"),
            Self::RepeatedPoint(p) => write!(f, "a face visits point {p} more than once"),
            Self::TooLarge => f.write_str("mesh exceeds the u32 index range"),
            Self::UvCountMismatch { uvs, corners } => {
                write!(f, "{uvs} UVs given for {corners} corners")
            }
        }
    }
}

impl std::error::Error for MeshError {}

/// A polygon mesh with n-gon faces, in face-vertex form.
///
/// Each face is a list of point indices ("corners"). All corners are stored in
/// one flat array; `face_starts[f]..face_starts[f + 1]` is face `f`'s range in it,
/// which is also how per-corner attributes (UVs) are indexed. Faces of any size
/// cost nothing extra — modelling tools (extrude, knife, bevel) create n-gons.
#[derive(Clone, Debug, PartialEq)]
pub struct PolyMesh {
    points: Vec<Vec3>,
    /// One entry per face plus a leading `0`: never empty.
    face_starts: Vec<u32>,
    face_points: Vec<u32>,
    /// One UV per corner, when the mesh has UVs.
    uvs: Option<Vec<Vec2>>,
}

impl Default for PolyMesh {
    fn default() -> Self {
        Self::new()
    }
}

impl PolyMesh {
    /// An empty mesh.
    #[must_use]
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            face_starts: vec![0],
            face_points: Vec::new(),
            uvs: None,
        }
    }

    /// Builds a mesh from points and faces, validating every face.
    pub fn from_faces<F: AsRef<[u32]>>(
        points: Vec<Vec3>,
        faces: impl IntoIterator<Item = F>,
    ) -> Result<Self, MeshError> {
        if u32::try_from(points.len()).is_err() {
            return Err(MeshError::TooLarge);
        }
        let mut mesh = Self { points, ..Self::new() };
        for face in faces {
            mesh.add_face(face.as_ref())?;
        }
        Ok(mesh)
    }

    /// Appends a point and returns its index.
    pub fn add_point(&mut self, point: Vec3) -> Result<u32, MeshError> {
        let index = u32::try_from(self.points.len()).map_err(|_| MeshError::TooLarge)?;
        if index == u32::MAX {
            return Err(MeshError::TooLarge);
        }
        self.points.push(point);
        Ok(index)
    }

    /// Appends a face (counter-clockwise from its front) and returns its index.
    /// If the mesh has UVs, the new corners get `(0, 0)`.
    pub fn add_face(&mut self, corners: &[u32]) -> Result<u32, MeshError> {
        if corners.len() < 3 {
            return Err(MeshError::TooFewCorners(corners.len()));
        }
        for (i, &point) in corners.iter().enumerate() {
            if us(point) >= self.points.len() {
                return Err(MeshError::PointOutOfRange {
                    point,
                    point_count: self.points.len(),
                });
            }
            if corners[..i].contains(&point) {
                return Err(MeshError::RepeatedPoint(point));
            }
        }
        let end = self
            .face_points
            .len()
            .checked_add(corners.len())
            .and_then(|end| u32::try_from(end).ok())
            .ok_or(MeshError::TooLarge)?;
        let face = u32_index(self.face_count());
        self.face_points.extend_from_slice(corners);
        self.face_starts.push(end);
        if let Some(uvs) = &mut self.uvs {
            uvs.resize(self.face_points.len(), Vec2::ZERO);
        }
        Ok(face)
    }

    /// Replaces the UVs (one per corner, in corner order) or removes them.
    pub fn set_uvs(&mut self, uvs: Option<Vec<Vec2>>) -> Result<(), MeshError> {
        if let Some(uvs) = &uvs
            && uvs.len() != self.face_points.len()
        {
            return Err(MeshError::UvCountMismatch {
                uvs: uvs.len(),
                corners: self.face_points.len(),
            });
        }
        self.uvs = uvs;
        Ok(())
    }

    #[must_use]
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// Moving points never changes the topology, so this cannot break the mesh.
    pub fn points_mut(&mut self) -> &mut [Vec3] {
        &mut self.points
    }

    /// Points corner `corner` at `point`. Crate-internal: the modelling operations
    /// swap a face's point for a new copy, which keeps every face's points distinct.
    pub(crate) fn set_corner_point(&mut self, corner: usize, point: u32) {
        debug_assert!(us(point) < self.points.len(), "point {point} out of range");
        self.face_points[corner] = point;
    }

    #[must_use]
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    #[must_use]
    pub fn face_count(&self) -> usize {
        self.face_starts.len() - 1
    }

    /// Total number of corners (face-vertices) over all faces.
    #[must_use]
    pub fn corner_count(&self) -> usize {
        self.face_points.len()
    }

    /// The range of face `face`'s corners in corner order (empty if out of range).
    #[must_use]
    pub fn corner_range(&self, face: usize) -> Range<usize> {
        match (self.face_starts.get(face), self.face_starts.get(face + 1)) {
            (Some(&start), Some(&end)) => us(start)..us(end),
            _ => 0..0,
        }
    }

    /// The point indices of face `face`, counter-clockwise (empty if out of range).
    #[must_use]
    pub fn face(&self, face: usize) -> &[u32] {
        &self.face_points[self.corner_range(face)]
    }

    /// Every face's point indices, in face order.
    pub fn faces(&self) -> impl ExactSizeIterator<Item = &[u32]> + '_ {
        self.face_starts
            .windows(2)
            .map(|w| &self.face_points[us(w[0])..us(w[1])])
    }

    /// All corners' point indices, in corner order.
    #[must_use]
    pub fn corner_points(&self) -> &[u32] {
        &self.face_points
    }

    /// One UV per corner, if the mesh has UVs.
    #[must_use]
    pub fn uvs(&self) -> Option<&[Vec2]> {
        self.uvs.as_deref()
    }

    /// Newell's vector of face `face`: perpendicular to the face, pointing to its
    /// front, with length twice the face's area. Robust for non-convex and slightly
    /// non-planar polygons, unlike a cross product of two edges.
    #[must_use]
    pub fn face_vector(&self, face: usize) -> Vec3 {
        newell(self.face(face).iter().map(|&p| self.points[us(p)]))
    }

    /// Unit normal of face `face`; zero for a degenerate (zero-area) face.
    #[must_use]
    pub fn face_normal(&self, face: usize) -> Vec3 {
        self.face_vector(face).normalize_or_zero()
    }

    #[must_use]
    pub fn face_area(&self, face: usize) -> f32 {
        self.face_vector(face).length() * 0.5
    }

    /// Average of the face's points.
    #[must_use]
    pub fn face_center(&self, face: usize) -> Vec3 {
        let corners = self.face(face);
        let sum: Vec3 = corners.iter().map(|&p| self.points[us(p)]).sum();
        sum / f32::from(u16::try_from(corners.len()).unwrap_or(u16::MAX).max(1))
    }

    /// The same points with only the faces `keep` accepts, in their order (UVs
    /// follow their corners). Point indices stay valid; unused points remain.
    #[must_use]
    pub fn with_faces_where(&self, keep: impl Fn(usize) -> bool) -> Self {
        let mut mesh = Self {
            points: self.points.clone(),
            uvs: self.uvs.as_ref().map(|_| Vec::new()),
            ..Self::new()
        };
        for face in (0..self.face_count()).filter(|&f| keep(f)) {
            let range = self.corner_range(face);
            mesh.face_points.extend_from_slice(&self.face_points[range.clone()]);
            mesh.face_starts.push(u32_index(mesh.face_points.len()));
            if let (Some(kept), Some(uvs)) = (&mut mesh.uvs, &self.uvs) {
                kept.extend_from_slice(&uvs[range]);
            }
        }
        mesh
    }

    /// Axis-aligned bounds of all points, as `(min, max)`; `None` without points.
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.points.first()?;
        Some(
            self.points
                .iter()
                .fold((first, first), |(min, max), &p| (min.min(p), max.max(p))),
        )
    }
}

/// Newell's method over a closed polygon's points.
pub(crate) fn newell(mut points: impl Iterator<Item = Vec3>) -> Vec3 {
    let mut sum = Vec3::ZERO;
    let Some(first) = points.next() else { return Vec3::ZERO };
    let mut prev = first;
    for p in points.chain(std::iter::once(first)) {
        sum += prev.cross(p);
        prev = p;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    #[test]
    fn faces_are_validated() {
        let points = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        assert_eq!(
            PolyMesh::from_faces(points.clone(), [[0u32, 1].as_slice()]),
            Err(MeshError::TooFewCorners(2))
        );
        assert_eq!(
            PolyMesh::from_faces(points.clone(), [[0u32, 1, 3]]),
            Err(MeshError::PointOutOfRange {
                point: 3,
                point_count: 3
            })
        );
        assert_eq!(
            PolyMesh::from_faces(points.clone(), [[0u32, 1, 0]]),
            Err(MeshError::RepeatedPoint(0))
        );
        let mesh = PolyMesh::from_faces(points, [[0u32, 1, 2]]).unwrap();
        assert_eq!((mesh.point_count(), mesh.face_count(), mesh.corner_count()), (3, 1, 3));
    }

    #[test]
    fn faces_of_any_size_are_stored_flat() {
        let mesh = shapes::cylinder(7);
        assert_eq!(mesh.face_count(), 7 + 2);
        assert_eq!(mesh.face(7).len(), 7, "a cap is one 7-gon");
        assert_eq!(mesh.corner_count(), 7 * 4 + 2 * 7);
        let total: usize = mesh.faces().map(<[u32]>::len).sum();
        assert_eq!(total, mesh.corner_count());
        assert!(mesh.face(99).is_empty());
    }

    #[test]
    fn face_vectors_measure_area_and_orientation() {
        let cube = shapes::cube(2.0);
        for f in 0..cube.face_count() {
            assert!((cube.face_area(f) - 4.0).abs() < 1e-5);
            let outward = cube.face_center(f).normalize();
            assert!(
                cube.face_normal(f).abs_diff_eq(outward, 1e-5),
                "face {f} points inwards"
            );
        }
        // Non-convex: an L made of three unit squares.
        let l = shapes::l_shape();
        assert!((l.face_area(0) - 3.0).abs() < 1e-5);
        assert!(l.face_normal(0).abs_diff_eq(Vec3::Z, 1e-6));
        assert_eq!(cube.bounds(), Some((Vec3::splat(-1.0), Vec3::splat(1.0))));
    }

    #[test]
    fn keeping_some_faces_keeps_points_and_uvs() {
        let mut mesh = shapes::grid(2, 1);
        let uvs: Vec<Vec2> = (0..mesh.corner_count())
            .map(|c| Vec2::splat(f32::from(u16::try_from(c).unwrap())))
            .collect();
        mesh.set_uvs(Some(uvs)).unwrap();
        let kept = mesh.with_faces_where(|f| f == 1);
        assert_eq!(kept.points(), mesh.points(), "indices stay valid");
        assert_eq!(kept.face_count(), 1);
        assert_eq!(kept.face(0), mesh.face(1));
        assert_eq!(kept.uvs().unwrap(), &mesh.uvs().unwrap()[mesh.corner_range(1)]);
        assert_eq!(mesh.with_faces_where(|_| true), mesh);
        assert_eq!(mesh.with_faces_where(|_| false).face_count(), 0);
    }

    #[test]
    fn uvs_follow_corners() {
        let mut mesh = shapes::grid(1, 1);
        assert_eq!(
            mesh.set_uvs(Some(vec![Vec2::ZERO; 3])),
            Err(MeshError::UvCountMismatch { uvs: 3, corners: 4 })
        );
        mesh.set_uvs(Some(vec![Vec2::ONE; 4])).unwrap();
        let p = mesh.add_point(Vec3::new(0.5, 0.0, -1.0)).unwrap();
        mesh.add_face(&[0, 1, p]).unwrap();
        let uvs = mesh.uvs().unwrap();
        assert_eq!(uvs.len(), 7);
        assert_eq!(uvs[4..], [Vec2::ZERO; 3], "new corners get zero UVs");
    }
}
