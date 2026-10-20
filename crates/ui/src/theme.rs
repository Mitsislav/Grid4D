//! The Grid4D look: a dense, dark "industrial" theme in the spirit of
//! Cinema 4D's default UI — neutral greys, crisp 1 px dark separators, small
//! corner radii, a steel-blue accent for selection and orange for activity.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2, style::WidgetVisuals,
};

/// The colour palette. Every UI colour comes from here — no ad-hoc literals in panels.
pub mod palette {
    use egui::Color32;

    /// Text fields, timeline track, deepest wells.
    pub const BG_WELL: Color32 = Color32::from_rgb(0x18, 0x19, 0x1b);
    /// Menu bar, manager headers, tab strips.
    pub const BG_HEADER: Color32 = Color32::from_rgb(0x1f, 0x20, 0x23);
    /// Manager bodies and toolbars.
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x2e);
    /// Buttons and other raised controls.
    pub const BG_RAISED: Color32 = Color32::from_rgb(0x35, 0x37, 0x3b);
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x3f, 0x42, 0x47);
    pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x4a, 0x4e, 0x55);
    /// The 1 px grooves between panels.
    pub const BORDER: Color32 = Color32::from_rgb(0x10, 0x11, 0x12);
    pub const SEPARATOR: Color32 = Color32::from_rgb(0x3b, 0x3d, 0x42);
    /// Darkens everything behind an overlay (file drop target).
    pub const SCRIM: Color32 = Color32::from_black_alpha(170);

    pub const TEXT: Color32 = Color32::from_rgb(0xc6, 0xc8, 0xcc);
    pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xee, 0xf0, 0xf2);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x82, 0x86, 0x8d);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x5a, 0x5d, 0x63);

    /// Selection, focus, playhead.
    pub const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x8f, 0xf2);
    /// Selected rows / toggled buttons background.
    pub const ACCENT_BG: Color32 = Color32::from_rgb(0x2b, 0x45, 0x6e);
    /// Active object, hovered gizmo handles, keyframes.
    pub const HIGHLIGHT: Color32 = Color32::from_rgb(0xf5, 0x8f, 0x2a);
    pub const RECORD: Color32 = Color32::from_rgb(0xe0, 0x3e, 0x3e);
    /// Key buttons: a temporary value that differs from its curve.
    pub const KEY_CHANGED: Color32 = Color32::from_rgb(0xe8, 0xc5, 0x47);
    /// Key marks on the timeline.
    pub const KEY: Color32 = Color32::from_rgb(0xb4, 0xb8, 0xbf);

    pub const AXIS_X: Color32 = Color32::from_rgb(0xe8, 0x4f, 0x4f);
    pub const AXIS_Y: Color32 = Color32::from_rgb(0x7f, 0xc8, 0x45);
    pub const AXIS_Z: Color32 = Color32::from_rgb(0x4c, 0x8e, 0xf2);

    /// Object Manager traffic lights.
    pub const VIS_DEFAULT: Color32 = Color32::from_rgb(0x6e, 0x71, 0x77);
    pub const VIS_HIDDEN: Color32 = Color32::from_rgb(0xd9, 0x4a, 0x45);
    pub const ENABLED: Color32 = Color32::from_rgb(0x6c, 0xc0, 0x5a);

    /// Viewport background gradient.
    pub const VIEWPORT_TOP: Color32 = Color32::from_rgb(0x4d, 0x50, 0x56);
    pub const VIEWPORT_BOTTOM: Color32 = Color32::from_rgb(0x2b, 0x2d, 0x31);

    /// Per-object-type icon tints (Object Manager, menus, toolbar).
    pub const ICON_PRIMITIVE: Color32 = Color32::from_rgb(0x6f, 0xa8, 0xf0);
    pub const ICON_NULL: Color32 = Color32::from_rgb(0xb8, 0xbb, 0xc0);
    pub const ICON_CAMERA: Color32 = Color32::from_rgb(0xa9, 0x8b, 0xe8);
    pub const ICON_LIGHT: Color32 = Color32::from_rgb(0xf2, 0xc9, 0x4c);
    /// Generators (Subdivision Surface …).
    pub const ICON_GENERATOR: Color32 = Color32::from_rgb(0x6c, 0xc2, 0x8a);
    /// Tag icons in the Object Manager.
    pub const TAG: Color32 = Color32::from_rgb(0xc8, 0xcd, 0xd6);
    /// The snap target a drag is stuck to, and its label.
    pub const SNAP: Color32 = Color32::from_rgb(0xff, 0xd8, 0x3d);

    pub const AXIS: [Color32; 3] = [AXIS_X, AXIS_Y, AXIS_Z];
}

use palette as p;

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

/// Installs the theme. Call once at start-up.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
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
        v.dark_mode = true;
        v.panel_fill = p::BG_PANEL;
        v.window_fill = p::BG_PANEL;
        v.extreme_bg_color = p::BG_WELL;
        v.text_edit_bg_color = Some(p::BG_WELL);
        v.faint_bg_color = Color32::from_rgb(0x2f, 0x30, 0x34);
        v.code_bg_color = p::BG_HEADER;
        v.hyperlink_color = p::ACCENT;
        v.warn_fg_color = p::HIGHLIGHT;
        v.error_fg_color = p::RECORD;
        v.window_stroke = Stroke::new(1.0, p::BORDER);
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(3);
        v.window_shadow = Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        v.popup_shadow = Shadow {
            offset: [0, 3],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(110),
        };
        v.selection.bg_fill = p::ACCENT_BG;
        v.selection.stroke = Stroke::new(1.0, p::ACCENT);
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
        // `noninteractive.bg_stroke` also draws panel separators: keep it a dark groove.
        v.widgets.noninteractive = widget(p::BG_PANEL, Stroke::new(1.0, p::BORDER), p::TEXT);
        v.widgets.inactive = widget(p::BG_RAISED, Stroke::NONE, p::TEXT);
        v.widgets.hovered = widget(p::BG_HOVER, Stroke::new(1.0, p::SEPARATOR), p::TEXT_STRONG);
        v.widgets.active = widget(p::BG_ACTIVE, Stroke::new(1.0, p::ACCENT), p::TEXT_STRONG);
        v.widgets.open = widget(p::BG_RAISED, Stroke::new(1.0, p::SEPARATOR), p::TEXT_STRONG);
    });
}

/// Frame for header strips (menu bar, manager headers).
pub fn header_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(p::BG_HEADER)
        .inner_margin(Margin::symmetric(6, 0))
}

/// Frame for panel bodies.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(p::BG_PANEL)
}
