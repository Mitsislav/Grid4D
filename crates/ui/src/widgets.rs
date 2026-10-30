//! Small reusable widgets shared by the panels.

use egui::{Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2, pos2, vec2};

use crate::{
    icons::{self, Icon},
    theme::{metrics, palette as p},
};

/// A length field: `value` is in centimetres, shown and typed in the
/// preferred unit (a typed unit symbol converts). Chain `speed`, `range` and
/// `max_decimals` as for any `DragValue`, in centimetres.
pub(crate) fn length<'a>(ui: &Ui, value: &'a mut f32) -> egui::DragValue<'a> {
    length_per(ui, value, "")
}

/// [`length`] of a rate: `per` follows the unit (`"/s"`, `"/s²"`).
pub(crate) fn length_per<'a>(ui: &Ui, value: &'a mut f32, per: &'static str) -> egui::DragValue<'a> {
    let units = crate::preferences::units(ui.ctx());
    egui::DragValue::new(value)
        .custom_formatter(move |cm, decimals| units.format(cm, *decimals.end(), per))
        .custom_parser(move |text| units.parse(text, per))
}

/// The snapping settings as menu entries (the snap button's menu, Tools ▸ Snap).
pub(crate) fn snap_menu(ui: &mut Ui, snap: &mut crate::SnapSettings) {
    ui.checkbox(&mut snap.enabled, "Enable Snapping  (Shift+S)");
    ui.separator();
    for kind in crate::SnapType::ALL {
        ui.radio_value(&mut snap.kind, kind, kind.label());
    }
    ui.separator();
    for kind in grid4d_render::SnapKind::ALL {
        ui.checkbox(snap.targets.flag_mut(kind), kind.label());
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Snap Radius");
        ui.add(
            egui::DragValue::new(&mut snap.radius)
                .speed(0.2)
                .range(0.1..=100.0)
                .suffix(" px"),
        );
    });
}

/// Visual options for [`icon_button`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct IconStyle {
    pub selected: bool,
    pub enabled: bool,
    pub tint: Option<Color32>,
    pub size: f32,
}

impl Default for IconStyle {
    fn default() -> Self {
        Self {
            selected: false,
            enabled: true,
            tint: None,
            size: metrics::ICON_BUTTON,
        }
    }
}

/// A square, frameless toolbar button showing a painted icon.
pub(crate) fn icon_button(ui: &mut Ui, icon: Icon, style: IconStyle, tooltip: &'static str) -> Response {
    icon_button_bare(ui, icon, style).on_hover_text(tooltip)
}

/// [`icon_button`] without a tooltip, for callers whose tooltip is built on
/// demand (`response.on_hover_ui(…)` only runs while hovered).
pub(crate) fn icon_button_bare(ui: &mut Ui, icon: Icon, style: IconStyle) -> Response {
    let sense = if style.enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(style.size), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = style.enabled && response.hovered();
        let pressed = style.enabled && response.is_pointer_button_down_on();
        let bg = if pressed {
            Some(p::BG_ACTIVE)
        } else if style.selected {
            Some(p::ACCENT_BG)
        } else if hovered {
            Some(p::BG_HOVER)
        } else {
            None
        };
        let frame = rect.shrink(1.0);
        if let Some(bg) = bg {
            painter.rect_filled(frame, CornerRadius::same(3), bg);
        }
        if style.selected {
            painter.rect_stroke(
                frame,
                CornerRadius::same(3),
                Stroke::new(1.0, p::ACCENT.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
        let color = if !style.enabled {
            p::TEXT_FAINT
        } else if let Some(tint) = style.tint {
            tint
        } else if style.selected || hovered {
            p::TEXT_STRONG
        } else {
            p::TEXT
        };
        icons::paint(painter, rect.shrink(style.size * 0.21), icon, color);
    }
    response
}

/// A header row across the panel (the F-Curve editor's key fields sit in it).
/// Returns its rect.
pub(crate) fn header_row(ui: &mut Ui) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), metrics::HEADER_HEIGHT), Sense::hover());
    ui.painter_at(rect).rect_filled(rect, CornerRadius::ZERO, p::BG_HEADER);
    rect
}

/// Attribute label with Cinema 4D's dotted leader: `Name . . . . . .`
pub(crate) fn attr_label(ui: &mut Ui, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, metrics::ROW_HEIGHT - 2.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let font = TextStyle::Body.resolve(ui.style());
    let galley = painter.layout_no_wrap(text.to_owned(), font, p::TEXT);
    let text_pos = pos2(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5);
    let mut x = text_pos.x + galley.size().x + 5.0;
    painter.galley(text_pos, galley, p::TEXT);
    let y = rect.center().y + 4.0;
    while x < rect.right() - 6.0 {
        painter.circle_filled(pos2(x, y), 0.75, p::TEXT_FAINT);
        x += 4.0;
    }
}

/// Section heading inside a manager ("Coordinates", "Object Properties"...).
pub(crate) fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p::BG_HEADER.gamma_multiply(0.8));
    painter.text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        p::TEXT_STRONG,
    );
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, PointerButton, RawInput, Shape};

    use super::*;
    use crate::preferences::{self, Units};

    /// Runs one frame of a single length field; returns its rect and the texts drawn.
    fn field(ctx: &egui::Context, value: &mut f32, events: Vec<Event>) -> (Rect, Vec<String>) {
        let mut rect = Rect::NOTHING;
        let mut output = ctx.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| rect = ui.add(length(ui, value).speed(1.0).max_decimals(2)).rect,
        );
        output.textures_delta.clear();
        let texts = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        (rect, texts)
    }

    #[test]
    fn a_length_field_shows_and_takes_the_preferred_unit() {
        let ctx = egui::Context::default();
        let mut value = 150.0;
        assert!(field(&ctx, &mut value, Vec::new()).1.contains(&"150 cm".to_owned()));
        preferences::set_units(&ctx, Units::Metres);
        let (rect, texts) = field(&ctx, &mut value, Vec::new());
        assert!(texts.contains(&"1.5 m".to_owned()), "{texts:?}");

        // A click edits the text; a typed length with its own unit converts.
        let pos = rect.center();
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        field(&ctx, &mut value, vec![Event::PointerMoved(pos)]);
        field(&ctx, &mut value, vec![button(true)]);
        field(&ctx, &mut value, vec![button(false)]);
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        field(
            &ctx,
            &mut value,
            vec![
                key(Key::A, Modifiers::COMMAND),
                Event::Text("25 mm".to_owned()),
                key(Key::Enter, Modifiers::NONE),
            ],
        );
        assert!((value - 2.5).abs() < 1e-6, "25 mm is 2.5 cm, got {value}");
        let (_, texts) = field(&ctx, &mut value, Vec::new());
        assert!(texts.contains(&"0.025 m".to_owned()), "{texts:?}");
    }
}
