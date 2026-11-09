//! Render passes (see `docs/reference/render-passes.md`): the light of each
//! sample split by where it came from — glowing surfaces, the background,
//! diffuse or glossy reflection straight from the lights or after bounces,
//! light through glass, and by light group — beside what the first surface
//! is (depth, normal, position, albedo, object and material). Only the
//! passes chosen get room per pixel; a render without passes keeps none.

use glam::Vec3;
use grid4d_scene::{Pass, Passes};

/// Light groups a render keeps apart: Default and 16 named ones (a light
/// in a later group counts as Default).
pub(crate) const GROUPS: usize = 17;

/// Where a contribution of light goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Emission,
    Background,
    DiffuseDirect,
    DiffuseIndirect,
    SpecularDirect,
    SpecularIndirect,
    Transmission,
}

impl Part {
    const fn index(self) -> usize {
        match self {
            Self::Emission => 0,
            Self::Background => 1,
            Self::DiffuseDirect => 2,
            Self::DiffuseIndirect => 3,
            Self::SpecularDirect => 4,
            Self::SpecularIndirect => 5,
            Self::Transmission => 6,
        }
    }
}

/// What the first surface a sample met is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct First {
    pub depth: f32,
    pub normal: Vec3,
    pub position: Vec3,
    pub albedo: Vec3,
    pub object: u32,
    pub material: u32,
}

/// One sample's light by part and by light group, and its first surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Split {
    parts: [Vec3; 7],
    groups: [Vec3; GROUPS],
    pub first: Option<First>,
}

impl Default for Split {
    fn default() -> Self {
        Self {
            parts: [Vec3::ZERO; 7],
            groups: [Vec3::ZERO; GROUPS],
            first: None,
        }
    }
}

impl Split {
    /// Adds `light` of light group `group` (0: none) to `part`.
    pub(crate) fn add(&mut self, part: Part, group: u16, light: Vec3) {
        self.parts[part.index()] += light;
        let slot = usize::from(group);
        self.groups[if slot < GROUPS { slot } else { 0 }] += light;
    }
}

/// One pass as a picture: its name (`Depth`, `DiffuseDirect`,
/// `Group.Key` …), its channels (`R G B`, `Z`, `X Y Z`, `ID`, `A`), whether
/// it holds data best kept as 32-bit floats, and its values, per pixel all
/// channels, rows from the top. Lighting passes are premultiplied by
/// coverage like the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct PassLayer {
    pub name: String,
    pub channels: Vec<&'static str>,
    pub float: bool,
    pub values: Vec<f32>,
}

impl PassLayer {
    /// A map of the samples each pixel got (adaptive sampling): a layer
    /// `Samples` with one channel `N`, shown from dark blue (fewest) through
    /// red to yellow (most).
    #[must_use]
    pub fn samples(counts: &[u32]) -> Self {
        Self {
            name: "Samples".to_owned(),
            channels: vec!["N"],
            float: true,
            values: counts
                .iter()
                .map(|&n| f32::from(u16::try_from(n).unwrap_or(u16::MAX)))
                .collect(),
        }
    }

    /// 8-bit RGB to look at, three bytes per pixel: colours sRGB-encoded
    /// over black; Alpha as grey; Depth from near (white) to far (black)
    /// over the range found; Normal × 0.5 + 0.5; Position over its range per
    /// axis; IDs as distinct colours (0 black);
    /// samples as a heat map.
    #[must_use]
    pub fn to_rgb8(&self) -> Vec<u8> {
        use crate::image::{linear_to_srgb, to_byte};
        let stride = self.channels.len().max(1);
        let pixels = self.values.chunks(stride);
        let range = |c: usize| {
            self.values
                .iter()
                .skip(c)
                .step_by(stride)
                .filter(|v| v.is_finite() && **v < f32::MAX)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
                    (lo.min(v), hi.max(v))
                })
        };
        let scaled = |v: f32, (lo, hi): (f32, f32)| {
            if hi > lo { (v - lo) / (hi - lo) } else { 0.5 }
        };
        match self.channels.as_slice() {
            ["A"] => pixels.flat_map(|p| [to_byte(p[0]); 3]).collect(),
            ["Z"] => {
                let r = range(0);
                pixels
                    .flat_map(|p| {
                        let v = if p[0] >= f32::MAX {
                            0.0
                        } else {
                            1.0 - scaled(p[0], r) * 0.9
                        };
                        [to_byte(v); 3]
                    })
                    .collect()
            }
            ["ID"] => pixels.flat_map(|p| id_colour(p[0])).collect(),
            ["N"] => {
                // From the fewest samples any pixel got to the most.
                let (fewest, most) = range(0);
                pixels
                    .flat_map(|p| {
                        let t = if most > fewest {
                            ((p[0] - fewest) / (most - fewest)).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let r = (t * 2.0).min(1.0);
                        let g = (t * 2.0 - 1.0).clamp(0.0, 1.0);
                        let b = 0.35 * (1.0 - t * 2.0).max(0.0) + 0.1;
                        [r, g, b].map(to_byte)
                    })
                    .collect()
            }
            ["X", "Y", "Z"] if self.name == "Normal" => pixels
                .flat_map(|p| [0, 1, 2].map(|c| to_byte(p[c] * 0.5 + 0.5)))
                .collect(),
            ["X", "Y", "Z"] => {
                let r = [range(0), range(1), range(2)];
                pixels
                    .flat_map(|p| [0, 1, 2].map(|c| to_byte(scaled(p[c], r[c]))))
                    .collect()
            }
            _ => pixels
                .flat_map(|p| [0, 1, 2].map(|c| to_byte(linear_to_srgb(p.get(c).copied().unwrap_or(0.0)))))
                .collect(),
        }
    }

    /// The values of channel `index`, one per pixel.
    #[must_use]
    pub fn channel(&self, index: usize) -> Vec<f32> {
        let stride = self.channels.len().max(1);
        self.values.iter().skip(index).step_by(stride).copied().collect()
    }
}

/// Where each chosen pass's sums sit among a pixel's floats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    passes: Passes,
    parts: [Option<usize>; 7],
    /// Light groups: where they start, and how many (Default first).
    groups: Option<(usize, usize)>,
    depth: Option<usize>,
    normal: Option<usize>,
    position: Option<usize>,
    albedo: Option<usize>,
    /// Samples that met a surface (data passes average over them).
    hits: Option<usize>,
    /// Votes: the ID in the lead and its margin.
    object: Option<usize>,
    material: Option<usize>,
    pub stride: usize,
}

impl Layout {
    /// The room `passes` need, with `named` light groups; `None` when no
    /// pass needs any (the picture and its Alpha come with every render).
    pub(crate) fn new(passes: Passes, named: usize) -> Option<Self> {
        let mut stride = 0;
        let mut take = |on: bool, floats: usize| {
            on.then(|| {
                let at = stride;
                stride += floats;
                at
            })
        };
        let parts = Pass::LIGHTING.map(|pass| take(passes.contains(pass), 3));
        let group_count = (named + 1).min(GROUPS);
        let groups = take(passes.contains(Pass::LightGroups), 3 * group_count).map(|at| (at, group_count));
        let depth = take(passes.contains(Pass::Depth), 1);
        let normal = take(passes.contains(Pass::Normal), 3);
        let position = take(passes.contains(Pass::Position), 3);
        let albedo = take(passes.contains(Pass::Albedo), 3);
        let data = depth.is_some() || normal.is_some() || position.is_some() || albedo.is_some();
        let hits = take(data, 1);
        let object = take(passes.contains(Pass::ObjectId), 2);
        let material = take(passes.contains(Pass::MaterialId), 2);
        (stride > 0 || passes.contains(Pass::Alpha)).then_some(Self {
            passes,
            parts,
            groups,
            depth,
            normal,
            position,
            albedo,
            hits,
            object,
            material,
            stride,
        })
    }

    /// Adds one sample to a pixel's `slots`.
    pub(crate) fn add(&self, slots: &mut [f32], split: &Split) {
        let mut put = |at: Option<usize>, v: Vec3| {
            if let Some(at) = at
                && let Some(s) = slots.get_mut(at..at + 3)
            {
                s[0] += v.x;
                s[1] += v.y;
                s[2] += v.z;
            }
        };
        for (at, &light) in self.parts.iter().zip(&split.parts) {
            put(*at, light);
        }
        if let Some((start, count)) = self.groups {
            for (k, &light) in split.groups.iter().take(count).enumerate() {
                put(Some(start + 3 * k), light);
            }
        }
        if let Some(first) = split.first {
            put(self.normal, first.normal);
            put(self.position, first.position);
            put(self.albedo, first.albedo);
            if let Some(at) = self.depth {
                slots[at] += first.depth;
            }
            if let Some(at) = self.hits {
                slots[at] += 1.0;
            }
        }
        let ids = split.first.map_or((0, 0), |f| (f.object, f.material));
        for (at, id) in [(self.object, ids.0), (self.material, ids.1)] {
            if let Some(at) = at {
                vote(&mut slots[at..at + 2], id);
            }
        }
    }

    /// The passes as pictures from `sums` (the picture's own: colour and
    /// coverage) and `extra` (this layout's per pixel), each pixel after
    /// `samples(pixel)` samples; `groups` names the light groups.
    pub(crate) fn layers(
        &self,
        sums: &[[f32; 4]],
        extra: &[f32],
        samples: &dyn Fn(usize) -> u32,
        groups: &[String],
    ) -> Vec<PassLayer> {
        let n = |i: usize| f32::from(u16::try_from(samples(i).max(1)).unwrap_or(u16::MAX));
        let pixels = sums.len();
        let pixel = |i: usize| extra.get(i * self.stride..(i + 1) * self.stride).unwrap_or(&[]);
        let mut layers = Vec::new();
        let colour = |name: String, at: usize| PassLayer {
            name,
            channels: vec!["R", "G", "B"],
            float: false,
            values: (0..pixels)
                .flat_map(|i| {
                    let p = pixel(i);
                    let n = n(i);
                    [0, 1, 2].map(|c| p.get(at + c).copied().unwrap_or(0.0) / n)
                })
                .collect(),
        };
        let averaged = |at: usize, count: usize, empty: f32| -> Vec<f32> {
            (0..pixels)
                .flat_map(|i| {
                    let p = pixel(i);
                    let hits = self.hits.and_then(|h| p.get(h).copied()).unwrap_or(0.0);
                    (0..count).map(move |c| {
                        if hits > 0.0 {
                            p.get(at + c).copied().unwrap_or(0.0) / hits
                        } else {
                            empty
                        }
                    })
                })
                .collect()
        };
        for pass in self.passes.iter() {
            match pass {
                Pass::Alpha => layers.push(PassLayer {
                    name: pass.name().to_owned(),
                    channels: vec!["A"],
                    float: false,
                    values: sums.iter().enumerate().map(|(i, s)| s[3] / n(i)).collect(),
                }),
                Pass::Depth => {
                    if let Some(at) = self.depth {
                        layers.push(PassLayer {
                            name: pass.name().to_owned(),
                            channels: vec!["Z"],
                            float: true,
                            values: averaged(at, 1, f32::MAX),
                        });
                    }
                }
                Pass::Normal | Pass::Position => {
                    let at = if pass == Pass::Normal {
                        self.normal
                    } else {
                        self.position
                    };
                    if let Some(at) = at {
                        layers.push(PassLayer {
                            name: pass.name().to_owned(),
                            channels: vec!["X", "Y", "Z"],
                            float: true,
                            values: averaged(at, 3, 0.0),
                        });
                    }
                }
                Pass::Albedo => {
                    if let Some(at) = self.albedo {
                        layers.push(PassLayer {
                            name: pass.name().to_owned(),
                            channels: vec!["R", "G", "B"],
                            float: false,
                            values: averaged(at, 3, 0.0),
                        });
                    }
                }
                Pass::Emission
                | Pass::Background
                | Pass::DiffuseDirect
                | Pass::DiffuseIndirect
                | Pass::SpecularDirect
                | Pass::SpecularIndirect
                | Pass::Transmission => {
                    let index = Pass::LIGHTING.iter().position(|&p| p == pass).unwrap_or(0);
                    if let Some(at) = self.parts[index] {
                        layers.push(colour(pass.name().to_owned(), at));
                    }
                }
                Pass::ObjectId | Pass::MaterialId => {
                    let at = if pass == Pass::ObjectId {
                        self.object
                    } else {
                        self.material
                    };
                    if let Some(at) = at {
                        layers.push(PassLayer {
                            name: pass.name().to_owned(),
                            channels: vec!["ID"],
                            float: true,
                            values: (0..pixels).map(|i| pixel(i).get(at).copied().unwrap_or(0.0)).collect(),
                        });
                    }
                }
                Pass::LightGroups => {
                    if let Some((start, count)) = self.groups {
                        for k in 0..count {
                            let name = if k == 0 {
                                "Default"
                            } else {
                                groups.get(k - 1).map_or("Group", String::as_str)
                            };
                            layers.push(colour(format!("{}.{name}", pass.name()), start + 3 * k));
                        }
                    }
                }
            }
        }
        layers
    }
}

/// A colour for ID `id`, apart from its neighbours' (0: black).
fn id_colour(id: f32) -> [u8; 3] {
    if id <= 0.0 {
        return [0; 3];
    }
    // The golden angle round the hue circle: neighbours far apart.
    let hue = (id * 0.618_034).fract();
    let channel = |offset: f32| {
        let x = ((hue + offset).fract() * 6.0 - 3.0).abs() - 1.0;
        crate::image::to_byte(0.25 + 0.7 * x.clamp(0.0, 1.0))
    };
    [channel(0.0), channel(2.0 / 3.0), channel(1.0 / 3.0)]
}

/// One vote for `id` among a pixel's (leader, margin): the ID most samples
/// met wins whenever one met more than half of them.
fn vote(slots: &mut [f32], id: u32) {
    // IDs stay far below 2²⁴: exact in an f32.
    let id = f32::from(u16::try_from(id).unwrap_or(u16::MAX));
    if slots[1] <= 0.0 {
        slots[0] = id;
        slots[1] = 1.0;
    } else if slots[0] == id {
        slots[1] += 1.0;
    } else {
        slots[1] -= 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_chosen_passes_take_room() {
        assert_eq!(Layout::new(Passes::NONE, 3), None);
        let alpha: Passes = [Pass::Alpha].into_iter().collect();
        assert_eq!(Layout::new(alpha, 0).unwrap().stride, 0);
        let some: Passes = [Pass::Depth, Pass::DiffuseDirect, Pass::LightGroups, Pass::ObjectId]
            .into_iter()
            .collect();
        // Diffuse Direct 3, groups 3 × (2 + 1), depth 1, hits 1, a vote 2.
        assert_eq!(Layout::new(some, 2).unwrap().stride, 3 + 9 + 1 + 1 + 2);
    }

    #[test]
    fn the_id_most_samples_met_wins() {
        let mut slots = [0.0; 2];
        for id in [3, 5, 3, 3, 7, 3] {
            vote(&mut slots, id);
        }
        assert_eq!(slots[0], 3.0);
    }
}
