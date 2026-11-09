//! The scene as the renderer sees it: every visible surface as world-space
//! triangles in one bounding volume hierarchy, with their materials (Material
//! tags as layers, read at each hit), and the lights.

use std::collections::HashMap;

use glam::{Affine2, Mat3, Mat4, Vec2, Vec3, Vec4};
use grid4d_eval::{Drawable, EvaluatedLight, EvaluatedScene, MaterialLayers, MeshKey};
use grid4d_geom::{Bvh, BvhSplit, PolyMesh, TriMesh, Triangulator, uv};
use grid4d_scene::{Aabb, Falloff, LightShadow, LightType, MaterialId, Materials, NodeId, Projection};

use crate::environment::EnvironmentLight;
use crate::{
    RenderCamera,
    channels::Channels,
    lights::{AreaLight, Entry, LightTable, SurfacePoint},
    picture::Pictures,
};

/// How a surface reflects and gives off light at a point: its material there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Surface {
    /// Linear RGB.
    pub color: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    /// A dielectric's reflection strength (0.5: 4 %).
    pub specular: f32,
    /// Light it gives off, linear RGB.
    pub emission: Vec3,
    /// Light passing through (0 – 1), bent by `ior`.
    pub transmission: f32,
    pub ior: f32,
    /// A varnish over the rest (0 – 1) and its roughness.
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
}

impl Surface {
    /// An opaque surface without coat.
    pub(crate) const fn plain(color: Vec3, roughness: f32, metallic: f32, emission: Vec3) -> Self {
        Self {
            color,
            roughness,
            metallic,
            specular: 0.5,
            emission,
            transmission: 0.0,
            ior: 1.5,
            clearcoat: 0.0,
            clearcoat_roughness: 0.03,
        }
    }

    /// How much light passes through it (no metal transmits).
    pub(crate) fn transmitted(&self) -> f32 {
        self.transmission.clamp(0.0, 1.0) * (1.0 - self.metallic.clamp(0.0, 1.0))
    }
}

/// A light as the renderer uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Light {
    pub kind: LightType,
    pub position: Vec3,
    /// The direction it shines in, unit length.
    pub direction: Vec3,
    /// Colour × intensity, linear RGB.
    pub color: Vec3,
    /// Cosines of half the outer and inner cone angles (equal for a hard edge).
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub falloff: Falloff,
    pub radius: f32,
    pub shadow: LightShadow,
    /// Its bit in the objects' `unlit` masks (`None`: it lights everything).
    pub link: Option<u8>,
    /// Its light group (0: none).
    pub group: u16,
}

impl Light {
    fn new(light: &EvaluatedLight) -> Self {
        let details = &light.details;
        let half_cos = |degrees: f32| (degrees.to_radians() * 0.5).cos();
        let cos_outer = half_cos(details.outer_angle_deg);
        Self {
            kind: light.light_type,
            position: light.position,
            direction: light.direction,
            color: Vec3::from(light.color),
            cos_outer,
            cos_inner: if details.use_inner {
                half_cos(details.inner_angle_deg.min(details.outer_angle_deg))
            } else {
                cos_outer
            },
            falloff: details.falloff,
            radius: details.radius,
            shadow: light.shadow,
            link: light.link,
            group: light.group,
        }
    }
}

/// How a picture bends the shading normal at a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bend {
    None,
    /// A Normal picture: the tangent (world, `w` the side of up the picture)
    /// and the picture's normal in tangent space.
    Normal {
        tangent: Vec4,
        bent: Vec3,
    },
    /// A Bump picture: the surface's change per unit of u and of v (world)
    /// and the height's (cm).
    Bump {
        pu: Vec3,
        pv: Vec3,
        hu: f32,
        hv: f32,
    },
}

impl Bend {
    /// `n` (unit, turned towards the viewer) bent by the picture.
    pub(crate) fn apply(self, n: Vec3) -> Vec3 {
        match self {
            Self::None => n,
            Self::Normal { tangent, bent } => {
                let t = tangent.truncate();
                let Some(t) = (t - n * n.dot(t)).try_normalize() else {
                    return n;
                };
                let b = n.cross(t) * tangent.w;
                (t * bent.x + b * bent.y + n * bent.z).normalize_or(n)
            }
            Self::Bump { pu, pv, hu, hv } => {
                // The bumped surface's tangents p_u + h_u·n and p_v + h_v·n
                // span a normal tilted against the slope.
                let area = pu.cross(pv);
                let length = area.length();
                if length <= f32::MIN_POSITIVE {
                    return n;
                }
                let side = if area.dot(n) < 0.0 { -1.0 } else { 1.0 };
                let tilt = (n.cross(pv) * hu + pu.cross(n) * hv) * (side / length);
                (n + tilt).normalize_or(n)
            }
        }
    }
}

/// Where a ray met a surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    /// The triangle met (its index in the scene).
    pub triangle: usize,
    pub position: Vec3,
    /// The triangle's own normal (unit; its front, counter-clockwise side).
    pub geometric: Vec3,
    /// The smoothed normal there (unit; either side).
    pub shading: Vec3,
    pub surface: Surface,
    pub bend: Bend,
    /// The moment of the shutter interval (0 – 1) the ray was cast at.
    pub time: f32,
}

/// Where the triangles are over the shutter interval (motion blur): their
/// corners and smoothed normals at each of its moments, evenly spread from
/// its opening (0) to its closing (1); in between, blended from the two
/// moments around.
#[derive(Debug)]
struct Motion {
    points: Vec<Vec<Vec3>>,
    normals: Vec<Vec<Vec3>>,
}

impl Motion {
    /// `moments`' value at corner `i`, at `time`.
    fn at(moments: &[Vec<Vec3>], i: usize, time: f32) -> Vec3 {
        let last = moments.len().saturating_sub(1);
        let scaled = time.clamp(0.0, 1.0) * to_f32(last);
        let below = scaled.floor();
        // A moment index below 2¹⁶.
        let k = usize::from(u16::try_from(below as u32).unwrap_or(0)).min(last.saturating_sub(1));
        let f = scaled - to_f32(k);
        let get = |m: usize| {
            moments
                .get(m)
                .and_then(|points| points.get(i))
                .copied()
                .unwrap_or(Vec3::NAN)
        };
        if last == 0 {
            return get(0);
        }
        get(k).lerp(get(k + 1), f)
    }
}

/// A small count as a float.
fn to_f32(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

/// What a render reads beyond the evaluated scene to show materials: the
/// document's materials, the pictures of its textures and the cameras that
/// Camera projections look through.
#[derive(Clone, Copy, Debug)]
pub struct Texturing<'a> {
    pub materials: &'a Materials,
    pub pictures: &'a Pictures,
    /// The render's camera: Camera projections without a camera object.
    pub camera: RenderCamera,
    /// The picture's width per unit of height.
    pub aspect: f32,
    /// Camera objects, where they are this frame.
    pub cameras: &'a [(NodeId, RenderCamera)],
}

/// Where a layer's texture coordinates come from.
#[derive(Clone, Copy, Debug)]
enum Coords {
    /// Its material reads no picture.
    None,
    /// Three per triangle of the object, from this index of `uvs`.
    Corners(usize),
    /// Seen through a camera, the tag's turn, tiling and offset after.
    Camera(RenderCamera, Affine2),
}

/// One Material tag on an object.
#[derive(Clone, Copy, Debug)]
struct Layer {
    /// Into `RenderScene::materials`.
    material: usize,
    coords: Coords,
    /// Three per triangle, from this index of `tangents` (Normal pictures).
    tangents: Option<usize>,
}

/// One drawn object or clone.
#[derive(Clone, Copy, Debug)]
struct Object {
    /// Without layers: its drawable's surface.
    plain: Surface,
    /// Its layers in `RenderScene::layers` (start, end), bottom first.
    layers: (usize, usize),
    first_triangle: usize,
    /// The linked lights that do not light it (bits by their `link`).
    unlit: u64,
    /// Render passes: its object ID and its top material's ID (0: none).
    object_id: u32,
    material_id: u32,
}

/// Everything a render needs from the scene, built once per render.
#[derive(Debug)]
pub struct RenderScene {
    /// Every triangle as its own face of three points, in world space.
    triangles: PolyMesh,
    bvh: Bvh,
    /// Three smoothed normals per triangle.
    normals: Vec<Vec3>,
    objects: Vec<Object>,
    /// The object of each triangle.
    object_of: Vec<u32>,
    layers: Vec<Layer>,
    materials: Vec<Channels>,
    uvs: Vec<Vec2>,
    tangents: Vec<Vec4>,
    aspect: f32,
    lights: Vec<Light>,
    /// The area lights among `lights`.
    areas: Vec<AreaLight>,
    /// Lights with a surface by power: area lights and emitting triangles,
    /// and area lights alone (for renders whose emitting surfaces are not
    /// sampled).
    with_emitters: LightTable,
    areas_only: LightTable,
    /// What surrounds the scene (a Sky or an Environment).
    environment: Option<EnvironmentLight>,
    bounds: Option<(Vec3, Vec3)>,
    /// The triangles over the shutter interval; `None`: they stand still.
    motion: Option<Motion>,
    /// Render passes: the object IDs in use and the objects they name.
    object_ids: Vec<(u32, NodeId)>,
    /// The light groups' names (a light's `group` n names entry n − 1).
    light_groups: Vec<String>,
}

/// A point chosen on a light with a surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceSample {
    pub source: Entry,
    pub point: SurfacePoint,
}

/// What the build keeps between objects.
#[derive(Default)]
struct Build {
    points: Vec<Vec3>,
    normals: Vec<Vec3>,
    objects: Vec<Object>,
    object_of: Vec<u32>,
    layers: Vec<Layer>,
    materials: Vec<Channels>,
    material_index: HashMap<MaterialId, usize>,
    uvs: Vec<Vec2>,
    tangents: Vec<Vec4>,
    bounds: Option<(Vec3, Vec3)>,
    triangulator: Triangulator,
    tri: TriMesh,
    corner_tangents: Vec<Vec4>,
    /// Render passes: the object IDs in use and their objects.
    object_ids: Vec<(u32, NodeId)>,
}

/// One object to add: its mesh at unit size and how it is drawn.
struct Part<'a> {
    mesh: &'a PolyMesh,
    world: Mat4,
    draw: &'a Drawable,
    bounds: Option<Aabb>,
    layers: MaterialLayers,
    unlit: u64,
    object_id: u32,
}

impl RenderScene {
    /// The scene `evaluated` shows — evaluate it with
    /// [`EvaluatedScene::for_render`] for what the renderer shows — in its
    /// objects' colours and surfaces, without pictures or layers. Light and
    /// camera markers are left out.
    #[must_use]
    pub fn new(evaluated: &EvaluatedScene) -> Self {
        Self::build(evaluated, None)
    }

    /// The scene `evaluated` shows in its materials: every Material tag of
    /// an object, the later over the earlier where their Opacity lets them,
    /// with their pictures.
    #[must_use]
    pub fn textured(evaluated: &EvaluatedScene, texturing: &Texturing<'_>) -> Self {
        Self::build(evaluated, Some(texturing))
    }

    fn build(evaluated: &EvaluatedScene, texturing: Option<&Texturing<'_>>) -> Self {
        let mut b = Build::default();
        // Object IDs (render passes): the place in the evaluated order + 1;
        // clones take their Cloner's.
        let place: HashMap<NodeId, u32> = evaluated
            .nodes()
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, u32::try_from(i + 1).unwrap_or(u32::MAX)))
            .collect();
        let id_of = |id: NodeId| place.get(&id).copied().unwrap_or(0);
        // Objects, then clones.
        let objects = evaluated
            .nodes()
            .iter()
            .map(|n| (n.world, n.drawable, n.bounds, n.materials, n.unlit, n.id));
        let clones = evaluated
            .instances()
            .iter()
            .map(|c| (c.world, c.drawable, c.bounds, c.materials, c.unlit, c.owner));
        let mut named = std::collections::HashSet::new();
        for (world, drawable, bounds, layers, unlit, owner) in objects.chain(clones) {
            if drawable.is_some_and(|d| !d.marker) && named.insert(owner) {
                b.object_ids.push((id_of(owner), owner));
            }
            let Some(draw) = drawable.filter(|d| !d.marker) else {
                continue;
            };
            let generated;
            let mesh = match draw.key {
                MeshKey::Polygon(id) => match evaluated.mesh(id) {
                    Some(handle) => handle.mesh(),
                    None => continue,
                },
                key => match key.build() {
                    Some(mesh) => {
                        generated = mesh;
                        &generated
                    }
                    None => continue,
                },
            };
            let part = Part {
                mesh,
                world,
                draw: &draw,
                bounds,
                layers,
                unlit,
                object_id: id_of(owner),
            };
            b.add(evaluated, &part, texturing);
        }
        let faces = (0..b.object_of.len()).map(|t| {
            let first = u32::try_from(t * 3).unwrap_or(u32::MAX - 2);
            [first, first + 1, first + 2]
        });
        let triangles = PolyMesh::from_faces(b.points, faces).unwrap_or_default();
        let bvh = Bvh::build_with(&triangles, &[], BvhSplit::SurfaceArea);
        Self {
            triangles,
            bvh,
            normals: b.normals,
            objects: b.objects,
            object_of: b.object_of,
            layers: b.layers,
            materials: b.materials,
            uvs: b.uvs,
            tangents: b.tangents,
            aspect: texturing.map_or(1.0, |t| t.aspect),
            lights: evaluated.lights().iter().map(Light::new).collect(),
            areas: evaluated.lights().iter().filter_map(AreaLight::new).collect(),
            with_emitters: LightTable::default(),
            areas_only: LightTable::default(),
            environment: evaluated
                .environment()
                .map(|environment| EnvironmentLight::new(environment, texturing.map(|t| t.pictures))),
            bounds: b.bounds,
            motion: None,
            object_ids: b.object_ids,
            light_groups: evaluated.light_groups().to_vec(),
        }
        .with_light_tables()
    }

    /// The scene over a shutter interval (motion blur) from `moments`, the
    /// scene at moments spread evenly from the interval's opening to its
    /// closing: everything but where the triangles are comes from moment
    /// `base` (the one at the frame). An object whose triangles differ in
    /// number between moments (a changing generator, a Cloner whose count
    /// changes) stays where it is at `base`, as do all objects when the
    /// moments do not have the same objects. Returns the scene and how many
    /// objects stayed still that way.
    #[must_use]
    pub fn moving(mut moments: Vec<Self>, base: usize) -> (Self, usize) {
        if moments.len() < 2 || base >= moments.len() {
            let scene = if base < moments.len() {
                moments.swap_remove(base)
            } else {
                moments
                    .pop()
                    .unwrap_or_else(|| Self::new(&EvaluatedScene::for_render()))
            };
            return (scene, 0);
        }
        let end_of = |s: &Self, i: usize| s.objects.get(i + 1).map_or(s.object_of.len(), |o| o.first_triangle);
        let (points, normals, still) = {
            let at_base = &moments[base];
            let objects = at_base.objects.len();
            let mut still = vec![false; objects];
            let mut points = Vec::with_capacity(moments.len());
            let mut normals = Vec::with_capacity(moments.len());
            for moment in &moments {
                let mut p = at_base.triangles.points().to_vec();
                let mut n = at_base.normals.clone();
                if moment.objects.len() == objects {
                    for (i, flag) in still.iter_mut().enumerate() {
                        let (from, to) = (at_base.objects[i].first_triangle * 3, end_of(at_base, i) * 3);
                        let (m_from, m_to) = (moment.objects[i].first_triangle * 3, end_of(moment, i) * 3);
                        if to - from == m_to - m_from {
                            p[from..to].copy_from_slice(&moment.triangles.points()[m_from..m_to]);
                            n[from..to].copy_from_slice(&moment.normals[m_from..m_to]);
                        } else {
                            *flag = true;
                        }
                    }
                } else {
                    still.fill(true);
                }
                points.push(p);
                normals.push(n);
            }
            // Still objects: every moment as at `base`.
            for (i, _) in still.iter().enumerate().filter(|(_, s)| **s) {
                let (from, to) = (at_base.objects[i].first_triangle * 3, end_of(at_base, i) * 3);
                for (p, n) in points.iter_mut().zip(normals.iter_mut()) {
                    p[from..to].copy_from_slice(&at_base.triangles.points()[from..to]);
                    n[from..to].copy_from_slice(&at_base.normals[from..to]);
                }
            }
            (points, normals, still)
        };
        let mut scene = moments.swap_remove(base);
        let refs: Vec<&[Vec3]> = points.iter().map(Vec::as_slice).collect();
        scene.bvh = Bvh::build_with(&scene.triangles, &refs, BvhSplit::SurfaceArea);
        for moment in &points {
            for &p in moment {
                scene.bounds = Some(scene.bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))));
            }
        }
        scene.motion = Some(Motion { points, normals });
        (scene, still.iter().filter(|s| **s).count())
    }

    /// The object IDs of the Object ID pass and the objects they name (a
    /// Cloner's for its clones).
    #[must_use]
    pub fn object_ids(&self) -> &[(u32, NodeId)] {
        &self.object_ids
    }

    /// The light groups' names, as the Light Groups pass names its layers
    /// (`Group.<name>`, after `Group.Default`).
    #[must_use]
    pub fn light_groups(&self) -> &[String] {
        &self.light_groups
    }

    /// `triangle`'s object's IDs: (object, material).
    pub(crate) fn ids_of(&self, triangle: usize) -> (u32, u32) {
        self.object_of
            .get(triangle)
            .and_then(|&o| self.objects.get(usize::try_from(o).ok()?))
            .map_or((0, 0), |o| (o.object_id, o.material_id))
    }

    /// Whether the triangles move over the shutter interval.
    #[must_use]
    pub fn is_moving(&self) -> bool {
        self.motion.is_some()
    }

    /// `triangle`'s corners at `time`.
    fn corner_points(&self, triangle: usize, time: f32) -> Option<[Vec3; 3]> {
        let first = triangle * 3;
        match &self.motion {
            None => {
                let points = self.triangles.points().get(first..first + 3)?;
                Some([points[0], points[1], points[2]])
            }
            Some(motion) => Some(std::array::from_fn(|i| Motion::at(&motion.points, first + i, time))),
        }
    }

    /// `triangle`'s smoothed normals at `time`.
    fn corner_normals(&self, triangle: usize, time: f32) -> Option<[Vec3; 3]> {
        let first = triangle * 3;
        match &self.motion {
            None => {
                let normals = self.normals.get(first..first + 3)?;
                Some([normals[0], normals[1], normals[2]])
            }
            Some(motion) => Some(std::array::from_fn(|i| Motion::at(&motion.normals, first + i, time))),
        }
    }

    /// The nearest triangle hit by the ray at `time`, from `start`.
    fn cast(&self, start: Vec3, dir: Vec3, time: f32) -> Option<grid4d_geom::RayHit> {
        match &self.motion {
            None => self.bvh.raycast(self.triangles.points(), start, dir),
            Some(motion) => self.bvh.raycast_by(
                |p| Motion::at(&motion.points, usize::try_from(p).unwrap_or(usize::MAX), time),
                start,
                dir,
                |_| true,
            ),
        }
    }

    /// The scene with its tables of lights with a surface: area lights by
    /// their power, emitting triangles by their area times the brightness of
    /// their emission (the mean of four points, so pictures on Emission count
    /// roughly) in the same units (π × brightness × area × sides).
    fn with_light_tables(mut self) -> Self {
        const POINTS: [Vec3; 4] = [
            Vec3::new(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0),
            Vec3::new(0.6, 0.2, 0.2),
            Vec3::new(0.2, 0.6, 0.2),
            Vec3::new(0.2, 0.2, 0.6),
        ];
        let areas: Vec<(Entry, f32)> = self
            .areas
            .iter()
            .enumerate()
            .map(|(i, light)| (Entry::Area(u32::try_from(i).unwrap_or(u32::MAX)), light.power()))
            .filter(|&(_, power)| power > 0.0 && power.is_finite())
            .collect();
        let mut all = areas.clone();
        for (index, object) in self.objects.iter().enumerate() {
            let can_emit = if object.layers.0 == object.layers.1 {
                object.plain.emission.max_element() > 0.0
            } else {
                self.layers
                    .get(object.layers.0..object.layers.1)
                    .into_iter()
                    .flatten()
                    .filter_map(|layer| self.materials.get(layer.material))
                    .any(Channels::can_emit)
            };
            if !can_emit {
                continue;
            }
            let end = self
                .objects
                .get(index + 1)
                .map_or(self.object_of.len(), |next| next.first_triangle);
            for triangle in object.first_triangle..end {
                let Some(corners) = self.triangles.points().get(triangle * 3..triangle * 3 + 3) else {
                    continue;
                };
                let area = (corners[1] - corners[0]).cross(corners[2] - corners[0]).length() * 0.5;
                let brightness = POINTS
                    .iter()
                    .map(|w| corners[0] * w.x + corners[1] * w.y + corners[2] * w.z)
                    .map(|p| crate::shade::luminance(self.emission_at(triangle, p, 0.0)))
                    .sum::<f32>()
                    / 4.0;
                // Emission is two-sided.
                let power = 2.0 * std::f32::consts::PI * area * brightness;
                if power > 0.0 && power.is_finite() {
                    all.push((Entry::Triangle(u32::try_from(triangle).unwrap_or(u32::MAX)), power));
                }
            }
        }
        let (triangles, area_count) = (self.object_of.len(), self.areas.len());
        self.with_emitters = LightTable::new(&all, triangles, area_count);
        self.areas_only = LightTable::new(&areas, triangles, area_count);
        self
    }
}

impl Build {
    /// Adds `part`'s triangles, and its layers when `texturing`.
    fn add(&mut self, evaluated: &EvaluatedScene, part: &Part<'_>, texturing: Option<&Texturing<'_>>) {
        let draw = part.draw;
        let model = part.world * Mat4::from_scale(draw.scale);
        let linear = Mat3::from_mat4(model);
        let determinant = linear.determinant();
        if determinant.abs() <= f32::EPSILON {
            return; // flattened to nothing
        }
        let normal_matrix = linear.inverse().transpose();
        self.triangulator
            .triangulate(part.mesh, draw.shading.smoothing(), &mut self.tri);
        let index = u32::try_from(self.objects.len()).unwrap_or(u32::MAX);
        let first_triangle = self.object_of.len();
        let tri = &self.tri;
        for triangle in tri.indices.as_chunks::<3>().0 {
            for &corner in triangle {
                let corner = usize::try_from(corner).unwrap_or(usize::MAX);
                let (Some(&p), Some(&n)) = (tri.positions.get(corner), tri.normals.get(corner)) else {
                    continue;
                };
                let p = model.transform_point3(p);
                self.points.push(p);
                self.normals.push((normal_matrix * n).normalize_or_zero());
                self.bounds = Some(self.bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))));
            }
            self.object_of.push(index);
        }

        let layer_start = self.layers.len();
        if let Some(texturing) = texturing {
            for layer in evaluated.material_layers(part.layers) {
                let Some(material) = texturing.materials.get(layer.material) else {
                    continue;
                };
                let slot = *self.material_index.entry(layer.material).or_insert_with(|| {
                    self.materials.push(Channels::new(material, texturing.pictures));
                    self.materials.len() - 1
                });
                let Some(channels) = self.materials.get(slot) else {
                    continue;
                };
                let (pictured, normal_map) = (channels.has_pictures(), channels.has_normal_map());
                let mut coords = Coords::None;
                let mut tangents = None;
                if pictured && layer.projection == Projection::Camera {
                    let camera = layer
                        .camera
                        .and_then(|id| texturing.cameras.iter().find(|(c, _)| *c == id))
                        .map_or(texturing.camera, |&(_, camera)| camera);
                    coords = Coords::Camera(camera, layer.uv_transform);
                } else if pictured {
                    // The mesh is the object's at unit size: its scale first.
                    let projected = layer
                        .to_unit(part.bounds)
                        .map(|to_unit| uv::project(layer.projection, part.mesh, to_unit * Mat4::from_scale(draw.scale)))
                        .filter(|uvs| uvs.len() == self.tri.positions.len());
                    let mut corner_uvs = projected.unwrap_or_else(|| self.tri.uvs.clone());
                    if layer.uv_transform != Affine2::IDENTITY {
                        for uv in &mut corner_uvs {
                            *uv = layer.uv_transform.transform_point2(*uv);
                        }
                    }
                    coords = Coords::Corners(self.uvs.len());
                    let tri = &self.tri;
                    for &corner in &tri.indices {
                        let corner = usize::try_from(corner).unwrap_or(usize::MAX);
                        self.uvs.push(corner_uvs.get(corner).copied().unwrap_or(Vec2::ZERO));
                    }
                    if normal_map {
                        uv::tangents(
                            &tri.positions,
                            &tri.normals,
                            &corner_uvs,
                            &tri.indices,
                            &mut self.corner_tangents,
                        );
                        tangents = Some(self.tangents.len());
                        // A mirrored object turns the picture's up around.
                        let side = if determinant < 0.0 { -1.0 } else { 1.0 };
                        for &corner in &tri.indices {
                            let corner = usize::try_from(corner).unwrap_or(usize::MAX);
                            let t = self.corner_tangents.get(corner).copied().unwrap_or(Vec4::X);
                            self.tangents.push((linear * t.truncate()).extend(t.w * side));
                        }
                    }
                }
                self.layers.push(Layer {
                    material: slot,
                    coords,
                    tangents,
                });
            }
        }
        self.objects.push(Object {
            plain: Surface::plain(
                Vec3::from(draw.color),
                draw.surface.roughness,
                draw.surface.metallic,
                Vec3::from(draw.surface.emission),
            ),
            layers: (layer_start, self.layers.len()),
            first_triangle,
            unlit: part.unlit,
            object_id: part.object_id,
            material_id: texturing
                .and_then(|t| {
                    let top = evaluated.material_layers(part.layers).last()?;
                    let place = t.materials.ids().iter().position(|&m| m == top.material)?;
                    u32::try_from(place + 1).ok()
                })
                .unwrap_or(0),
        });
    }
}

/// The most cut-out surfaces a ray passes through.
const MAX_CUT_OUTS: u32 = 64;

impl RenderScene {
    /// The world bounds of every surface (min, max); `None` for an empty scene.
    #[must_use]
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.bounds
    }

    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.object_of.len()
    }

    pub(crate) fn lights(&self) -> &[Light] {
        &self.lights
    }

    /// What surrounds the scene.
    pub(crate) const fn environment(&self) -> Option<&EnvironmentLight> {
        self.environment.as_ref()
    }

    /// The area lights.
    pub(crate) fn areas(&self) -> &[AreaLight] {
        &self.areas
    }

    /// The table of lights with a surface: with emitting triangles or
    /// without.
    pub(crate) fn light_table(&self, emitters: bool) -> &LightTable {
        if emitters {
            &self.with_emitters
        } else {
            &self.areas_only
        }
    }

    /// A point on a light with a surface, chosen from `table` by power (`u`)
    /// and on the light as seen from `from` (`s`, `t`); the density includes
    /// the chance of choosing that light.
    pub(crate) fn sample_surface_light(
        &self,
        table: &LightTable,
        from: Vec3,
        (u, s, t): (f32, f32, f32),
        time: f32,
    ) -> Option<SurfaceSample> {
        let source = table.choose(u)?;
        let point = match source {
            Entry::Triangle(triangle) => {
                let triangle = usize::try_from(triangle).ok()?;
                let corners = self.corner_points(triangle, time)?;
                let cross = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let area = cross.length() * 0.5;
                if area <= f32::MIN_POSITIVE {
                    return None;
                }
                // Even over the triangle: the square root folds the unit square onto it.
                let r = s.sqrt();
                let (a, b) = (1.0 - r, r * (1.0 - t));
                SurfacePoint {
                    position: corners[0] * a + corners[1] * b + corners[2] * (1.0 - a - b),
                    normal: cross / (2.0 * area),
                    density: table.triangle_chance(triangle) / area,
                }
            }
            Entry::Area(index) => {
                let index = usize::try_from(index).ok()?;
                let mut point = self.areas.get(index)?.sample(from, s, t);
                point.density *= table.area_chance(index);
                point
            }
        };
        Some(SurfaceSample { source, point })
    }

    /// Whether the light with `link` lights `triangle`'s object (every light
    /// without a link does).
    pub(crate) fn lit_by(&self, triangle: usize, link: Option<u8>) -> bool {
        let Some(bit) = link else {
            return true;
        };
        self.object_of
            .get(triangle)
            .and_then(|&o| self.objects.get(usize::try_from(o).ok()?))
            .is_none_or(|object| object.unlit & (1u64 << bit) == 0)
    }

    /// The area of `triangle`.
    pub(crate) fn triangle_area(&self, triangle: usize) -> f32 {
        self.triangles
            .points()
            .get(triangle * 3..triangle * 3 + 3)
            .map_or(0.0, |c| (c[1] - c[0]).cross(c[2] - c[0]).length() * 0.5)
    }

    /// The light `triangle` gives off at `position` at `time` (none where it
    /// is cut out).
    pub(crate) fn emission_at(&self, triangle: usize, position: Vec3, time: f32) -> Vec3 {
        self.surface_at(triangle, position, time)
            .map_or(Vec3::ZERO, |(surface, _)| surface.emission)
    }

    /// The nearest surface the ray `origin + t · dir` meets at `time` (0 – 1
    /// over the shutter interval; passing where surfaces are cut out).
    pub(crate) fn intersect(&self, origin: Vec3, dir: Vec3, time: f32) -> Option<Hit> {
        let mut start = origin;
        for _ in 0..MAX_CUT_OUTS {
            let hit = self.cast(start, dir, time)?;
            let triangle = usize::try_from(hit.face).ok()?;
            let position = start + dir * hit.t;
            if let Some(found) = self.hit(triangle, position, time) {
                return Some(found);
            }
            start = position + dir * crate::trace::offset(position);
        }
        None
    }

    /// How much light passes along `origin + t · dir` before `distance`:
    /// none through an opaque surface, all where surfaces are cut out or
    /// belong to objects the light with `link` does not light (they cast no
    /// shadow of it), tinted through transmitting ones when `through_glass`
    /// (lights: their light cannot reach by refraction), none otherwise
    /// (emitting surfaces: paths refracted through the glass find them).
    pub(crate) fn transmittance(
        &self,
        origin: Vec3,
        dir: Vec3,
        distance: f32,
        through_glass: bool,
        link: Option<u8>,
        time: f32,
    ) -> Vec3 {
        let mut start = origin;
        let mut left = distance;
        let mut passed = Vec3::ONE;
        for _ in 0..MAX_CUT_OUTS {
            let Some(hit) = self.cast(start, dir, time) else {
                return passed;
            };
            if hit.t >= left {
                return passed;
            }
            let Ok(triangle) = usize::try_from(hit.face) else {
                return passed;
            };
            let position = start + dir * hit.t;
            let surface = if self.lit_by(triangle, link) {
                self.surface_at(triangle, position, time)
            } else {
                None
            };
            if let Some((surface, _)) = surface {
                let through = if through_glass { surface.transmitted() } else { 0.0 };
                if through <= 0.0 {
                    return Vec3::ZERO;
                }
                passed *= surface.color * through;
                if passed.max_element() <= 1e-4 {
                    return Vec3::ZERO;
                }
            }
            let step = crate::trace::offset(position);
            start = position + dir * step;
            left -= hit.t + step;
        }
        Vec3::ZERO
    }

    /// The triangle's corners (world) at `time` and its barycentric weights
    /// at `position`.
    fn corners(&self, triangle: usize, position: Vec3, time: f32) -> Option<([Vec3; 3], Vec3)> {
        let points = self.corner_points(triangle, time)?;
        let (a, e1, e2) = (points[0], points[1] - points[0], points[2] - points[0]);
        let p = position - a;
        let (d00, d01, d11) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let (d20, d21) = (p.dot(e1), p.dot(e2));
        let denominator = d00 * d11 - d01 * d01;
        let (wb, wc) = if denominator.abs() > f32::MIN_POSITIVE {
            (
                (d11 * d20 - d01 * d21) / denominator,
                (d00 * d21 - d01 * d20) / denominator,
            )
        } else {
            (0.0, 0.0)
        };
        Some((points, Vec3::new(1.0 - wb - wc, wb, wc)))
    }

    /// The surface where `triangle` is met at `position` and how its pictures
    /// bend the shading; `None` where every layer is cut out.
    fn surface_at(&self, triangle: usize, position: Vec3, time: f32) -> Option<(Surface, Bend)> {
        let object = self
            .object_of
            .get(triangle)
            .and_then(|&o| self.objects.get(usize::try_from(o).ok()?))?;
        let layers = self.layers.get(object.layers.0..object.layers.1)?;
        if layers.is_empty() {
            return Some((object.plain, Bend::None));
        }
        let (corners, weights) = self.corners(triangle, position, time)?;
        let points = &corners[..];
        let local = triangle - object.first_triangle;
        // From the top layer down, the first one that is there.
        for layer in layers.iter().rev() {
            let channels = self.materials.get(layer.material)?;
            let corner_uvs = self.layer_uvs(layer, local, points);
            let uv = match layer.coords {
                Coords::None => None,
                Coords::Corners(_) => corner_uvs.map(|[a, b, c]| a * weights.x + b * weights.y + c * weights.z),
                Coords::Camera(camera, transform) => camera
                    .screen(position, self.aspect)
                    .map(|s| transform.transform_point2(Vec2::new(0.5 + s.x * 0.5, 0.5 - s.y * 0.5))),
            };
            if !channels.is_solid(uv) {
                continue;
            }
            let surface = channels.surface(uv);
            let mut bend = Bend::None;
            if let (Some(uv), Some(corner_uvs)) = (uv, corner_uvs) {
                if let Some(bent) = channels.normal(Some(uv)) {
                    let tangent = layer
                        .tangents
                        .and_then(|start| self.tangents.get(start + local * 3..start + local * 3 + 3))
                        .map(|t| t[0] * weights.x + t[1] * weights.y + t[2] * weights.z)
                        .or_else(|| triangle_tangent(points, corner_uvs));
                    if let Some(tangent) = tangent {
                        bend = Bend::Normal { tangent, bent };
                    }
                } else if channels.has_bump_map()
                    && let Some((pu, pv)) = derivatives(points, corner_uvs)
                    && let Some((_, step)) = channels.height(uv)
                {
                    // Half a texel either way: within the cell the picture
                    // is read from, its slope there.
                    let half = step * 0.5;
                    let height = |du: f32, dv: f32| channels.height(uv + Vec2::new(du, dv)).map_or(0.0, |(h, _)| h);
                    let hu = (height(half.x, 0.0) - height(-half.x, 0.0)) / step.x;
                    let hv = (height(0.0, half.y) - height(0.0, -half.y)) / step.y;
                    bend = Bend::Bump { pu, pv, hu, hv };
                }
            }
            return Some((surface, bend));
        }
        None
    }

    /// A layer's coordinates at the triangle's corners (`local`: the
    /// triangle's number in its object); `None` without pictures.
    fn layer_uvs(&self, layer: &Layer, local: usize, points: &[Vec3]) -> Option<[Vec2; 3]> {
        match layer.coords {
            Coords::None => None,
            Coords::Corners(start) => {
                let at = start + local * 3;
                let uvs = self.uvs.get(at..at + 3)?;
                Some([uvs[0], uvs[1], uvs[2]])
            }
            Coords::Camera(camera, transform) => {
                let mut out = [Vec2::ZERO; 3];
                for (uv, &p) in out.iter_mut().zip(points) {
                    let s = camera.screen(p, self.aspect)?;
                    *uv = transform.transform_point2(Vec2::new(0.5 + s.x * 0.5, 0.5 - s.y * 0.5));
                }
                Some(out)
            }
        }
    }

    /// The hit on `triangle` at `position` at `time`; `None` where it is cut
    /// out.
    fn hit(&self, triangle: usize, position: Vec3, time: f32) -> Option<Hit> {
        let (surface, bend) = self.surface_at(triangle, position, time)?;
        let (points, weights) = self.corners(triangle, position, time)?;
        let normals = self.corner_normals(triangle, time)?;
        let geometric = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize_or(Vec3::Y);
        let shading =
            (normals[0] * weights.x + normals[1] * weights.y + normals[2] * weights.z).normalize_or(geometric);
        Some(Hit {
            triangle,
            position,
            geometric,
            shading,
            surface,
            bend,
            time,
        })
    }
}

/// How a triangle's world points change per unit of u and of v; `None` when
/// its coordinates have no area.
fn derivatives(points: &[Vec3], uvs: [Vec2; 3]) -> Option<(Vec3, Vec3)> {
    let (dp1, dp2) = (points[1] - points[0], points[2] - points[0]);
    let (d1, d2) = (uvs[1] - uvs[0], uvs[2] - uvs[0]);
    let det = d1.x * d2.y - d1.y * d2.x;
    if det.abs() <= 1e-12 || !det.is_finite() {
        return None;
    }
    let pu = (dp1 * d2.y - dp2 * d1.y) / det;
    let pv = (dp2 * d1.x - dp1 * d2.x) / det;
    Some((pu, pv))
}

/// A triangle's own tangent (along increasing u, `w` the side of up the
/// picture), where no tangents are stored (Camera projections).
fn triangle_tangent(points: &[Vec3], uvs: [Vec2; 3]) -> Option<Vec4> {
    let (pu, pv) = derivatives(points, uvs)?;
    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
    let t = pu.try_normalize()?;
    // Up the picture is decreasing v.
    let w = if normal.cross(t).dot(-pv) < 0.0 { -1.0 } else { 1.0 };
    Some(t.extend(w))
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Document, Node, ObjectKind, ObjectKindTag};

    use super::*;

    #[test]
    fn surfaces_become_world_triangles_and_markers_stay_out() {
        let mut doc = Document::new("scene");
        doc.scene
            .insert(Node::new("Cube", ObjectKindTag::Cube.default_kind()).with_position(Vec3::new(0.0, 100.0, 0.0)));
        doc.scene.insert(Node::new("Light", ObjectKind::light([1.0; 3], 1.0)));
        doc.scene
            .insert(Node::new("Camera", ObjectKindTag::Camera.default_kind()));
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let scene = RenderScene::new(&eval);
        assert_eq!(scene.triangle_count(), 12, "the cube only");
        let (min, max) = scene.bounds().unwrap();
        assert!(min.abs_diff_eq(Vec3::new(-100.0, 0.0, -100.0), 1e-3));
        assert!(max.abs_diff_eq(Vec3::new(100.0, 200.0, 100.0), 1e-3));
        assert_eq!(scene.lights().len(), 1);

        // A ray down onto the top face.
        let hit = scene.intersect(Vec3::new(10.0, 500.0, 20.0), Vec3::NEG_Y, 0.0).unwrap();
        assert!(hit.position.abs_diff_eq(Vec3::new(10.0, 200.0, 20.0), 1e-3));
        assert!(hit.geometric.y.abs() > 0.999 && hit.shading.abs_diff_eq(Vec3::Y, 1e-4));
        let above = Vec3::new(10.0, 500.0, 20.0);
        assert_eq!(
            scene.transmittance(above, Vec3::NEG_Y, 400.0, true, None, 0.0),
            Vec3::ZERO
        );
        assert_eq!(
            scene.transmittance(above, Vec3::NEG_Y, 250.0, true, None, 0.0),
            Vec3::ONE
        );
    }

    #[test]
    fn clones_are_rendered() {
        use grid4d_geom::cloner::{Cloner, Layout, Radial};
        let mut doc = Document::new("clones");
        let cloner = doc.scene.insert(Node::new(
            "Cloner",
            ObjectKind::cloner(Cloner {
                layout: Layout::Radial(Radial {
                    count: 8,
                    ..Radial::default()
                }),
                ..Cloner::default()
            }),
        ));
        doc.scene
            .insert_child(cloner, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let scene = RenderScene::new(&eval);
        assert_eq!(scene.triangle_count(), 8 * 12, "eight cubes");
        let (min, max) = scene.bounds().unwrap();
        assert!(min.x < -400.0 && max.x > 400.0, "around the ring: {min} {max}");
    }

    #[test]
    fn clones_take_their_cloners_id_and_materials_their_place() {
        use grid4d_geom::cloner::Cloner;
        use grid4d_scene::{Material, MaterialTag, Tag};
        let mut doc = Document::new("ids");
        let red = doc.scene.materials_mut().insert(Material::new("Red"));
        let blue = doc.scene.materials_mut().insert(Material::new("Blue"));
        let mut plane = Node::new("Plane", ObjectKindTag::Plane.default_kind());
        plane.data.tags = vec![Tag::Material(MaterialTag::new(Some(blue)))];
        doc.scene.insert(plane);
        let mut cloner = Node::new("Cloner", ObjectKind::cloner(Cloner::default()));
        cloner.data.tags = vec![Tag::Material(MaterialTag::new(Some(red)))];
        let cloner = doc.scene.insert(cloner);
        doc.scene
            .insert_child(cloner, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let pictures = Pictures::new();
        let texturing = Texturing {
            materials: doc.scene.materials(),
            pictures: &pictures,
            camera: RenderCamera::framing(Vec3::splat(-1.0), Vec3::ONE, 1.0),
            aspect: 1.0,
            cameras: &[],
        };
        let scene = RenderScene::textured(&eval, &texturing);
        let id_of = |node| scene.object_ids().iter().find(|&&(_, n)| n == node).map(|&(id, _)| id);
        let cloner_id = id_of(cloner).unwrap();
        let mut seen = Vec::new();
        for triangle in 0..scene.triangle_count() {
            let ids = scene.ids_of(triangle);
            if !seen.contains(&ids) {
                seen.push(ids);
            }
        }
        seen.sort_unstable();
        // The plane (Blue, second in the Material Manager) and every clone
        // as the Cloner (Red, first).
        let plane = seen.iter().find(|&&(id, _)| id != cloner_id).unwrap().0;
        let mut expected = vec![(plane, 2), (cloner_id, 1)];
        expected.sort_unstable();
        assert_eq!(seen, expected);
        // Without materials, no material IDs.
        let plain = RenderScene::new(&eval);
        assert!((0..plain.triangle_count()).all(|t| plain.ids_of(t).1 == 0));
    }

    #[test]
    fn rays_and_shadows_pass_where_surfaces_are_cut_out() {
        use std::sync::Arc;

        use grid4d_scene::{Channel, Material, MaterialTag, Tag, Texture, TextureSlot};

        use crate::picture::{Picture, Texels};

        let mut doc = Document::new("cut");
        let mask = doc.scene.textures_mut().insert(Texture::linked("mask.png".into()));
        let mut material = Material::new("Mask");
        *material.map_mut(Channel::Opacity) = Some(TextureSlot::new(mask));
        let material = doc.scene.materials_mut().insert(material);
        let mut plane = Node::new("Plane", ObjectKindTag::Plane.default_kind());
        plane.data.tags = vec![Tag::Material(MaterialTag::new(Some(material)))];
        doc.scene.insert(plane);
        let mut eval = EvaluatedScene::for_render();
        eval.update(&doc.scene);
        let mut pictures = Pictures::new();
        // Solid on the left half (u < 0.5: x < 0), cut out on the right.
        let texels = Texels::Linear8(Arc::from([255u8, 0, 0, 255, 0, 0, 0, 255]));
        pictures.insert(mask, Arc::new(Picture::new(2, 1, texels).unwrap()));
        let texturing = Texturing {
            materials: doc.scene.materials(),
            pictures: &pictures,
            camera: RenderCamera::framing(Vec3::splat(-1.0), Vec3::ONE, 1.0),
            aspect: 1.0,
            cameras: &[],
        };
        let scene = RenderScene::textured(&eval, &texturing);
        let (solid, cut) = (Vec3::new(-50.0, 100.0, 0.0), Vec3::new(50.0, 100.0, 0.0));
        assert!(scene.intersect(solid, Vec3::NEG_Y, 0.0).is_some());
        assert_eq!(
            scene.transmittance(solid, Vec3::NEG_Y, 200.0, true, None, 0.0),
            Vec3::ZERO
        );
        assert!(scene.intersect(cut, Vec3::NEG_Y, 0.0).is_none(), "cut out");
        assert_eq!(scene.transmittance(cut, Vec3::NEG_Y, 200.0, true, None, 0.0), Vec3::ONE);
        // Without materials it is whole.
        assert!(RenderScene::new(&eval).intersect(cut, Vec3::NEG_Y, 0.0).is_some());
    }
}
