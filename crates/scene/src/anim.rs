//! Animation data (see `docs/reference/animation-keys.md`): an object's animated
//! parameters each have a [`Track`] with a [`Curve`] of [`Key`]s. Tracks live in
//! [`NodeData::tracks`](crate::NodeData::tracks), so editing them is an ordinary
//! data edit (undo, save).

use crate::Transform;

/// An axis of a vector parameter (H, P, B for rotations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

/// An animatable parameter of an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Param {
    /// Centimetres.
    Position(Axis),
    /// Radians: X = heading, Y = pitch, Z = bank.
    Rotation(Axis),
    Scale(Axis),
}

impl Param {
    /// Every transform parameter: position, rotation, scale; X, Y, Z each.
    pub const TRANSFORM: [Self; 9] = [
        Self::Position(Axis::X),
        Self::Position(Axis::Y),
        Self::Position(Axis::Z),
        Self::Rotation(Axis::X),
        Self::Rotation(Axis::Y),
        Self::Rotation(Axis::Z),
        Self::Scale(Axis::X),
        Self::Scale(Axis::Y),
        Self::Scale(Axis::Z),
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Position(Axis::X) => "Position . X",
            Self::Position(Axis::Y) => "Position . Y",
            Self::Position(Axis::Z) => "Position . Z",
            Self::Rotation(Axis::X) => "Rotation . H",
            Self::Rotation(Axis::Y) => "Rotation . P",
            Self::Rotation(Axis::Z) => "Rotation . B",
            Self::Scale(Axis::X) => "Scale . X",
            Self::Scale(Axis::Y) => "Scale . Y",
            Self::Scale(Axis::Z) => "Scale . Z",
        }
    }

    /// This parameter's value in `transform`.
    #[must_use]
    pub fn get(self, transform: &Transform) -> f32 {
        match self {
            Self::Position(axis) => transform.position[axis.index()],
            Self::Rotation(axis) => transform.rotation[axis.index()],
            Self::Scale(axis) => transform.scale[axis.index()],
        }
    }

    /// Sets this parameter in `transform`.
    pub fn set(self, transform: &mut Transform, value: f32) {
        match self {
            Self::Position(axis) => transform.position[axis.index()] = value,
            Self::Rotation(axis) => transform.rotation[axis.index()] = value,
            Self::Scale(axis) => transform.scale[axis.index()] = value,
        }
    }
}

/// How the curve goes from a key to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interpolation {
    /// A smooth Bézier segment shaped by the keys' tangents.
    #[default]
    Spline,
    Linear,
    /// The value holds until the next key.
    Step,
}

impl Interpolation {
    pub const ALL: [Self; 3] = [Self::Spline, Self::Linear, Self::Step];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Spline => "Spline",
            Self::Linear => "Linear",
            Self::Step => "Step",
        }
    }
}

/// A key's tangent handles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tangents {
    /// As smooth as possible, flat at local extrema and between equal values
    /// (clamped), a third of the way to each neighbour.
    #[default]
    Auto,
    /// Handles as (frames, value change): the left one points back in time, the
    /// right one forwards; frames are lengths (≥ 0).
    Manual { left: (f64, f32), right: (f64, f32) },
}

/// A value at a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub frame: f64,
    pub value: f32,
    /// Of the segment from this key to the next.
    pub interpolation: Interpolation,
    pub tangents: Tangents,
}

impl Key {
    /// A key with the defaults: Spline, Auto tangents.
    #[must_use]
    pub fn new(frame: f64, value: f32) -> Self {
        Self {
            frame,
            value,
            interpolation: Interpolation::Spline,
            tangents: Tangents::Auto,
        }
    }
}

/// Keys sorted by frame, at most one per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    keys: Vec<Key>,
}

impl Curve {
    /// A curve of `keys` (sorted; at equal frames the later one wins).
    #[must_use]
    pub fn from_keys(keys: impl IntoIterator<Item = Key>) -> Self {
        let mut curve = Self::default();
        for key in keys {
            curve.set_key(key);
        }
        curve
    }

    #[must_use]
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Adds `key`, replacing a key at the same frame.
    pub fn set_key(&mut self, key: Key) {
        match self.keys.binary_search_by(|k| k.frame.total_cmp(&key.frame)) {
            Ok(i) => self.keys[i] = key,
            Err(i) => self.keys.insert(i, key),
        }
    }

    /// Removes the key at `frame`; `true` if there was one.
    pub fn remove_key(&mut self, frame: f64) -> bool {
        match self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)) {
            Ok(i) => {
                self.keys.remove(i);
                true
            }
            Err(_) => false,
        }
    }

    /// The curve's value at `frame`; `None` without keys. Before the first and
    /// after the last key it holds their values.
    #[must_use]
    pub fn value_at(&self, frame: f64) -> Option<f32> {
        let (first, last) = (self.keys.first()?, self.keys.last()?);
        if frame <= first.frame {
            return Some(first.value);
        }
        if frame >= last.frame {
            return Some(last.value);
        }
        // The segment [i, i + 1] that holds `frame`.
        let i = self.keys.partition_point(|k| k.frame <= frame).saturating_sub(1);
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        let span = b.frame - a.frame;
        if span <= 0.0 {
            return Some(b.value);
        }
        Some(match a.interpolation {
            Interpolation::Step => a.value,
            Interpolation::Linear => {
                let t = ((frame - a.frame) / span) as f32;
                a.value + (b.value - a.value) * t
            }
            Interpolation::Spline => {
                let (_, out) = self.handles(i);
                let (into, _) = self.handles(i + 1);
                bezier_at(
                    (a.frame, f64::from(a.value)),
                    (a.frame + out.0, f64::from(a.value) + f64::from(out.1)),
                    (b.frame - into.0, f64::from(b.value) - f64::from(into.1)),
                    (b.frame, f64::from(b.value)),
                    frame,
                ) as f32
            }
        })
    }

    /// The left and right handles of key `i` as (frames, value change), each
    /// shortened so it stays within the neighbouring segment.
    fn handles(&self, i: usize) -> ((f64, f32), (f64, f32)) {
        let key = self.keys[i];
        let before = i.checked_sub(1).map(|j| self.keys[j]);
        let after = self.keys.get(i + 1).copied();
        let left_span = before.map_or(0.0, |k| key.frame - k.frame);
        let right_span = after.map_or(0.0, |k| k.frame - key.frame);
        match key.tangents {
            Tangents::Auto => {
                let slope = match (before, after) {
                    (Some(b), Some(a)) => {
                        let extremum = (key.value - b.value) * (a.value - key.value) <= 0.0;
                        if extremum || a.frame <= b.frame {
                            0.0 // clamped: flat at a peak, a dip or next to an equal value
                        } else {
                            f64::from(a.value - b.value) / (a.frame - b.frame)
                        }
                    }
                    _ => 0.0, // the ends are flat
                };
                let (l, r) = (left_span / 3.0, right_span / 3.0);
                ((l, (slope * l) as f32), (r, (slope * r) as f32))
            }
            Tangents::Manual { left, right } => {
                let clamp = |(frames, value): (f64, f32), span: f64| {
                    let frames = frames.max(0.0);
                    if frames <= span || frames <= 0.0 {
                        (frames, value)
                    } else {
                        // Shorter, same direction.
                        (span, (f64::from(value) * span / frames) as f32)
                    }
                };
                (clamp(left, left_span), clamp(right, right_span))
            }
        }
    }
}

/// The value of the cubic Bézier `p0 … p3` (points as (frame, value)) at `frame`,
/// for a curve whose frame grows from `p0` to `p3`: the curve parameter is found
/// by bisection on the frame.
fn bezier_at(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), frame: f64) -> f64 {
    let point = |u: f64| {
        let v = 1.0 - u;
        let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
        (
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
        )
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..60 {
        let mid = (lo + hi) * 0.5;
        if point(mid).0 < frame {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    point((lo + hi) * 0.5).1
}

/// An animated parameter of an object.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub param: Param,
    pub curve: Curve,
}

/// Reading an object's tracks.
pub trait Tracks {
    /// `base` with every animated parameter set to its curve's value at `frame`.
    fn animate(&self, base: &Transform, frame: f64) -> Transform;
    /// The track of `param`, if it has one.
    fn track(&self, param: Param) -> Option<&Track>;
}

impl Tracks for [Track] {
    fn animate(&self, base: &Transform, frame: f64) -> Transform {
        let mut transform = *base;
        for track in self {
            if let Some(value) = track.curve.value_at(frame) {
                track.param.set(&mut transform, value);
            }
        }
        transform
    }

    fn track(&self, param: Param) -> Option<&Track> {
        self.iter().find(|t| t.param == param)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(keys: &[(f64, f32)], interpolation: Interpolation) -> Curve {
        Curve::from_keys(keys.iter().map(|&(frame, value)| Key {
            interpolation,
            ..Key::new(frame, value)
        }))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn keys_stay_sorted_and_a_key_at_the_same_frame_replaces_it() {
        let mut c = Curve::from_keys([Key::new(10.0, 1.0), Key::new(0.0, 0.0), Key::new(5.0, 3.0)]);
        assert_eq!(c.keys().iter().map(|k| k.frame).collect::<Vec<_>>(), [0.0, 5.0, 10.0]);
        c.set_key(Key::new(5.0, 7.0));
        assert_eq!(c.keys().len(), 3);
        assert_eq!(c.keys()[1].value, 7.0);
        assert!(c.remove_key(5.0) && !c.remove_key(5.0));
        assert_eq!(Curve::default().value_at(3.0), None);
    }

    #[test]
    fn step_linear_and_holding_beyond_the_keys() {
        let keys = [(0.0, 0.0), (10.0, 100.0), (20.0, 50.0)];
        let step = curve(&keys, Interpolation::Step);
        assert_eq!(step.value_at(9.9), Some(0.0));
        assert_eq!(step.value_at(10.0), Some(100.0));
        let linear = curve(&keys, Interpolation::Linear);
        assert!(close(linear.value_at(5.0).unwrap(), 50.0));
        assert!(close(linear.value_at(15.0).unwrap(), 75.0));
        assert_eq!(linear.value_at(-5.0), Some(0.0), "before the first key: its value");
        assert_eq!(linear.value_at(99.0), Some(50.0), "after the last key: its value");
    }

    #[test]
    fn splines_hit_their_keys_and_are_smooth_through_them() {
        let c = curve(
            &[(0.0, 0.0), (10.0, 40.0), (20.0, 100.0), (30.0, 60.0)],
            Interpolation::Spline,
        );
        for k in c.keys() {
            assert!(close(c.value_at(k.frame).unwrap(), k.value), "at frame {}", k.frame);
        }
        // Slopes left and right of the interior key at 10 agree (auto tangents).
        let slope = |a: f64, b: f64| f64::from(c.value_at(b).unwrap() - c.value_at(a).unwrap()) / (b - a);
        let (left, right) = (slope(9.99, 10.0), slope(10.0, 10.01));
        assert!((left - right).abs() < 0.05, "{left} vs {right}");
        assert!(
            (right - 5.0).abs() < 0.05,
            "the slope through the neighbours: (100 - 0) / 20"
        );
    }

    #[test]
    fn auto_tangents_do_not_overshoot_at_extrema() {
        // A bounce: up, down, up.
        let c = curve(&[(0.0, 0.0), (10.0, 100.0), (20.0, 0.0)], Interpolation::Spline);
        let samples: Vec<f32> = (0..=200).map(|i| c.value_at(f64::from(i) * 0.1).unwrap()).collect();
        let max = samples.iter().copied().fold(f32::MIN, f32::max);
        assert!(close(max, 100.0), "the peak is the key, not above it: {max}");
        assert!(samples.iter().all(|&v| v >= -1e-4), "no dip below the floor");
        // Equal neighbours: flat between them.
        let plateau = curve(&[(0.0, 5.0), (10.0, 5.0), (20.0, 9.0)], Interpolation::Spline);
        assert!((0..=10).all(|f| close(plateau.value_at(f64::from(f)).unwrap(), 5.0)));
    }

    #[test]
    fn manual_tangents_set_the_slope_at_their_key() {
        let mut c = curve(&[(0.0, 0.0), (10.0, 0.0)], Interpolation::Spline);
        c.set_key(Key {
            tangents: Tangents::Manual {
                left: (2.0, -4.0),
                right: (3.0, 6.0), // slope 2 per frame
            },
            ..Key::new(0.0, 0.0)
        });
        let slope = f64::from(c.value_at(0.001).unwrap() - c.value_at(0.0).unwrap()) / 0.001;
        assert!((slope - 2.0).abs() < 0.05, "{slope}");
    }

    #[test]
    fn tracks_animate_a_transform() {
        let tracks = [
            Track {
                param: Param::Position(Axis::Y),
                curve: curve(&[(0.0, 0.0), (10.0, 200.0)], Interpolation::Linear),
            },
            Track {
                param: Param::Scale(Axis::X),
                curve: curve(&[(0.0, 1.0)], Interpolation::Linear),
            },
        ];
        let base = Transform {
            scale: glam::Vec3::splat(3.0),
            ..Transform::IDENTITY
        };
        let t = tracks.animate(&base, 5.0);
        assert!(close(t.position.y, 100.0));
        assert_eq!(
            t.scale,
            glam::Vec3::new(1.0, 3.0, 3.0),
            "unanimated parameters keep the base"
        );
        assert!(tracks.track(Param::Scale(Axis::X)).is_some());
        assert!(tracks.track(Param::Rotation(Axis::X)).is_none());
        for p in Param::TRANSFORM {
            let mut t = base;
            p.set(&mut t, 0.5);
            assert_eq!(p.get(&t), 0.5, "{}", p.label());
        }
    }
}
