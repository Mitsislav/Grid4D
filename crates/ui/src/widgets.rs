//! Small reusable widgets shared by the panels.

use egui::{Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2, pos2, vec2};

use crate::i18n::tr;
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
    ui.checkbox(&mut snap.enabled, tr("Enable Snapping  (Shift+S)"));
    ui.separator();
    for kind in crate::SnapType::ALL {
        ui.radio_value(&mut snap.kind, kind, tr(kind.label()));
    }
    ui.separator();
    for kind in grid4d_render::SnapKind::ALL {
        ui.checkbox(snap.targets.flag_mut(kind), tr(kind.label()));
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Snap Radius"));
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
    icon_button_bare(ui, icon, style).on_hover_text(tr(tooltip))
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
            Some(p().bg_active)
        } else if style.selected {
            Some(p().accent_bg)
        } else if hovered {
            Some(p().bg_hover)
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
                Stroke::new(1.0, p().accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
        let color = if !style.enabled {
            p().text_faint
        } else if let Some(tint) = style.tint {
            tint
        } else if style.selected || hovered {
            p().text_strong
        } else {
            p().text
        };
        icons::paint(painter, rect.shrink(style.size * 0.21), icon, color);
    }
    response
}

/// A header row across the panel (the F-Curve editor's key fields sit in it).
/// Returns its rect.
pub(crate) fn header_row(ui: &mut Ui) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), metrics::HEADER_HEIGHT), Sense::hover());
    ui.painter_at(rect).rect_filled(rect, CornerRadius::ZERO, p().bg_header);
    rect
}

/// Attribute label with Cinema 4D's dotted leader: `Name . . . . . .`
pub(crate) fn attr_label(ui: &mut Ui, text: &str, width: f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, metrics::ROW_HEIGHT - 2.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Clipped to its column (a long translation must not run into the field);
    // the whole text shows on hover then.
    let painter = ui.painter_at(rect.shrink2(vec2(0.0, -2.0)));
    let font = TextStyle::Body.resolve(ui.style());
    let text = tr(text);
    let galley = painter.layout_no_wrap(text.to_owned(), font, p().text);
    let text_pos = pos2(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5);
    let mut x = text_pos.x + galley.size().x + 5.0;
    if text_pos.x + galley.size().x > rect.right() - 4.0 {
        response.on_hover_text(text);
    }
    painter.galley(text_pos, galley, p().text);
    let y = rect.center().y + 4.0;
    while x < rect.right() - 6.0 {
        painter.circle_filled(pos2(x, y), 0.75, p().text_faint);
        x += 4.0;
    }
}

/// Section heading inside a manager ("Coordinates", "Object Properties"...).
pub(crate) fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p().bg_header.gamma_multiply(0.8));
    painter.text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        tr(text),
        FontId::proportional(12.0),
        p().text_strong,
    );
    ui.add_space(2.0);
}

/// A colour swatch with its picker. The picker converts the colour on every
/// frame (0.9 comes back as 0.90000004), so the value is written back only when
/// the user changes it: showing a colour must not edit the document.
pub fn colour_button(ui: &mut Ui, colour: &mut [f32; 3]) -> Response {
    let mut shown = *colour;
    let response = ui.color_edit_button_rgb(&mut shown);
    if response.changed() {
        *colour = shown;
    }
    response
}

/// The field of a plugin's parameter, `width` wide, as its description says:
/// a length in the user's unit, an angle in degrees, a fraction in percent,
/// a whole number, a check box, a choice, a vector or a colour (within its
/// range). `value` changes only when the user changes it.
pub fn plugin_value(
    ui: &mut Ui,
    spec: &grid4d_plugin::ParamSpec,
    value: &mut grid4d_scene::PluginValue,
    width: f32,
) -> Response {
    use egui::DragValue;
    use grid4d_plugin::{ParamKind, Unit};
    use grid4d_scene::PluginValue;
    match (spec.kind, value) {
        (ParamKind::Float { min, max, unit }, PluginValue::Float(v)) => match unit {
            Unit::Length => ui.add_sized([width, 19.0], length(ui, v).speed(1.0).range(min..=max).max_decimals(2)),
            Unit::Angle => {
                let mut degrees = v.to_degrees();
                let response = ui.add_sized(
                    [width, 19.0],
                    DragValue::new(&mut degrees)
                        .speed(0.5)
                        .range(min.to_degrees()..=max.to_degrees())
                        .suffix("°"),
                );
                if response.changed() {
                    *v = degrees.to_radians();
                }
                response
            }
            Unit::Percent => {
                let mut percent = *v * 100.0;
                let response = ui.add_sized(
                    [width, 19.0],
                    DragValue::new(&mut percent)
                        .speed(0.5)
                        .range(min * 100.0..=max * 100.0)
                        .suffix(" %"),
                );
                if response.changed() {
                    *v = percent / 100.0;
                }
                response
            }
            Unit::None => ui.add_sized(
                [width, 19.0],
                DragValue::new(v).speed(0.01).range(min..=max).max_decimals(3),
            ),
        },
        (ParamKind::Int { min, max }, PluginValue::Int(v)) => {
            ui.add_sized([width, 19.0], DragValue::new(v).speed(0.1).range(min..=max))
        }
        (ParamKind::Bool, PluginValue::Bool(v)) => ui.checkbox(v, ""),
        (ParamKind::Choice(choices), PluginValue::Choice(v)) => {
            let shown = choices.get(*v as usize).copied().unwrap_or_default();
            egui::ComboBox::from_id_salt(("grid4d.attr.plugin", spec.name))
                .selected_text(tr(shown))
                .show_ui(ui, |ui| {
                    for (index, choice) in (0u32..).zip(choices.iter()) {
                        ui.selectable_value(v, index, tr(choice));
                    }
                })
                .response
        }
        (ParamKind::Vector { unit }, PluginValue::Vector(v)) => {
            ui.horizontal(|ui| {
                for axis in 0..3 {
                    let field = if unit == Unit::Length {
                        length(ui, &mut v[axis])
                    } else {
                        DragValue::new(&mut v[axis])
                    };
                    ui.add_sized([(width * 0.6).max(40.0), 19.0], field.speed(0.5).max_decimals(2));
                }
            })
            .response
        }
        (ParamKind::Color, PluginValue::Color(c)) => colour_button(ui, c),
        // `Values::get` gives each parameter its own type.
        _ => ui.label(""),
    }
}

/// The UV tools on the active polygon object (Mesh ▸ UV and the UV
/// Editor's UV menu).
pub(crate) fn uv_menu(ui: &mut Ui, editor: &mut crate::EditorState, enabled: bool) {
    use crate::uv_tools::{PROJECTIONS, UvOp};
    use grid4d_geom::uv::FlipAxis;

    let mut item = |ui: &mut Ui, op: UvOp, label: &str| {
        if ui
            .add_enabled(enabled, egui::Button::new(tr(label)))
            .on_disabled_hover_text(tr("Select a polygon object"))
            .clicked()
        {
            editor.commands.push(crate::EditorCommand::UvCommand(op));
            ui.close();
        }
    };
    item(ui, UvOp::Unwrap, crate::i18n::key("Unwrap"));
    item(ui, UvOp::Relax, crate::i18n::key("Relax"));
    item(ui, UvOp::Pack, crate::i18n::key("Pack Islands"));
    ui.menu_button(tr("Project"), |ui| {
        for (projection, label) in PROJECTIONS {
            item(ui, UvOp::Project(projection), label);
        }
    });
    ui.separator();
    item(ui, UvOp::Flip(FlipAxis::U), crate::i18n::key("Flip U"));
    item(ui, UvOp::Flip(FlipAxis::V), crate::i18n::key("Flip V"));
    ui.separator();
    item(ui, UvOp::MarkSeams, crate::i18n::key("Mark Seams"));
    item(ui, UvOp::ClearSeams, crate::i18n::key("Clear Seams"));
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
