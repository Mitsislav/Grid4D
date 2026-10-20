//! glTF 2.0 import (`.gltf` with external or embedded buffers, `.glb`); see
//! `docs/reference/interchange-gltf.md`. Lengths are metres in glTF and
//! centimetres here (× 100); both are right-handed and Y-up.

use std::{collections::HashMap, fs, path::Path};

use glam::{Quat, Vec2, Vec3};
use gltf::{buffer::Source, camera::Projection, mesh::Mode};
use grid4d_geom::PolyMesh;
use grid4d_scene::{MeshHandle, NodeData, ObjectKind, Transform};

use crate::{
    IoError,
    import::{ObjectEntry, Objects},
};

/// glTF metres → Grid4D centimetres.
const UNIT: f32 = 100.0;

/// A glTF mesh, read once: its geometry and its first material's base colour;
/// `None` if it has no triangles.
type ReadMesh = Option<(MeshHandle, Option<[f32; 3]>)>;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Invalid(message.into())
}

/// The objects of a glTF file (`bytes`: a `.gltf` JSON or a `.glb` container).
/// External buffers are read relative to `base`, the file's directory.
///
/// # Errors
///
/// [`IoError::Invalid`] for broken or unsupported content, [`IoError::Io`] if an
/// external buffer cannot be read.
pub fn import_gltf(bytes: &[u8], base: Option<&Path>) -> Result<Objects, IoError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|err| invalid(format!("not a valid glTF file: {err}")))?;
    let buffers = load_buffers(&gltf, base)?;
    let mut meshes: Vec<Option<ReadMesh>> = vec![None; gltf.meshes().count()];
    let mut visited = vec![false; gltf.nodes().count()];
    let mut nodes = Vec::new();

    let roots: Vec<gltf::Node<'_>> = match gltf.default_scene().or_else(|| gltf.scenes().next()) {
        Some(scene) => scene.nodes().collect(),
        // No scene: every node that is nobody's child.
        None => {
            let mut is_child = vec![false; visited.len()];
            for node in gltf.nodes() {
                for child in node.children() {
                    if let Some(flag) = is_child.get_mut(child.index()) {
                        *flag = true;
                    }
                }
            }
            gltf.nodes()
                .filter(|n| !is_child.get(n.index()).copied().unwrap_or(false))
                .collect()
        }
    };
    let mut stack: Vec<(gltf::Node<'_>, Option<usize>)> = roots.into_iter().rev().map(|n| (n, None)).collect();
    while let Some((node, parent)) = stack.pop() {
        // A broken file may reference a node twice (or in a cycle): read it once.
        match visited.get_mut(node.index()) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        let index = nodes.len();
        let mut data = node_data(&node, &buffers, &mut meshes)?;
        data.transform = transform(&node)?;
        // A mesh node that is also a camera or light: the other one as a child.
        let extra = data
            .kind
            .tag()
            .eq(&grid4d_scene::ObjectKindTag::Polygon)
            .then(|| camera_or_light(&node))
            .flatten();
        nodes.push(ObjectEntry { parent, data });
        if let Some(kind) = extra {
            let name = kind.tag().label();
            nodes.push(ObjectEntry {
                parent: Some(index),
                data: NodeData::new(name, kind),
            });
        }
        // Children reversed on the stack, so they come out in order.
        let first = stack.len();
        stack.extend(node.children().map(|child| (child, Some(index))));
        stack[first..].reverse();
    }
    Ok(Objects { nodes })
}

/// Each buffer's bytes: the `.glb` binary chunk, a `data:` URI or a file next to the glTF.
fn load_buffers(gltf: &gltf::Gltf, base: Option<&Path>) -> Result<Vec<Vec<u8>>, IoError> {
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        let data = match buffer.source() {
            Source::Bin => gltf
                .blob
                .clone()
                .ok_or_else(|| invalid("the binary chunk of the .glb file is missing"))?,
            Source::Uri(uri) => load_uri(uri, base)?,
        };
        if data.len() < buffer.length() {
            return Err(invalid(format!(
                "buffer {} holds {} bytes but declares {}",
                buffer.index(),
                data.len(),
                buffer.length()
            )));
        }
        buffers.push(data);
    }
    Ok(buffers)
}

fn load_uri(uri: &str, base: Option<&Path>) -> Result<Vec<u8>, IoError> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, payload) = rest
            .split_once(";base64,")
            .ok_or_else(|| invalid("only base64 data URIs are supported"))?;
        return decode_base64(payload).ok_or_else(|| invalid("a data URI is not valid base64"));
    }
    let relative = percent_decode(uri);
    let path = Path::new(&relative);
    if relative.contains(':') || path.is_absolute() || relative.starts_with('/') || relative.starts_with('\\') {
        return Err(invalid(format!(
            "external buffer \"{uri}\" must be a path relative to the glTF file"
        )));
    }
    let base = base.ok_or_else(|| invalid("external buffers need the glTF file's folder"))?;
    fs::read(base.join(path)).map_err(IoError::Io)
}

/// `%20` and friends → bytes (URIs in glTF are percent-encoded).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(&h), Some(&l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
            && let Ok(byte) = u8::try_from(h * 16 + l)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Standard base64 (with or without padding); `None` on any other character.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let trimmed = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.chunks(4) {
        let mut acc = 0u32;
        for (k, &c) in chunk.iter().enumerate() {
            acc |= value(c)? << (18 - 6 * k);
        }
        let bytes = acc.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

/// What a node is: its mesh, else its camera or light, else a Null.
fn node_data(node: &gltf::Node<'_>, buffers: &[Vec<u8>], meshes: &mut [Option<ReadMesh>]) -> Result<NodeData, IoError> {
    let named = |kind: ObjectKind, fallback: Option<&str>| {
        let name = node
            .name()
            .filter(|n| !n.trim().is_empty())
            .or(fallback.filter(|n| !n.trim().is_empty()))
            .map_or_else(|| kind.tag().label().to_owned(), str::to_owned);
        NodeData::new(name, kind)
    };
    if let Some(mesh) = node.mesh() {
        let slot = meshes
            .get_mut(mesh.index())
            .ok_or_else(|| invalid("a node refers to a missing mesh"))?;
        if slot.is_none() {
            *slot = Some(read_mesh(&mesh, buffers)?);
        }
        if let Some(Some((handle, colour))) = slot {
            let mut data = named(ObjectKind::polygon(handle.clone()), mesh.name());
            if let Some(colour) = colour {
                data.display_color = *colour;
            }
            return Ok(data);
        }
    }
    Ok(match camera_or_light(node) {
        Some(kind) => named(kind, None),
        None => named(ObjectKind::Null, None),
    })
}

/// The node's camera or light as an object kind.
fn camera_or_light(node: &gltf::Node<'_>) -> Option<ObjectKind> {
    if let Some(camera) = node.camera() {
        let fov_y_deg = match camera.projection() {
            Projection::Perspective(p) => p.yfov().to_degrees(),
            Projection::Orthographic(_) => 40.0,
        };
        let fov_y_deg = if fov_y_deg.is_finite() {
            fov_y_deg.clamp(1.0, 179.0)
        } else {
            40.0
        };
        return Some(ObjectKind::Camera { fov_y_deg });
    }
    let light = node.light()?;
    let color = light.color();
    let intensity = light.intensity();
    let finite = color.iter().all(|c| c.is_finite()) && intensity.is_finite();
    Some(ObjectKind::Light {
        color: if finite { color } else { [1.0; 3] },
        intensity: if finite { intensity } else { 1.0 },
    })
}

/// The node's TRS (a matrix is decomposed), translation in centimetres.
fn transform(node: &gltf::Node<'_>) -> Result<Transform, IoError> {
    let (t, r, s) = node.transform().decomposed();
    let finite = t.iter().chain(&r).chain(&s).all(|v| v.is_finite());
    if !finite {
        return Err(invalid(format!("node {} has a non-finite transform", node.index())));
    }
    let mut transform = Transform {
        position: Vec3::from(t) * UNIT,
        scale: Vec3::from(s),
        ..Transform::default()
    };
    let rotation = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    if rotation.length_squared() > 0.0 {
        transform.set_rotation_quat(rotation);
    }
    Ok(transform)
}

/// One mesh: its triangle primitives merged into a polygon mesh with identical
/// positions welded, `TEXCOORD_0` per corner; the first material's base colour.
/// `None` if the mesh has no triangles.
fn read_mesh(mesh: &gltf::Mesh<'_>, buffers: &[Vec<u8>]) -> Result<ReadMesh, IoError> {
    let mut points: Vec<Vec3> = Vec::new();
    let mut welded: HashMap<[u32; 3], u32> = HashMap::new();
    let mut faces: Vec<[u32; 3]> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut any_uvs = false;
    let mut colour = None;

    for primitive in mesh.primitives() {
        let mode = primitive.mode();
        if !matches!(mode, Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan) {
            continue; // points and lines
        }
        let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
        let Some(positions) = reader.read_positions() else {
            continue;
        };
        let mut local: Vec<u32> = Vec::new();
        for p in positions {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(invalid(format!("mesh {} has a non-finite position", mesh.index())));
            }
            // `+ 0.0` turns -0.0 into 0.0: the same point, whatever the sign of a zero.
            let key = p.map(|v| (v + 0.0).to_bits());
            let next = u32::try_from(points.len()).map_err(|_| invalid("a mesh has too many points"))?;
            let index = *welded.entry(key).or_insert_with(|| {
                points.push(Vec3::from(p) * UNIT);
                next
            });
            local.push(index);
        }
        let texcoords: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|t| t.into_f32().collect());
        let indices: Vec<u32> = match reader.read_indices() {
            Some(indices) => indices.into_u32().collect(),
            None => (0..u32::try_from(local.len()).map_err(|_| invalid("a mesh has too many points"))?).collect(),
        };
        let triangles = triangles(mode, &indices);
        if colour.is_none() && primitive.material().index().is_some() {
            let [r, g, b, _] = primitive.material().pbr_metallic_roughness().base_color_factor();
            if [r, g, b].iter().all(|c| c.is_finite()) {
                colour = Some([r, g, b]);
            }
        }
        for corners in triangles {
            let lookup = |i: u32| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| local.get(i).copied())
                    .ok_or_else(|| invalid(format!("mesh {} has an index out of range", mesh.index())))
            };
            let face = [lookup(corners[0])?, lookup(corners[1])?, lookup(corners[2])?];
            if face[0] == face[1] || face[1] == face[2] || face[0] == face[2] {
                continue; // degenerate once welded
            }
            faces.push(face);
            for &corner in &corners {
                let uv = texcoords
                    .as_ref()
                    .and_then(|t| usize::try_from(corner).ok().and_then(|c| t.get(c)))
                    .copied();
                if let Some(uv) = uv {
                    any_uvs = true;
                    uvs.push(if uv.iter().all(|v| v.is_finite()) {
                        Vec2::from(uv)
                    } else {
                        Vec2::ZERO
                    });
                } else {
                    uvs.push(Vec2::ZERO);
                }
            }
        }
    }
    if faces.is_empty() {
        return Ok(None);
    }
    // (Points used only by dropped degenerate triangles stay as loose points.)
    let mut mesh_out =
        PolyMesh::from_faces(points, faces).map_err(|err| invalid(format!("mesh {}: {err}", mesh.index())))?;
    if any_uvs {
        mesh_out
            .set_uvs(Some(uvs))
            .map_err(|err| invalid(format!("mesh {}: {err}", mesh.index())))?;
    }
    Ok(Some((MeshHandle::new(mesh_out), colour)))
}

/// The triangles of a primitive (corner indices into its vertices).
fn triangles(mode: Mode, indices: &[u32]) -> Vec<[u32; 3]> {
    match mode {
        Mode::Triangles => indices.as_chunks::<3>().0.to_vec(),
        // Every other triangle is flipped, so all keep the same winding.
        Mode::TriangleStrip => indices
            .windows(3)
            .enumerate()
            .map(|(i, w)| {
                if i % 2 == 0 {
                    [w[0], w[1], w[2]]
                } else {
                    [w[0], w[2], w[1]]
                }
            })
            .collect(),
        Mode::TriangleFan => match indices.split_first() {
            Some((&first, rest)) => rest.windows(2).map(|w| [w[0], w[1], first]).collect(),
            None => Vec::new(),
        },
        Mode::Points | Mode::Lines | Mode::LineLoop | Mode::LineStrip => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use grid4d_geom::Topology;
    use grid4d_scene::ObjectKindTag;
    use serde_json::{Value, json};

    use super::*;

    fn b64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(char::from(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A 2 m cube with split vertices (4 per face, 24 in all), UVs and u16 indices:
    /// positions (288 bytes), UVs (192), indices (72).
    fn cube_buffer() -> Vec<u8> {
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.0, -1.0, 0.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        for (f, (n, u, v)) in faces.iter().enumerate() {
            let (n, u, v) = (Vec3::from(*n), Vec3::from(*u), Vec3::from(*v));
            // Counter-clockwise seen from outside: u × v = n.
            let (u, v) = if u.cross(v).dot(n) > 0.0 { (u, v) } else { (v, u) };
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(n + u * a + v * b);
                uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
            }
            let base = u16::try_from(f * 4).unwrap();
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let mut bytes = Vec::new();
        for p in &positions {
            for c in p.to_array() {
                bytes.extend(c.to_le_bytes());
            }
        }
        for uv in &uvs {
            for c in uv {
                bytes.extend(c.to_le_bytes());
            }
        }
        for i in &indices {
            bytes.extend(i.to_le_bytes());
        }
        bytes
    }

    /// The test scene: Root (a matrix) > Box (TRS, the cube), Box2 (the same mesh),
    /// Cam (a camera), Lamp (a point light), an unnamed empty node.
    fn document(buffer: Value) -> Value {
        json!({
            "asset": { "version": "2.0" },
            "extensionsUsed": ["KHR_lights_punctual"],
            "extensions": { "KHR_lights_punctual": { "lights": [
                { "type": "point", "color": [1.0, 0.5, 0.25], "intensity": 20.0 }
            ] } },
            "scene": 0,
            "scenes": [ { "nodes": [0] } ],
            "nodes": [
                { "name": "Root", "children": [1, 2, 3, 4, 5],
                  "matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 1,0,0,1] },
                { "name": "Box", "mesh": 0, "translation": [0.0, 2.0, 0.0],
                  "rotation": [0.0, 0.70710677, 0.0, 0.70710677], "scale": [1.0, 2.0, 1.0] },
                { "name": "Box2", "mesh": 0 },
                { "name": "Cam", "camera": 0 },
                { "name": "Lamp", "extensions": { "KHR_lights_punctual": { "light": 0 } } },
                { }
            ],
            "cameras": [ { "type": "perspective", "perspective": { "yfov": 0.7, "znear": 0.1 } } ],
            "materials": [ { "pbrMetallicRoughness": { "baseColorFactor": [0.8, 0.2, 0.1, 1.0] } } ],
            "meshes": [ { "name": "CubeMesh", "primitives": [ {
                "attributes": { "POSITION": 0, "TEXCOORD_0": 1 }, "indices": 2, "material": 0
            } ] } ],
            "buffers": [ buffer ],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 288 },
                { "buffer": 0, "byteOffset": 288, "byteLength": 192 },
                { "buffer": 0, "byteOffset": 480, "byteLength": 72 }
            ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3",
                  "min": [-1.0, -1.0, -1.0], "max": [1.0, 1.0, 1.0] },
                { "bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC2" },
                { "bufferView": 2, "componentType": 5123, "count": 36, "type": "SCALAR" }
            ]
        })
    }

    fn embedded() -> Vec<u8> {
        let data = cube_buffer();
        let uri = format!("data:application/octet-stream;base64,{}", b64(&data));
        serde_json::to_vec(&document(json!({ "byteLength": data.len(), "uri": uri }))).unwrap()
    }

    fn glb() -> Vec<u8> {
        let mut bin = cube_buffer();
        let mut json = serde_json::to_vec(&document(json!({ "byteLength": bin.len() }))).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend(u32::try_from(total).unwrap().to_le_bytes());
        out.extend(u32::try_from(json.len()).unwrap().to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(&json);
        out.extend(u32::try_from(bin.len()).unwrap().to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(&bin);
        out
    }

    fn names(imported: &Objects) -> Vec<(&str, Option<usize>, ObjectKindTag)> {
        imported
            .nodes
            .iter()
            .map(|n| (n.data.name.as_str(), n.parent, n.data.kind.tag()))
            .collect()
    }

    fn mesh_of(imported: &Objects, name: &str) -> MeshHandle {
        let node = imported.nodes.iter().find(|n| n.data.name == name).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("{name} is not a polygon object")
        };
        mesh.clone()
    }

    #[test]
    fn a_scene_with_hierarchy_mesh_camera_and_light_imports() {
        let imported = import_gltf(&embedded(), None).unwrap();
        assert_eq!(
            names(&imported),
            [
                ("Root", None, ObjectKindTag::Null),
                ("Box", Some(0), ObjectKindTag::Polygon),
                ("Box2", Some(0), ObjectKindTag::Polygon),
                ("Cam", Some(0), ObjectKindTag::Camera),
                ("Lamp", Some(0), ObjectKindTag::Light),
                ("Null", Some(0), ObjectKindTag::Null),
            ]
        );
        let node = |i: usize| &imported.nodes[i].data;
        assert!(
            node(0).transform.position.abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-4),
            "matrix, in cm"
        );
        let boxed = node(1).transform;
        assert!(boxed.position.abs_diff_eq(Vec3::new(0.0, 200.0, 0.0), 1e-4));
        assert!(boxed.scale.abs_diff_eq(Vec3::new(1.0, 2.0, 1.0), 1e-6));
        assert!(
            boxed.rotation.abs_diff_eq(Vec3::new(FRAC_PI_2, 0.0, 0.0), 1e-5),
            "{}",
            boxed.rotation
        );
        assert_eq!(node(1).display_color, [0.8, 0.2, 0.1], "the base colour");
        assert_eq!(
            node(3).kind,
            ObjectKind::Camera {
                fov_y_deg: 0.7f32.to_degrees()
            }
        );
        assert_eq!(
            node(4).kind,
            ObjectKind::Light {
                color: [1.0, 0.5, 0.25],
                intensity: 20.0
            }
        );
        assert_eq!(
            mesh_of(&imported, "Box").id(),
            mesh_of(&imported, "Box2").id(),
            "one shared mesh"
        );
    }

    #[test]
    fn split_vertices_are_welded_into_a_closed_mesh_in_centimetres() {
        let imported = import_gltf(&embedded(), None).unwrap();
        let handle = mesh_of(&imported, "Box");
        let mesh = handle.mesh();
        assert_eq!(
            (mesh.point_count(), mesh.face_count()),
            (8, 12),
            "24 split vertices → 8 points"
        );
        let topology = Topology::build(mesh);
        assert!(topology.is_closed() && topology.is_manifold());
        let (min, max) = mesh.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::splat(-100.0), 1e-4) && max.abs_diff_eq(Vec3::splat(100.0), 1e-4));
        let uvs = mesh.uvs().unwrap();
        assert_eq!(uvs.len(), 36, "per corner, so the seams survive the welding");
        assert!(
            uvs.iter()
                .all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y))
        );
        // Faces point outwards (counter-clockwise, as in glTF).
        for f in 0..mesh.face_count() {
            assert!(mesh.face_normal(f).dot(mesh.face_center(f)) > 0.0, "face {f}");
        }
    }

    #[test]
    fn glb_and_external_buffers_give_the_same_model() {
        let embedded = import_gltf(&embedded(), None).unwrap();
        let from_glb = import_gltf(&glb(), None).unwrap();
        assert_eq!(names(&from_glb), names(&embedded));
        assert_eq!(mesh_of(&from_glb, "Box").mesh(), mesh_of(&embedded, "Box").mesh());

        let dir = std::env::temp_dir().join(format!("grid4d-gltf-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let data = cube_buffer();
        fs::write(dir.join("cube data.bin"), &data).unwrap();
        let json =
            serde_json::to_vec(&document(json!({ "byteLength": data.len(), "uri": "cube%20data.bin" }))).unwrap();
        let path = dir.join("model.gltf");
        fs::write(&path, json).unwrap();
        let from_file = crate::import(&path).unwrap();
        assert_eq!(mesh_of(&from_file, "Box").mesh(), mesh_of(&embedded, "Box").mesh());

        // A missing external buffer is an I/O error.
        fs::remove_file(dir.join("cube data.bin")).unwrap();
        assert!(matches!(crate::import(&path), Err(IoError::Io(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        assert!(import_gltf(b"{ not json", None).is_err());
        assert!(import_gltf(b"glTF\x02\x00\x00\x00", None).is_err(), "truncated .glb");

        let data = cube_buffer();
        let short = format!("data:application/octet-stream;base64,{}", b64(&data[..400]));
        let json = serde_json::to_vec(&document(json!({ "byteLength": data.len(), "uri": short }))).unwrap();
        let err = import_gltf(&json, None).unwrap_err();
        assert!(err.to_string().contains("declares"), "{err}");

        for uri in ["C:/models/cube.bin", "/etc/cube.bin", "https://example.com/cube.bin"] {
            let json = serde_json::to_vec(&document(json!({ "byteLength": data.len(), "uri": uri }))).unwrap();
            assert!(import_gltf(&json, Some(Path::new("."))).is_err(), "{uri}");
        }

        let mut nan = cube_buffer();
        nan[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        let uri = format!("data:application/octet-stream;base64,{}", b64(&nan));
        let json = serde_json::to_vec(&document(json!({ "byteLength": nan.len(), "uri": uri }))).unwrap();
        let err = import_gltf(&json, None).unwrap_err();
        assert!(err.to_string().contains("non-finite"), "{err}");
    }

    #[test]
    fn a_node_cycle_is_cut_and_scenes_are_optional() {
        let json = serde_json::to_vec(&json!({
            "asset": { "version": "2.0" },
            "nodes": [ { "name": "A", "children": [1] }, { "name": "B" }, { "name": "C" } ]
        }))
        .unwrap();
        let imported = import_gltf(&json, None).unwrap();
        assert_eq!(
            names(&imported),
            [
                ("A", None, ObjectKindTag::Null),
                ("B", Some(0), ObjectKindTag::Null),
                ("C", None, ObjectKindTag::Null)
            ],
            "without a scene, the nodes that are nobody's children are the roots"
        );

        // A → B → A: rejected by validation or read once each, never an endless loop.
        let json = serde_json::to_vec(&json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [ { "nodes": [0] } ],
            "nodes": [ { "name": "A", "children": [1] }, { "name": "B", "children": [0] } ]
        }))
        .unwrap();
        match import_gltf(&json, None) {
            Ok(imported) => assert_eq!(imported.nodes.len(), 2),
            Err(err) => assert!(!err.to_string().is_empty()),
        }
    }

    #[test]
    fn zeros_of_either_sign_are_one_point() {
        // A fan of three triangles around a point written once as 0 and once as -0.
        let mut bin = Vec::new();
        let points: [[f32; 3]; 6] = [
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [-0.0, 1.0, -0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
        ];
        for p in points {
            for c in p {
                bin.extend(c.to_le_bytes());
            }
        }
        let uri = format!("data:application/octet-stream;base64,{}", b64(&bin));
        let json = serde_json::to_vec(&json!({
            "asset": { "version": "2.0" },
            "scene": 0, "scenes": [ { "nodes": [0] } ],
            "nodes": [ { "name": "Fan", "mesh": 0 } ],
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 } } ] } ],
            "buffers": [ { "byteLength": bin.len(), "uri": uri } ],
            "bufferViews": [ { "buffer": 0, "byteLength": bin.len() } ],
            "accessors": [ { "bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3",
                             "min": [-1.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0] } ]
        }))
        .unwrap();
        let imported = import_gltf(&json, None).unwrap();
        let mesh = mesh_of(&imported, "Fan");
        assert_eq!(mesh.mesh().point_count(), 4, "the apex once");
        assert!(
            Topology::build(mesh.mesh()).boundary_edge_count() < 6,
            "the triangles share an edge"
        );
    }

    #[test]
    fn strips_and_fans_become_triangles_with_one_winding() {
        assert_eq!(triangles(Mode::TriangleStrip, &[0, 1, 2, 3]), [[0, 1, 2], [1, 3, 2]]);
        assert_eq!(triangles(Mode::TriangleFan, &[0, 1, 2, 3]), [[1, 2, 0], [2, 3, 0]]);
        assert_eq!(triangles(Mode::Triangles, &[0, 1, 2, 3]), [[0, 1, 2]]);
        assert!(triangles(Mode::Lines, &[0, 1]).is_empty());
    }

    #[test]
    fn base64_and_percent_decoding() {
        assert_eq!(decode_base64("TWFu").unwrap(), b"Man");
        assert_eq!(decode_base64("TWE=").unwrap(), b"Ma");
        assert_eq!(decode_base64("TQ==").unwrap(), b"M");
        assert_eq!(decode_base64(&b64(&[0, 255, 7, 128, 1])).unwrap(), [0, 255, 7, 128, 1]);
        assert_eq!(decode_base64("TW!u"), None);
        assert_eq!(percent_decode("my%20cube%2Bv2.bin"), "my cube+v2.bin");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
