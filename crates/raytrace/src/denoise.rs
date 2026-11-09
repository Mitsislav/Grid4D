//! The denoiser: an edge-avoiding à-trous wavelet filter (after Dammertz et
//! al., 2010) over the finished picture, guided by what the render saw
//! first in each pixel — the surface's colour, which way it faces, how far
//! it is — and by how noisy each pixel still is (see
//! `docs/reference/render-sampling.md`).
//!
//! Light is filtered divided by the surface colour and multiplied back, so
//! textures stay sharp; the lighting and light-group passes are filtered
//! with the picture's own weights, so they still add up to it.

use glam::Vec3;
use rayon::prelude::*;

use crate::{Image, PassLayer};

/// Rounds of the filter; the holes between taps double each round
/// (1, 2, 4, 8, 16 pixels).
const ROUNDS: u32 = 5;
/// The B3 spline's weights for the five taps along each axis.
const KERNEL: [f32; 5] = [1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
/// How alike two normals must be: their cosine to this power.
const NORMAL_POWER: i32 = 64;
/// Allowed depth difference, relative to the depth's change per pixel.
const DEPTH_SIGMA: f32 = 1.0;
/// Allowed difference of surface colours (squared, per channel summed).
const ALBEDO_SIGMA: f32 = 0.02;
/// Allowed difference of brightness, in standard deviations of the noise.
const LUMINANCE_SIGMA: f32 = 4.0;
/// Surface colours darker than this are not divided by (no light to keep).
const ALBEDO_FLOOR: f32 = 0.02;
/// Depth of pixels that met nothing.
const NOTHING: f32 = f32::MAX;

/// What the render saw first in each pixel, for the denoiser: averaged
/// over the samples that met a surface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guides {
    /// The surface colour, three per pixel.
    pub(crate) albedo: Vec<f32>,
    /// The world normal, three per pixel.
    pub(crate) normal: Vec<f32>,
    /// Distance along the view; [`f32::MAX`] where nothing was met.
    pub(crate) depth: Vec<f32>,
    /// How noisy the pixel's brightness still is: the variance of its
    /// average, estimated from its two halves of samples.
    pub(crate) variance: Vec<f32>,
}

/// The names of the passes filtered with the picture.
fn filtered(layer: &PassLayer) -> bool {
    layer.channels.len() == 3
        && !layer.float
        && (grid4d_scene::Pass::LIGHTING.iter().any(|p| p.name() == layer.name) || layer.name.starts_with("Group."))
}

/// Cleans `image` (rendered with [`Progressive::with_denoising`]
/// (crate::Progressive::with_denoising)), keeping `strength` (0…1) of the
/// result over the noisy picture. The picture before is kept as a pass
/// named `Noisy`. Returns `false`, changing nothing, when the image has no
/// guides.
pub fn denoise(image: &mut Image, strength: f32) -> bool {
    let Some(guides) = image.guides.as_ref() else {
        return false;
    };
    let (w, h) = (
        usize::try_from(image.width).unwrap_or(0),
        usize::try_from(image.height).unwrap_or(0),
    );
    let count = w * h;
    if count == 0 || guides.depth.len() != count {
        return false;
    }
    let strength = strength.clamp(0.0, 1.0);
    // Light divided by the surface colour, for the picture and each
    // filtered pass.
    // Colours are premultiplied by coverage: an edge pixel half over the
    // background holds half its surface's light. Divided by its coverage
    // too, it filters like its neighbours and keeps its soft edge.
    let divisor: Vec<Vec3> = (0..count)
        .map(|i| {
            let a = vec3(&guides.albedo, i);
            let colour = Vec3::select(a.cmplt(Vec3::splat(ALBEDO_FLOOR)), Vec3::ONE, a);
            let coverage = image.pixels.get(i).map_or(1.0, |p| p[3]);
            if coverage > 0.0 { colour * coverage } else { colour }
        })
        .collect();
    let mut layers: Vec<Vec<Vec3>> = Vec::new();
    layers.push(
        image
            .pixels
            .iter()
            .zip(&divisor)
            .map(|(p, d)| Vec3::new(p[0], p[1], p[2]) / *d)
            .collect(),
    );
    let passes: Vec<usize> = (0..image.passes.len())
        .filter(|&i| filtered(&image.passes[i]))
        .collect();
    for &index in &passes {
        let values = &image.passes[index].values;
        layers.push((0..count).map(|i| vec3(values, i) / divisor[i]).collect());
    }
    // The noise in the same units as the light filtered: divided by the
    // surface colour and coverage too (squared, for a variance).
    let demodulated: Vec<f32> = guides
        .variance
        .iter()
        .zip(&divisor)
        .map(|(v, d)| v / luminance(*d).max(ALBEDO_FLOOR).powi(2))
        .collect();
    let mut variance = prefiltered(&demodulated, w, h);
    let gradient = depth_gradient(&guides.depth, w, h);
    let mut next: Vec<Vec<Vec3>> = layers.iter().map(|l| vec![Vec3::ZERO; l.len()]).collect();
    let mut next_variance = vec![0.0; count];
    for round in 0..ROUNDS {
        let step = 1usize << round;
        let filter = Round {
            guides,
            gradient: &gradient,
            light: &layers[0],
            variance: &variance,
            w,
            h,
            step,
        };
        // Pixel by pixel, rows in parallel: the weights once, applied to
        // every layer.
        next_variance.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = filter.variance_at(x, y);
            }
        });
        let outputs: Vec<&mut Vec<Vec3>> = next.iter_mut().collect();
        let mut rows: Vec<Vec<&mut [Vec3]>> = (0..h).map(|_| Vec::with_capacity(layers.len())).collect();
        for out in outputs {
            for (y, chunk) in out.chunks_mut(w).enumerate() {
                rows[y].push(chunk);
            }
        }
        rows.par_iter_mut().enumerate().for_each(|(y, outs)| {
            let mut sums = vec![Vec3::ZERO; layers.len()];
            for x in 0..w {
                filter.apply(x, y, &layers, &mut sums);
                for (out, sum) in outs.iter_mut().zip(&sums) {
                    out[x] = *sum;
                }
            }
        });
        std::mem::swap(&mut layers, &mut next);
        std::mem::swap(&mut variance, &mut next_variance);
    }
    // The noisy picture, kept beside the result.
    let noisy = PassLayer {
        name: "Noisy".to_owned(),
        channels: vec!["R", "G", "B"],
        float: false,
        values: image.pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(),
    };
    let mix = |noisy: Vec3, clean: Vec3| noisy.lerp(clean, strength);
    for (i, pixel) in image.pixels.iter_mut().enumerate() {
        let clean = layers[0][i] * divisor[i];
        let out = mix(Vec3::new(pixel[0], pixel[1], pixel[2]), clean);
        (pixel[0], pixel[1], pixel[2]) = (out.x, out.y, out.z);
    }
    for (k, &index) in passes.iter().enumerate() {
        let values = &mut image.passes[index].values;
        for i in 0..count {
            let out = mix(vec3(values, i), layers[k + 1][i] * divisor[i]);
            if let Some(v) = values.get_mut(i * 3..i * 3 + 3) {
                v.copy_from_slice(&[out.x, out.y, out.z]);
            }
        }
    }
    image.passes.push(noisy);
    true
}

/// Three floats of `values` at pixel `i` (zero past the end).
fn vec3(values: &[f32], i: usize) -> Vec3 {
    values
        .get(i * 3..i * 3 + 3)
        .map_or(Vec3::ZERO, |v| Vec3::new(v[0], v[1], v[2]))
}

/// The variance blurred over 3 × 3 pixels: one pixel's estimate is noisy.
fn prefiltered(variance: &[f32], w: usize, h: usize) -> Vec<f32> {
    const WEIGHTS: [f32; 3] = [0.25, 0.5, 0.25];
    (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let (mut sum, mut total) = (0.0, 0.0);
            for (dy, wy) in WEIGHTS.iter().enumerate() {
                for (dx, wx) in WEIGHTS.iter().enumerate() {
                    let (Some(nx), Some(ny)) = ((x + dx).checked_sub(1), (y + dy).checked_sub(1)) else {
                        continue;
                    };
                    if nx < w && ny < h {
                        let weight = wx * wy;
                        sum += variance.get(ny * w + nx).copied().unwrap_or(0.0) * weight;
                        total += weight;
                    }
                }
            }
            if total > 0.0 { sum / total } else { 0.0 }
        })
        .collect()
}

/// How much the depth changes from each pixel to the next (the larger of
/// across and down), so slanted surfaces are not taken for edges.
fn depth_gradient(depth: &[f32], w: usize, h: usize) -> Vec<f32> {
    (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let z = depth[i];
            if z >= NOTHING {
                return 0.0;
            }
            let change = |j: Option<usize>| {
                j.and_then(|j| depth.get(j))
                    .filter(|&&n| n < NOTHING)
                    .map_or(0.0, |&n| (n - z).abs())
            };
            let right = change((x + 1 < w).then_some(i + 1));
            let left = change(x.checked_sub(1).map(|_| i - 1));
            let down = change((y + 1 < h).then_some(i + w));
            let up = change(y.checked_sub(1).map(|_| i - w));
            right.min(left).max(down.min(up))
        })
        .collect()
}

/// One round of the filter.
struct Round<'a> {
    guides: &'a Guides,
    gradient: &'a [f32],
    /// The picture's light (divided by the surface colour) going in.
    light: &'a [Vec3],
    variance: &'a [f32],
    w: usize,
    h: usize,
    step: usize,
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

impl Round<'_> {
    /// The neighbours of (`x`, `y`) this round, with the kernel's weight:
    /// `step` pixels apart, inside the picture.
    fn taps(&self, x: usize, y: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let step = isize::try_from(self.step).unwrap_or(1);
        let (x, y) = (isize::try_from(x).unwrap_or(0), isize::try_from(y).unwrap_or(0));
        let (w, h) = (
            isize::try_from(self.w).unwrap_or(0),
            isize::try_from(self.h).unwrap_or(0),
        );
        KERNEL.iter().enumerate().flat_map(move |(j, ky)| {
            KERNEL.iter().enumerate().filter_map(move |(i, kx)| {
                let nx = x + (isize::try_from(i).unwrap_or(2) - 2) * step;
                let ny = y + (isize::try_from(j).unwrap_or(2) - 2) * step;
                ((0..w).contains(&nx) && (0..h).contains(&ny))
                    .then(|| (usize::try_from(ny * w + nx).unwrap_or(0), kx * ky))
            })
        })
    }

    /// How much neighbour `q` counts for pixel `p` (1 for itself).
    fn weight(&self, p: usize, q: usize, sigma_light: f32) -> f32 {
        let g = self.guides;
        let (zp, zq) = (g.depth[p], g.depth[q]);
        // Pixels that met nothing (a background, maybe a picture) stay as
        // they are, and surfaces do not mix with them.
        if p == q {
            return 1.0;
        }
        if zp >= NOTHING || zq >= NOTHING {
            return 0.0;
        }
        let distance = {
            let (px, py, qx, qy) = (p % self.w, p / self.w, q % self.w, q / self.w);
            let (dx, dy) = (px.abs_diff(qx), py.abs_diff(qy));
            #[expect(clippy::cast_precision_loss, reason = "pixel distances")]
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            d
        };
        let depth = (zp - zq).abs() / (DEPTH_SIGMA * self.gradient[p] * distance + 1e-3 * zp.abs() + 1e-4);
        // Averaged normals may be short (a thin surface seen from both
        // sides in one pixel): only their directions are compared, and a
        // pixel with none is like any other.
        let (np, nq) = (
            vec3(&g.normal, p).normalize_or_zero(),
            vec3(&g.normal, q).normalize_or_zero(),
        );
        let normal = if np == Vec3::ZERO || nq == Vec3::ZERO {
            1.0
        } else {
            np.dot(nq).max(0.0).powi(NORMAL_POWER)
        };
        let albedo = (vec3(&g.albedo, p) - vec3(&g.albedo, q)).length_squared() / ALBEDO_SIGMA;
        let light = (luminance(self.light[p]) - luminance(self.light[q])).abs() / sigma_light;
        normal * (-depth - albedo - light).exp()
    }

    fn sigma_light(&self, p: usize) -> f32 {
        LUMINANCE_SIGMA * self.variance[p].max(0.0).sqrt() + 1e-4
    }

    /// The filtered value of every layer at (`x`, `y`), into `sums`.
    fn apply(&self, x: usize, y: usize, layers: &[Vec<Vec3>], sums: &mut [Vec3]) {
        let p = y * self.w + x;
        let sigma = self.sigma_light(p);
        sums.fill(Vec3::ZERO);
        let mut total = 0.0;
        for (q, k) in self.taps(x, y) {
            let weight = k * self.weight(p, q, sigma);
            if weight <= 0.0 {
                continue;
            }
            total += weight;
            for (sum, layer) in sums.iter_mut().zip(layers) {
                *sum += layer[q] * weight;
            }
        }
        if total > 0.0 {
            for sum in sums.iter_mut() {
                *sum /= total;
            }
        } else {
            for (sum, layer) in sums.iter_mut().zip(layers) {
                *sum = layer[p];
            }
        }
    }

    /// The variance of the filtered value at (`x`, `y`).
    fn variance_at(&self, x: usize, y: usize) -> f32 {
        let p = y * self.w + x;
        let sigma = self.sigma_light(p);
        let (mut sum, mut total) = (0.0, 0.0);
        for (q, k) in self.taps(x, y) {
            let weight = k * self.weight(p, q, sigma);
            sum += weight * weight * self.variance[q];
            total += weight;
        }
        if total > 0.0 {
            sum / (total * total)
        } else {
            self.variance[p]
        }
    }
}
