//! Wavefront OBJ import and export (see `docs/reference/interchange-obj.md`).
//! OBJ has no units (1 unit = 1 cm here), no hierarchy and no transforms: objects
//! come in at the origin, and go out with their points in world space. Polygons
//! keep all their corners both ways. Texture coordinates start at the bottom left
//! in OBJ and at the top left in Grid4D, so V is flipped both ways.

use std::{collections::HashMap, fmt::Write as _, fs, path::Path};

use glam::{Mat3, Mat4, Vec2, Vec3};
use grid4d_geom::{PolyMesh, Smoothing, TriMesh, Triangulator};
use grid4d_scene::{Material, MaterialId, Materials, MeshHandle, NodeData, ObjectKind, Tag, Tags};

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
    let material_ids: Vec<MaterialId> = mtl
        .iter()
        .map(|m| {
            let name = match m.name.trim() {
                "" => materials.unique_name("Mat"),
                name => name.to_owned(),
            };
            let mut material = Material::new(name);
            if let Some(colour) = m.diffuse.filter(|c| finite(c)) {
                material.color = colour.map(|c| c.max(0.0));
            }
            if let Some(emission) = m.emissive.filter(|c| finite(c)) {
                material.emission = emission.map(|c| c.max(0.0));
            }
            materials.insert(material)
        })
        .collect();

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
            data.tags.push(Tag::Material {
                material: Some(material),
            });
        }
        nodes.push(ObjectEntry { parent: None, data });
    }
    Ok(Objects {
        nodes,
        materials,
        sources: Vec::new(),
    })
}

/// The `.mtl` named by `mtllib`, read only from a path relative to the OBJ.
fn load_mtl(path: &Path, base: Option<&Path>) -> tobj::MTLLoadResult {
    let text = path.to_string_lossy();
    let relative = !path.is_absolute() && !text.contains(':') && !text.starts_with(['/', '\\']);
    let file = base
        .filter(|_| relative)
        .and_then(|base| fs::read(base.join(path)).ok())
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
    let (obj, mtl) = obj_text(objects, mtl_name.as_deref())?;
    if !mtl.is_empty() {
        write_atomic(&mtl_path, mtl.as_bytes())?;
    }
    write_atomic(path, obj.as_bytes())
}

/// The OBJ text and its `.mtl` text (empty without polygon objects). `mtl_name`
/// is the `.mtl` file the OBJ refers to (`None`: no materials).
///
/// # Errors
///
/// [`IoError::Invalid`] for non-finite numbers.
pub fn obj_text(objects: &Objects, mtl_name: Option<&str>) -> Result<(String, String), IoError> {
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
            | ObjectKind::SubdivisionSurface { .. }
            | ObjectKind::Extrude { .. }
            | ObjectKind::Lathe(_)
            | ObjectKind::Sweep(_)
            | ObjectKind::Loft(_)
            | ObjectKind::Cloner { .. }
            | ObjectKind::Bend(_)
            | ObjectKind::Twist(_)
            | ObjectKind::Taper(_)
            | ObjectKind::Bulge(_)
            | ObjectKind::Ffd { .. }
            | ObjectKind::SplinePrimitive(_)
            | ObjectKind::Text(_)
            | ObjectKind::Spline { .. } => continue,
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
                let [r, g, b] = colour;
                let _ = write!(mtl, "newmtl {name}\nKd {r} {g} {b}\nKa 0 0 0\nKs 0 0 0\n");
                if emission != [0.0; 3] {
                    let [r, g, b] = emission;
                    let _ = writeln!(mtl, "Ke {r} {g} {b}");
                }
                let _ = writeln!(mtl, "illum 1\n");
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
    Ok((obj, mtl))
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
    use grid4d_geom::{Topology, primitives};
    use grid4d_scene::Transform;

    use super::*;

    fn temp_dir(test: &str) -> std::path::PathBuf {
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
        objects.nodes[0].data.tags.push(Tag::Material { material: Some(lamp) });
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
        assert_eq!(count("vt "), 5);
        let (plain, no_mtl) = obj_text(&scene(), None).unwrap();
        assert!(!plain.contains("mtllib") && !plain.contains("usemtl") && no_mtl.is_empty());
        assert!(obj_text(&Objects::default(), Some("x.mtl")).unwrap().1.is_empty());
    }
}
