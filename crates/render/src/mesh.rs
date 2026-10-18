//! Procedural unit meshes for the primitive objects.
//!
//! Every mesh is generated once at renderer start-up at unit size and scaled per
//! instance by the object's parameters, so editing a cube's size never rebuilds
//! geometry. Triangles are wound counter-clockwise when seen from outside.

use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::MeshKind;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    fn new(position: Vec3, normal: Vec3) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    fn next_index(&self) -> u32 {
        u32::try_from(self.vertices.len()).unwrap_or(u32::MAX)
    }
}

impl MeshKind {
    /// Builds the unit mesh for this primitive.
    #[must_use]
    pub fn build(self) -> MeshData {
        match self {
            Self::Cube => unit_cube(),
            Self::Sphere => unit_sphere(48, 24),
            Self::Cylinder => unit_cylinder(48),
            Self::Plane => unit_plane(),
        }
    }
}

/// Axis-aligned cube spanning `[-0.5, 0.5]³`, flat-shaded (24 vertices).
fn unit_cube() -> MeshData {
    // (normal, u, v) with u × v = normal, so (c-u-v, c+u-v, c+u+v) is CCW from outside.
    const FACES: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    let mut mesh = MeshData::default();
    for (n, u, v) in FACES {
        let base = mesh.next_index();
        let c = n * 0.5;
        for (su, sv) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            mesh.vertices.push(Vertex::new(c + u * su + v * sv, n));
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

/// UV sphere of radius 1, smooth-shaded.
fn unit_sphere(segments: u32, rings: u32) -> MeshData {
    let mut mesh = MeshData::default();
    for i in 0..=rings {
        let theta = PI * i as f32 / rings as f32;
        for j in 0..=segments {
            let phi = TAU * j as f32 / segments as f32;
            let p = Vec3::new(theta.sin() * phi.sin(), theta.cos(), theta.sin() * phi.cos());
            mesh.vertices.push(Vertex::new(p, p));
        }
    }
    let stride = segments + 1;
    for i in 0..rings {
        for j in 0..segments {
            let a = i * stride + j;
            let b = a + stride;
            let c = b + 1;
            let d = a + 1;
            // Skip the zero-area triangles that would touch a pole twice.
            if i != rings - 1 {
                mesh.indices.extend_from_slice(&[a, b, c]);
            }
            if i != 0 {
                mesh.indices.extend_from_slice(&[a, c, d]);
            }
        }
    }
    mesh
}

/// Capped cylinder: radius 1, height 1, centred on the origin, axis +Y.
fn unit_cylinder(segments: u32) -> MeshData {
    let mut mesh = MeshData::default();
    let ring = |j: u32| {
        let phi = TAU * j as f32 / segments as f32;
        Vec3::new(phi.sin(), 0.0, phi.cos())
    };

    // Side: duplicated seam vertex so normals stay smooth around the barrel.
    let side_base = mesh.next_index();
    for j in 0..=segments {
        let r = ring(j);
        mesh.vertices.push(Vertex::new(r + Vec3::Y * 0.5, r));
        mesh.vertices.push(Vertex::new(r - Vec3::Y * 0.5, r));
    }
    for j in 0..segments {
        let top = side_base + j * 2;
        let bottom = top + 1;
        let next_top = top + 2;
        let next_bottom = top + 3;
        mesh.indices
            .extend_from_slice(&[top, bottom, next_bottom, top, next_bottom, next_top]);
    }

    // Caps.
    for (y, normal) in [(0.5f32, Vec3::Y), (-0.5, Vec3::NEG_Y)] {
        let center = mesh.next_index();
        mesh.vertices.push(Vertex::new(Vec3::Y * y, normal));
        for j in 0..=segments {
            mesh.vertices.push(Vertex::new(ring(j) + Vec3::Y * y, normal));
        }
        for j in 0..segments {
            let (a, b) = (center + 1 + j, center + 2 + j);
            if y > 0.0 {
                mesh.indices.extend_from_slice(&[center, a, b]);
            } else {
                mesh.indices.extend_from_slice(&[center, b, a]);
            }
        }
    }
    mesh
}

/// Plane in XZ spanning `[-0.5, 0.5]²`, facing +Y.
fn unit_plane() -> MeshData {
    let n = Vec3::Y;
    MeshData {
        vertices: vec![
            Vertex::new(Vec3::new(-0.5, 0.0, -0.5), n),
            Vertex::new(Vec3::new(-0.5, 0.0, 0.5), n),
            Vertex::new(Vec3::new(0.5, 0.0, 0.5), n),
            Vertex::new(Vec3::new(0.5, 0.0, -0.5), n),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every non-degenerate triangle must face the same way as its vertex normals,
    /// i.e. be counter-clockwise when seen from outside.
    #[test]
    fn winding_matches_normals() {
        for kind in MeshKind::ALL {
            let mesh = kind.build();
            assert_eq!(mesh.indices.len() % 3, 0);
            for tri in mesh.indices.as_chunks::<3>().0 {
                let [a, b, c] = tri.map(|i| mesh.vertices[i as usize]);
                let (pa, pb, pc) = (Vec3::from(a.position), Vec3::from(b.position), Vec3::from(c.position));
                let face = (pb - pa).cross(pc - pa);
                assert!(face.length() > 1e-7, "{kind:?}: degenerate triangle {tri:?}");
                let avg_normal = Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal);
                assert!(
                    face.dot(avg_normal) > 0.0,
                    "{kind:?}: triangle {tri:?} is wound inside-out"
                );
            }
        }
    }

    #[test]
    fn indices_in_bounds() {
        for kind in MeshKind::ALL {
            let mesh = kind.build();
            let n = mesh.vertices.len() as u32;
            assert!(mesh.indices.iter().all(|&i| i < n), "{kind:?}");
        }
    }
}
