//! Render ▸ Edit Render Settings… (Ctrl+B): the document's render settings in a
//! window that stays open beside the workspace — pages on the left, the page's
//! settings on the right (see `docs/reference/render-settings.md`).
//!
//! The fields write `Document::render` live; each interaction is one undo step
//! through [`LiveEdit`].

use egui::{Button, ComboBox, DragValue, Frame, Grid, Id, RichText, TextEdit, Ui, Window};
use grid4d_scene::{
    Document, FrameNaming, FrameRange, ImageFormat, Pass, PngPasses, RenderSettings, SceneEdit, TimeState,
};

use crate::i18n::{key, tr, trf};
use crate::{
    EditorCommand, EditorState,
    live_edit::{LiveEdit, interaction_ongoing},
    theme::palette as p,
    widgets::{attr_label, section_heading},
};

const LABEL_WIDTH: f32 = 112.0;
const FIELD_WIDTH: f32 = 96.0;
const PAGE_LIST_WIDTH: f32 = 96.0;
/// The window's size (its height is a guess for placing it the first time).
const WINDOW_SIZE: egui::Vec2 = egui::vec2(460.0, 380.0);
/// The height of the tallest page (Output).
const PAGE_HEIGHT: f32 = 310.0;

/// Common picture sizes (the Preset list).
const PRESETS: [(&str, u32, u32); 8] = [
    (key("Preview 640 × 360"), 640, 360),
    (key("VGA 640 × 480"), 640, 480),
    (key("HD 720 (1280 × 720)"), 1280, 720),
    (key("HD 1080 (1920 × 1080)"), 1920, 1080),
    ("2K (2560 × 1440)", 2560, 1440),
    (key("4K UHD (3840 × 2160)"), 3840, 2160),
    (key("Square 1080 × 1080"), 1080, 1080),
    (key("Portrait 1080 × 1920"), 1080, 1920),
];

/// The window's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Output,
    Save,
    Sampling,
    Passes,
    Options,
}

impl Page {
    pub const ALL: [Self; 5] = [Self::Output, Self::Save, Self::Sampling, Self::Passes, Self::Options];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Output => "Output",
            Self::Save => "Save",
            Self::Sampling => "Sampling",
            Self::Passes => "Passes",
            Self::Options => "Options",
        }
    }
}

#[derive(Debug)]
pub struct RenderSettingsPanel {
    edit: LiveEdit<(), RenderSettings>,
    pub page: Page,
    /// Width and height from before the current interaction: Lock Ratio keeps
    /// their proportions (so dragging a side never drifts by rounding).
    ratio: (u32, u32),
    /// Shown on the previous frame: closing it records an unfinished edit.
    was_open: bool,
}

impl Default for RenderSettingsPanel {
    fn default() -> Self {
        let defaults = RenderSettings::default();
        Self {
            edit: LiveEdit::default(),
            page: Page::default(),
            ratio: (defaults.width, defaults.height),
            was_open: false,
        }
    }
}

impl RenderSettingsPanel {
    pub fn show(&mut self, ctx: &egui::Context, doc: &mut Document, editor: &mut EditorState) {
        let mut open = editor.show_render_settings;
        if !open {
            if std::mem::take(&mut self.was_open) {
                // Closed mid-interaction (a focused field): record it now.
                self.commit(doc, editor, false);
            }
            return;
        }
        self.was_open = true;
        self.edit.begin((), &doc.render);
        let mut ongoing = false;
        Window::new(tr("Render Settings"))
            .id(Id::unique("grid4d.render_settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(WINDOW_SIZE.x)
            // Centred the first time; then it stays where the user puts it.
            .default_pos(ctx.content_rect().center() - WINDOW_SIZE * 0.5)
            .show(ctx, |ui| {
                ongoing = interaction_ongoing(ui);
                if !ongoing {
                    self.ratio = (doc.render.width, doc.render.height);
                }
                ui.horizontal_top(|ui| {
                    self.page_list(ui);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        // Every page as tall as the tallest: the window keeps its size.
                        ui.set_min_size(egui::vec2(330.0, PAGE_HEIGHT));
                        let Document {
                            render, time, takes, ..
                        } = doc;
                        match self.page {
                            Page::Output => {
                                take_field(ui, takes, &mut editor.commands);
                                output_page(ui, render, time, self.ratio);
                            }
                            Page::Save => save_page(ui, render, time, editor),
                            Page::Sampling => sampling_page(ui, render),
                            Page::Passes => passes_page(ui, render),
                            Page::Options => options_page(ui, render),
                        }
                    });
                });
            });
        editor.show_render_settings = open;
        self.commit(doc, editor, ongoing && open);
    }

    /// Records the end of an interaction that changed the settings.
    fn commit(&mut self, doc: &Document, editor: &mut EditorState, ongoing: bool) {
        if let Some(((), before)) = self.edit.end((), &doc.render, ongoing) {
            editor.commands.push(EditorCommand::CommitLive {
                inverse: SceneEdit::SetRender(before),
                label: "Render Settings",
            });
        }
    }

    fn page_list(&mut self, ui: &mut Ui) {
        // A well of its own height (a separator would stretch the window).
        Frame::new()
            .fill(p().bg_well)
            .inner_margin(4)
            .corner_radius(3)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(PAGE_LIST_WIDTH);
                    ui.label(RichText::new(tr("Path Tracer")).size(11.0).color(p().text_dim));
                    ui.add_space(2.0);
                    for page in Page::ALL {
                        let button = Button::selectable(self.page == page, tr(page.label()))
                            .min_size([PAGE_LIST_WIDTH, 0.0].into());
                        if ui.add(button).clicked() {
                            self.page = page;
                        }
                    }
                });
            });
    }
}

/// Render Settings ▸ Take: the take renders show (Current: the active one).
fn take_field(ui: &mut Ui, takes: &grid4d_scene::Takes, commands: &mut crate::CommandQueue) {
    if takes.is_empty() {
        return;
    }
    section_heading(ui, "Take");
    Grid::new("grid4d.render.take")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Take", LABEL_WIDTH);
            let name = |id: Option<grid4d_scene::TakeId>| {
                id.and_then(|t| takes.get(t))
                    .map_or_else(|| "Current".to_owned(), |t| t.name.clone())
            };
            let mut chosen = takes.render_take.filter(|&t| takes.contains(t));
            ComboBox::from_id_salt("grid4d.render.take")
                .selected_text(name(chosen))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, None, tr("Current"));
                    for (id, take) in takes.iter() {
                        ui.selectable_value(&mut chosen, Some(id), &take.name);
                    }
                })
                .response
                .on_hover_text(tr("The take renders show; Current: the active one"));
            if chosen != takes.render_take {
                let mut changed = takes.clone();
                changed.render_take = chosen;
                commands.push(EditorCommand::Edit {
                    edit: SceneEdit::SetTakes(changed),
                    label: "Render Take",
                });
            }
            ui.end_row();
        });
}

fn output_page(ui: &mut Ui, s: &mut RenderSettings, time: &TimeState, ratio: (u32, u32)) {
    section_heading(ui, "Output");
    Grid::new("grid4d.render.output")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Preset", LABEL_WIDTH);
            let current = PRESETS
                .iter()
                .find(|&&(_, w, h)| (w, h) == (s.width, s.height))
                .map_or(key("Custom"), |&(name, _, _)| name);
            ComboBox::from_id_salt("grid4d.render.preset")
                .selected_text(tr(current))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (name, w, h) in PRESETS {
                        if ui.selectable_label(current == name, tr(name)).clicked() {
                            (s.width, s.height) = (w, h);
                        }
                    }
                });
            ui.end_row();

            let size = 1..=RenderSettings::MAX_SIZE;
            attr_label(ui, "Width", LABEL_WIDTH);
            let width = ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut s.width).range(size.clone()).suffix(" px"),
            );
            ui.end_row();
            attr_label(ui, "Height", LABEL_WIDTH);
            let height = ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut s.height).range(size).suffix(" px"),
            );
            ui.end_row();
            if s.lock_ratio {
                let (w0, h0) = ratio;
                if width.changed() {
                    s.height = scaled(s.width, h0, w0);
                } else if height.changed() {
                    s.width = scaled(s.height, w0, h0);
                }
            }
            attr_label(ui, "Lock Ratio", LABEL_WIDTH);
            ui.checkbox(&mut s.lock_ratio, "");
            ui.end_row();
            attr_label(ui, "Aspect", LABEL_WIDTH);
            ui.label(RichText::new(aspect_text(s.width, s.height)).color(p().text_dim));
            ui.end_row();
            // The render region, drawn in the view (Render ▸ Render Region).
            attr_label(ui, "Render Region", LABEL_WIDTH);
            let text = s.region.map_or_else(
                || tr("None (Render › Render Region draws one)").to_owned(),
                |region| {
                    let [x0, y0, x1, y1] = region.pixels(s.width, s.height);
                    trf("{0} × {1} px", &[&(x1 - x0), &(y1 - y0)])
                },
            );
            ui.label(RichText::new(text).color(p().text_dim));
            ui.end_row();
            if s.region.is_some() {
                check(
                    ui,
                    key("Use for Final Render"),
                    &mut s.region_final,
                    key("Render to Picture Viewer renders only the region too (the rest transparent)"),
                );
            }
        });

    section_heading(ui, "Frames");
    Grid::new("grid4d.render.frames")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Frame Range", LABEL_WIDTH);
            let span = s.frame_span(time);
            ComboBox::from_id_salt("grid4d.render.range")
                .selected_text(tr(s.frames.label()))
                .width(FIELD_WIDTH + 30.0)
                .show_ui(ui, |ui| {
                    let manual = FrameRange::Manual {
                        from: *span.start(),
                        to: *span.end(),
                    };
                    for choice in [FrameRange::Current, FrameRange::All, manual] {
                        let selected = std::mem::discriminant(&s.frames) == std::mem::discriminant(&choice);
                        if ui.selectable_label(selected, tr(choice.label())).clicked() && !selected {
                            s.frames = choice;
                        }
                    }
                });
            ui.end_row();

            // From and To show the range's frames; typing one makes it Manual.
            let (mut from, mut to) = (*span.start(), *span.end());
            attr_label(ui, "From", LABEL_WIDTH);
            let changed_from = ui
                .add_sized([FIELD_WIDTH, 19.0], DragValue::new(&mut from).suffix(" F"))
                .changed();
            ui.end_row();
            attr_label(ui, "To", LABEL_WIDTH);
            let changed_to = ui
                .add_sized([FIELD_WIDTH, 19.0], DragValue::new(&mut to).suffix(" F"))
                .changed();
            ui.end_row();
            if changed_from || changed_to {
                s.frames = FrameRange::Manual {
                    from,
                    to: if changed_from { to.max(from) } else { to },
                };
                if let FrameRange::Manual { from, to } = &mut s.frames {
                    *from = (*from).min(*to);
                }
            }

            attr_label(ui, "Frame Step", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut s.frame_step).range(1..=RenderSettings::MAX_FRAME_STEP),
            );
            ui.end_row();
            attr_label(ui, "Frames", LABEL_WIDTH);
            let count = s.frame_count(time);
            let span = s.frame_span(time);
            ui.label(
                RichText::new(trf("{0} (from {1} to {2})", &[&count, &span.start(), &span.end()])).color(p().text_dim),
            );
            ui.end_row();
        });
    ui.label(
        RichText::new(tr(
            "Render to Picture Viewer renders every frame of the range; the Render View the current one.",
        ))
        .size(11.0)
        .color(p().text_faint),
    );
}

fn save_page(ui: &mut Ui, s: &mut RenderSettings, time: &TimeState, editor: &mut EditorState) {
    section_heading(ui, "Save");
    Grid::new("grid4d.render.save")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Save", LABEL_WIDTH);
            ui.checkbox(&mut s.save, "");
            ui.end_row();

            attr_label(ui, "File", LABEL_WIDTH);
            ui.add_enabled_ui(s.save, |ui| {
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut s.output).desired_width(190.0));
                    if ui.button(tr("…")).on_hover_text(tr("Choose where to save")).clicked() {
                        editor.commands.push(EditorCommand::BrowseRenderOutput);
                    }
                });
            });
            ui.end_row();

            attr_label(ui, "Format", LABEL_WIDTH);
            ComboBox::from_id_salt("grid4d.render.format")
                .selected_text(tr(s.format.label()))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for format in ImageFormat::ALL {
                        ui.selectable_value(&mut s.format, format, tr(format.label()));
                    }
                })
                .response
                .on_hover_text(tr(
                    "OpenEXR keeps the light's full range, in linear colour, for compositing",
                ));
            ui.end_row();
            attr_label(ui, "Name", LABEL_WIDTH);
            ComboBox::from_id_salt("grid4d.render.naming")
                .selected_text(tr(s.naming.label()))
                .width(FIELD_WIDTH + 30.0)
                .show_ui(ui, |ui| {
                    for naming in FrameNaming::ALL {
                        ui.selectable_value(&mut s.naming, naming, tr(naming.label()));
                    }
                })
                .response
                .on_hover_text(tr("How frame numbers join the file names of an animation"));
            ui.end_row();
            attr_label(ui, "Alpha Channel", LABEL_WIDTH);
            ui.checkbox(&mut s.alpha, "")
                .on_hover_text(tr("Keep the background transparent; off: black"));
            ui.end_row();
        });
    let hint = match (s.save, s.output.trim().is_empty()) {
        (false, _) => tr("Renders are not saved.").to_owned(),
        (true, true) => tr("No file: renders are not saved.").to_owned(),
        (true, false) => match s.output_path(editor.file.path()) {
            Some(_) if s.is_animation(time) => {
                let first = s.frames(time).next().unwrap_or(0);
                let file = s.output_file(editor.file.path(), Some(first)).unwrap_or_default();
                trf("Saves {0} frames as {1} …", &[&s.frame_count(time), &file.display()])
            }
            Some(path) => trf("Saves to {0}", &[&path.display()]),
            None => tr("Relative to the document's folder: renders are saved once the document is saved.").to_owned(),
        },
    };
    ui.add(egui::Label::new(RichText::new(hint).size(11.0).color(p().text_dim)).wrap());
}

fn sampling_page(ui: &mut Ui, s: &mut RenderSettings) {
    section_heading(ui, "Sampling");
    Grid::new("grid4d.render.sampling")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            attr_label(ui, "Samples", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut s.samples).range(1..=RenderSettings::MAX_SAMPLES),
            )
            .on_hover_text(if s.adaptive {
                tr("The most a pixel gets: noisy pixels get up to this many")
            } else {
                tr("Per pixel: smoother edges and less noise, but slower")
            });
            ui.end_row();
            check(
                ui,
                key("Adaptive"),
                &mut s.adaptive,
                key("Spends samples where the picture is still noisy: clean areas stop early"),
            );
            if s.adaptive {
                attr_label(ui, "Minimum Samples", LABEL_WIDTH);
                let field = ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut s.min_samples).range(1..=RenderSettings::MAX_SAMPLES),
                    )
                    .on_hover_text(tr("Every pixel gets these before any stops"));
                ui.ctx()
                    .accesskit_node_builder(field.id, |node| node.set_label(tr("Minimum Samples")));
                ui.end_row();
                attr_label(ui, "Noise Threshold", LABEL_WIDTH);
                let mut threshold = s.noise_threshold.get();
                let field = ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut threshold)
                            .range(0.001..=0.5)
                            .speed(0.001)
                            .fixed_decimals(3),
                    )
                    .on_hover_text(tr(
                        "A pixel stops once its noise is below this: smaller is cleaner and slower",
                    ));
                ui.ctx()
                    .accesskit_node_builder(field.id, |node| node.set_label(tr("Noise Threshold")));
                if field.changed() {
                    s.noise_threshold = grid4d_scene::NoiseThreshold::from_f32(threshold);
                }
                ui.end_row();
            }
            attr_label(ui, "Denoise", LABEL_WIDTH);
            let combo = ComboBox::from_id_salt("grid4d.render.denoise")
                .selected_text(tr(s.denoise.label()))
                .show_ui(ui, |ui| {
                    for option in grid4d_scene::Denoise::ALL {
                        ui.selectable_value(&mut s.denoise, option, tr(option.label()));
                    }
                });
            let id = combo.response.id;
            combo.response.on_hover_text(tr(
                "Removes the noise left in finished renders, keeping edges and textures",
            ));
            ui.ctx()
                .accesskit_node_builder(id, |node| node.set_label(tr("Denoise")));
            ui.end_row();
            if s.denoise != grid4d_scene::Denoise::Off {
                attr_label(ui, "Denoise Strength", LABEL_WIDTH);
                let field = ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut s.denoise_strength).range(0..=100).suffix(" %"),
                    )
                    .on_hover_text(tr("How much of the denoised picture is used over the noisy one"));
                ui.ctx()
                    .accesskit_node_builder(field.id, |node| node.set_label(tr("Denoise Strength")));
                ui.end_row();
            }
            attr_label(ui, "Triangle Budget", LABEL_WIDTH);
            let field = ui
                .add_sized(
                    [FIELD_WIDTH, 19.0],
                    DragValue::new(&mut s.displacement_budget)
                        .range(1..=RenderSettings::MAX_DISPLACEMENT_BUDGET)
                        .suffix(" M"),
                )
                .on_hover_text(tr(
                    "The most triangles displacement may make, in millions: objects beyond are split less",
                ));
            ui.ctx()
                .accesskit_node_builder(field.id, |node| node.set_label(tr("Triangle Budget")));
            ui.end_row();
            attr_label(ui, "Bounces", LABEL_WIDTH);
            ui.add_sized(
                [FIELD_WIDTH, 19.0],
                DragValue::new(&mut s.bounces).range(0..=RenderSettings::MAX_BOUNCES),
            )
            .on_hover_text(tr("How often light bounces between surfaces (0: direct light only)"));
            ui.end_row();
            check(
                ui,
                key("Clamp Indirect"),
                &mut s.clamp_indirect,
                key("Caps light found after a bounce: fewer bright specks, slightly darker caustics"),
            );
            check(
                ui,
                key("Emissive Surfaces Light the Scene"),
                &mut s.emissive_lights,
                key("Surfaces with Emission are sampled as lights: far less noise where they light the scene"),
            );
            check(
                ui,
                key("Motion Blur"),
                &mut s.motion_blur,
                key("Renders over the camera's shutter: moving things smear (the camera's Shutter sets how long)"),
            );
            if s.motion_blur {
                attr_label(ui, "Segments", LABEL_WIDTH);
                let field = ui
                    .add_sized(
                        [FIELD_WIDTH, 19.0],
                        DragValue::new(&mut s.segments)
                            .range(RenderSettings::MIN_SEGMENTS..=RenderSettings::MAX_SEGMENTS),
                    )
                    .on_hover_text(tr(
                        "Moments of the shutter evaluated: motion between them is straight, so curved paths want more",
                    ));
                ui.ctx()
                    .accesskit_node_builder(field.id, |node| node.set_label(tr("Segments")));
                ui.end_row();
            }
        });
}

/// A labelled check box in a page's grid, named for screen readers.
fn check(ui: &mut Ui, label: &str, value: &mut bool, hint: &str) {
    attr_label(ui, label, LABEL_WIDTH);
    let field = ui.checkbox(value, "").on_hover_text(tr(hint));
    ui.ctx()
        .accesskit_node_builder(field.id, |node| node.set_label(tr(label)));
    ui.end_row();
}

/// The render passes saved beside the picture
/// (`docs/reference/render-passes.md`).
fn passes_page(ui: &mut Ui, s: &mut RenderSettings) {
    section_heading(ui, "Passes");
    ui.label(
        RichText::new(tr(
            "Pieces of the picture for compositing: with OpenEXR all in one file, with PNG one file each.",
        ))
        .size(11.0)
        .color(p().text_dim),
    );
    Grid::new("grid4d.render.passes")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for pass in Pass::ALL {
                let mut on = s.passes.contains(pass);
                check(ui, pass.label(), &mut on, pass_hint(pass));
                s.passes.set(pass, on);
            }
            attr_label(ui, "PNG Passes", LABEL_WIDTH);
            let combo = ComboBox::from_id_salt("grid4d.render.png_passes")
                .selected_text(tr(s.png_passes.label()))
                .show_ui(ui, |ui| {
                    for option in PngPasses::ALL {
                        ui.selectable_value(&mut s.png_passes, option, tr(option.label()));
                    }
                });
            ui.ctx()
                .accesskit_node_builder(combo.response.id, |node| node.set_label(tr("PNG Passes")));
            ui.end_row();
        });
}

/// What a pass holds (its row's hint).
const fn pass_hint(pass: Pass) -> &'static str {
    match pass {
        Pass::Alpha => key("Coverage: how much of each pixel objects fill"),
        Pass::Depth => key("Distance from the camera along its view, in cm"),
        Pass::Normal => key("Which way the surface faces, in world space"),
        Pass::Position => key("Where the surface is, in world space"),
        Pass::Albedo => key("The surface's own colour, unlit"),
        Pass::Emission => key("Light given off by glowing surfaces and visible lights"),
        Pass::Background => key("The Sky or Environment seen directly"),
        Pass::DiffuseDirect => key("Matte reflection straight from the lights"),
        Pass::DiffuseIndirect => key("Matte reflection of light that has bounced"),
        Pass::SpecularDirect => key("Glossy reflection straight from the lights"),
        Pass::SpecularIndirect => key("Glossy reflection of light that has bounced"),
        Pass::Transmission => key("Light through glass"),
        Pass::ObjectId => key("A number per object (the file names them)"),
        Pass::MaterialId => key("A number per material (the file names them)"),
        Pass::LightGroups => key("The light of each light group apart, and of lights in none"),
    }
}

fn options_page(ui: &mut Ui, s: &mut RenderSettings) {
    section_heading(ui, "Options");
    Grid::new("grid4d.render.options")
        .num_columns(2)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            check(
                ui,
                key("Auto Light"),
                &mut s.auto_light,
                key("Light a scene that has no lights"),
            );
        });
}

/// `side` × `numerator` ÷ `denominator`, rounded, kept in the picture's range.
fn scaled(side: u32, numerator: u32, denominator: u32) -> u32 {
    let denominator = u64::from(denominator.max(1));
    let value = (u64::from(side) * u64::from(numerator) + denominator / 2) / denominator;
    u32::try_from(value)
        .unwrap_or(u32::MAX)
        .clamp(1, RenderSettings::MAX_SIZE)
}

/// "16:9 · 1.778" (the reduced ratio, then width ÷ height).
fn aspect_text(width: u32, height: u32) -> String {
    let (mut a, mut b) = (width.max(1), height.max(1));
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let (w, h) = (width.max(1) / a, height.max(1) / a);
    let decimal = f64::from(width.max(1)) / f64::from(height.max(1));
    if w <= 64 && h <= 64 {
        format!("{w}:{h} · {decimal:.3}")
    } else {
        format!("{decimal:.3}")
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Pos2, RawInput};

    use super::*;
    use crate::{ViewportPanel, commands};

    #[test]
    fn lock_ratio_scales_from_the_proportions_before_the_change() {
        assert_eq!(scaled(1920, 720, 1280), 1080);
        assert_eq!(scaled(1281, 720, 1280), 721, "rounded");
        assert_eq!(scaled(0, 720, 1280), 1, "never zero");
        assert_eq!(scaled(16_000, 16_000, 1), RenderSettings::MAX_SIZE);
        assert_eq!(aspect_text(1920, 1080), "16:9 · 1.778");
        assert_eq!(aspect_text(1000, 999), "1.001");
    }

    /// One headless frame of the window (with `events`), then the commands.
    fn frame(
        ctx: &egui::Context,
        panel: &mut RenderSettingsPanel,
        doc: &mut Document,
        editor: &mut EditorState,
        events: Vec<Event>,
    ) {
        let input = RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            ..RawInput::default()
        };
        ctx.run_ui(input, |ui| panel.show(ui.ctx(), doc, editor))
            .drop_without_applying_deltas();
        commands::apply_pending(doc, editor, &mut ViewportPanel::new(None));
    }

    #[test]
    fn showing_every_page_changes_nothing() {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::with_demo_scene(), EditorState::default());
        editor.show_render_settings = true;
        let mut panel = RenderSettingsPanel::default();
        for page in Page::ALL {
            panel.page = page;
            for _ in 0..3 {
                frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());
            }
        }
        assert_eq!(doc.render, RenderSettings::default());
        assert!(!editor.history.can_undo(), "no undo step");
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn a_typed_value_is_one_undo_step() {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::with_demo_scene(), EditorState::default());
        editor.show_render_settings = true;
        let mut panel = RenderSettingsPanel {
            page: Page::Sampling,
            ..RenderSettingsPanel::default()
        };
        frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());

        // Tab past the close button, the title bar and the five page buttons to
        // the Samples field, type 64, Enter.
        for _ in 0..8 {
            frame(
                &ctx,
                &mut panel,
                &mut doc,
                &mut editor,
                vec![key(Key::Tab, egui::Modifiers::NONE)],
            );
        }
        let typed = vec![key(Key::A, egui::Modifiers::COMMAND), Event::Text("64".to_owned())];
        frame(&ctx, &mut panel, &mut doc, &mut editor, typed);
        assert!(!editor.history.can_undo(), "nothing recorded while typing");
        frame(
            &ctx,
            &mut panel,
            &mut doc,
            &mut editor,
            vec![key(Key::Enter, egui::Modifiers::NONE)],
        );
        frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());
        assert_eq!(doc.render.samples, 64);
        assert_eq!(editor.history.undo_label(), Some("Render Settings"));
        assert_eq!(editor.history.undo_len(), 1);

        editor.commands.push(EditorCommand::Undo);
        frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());
        assert_eq!(doc.render.samples, 16, "undo restores the setting");
        frame(&ctx, &mut panel, &mut doc, &mut editor, Vec::new());
        assert!(!editor.history.can_undo(), "the undo itself is not recorded again");
    }

    /// Tabs `tabs` times from the start of the window, types `text` and Enter.
    fn type_into(
        ctx: &egui::Context,
        panel: &mut RenderSettingsPanel,
        doc: &mut Document,
        editor: &mut EditorState,
        tabs: usize,
        text: &str,
    ) {
        frame(ctx, panel, doc, editor, Vec::new());
        for _ in 0..tabs {
            frame(ctx, panel, doc, editor, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        }
        let typed = vec![key(Key::A, egui::Modifiers::COMMAND), Event::Text(text.to_owned())];
        frame(ctx, panel, doc, editor, typed);
        frame(ctx, panel, doc, editor, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        frame(ctx, panel, doc, editor, Vec::new());
        // Leave the field, so the next call starts from the window's beginning.
        ctx.memory_mut(egui::Memory::stop_text_input);
        frame(ctx, panel, doc, editor, Vec::new());
    }

    #[test]
    fn lock_ratio_and_a_typed_from_frame() {
        let ctx = egui::Context::default();
        let (mut doc, mut editor) = (Document::with_demo_scene(), EditorState::default());
        doc.render.lock_ratio = true;
        editor.show_render_settings = true;
        let mut panel = RenderSettingsPanel::default();

        // Close, title, five pages, Preset, then Width.
        type_into(&ctx, &mut panel, &mut doc, &mut editor, 9, "1920");
        assert_eq!((doc.render.width, doc.render.height), (1920, 1080), "16:9 kept");
        assert_eq!(editor.history.undo_len(), 1, "one step for both sides");

        // … Height, Lock Ratio, Frame Range, then From: the range becomes Manual.
        type_into(&ctx, &mut panel, &mut doc, &mut editor, 13, "5");
        assert_eq!(doc.render.frames, FrameRange::Manual { from: 5, to: 5 });
        assert_eq!(editor.history.undo_len(), 2);
    }
}
