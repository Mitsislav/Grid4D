//! The Render Queue window (Render ▸ Render Queue…): the jobs, one row each,
//! and the buttons that add, start and stop them (see
//! `docs/reference/render-queue.md`). Editing the list changes editor state
//! only; adding and starting go through commands.

use egui::{Button, Checkbox, ComboBox, DragValue, Id, ProgressBar, RichText, ScrollArea, TextEdit, Ui, Window, vec2};

use crate::{
    EditorCommand, EditorState,
    i18n::{tr, trf},
    render_queue::{JobFrames, JobStatus, JobTake, QueueJob},
    theme::palette as p,
};

/// How often the window looks at a running queue.
const POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Shows the window while it is open.
pub fn show(ctx: &egui::Context, doc: &grid4d_scene::Document, editor: &mut EditorState) {
    if editor.render_queue.is_running() {
        // The editor moves the queue on once a frame, also with the window closed.
        ctx.request_repaint_after(POLL);
    }
    if !editor.show_render_queue {
        return;
    }
    let mut open = true;
    let screen = ctx.content_rect();
    let (done, total) = editor
        .render_queue
        .jobs
        .iter()
        .filter(|job| job.enabled)
        .fold((0, 0), |(d, t), job| (d + job.done, t + job.total));
    // The title says how far the whole queue is.
    let title = if editor.render_queue.is_running() && total > 0 {
        trf("Render Queue · {0} of {1} frames", &[&done, &total])
    } else {
        tr("Render Queue").to_owned()
    };
    Window::new(title)
        .id(Id::unique("grid4d.render_queue"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([640.0, 320.0])
        .default_pos(screen.center() - vec2(320.0, 160.0))
        .min_size([480.0, 200.0])
        .show(ctx, |ui| contents(ui, doc, editor));
    editor.show_render_queue = open;
}

fn contents(ui: &mut Ui, doc: &grid4d_scene::Document, editor: &mut EditorState) {
    let running = editor.render_queue.is_running();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!running, Button::new(tr("Start")))
            .on_hover_text(tr("Render the enabled jobs from the top, one after another"))
            .clicked()
        {
            editor.commands.push(EditorCommand::StartRenderQueue);
        }
        if ui
            .add_enabled(running, Button::new(tr("Stop")))
            .on_hover_text(tr(
                "Stop after the current pass; Start goes on from the first missing frame",
            ))
            .clicked()
        {
            editor.commands.push(EditorCommand::StopRenderQueue);
        }
        ui.separator();
        if ui
            .button(tr("Add Current Document"))
            .on_hover_text(tr("The document as saved: unsaved changes are not rendered"))
            .clicked()
        {
            editor
                .commands
                .push(EditorCommand::QueueCurrentDocument { all_takes: false });
        }
        if ui
            .button(tr("Add All Takes"))
            .on_hover_text(tr("One job per take of the current document, Main first"))
            .clicked()
        {
            editor
                .commands
                .push(EditorCommand::QueueCurrentDocument { all_takes: true });
        }
        if ui.button(tr("Add Files…")).clicked() {
            editor.commands.push(EditorCommand::QueueFiles);
        }
    });
    ui.separator();
    let current = editor.file.path().map(std::path::Path::to_path_buf);
    let takes: Vec<String> = doc.takes.iter().map(|(_, take)| take.name.clone()).collect();
    let mut action = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if editor.render_queue.jobs.is_empty() {
            ui.label(
                RichText::new(tr(
                    "No jobs. Save the document and add it, or add saved documents with Add Files…",
                ))
                .color(p().text_dim),
            );
        }
        let running = editor.render_queue.running_index();
        let count = editor.render_queue.jobs.len();
        for (index, job) in editor.render_queue.jobs.iter_mut().enumerate() {
            let same = current.as_deref() == Some(job.path.as_path());
            ui.push_id(index, |ui| {
                if let Some(chosen) = row(ui, job, (index, count), running == Some(index), same.then_some(&takes)) {
                    action = Some((index, chosen));
                }
            });
            ui.separator();
        }
    });
    if let Some((index, action)) = action {
        let queue = &mut editor.render_queue;
        match action {
            RowAction::Up => queue.move_job(index, true),
            RowAction::Down => queue.move_job(index, false),
            RowAction::Duplicate => queue.duplicate(index),
            RowAction::Remove => queue.remove(index, &mut editor.renders),
            RowAction::Reset => {
                if let Some(job) = queue.jobs.get_mut(index) {
                    job.reset();
                }
            }
            RowAction::Show => {
                if let Some(number) = queue.running_render() {
                    editor.renders.shown = Some(number);
                }
                editor.renders.viewer_open = true;
            }
        }
    }
}

/// What a row's buttons ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    Up,
    Down,
    Duplicate,
    Remove,
    Reset,
    Show,
}

/// One job: its switch, name, take, frames, output and status.
fn row(
    ui: &mut Ui,
    job: &mut QueueJob,
    (index, count): (usize, usize),
    running: bool,
    takes: Option<&Vec<String>>,
) -> Option<RowAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let check = ui.add_enabled(!running, Checkbox::without_text(&mut job.enabled));
        ui.ctx()
            .accesskit_node_builder(check.id, |node| node.set_label(tr("Enabled")));
        ui.label(RichText::new(job.name()).strong())
            .on_hover_text(job.path.display().to_string());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!running, Button::new(tr("Remove"))).clicked() {
                action = Some(RowAction::Remove);
            }
            if ui.button(tr("Duplicate")).clicked() {
                action = Some(RowAction::Duplicate);
            }
            if ui
                .add_enabled(index + 1 < count, Button::new(tr("Down")))
                .on_hover_text(tr("Move Down"))
                .clicked()
            {
                action = Some(RowAction::Down);
            }
            if ui
                .add_enabled(index > 0, Button::new(tr("Up")))
                .on_hover_text(tr("Move Up"))
                .clicked()
            {
                action = Some(RowAction::Up);
            }
            let finished = !matches!(job.status, JobStatus::Ready);
            if ui
                .add_enabled(finished && !running, Button::new(tr("Reset")))
                .on_hover_text(tr("Back to Ready: Start renders every frame again"))
                .clicked()
            {
                action = Some(RowAction::Reset);
            }
            if running
                && ui
                    .button(tr("Show"))
                    .on_hover_text(tr("Show the render in the Picture Viewer"))
                    .clicked()
            {
                action = Some(RowAction::Show);
            }
        });
    });
    ui.add_enabled_ui(!running, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Take")).color(p().text_dim));
            take_menu(ui, job, takes);
            ui.label(RichText::new(tr("Frames")).color(p().text_dim));
            frames_menu(ui, job);
            ui.label(RichText::new(tr("Output")).color(p().text_dim));
            output_field(ui, job);
        });
    });
    // Status and message; a bar while it renders.
    let status = tr(job.status.label());
    let colour = match job.status {
        JobStatus::Error => p().record,
        JobStatus::Finished => p().enabled,
        _ => p().text_dim,
    };
    if running && job.total > 0 {
        #[expect(clippy::cast_precision_loss, reason = "a progress bar")]
        let fraction = job.done as f32 / job.total as f32;
        ui.add(
            ProgressBar::new(fraction)
                .desired_height(14.0)
                .text(format!("{status} · {}", job.message)),
        );
    } else if job.message.is_empty() {
        ui.label(RichText::new(status).color(colour).size(11.5));
    } else {
        ui.label(
            RichText::new(format!("{status} · {}", job.message))
                .color(colour)
                .size(11.5),
        );
    }
    action
}

/// The take a job renders: its document's render take, Main, or one by
/// name (the current document's names when it is that document).
fn take_menu(ui: &mut Ui, job: &mut QueueJob, takes: Option<&Vec<String>>) {
    let label = |take: &JobTake| match take {
        JobTake::RenderTake => tr("Render Take").to_owned(),
        JobTake::Main => tr("Main").to_owned(),
        JobTake::Named(name) => name.clone(),
    };
    let combo = ComboBox::from_id_salt("take")
        .selected_text(label(&job.take))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut job.take, JobTake::RenderTake, tr("Render Take"));
            ui.selectable_value(&mut job.take, JobTake::Main, tr("Main"));
            for name in takes.into_iter().flatten() {
                ui.selectable_value(&mut job.take, JobTake::Named(name.clone()), name);
            }
        });
    ui.ctx()
        .accesskit_node_builder(combo.response.id, |node| node.set_label(tr("Take")));
}

/// From the document, or a manual range.
fn frames_menu(ui: &mut Ui, job: &mut QueueJob) {
    let manual = matches!(job.frames, JobFrames::Manual { .. });
    let combo = ComboBox::from_id_salt("frames")
        .selected_text(if manual { tr("Manual") } else { tr("From Document") })
        .show_ui(ui, |ui| {
            if ui.selectable_label(!manual, tr("From Document")).clicked() {
                job.frames = JobFrames::FromDocument;
            }
            if ui.selectable_label(manual, tr("Manual")).clicked() && !manual {
                job.frames = JobFrames::Manual {
                    from: 0,
                    to: 0,
                    step: 1,
                };
            }
        });
    ui.ctx()
        .accesskit_node_builder(combo.response.id, |node| node.set_label(tr("Frames")));
    if let JobFrames::Manual { from, to, step } = &mut job.frames {
        let field = ui.add(DragValue::new(from).suffix(" F"));
        ui.ctx()
            .accesskit_node_builder(field.id, |node| node.set_label(tr("From")));
        let field = ui.add(DragValue::new(to).suffix(" F"));
        ui.ctx()
            .accesskit_node_builder(field.id, |node| node.set_label(tr("To")));
        let field = ui.add(DragValue::new(step).range(1..=10_000).prefix("× "));
        ui.ctx()
            .accesskit_node_builder(field.id, |node| node.set_label(tr("Frame Step")));
    }
}

/// From the document, or a file of the job's own.
fn output_field(ui: &mut Ui, job: &mut QueueJob) {
    let mut own = job.output.is_some();
    let check = ui
        .add(Checkbox::new(&mut own, tr("Own File")))
        .on_hover_text(tr("Save to another file than the document's Render Settings say"));
    if check.changed() {
        job.output = own.then(String::new);
    }
    if let Some(output) = &mut job.output {
        let field = ui.add(
            TextEdit::singleline(output)
                .desired_width(180.0)
                .hint_text(tr("renders/shot")),
        );
        ui.ctx()
            .accesskit_node_builder(field.id, |node| node.set_label(tr("Output")));
    }
}
