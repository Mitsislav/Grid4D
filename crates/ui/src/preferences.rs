//! The user's preferences: settings that belong to the user, not to a
//! document — the length unit, undo depth, autosave and view defaults. The
//! binary saves them in eframe storage. Spec:
//! `docs/reference/workspace-preferences.md`.

use grid4d_render::ShadingMode;
use serde::{Deserialize, Serialize};

/// Where the binary keeps the preferences in eframe storage.
pub const STORAGE_KEY: &str = "grid4d.preferences";

/// The unit lengths are shown and typed in. Documents keep centimetres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Centimetres,
    Metres,
    Millimetres,
    Inches,
    Feet,
}

impl Units {
    pub const ALL: [Self; 5] = [
        Self::Centimetres,
        Self::Metres,
        Self::Millimetres,
        Self::Inches,
        Self::Feet,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Centimetres => "Centimetres",
            Self::Metres => "Metres",
            Self::Millimetres => "Millimetres",
            Self::Inches => "Inches",
            Self::Feet => "Feet",
        }
    }

    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Centimetres => "cm",
            Self::Metres => "m",
            Self::Millimetres => "mm",
            Self::Inches => "in",
            Self::Feet => "ft",
        }
    }

    /// Centimetres in one of this unit.
    #[must_use]
    pub const fn centimetres(self) -> f64 {
        match self {
            Self::Centimetres => 1.0,
            Self::Metres => 100.0,
            Self::Millimetres => 0.1,
            Self::Inches => 2.54,
            Self::Feet => 30.48,
        }
    }

    /// Decimals to add to a centimetre field's so a value keeps its precision.
    const fn extra_decimals(self) -> usize {
        match self {
            Self::Centimetres | Self::Millimetres => 0,
            Self::Inches => 1,
            Self::Metres | Self::Feet => 2,
        }
    }

    /// `cm` as text in this unit with at most `decimals` (a centimetre
    /// field's) plus what the unit needs, trailing zeros dropped, then the
    /// symbol and `per` (`"/s"` for speeds): `150` → `"1.5 m"`.
    #[must_use]
    pub fn format(self, cm: f64, decimals: usize, per: &str) -> String {
        let value = cm / self.centimetres();
        let mut text = format!("{value:.*}", decimals + self.extra_decimals());
        if text.contains('.') {
            let kept = text.trim_end_matches('0').trim_end_matches('.').len();
            text.truncate(kept);
        }
        if text == "-0" {
            text.remove(0);
        }
        format!("{text} {}{per}", self.symbol())
    }

    /// Text typed into a length field, in centimetres: a number in this unit,
    /// or one followed by a unit's symbol (`"2 m"`, `"15mm"`), optionally with
    /// `per` after it. `None` if it is not a length.
    #[must_use]
    pub fn parse(self, text: &str, per: &str) -> Option<f64> {
        let text = text.trim();
        let text = if per.is_empty() {
            text
        } else {
            text.strip_suffix(per).unwrap_or(text)
        };
        let number_end = text
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_ascii_alphabetic())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let (number, symbol) = text.split_at(number_end);
        let unit = match symbol.to_ascii_lowercase().as_str() {
            "" => self,
            symbol => *Self::ALL.iter().find(|u| u.symbol() == symbol)?,
        };
        let value: f64 = number.trim().parse().ok()?;
        value.is_finite().then(|| value * unit.centimetres())
    }
}

/// The views' look when Grid4D starts (and on Apply to Views).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewDefaults {
    #[serde(with = "shading_name")]
    pub shading: ShadingMode,
    pub grid: bool,
    pub world_axis: bool,
    pub hud: bool,
}

impl Default for ViewDefaults {
    fn default() -> Self {
        Self {
            shading: ShadingMode::default(),
            grid: true,
            world_axis: true,
            hud: true,
        }
    }
}

/// The user's preferences. Values missing from a stored copy take their
/// defaults; [`Preferences::checked`] brings the rest into range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub units: Units,
    /// Undo steps kept.
    pub undo_depth: usize,
    /// Write a recovery copy while there are unsaved changes.
    pub autosave: bool,
    /// How often, in minutes.
    pub autosave_minutes: u32,
    pub views: ViewDefaults,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            units: Units::default(),
            undo_depth: crate::History::DEFAULT_LIMIT,
            autosave: true,
            autosave_minutes: 5,
            views: ViewDefaults::default(),
        }
    }
}

impl Preferences {
    pub const UNDO_DEPTH: std::ops::RangeInclusive<usize> = 1..=10_000;
    pub const AUTOSAVE_MINUTES: std::ops::RangeInclusive<u32> = 1..=120;

    /// The preferences with every value in its range (a stored copy may come
    /// from another version).
    #[must_use]
    pub fn checked(mut self) -> Self {
        self.undo_depth = self
            .undo_depth
            .clamp(*Self::UNDO_DEPTH.start(), *Self::UNDO_DEPTH.end());
        self.autosave_minutes = self
            .autosave_minutes
            .clamp(*Self::AUTOSAVE_MINUTES.start(), *Self::AUTOSAVE_MINUTES.end());
        self
    }

    /// Seconds between recovery copies; `None` when autosave is off.
    #[must_use]
    pub fn autosave_interval(&self) -> Option<f64> {
        self.autosave.then(|| f64::from(self.autosave_minutes) * 60.0)
    }
}

/// Shading modes by name in stored preferences.
mod shading_name {
    use grid4d_render::ShadingMode;
    use serde::{Deserialize, Deserializer, Serializer};

    const fn name(mode: ShadingMode) -> &'static str {
        match mode {
            ShadingMode::Gouraud => "Gouraud",
            ShadingMode::GouraudLines => "GouraudLines",
            ShadingMode::Quick => "Quick",
            ShadingMode::QuickLines => "QuickLines",
            ShadingMode::Constant => "Constant",
            ShadingMode::ConstantLines => "ConstantLines",
            ShadingMode::HiddenLine => "HiddenLine",
            ShadingMode::Lines => "Lines",
        }
    }

    #[expect(clippy::trivially_copy_pass_by_ref, reason = "the signature serde's `with` expects")]
    pub fn serialize<S: Serializer>(mode: &ShadingMode, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(name(*mode))
    }

    /// An unknown name is the default shading.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ShadingMode, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(ShadingMode::ALL
            .into_iter()
            .find(|&mode| name(mode) == text)
            .unwrap_or_default())
    }
}

/// The length unit the panels show this frame (set by the app from the
/// preferences; centimetres if never set).
#[must_use]
pub fn units(ctx: &egui::Context) -> Units {
    ctx.data(|d| d.get_temp(units_id())).unwrap_or_default()
}

pub fn set_units(ctx: &egui::Context, units: Units) {
    ctx.data_mut(|d| d.insert_temp(units_id(), units));
}

fn units_id() -> egui::Id {
    egui::Id::unique("grid4d.units")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_show_in_the_unit_and_typed_symbols_convert() {
        assert_eq!(Units::Centimetres.format(150.0, 2, ""), "150 cm");
        assert_eq!(Units::Metres.format(150.0, 2, ""), "1.5 m");
        assert_eq!(Units::Millimetres.format(1.25, 2, ""), "12.5 mm");
        assert_eq!(Units::Inches.format(2.54, 2, ""), "1 in");
        assert_eq!(Units::Feet.format(-30.48, 2, "/s"), "-1 ft/s");
        assert_eq!(Units::Metres.format(-0.000_01, 2, ""), "0 m");
        assert_eq!(Units::Metres.format(12.346, 2, ""), "0.1235 m");

        assert_eq!(Units::Metres.parse("2", ""), Some(200.0));
        assert_eq!(Units::Metres.parse(" 2 m ", ""), Some(200.0));
        assert_eq!(Units::Metres.parse("15mm", ""), Some(1.5));
        assert_eq!(Units::Centimetres.parse("3 in", ""), Some(3.0 * 2.54));
        assert_eq!(Units::Centimetres.parse("-1.5 FT", ""), Some(-1.5 * 30.48));
        assert_eq!(Units::Metres.parse("3 m/s", "/s"), Some(300.0));
        assert_eq!(Units::Metres.parse("3 km", ""), None);
        assert_eq!(Units::Metres.parse("m", ""), None);
        assert_eq!(Units::Metres.parse("inf", ""), None);

        // What a field shows reads back as the same length in every unit.
        for units in Units::ALL {
            for cm in [0.0, 1.0, 12.34, -250.5, 100_000.0] {
                let back = units.parse(&units.format(cm, 2, ""), "").unwrap();
                assert!((back - cm).abs() < 0.01, "{units:?} {cm} → {back}");
            }
        }
    }

    #[test]
    fn stored_preferences_round_trip_and_bad_values_are_brought_into_range() {
        let preferences = Preferences {
            units: Units::Feet,
            undo_depth: 25,
            autosave: false,
            autosave_minutes: 10,
            views: ViewDefaults {
                shading: ShadingMode::QuickLines,
                grid: false,
                world_axis: true,
                hud: false,
            },
        };
        let text = serde_json::to_string(&preferences).unwrap();
        assert_eq!(serde_json::from_str::<Preferences>(&text).unwrap(), preferences);

        // Another version's copy: missing fields, an unknown shading, values out of range.
        let other: Preferences =
            serde_json::from_str(r#"{"undo_depth": 0, "autosave_minutes": 999, "views": {"shading": "Toon"}}"#)
                .unwrap();
        let other = other.checked();
        assert_eq!(other.units, Units::Centimetres);
        assert_eq!((other.undo_depth, other.autosave_minutes), (1, 120));
        assert_eq!(other.views, ViewDefaults::default());
        assert_eq!(Preferences::default().autosave_interval(), Some(300.0));
    }
}
