//! glTF 2.0 export (`.glb`, or `.gltf` with a `.bin` next to it); see
//! `docs/reference/interchange-gltf.md`. Lengths are centimetres here and metres
//! in glTF (÷ 100); both are right-handed and Y-up, so nothing flips.
//!
//! The objects come in as [`Objects`] whose meshes are already baked (the caller
//! turns primitives and generators into polygon objects); anything else without
//! geometry is written as an empty node.

use std::{collections::HashMap, path::Path};

use glam::{Quat, Vec3};
use grid4d_geom::{Smoothing, TriMesh, Triangulator};
use grid4d_scene::{MeshHandle, MeshId, ObjectKind};
use serde_json::{Map, Value, json};

use crate::{IoError, import::Objects, write_atomic};

/// Grid4D centimetres → glTF metres.
const TO_METRES: f32 = 0.01;
/// glTF component types and buffer view targets.
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// `.glb` chunk types.
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Invalid(message.into())
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}

/// Writes `objects` to `path`: a `.glb`, or a `.gltf` plus `<stem>.bin` next to it
/// (by the extension; anything else is an error).
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers or an unknown extension,
/// [`IoError::Io`] if a file cannot be written.
pub fn export_gltf(objects: &Objects, path: &Path) -> Result<(), IoError> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    if extension.eq_ignore_ascii_case("glb") {
        return write_atomic(path, &glb_bytes(objects)?);
    }
    if !extension.eq_ignore_ascii_case("gltf") {
        return Err(invalid("export as .glb or .gltf"));
    }
    let stem = path.file_stem().map_or_else(|| "scene".into(), |s| s.to_string_lossy());
    let bin_name = format!("{stem}.bin");
    let (json, bin) = gltf_document(objects, Some(&percent_encode(&bin_name)))?;
    if !bin.is_empty() {
        write_atomic(&path.with_file_name(&bin_name), &bin)?;
    }
    write_atomic(path, &json)
}

/// A `.glb` file: the JSON and binary chunks in one container.
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers.
pub fn glb_bytes(objects: &Objects) -> Result<Vec<u8>, IoError> {
    let (mut json, mut bin) = gltf_document(objects, None)?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let chunk = |len: usize| u32::try_from(len).map_err(|_| invalid("the scene is too large for a .glb file"));
    let bin_part = if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = chunk(12 + 8 + json.len() + bin_part)?;
    let mut out = Vec::with_capacity(12 + 8 + json.len() + bin_part);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend(total.to_le_bytes());
    out.extend(chunk(json.len())?.to_le_bytes());
    out.extend(CHUNK_JSON.to_le_bytes());
    out.extend(&json);
    if !bin.is_empty() {
        out.extend(chunk(bin.len())?.to_le_bytes());
        out.extend(CHUNK_BIN.to_le_bytes());
        out.extend(&bin);
    }
    Ok(out)
}

/// The glTF JSON and its binary buffer. `buffer_uri` names the `.bin` file the
/// JSON refers to (`None` for a `.glb`, whose buffer is its binary chunk).
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers.
pub fn gltf_document(objects: &Objects, buffer_uri: Option<&str>) -> Result<(Vec<u8>, Vec<u8>), IoError> {
    let mut writer = Writer::default();
    let mut nodes: Vec<Map<String, Value>> = Vec::with_capacity(objects.nodes.len());
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); objects.nodes.len()];
    let mut roots = Vec::new();
    let mut cameras = Vec::new();
    let mut lights = Vec::new();

    for (index, entry) in objects.nodes.iter().enumerate() {
        match entry.parent.filter(|&p| p < index) {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
        let data = &entry.data;
        let mut node = Map::new();
        node.insert("name".into(), json!(data.name));
        let t = &data.transform;
        let translation = t.position * TO_METRES;
        let rotation = t.rotation_quat();
        if !finite(&translation.to_array()) || !finite(&rotation.to_array()) || !finite(&t.scale.to_array()) {
            return Err(invalid(format!("\"{}\" has a non-finite transform", data.name)));
        }
        if translation != Vec3::ZERO {
            node.insert("translation".into(), json!(translation.to_array()));
        }
        if !rotation.abs_diff_eq(Quat::IDENTITY, 1e-7) {
            node.insert("rotation".into(), json!(rotation.normalize().to_array()));
        }
        if t.scale != Vec3::ONE {
            node.insert("scale".into(), json!(t.scale.to_array()));
        }
        match &data.kind {
            ObjectKind::Polygon { mesh, .. } => {
                if let Some(index) = writer.mesh(mesh, data.display_color, &data.name)? {
                    node.insert("mesh".into(), json!(index));
                }
            }
            ObjectKind::Camera { fov_y_deg } => {
                let yfov = fov_y_deg.to_radians();
                if !yfov.is_finite() || yfov <= 0.0 {
                    return Err(invalid(format!(
                        "camera \"{}\" has an invalid field of view",
                        data.name
                    )));
                }
                node.insert("camera".into(), json!(cameras.len()));
                cameras.push(json!({ "type": "perspective", "perspective": { "yfov": yfov, "znear": 0.01 } }));
            }
            ObjectKind::Light { color, intensity } => {
                if !finite(color) || !intensity.is_finite() {
                    return Err(invalid(format!("light \"{}\" has a non-finite colour", data.name)));
                }
                node.insert(
                    "extensions".into(),
                    json!({ "KHR_lights_punctual": { "light": lights.len() } }),
                );
                lights.push(json!({ "type": "point", "color": color, "intensity": intensity }));
            }
            // Baked by the caller; whatever is left has no geometry here.
            ObjectKind::Null
            | ObjectKind::Cube { .. }
            | ObjectKind::Sphere { .. }
            | ObjectKind::Cylinder { .. }
            | ObjectKind::Plane { .. }
            | ObjectKind::Cone { .. }
            | ObjectKind::Torus { .. }
            | ObjectKind::Disc { .. }
            | ObjectKind::Capsule { .. }
            | ObjectKind::SubdivisionSurface { .. } => {}
        }
        nodes.push(node);
    }
    for (node, kids) in nodes.iter_mut().zip(children) {
        if !kids.is_empty() {
            node.insert("children".into(), json!(kids));
        }
    }

    let mut root = Map::new();
    root.insert(
        "asset".into(),
        json!({ "version": "2.0", "generator": concat!("Grid4D ", env!("CARGO_PKG_VERSION")) }),
    );
    // An empty document has no scene at all (readers expect a scene to list nodes).
    if !roots.is_empty() {
        root.insert("scene".into(), json!(0));
        root.insert("scenes".into(), json!([{ "nodes": roots }]));
    }
    // glTF forbids empty arrays: every list is written only when it has entries.
    let mut list = |key: &str, values: Vec<Value>| {
        if !values.is_empty() {
            root.insert(key.into(), Value::Array(values));
        }
    };
    list("nodes", nodes.into_iter().map(Value::Object).collect());
    list("meshes", writer.meshes);
    list("materials", writer.materials);
    list("cameras", cameras);
    list("accessors", writer.accessors);
    list("bufferViews", writer.views);
    if !writer.bin.is_empty() {
        let mut buffer = json!({ "byteLength": writer.bin.len() });
        if let Some(uri) = buffer_uri {
            buffer["uri"] = json!(uri);
        }
        root.insert("buffers".into(), json!([buffer]));
    }
    if !lights.is_empty() {
        root.insert("extensionsUsed".into(), json!(["KHR_lights_punctual"]));
        root.insert(
            "extensions".into(),
            json!({ "KHR_lights_punctual": { "lights": lights } }),
        );
    }
    let json = serde_json::to_vec(&Value::Object(root)).map_err(|err| invalid(err.to_string()))?;
    Ok((json, writer.bin))
}

/// The binary buffer and the lists that refer to it, filled as meshes are written.
#[derive(Default)]
struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    materials: Vec<Value>,
    /// One material per display colour (by bits).
    material_of: HashMap<[u32; 3], usize>,
    /// One glTF mesh per (mesh, colour): objects sharing both share the mesh.
    mesh_of: HashMap<(MeshId, [u32; 3]), Option<usize>>,
    triangulator: Triangulator,
    tri: TriMesh,
}

impl Writer {
    /// Appends `bytes` as a buffer view (4-byte aligned) and returns its index.
    fn view(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        self.views.push(json!({
            "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len(), "target": target
        }));
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    fn accessor(&mut self, accessor: Value) -> usize {
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn material(&mut self, colour: [f32; 3]) -> Result<usize, IoError> {
        if !finite(&colour) {
            return Err(invalid("a display colour is not finite"));
        }
        let key = colour.map(f32::to_bits);
        if let Some(&index) = self.material_of.get(&key) {
            return Ok(index);
        }
        self.materials.push(json!({
            "pbrMetallicRoughness": {
                "baseColorFactor": [colour[0], colour[1], colour[2], 1.0],
                "metallicFactor": 0.0,
                "roughnessFactor": 0.8
            }
        }));
        let index = self.materials.len() - 1;
        self.material_of.insert(key, index);
        Ok(index)
    }

    /// The glTF mesh of `handle` in `colour` (written on first use); `None` for a
    /// mesh without polygons.
    fn mesh(&mut self, handle: &MeshHandle, colour: [f32; 3], name: &str) -> Result<Option<usize>, IoError> {
        let key = (handle.id(), colour.map(f32::to_bits));
        if let Some(&index) = self.mesh_of.get(&key) {
            return Ok(index);
        }
        let index = self.write_mesh(handle, colour, name)?;
        self.mesh_of.insert(key, index);
        Ok(index)
    }

    fn write_mesh(&mut self, handle: &MeshHandle, colour: [f32; 3], name: &str) -> Result<Option<usize>, IoError> {
        let mesh = handle.mesh();
        if mesh.face_count() == 0 {
            return Ok(None);
        }
        let mut tri = std::mem::take(&mut self.tri);
        self.triangulator.triangulate(mesh, Smoothing::default(), &mut tri);
        let has_uvs = tri.uvs.len() == tri.positions.len();

        // Corners that agree in position, normal and UV share one vertex.
        let mut shared: HashMap<[u32; 8], u32> = HashMap::new();
        let mut positions: Vec<f32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();
        let mut uvs: Vec<f32> = Vec::new();
        let mut remap: Vec<u32> = Vec::with_capacity(tri.positions.len());
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for corner in 0..tri.positions.len() {
            let p = tri.positions[corner] * TO_METRES;
            let n = tri
                .normals
                .get(corner)
                .copied()
                .unwrap_or(Vec3::Y)
                .normalize_or(Vec3::Y);
            let uv = if has_uvs { tri.uvs[corner].to_array() } else { [0.0; 2] };
            if !finite(&p.to_array()) || !finite(&uv) {
                self.tri = tri;
                return Err(invalid(format!("\"{name}\" has non-finite points or UVs")));
            }
            // `+ 0.0`: -0.0 and 0.0 are the same value, so they share a vertex.
            let key = [p.x, p.y, p.z, n.x, n.y, n.z, uv[0], uv[1]].map(|v| (v + 0.0).to_bits());
            let next =
                u32::try_from(positions.len() / 3).map_err(|_| invalid(format!("\"{name}\" has too many vertices")))?;
            let index = *shared.entry(key).or_insert_with(|| {
                positions.extend(p.to_array());
                normals.extend(n.to_array());
                if has_uvs {
                    uvs.extend(uv);
                }
                min = min.min(p);
                max = max.max(p);
                next
            });
            remap.push(index);
        }
        let vertex_count = positions.len() / 3;
        let indices: Vec<u32> = tri
            .indices
            .iter()
            .map(|&i| usize::try_from(i).ok().and_then(|i| remap.get(i)).copied().unwrap_or(0))
            .collect();
        self.tri = tri;

        let floats = |values: &[f32]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let view = self.view(&floats(&positions), ARRAY_BUFFER);
        let position = self.accessor(json!({
            "bufferView": view, "componentType": FLOAT, "count": vertex_count, "type": "VEC3",
            "min": min.to_array(), "max": max.to_array()
        }));
        let view = self.view(&floats(&normals), ARRAY_BUFFER);
        let normal = self.accessor(json!({
            "bufferView": view, "componentType": FLOAT, "count": vertex_count, "type": "VEC3"
        }));
        let mut attributes = json!({ "POSITION": position, "NORMAL": normal });
        if has_uvs {
            let view = self.view(&floats(&uvs), ARRAY_BUFFER);
            let uv = self.accessor(json!({
                "bufferView": view, "componentType": FLOAT, "count": vertex_count, "type": "VEC2"
            }));
            attributes["TEXCOORD_0"] = json!(uv);
        }
        let (bytes, component): (Vec<u8>, u32) = if vertex_count <= usize::from(u16::MAX) {
            let small: Vec<u8> = indices
                .iter()
                .flat_map(|&i| u16::try_from(i).unwrap_or(u16::MAX).to_le_bytes())
                .collect();
            (small, UNSIGNED_SHORT)
        } else {
            (indices.iter().flat_map(|i| i.to_le_bytes()).collect(), UNSIGNED_INT)
        };
        let view = self.view(&bytes, ELEMENT_ARRAY_BUFFER);
        let index_accessor = self.accessor(json!({
            "bufferView": view, "componentType": component, "count": indices.len(), "type": "SCALAR"
        }));
        let material = self.material(colour)?;
        self.meshes.push(json!({
            "name": name,
            "primitives": [ {
                "attributes": attributes, "indices": index_accessor, "material": material, "mode": 4
            } ]
        }));
        Ok(Some(self.meshes.len() - 1))
    }
}

/// Characters that cannot stand as they are in a URI → `%XX` (spaces and the like).
fn percent_encode(name: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use grid4d_geom::{PolyMesh, primitives};
    use grid4d_scene::{NodeData, ObjectKindTag, Transform};

    use super::*;
    use crate::{import::ObjectEntry, import_gltf};

    fn entry(parent: Option<usize>, name: &str, kind: ObjectKind) -> ObjectEntry {
        ObjectEntry {
            parent,
            data: NodeData::new(name, kind),
        }
    }

    /// Root (Null, moved, turned, scaled) > Box (a cube) + Pillar (a 7-sided
    /// cylinder: n-gon caps) + Twin (the cube's mesh again); Cam; Lamp; Empty (a
    /// polygon object without polygons).
    fn scene() -> Objects {
        let cube = MeshHandle::new(primitives::cube(Vec3::splat(200.0), [1, 1, 1]));
        let cylinder = MeshHandle::new(primitives::cylinder(50.0, 120.0, 7, 1));
        let mut nodes = vec![
            entry(None, "Root", ObjectKind::Null),
            entry(Some(0), "Box", ObjectKind::polygon(cube.clone())),
            entry(Some(0), "Pillar", ObjectKind::polygon(cylinder)),
            entry(Some(0), "Twin", ObjectKind::polygon(cube)),
            entry(None, "Cam", ObjectKind::Camera { fov_y_deg: 35.0 }),
            entry(
                None,
                "Lamp",
                ObjectKind::Light {
                    color: [1.0, 0.8, 0.6],
                    intensity: 2.5,
                },
            ),
            entry(None, "Empty", ObjectKind::polygon(MeshHandle::new(PolyMesh::new()))),
        ];
        nodes[0].data.transform = Transform {
            position: Vec3::new(100.0, 50.0, -20.0),
            rotation: Vec3::new(0.3, -0.2, 0.1),
            scale: Vec3::new(1.0, 2.0, 1.0),
        };
        nodes[1].data.display_color = [0.9, 0.1, 0.2];
        nodes[3].data.display_color = [0.9, 0.1, 0.2];
        nodes[2].data.transform.position = Vec3::new(300.0, 0.0, 0.0);
        nodes[4].data.transform.position = Vec3::new(0.0, 150.0, 800.0);
        Objects { nodes }
    }

    fn mesh(objects: &Objects, name: &str) -> MeshHandle {
        let node = objects.nodes.iter().find(|n| n.data.name == name).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("{name} is not a polygon object: {:?}", node.data.kind)
        };
        mesh.clone()
    }

    /// The distinct positions of a mesh (an exported mesh is welded on import).
    fn distinct_points(mesh: &PolyMesh) -> Vec<Vec3> {
        let mut points: Vec<Vec3> = Vec::new();
        for &p in mesh.points() {
            if !points.iter().any(|q| q.abs_diff_eq(p, 1e-3)) {
                points.push(p);
            }
        }
        points
    }

    fn triangles(mesh: &PolyMesh) -> usize {
        (0..mesh.face_count()).map(|f| mesh.face(f).len() - 2).sum()
    }

    #[test]
    fn export_then_import_gives_the_scene_back() {
        let original = scene();
        let back = import_gltf(&glb_bytes(&original).unwrap(), None).unwrap();
        let shape = |o: &Objects| -> Vec<(String, Option<usize>)> {
            o.nodes.iter().map(|n| (n.data.name.clone(), n.parent)).collect()
        };
        assert_eq!(shape(&back), shape(&original), "names and hierarchy");

        for (a, b) in original.nodes.iter().zip(&back.nodes) {
            let (ta, tb) = (&a.data.transform, &b.data.transform);
            assert!(
                ta.position.abs_diff_eq(tb.position, 1e-3),
                "{}: {} vs {}",
                a.data.name,
                ta.position,
                tb.position
            );
            assert!(ta.scale.abs_diff_eq(tb.scale, 1e-5), "{}", a.data.name);
            assert!(
                ta.rotation_quat().dot(tb.rotation_quat()).abs() > 1.0 - 1e-6,
                "{}: same orientation",
                a.data.name
            );
        }
        assert_eq!(back.nodes[0].data.kind, ObjectKind::Null);
        let ObjectKind::Camera { fov_y_deg } = back.nodes[4].data.kind else {
            panic!("not a camera")
        };
        assert!((fov_y_deg - 35.0).abs() < 1e-4);
        assert_eq!(
            back.nodes[5].data.kind, original.nodes[5].data.kind,
            "the light, exactly"
        );
        assert_eq!(
            back.nodes[6].data.kind.tag(),
            ObjectKindTag::Null,
            "no polygons: an empty node"
        );
        assert_eq!(
            back.nodes[1].data.display_color,
            [0.9, 0.1, 0.2],
            "colour via the material"
        );

        for name in ["Box", "Pillar"] {
            let (a, b) = (mesh(&original, name), mesh(&back, name));
            let (expected, got) = (distinct_points(a.mesh()), distinct_points(b.mesh()));
            assert_eq!(got.len(), expected.len(), "{name}: the same points");
            assert!(
                expected.iter().all(|p| got.iter().any(|q| q.abs_diff_eq(*p, 1e-3))),
                "{name}"
            );
            assert_eq!(
                b.mesh().face_count(),
                triangles(a.mesh()),
                "{name}: n-gons as n − 2 triangles"
            );
        }
        assert_eq!(
            mesh(&back, "Box").id(),
            mesh(&back, "Twin").id(),
            "a shared mesh stays shared"
        );
    }

    #[test]
    fn the_json_names_one_mesh_per_mesh_and_colour_and_has_normals() {
        let (json, bin) = gltf_document(&scene(), Some("scene.bin")).unwrap();
        let doc: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(doc["meshes"].as_array().unwrap().len(), 2, "Box and Twin share one");
        assert_eq!(doc["materials"].as_array().unwrap().len(), 2, "one per colour");
        assert_eq!(doc["buffers"][0]["uri"], "scene.bin");
        assert_eq!(doc["buffers"][0]["byteLength"], bin.len());
        let primitive = &doc["meshes"][0]["primitives"][0];
        assert!(primitive["attributes"]["NORMAL"].is_number());
        assert_eq!(doc["extensionsUsed"], json!(["KHR_lights_punctual"]));
        assert_eq!(doc["nodes"][0]["children"], json!([1, 2, 3]));
        assert_eq!(doc["scenes"][0]["nodes"], json!([0, 4, 5, 6]));
        // Cube corners are hard: the flat normals of a 1-segment cube give 24 vertices.
        let position = primitive["attributes"]["POSITION"].as_u64().unwrap();
        assert_eq!(doc["accessors"][position as usize]["count"], 24);
        assert_eq!(
            doc["accessors"][position as usize]["max"],
            json!([1.0, 1.0, 1.0]),
            "metres"
        );

        let empty = gltf_document(&Objects::default(), None).unwrap().0;
        let empty: Value = serde_json::from_slice(&empty).unwrap();
        for key in [
            "scenes",
            "nodes",
            "meshes",
            "materials",
            "accessors",
            "bufferViews",
            "buffers",
        ] {
            assert!(empty.get(key).is_none(), "no empty \"{key}\" array (glTF forbids them)");
        }
        assert!(
            import_gltf(&glb_bytes(&Objects::default()).unwrap(), None)
                .unwrap()
                .nodes
                .is_empty()
        );
    }

    #[test]
    fn gltf_with_a_bin_file_and_uvs_round_trip() {
        let mut objects = scene();
        let mut quad = PolyMesh::from_faces(
            vec![
                Vec3::ZERO,
                Vec3::X * 100.0,
                Vec3::new(100.0, 0.0, 100.0),
                Vec3::Z * 100.0,
            ],
            [[0u32, 3, 2, 1]],
        )
        .unwrap();
        let uvs = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
        ];
        quad.set_uvs(Some(uvs.clone())).unwrap();
        objects
            .nodes
            .push(entry(None, "Floor", ObjectKind::polygon(MeshHandle::new(quad))));

        let dir = std::env::temp_dir().join(format!("grid4d-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my scene.gltf");
        export_gltf(&objects, &path).unwrap();
        assert!(dir.join("my scene.bin").is_file(), "the buffer next to it");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("my%20scene.bin"), "the URI is encoded");

        let from_files = crate::import(&path).unwrap();
        let from_glb = import_gltf(&glb_bytes(&objects).unwrap(), None).unwrap();
        assert_eq!(from_files, from_glb, "the same model either way");
        let floor = mesh(&from_files, "Floor");
        let got = floor.mesh().uvs().unwrap();
        assert_eq!(got.len(), 6, "two triangles");
        assert!(got.iter().all(|uv| uvs.contains(uv)), "{got:?}");

        assert!(
            export_gltf(&objects, &dir.join("scene.obj")).is_err(),
            "only .glb and .gltf"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_finite_numbers_are_refused() {
        let mut objects = scene();
        objects.nodes[0].data.transform.position.x = f32::NAN;
        assert!(glb_bytes(&objects).is_err());
        let mut objects = scene();
        objects.nodes[4].data.kind = ObjectKind::Camera {
            fov_y_deg: f32::INFINITY,
        };
        assert!(glb_bytes(&objects).is_err());
    }

    #[test]
    fn file_names_are_percent_encoded() {
        assert_eq!(percent_encode("my scene#1.bin"), "my%20scene%231.bin");
        assert_eq!(percent_encode("plain-name_v2.bin"), "plain-name_v2.bin");
    }
}
