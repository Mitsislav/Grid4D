//! Animation data (see `docs/reference/animation-keys.md`): an object's animated
//! parameters each have a [`Track`] with a [`Curve`] of [`Key`]s. Tracks live in
//! [`NodeData::tracks`](crate::NodeData::tracks), so editing them is an ordinary
//! data edit (undo, save).
//!
//! What the user sees and edits are the values **at the current frame**
//! ([`NodeData::transform_at`]); edits go back through
//! [`NodeData::set_transform_at`], which writes keys (Autokeying) or temporary
//! values (see `docs/reference/animation-recording.md`).

use std::borrow::Cow;

use crate::{NodeData, ObjectKind, ObjectParam, Transform};

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
    /// One of the object's own numbers (a radius, an offset, …).
    Object(ObjectParam),
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
            Self::Object(param) => param.label(),
        }
    }

    /// A distinct number: the position in [`Param::TRANSFORM`], then the object
    /// parameters after them.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Position(axis) => axis.index(),
            Self::Rotation(axis) => 3 + axis.index(),
            Self::Scale(axis) => 6 + axis.index(),
            Self::Object(param) => 9 + param.index(),
        }
    }

    /// This parameter's value in `transform`; `None` for object parameters.
    #[must_use]
    pub fn get(self, transform: &Transform) -> Option<f32> {
        match self {
            Self::Position(axis) => Some(transform.position[axis.index()]),
            Self::Rotation(axis) => Some(transform.rotation[axis.index()]),
            Self::Scale(axis) => Some(transform.scale[axis.index()]),
            Self::Object(_) => None,
        }
    }

    /// Sets this parameter in `transform` (object parameters are not in it).
    pub fn set(self, transform: &mut Transform, value: f32) {
        match self {
            Self::Position(axis) => transform.position[axis.index()] = value,
            Self::Rotation(axis) => transform.rotation[axis.index()] = value,
            Self::Scale(axis) => transform.scale[axis.index()] = value,
            Self::Object(_) => {}
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

/// A preset for a key's tangents (the F-Curve editor's Tangents menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TangentPreset {
    /// Automatic: smooth, clamped.
    Auto,
    /// Flat handles of the current lengths: the curve does not pass the key's value
    /// next to it.
    ZeroAngle,
    /// No handles: next to another such key the segment is straight.
    ZeroLength,
}

impl TangentPreset {
    pub const ALL: [Self; 3] = [Self::Auto, Self::ZeroAngle, Self::ZeroLength];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto Tangents",
            Self::ZeroAngle => "Zero Angle",
            Self::ZeroLength => "Zero Length",
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

    /// The key at `frame`, if there is one.
    #[must_use]
    pub fn key_at(&self, frame: f64) -> Option<&Key> {
        let i = self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)).ok()?;
        self.keys.get(i)
    }

    /// Sets the value at `frame`: a key already there keeps its interpolation and
    /// tangents, a new one gets the defaults.
    pub fn record(&mut self, frame: f64, value: f32) {
        match self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)) {
            Ok(i) => self.keys[i].value = value,
            Err(i) => self.keys.insert(i, Key::new(frame, value)),
        }
    }

    /// Moves the keys whose frame is `selected` by `by` frames; they replace keys
    /// already at their new frames.
    pub fn move_keys(&mut self, selected: impl Fn(f64) -> bool, by: f64) {
        self.shift_keys(selected, by, 0.0);
    }

    /// Moves the keys whose frame is `selected` by `frames` in time and `value`
    /// in value; they replace keys already at their new frames.
    pub fn shift_keys(&mut self, selected: impl Fn(f64) -> bool, frames: f64, value: f32) {
        let (mut moved, kept): (Vec<Key>, Vec<Key>) = self.keys.drain(..).partition(|k| selected(k.frame));
        self.keys = kept;
        for key in &mut moved {
            key.frame += frames;
            key.value += value;
        }
        for key in moved {
            self.set_key(key);
        }
    }

    /// Sets the interpolation of the key at `frame`; `true` if there is one.
    pub fn set_interpolation(&mut self, frame: f64, interpolation: Interpolation) -> bool {
        let Ok(i) = self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)) else {
            return false;
        };
        self.keys[i].interpolation = interpolation;
        true
    }

    /// Sets the tangents of the key at `frame`; `true` if there is one.
    pub fn set_tangents(&mut self, frame: f64, tangents: Tangents) -> bool {
        let Ok(i) = self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)) else {
            return false;
        };
        self.keys[i].tangents = tangents;
        true
    }

    /// Applies `preset` to the key at `frame`; `true` if there is one.
    pub fn apply_tangent_preset(&mut self, frame: f64, preset: TangentPreset) -> bool {
        let Ok(i) = self.keys.binary_search_by(|k| k.frame.total_cmp(&frame)) else {
            return false;
        };
        let ((left, _), (right, _)) = self.handles(i);
        self.keys[i].tangents = match preset {
            TangentPreset::Auto => Tangents::Auto,
            TangentPreset::ZeroAngle => Tangents::Manual {
                left: (left, 0.0),
                right: (right, 0.0),
            },
            TangentPreset::ZeroLength => Tangents::Manual {
                left: (0.0, 0.0),
                right: (0.0, 0.0),
            },
        };
        true
    }

    /// Removes the keys whose frame is `selected`; `true` if there were any.
    pub fn remove_keys(&mut self, selected: impl Fn(f64) -> bool) -> bool {
        let before = self.keys.len();
        self.keys.retain(|k| !selected(k.frame));
        self.keys.len() != before
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
    /// shortened so it stays within the neighbouring segment: the left one points
    /// back in time (its end is at `frame - left.0`, `value - left.1`), the right
    /// one forwards.
    ///
    /// # Panics
    /// If `i` is not a key index.
    #[must_use]
    pub fn handles(&self, i: usize) -> ((f64, f32), (f64, f32)) {
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

/// A set of [`Param`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParamSet(u128);

impl ParamSet {
    #[must_use]
    pub const fn contains(self, param: Param) -> bool {
        self.0 & (1 << param.index()) != 0
    }

    pub fn insert(&mut self, param: Param) {
        self.0 |= 1 << param.index();
    }

    pub fn remove(&mut self, param: Param) {
        self.0 &= !(1 << param.index());
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Animated parameters changed at one frame without being recorded: their new
/// values are in the object's base transform and are shown only at `frame`.
/// Kept with the object so that undo covers it; never saved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unkeyed {
    pub frame: f64,
    pub params: ParamSet,
}

/// What a parameter's key button shows at a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// No track.
    Unanimated,
    /// A track, no key at this frame.
    Track,
    /// A key at this frame.
    Key,
    /// A temporary value that differs from the curve; `key`: a key at this frame.
    Changed { key: bool },
}

impl NodeData {
    /// `param`'s value without animation: in the base transform or the
    /// object's own parameters (0 for a parameter it does not have).
    #[must_use]
    pub fn base_value(&self, param: Param) -> f32 {
        match param {
            Param::Object(p) => self.kind.param(p).unwrap_or(0.0),
            Param::Position(_) | Param::Rotation(_) | Param::Scale(_) => param.get(&self.transform).unwrap_or(0.0),
        }
    }

    fn set_base_value(&mut self, param: Param, value: f32) {
        match param {
            Param::Object(p) => self.kind.set_param(p, value),
            Param::Position(_) | Param::Rotation(_) | Param::Scale(_) => param.set(&mut self.transform, value),
        }
    }

    /// `param`'s value at `frame`: its curve's, unless it was changed at that
    /// frame without recording (then the base value).
    #[must_use]
    pub fn value_at(&self, param: Param, frame: f64) -> f32 {
        let base = self.base_value(param);
        if self.is_unkeyed(param, frame) {
            return base;
        }
        self.tracks
            .track(param)
            .and_then(|t| t.curve.value_at(frame))
            .unwrap_or(base)
    }

    /// The transform shown at `frame` (what the viewport, the gizmo and the
    /// Coordinates fields work with).
    #[must_use]
    pub fn transform_at(&self, frame: f64) -> Transform {
        let mut transform = self.transform;
        for track in &self.tracks {
            if !self.is_unkeyed(track.param, frame)
                && let Some(value) = track.curve.value_at(frame)
            {
                track.param.set(&mut transform, value);
            }
        }
        transform
    }

    /// The object's own parameters at `frame` (what the viewport, generators
    /// and the Attribute Manager work with). Borrowed — no copy — when none is
    /// animated.
    #[must_use]
    pub fn kind_at(&self, frame: f64) -> Cow<'_, ObjectKind> {
        if !self.tracks.iter().any(|t| matches!(t.param, Param::Object(_))) {
            return Cow::Borrowed(&self.kind);
        }
        let mut kind = self.kind.clone();
        for track in &self.tracks {
            if let Param::Object(param) = track.param
                && !self.is_unkeyed(track.param, frame)
                && let Some(value) = track.curve.value_at(frame)
            {
                kind.set_param(param, value);
            }
        }
        Cow::Owned(kind)
    }

    /// Writes `shown`, the transform as edited at `frame`, back: each parameter
    /// whose shown value changed is set — an unanimated one in the base
    /// transform; an animated one as a key at `frame` with `autokey`, else as a
    /// temporary value.
    pub fn set_transform_at(&mut self, shown: &Transform, frame: f64, autokey: bool) {
        let current = self.transform_at(frame);
        for param in Param::TRANSFORM {
            if let Some(value) = param.get(shown)
                && Some(value) != param.get(&current)
            {
                self.set_value_at(param, value, frame, autokey);
            }
        }
    }

    /// Writes `edited`, the object's parameters as edited at `frame`, back like
    /// [`set_transform_at`](Self::set_transform_at): changed animated parameters
    /// become keys (with `autokey`) or temporary values; everything else —
    /// unanimated numbers, choices, counts, text — is set for all frames.
    pub fn set_kind_at(&mut self, edited: &ObjectKind, frame: f64, autokey: bool) {
        let shown = self.kind_at(frame).into_owned();
        let animated: Vec<ObjectParam> = self
            .tracks
            .iter()
            .filter_map(|t| match t.param {
                Param::Object(p) => Some(p),
                Param::Position(_) | Param::Rotation(_) | Param::Scale(_) => None,
            })
            .collect();
        let mut base = edited.clone();
        // Animated parameters keep their base values; changes go through keys.
        for &p in &animated {
            if let Some(value) = self.kind.param(p) {
                base.set_param(p, value);
            }
        }
        self.kind = base;
        for p in animated {
            if let Some(value) = edited.param(p)
                && Some(value) != shown.param(p)
            {
                self.set_value_at(Param::Object(p), value, frame, autokey);
            }
        }
    }

    /// Sets one parameter as [`set_transform_at`](Self::set_transform_at) does.
    pub fn set_value_at(&mut self, param: Param, value: f32, frame: f64, autokey: bool) {
        let Some(track) = self.tracks.iter_mut().find(|t| t.param == param) else {
            self.set_base_value(param, value);
            return;
        };
        if autokey {
            track.curve.record(frame, value);
            self.forget_unkeyed(param);
        } else {
            self.set_base_value(param, value);
            let mut params = self
                .unkeyed
                .filter(|u| u.frame == frame)
                .map_or_else(ParamSet::default, |u| u.params);
            params.insert(param);
            self.unkeyed = Some(Unkeyed { frame, params });
        }
    }

    /// The key button of `param` at `frame`.
    #[must_use]
    pub fn key_state(&self, param: Param, frame: f64) -> KeyState {
        let Some(track) = self.tracks.track(param) else {
            return KeyState::Unanimated;
        };
        let key = track.curve.key_at(frame).is_some();
        if self.is_unkeyed(param, frame) && track.curve.value_at(frame) != Some(self.base_value(param)) {
            KeyState::Changed { key }
        } else if key {
            KeyState::Key
        } else {
            KeyState::Track
        }
    }

    /// Records the values of `params` shown at frame `from` as keys at frame `at`,
    /// creating tracks as needed; those parameters are no longer temporary.
    pub fn record(&mut self, params: impl IntoIterator<Item = Param>, from: f64, at: f64) {
        for param in params {
            let value = self.value_at(param, from);
            match self.tracks.iter_mut().find(|t| t.param == param) {
                Some(track) => track.curve.record(at, value),
                None => self.tracks.push(Track {
                    param,
                    curve: Curve::from_keys([Key::new(at, value)]),
                }),
            }
            self.forget_unkeyed(param);
        }
    }

    /// A click on `param`'s key button at `frame`: removes the key there if the
    /// button shows one, else records the shown value as a key.
    pub fn toggle_key(&mut self, param: Param, frame: f64) {
        if self.key_state(param, frame) == KeyState::Key {
            self.remove_keys(|p, f| p == param && f == frame, frame);
        } else {
            self.record([param], frame, frame);
        }
    }

    /// Removes `param`'s track; the parameter keeps the value shown at `frame`.
    pub fn delete_track(&mut self, param: Param, frame: f64) {
        self.remove_keys(|p, _| p == param, frame);
    }

    /// Removes the keys for which `selected(param, frame)` holds. A track left
    /// without keys goes, and its parameter keeps the value shown at `current`.
    /// Returns whether anything was removed.
    pub fn remove_keys(&mut self, selected: impl Fn(Param, f64) -> bool, current: f64) -> bool {
        let shown: Vec<(Param, f32)> = self
            .tracks
            .iter()
            .map(|t| (t.param, self.value_at(t.param, current)))
            .collect();
        let mut removed = false;
        for track in &mut self.tracks {
            let param = track.param;
            removed |= track.curve.remove_keys(|f| selected(param, f));
        }
        for (param, value) in shown {
            if self.tracks.track(param).is_some_and(|t| t.curve.is_empty()) {
                self.set_base_value(param, value);
                self.forget_unkeyed(param);
            }
        }
        self.tracks.retain(|t| !t.curve.is_empty());
        removed
    }

    /// Moves every key whose frame is `selected` by `by` frames, on every track.
    pub fn move_keys(&mut self, selected: impl Fn(f64) -> bool + Copy, by: f64) {
        for track in &mut self.tracks {
            track.curve.move_keys(selected, by);
        }
    }

    fn is_unkeyed(&self, param: Param, frame: f64) -> bool {
        self.unkeyed
            .is_some_and(|u| u.frame == frame && u.params.contains(param))
    }

    fn forget_unkeyed(&mut self, param: Param) {
        if let Some(unkeyed) = &mut self.unkeyed {
            unkeyed.params.remove(param);
            if unkeyed.params.is_empty() {
                self.unkeyed = None;
            }
        }
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
            assert_eq!(p.get(&t), Some(0.5), "{}", p.label());
            assert_eq!(Param::TRANSFORM[p.index()], p);
        }
    }

    const PX: Param = Param::Position(Axis::X);
    const PY: Param = Param::Position(Axis::Y);

    /// An object whose position X goes 0 → 100 over frames 0–10 (linear).
    fn animated() -> NodeData {
        let mut data = NodeData::new("a", ObjectKind::Null);
        data.tracks.push(Track {
            param: PX,
            curve: curve(&[(0.0, 0.0), (10.0, 100.0)], Interpolation::Linear),
        });
        data
    }

    fn moved_x(data: &NodeData, frame: f64, x: f32) -> Transform {
        let mut shown = data.transform_at(frame);
        shown.position.x = x;
        shown
    }

    #[test]
    fn a_change_without_autokeying_is_temporary_at_its_frame() {
        let mut data = animated();
        assert_eq!(data.transform_at(5.0).position.x, 50.0, "the curve's value");
        data.set_transform_at(&moved_x(&data, 5.0, 70.0), 5.0, false);
        assert_eq!(data.value_at(PX, 5.0), 70.0, "shown at its frame");
        assert!(close(data.value_at(PX, 6.0), 60.0), "the curve elsewhere");
        assert_eq!(data.key_state(PX, 5.0), KeyState::Changed { key: false });
        assert_eq!(data.key_state(PX, 6.0), KeyState::Track);
        assert_eq!(data.key_state(PY, 5.0), KeyState::Unanimated);
        assert_eq!(data.tracks[0].curve.keys().len(), 2, "no key written");
        // Recording keeps it as a key.
        data.record(Param::TRANSFORM, 5.0, 5.0);
        assert_eq!(data.unkeyed, None);
        assert_eq!(data.tracks.len(), 9, "every parameter recorded");
        assert_eq!(data.value_at(PX, 5.0), 70.0);
        assert_eq!(data.key_state(PX, 5.0), KeyState::Key);
    }

    #[test]
    fn autokeying_writes_keys_for_animated_parameters_only() {
        let mut data = animated();
        let mut shown = moved_x(&data, 5.0, 80.0);
        shown.position.y = 30.0;
        data.set_transform_at(&shown, 5.0, true);
        assert_eq!(data.tracks.len(), 1, "no track for the unanimated Y");
        assert_eq!(data.transform.position.y, 30.0, "Y simply changes");
        assert_eq!(data.tracks[0].curve.key_at(5.0).map(|k| k.value), Some(80.0));
        assert_eq!(data.unkeyed, None);
        // A key already there keeps its interpolation.
        data.set_transform_at(&moved_x(&data, 0.0, -5.0), 0.0, true);
        let first = data.tracks[0].curve.keys()[0];
        assert_eq!((first.value, first.interpolation), (-5.0, Interpolation::Linear));
    }

    #[test]
    fn key_buttons_set_and_remove_keys_and_tracks() {
        let mut data = animated();
        data.toggle_key(PX, 5.0);
        assert_eq!(data.key_state(PX, 5.0), KeyState::Key);
        assert_eq!(data.value_at(PX, 5.0), 50.0, "the shown value");
        data.toggle_key(PX, 5.0);
        assert_eq!(data.key_state(PX, 5.0), KeyState::Track, "a second click removes it");
        // A key on an unanimated parameter starts its track.
        data.toggle_key(PY, 3.0);
        assert_eq!(data.key_state(PY, 3.0), KeyState::Key);
        // Removing the last key removes the track; the value shown stays.
        data.toggle_key(PY, 3.0);
        assert!(data.tracks.track(PY).is_none());
        data.delete_track(PX, 4.0);
        assert!(data.tracks.is_empty());
        assert_eq!(data.transform.position.x, 40.0, "kept as shown at frame 4");
    }

    #[test]
    fn moving_and_removing_keys() {
        let mut c = curve(&[(0.0, 1.0), (5.0, 2.0), (10.0, 3.0)], Interpolation::Linear);
        c.move_keys(|f| f == 0.0 || f == 5.0, 5.0);
        let keys: Vec<(f64, f32)> = c.keys().iter().map(|k| (k.frame, k.value)).collect();
        assert_eq!(keys, [(5.0, 1.0), (10.0, 2.0)], "moved keys replace the key at 10");
        let mut data = animated();
        assert!(data.remove_keys(|_, f| f == 10.0, 10.0));
        assert_eq!(data.tracks[0].curve.keys().len(), 1);
        assert!(!data.remove_keys(|_, f| f == 99.0, 0.0), "nothing there");
    }

    #[test]
    fn shifting_keys_in_time_and_value() {
        let mut c = curve(&[(0.0, 1.0), (10.0, 2.0), (20.0, 3.0)], Interpolation::Linear);
        c.shift_keys(|f| f == 10.0, 5.0, 0.5);
        let keys: Vec<(f64, f32)> = c.keys().iter().map(|k| (k.frame, k.value)).collect();
        assert_eq!(keys, [(0.0, 1.0), (15.0, 2.5), (20.0, 3.0)]);
        assert_eq!(
            c.keys()[1].interpolation,
            Interpolation::Linear,
            "the key is kept as it was"
        );
    }

    #[test]
    fn interpolation_and_tangent_presets() {
        let mut c = curve(&[(0.0, 0.0), (10.0, 100.0), (20.0, 50.0)], Interpolation::Spline);
        assert!(c.set_interpolation(0.0, Interpolation::Step));
        assert!(!c.set_interpolation(3.0, Interpolation::Step), "no key there");
        assert_eq!(c.value_at(9.0), Some(0.0), "Step holds");

        // Zero Angle: flat at the key, so the curve does not go past its value.
        let mut c = curve(&[(0.0, 0.0), (10.0, 50.0), (20.0, 100.0)], Interpolation::Spline);
        assert!(c.apply_tangent_preset(10.0, TangentPreset::ZeroAngle));
        let ((l, lv), (r, rv)) = c.handles(1);
        assert!(
            (l - 10.0 / 3.0).abs() < 1e-9 && (r - 10.0 / 3.0).abs() < 1e-9,
            "lengths kept"
        );
        assert_eq!((lv, rv), (0.0, 0.0));
        let slope = f64::from(c.value_at(10.01).unwrap() - c.value_at(10.0).unwrap()) / 0.01;
        assert!(slope.abs() < 0.05, "flat: {slope}");

        // Zero Length on both ends of a segment: a straight line.
        let mut c = curve(&[(0.0, 0.0), (10.0, 100.0)], Interpolation::Spline);
        for frame in [0.0, 10.0] {
            c.apply_tangent_preset(frame, TangentPreset::ZeroLength);
        }
        assert!(close(c.value_at(5.0).unwrap(), 50.0));
        assert!(close(c.value_at(2.5).unwrap(), 25.0));
        c.apply_tangent_preset(0.0, TangentPreset::Auto);
        assert_eq!(c.keys()[0].tangents, Tangents::Auto);
    }

    #[test]
    fn object_parameters_key_and_stay_temporary_like_transforms() {
        use crate::ObjectParam::Radius;
        let mut data = NodeData::new(
            "s",
            ObjectKind::Sphere {
                radius: 100.0,
                segments: 24,
            },
        );
        let r = Param::Object(Radius);
        assert!(
            matches!(data.kind_at(3.0), Cow::Borrowed(_)),
            "nothing animated: no copy"
        );
        assert_eq!(data.key_state(r, 0.0), KeyState::Unanimated);
        data.record([r], 0.0, 0.0);
        data.set_value_at(r, 200.0, 10.0, true);
        assert_eq!(data.kind_at(0.0).param(Radius), Some(100.0));
        assert_eq!(data.kind_at(10.0).param(Radius), Some(200.0));
        assert_eq!(data.key_state(r, 10.0), KeyState::Key);
        assert_eq!(data.value_at(r, 10.0), 200.0);

        // Without Autokeying an edit is temporary: shown at its frame only.
        let mut edited = data.kind_at(5.0).into_owned();
        edited.set_param(Radius, 999.0);
        data.set_kind_at(&edited, 5.0, false);
        assert_eq!(data.kind_at(5.0).param(Radius), Some(999.0));
        assert_eq!(data.key_state(r, 5.0), KeyState::Changed { key: false });
        assert_ne!(data.kind_at(6.0).param(Radius), Some(999.0));

        // Choices and counts are set for all frames; animated values keep their keys.
        let mut edited = data.kind_at(5.0).into_owned();
        let ObjectKind::Sphere { segments, .. } = &mut edited else {
            unreachable!()
        };
        *segments = 8;
        data.set_kind_at(&edited, 5.0, true);
        assert!(matches!(data.kind, ObjectKind::Sphere { segments: 8, .. }));
        assert_eq!(
            data.tracks[0].curve.keys().len(),
            2,
            "no key added for an unchanged value"
        );

        // Removing every key leaves the value shown at the current frame.
        data.remove_keys(|_, _| true, 10.0);
        assert!(data.tracks.is_empty());
        assert_eq!(data.kind.param(Radius), Some(200.0));
    }
}
