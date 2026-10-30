//! The Grid4D look: dense "industrial" themes in the spirit of Cinema 4D's
//! UI — neutral greys, crisp 1 px separators, small corner radii, a
//! steel-blue accent for selection and orange for activity. Three palettes
//! (Dark, Darker, Light), one in use at a time (Preferences ▸ Theme). Spec:
//! `docs/reference/workspace-preferences.md`.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2, style::WidgetVisuals};
use serde::{Deserialize, Serialize};

/// Every UI colour of one theme — no ad-hoc literals in panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Text fields, timeline track, deepest wells.
    pub bg_well: Color32,
    /// Menu bar, manager headers, tab strips.
    pub bg_header: Color32,
    /// Manager bodies and toolbars.
    pub bg_panel: Color32,
    /// Buttons and other raised controls.
    pub bg_raised: Color32,
    pub bg_hover: Color32,
    pub bg_active: Color32,
    /// Faint stripes and fills.
    pub bg_faint: Color32,
    /// The 1 px grooves between panels.
    pub border: Color32,
    pub separator: Color32,
    /// Darkens everything behind an overlay (file drop target).
    pub scrim: Color32,
    /// Window and popup shadows.
    pub shadow: Color32,

    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,

    /// Selection, focus, playhead.
    pub accent: Color32,
    /// Selected rows / toggled buttons background.
    pub accent_bg: Color32,
    /// Active object, hovered gizmo handles, keyframes.
    pub highlight: Color32,
    pub record: Color32,
    /// Key buttons: a temporary value that differs from its curve.
    pub key_changed: Color32,
    /// Key marks on the timeline.
    pub key: Color32,

    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    pub axis: [Color32; 3],

    /// Object Manager traffic lights.
    pub vis_default: Color32,
    pub vis_hidden: Color32,
    pub enabled: Color32,

    /// Viewport background gradient.
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,

    /// Per-object-type icon tints (Object Manager, menus, toolbar).
    pub icon_primitive: Color32,
    pub icon_null: Color32,
    pub icon_camera: Color32,
    pub icon_light: Color32,
    /// Generators (Subdivision Surface …).
    pub icon_generator: Color32,
    /// Splines (curves).
    pub icon_spline: Color32,
    pub icon_deformer: Color32,
    pub icon_effector: Color32,
    /// A warning in the status bar.
    pub warning: Color32,
    /// The simulated frames on the timeline.
    pub sim_cache: Color32,
    /// Tag icons in the Object Manager.
    pub tag: Color32,
    /// The snap target a drag is stuck to, and its label.
    pub snap: Color32,
    /// Curves of an object's own parameters (F-Curve editor), in turn.
    pub param_curves: [Color32; 4],
}

const fn rgb(hex: u32) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

impl Palette {
    /// The original Grid4D look.
    pub const DARK: Self = Self {
        bg_well: rgb(0x18191b),
        bg_header: rgb(0x1f2023),
        bg_panel: rgb(0x2a2b2e),
        bg_raised: rgb(0x35373b),
        bg_hover: rgb(0x3f4247),
        bg_active: rgb(0x4a4e55),
        bg_faint: rgb(0x2f3034),
        border: rgb(0x101112),
        separator: rgb(0x3b3d42),
        scrim: Color32::from_black_alpha(170),
        shadow: Color32::from_black_alpha(120),
        text: rgb(0xc6c8cc),
        text_strong: rgb(0xeef0f2),
        text_dim: rgb(0x82868d),
        text_faint: rgb(0x5a5d63),
        accent: rgb(0x4f8ff2),
        accent_bg: rgb(0x2b456e),
        highlight: rgb(0xf58f2a),
        record: rgb(0xe03e3e),
        key_changed: rgb(0xe8c547),
        key: rgb(0xb4b8bf),
        axis_x: rgb(0xe84f4f),
        axis_y: rgb(0x7fc845),
        axis_z: rgb(0x4c8ef2),
        axis: [rgb(0xe84f4f), rgb(0x7fc845), rgb(0x4c8ef2)],
        vis_default: rgb(0x6e7177),
        vis_hidden: rgb(0xd94a45),
        enabled: rgb(0x6cc05a),
        viewport_top: rgb(0x4d5056),
        viewport_bottom: rgb(0x2b2d31),
        icon_primitive: rgb(0x6fa8f0),
        icon_null: rgb(0xb8bbc0),
        icon_camera: rgb(0xa98be8),
        icon_light: rgb(0xf2c94c),
        icon_generator: rgb(0x6cc28a),
        icon_spline: rgb(0x5cc6d6),
        icon_deformer: rgb(0xb78cf0),
        icon_effector: rgb(0xf09a5c),
        warning: rgb(0xf0b44c),
        sim_cache: rgb(0x5fb37a),
        tag: rgb(0xc8cdd6),
        snap: rgb(0xffd83d),
        param_curves: [rgb(0xf58f2a), rgb(0xa98be8), rgb(0x5cc6d6), rgb(0xf2c94c)],
    };

    /// Deeper backgrounds and a darker view, for dim rooms.
    pub const DARKER: Self = Self {
        bg_well: rgb(0x0f1011),
        bg_header: rgb(0x151618),
        bg_panel: rgb(0x1c1d1f),
        bg_raised: rgb(0x27282b),
        bg_hover: rgb(0x313337),
        bg_active: rgb(0x3c3f45),
        bg_faint: rgb(0x212225),
        border: rgb(0x08090a),
        separator: rgb(0x2e3034),
        text_dim: rgb(0x8a8e95),
        text_faint: rgb(0x55585e),
        accent_bg: rgb(0x263f66),
        vis_default: rgb(0x666970),
        viewport_top: rgb(0x3a3c41),
        viewport_bottom: rgb(0x1d1e21),
        ..Self::DARK
    };

    /// Light greys and dark text; the view keeps its darker grey (see
    /// [`view_palette`]).
    pub const LIGHT: Self = Self {
        bg_well: rgb(0xffffff),
        bg_header: rgb(0xd5d7db),
        bg_panel: rgb(0xe4e6e9),
        bg_raised: rgb(0xf4f5f7),
        bg_hover: rgb(0xd2d6dc),
        bg_active: rgb(0xc2c8d0),
        bg_faint: rgb(0xdcdee2),
        border: rgb(0xa9adb3),
        separator: rgb(0xc4c7cc),
        scrim: Color32::from_black_alpha(110),
        shadow: Color32::from_black_alpha(60),
        text: rgb(0x26282c),
        text_strong: rgb(0x0b0c0e),
        text_dim: rgb(0x565a61),
        text_faint: rgb(0x9a9ea5),
        accent: rgb(0x2463c9),
        accent_bg: rgb(0xc3d6f3),
        highlight: rgb(0xb05400),
        record: rgb(0xc62828),
        key_changed: rgb(0xa8800a),
        key: rgb(0x55595f),
        axis_x: rgb(0xc93434),
        axis_y: rgb(0x3f8a1a),
        axis_z: rgb(0x2463c9),
        axis: [rgb(0xc93434), rgb(0x3f8a1a), rgb(0x2463c9)],
        vis_default: rgb(0x8a8e94),
        vis_hidden: rgb(0xc93b36),
        enabled: rgb(0x3a8f2a),
        icon_primitive: rgb(0x2f72c9),
        icon_null: rgb(0x5d6168),
        icon_camera: rgb(0x7a55c9),
        icon_light: rgb(0xa87a00),
        icon_generator: rgb(0x2a8a50),
        icon_spline: rgb(0x1a8396),
        icon_deformer: rgb(0x8050d0),
        icon_effector: rgb(0xc0601c),
        warning: rgb(0xa86e00),
        sim_cache: rgb(0x3a8f56),
        tag: rgb(0x5d6168),
        param_curves: [rgb(0xb05400), rgb(0x7a55c9), rgb(0x1a8396), rgb(0xa87a00)],
        ..Self::DARK
    };
}

/// The themes a user can pick (Preferences ▸ Theme).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorTheme {
    #[default]
    Dark,
    Darker,
    Light,
}

impl ColorTheme {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Darker, Self::Light];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Darker => "Darker",
            Self::Light => "Light",
        }
    }

    #[must_use]
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &Palette::DARK,
            Self::Darker => &Palette::DARKER,
            Self::Light => &Palette::LIGHT,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Darker,
            2 => Self::Light,
            _ => Self::Dark,
        }
    }

    const fn to_u8(self) -> u8 {
        match self {
            Self::Dark => 0,
            Self::Darker => 1,
            Self::Light => 2,
        }
    }
}

/// The theme in use (a plain atomic, so reading a colour costs nothing).
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The theme the panels draw with.
#[must_use]
pub fn current() -> ColorTheme {
    ColorTheme::from_u8(CURRENT.load(Ordering::Relaxed))
}

/// The palette of the theme in use.
#[must_use]
pub fn palette() -> &'static Palette {
    current().palette()
}

/// Colours for what is drawn over the 3D view (HUD, gizmo, handles, overlays):
/// the view stays a darker grey in every theme, so a light theme draws there
/// with the Dark palette.
#[must_use]
pub fn view_palette() -> &'static Palette {
    match current() {
        ColorTheme::Dark | ColorTheme::Light => &Palette::DARK,
        ColorTheme::Darker => &Palette::DARKER,
    }
}

/// Standard heights (points) shared by panels so rows line up across managers.
pub mod metrics {
    pub const MENU_BAR_HEIGHT: f32 = 22.0;
    pub const TOOLBAR_HEIGHT: f32 = 36.0;
    pub const HEADER_HEIGHT: f32 = 24.0;
    pub const ROW_HEIGHT: f32 = 21.0;
    pub const STATUS_BAR_HEIGHT: f32 = 22.0;
    pub const ICON_BUTTON: f32 = 28.0;
    /// Component overlay (points): square size.
    pub const COMPONENT_POINT: f32 = 5.0;
    /// Component overlay: edge width, and selected/hovered edge width.
    pub const COMPONENT_EDGE: f32 = 1.25;
    pub const COMPONENT_EDGE_SELECTED: f32 = 2.75;
    /// How close (screen distance) a click must be to a point or edge.
    pub const COMPONENT_PICK_RADIUS: f32 = 8.0;
}

/// Switches to `theme` if it is not the one in use (cheap to call every frame).
pub fn follow(ctx: &egui::Context, theme: ColorTheme) {
    if theme != current() {
        apply(ctx, theme);
    }
}

/// Installs `theme`: the palette the panels read and egui's own style.
pub fn apply(ctx: &egui::Context, theme: ColorTheme) {
    CURRENT.store(theme.to_u8(), Ordering::Relaxed);
    let p = theme.palette();
    let egui_theme = match theme {
        ColorTheme::Dark | ColorTheme::Darker => egui::Theme::Dark,
        ColorTheme::Light => egui::Theme::Light,
    };
    ctx.set_theme(egui_theme);
    ctx.style_mut_of(egui_theme, |style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(6.0, 4.0);
        s.button_padding = Vec2::new(6.0, 2.0);
        s.interact_size = Vec2::new(36.0, 20.0);
        s.menu_margin = Margin::symmetric(4, 4);
        s.window_margin = Margin::same(8);
        s.indent = 14.0;
        s.combo_width = 90.0;
        s.icon_width = 13.0;
        s.icon_width_inner = 7.0;
        s.menu_width = 210.0;

        let v = &mut style.visuals;
        v.dark_mode = egui_theme == egui::Theme::Dark;
        v.override_text_color = None;
        v.panel_fill = p.bg_panel;
        v.window_fill = p.bg_panel;
        v.extreme_bg_color = p.bg_well;
        v.text_edit_bg_color = Some(p.bg_well);
        v.faint_bg_color = p.bg_faint;
        v.code_bg_color = p.bg_header;
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.highlight;
        v.error_fg_color = p.record;
        v.window_stroke = Stroke::new(1.0, p.border);
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(3);
        v.window_shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: p.shadow,
        };
        v.popup_shadow = Shadow {
            offset: [0, 3],
            blur: 8,
            spread: 0,
            color: p.shadow,
        };
        v.selection.bg_fill = p.accent_bg;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.striped = false;
        v.indent_has_left_vline = false;
        v.collapsing_header_frame = false;
        v.slider_trailing_fill = true;
        v.button_frame = true;

        let radius = CornerRadius::same(2);
        let widget = |bg: Color32, stroke: Stroke, fg: Color32| WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: stroke,
            corner_radius: radius,
            fg_stroke: Stroke::new(1.0, fg),
            expansion: 0.0,
        };
        // `noninteractive.bg_stroke` also draws panel separators: keep it a groove.
        v.widgets.noninteractive = widget(p.bg_panel, Stroke::new(1.0, p.border), p.text);
        v.widgets.inactive = widget(p.bg_raised, Stroke::NONE, p.text);
        v.widgets.hovered = widget(p.bg_hover, Stroke::new(1.0, p.separator), p.text_strong);
        v.widgets.active = widget(p.bg_active, Stroke::new(1.0, p.accent), p.text_strong);
        v.widgets.open = widget(p.bg_raised, Stroke::new(1.0, p.separator), p.text_strong);
    });
}

/// Frame for header strips (menu bar, manager headers).
pub fn header_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(palette().bg_header)
        .inner_margin(Margin::symmetric(6, 0))
}

/// Frame for panel bodies.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(palette().bg_panel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_theme_keeps_its_text_readable() {
        for theme in ColorTheme::ALL {
            let p = theme.palette();
            let backgrounds = [p.bg_well, p.bg_header, p.bg_panel, p.bg_raised, p.bg_hover, p.accent_bg];
            for (i, background) in backgrounds.into_iter().enumerate() {
                for text in [p.text, p.text_strong] {
                    let ratio = contrast(text, background);
                    assert!(ratio >= 4.5, "{theme:?}: text on background {i}: {ratio:.2}");
                }
            }
            // Secondary text, the accent and the highlight: at least 3 : 1 (WCAG's
            // large text and user interface components).
            for (name, colour) in [
                ("dim text", p.text_dim),
                ("accent", p.accent),
                ("highlight", p.highlight),
            ] {
                for background in [p.bg_header, p.bg_panel] {
                    let ratio = contrast(colour, background);
                    assert!(ratio >= 3.0, "{theme:?}: {name}: {ratio:.2}");
                }
            }
            for (i, icon) in [
                p.icon_primitive,
                p.icon_null,
                p.icon_camera,
                p.icon_light,
                p.icon_generator,
                p.icon_spline,
                p.icon_deformer,
                p.icon_effector,
                p.tag,
            ]
            .into_iter()
            .enumerate()
            {
                let ratio = contrast(icon, p.bg_panel);
                assert!(ratio >= 3.0, "{theme:?}: icon {i}: {ratio:.2}");
            }
            assert_eq!(p.axis, [p.axis_x, p.axis_y, p.axis_z]);
        }
    }

    #[test]
    fn the_view_keeps_a_dark_palette_and_switching_installs_the_style() {
        let ctx = egui::Context::default();
        apply(&ctx, ColorTheme::Light);
        assert_eq!(palette(), &Palette::LIGHT);
        assert_eq!(view_palette(), &Palette::DARK);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        let light = ctx.style_of(egui::Theme::Light);
        assert_eq!(light.visuals.panel_fill, Palette::LIGHT.bg_panel);
        assert!(!light.visuals.dark_mode);
        follow(&ctx, ColorTheme::Dark);
        assert_eq!((palette(), ctx.theme()), (&Palette::DARK, egui::Theme::Dark));
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Palette::DARK.bg_panel
        );
    }
}
