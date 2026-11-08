//! Wavefront OBJ import and export (see `docs/reference/interchange-obj.md`).
//! OBJ has no units (1 unit = 1 cm here), no hierarchy and no transforms: objects
//! come in at the origin, and go out with their points in world space. Polygons
//! keep all their corners both ways. Texture coordinates start at the bottom left
//! in OBJ and at the top left in Grid4D, so V is flipped both ways (a picture's
//! `-o` offset in V too). Materials go through the `.mtl`: colours, the PBR
//! extension's numbers (`Pr`, `Pm`, `Pc`, `Pcr`) and the pictures (`map_Kd`,
//! `map_Ke`, `map_d`, `bump`, `norm`, `map_Pr`, `map_Pm`) with their `-s`, `-o`,
//! `-bm` and `-imfchan` options.

use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use glam::{Mat3, Mat4, Vec2, Vec3};
use grid4d_geom::{PolyMesh, Smoothing, TriMesh, Triangulator};
use grid4d_scene::{
    Channel, ColorSpace, Material, MaterialId, Materials, MeshHandle, NodeData, ObjectKind, PictureChannel, Tag, Tags,
    Texture, TextureId, TextureSlot, TextureSource, Textures,
};

use crate::{
    IoError,
    import::Look,
    import::{ObjectEntry, Objects},
    write_atomic,
};

/// The name `tobj` gives objects without one.
const UNNAMED: &str = "unnamed_object";

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Invalid(message.into())
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}

/// The objects of an OBJ file (`bytes`); `fallback` names an object the file
/// leaves unnamed. The `.mtl` it names is read relative to `base`, the file's folder.
///
/// # Errors
///
/// [`IoError::Invalid`] for unreadable text, indices out of range or non-finite
/// numbers. A missing or broken `.mtl` is not an error (default colours).
pub fn import_obj(bytes: &[u8], base: Option<&Path>, fallback: &str) -> Result<Objects, IoError> {
    let options = tobj::LoadOptions {
        single_index: false,
        triangulate: false,
        ignore_points: true,
        ignore_lines: true,
    };
    let mut reader = bytes;
    let (models, materials) = tobj::load_obj_buf(&mut reader, &options, |path| load_mtl(path, base))
        .map_err(|err| invalid(format!("not a valid OBJ file: {err}")))?;
    let mtl = materials.unwrap_or_default();
    // Every material of the .mtl, in its order; objects get a Material tag for theirs.
    let mut materials = Materials::default();
    let mut pictures = MtlPictures {
        base,
        textures: Textures::default(),
        made: HashMap::new(),
    };
    let material_ids: Vec<MaterialId> = mtl
        .iter()
        .map(|m| {
            let material = read_material(m, &materials, &mut pictures);
            materials.insert(material)
        })
        .collect();
    let textures = pictures.textures;

    let mut nodes = Vec::new();
    for model in models {
        let Some(mesh) = read_mesh(&model.mesh, &model.name)? else {
            continue;
        };
        let name = match model.name.trim() {
            "" | UNNAMED => fallback,
            name => name,
        };
        let mut data = NodeData::new(name, ObjectKind::polygon(MeshHandle::new(mesh)));
        if let Some(colour) = model
            .mesh
            .material_id
            .and_then(|id| mtl.get(id))
            .and_then(|m| m.diffuse)
            .filter(|c| finite(c))
        {
            data.display_color = colour;
        }
        if let Some(&material) = model.mesh.material_id.and_then(|id| material_ids.get(id)) {
            data.tags
                .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(material))));
        }
        nodes.push(ObjectEntry { parent: None, data });
    }
    Ok(Objects {
        nodes,
        materials,
        textures,
        layers: grid4d_scene::Layers::default(),
        sources: Vec::new(),
    })
}

/// A `.mtl` material as a Grid4D one.
fn read_material(m: &tobj::Material, taken: &Materials, pictures: &mut MtlPictures<'_>) -> Material {
    let name = match m.name.trim() {
        "" => taken.unique_name("Mat"),
        name => name.to_owned(),
    };
    let mut material = Material::new(name);
    if let Some(colour) = m.diffuse.filter(|c| finite(c)) {
        material.color = colour.map(|c| c.max(0.0));
    }
    if let Some(emission) = m.emissive.filter(|c| finite(c)) {
        material.emission = emission.map(|c| c.max(0.0));
    }
    if let Some(ior) = m.optical_density.filter(|v| v.is_finite() && *v >= 1.0) {
        material.ior = ior.clamp(Material::IOR_RANGE.0, Material::IOR_RANGE.1);
    }
    let number = |key: &str| {
        m.unknown_param
            .get(key)
            .and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok())
            .filter(|v| v.is_finite())
    };
    // Roughness from the PBR extension, else from Phong's shininess.
    if let Some(rough) = number("Pr") {
        material.roughness = rough.clamp(0.0, 1.0);
    } else if let Some(shine) = m.shininess.filter(|s| s.is_finite() && *s >= 0.0) {
        material.roughness = (2.0 / (shine + 2.0)).sqrt().clamp(0.0, 1.0);
    }
    if let Some(metal) = number("Pm") {
        material.metallic = metal.clamp(0.0, 1.0);
    }
    if let Some(coat) = number("Pc") {
        material.clearcoat = coat.clamp(0.0, 1.0);
    }
    if let Some(coat) = number("Pcr") {
        material.clearcoat_roughness = coat.clamp(0.0, 1.0);
    }

    let unknown = |key: &str| m.unknown_param.get(key).map(String::as_str);
    let maps: [(Channel, Option<&str>); 7] = [
        (Channel::BaseColor, m.diffuse_texture.as_deref()),
        (Channel::Emission, unknown("map_Ke")),
        (Channel::Opacity, m.dissolve_texture.as_deref()),
        // `tobj` files `bump` and `map_Bump` (height pictures) as its normal texture.
        (Channel::Bump, m.normal_texture.as_deref()),
        (Channel::Normal, unknown("norm")),
        (Channel::Roughness, unknown("map_Pr")),
        (Channel::Metallic, unknown("map_Pm")),
    ];
    for (channel, line) in maps {
        let Some(map) = line.and_then(MapLine::parse) else {
            continue;
        };
        let space = if channel.is_colour() {
            ColorSpace::Srgb
        } else {
            ColorSpace::Linear
        };
        let texture = pictures.texture(&map.path, space);
        let mut slot = TextureSlot::new(texture);
        slot.tiling = map.scale;
        // OBJ's V runs up the picture: an offset there turns round.
        slot.offset = [map.offset[0], 1.0 - map.scale[1] - map.offset[1]];
        if let Some(source) = map.channel {
            slot.source = source;
        }
        if channel == Channel::Bump
            && let Some(height) = map.bump
        {
            material.bump_height = height;
        }
        *material.map_mut(channel) = Some(slot);
    }
    material
}

/// The `.mtl`'s pictures as linked textures, one per file and colour space.
struct MtlPictures<'a> {
    base: Option<&'a Path>,
    textures: Textures,
    made: HashMap<(PathBuf, bool), TextureId>,
}

impl MtlPictures<'_> {
    fn texture(&mut self, path: &str, space: ColorSpace) -> TextureId {
        // A relative path is next to the OBJ (backslashes as folder breaks too).
        let written = Path::new(path);
        let path = if written.is_absolute() || path.contains(':') {
            written.to_path_buf()
        } else {
            let relative = PathBuf::from(path.replace('\\', "/"));
            self.base.map_or_else(|| relative.clone(), |base| base.join(&relative))
        };
        let key = (path.clone(), space == ColorSpace::Srgb);
        if let Some(&id) = self.made.get(&key) {
            return id;
        }
        let mut texture = Texture::linked(path);
        texture.name = self.textures.unique_name(&texture.name);
        texture.color_space = space;
        let id = self.textures.insert(texture);
        self.made.insert(key, id);
        id
    }
}

/// A `.mtl` picture line: its file and the options Grid4D keeps.
#[derive(Clone, Debug, PartialEq)]
struct MapLine {
    path: String,
    scale: [f32; 2],
    offset: [f32; 2],
    bump: Option<f32>,
    channel: Option<PictureChannel>,
}

impl MapLine {
    /// `text` after the keyword: options (`-name values…`), then the file
    /// (which may hold spaces). `None` without a file.
    fn parse(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace().peekable();
        let mut line = Self {
            path: String::new(),
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            bump: None,
            channel: None,
        };
        let numbers = |words: &mut std::iter::Peekable<std::str::SplitWhitespace<'_>>, most: usize| {
            let mut out = Vec::new();
            while out.len() < most
                && let Some(value) = words.peek().and_then(|w| w.parse::<f32>().ok())
            {
                out.push(value);
                words.next();
            }
            out
        };
        while let Some(&word) = words.peek() {
            if !word.starts_with('-') || word.parse::<f32>().is_ok() {
                break;
            }
            words.next();
            match word {
                "-s" => {
                    let v = numbers(&mut words, 3);
                    if let [u, v, ..] = v[..]
                        && u.is_finite()
                        && v.is_finite()
                        && u.abs() > 1e-6
                        && v.abs() > 1e-6
                    {
                        line.scale = [u, v];
                    } else if let [u] = v[..]
                        && u.is_finite()
                        && u.abs() > 1e-6
                    {
                        line.scale = [u, u];
                    }
                }
                "-o" => {
                    let v = numbers(&mut words, 3);
                    if let [u, v, ..] = v[..]
                        && u.is_finite()
                        && v.is_finite()
                    {
                        line.offset = [u, v];
                    } else if let [u] = v[..]
                        && u.is_finite()
                    {
                        line.offset = [u, 0.0];
                    }
                }
                "-t" => {
                    numbers(&mut words, 3);
                }
                "-bm" | "-boost" | "-texres" => {
                    let v = numbers(&mut words, 1);
                    if word == "-bm"
                        && let Some(&height) = v.first().filter(|h| h.is_finite())
                    {
                        line.bump = Some(height);
                    }
                }
                "-mm" => {
                    numbers(&mut words, 2);
                }
                "-imfchan" => {
                    line.channel = match words.next() {
                        Some("g") => Some(PictureChannel::Green),
                        Some("b") => Some(PictureChannel::Blue),
                        Some("m") => Some(PictureChannel::Alpha),
                        _ => Some(PictureChannel::Red),
                    };
                }
                // On / off switches and anything else with one word after it.
                _ => {
                    words.next();
                }
            }
        }
        line.path = words.collect::<Vec<_>>().join(" ");
        (!line.path.is_empty()).then_some(line)
    }
}

/// The `.mtl` named by `mtllib`, read only from a path relative to the OBJ.
fn load_mtl(path: &Path, base: Option<&Path>) -> tobj::MTLLoadResult {
    let text = path.to_string_lossy();
    let relative = !path.is_absolute() && !text.contains(':') && !text.starts_with(['/', '\\']);
    let file = base
        .filter(|_| relative)
        .and_then(|base| crate::read_file(&base.join(path)).ok())
        .ok_or(tobj::LoadError::OpenFileFailed)?;
    tobj::load_mtl_buf(&mut file.as_slice())
}

/// One object's polygons: corners as written (a repeated corner dropped, faces
/// left with fewer than three skipped), texture coordinates per corner.
fn read_mesh(mesh: &tobj::Mesh, name: &str) -> Result<Option<PolyMesh>, IoError> {
    if !finite(&mesh.positions) {
        return Err(invalid(format!("object \"{name}\" has a non-finite position")));
    }
    let points: Vec<Vec3> = mesh
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&p| Vec3::from(p))
        .collect();
    let has_uvs = !mesh.texcoords.is_empty() && mesh.texcoord_indices.len() == mesh.indices.len();
    let uv_of = |corner: usize| -> Result<Vec2, IoError> {
        let index = usize::try_from(mesh.texcoord_indices[corner]).unwrap_or(usize::MAX);
        match (mesh.texcoords.get(2 * index), mesh.texcoords.get(2 * index + 1)) {
            (Some(&u), Some(&v)) if u.is_finite() && v.is_finite() => Ok(Vec2::new(u, 1.0 - v)),
            (Some(_), Some(_)) => Err(invalid(format!("object \"{name}\" has a non-finite UV"))),
            _ => Err(invalid(format!("object \"{name}\" has a UV index out of range"))),
        }
    };
    // All triangles when the arities are left out.
    let triangles = vec![3; mesh.indices.len() / 3];
    let arities = if mesh.face_arities.is_empty() {
        &triangles
    } else {
        &mesh.face_arities
    };

    let mut faces: Vec<Vec<u32>> = Vec::with_capacity(arities.len());
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut start = 0usize;
    for &arity in arities {
        let end = start + usize::try_from(arity).unwrap_or(0);
        let corners = mesh
            .indices
            .get(start..end)
            .ok_or_else(|| invalid(format!("object \"{name}\" has a broken face")))?;
        let mut face: Vec<u32> = Vec::with_capacity(corners.len());
        let mut face_uvs: Vec<Vec2> = Vec::with_capacity(corners.len());
        for (k, &point) in corners.iter().enumerate() {
            if usize::try_from(point).map_or(true, |p| p >= points.len()) {
                return Err(invalid(format!("object \"{name}\" has an index out of range")));
            }
            if face.contains(&point) {
                continue;
            }
            face.push(point);
            if has_uvs {
                face_uvs.push(uv_of(start + k)?);
            }
        }
        start = end;
        if face.len() >= 3 {
            faces.push(face);
            uvs.extend(face_uvs);
        }
    }
    if faces.is_empty() {
        return Ok(None);
    }
    let mut out = PolyMesh::from_faces(points, faces).map_err(|err| invalid(format!("object \"{name}\": {err}")))?;
    // Faces without texture coordinates get the reader's stand-in (one value for
    // every corner) when other objects of the file have some: such a constant map
    // carries nothing, so it is left out.
    let constant = uvs.windows(2).all(|w| w[0] == w[1]);
    if has_uvs && !constant {
        out.set_uvs(Some(uvs))
            .map_err(|err| invalid(format!("object \"{name}\": {err}")))?;
    }
    Ok(Some(out))
}

/// Writes `objects` as an OBJ file at `path`, with their colours in a `.mtl` of
/// the same name next to it.
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers, [`IoError::Io`] if a file cannot
/// be written.
pub fn export_obj(objects: &Objects, path: &Path) -> Result<(), IoError> {
    let mtl_path = path.with_extension("mtl");
    let mtl_name = mtl_path.file_name().map(|n| n.to_string_lossy().into_owned());
    let folder = path.parent();
    let (obj, mtl, files) = texts(objects, mtl_name.as_deref(), folder)?;
    // Embedded pictures go next to the OBJ, where the .mtl names them.
    for (name, bytes) in files {
        let target = folder.map_or_else(|| PathBuf::from(&name), |f| f.join(&name));
        write_atomic(&target, &bytes)?;
    }
    if !mtl.is_empty() {
        write_atomic(&mtl_path, mtl.as_bytes())?;
    }
    write_atomic(path, obj.as_bytes())
}

/// The OBJ text and its `.mtl` text (empty without polygon objects). `mtl_name`
/// is the `.mtl` file the OBJ refers to (`None`: no materials). Embedded
/// pictures are named by their file names (not written).
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers.
pub fn obj_text(objects: &Objects, mtl_name: Option<&str>) -> Result<(String, String), IoError> {
    texts(objects, mtl_name, None).map(|(obj, mtl, _)| (obj, mtl))
}

/// An embedded picture to write next to the OBJ: its file name and bytes.
type Picture = (String, Arc<[u8]>);

/// Where each picture a `.mtl` names is: a linked file by its path (relative
/// to `folder` when inside it), an embedded one by a file name next to the
/// OBJ, written there.
struct Names<'a> {
    folder: Option<&'a Path>,
    named: HashMap<TextureId, Option<String>>,
    /// Embedded pictures to write.
    files: Vec<Picture>,
}

impl Names<'_> {
    fn name(&mut self, id: TextureId, textures: &Textures) -> Option<String> {
        if let Some(named) = self.named.get(&id) {
            return named.clone();
        }
        let named = textures.get(id).map(|texture| match &texture.source {
            TextureSource::Linked(path) => self
                .folder
                .and_then(|folder| path.strip_prefix(folder).ok())
                .map_or_else(
                    || path.to_string_lossy().into_owned(),
                    |relative| relative.to_string_lossy().replace('\\', "/"),
                ),
            TextureSource::Embedded { file_name, bytes } => {
                let file: String = file_name
                    .chars()
                    .map(|c| {
                        if c.is_whitespace() || c.is_control() || "/\\:".contains(c) {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
                let file = if file.is_empty() {
                    "picture.png".to_owned()
                } else {
                    file
                };
                // Two pictures with one name stay apart.
                let mut name = file.clone();
                let mut n = 1;
                while self.files.iter().any(|(taken, _)| *taken == name) {
                    let stem = Path::new(&file)
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    let extension = Path::new(&file)
                        .extension()
                        .map_or_else(String::new, |e| format!(".{}", e.to_string_lossy()));
                    name = format!("{stem}_{n}{extension}");
                    n += 1;
                }
                self.files.push((name.clone(), Arc::clone(bytes)));
                name
            }
        });
        self.named.insert(id, named.clone());
        named
    }
}

/// `material`'s `.mtl` lines after `newmtl`.
fn material_lines(mtl: &mut String, m: &Material, textures: &Textures, names: &mut Names<'_>) {
    let [r, g, b] = m.color;
    let _ = write!(mtl, "Kd {r} {g} {b}\nKa 0 0 0\nKs 0 0 0\n");
    if m.emission != [0.0; 3] {
        let [r, g, b] = m.emitted();
        let _ = writeln!(mtl, "Ke {r} {g} {b}");
    }
    let _ = writeln!(mtl, "Pr {}\nPm {}", m.roughness, m.metallic);
    if (m.ior - 1.5).abs() > 1e-6 {
        let _ = writeln!(mtl, "Ni {}", m.ior);
    }
    if m.clearcoat > 0.0 {
        let _ = writeln!(mtl, "Pc {}\nPcr {}", m.clearcoat, m.clearcoat_roughness);
    }
    let keys = [
        (Channel::BaseColor, "map_Kd"),
        (Channel::Emission, "map_Ke"),
        (Channel::Opacity, "map_d"),
        (Channel::Bump, "bump"),
        (Channel::Normal, "norm"),
        (Channel::Roughness, "map_Pr"),
        (Channel::Metallic, "map_Pm"),
    ];
    for (channel, key) in keys {
        let Some(slot) = m.map(channel) else { continue };
        let Some(file) = names.name(slot.texture, textures) else {
            continue;
        };
        let mut line = key.to_owned();
        let [su, sv] = slot.tiling;
        if [su, sv] != [1.0, 1.0] {
            let _ = write!(line, " -s {su} {sv} 1");
        }
        // OBJ's V runs up the picture: the offset turns round.
        let (ou, ov) = (slot.offset[0], 1.0 - sv - slot.offset[1]);
        if ou != 0.0 || ov.abs() > 1e-6 {
            let _ = write!(line, " -o {ou} {ov} 0");
        }
        if channel == Channel::Bump {
            let _ = write!(line, " -bm {}", m.bump_height);
        }
        if !channel.is_colour() && channel != Channel::Normal && slot.source != PictureChannel::Red {
            let name = match slot.source {
                PictureChannel::Green => "g",
                PictureChannel::Blue => "b",
                PictureChannel::Alpha => "m",
                PictureChannel::Red => "r",
            };
            let _ = write!(line, " -imfchan {name}");
        }
        let _ = writeln!(mtl, "{line} {file}");
    }
}

/// [`obj_text`], and the embedded pictures to write next to the OBJ (named
/// relative to `folder`).
fn texts(
    objects: &Objects,
    mtl_name: Option<&str>,
    folder: Option<&Path>,
) -> Result<(String, String, Vec<Picture>), IoError> {
    let mut names = Names {
        folder,
        named: HashMap::new(),
        files: Vec::new(),
    };
    let worlds = world_matrices(objects);
    let mut obj = format!("# Grid4D {}\n", env!("CARGO_PKG_VERSION"));
    let mut mtl = String::new();
    let mut material_of: HashMap<Look, String> = HashMap::new();
    let effective = objects.effective_materials();
    let mut triangulator = Triangulator::new();
    let mut tri = TriMesh::default();
    // Running 1-based offsets of the v / vt / vn lists.
    let (mut v_base, mut vt_base, mut vn_base) = (1usize, 1usize, 1usize);
    let mut body = String::new();

    for ((entry, world), material) in objects.nodes.iter().zip(&worlds).zip(effective) {
        let data = &entry.data;
        // Only polygon objects have something to write (OBJ holds no cameras or lights).
        let mesh = match &data.kind {
            ObjectKind::Polygon { mesh, .. } if mesh.mesh().face_count() > 0 => mesh,
            ObjectKind::Polygon { .. }
            | ObjectKind::Null
            | ObjectKind::Cube { .. }
            | ObjectKind::Sphere { .. }
            | ObjectKind::Cylinder { .. }
            | ObjectKind::Plane { .. }
            | ObjectKind::Cone { .. }
            | ObjectKind::Torus { .. }
            | ObjectKind::Disc { .. }
            | ObjectKind::Capsule { .. }
            | ObjectKind::Camera { .. }
            | ObjectKind::Light { .. }
            | ObjectKind::Sky(_)
            | ObjectKind::Environment(_)
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
            | ObjectKind::SplinePrimitive(_)
            | ObjectKind::Text(_)
            | ObjectKind::Spline { .. }
            | ObjectKind::Plugin(_) => continue,
        };
        let mesh = mesh.mesh();
        if !finite(&world.to_cols_array()) {
            return Err(invalid(format!("\"{}\" has a non-finite transform", data.name)));
        }
        let normal_matrix = Mat3::from_mat4(*world).inverse().transpose();
        // Per-corner normals in the mesh's corner order (what the viewport shows).
        triangulator.triangulate(mesh, Smoothing::Angle(data.tags.smoothing_angle()), &mut tri);

        let _ = writeln!(body, "o {}", one_line(&data.name));
        if mtl_name.is_some() {
            // Its material (own or inherited) under its name, else its display colour.
            let material = material.and_then(|id| objects.materials.get(id).map(|m| (id, m)));
            let key = match material {
                Some((id, _)) => Look::Material(id),
                None => Look::Colour(data.display_color.map(f32::to_bits)),
            };
            if !material_of.contains_key(&key) {
                let next = material_of.len() + 1;
                let (name, colour, emission) = match material {
                    Some((_, m)) => {
                        let name: String = m
                            .name
                            .chars()
                            .map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c })
                            .collect();
                        // Two materials with the same name stay apart.
                        let name = if name.is_empty() || material_of.values().any(|n| *n == name) {
                            format!("{name}_{next}")
                        } else {
                            name
                        };
                        (name, m.color, m.emission)
                    }
                    None => (format!("color_{next}"), data.display_color, [0.0; 3]),
                };
                if !finite(&colour) || !finite(&emission) {
                    return Err(invalid(format!("\"{}\" has a non-finite colour", data.name)));
                }
                let _ = writeln!(mtl, "newmtl {name}");
                match material {
                    Some((_, m)) => {
                        if !finite(&m.numbers()) {
                            return Err(invalid(format!("material \"{}\" is not finite", m.name)));
                        }
                        material_lines(&mut mtl, m, &objects.textures, &mut names);
                    }
                    None => {
                        let [r, g, b] = colour;
                        let _ = write!(mtl, "Kd {r} {g} {b}\nKa 0 0 0\nKs 0 0 0\n");
                    }
                }
                let _ = writeln!(mtl, "illum 2\n");
                material_of.insert(key, name);
            }
            if let Some(name) = material_of.get(&key) {
                let _ = writeln!(body, "usemtl {name}");
            }
        }
        for &p in mesh.points() {
            let p = world.transform_point3(p);
            if !finite(&p.to_array()) {
                return Err(invalid(format!("\"{}\" has non-finite points", data.name)));
            }
            let _ = writeln!(body, "v {} {} {}", p.x, p.y, p.z);
        }
        // Texture coordinates and normals, each written once per distinct value.
        let uvs = mesh.uvs();
        let mut vt_index: HashMap<[u32; 2], usize> = HashMap::new();
        let mut vn_index: HashMap<[u32; 3], usize> = HashMap::new();
        let mut corner_refs: Vec<(Option<usize>, usize)> = Vec::with_capacity(mesh.corner_count());
        for corner in 0..mesh.corner_count() {
            let vt = match uvs {
                Some(uvs) => {
                    let uv = uvs[corner];
                    if !finite(&uv.to_array()) {
                        return Err(invalid(format!("\"{}\" has non-finite UVs", data.name)));
                    }
                    let flipped = [uv.x, 1.0 - uv.y];
                    let next = vt_index.len();
                    let index = *vt_index.entry(flipped.map(|v| (v + 0.0).to_bits())).or_insert_with(|| {
                        let _ = writeln!(body, "vt {} {}", flipped[0], flipped[1]);
                        next
                    });
                    Some(vt_base + index)
                }
                None => None,
            };
            let n = (normal_matrix * tri.normals.get(corner).copied().unwrap_or(Vec3::Y)).normalize_or(Vec3::Y);
            let next = vn_index.len();
            let index = *vn_index
                .entry(n.to_array().map(|v| (v + 0.0).to_bits()))
                .or_insert_with(|| {
                    let _ = writeln!(body, "vn {} {} {}", n.x, n.y, n.z);
                    next
                });
            corner_refs.push((vt, vn_base + index));
        }
        let mirrored = world.determinant() < 0.0;
        for f in 0..mesh.face_count() {
            let range = mesh.corner_range(f);
            body.push('f');
            let mut corners: Vec<usize> = range.collect();
            if mirrored {
                corners.reverse(); // a mirroring transform would turn the faces inside out
            }
            for corner in corners {
                let v = v_base + usize::try_from(mesh.corner_points()[corner]).unwrap_or(0);
                let (vt, vn) = corner_refs[corner];
                let _ = match vt {
                    Some(vt) => write!(body, " {v}/{vt}/{vn}"),
                    None => write!(body, " {v}//{vn}"),
                };
            }
            body.push('\n');
        }
        v_base += mesh.point_count();
        vt_base += vt_index.len();
        vn_base += vn_index.len();
    }
    if let Some(name) = mtl_name.filter(|_| !material_of.is_empty()) {
        let _ = writeln!(obj, "mtllib {name}");
    }
    obj.push_str(&body);
    Ok((obj, mtl, names.files))
}

/// Each object's world matrix (parents come first in `objects`).
fn world_matrices(objects: &Objects) -> Vec<Mat4> {
    let mut worlds: Vec<Mat4> = Vec::with_capacity(objects.nodes.len());
    for (index, entry) in objects.nodes.iter().enumerate() {
        let local = entry.data.transform.matrix();
        let parent = entry.parent.filter(|&p| p < index).and_then(|p| worlds.get(p));
        worlds.push(parent.map_or(local, |parent| *parent * local));
    }
    worlds
}

/// A name on one line (OBJ statements end at the line break).
fn one_line(name: &str) -> String {
    let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    match name.trim() {
        "" => "object".to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use grid4d_geom::{Topology, primitives};
    use grid4d_scene::Transform;

    use super::*;

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grid4d-obj-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mesh_of(objects: &Objects, name: &str) -> MeshHandle {
        let node = objects.nodes.iter().find(|n| n.data.name == name).unwrap();
        let ObjectKind::Polygon { mesh, .. } = &node.data.kind else {
            panic!("{name} is not a polygon object")
        };
        mesh.clone()
    }

    /// A quad, a triangle (negative indices) and a pentagon in two groups.
    const SHAPES: &str = "\
# test
mtllib shapes.mtl
o Panel
usemtl red
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
f 1/1 2/2 3/3 4/4
f -4/1 -2/3 -1/4
g Star
v 0 0 2
v 2 0 2
v 3 1 2
v 1 3 2
v -1 1 2
f 5 6 7 8 9
l 5 6
";

    #[test]
    fn polygons_keep_their_corners_and_uvs_flip() {
        let dir = temp_dir("shapes");
        fs::write(dir.join("shapes.mtl"), "newmtl red\nKd 0.8 0.1 0.1\n").unwrap();
        let objects = import_obj(SHAPES.as_bytes(), Some(&dir), "shapes").unwrap();
        let names: Vec<&str> = objects.nodes.iter().map(|n| n.data.name.as_str()).collect();
        assert_eq!(names, ["Panel", "Star"], "one object per o / g");
        assert!(
            objects
                .nodes
                .iter()
                .all(|n| n.parent.is_none() && n.data.transform == Transform::default())
        );

        let panel = mesh_of(&objects, "Panel");
        let panel = panel.mesh();
        assert_eq!((panel.point_count(), panel.face_count()), (4, 2));
        assert_eq!(panel.face(0).len(), 4, "the quad stays a quad");
        assert_eq!(panel.face(1).len(), 3, "negative indices: the triangle");
        let uvs = panel.uvs().unwrap();
        assert_eq!(uvs[0], Vec2::new(0.0, 1.0), "V flipped: bottom left → top left");
        assert_eq!(uvs[2], Vec2::new(1.0, 0.0));
        assert_eq!(objects.nodes[0].data.display_color, [0.8, 0.1, 0.1], "Kd from the .mtl");

        let star = mesh_of(&objects, "Star");
        assert_eq!(star.mesh().face(0).len(), 5, "the pentagon stays one polygon");
        assert!(star.mesh().uvs().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_absolute_material_files_leave_default_colours() {
        let dir = temp_dir("mtl");
        let mtl = dir.join("abs.mtl");
        fs::write(&mtl, "newmtl red\nKd 1 0 0\n").unwrap();
        let default = NodeData::new("x", ObjectKind::Null).display_color;
        for text in [
            SHAPES.to_owned(),
            SHAPES.replace("mtllib shapes.mtl", &format!("mtllib {}", mtl.display())),
        ] {
            let objects = import_obj(text.as_bytes(), Some(&dir), "shapes").unwrap();
            assert_eq!(
                objects.nodes[0].data.display_color,
                default,
                "{}",
                text.lines().nth(1).unwrap()
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_files_are_errors_and_unnamed_objects_get_the_file_name() {
        assert!(
            import_obj(b"v 0 0 0\nv 1 0 0\nf 1 2 9\n", None, "x").is_err(),
            "index out of range"
        );
        let err = import_obj(b"v nan 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", None, "x").unwrap_err();
        assert!(err.to_string().contains("non-finite"), "{err}");
        let objects = import_obj(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\nf 1 1 2\n", None, "Model").unwrap();
        assert_eq!(objects.nodes[0].data.name, "Model");
        assert_eq!(
            mesh_of(&objects, "Model").mesh().face_count(),
            1,
            "the degenerate face is skipped"
        );
    }

    /// Root (moved, turned) > Box (a cube); Pentagon (an n-gon with UVs, mirrored);
    /// Cam and Lamp (not written).
    #[test]
    fn materials_are_written_under_their_names_and_read_back() {
        use grid4d_scene::{Material, Tag};
        let mut objects = scene();
        let mut lamp = Material::new("Warm Lamp");
        lamp.color = [0.9, 0.8, 0.1];
        lamp.emission = [1.0, 0.5, 0.0];
        let lamp = objects.materials.insert(lamp);
        objects.nodes[0]
            .data
            .tags
            .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(lamp))));
        let (obj, mtl) = obj_text(&objects, Some("scene.mtl")).unwrap();
        assert!(mtl.contains("newmtl Warm_Lamp\nKd 0.9 0.8 0.1"), "{mtl}");
        assert!(mtl.contains("Ke 1 0.5 0"), "{mtl}");
        assert!(obj.contains("usemtl Warm_Lamp"));

        let dir = temp_dir("material_names");
        fs::write(dir.join("scene.mtl"), &mtl).unwrap();
        let back = import_obj(obj.as_bytes(), Some(&dir), "scene").unwrap();
        let (id, material) = back.materials.iter().find(|(_, m)| m.name == "Warm_Lamp").unwrap();
        assert_eq!(material.color, [0.9, 0.8, 0.1]);
        assert_eq!(material.emission, [1.0, 0.5, 0.0]);
        assert!(
            back.nodes.iter().any(|n| n.data.tags.material() == Some(id)),
            "objects get Material tags"
        );
    }

    fn scene() -> Objects {
        let mut pentagon = PolyMesh::from_faces(
            (0..5)
                .map(|k| {
                    let a = std::f32::consts::TAU * k as f32 / 5.0;
                    Vec3::new(a.cos() * 50.0, 0.0, -a.sin() * 50.0)
                })
                .collect(),
            [[0u32, 1, 2, 3, 4]],
        )
        .unwrap();
        pentagon
            .set_uvs(Some((0..5).map(|k| Vec2::new(k as f32 * 0.25, 0.5)).collect()))
            .unwrap();
        let entry = |parent, name: &str, kind| ObjectEntry {
            parent,
            data: NodeData::new(name, kind),
        };
        let mut nodes = vec![
            entry(None, "Root", ObjectKind::Null),
            entry(
                Some(0),
                "Box",
                ObjectKind::polygon(MeshHandle::new(primitives::cube(Vec3::splat(100.0), [1, 1, 1]))),
            ),
            entry(None, "Pentagon", ObjectKind::polygon(MeshHandle::new(pentagon))),
            entry(None, "Cam", ObjectKind::Camera { fov_y_deg: 40.0 }),
            entry(None, "Lamp", ObjectKind::light([1.0; 3], 1.0)),
        ];
        nodes[0].data.transform = Transform {
            position: Vec3::new(200.0, 50.0, 0.0),
            rotation: Vec3::new(0.5, 0.0, 0.2),
            scale: Vec3::ONE,
        };
        nodes[1].data.display_color = [0.2, 0.6, 0.9];
        nodes[2].data.transform.scale = Vec3::new(-1.0, 1.0, 1.0);
        Objects {
            nodes,
            ..Objects::default()
        }
    }

    #[test]
    fn export_then_import_keeps_world_points_polygons_uvs_and_colours() {
        let original = scene();
        let dir = temp_dir("round_trip");
        let path = dir.join("my scene.obj");
        export_obj(&original, &path).unwrap();
        assert!(dir.join("my scene.mtl").is_file());
        let back = crate::import(&path).unwrap();
        let names: Vec<&str> = back.nodes.iter().map(|n| n.data.name.as_str()).collect();
        assert_eq!(names, ["Box", "Pentagon"], "cameras, lights and Nulls are not written");

        let worlds = world_matrices(&original);
        for (index, name) in [(1usize, "Box"), (2, "Pentagon")] {
            let a = mesh_of(&original, name);
            let b = mesh_of(&back, name);
            let (a, b) = (a.mesh(), b.mesh());
            assert_eq!(b.point_count(), a.point_count(), "{name}");
            // (The reader numbers points in the order the faces use them.)
            for p in a.points() {
                let p = worlds[index].transform_point3(*p);
                assert!(
                    b.points().iter().any(|q| p.abs_diff_eq(*q, 1e-3)),
                    "{name}: world-space point {p}"
                );
            }
            assert_eq!(b.face_count(), a.face_count(), "{name}: polygons, not triangles");
            for f in 0..a.face_count() {
                assert_eq!(b.face(f).len(), a.face(f).len(), "{name}: face {f} keeps its corners");
            }
        }
        // Mirrored by its scale: written turned back, so the polygon still faces up.
        let pentagon = mesh_of(&back, "Pentagon");
        assert!(
            pentagon.mesh().face_normal(0).y > 0.9,
            "{}",
            pentagon.mesh().face_normal(0)
        );
        let uvs = pentagon.mesh().uvs().unwrap();
        let mut expected: Vec<Vec2> = mesh_of(&original, "Pentagon").mesh().uvs().unwrap().to_vec();
        expected.reverse();
        assert!(
            uvs.iter().zip(&expected).all(|(a, b)| a.abs_diff_eq(*b, 1e-6)),
            "{uvs:?}"
        );
        assert_eq!(back.nodes[0].data.display_color, [0.2, 0.6, 0.9], "via the .mtl");
        assert!(
            Topology::build(mesh_of(&back, "Box").mesh()).is_closed(),
            "the cube stays closed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_text_has_one_material_per_colour_and_shared_normals() {
        let (obj, mtl) = obj_text(&scene(), Some("scene.mtl")).unwrap();
        assert!(obj.contains("mtllib scene.mtl"));
        assert_eq!(mtl.matches("newmtl").count(), 2, "two colours");
        let count = |prefix: &str| obj.lines().filter(|l| l.starts_with(prefix)).count();
        assert_eq!((count("o "), count("v "), count("f ")), (2, 8 + 5, 6 + 1));
        assert_eq!(
            count("vn "),
            6 + 1,
            "a hard cube needs six normals, the flat pentagon one"
        );
        assert_eq!(count("vt "), 4 + 5, "the cube's picture corners, the pentagon's own");
        let (plain, no_mtl) = obj_text(&scene(), None).unwrap();
        assert!(!plain.contains("mtllib") && !plain.contains("usemtl") && no_mtl.is_empty());
        assert!(obj_text(&Objects::default(), Some("x.mtl")).unwrap().1.is_empty());
    }

    #[test]
    fn picture_lines_keep_their_file_and_options() {
        let line = MapLine::parse("-bm 0.5 -s 2 3 1 -o 0.25 0.5 0 -clamp on -imfchan g maps/wood grain.png").unwrap();
        assert_eq!(line.path, "maps/wood grain.png", "a name with a space");
        assert_eq!((line.scale, line.offset), ([2.0, 3.0], [0.25, 0.5]));
        assert_eq!((line.bump, line.channel), (Some(0.5), Some(PictureChannel::Green)));
        assert_eq!(MapLine::parse("plain.png").unwrap().scale, [1.0, 1.0]);
        assert!(MapLine::parse("-s 2 2").is_none(), "no file");
    }

    #[test]
    fn a_mtl_with_pictures_and_pbr_numbers_brings_them_in() {
        let dir = temp_dir("mtl_pictures");
        fs::write(
            dir.join("shapes.mtl"),
            "newmtl red\nKd 1 1 1\nNs 100\nPm 0.75\nPc 0.5\nPcr 0.1\nNi 1.33\n\
             map_Kd -s 2 2 1 -o 0.5 0 0 maps/paint.png\n\
             map_Ke glow.png\n\
             map_d -imfchan m maps/paint.png\n\
             bump -bm 0.3 maps/height.png\n\
             norm maps/normal.png\n\
             map_Pr -imfchan g maps/packed.png\n\
             map_Pm -imfchan b maps/packed.png\n",
        )
        .unwrap();
        let objects = import_obj(SHAPES.as_bytes(), Some(&dir), "Shapes").unwrap();
        let (_, red) = objects.materials.iter().find(|(_, m)| m.name == "red").unwrap();
        // Shininess 100 is a rough guess at roughness; the PBR numbers come as written.
        assert!((red.roughness - (2.0f32 / 102.0).sqrt()).abs() < 1e-5);
        assert_eq!(
            (red.metallic, red.clearcoat, red.clearcoat_roughness, red.ior),
            (0.75, 0.5, 0.1, 1.33)
        );
        let slot = |channel| *red.map(channel).unwrap_or_else(|| panic!("{channel:?}"));
        let base = slot(Channel::BaseColor);
        // V turned round: −o's 0 becomes 1 − 2 − 0.
        assert_eq!((base.tiling, base.offset), ([2.0, 2.0], [0.5, -1.0]));
        let texture = |slot: TextureSlot| objects.textures.get(slot.texture).unwrap();
        assert_eq!(texture(base).source, TextureSource::Linked(dir.join("maps/paint.png")));
        assert_eq!(texture(base).color_space, ColorSpace::Srgb);
        assert_eq!(slot(Channel::Opacity).source, PictureChannel::Alpha);
        assert_eq!(
            texture(slot(Channel::Opacity)).color_space,
            ColorSpace::Linear,
            "data: its own texture"
        );
        assert!((red.bump_height - 0.3).abs() < 1e-6);
        let _ = slot(Channel::Normal);
        let _ = slot(Channel::Emission);
        let (rough, metal) = (slot(Channel::Roughness), slot(Channel::Metallic));
        assert_eq!(
            (rough.source, metal.source),
            (PictureChannel::Green, PictureChannel::Blue)
        );
        assert_eq!(rough.texture, metal.texture);
        // paint (colour), paint (data), glow, height, normal, packed.
        assert_eq!(objects.textures.len(), 6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pictures_go_out_with_the_mtl_and_come_back() {
        let dir = temp_dir("mtl_out");
        fs::create_dir_all(dir.join("maps")).unwrap();
        let mut objects = scene();
        let wood = objects
            .textures
            .insert(Texture::linked(dir.join("maps").join("wood.png")));
        let stamp = objects.textures.insert(Texture {
            name: "Stamp".to_owned(),
            source: TextureSource::Embedded {
                file_name: "stamp.png".to_owned(),
                bytes: vec![1u8, 2, 3].into(),
            },
            color_space: ColorSpace::Linear,
        });
        let mut painted = Material::new("Painted");
        (painted.roughness, painted.metallic, painted.bump_height) = (0.4, 0.2, 0.7);
        *painted.map_mut(Channel::BaseColor) = Some(TextureSlot {
            tiling: [3.0, 2.0],
            offset: [0.25, 0.5],
            ..TextureSlot::new(wood)
        });
        *painted.map_mut(Channel::Bump) = Some(TextureSlot::new(stamp));
        *painted.map_mut(Channel::Roughness) = Some(TextureSlot {
            source: PictureChannel::Blue,
            ..TextureSlot::new(stamp)
        });
        let painted = objects.materials.insert(painted);
        objects.nodes[0]
            .data
            .tags
            .push(Tag::Material(grid4d_scene::MaterialTag::new(Some(painted))));
        let path = dir.join("scene.obj");
        export_obj(&objects, &path).unwrap();
        let mtl = fs::read_to_string(dir.join("scene.mtl")).unwrap();
        assert!(mtl.contains("map_Kd -s 3 2 1 -o 0.25 -1.5 0 maps/wood.png"), "{mtl}");
        assert!(mtl.contains("bump -bm 0.7 stamp.png"), "{mtl}");
        assert!(mtl.contains("map_Pr -imfchan b stamp.png"), "{mtl}");
        assert_eq!(
            fs::read(dir.join("stamp.png")).unwrap(),
            [1, 2, 3],
            "the embedded picture next to it"
        );

        let back = import_obj(&fs::read(&path).unwrap(), Some(&dir), "scene").unwrap();
        let (_, found) = back.materials.iter().find(|(_, m)| m.name == "Painted").unwrap();
        assert_eq!((found.roughness, found.metallic, found.bump_height), (0.4, 0.2, 0.7));
        let base = *found.map(Channel::BaseColor).unwrap();
        assert_eq!(
            (base.tiling, base.offset),
            ([3.0, 2.0], [0.25, 0.5]),
            "offsets turn back"
        );
        assert_eq!(
            back.textures.get(base.texture).unwrap().source,
            TextureSource::Linked(dir.join("maps/wood.png"))
        );
        assert_eq!(found.map(Channel::Roughness).unwrap().source, PictureChannel::Blue);
        let _ = fs::remove_dir_all(&dir);
    }
}
