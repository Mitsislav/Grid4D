//! Deformers 2 (spec `docs/reference/deformer-2.md`): Wrap, Follow Spline,
//! Displace, Project to Surface, Smooth and Explode. Each works on a whole
//! mesh given in the deformer's own space (some need its normals, edges or
//! polygons); links to other objects (a path, a target, a picture) are
//! handed in already placed in that space. Strength blends the result with
//! the input.

use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};

use crate::{
    BitSet, PolyMesh, Topology,
    spline::{Polyline, Segment},
    us,
};

/// The size (cm) of the guide drawn for a deformer without a box of its own.
pub const GUIDE_SIZE: f32 = 50.0;

/// The guide drawn for a deformer without a box: three crossed lines of
/// [`GUIDE_SIZE`] and a small diamond, so it can be seen and picked.
#[must_use]
pub fn marker_guide() -> Polyline {
    let h = GUIDE_SIZE * 0.5;
    let mut out = Polyline::default();
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        out.points.extend([-axis * h, axis * h]);
        out.segments.push(Segment {
            count: 2,
            closed: false,
        });
    }
    let q = h * 0.4;
    out.points
        .extend([Vec3::X * q, Vec3::Z * q, Vec3::NEG_X * q, Vec3::NEG_Z * q]);
    out.segments.push(Segment { count: 4, closed: true });
    out
}

/// What Wrap wraps onto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WrapShape {
    #[default]
    Cylinder,
    Sphere,
}

impl WrapShape {
    pub const ALL: [Self; 2] = [Self::Cylinder, Self::Sphere];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cylinder => "Cylinder",
            Self::Sphere => "Sphere",
        }
    }
}

/// Wrap: a flat region (X across Width, Y up Height, Z out) laid round a
/// cylinder or a sphere about the Y axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrap {
    pub shape: WrapShape,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    /// Where the width's ends go round (radians).
    pub start: f32,
    pub end: f32,
    /// 0 – 1.
    pub strength: f32,
}

impl Default for Wrap {
    fn default() -> Self {
        Self {
            shape: WrapShape::Cylinder,
            width: 400.0,
            height: 200.0,
            radius: 100.0,
            start: -std::f32::consts::PI,
            end: std::f32::consts::PI,
            strength: 1.0,
        }
    }
}

impl Wrap {
    /// `point` wrapped.
    #[must_use]
    pub fn apply(&self, point: Vec3) -> Vec3 {
        if self.width.abs() < 1e-6 || !point.is_finite() {
            return point;
        }
        let t = point.x / self.width + 0.5;
        let angle = self.start + (self.end - self.start) * t;
        let r = self.radius + point.z;
        let wrapped = match self.shape {
            WrapShape::Cylinder => Vec3::new(r * angle.sin(), point.y, r * angle.cos()),
            WrapShape::Sphere => {
                let latitude = if self.height.abs() < 1e-6 {
                    0.0
                } else {
                    (point.y / self.height).clamp(-0.5, 0.5) * std::f32::consts::PI
                };
                Vec3::new(
                    r * latitude.cos() * angle.sin(),
                    r * latitude.sin(),
                    r * latitude.cos() * angle.cos(),
                )
            }
        };
        point.lerp(wrapped, self.strength)
    }

    /// The shape it wraps onto, as drawn.
    #[must_use]
    pub fn guide(&self) -> Polyline {
        let mut out = Polyline::default();
        let circle = |out: &mut Polyline, y: f32, r: f32| {
            let n = 32u32;
            out.points.extend((0..n).map(|k| {
                let a = std::f32::consts::TAU * k as f32 / n as f32;
                Vec3::new(r * a.sin(), y, r * a.cos())
            }));
            out.segments.push(Segment { count: n, closed: true });
        };
        let half = self.height * 0.5;
        match self.shape {
            WrapShape::Cylinder => {
                circle(&mut out, -half, self.radius);
                circle(&mut out, half, self.radius);
            }
            WrapShape::Sphere => {
                circle(&mut out, 0.0, self.radius);
                circle(&mut out, self.radius * 0.5, self.radius * 0.866);
                circle(&mut out, -self.radius * 0.5, self.radius * 0.866);
            }
        }
        out
    }
}

/// One of the six axis directions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AxisDirection {
    PlusX,
    MinusX,
    #[default]
    PlusY,
    MinusY,
    PlusZ,
    MinusZ,
}

impl AxisDirection {
    pub const ALL: [Self; 6] = [
        Self::PlusX,
        Self::MinusX,
        Self::PlusY,
        Self::MinusY,
        Self::PlusZ,
        Self::MinusZ,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlusX => "+X",
            Self::MinusX => "-X",
            Self::PlusY => "+Y",
            Self::MinusY => "-Y",
            Self::PlusZ => "+Z",
            Self::MinusZ => "-Z",
        }
    }

    #[must_use]
    pub const fn vector(self) -> Vec3 {
        match self {
            Self::PlusX => Vec3::X,
            Self::MinusX => Vec3::NEG_X,
            Self::PlusY => Vec3::Y,
            Self::MinusY => Vec3::NEG_Y,
            Self::PlusZ => Vec3::Z,
            Self::MinusZ => Vec3::NEG_Z,
        }
    }
}

/// How Follow Spline spreads the object along the path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PathFit {
    /// The object keeps its length along the path.
    #[default]
    KeepLength,
    /// The object is stretched over the whole path.
    FitToSpline,
}

impl PathFit {
    pub const ALL: [Self; 2] = [Self::KeepLength, Self::FitToSpline];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::KeepLength => "Keep Length",
            Self::FitToSpline => "Fit to Spline",
        }
    }
}

/// Follow Spline's settings (the path is handed in).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowSpline {
    /// The object's axis laid along the path.
    pub axis: AxisDirection,
    pub fit: PathFit,
    /// Where along the path the object starts (share of its length).
    pub offset: f32,
    pub strength: f32,
}

impl Default for FollowSpline {
    fn default() -> Self {
        Self {
            axis: AxisDirection::PlusY,
            fit: PathFit::KeepLength,
            offset: 0.0,
            strength: 1.0,
        }
    }
}

impl FollowSpline {
    /// `points` laid along `path` (the deformer's space): each point's place
    /// along the axis becomes a distance along the path, its offset from the
    /// axis turned with the path (parallel transport). A closed path wraps.
    pub fn apply(&self, points: &mut [Vec3], path: &Polyline) {
        let Some((line, closed)) = path.runs().next() else {
            return;
        };
        let mut line: Vec<Vec3> = line.to_vec();
        if closed && let Some(&first) = line.first() {
            line.push(first);
        }
        let lengths: Vec<f32> = line.windows(2).map(|w| w[0].distance(w[1])).collect();
        let total: f32 = lengths.iter().sum();
        if line.len() < 2 || total <= 1e-6 {
            return;
        }
        let axis = self.axis.vector();
        let tangents: Vec<Vec3> = line.windows(2).map(|w| (w[1] - w[0]).normalize_or(axis)).collect();
        // The frame at each segment, carried along.
        let mut frames = Vec::with_capacity(tangents.len());
        let mut frame = Quat::from_rotation_arc(axis, tangents[0]);
        for (i, &t) in tangents.iter().enumerate() {
            if i > 0 {
                frame = (Quat::from_rotation_arc(tangents[i - 1], t) * frame).normalize();
            }
            frames.push(frame);
        }
        let along = |p: Vec3| p.dot(axis);
        let (lo, hi) = points.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &p| {
            (lo.min(along(p)), hi.max(along(p)))
        });
        if !lo.is_finite() {
            return;
        }
        let span = (hi - lo).max(1e-6);
        for p in points.iter_mut() {
            let s = along(*p) - lo;
            let mut d = total * self.offset
                + match self.fit {
                    PathFit::KeepLength => s,
                    PathFit::FitToSpline => s / span * total,
                };
            if closed {
                d = d.rem_euclid(total);
            }
            // The segment the distance falls on (beyond the ends: the end ones).
            let mut start = 0.0;
            let mut k = 0;
            while k + 1 < lengths.len() && start + lengths[k] < d {
                start += lengths[k];
                k += 1;
            }
            let on = line[k] + tangents[k] * (d - start);
            let offset = *p - axis * along(*p);
            let moved = on + frames[k] * offset;
            *p = p.lerp(moved, self.strength);
        }
    }
}

/// Where Displace reads its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplaceSource {
    #[default]
    Noise,
    Texture,
}

impl DisplaceSource {
    pub const ALL: [Self; 2] = [Self::Noise, Self::Texture];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Noise => "Noise",
            Self::Texture => "Texture",
        }
    }
}

/// Where a Displace picture is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplaceMapping {
    /// The mesh's UVs.
    #[default]
    Uv,
    /// Projected along the deformer's Z (its X and Y, 100 cm a picture).
    Flat,
}

impl DisplaceMapping {
    pub const ALL: [Self; 2] = [Self::Uv, Self::Flat];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uv => "UV",
            Self::Flat => "Flat",
        }
    }
}

/// Which way Displace moves points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplaceDirection {
    #[default]
    Normal,
    X,
    Y,
    Z,
}

impl DisplaceDirection {
    pub const ALL: [Self; 4] = [Self::Normal, Self::X, Self::Y, Self::Z];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }
}

/// Displace's settings (its picture is handed in).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Displace {
    pub source: DisplaceSource,
    pub mapping: DisplaceMapping,
    pub noise_scale: f32,
    pub octaves: u32,
    pub seed: u32,
    /// How far a value of 1 moves (cm).
    pub height: f32,
    /// The value that does not move.
    pub mid: f32,
    pub direction: DisplaceDirection,
    pub strength: f32,
}

impl Displace {
    /// The smallest noise scale (cm).
    pub const MIN_NOISE_SCALE: f32 = 0.01;
    pub const MAX_OCTAVES: u32 = 8;
}

impl Default for Displace {
    fn default() -> Self {
        Self {
            source: DisplaceSource::Noise,
            mapping: DisplaceMapping::Uv,
            noise_scale: 50.0,
            octaves: 3,
            seed: 0,
            height: 10.0,
            mid: 0.5,
            direction: DisplaceDirection::Normal,
            strength: 1.0,
        }
    }
}

/// A grey picture (0 – 1 per texel, rows top first) for Displace.
#[derive(Clone, Debug, PartialEq)]
pub struct GreyPicture {
    pub width: u32,
    pub height: u32,
    pub values: Arc<[f32]>,
}

impl GreyPicture {
    /// The value at `uv` (u right, v down, repeating), bilinear.
    #[must_use]
    pub fn sample(&self, uv: Vec2) -> f32 {
        if self.width == 0 || self.height == 0 || self.values.is_empty() || !uv.is_finite() {
            return 0.0;
        }
        let (w, h) = (self.width as f32, self.height as f32);
        let x = (uv.x.rem_euclid(1.0)) * w - 0.5;
        let y = (uv.y.rem_euclid(1.0)) * h - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| {
            let i = (i.rem_euclid(w)) as usize;
            let j = (j.rem_euclid(h)) as usize;
            self.values.get(j * self.width as usize + i).copied().unwrap_or(0.0)
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Smooth value noise in 0 – 1 (fractal, `octaves` layers).
#[must_use]
pub fn noise(p: Vec3, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut weight = 0.5;
    let mut total = 0.0;
    let mut q = p;
    for o in 0..octaves.clamp(1, Displace::MAX_OCTAVES) {
        sum += value_noise(q, seed.wrapping_add(o * 7919)) * weight;
        total += weight;
        weight *= 0.5;
        q *= 2.03;
    }
    if total > 0.0 { sum / total } else { 0.5 }
}

fn hash(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

fn value_noise(p: Vec3, seed: u32) -> f32 {
    let f = p.floor();
    let t = p - f;
    let s = t * t * (Vec3::splat(3.0) - t * 2.0);
    let (x, y, z) = (f.x as i32, f.y as i32, f.z as i32);
    let c = |dx: i32, dy: i32, dz: i32| hash(x + dx, y + dy, z + dz, seed);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), s.x);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), s.x);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), s.x);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), s.x);
    lerp(lerp(x00, x10, s.y), lerp(x01, x11, s.y), s.z)
}

/// Each point's normal: its polygons' area-weighted normals summed.
fn point_normals(mesh: &PolyMesh) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; mesh.point_count()];
    for f in 0..mesh.face_count() {
        let v = mesh.face_vector(f);
        for &p in mesh.face(f) {
            normals[us(p)] += v;
        }
    }
    normals.iter().map(|n| n.normalize_or_zero()).collect()
}

/// Each point's UV: the first of its corners' (zero without UVs).
fn point_uvs(mesh: &PolyMesh) -> Vec<Vec2> {
    let mut uvs = vec![Vec2::ZERO; mesh.point_count()];
    if let Some(corner_uvs) = mesh.uvs() {
        let mut seen = BitSet::new();
        for (c, &p) in mesh.corner_points().iter().enumerate() {
            if !seen.contains(us(p)) {
                seen.insert(us(p));
                uvs[us(p)] = corner_uvs[c];
            }
        }
    }
    uvs
}

impl Displace {
    /// `mesh`'s points moved by height × (value − mid) along `direction`;
    /// a Texture source without `picture` moves nothing.
    pub fn apply(&self, mesh: &mut PolyMesh, picture: Option<&GreyPicture>) {
        if self.source == DisplaceSource::Texture && picture.is_none() {
            return;
        }
        let normals = point_normals(mesh);
        let uvs = point_uvs(mesh);
        let scale = self.noise_scale.max(Self::MIN_NOISE_SCALE);
        for (p, point) in mesh.points_mut().iter_mut().enumerate() {
            let value = match (self.source, picture) {
                (DisplaceSource::Texture, Some(picture)) => {
                    let uv = match self.mapping {
                        DisplaceMapping::Uv => uvs[p],
                        DisplaceMapping::Flat => Vec2::new(point.x / 100.0 + 0.5, 0.5 - point.y / 100.0),
                    };
                    picture.sample(uv)
                }
                _ => noise(*point / scale, self.octaves, self.seed),
            };
            let along = match self.direction {
                DisplaceDirection::Normal => normals[p],
                DisplaceDirection::X => Vec3::X,
                DisplaceDirection::Y => Vec3::Y,
                DisplaceDirection::Z => Vec3::Z,
            };
            *point += along * (self.height * (value - self.mid) * self.strength);
        }
    }
}

/// How Project to Surface finds the surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProjectMode {
    #[default]
    NearestPoint,
    /// Along the deformer's −Z.
    AlongDirection,
    /// Along each point's normal, both ways (the nearer hit).
    AlongNormal,
}

impl ProjectMode {
    pub const ALL: [Self; 3] = [Self::NearestPoint, Self::AlongDirection, Self::AlongNormal];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NearestPoint => "Nearest Point",
            Self::AlongDirection => "Along Direction",
            Self::AlongNormal => "Along Normal",
        }
    }
}

/// Project to Surface's settings (the target is handed in).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectToSurface {
    pub mode: ProjectMode,
    /// Kept off the surface along its normal (cm).
    pub offset: f32,
    pub strength: f32,
}

impl Default for ProjectToSurface {
    fn default() -> Self {
        Self {
            mode: ProjectMode::NearestPoint,
            offset: 0.0,
            strength: 1.0,
        }
    }
}

/// A surface to project onto: where a point lands (nearest or along a ray)
/// and the surface's normal there, in the deformer's space.
pub trait Surface {
    fn nearest(&self, point: Vec3) -> Option<(Vec3, Vec3)>;
    fn cast(&self, origin: Vec3, dir: Vec3) -> Option<(Vec3, Vec3)>;
}

impl ProjectToSurface {
    /// `mesh`'s points pulled onto `surface`; a point that finds no surface
    /// stays.
    pub fn apply(&self, mesh: &mut PolyMesh, surface: &dyn Surface) {
        let normals = point_normals(mesh);
        for (p, point) in mesh.points_mut().iter_mut().enumerate() {
            let landing = match self.mode {
                ProjectMode::NearestPoint => surface.nearest(*point),
                ProjectMode::AlongDirection => surface.cast(*point, Vec3::NEG_Z),
                ProjectMode::AlongNormal => {
                    let n = normals[p];
                    let ahead = surface.cast(*point, n);
                    let behind = surface.cast(*point, -n);
                    match (ahead, behind) {
                        (Some(a), Some(b)) => Some(if a.0.distance(*point) <= b.0.distance(*point) {
                            a
                        } else {
                            b
                        }),
                        (a, b) => a.or(b),
                    }
                }
            };
            if let Some((at, normal)) = landing {
                let target = at + normal * self.offset;
                *point = point.lerp(target, self.strength);
            }
        }
    }
}

/// Smooth's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smooth {
    pub iterations: u32,
    /// Alternating shrink and inflate steps (Taubin), so the shape keeps its
    /// size.
    pub keep_volume: bool,
    /// Open borders do not move.
    pub keep_borders: bool,
    pub strength: f32,
}

impl Smooth {
    pub const MAX_ITERATIONS: u32 = 200;
}

impl Default for Smooth {
    fn default() -> Self {
        Self {
            iterations: 10,
            keep_volume: true,
            keep_borders: true,
            strength: 1.0,
        }
    }
}

impl Smooth {
    /// `mesh`'s points smoothed towards their neighbours' average.
    pub fn apply(&self, mesh: &mut PolyMesh) {
        let topology = Topology::build(mesh);
        let n = mesh.point_count();
        let mut fixed = BitSet::new();
        if self.keep_borders {
            for e in 0..topology.edge_count() {
                if topology.edge_faces(e).len() == 1 {
                    for p in topology.edges()[e] {
                        fixed.insert(us(p));
                    }
                }
            }
        }
        let original = mesh.points().to_vec();
        let mut points = original.clone();
        let mut next = points.clone();
        let step = |points: &[Vec3], next: &mut Vec<Vec3>, factor: f32| {
            for p in 0..n {
                let edges = topology.point_edges(p);
                if fixed.contains(p) || edges.is_empty() {
                    next[p] = points[p];
                    continue;
                }
                let mut sum = Vec3::ZERO;
                for &e in edges {
                    let [a, b] = topology.edges()[us(e)];
                    sum += points[us(if us(a) == p { b } else { a })];
                }
                let average = sum / edges.len() as f32;
                next[p] = points[p] + (average - points[p]) * factor;
            }
        };
        for _ in 0..self.iterations.min(Self::MAX_ITERATIONS) {
            step(&points, &mut next, 0.5);
            std::mem::swap(&mut points, &mut next);
            if self.keep_volume {
                step(&points, &mut next, -0.53);
                std::mem::swap(&mut points, &mut next);
            }
        }
        for (out, (from, to)) in mesh.points_mut().iter_mut().zip(original.iter().zip(&points)) {
            *out = from.lerp(*to, self.strength);
        }
    }
}

/// Explode's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explode {
    /// How far the explosion has gone (0 – 1).
    pub strength: f32,
    /// How far polygons fly at full strength (cm).
    pub distance: f32,
    /// Random spread of each polygon's distance (0 – 1).
    pub variation: f32,
    /// Random turn at full strength (radians).
    pub spin: f32,
    /// Each polygon's size at full strength (1 = as it is).
    pub end_size: f32,
    pub seed: u32,
}

impl Default for Explode {
    fn default() -> Self {
        Self {
            strength: 0.0,
            distance: 300.0,
            variation: 0.5,
            spin: std::f32::consts::PI,
            end_size: 1.0,
            seed: 0,
        }
    }
}

impl Explode {
    /// `mesh` exploded: at strength 0 unchanged; otherwise every polygon
    /// gets its own points and flies outward from the origin.
    #[must_use]
    pub fn apply(&self, mesh: &PolyMesh) -> PolyMesh {
        if self.strength <= 0.0 {
            return mesh.clone();
        }
        let mut points = Vec::with_capacity(mesh.corner_count());
        let mut faces: Vec<Vec<u32>> = Vec::with_capacity(mesh.face_count());
        for f in 0..mesh.face_count() {
            let centre = mesh.face_center(f);
            let normal = mesh.face_normal(f);
            let away = centre.try_normalize().unwrap_or(normal);
            let random = |k: u32| {
                hash(
                    i32::try_from(f).unwrap_or(0),
                    i32::try_from(k).unwrap_or(0),
                    0,
                    self.seed,
                )
            };
            let distance = self.distance * self.strength * (1.0 + self.variation * (random(1) * 2.0 - 1.0));
            let axis = Vec3::new(random(2) - 0.5, random(3) - 0.5, random(4) - 0.5).normalize_or(Vec3::Y);
            let turn = Quat::from_axis_angle(axis, self.spin * self.strength * (random(5) * 2.0 - 1.0));
            let size = 1.0 + (self.end_size - 1.0) * self.strength;
            let moved = centre + away * distance;
            let start = u32::try_from(points.len()).unwrap_or(u32::MAX);
            points.extend(
                mesh.face(f)
                    .iter()
                    .map(|&p| moved + turn * ((mesh.points()[us(p)] - centre) * size)),
            );
            faces.push((start..start + u32::try_from(mesh.face(f).len()).unwrap_or(0)).collect());
        }
        let Ok(mut out) = PolyMesh::from_faces(points, faces) else {
            return mesh.clone();
        };
        if let Some(uvs) = mesh.uvs() {
            let _ = out.set_uvs(Some(uvs.to_vec()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn wrapping_a_plane_round_a_cylinder_closes_its_seam() {
        let plane = primitives::plane(400.0, 200.0, 40, 4);
        // The plane lies in XZ: turn it to stand in XY.
        let points: Vec<Vec3> = plane.points().iter().map(|p| Vec3::new(p.x, p.z, 0.0)).collect();
        let wrap = Wrap {
            radius: 400.0 / std::f32::consts::TAU,
            ..Wrap::default()
        };
        let wrapped: Vec<Vec3> = points.iter().map(|&p| wrap.apply(p)).collect();
        for (p, w) in points.iter().zip(&wrapped) {
            assert!(
                (Vec2::new(w.x, w.z).length() - wrap.radius).abs() < 1e-3,
                "on the cylinder"
            );
            if (p.x - 200.0).abs() < 1e-3 {
                let partner = points
                    .iter()
                    .position(|q| (q.x + 200.0).abs() < 1e-3 && (q.y - p.y).abs() < 1e-3)
                    .unwrap();
                assert!(w.distance(wrapped[partner]) < 1e-3, "the seam closes");
            }
        }
        let none = Wrap { strength: 0.0, ..wrap };
        assert_eq!(none.apply(points[5]), points[5]);
    }

    #[test]
    fn a_cylinder_follows_a_quarter_circle_keeping_its_section() {
        let cylinder = primitives::cylinder(10.0, 100.0, 12, 10);
        let arc: Vec<Vec3> = (0..=64)
            .map(|k| {
                let a = std::f32::consts::FRAC_PI_2 * k as f32 / 64.0;
                Vec3::new(200.0 * (1.0 - a.cos()), 200.0 * a.sin(), 0.0)
            })
            .collect();
        let path = Polyline {
            segments: vec![Segment {
                count: 65,
                closed: false,
            }],
            points: arc,
        };
        let mut points = cylinder.points().to_vec();
        FollowSpline::default().apply(&mut points, &path);
        // Every point stays 10 cm from the arc (a tube of constant section).
        let centre = Vec3::new(200.0, 0.0, 0.0);
        for p in &points {
            let off_plane = p.z;
            let from_axis = (Vec2::new(p.x, p.y) - Vec2::new(centre.x, centre.y)).length() - 200.0;
            let distance = (off_plane * off_plane + from_axis * from_axis).sqrt();
            assert!((distance - 10.0).abs() < 0.5, "{distance}");
        }
    }

    #[test]
    fn displace_moves_by_the_picture_and_nothing_without_it() {
        let mut plane = primitives::plane(100.0, 100.0, 4, 4);
        let before = plane.clone();
        let picture = GreyPicture {
            width: 1,
            height: 1,
            values: Arc::from(vec![0.75f32]),
        };
        let displace = Displace {
            source: DisplaceSource::Texture,
            height: 20.0,
            ..Displace::default()
        };
        displace.apply(&mut plane, None);
        assert_eq!(plane, before, "no picture: nothing moves");
        displace.apply(&mut plane, Some(&picture));
        for (a, b) in plane.points().iter().zip(before.points()) {
            assert!(
                (a.y - b.y - 20.0 * 0.25).abs() < 1e-4,
                "along the normal by height × (value − mid)"
            );
        }
    }

    struct Ball(f32);

    impl Surface for Ball {
        fn nearest(&self, point: Vec3) -> Option<(Vec3, Vec3)> {
            let n = point.try_normalize()?;
            Some((n * self.0, n))
        }

        fn cast(&self, _: Vec3, _: Vec3) -> Option<(Vec3, Vec3)> {
            None
        }
    }

    #[test]
    fn projected_points_land_on_the_target() {
        let mut cube = primitives::cube(Vec3::splat(80.0), [3, 3, 3]);
        ProjectToSurface::default().apply(&mut cube, &Ball(100.0));
        assert!(cube.points().iter().all(|p| (p.length() - 100.0).abs() < 1e-3));
        let mut off = primitives::cube(Vec3::splat(80.0), [3, 3, 3]);
        ProjectToSurface {
            offset: 5.0,
            ..ProjectToSurface::default()
        }
        .apply(&mut off, &Ball(100.0));
        assert!(off.points().iter().all(|p| (p.length() - 105.0).abs() < 1e-3));
    }

    #[test]
    fn smoothing_keeps_volume_and_borders() {
        let mut sphere = primitives::sphere(100.0, 32);
        for (i, p) in sphere.points_mut().iter_mut().enumerate() {
            *p *= 1.0 + 0.05 * ((i * 7919 % 13) as f32 / 13.0 - 0.5);
        }
        let before = crate::boolean::volume(&sphere);
        Smooth::default().apply(&mut sphere);
        let after = crate::boolean::volume(&sphere);
        assert!((after - before).abs() / before < 0.02, "{before} → {after}");
        let mut plane = primitives::plane(100.0, 100.0, 6, 6);
        for p in plane.points_mut() {
            p.y = (p.x * 0.3).sin() * 5.0;
        }
        let border: Vec<Vec3> = plane
            .points()
            .iter()
            .copied()
            .filter(|p| p.x.abs() > 49.0 || p.z.abs() > 49.0)
            .collect();
        Smooth::default().apply(&mut plane);
        for b in &border {
            assert!(plane.points().iter().any(|p| p.distance(*b) < 1e-4), "border kept");
        }
    }

    #[test]
    fn explode_splits_and_flies_out() {
        let cube = primitives::cube(Vec3::splat(100.0), [1, 1, 1]);
        assert_eq!(Explode::default().apply(&cube), cube, "0 %: the input");
        let full = Explode {
            strength: 1.0,
            variation: 0.0,
            spin: 0.0,
            ..Explode::default()
        };
        let out = full.apply(&cube);
        assert_eq!(out.point_count(), 24, "every polygon its own points");
        for f in 0..out.face_count() {
            let moved = out.face_center(f).length() - cube.face_center(f).length();
            assert!((moved - 300.0).abs() < 1e-3, "{moved}");
        }
    }
}
