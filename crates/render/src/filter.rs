//! What a view shows: its Filter menu (see `docs/reference/viewport-filter.md`).

use grid4d_scene::ObjectKindTag;

/// One entry of the Filter menu: a kind of object, or an element of the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterItem {
    Polygon,
    /// Parametric objects: the primitives and the spline generators.
    Generator,
    SubdivisionSurface,
    /// Splines (curves).
    Spline,
    Camera,
    Light,
    /// The grid.
    Grid,
    /// The axis triad in the corner.
    WorldAxis,
    Hud,
}

impl FilterItem {
    pub const ALL: [Self; 9] = [
        Self::Polygon,
        Self::Generator,
        Self::SubdivisionSurface,
        Self::Spline,
        Self::Camera,
        Self::Light,
        Self::Grid,
        Self::WorldAxis,
        Self::Hud,
    ];

    /// The entry that shows objects of `kind`; `None` for kinds that draw nothing
    /// (Nulls).
    #[must_use]
    pub const fn of(kind: ObjectKindTag) -> Option<Self> {
        match kind {
            ObjectKindTag::Null => None,
            ObjectKindTag::Cube
            | ObjectKindTag::Sphere
            | ObjectKindTag::Cylinder
            | ObjectKindTag::Plane
            | ObjectKindTag::Cone
            | ObjectKindTag::Torus
            | ObjectKindTag::Disc
            | ObjectKindTag::Capsule => Some(Self::Generator),
            ObjectKindTag::Polygon => Some(Self::Polygon),
            ObjectKindTag::SubdivisionSurface => Some(Self::SubdivisionSurface),
            ObjectKindTag::Extrude | ObjectKindTag::Lathe | ObjectKindTag::Sweep | ObjectKindTag::Loft => {
                Some(Self::Generator)
            }
            ObjectKindTag::Camera => Some(Self::Camera),
            ObjectKindTag::Light => Some(Self::Light),
            ObjectKindTag::Circle
            | ObjectKindTag::Rectangle
            | ObjectKindTag::Arc
            | ObjectKindTag::NSide
            | ObjectKindTag::Star
            | ObjectKindTag::Helix
            | ObjectKindTag::Flower
            | ObjectKindTag::Spline => Some(Self::Spline),
        }
    }

    const fn bit(self) -> u16 {
        match self {
            Self::Polygon => 1,
            Self::Generator => 1 << 1,
            Self::SubdivisionSurface => 1 << 2,
            Self::Camera => 1 << 3,
            Self::Light => 1 << 4,
            Self::Grid => 1 << 5,
            Self::WorldAxis => 1 << 6,
            Self::Hud => 1 << 7,
            Self::Spline => 1 << 8,
        }
    }
}

/// What a view shows: each [`FilterItem`] on or off (all on by default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewFilter {
    hidden: u16,
}

impl ViewFilter {
    /// Everything shown.
    pub const ALL: Self = Self { hidden: 0 };
    /// Everything hidden.
    pub const NONE: Self = Self { hidden: 0x1FF };

    #[must_use]
    pub const fn shows(self, item: FilterItem) -> bool {
        self.hidden & item.bit() == 0
    }

    /// Whether objects of `kind` are drawn and picked (kinds without an entry are).
    #[must_use]
    pub const fn shows_object(self, kind: ObjectKindTag) -> bool {
        match FilterItem::of(kind) {
            Some(item) => self.shows(item),
            None => true,
        }
    }

    pub const fn set(&mut self, item: FilterItem, shown: bool) {
        if shown {
            self.hidden &= !item.bit();
        } else {
            self.hidden |= item.bit();
        }
    }

    /// Only `item` shown (Ctrl + choosing it).
    #[must_use]
    pub const fn only(item: FilterItem) -> Self {
        Self {
            hidden: Self::NONE.hidden & !item.bit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_toggle_and_all_none_and_only_cover_every_entry() {
        let mut filter = ViewFilter::default();
        assert!(FilterItem::ALL.iter().all(|&i| filter.shows(i)), "all shown at first");
        filter.set(FilterItem::Camera, false);
        assert!(!filter.shows(FilterItem::Camera) && filter.shows(FilterItem::Light));
        assert!(!filter.shows_object(ObjectKindTag::Camera));
        assert!(filter.shows_object(ObjectKindTag::Null), "Nulls have no entry");
        filter.set(FilterItem::Camera, true);
        assert_eq!(filter, ViewFilter::ALL);

        assert!(FilterItem::ALL.iter().all(|&i| !ViewFilter::NONE.shows(i)));
        let only = ViewFilter::only(FilterItem::Polygon);
        assert!(
            FilterItem::ALL
                .iter()
                .all(|&i| only.shows(i) == (i == FilterItem::Polygon))
        );
        // Every primitive is a generator; splines have their own entry.
        assert!(!only.shows_object(ObjectKindTag::Torus));
        assert!(!only.shows_object(ObjectKindTag::Circle));
        assert!(ViewFilter::only(FilterItem::Spline).shows_object(ObjectKindTag::Helix));
        assert!(only.shows_object(ObjectKindTag::Polygon));
    }
}
