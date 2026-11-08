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
use grid4d_scene::{
    Channel, LightType, Material, MeshHandle, MeshId, ObjectKind, PictureChannel, Tags, TextureId, TextureSlot,
    TextureSource,
};
use serde_json::{Map, Value, json};

use crate::{
    IoError,
    import::{Look, Objects},
    write_atomic,
};

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
    let effective = objects.effective_materials();

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
                let angle = data.tags.smoothing_angle();
                // Its material (own or inherited), else its display colour.
                let look = match effective.get(index).copied().flatten() {
                    Some(material) => Look::Material(material),
                    None => Look::Colour(data.display_color.map(f32::to_bits)),
                };
                if let Some(index) = writer.mesh(mesh, look, angle, &data.name, objects)? {
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
            // glTF lights have no shadow setting.
            ObjectKind::Light {
                color,
                intensity,
                light_type,
                details,
                ..
            } => {
                if !finite(color) || !intensity.is_finite() {
                    return Err(invalid(format!("light \"{}\" has a non-finite colour", data.name)));
                }
                node.insert(
                    "extensions".into(),
                    json!({ "KHR_lights_punctual": { "light": lights.len() } }),
                );
                // glTF lights shine along −Z, like Grid4D's; cone angles are half openings.
                lights.push(match light_type {
                    LightType::Omni => json!({ "type": "point", "color": color, "intensity": intensity }),
                    LightType::Infinite => {
                        json!({ "type": "directional", "color": color, "intensity": intensity })
                    }
                    // glTF has no area lights: a one-sided flat one shines
                    // into its half of space like a wide spot, the others all
                    // round like a point.
                    LightType::Area if details.area_shape.is_flat() && !details.two_sided => json!({
                        "type": "spot",
                        "color": color,
                        "intensity": intensity,
                        "spot": {
                            "innerConeAngle": std::f32::consts::FRAC_PI_4,
                            "outerConeAngle": std::f32::consts::FRAC_PI_2 * 0.999,
                        },
                    }),
                    LightType::Area => json!({ "type": "point", "color": color, "intensity": intensity }),
                    LightType::Spot => {
                        let outer = details.outer_angle_deg.to_radians() * 0.5;
                        // glTF wants the inner cone strictly inside the outer one.
                        let inner = if details.use_inner {
                            (details.inner_angle_deg.to_radians() * 0.5).min(outer * 0.999)
                        } else {
                            outer * 0.999
                        };
                        if !outer.is_finite() || !inner.is_finite() {
                            return Err(invalid(format!("light \"{}\" has a non-finite cone", data.name)));
                        }
                        json!({
                            "type": "spot",
                            "color": color,
                            "intensity": intensity,
                            "spot": { "innerConeAngle": inner, "outerConeAngle": outer },
                        })
                    }
                });
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
            | ObjectKind::SubdivisionSurface { .. }
            | ObjectKind::Extrude { .. }
            | ObjectKind::Lathe(_)
            | ObjectKind::Sweep(_)
            | ObjectKind::Loft(_)
            | ObjectKind::Cloner { .. }
            | ObjectKind::PlainEffector(_)
            | ObjectKind::RandomEffector(_)
            | ObjectKind::Bend(_)
            | ObjectKind::Twist(_)
            | ObjectKind::Taper(_)
            | ObjectKind::Bulge(_)
            | ObjectKind::Ffd { .. }
            // glTF has no sky or surrounding picture: the node keeps its place.
            | ObjectKind::Sky(_)
            | ObjectKind::Environment(_)
            // Curves have no surface: the node keeps its place, nothing more.
            | ObjectKind::SplinePrimitive(_)
            | ObjectKind::Text(_)
            | ObjectKind::Spline { .. }
            // Baked to a polygon object before export (`file::exportable`).
            | ObjectKind::Plugin(_) => {}
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
    list("textures", writer.textures);
    list("images", writer.images);
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
    let mut used: Vec<&str> = writer.extensions.iter().copied().collect();
    if !lights.is_empty() {
        used.push("KHR_lights_punctual");
        root.insert(
            "extensions".into(),
            json!({ "KHR_lights_punctual": { "lights": lights } }),
        );
    }
    if !used.is_empty() {
        used.sort_unstable();
        root.insert("extensionsUsed".into(), json!(used));
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
    /// One glTF material per document material or display colour.
    material_of: HashMap<Look, usize>,
    images: Vec<Value>,
    textures: Vec<Value>,
    /// One glTF texture per document texture (`None`: its picture cannot be
    /// written: missing, or not PNG or JPEG).
    texture_of: HashMap<TextureId, Option<usize>>,
    /// The extensions the materials use.
    extensions: std::collections::BTreeSet<&'static str>,
    /// One glTF mesh per (mesh, look, Phong angle): objects sharing all three
    /// share the mesh.
    mesh_of: HashMap<(MeshId, Look, u32), Option<usize>>,
    triangulator: Triangulator,
    tri: TriMesh,
}

impl Writer {
    /// Appends `bytes` as a buffer view (4-byte aligned) and returns its index;
    /// `target` 0 writes none (pictures).
    fn view(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if target != 0 {
            view["target"] = json!(target);
        }
        self.views.push(view);
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    fn accessor(&mut self, accessor: Value) -> usize {
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    /// The glTF texture of document texture `id` (its picture in the buffer,
    /// written on first use); `None` if the picture cannot be read or is
    /// neither PNG nor JPEG (glTF's own two formats).
    fn texture(&mut self, id: TextureId, objects: &Objects) -> Option<usize> {
        if let Some(&made) = self.texture_of.get(&id) {
            return made;
        }
        let made = objects.textures.get(id).and_then(|texture| {
            let bytes: std::borrow::Cow<'_, [u8]> = match &texture.source {
                TextureSource::Embedded { bytes, .. } => std::borrow::Cow::Borrowed(bytes),
                TextureSource::Linked(path) => std::borrow::Cow::Owned(crate::read_file(path).ok()?),
            };
            let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                "image/jpeg"
            } else {
                return None;
            };
            let view = self.view(&bytes, 0);
            self.images
                .push(json!({ "name": texture.name, "bufferView": view, "mimeType": mime }));
            self.textures.push(json!({ "source": self.images.len() - 1 }));
            Some(self.textures.len() - 1)
        });
        self.texture_of.insert(id, made);
        made
    }

    /// The texture info of `slot` (`None` without a writable picture), with
    /// its tiling and offset as `KHR_texture_transform`.
    fn info(&mut self, slot: Option<&TextureSlot>, objects: &Objects) -> Option<Value> {
        let slot = slot?;
        let index = self.texture(slot.texture, objects)?;
        let mut info = json!({ "index": index });
        if slot.tiling != [1.0, 1.0] || slot.offset != [0.0, 0.0] {
            self.extensions.insert("KHR_texture_transform");
            info["extensions"] = json!({
                "KHR_texture_transform": { "offset": slot.offset, "scale": slot.tiling }
            });
        }
        Some(info)
    }

    /// `material` in glTF's model: factors, pictures and the extensions for
    /// what the core leaves out.
    fn material_value(&mut self, material: &Material, objects: &Objects) -> Value {
        let [r, g, b] = material.color;
        let mut value = json!({
            "name": material.name,
            "pbrMetallicRoughness": {
                "baseColorFactor": [r, g, b, 1.0],
                "metallicFactor": material.metallic,
                "roughnessFactor": material.roughness
            }
        });
        let base = material.map(Channel::BaseColor).copied();
        if let Some(info) = self.info(base.as_ref(), objects) {
            value["pbrMetallicRoughness"]["baseColorTexture"] = info;
        }
        // Roughness and metallic from one picture, in green and blue (as
        // glTF packs them); the ranges' tops become the factors.
        let (rough, metal) = (material.map(Channel::Roughness), material.map(Channel::Metallic));
        if let (Some(rough), Some(metal)) = (rough, metal)
            && rough.texture == metal.texture
            && rough.source == PictureChannel::Green
            && metal.source == PictureChannel::Blue
            && let Some(info) = self.info(Some(rough), objects)
        {
            value["pbrMetallicRoughness"]["metallicRoughnessTexture"] = info;
            value["pbrMetallicRoughness"]["roughnessFactor"] = json!(rough.range[1].clamp(0.0, 1.0));
            value["pbrMetallicRoughness"]["metallicFactor"] = json!(metal.range[1].clamp(0.0, 1.0));
        }
        if let Some(mut info) = self.info(material.map(Channel::Normal), objects) {
            info["scale"] = json!(material.normal_strength);
            value["normalTexture"] = info;
        }
        let emissive = self.info(material.map(Channel::Emission), objects);
        if material.emission != [0.0; 3] || emissive.is_some() {
            let colour = if emissive.is_some() {
                [1.0; 3]
            } else {
                material.emission
            };
            value["emissiveFactor"] = json!(colour.map(|c| c.clamp(0.0, 1.0)));
            if let Some(info) = emissive {
                value["emissiveTexture"] = info;
            }
            if (material.emission_strength - 1.0).abs() > 1e-6 {
                self.extension(
                    &mut value,
                    "KHR_materials_emissive_strength",
                    json!({
                        "emissiveStrength": material.emission_strength.max(0.0)
                    }),
                );
            }
        }
        // A cut-out from the base colour picture's alpha.
        if let (Some(opacity), Some(base)) = (material.map(Channel::Opacity), base)
            && opacity.texture == base.texture
            && opacity.source == PictureChannel::Alpha
        {
            value["alphaMode"] = json!("MASK");
            value["alphaCutoff"] = json!(material.opacity_threshold);
        }
        if material.transmission > 0.0 {
            self.extension(
                &mut value,
                "KHR_materials_transmission",
                json!({
                    "transmissionFactor": material.transmission
                }),
            );
        }
        if (material.ior - 1.5).abs() > 1e-6 {
            self.extension(&mut value, "KHR_materials_ior", json!({ "ior": material.ior }));
        }
        if (material.specular - 0.5).abs() > 1e-6 {
            self.extension(
                &mut value,
                "KHR_materials_specular",
                json!({
                    "specularFactor": (material.specular * 2.0).clamp(0.0, 1.0)
                }),
            );
        }
        if material.clearcoat > 0.0 {
            self.extension(
                &mut value,
                "KHR_materials_clearcoat",
                json!({
                    "clearcoatFactor": material.clearcoat,
                    "clearcoatRoughnessFactor": material.clearcoat_roughness
                }),
            );
        }
        if material.two_sided {
            value["doubleSided"] = json!(true);
        }
        value
    }

    /// Adds extension `name` to a material and to the file's list.
    fn extension(&mut self, value: &mut Value, name: &'static str, body: Value) {
        self.extensions.insert(name);
        if value.get("extensions").is_none() {
            value["extensions"] = json!({});
        }
        value["extensions"][name] = body;
    }

    /// The glTF material for `look` (written on first use).
    fn material(&mut self, look: Look, objects: &Objects) -> Result<usize, IoError> {
        if let Some(&index) = self.material_of.get(&look) {
            return Ok(index);
        }
        let value = match look {
            Look::Material(id) => {
                let material = objects
                    .materials
                    .get(id)
                    .ok_or_else(|| invalid("a Material tag refers to a missing material"))?;
                if !finite(&material.numbers()) {
                    return Err(invalid(format!("material \"{}\" is not finite", material.name)));
                }
                self.material_value(material, objects)
            }
            Look::Colour(bits) => {
                let colour = bits.map(f32::from_bits);
                if !finite(&colour) {
                    return Err(invalid("a display colour is not finite"));
                }
                json!({
                    "pbrMetallicRoughness": {
                        "baseColorFactor": [colour[0], colour[1], colour[2], 1.0],
                        "metallicFactor": 0.0,
                        "roughnessFactor": 0.8
                    }
                })
            }
        };
        self.materials.push(value);
        let index = self.materials.len() - 1;
        self.material_of.insert(look, index);
        Ok(index)
    }

    /// The glTF mesh of `handle` looking like `look`, smoothed up to `angle`
    /// (radians; written on first use); `None` for a mesh without polygons.
    fn mesh(
        &mut self,
        handle: &MeshHandle,
        look: Look,
        angle: f32,
        name: &str,
        objects: &Objects,
    ) -> Result<Option<usize>, IoError> {
        let key = (handle.id(), look, angle.to_bits());
        if let Some(&index) = self.mesh_of.get(&key) {
            return Ok(index);
        }
        let index = self.write_mesh(handle, look, angle, name, objects)?;
        self.mesh_of.insert(key, index);
        Ok(index)
    }

    fn write_mesh(
        &mut self,
        handle: &MeshHandle,
        look: Look,
        angle: f32,
        name: &str,
        objects: &Objects,
    ) -> Result<Option<usize>, IoError> {
        let mesh = handle.mesh();
        if mesh.face_count() == 0 {
            return Ok(None);
        }
        let mut tri = std::mem::take(&mut self.tri);
        self.triangulator.triangulate(mesh, Smoothing::Angle(angle), &mut tri);
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
        let material = self.material(look, objects)?;
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
    use grid4d_scene::{LightDetails, NodeData, ObjectKindTag, Transform};

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
    #[test]
    fn materials_round_trip_through_gltf() {
        use grid4d_scene::{Material, Tag};
        let mut objects = scene();
        let mut gold = Material::new("Gold");
        gold.color = [1.0, 0.7, 0.2];
        gold.metallic = 1.0;
        gold.roughness = 0.3;
        gold.emission = [0.1, 0.0, 0.0];
        let gold = objects.materials.insert(gold);
        // On the first object; its children inherit it.
        objects.nodes[0]
            .data
            .tags
            .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(gold))));
        let bytes = glb_bytes(&objects).unwrap();
        let back = import_gltf(&bytes, None).unwrap();
        let materials: Vec<&Material> = back.materials.iter().map(|(_, m)| m).collect();
        let found = materials
            .iter()
            .find(|m| m.name == "Gold")
            .expect("the material comes back");
        assert_eq!((found.metallic, found.roughness), (1.0, 0.3));
        assert_eq!(found.color, [1.0, 0.7, 0.2]);
        assert_eq!(found.emission, [0.1, 0.0, 0.0]);
        let tagged = back.nodes.iter().filter(|n| n.data.tags.material().is_some()).count();
        assert!(tagged >= 1, "meshes come back with Material tags");
    }

    /// A 2 × 2 PNG of RGBA `pixel`.
    fn png(pixel: [u8; 4]) -> Vec<u8> {
        crate::png_bytes(2, 2, crate::Channels::Rgba, &pixel.repeat(4)).unwrap()
    }

    #[test]
    fn every_channel_and_its_pictures_round_trip_through_gltf() {
        use grid4d_scene::{ColorSpace, Material, Tag, Texture, TextureSlot, TextureSource};
        let mut objects = scene();
        let mut texture = |name: &str, pixel: [u8; 4], space: ColorSpace| {
            objects.textures.insert(Texture {
                name: name.to_owned(),
                source: TextureSource::Embedded {
                    file_name: format!("{name}.png"),
                    bytes: png(pixel).into(),
                },
                color_space: space,
            })
        };
        let (wood, packed, bumps, glow) = (
            texture("Wood", [200, 120, 60, 128], ColorSpace::Srgb),
            texture("Packed", [0, 200, 40, 255], ColorSpace::Linear),
            texture("Bumps", [128, 128, 255, 255], ColorSpace::Linear),
            texture("Glow", [255, 200, 0, 255], ColorSpace::Srgb),
        );
        let mut all = Material::new("All");
        (all.color, all.roughness, all.metallic) = ([1.0; 3], 0.8, 0.5);
        (all.normal_strength, all.emission, all.emission_strength) = (1.5, [1.0; 3], 3.0);
        (all.opacity_threshold, all.transmission, all.ior) = (0.4, 0.25, 1.33);
        (all.specular, all.clearcoat, all.clearcoat_roughness, all.two_sided) = (0.3, 0.6, 0.2, true);
        *all.map_mut(Channel::BaseColor) = Some(TextureSlot {
            tiling: [2.0, 3.0],
            offset: [0.5, 0.25],
            ..TextureSlot::new(wood)
        });
        let packed_slot = |source, top| TextureSlot {
            range: [0.0, top],
            source,
            ..TextureSlot::new(packed)
        };
        *all.map_mut(Channel::Roughness) = Some(packed_slot(PictureChannel::Green, 0.8));
        *all.map_mut(Channel::Metallic) = Some(packed_slot(PictureChannel::Blue, 0.5));
        *all.map_mut(Channel::Normal) = Some(TextureSlot::new(bumps));
        *all.map_mut(Channel::Emission) = Some(TextureSlot::new(glow));
        *all.map_mut(Channel::Opacity) = Some(TextureSlot {
            source: PictureChannel::Alpha,
            ..TextureSlot::new(wood)
        });
        let all = objects.materials.insert(all);
        objects.nodes[0]
            .data
            .tags
            .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(all))));

        let bytes = glb_bytes(&objects).unwrap();
        let json = String::from_utf8_lossy(&bytes);
        for extension in [
            "KHR_texture_transform",
            "KHR_materials_emissive_strength",
            "KHR_materials_transmission",
            "KHR_materials_ior",
            "KHR_materials_specular",
            "KHR_materials_clearcoat",
        ] {
            assert!(json.contains(extension), "{extension} written");
        }
        let back = import_gltf(&bytes, None).unwrap();
        let (_, found) = back.materials.iter().find(|(_, m)| m.name == "All").unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(found.roughness, 0.8) && close(found.metallic, 0.5));
        assert!(close(found.normal_strength, 1.5) && close(found.emission_strength, 3.0));
        assert!(close(found.opacity_threshold, 0.4) && close(found.transmission, 0.25));
        assert!(close(found.ior, 1.33) && close(found.specular, 0.3));
        assert!(close(found.clearcoat, 0.6) && close(found.clearcoat_roughness, 0.2) && found.two_sided);
        let slot = |channel| {
            *found
                .map(channel)
                .unwrap_or_else(|| panic!("{channel:?} has a texture"))
        };
        let base = slot(Channel::BaseColor);
        assert_eq!((base.tiling, base.offset), ([2.0, 3.0], [0.5, 0.25]));
        let (rough, metal) = (slot(Channel::Roughness), slot(Channel::Metallic));
        assert_eq!(
            (rough.source, metal.source),
            (PictureChannel::Green, PictureChannel::Blue)
        );
        assert_eq!(rough.texture, metal.texture, "one packed picture");
        assert!(close(rough.range[1], 0.8) && close(metal.range[1], 0.5));
        assert_eq!(slot(Channel::Opacity).source, PictureChannel::Alpha);
        let _ = slot(Channel::Normal);
        let _ = slot(Channel::Emission);
        // The pictures come back embedded, byte for byte, in their colour spaces.
        let picture = |slot: TextureSlot| back.textures.get(slot.texture).unwrap().clone();
        let wood_back = picture(base);
        assert_eq!(wood_back.color_space, ColorSpace::Srgb);
        let TextureSource::Embedded { bytes: wood_bytes, .. } = &wood_back.source else {
            panic!("embedded")
        };
        assert_eq!(&wood_bytes[..], &png([200, 120, 60, 128])[..]);
        assert_eq!(picture(rough).color_space, ColorSpace::Linear);
    }

    #[test]
    fn pictures_gltf_cannot_hold_are_left_out_and_the_material_stays() {
        use grid4d_scene::{Material, Tag, Texture, TextureSlot, TextureSource};
        let mut objects = scene();
        let tiff = objects.textures.insert(Texture {
            name: "Scan".to_owned(),
            source: TextureSource::Embedded {
                file_name: "scan.tif".to_owned(),
                bytes: b"II*\0 not really".to_vec().into(),
            },
            color_space: grid4d_scene::ColorSpace::Srgb,
        });
        let missing = objects.textures.insert(Texture::linked("/nowhere/at/all.png".into()));
        let mut plain = Material::new("Plain");
        *plain.map_mut(Channel::BaseColor) = Some(TextureSlot::new(tiff));
        *plain.map_mut(Channel::Normal) = Some(TextureSlot::new(missing));
        let plain = objects.materials.insert(plain);
        objects.nodes[0]
            .data
            .tags
            .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(plain))));
        let back = import_gltf(&glb_bytes(&objects).unwrap(), None).unwrap();
        let (_, found) = back.materials.iter().find(|(_, m)| m.name == "Plain").unwrap();
        assert_eq!(found.maps().count(), 0);
        assert!(back.textures.is_empty());
    }

    fn scene() -> Objects {
        let cube = MeshHandle::new(primitives::cube(Vec3::splat(200.0), [1, 1, 1]));
        let cylinder = MeshHandle::new(primitives::cylinder(50.0, 120.0, 7, 1));
        let mut nodes = vec![
            entry(None, "Root", ObjectKind::Null),
            entry(Some(0), "Box", ObjectKind::polygon(cube.clone())),
            entry(Some(0), "Pillar", ObjectKind::polygon(cylinder)),
            entry(Some(0), "Twin", ObjectKind::polygon(cube)),
            entry(None, "Cam", ObjectKind::Camera { fov_y_deg: 35.0 }),
            entry(None, "Lamp", ObjectKind::light([1.0, 0.8, 0.6], 2.5)),
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
        Objects {
            nodes,
            ..Objects::default()
        }
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
    fn spot_and_infinite_lights_keep_their_type_and_cone() {
        let light = |light_type, details| ObjectKind::Light {
            color: [1.0, 0.9, 0.8],
            intensity: 1.5,
            light_type,
            details,
            shadow: grid4d_scene::LightShadow::None,
        };
        let soft = LightDetails {
            inner_angle_deg: 20.0,
            outer_angle_deg: 50.0,
            ..LightDetails::default()
        };
        let hard = LightDetails {
            use_inner: false,
            ..soft
        };
        let objects = Objects {
            nodes: vec![
                entry(None, "Soft", light(LightType::Spot, soft)),
                entry(None, "Hard", light(LightType::Spot, hard)),
                entry(None, "Sun", light(LightType::Infinite, LightDetails::default())),
                // glTF has no area lights: a one-sided panel becomes a wide
                // spot, a bulb (Sphere) a point light.
                entry(None, "Panel", light(LightType::Area, LightDetails::default())),
                entry(
                    None,
                    "Bulb",
                    light(
                        LightType::Area,
                        LightDetails {
                            area_shape: grid4d_scene::AreaShape::Sphere,
                            ..LightDetails::default()
                        },
                    ),
                ),
            ],
            ..Objects::default()
        };
        let back = import_gltf(&glb_bytes(&objects).unwrap(), None).unwrap();
        let kinds: Vec<(LightType, LightDetails)> = back
            .nodes
            .iter()
            .map(|n| match n.data.kind {
                ObjectKind::Light {
                    light_type, details, ..
                } => (light_type, details),
                _ => panic!("not a light"),
            })
            .collect();
        assert_eq!(kinds[0].0, LightType::Spot);
        let (a, b) = (kinds[0].1, soft);
        assert!(
            a.use_inner && (a.inner_angle_deg - b.inner_angle_deg).abs() < 1e-3,
            "{a:?}"
        );
        assert!((a.outer_angle_deg - b.outer_angle_deg).abs() < 1e-3);
        assert_eq!(kinds[1].0, LightType::Spot);
        assert!(!kinds[1].1.use_inner, "a hard cone stays hard");
        assert_eq!(kinds[2], (LightType::Infinite, LightDetails::default()));
        assert_eq!(kinds[3].0, LightType::Spot);
        assert!(
            kinds[3].1.outer_angle_deg >= 170.0,
            "as wide as a cone can be: {:?}",
            kinds[3].1
        );
        assert_eq!(kinds[4].0, LightType::Omni);
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
    fn a_phong_tag_sets_the_exported_normals() {
        let sphere = MeshHandle::new(primitives::sphere(100.0, 16));
        let vertices = |tags: Vec<grid4d_scene::Tag>| {
            let mut node = entry(None, "Ball", ObjectKind::polygon(sphere.clone()));
            node.data.tags = tags;
            let (json, _) = gltf_document(
                &Objects {
                    nodes: vec![node],
                    ..Objects::default()
                },
                None,
            )
            .unwrap();
            let doc: Value = serde_json::from_slice(&json).unwrap();
            let position = doc["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
                .as_u64()
                .unwrap();
            doc["accessors"][position as usize]["count"].as_u64().unwrap()
        };
        let smooth = vertices(Vec::new());
        let flat = vertices(vec![grid4d_scene::Tag::Phong {
            angle_limit: true,
            angle: 0.0,
        }]);
        assert!(
            flat > smooth,
            "flat shading needs a vertex per polygon corner: {flat} vs {smooth}"
        );
        let unlimited = vertices(vec![grid4d_scene::Tag::Phong {
            angle_limit: false,
            angle: 0.0,
        }]);
        assert!(unlimited <= smooth);
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
