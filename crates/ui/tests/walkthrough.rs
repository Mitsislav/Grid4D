//! The interface rendered to pictures for review, in English and Greek and
//! in the Dark and Light themes: the workspace, every menu, the managers'
//! tabs, the windows and dialogs, and the viewport's menus — what the manual
//! shows, without a window or a mouse. Run by hand (it needs a graphics
//! adapter; a software one will do) and look at the pictures:
//!
//! `GRID4D_WALKTHROUGH=<folder> cargo test -p grid4d-ui --test walkthrough -- --ignored`

#![allow(clippy::unwrap_used, reason = "test code")]

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable, wgpu::WgpuTestRenderer};
use grid4d_scene::{Document, NodeId};
use grid4d_ui::{
    EditorState, ViewportPanel,
    i18n::Language,
    shell::Shell,
    theme::ColorTheme,
    workspace::{Pane, Workspace},
};

const SIZE: [f32; 2] = [1600.0, 900.0];

fn harness(language: Language, theme: ColorTheme) -> Harness<'static, Shell> {
    let render_state = egui_kittest::wgpu::create_render_state(
        egui_kittest::wgpu::default_wgpu_setup(),
        egui_wgpu::RendererOptions::PREDICTABLE,
    );
    let viewport = ViewportPanel::new(Some(&render_state));
    let doc = Document::with_demo_scene();
    let mut editor = EditorState::default();
    if let Some(&first) = doc.scene.roots().first() {
        editor.selection.set_single(first);
    }
    // What About shows: this checkout's commit (as build.rs gives the app).
    let commit = git(&["rev-parse", "--short=7", "HEAD"]);
    let date = git(&["log", "-1", "--format=%cs"]);
    editor.build = grid4d_ui::BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: Box::leak(commit.into_boxed_str()),
        date: Box::leak(date.into_boxed_str()),
        licences: include_str!("../../../assets/THIRD_PARTY_LICENSES.txt"),
    };
    editor.preferences.language = language;
    editor.preferences.theme = theme;
    let shell = Shell::new(doc, editor, viewport, Workspace::default());
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .renderer(WgpuTestRenderer::from_render_state(render_state))
        // The harness sets its own theme after the setup: name the same one.
        .with_theme(match theme {
            ColorTheme::Dark | ColorTheme::Darker => egui::Theme::Dark,
            ColorTheme::Light => egui::Theme::Light,
        })
        .with_context_setup(move |ctx| grid4d_ui::theme::apply(ctx, theme))
        .build_ui_state(|ui, shell: &mut Shell| shell.ui(ui), shell);
    harness.run_steps(4);
    harness
}

/// A line of git's output, empty without git.
fn git(args: &[&str]) -> String {
    std::process::Command::new("git")
        .args(args)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn save(harness: &mut Harness<'_, Shell>, dir: &Path, name: &str) {
    harness.run_steps(2);
    let image = harness.render().unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(image.as_raw())
        .unwrap();
}

/// The buttons in a row of the interface (`top`..`bottom` in points), left to
/// right: the main menu, a view's header.
fn buttons_in_row(harness: &Harness<'_, Shell>, top: f32, bottom: f32, left: f32, right: f32) -> Vec<egui::Pos2> {
    let mut centres: Vec<egui::Pos2> = harness
        .get_all_by_role(Role::Button)
        .map(|node| node.rect().center())
        .filter(|c| c.y > top && c.y < bottom && c.x > left && c.x < right)
        .collect();
    centres.sort_by(|a, b| a.x.total_cmp(&b.x));
    centres
}

fn click(harness: &mut Harness<'_, Shell>, at: egui::Pos2) {
    harness.hover_at(at);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(2);
}

fn close_popups(harness: &mut Harness<'_, Shell>) {
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
}

fn named(shell: &Shell, name: &str) -> NodeId {
    shell.doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
}

/// Opens a window by setting the editor state that shows it.
type Open = fn(&mut EditorState);

fn pass(dir: &Path, language: Language, theme: ColorTheme) {
    let tag = format!(
        "{}-{}",
        match language {
            Language::English => "en",
            Language::Greek => "el",
        },
        match theme {
            ColorTheme::Dark => "dark",
            ColorTheme::Darker => "darker",
            ColorTheme::Light => "light",
        }
    );
    let mut h = harness(language, theme);
    save(&mut h, dir, &format!("{tag}-00-workspace"));

    // Every main menu.
    let menus = buttons_in_row(&h, 0.0, 26.0, 0.0, SIZE[0] - 200.0);
    for (k, at) in menus.iter().enumerate() {
        click(&mut h, *at);
        save(&mut h, dir, &format!("{tag}-01-menu-{k:02}"));
        close_popups(&mut h);
    }

    // The viewport's header menus (the row under the command palette).
    let view = buttons_in_row(&h, 66.0, 90.0, 40.0, SIZE[0] * 0.65);
    for (k, at) in view.iter().take(6).enumerate() {
        click(&mut h, *at);
        save(&mut h, dir, &format!("{tag}-02-view-menu-{k}"));
        close_popups(&mut h);
    }

    // The managers that start as tabs behind others.
    for (k, pane) in [
        Pane::Takes,
        Pane::Layers,
        Pane::Coordinates,
        Pane::FCurve,
        Pane::Materials,
    ]
    .into_iter()
    .enumerate()
    {
        h.state_mut().editor.panes.show = Some(pane);
        h.run_steps(3);
        save(&mut h, dir, &format!("{tag}-03-pane-{k}"));
    }

    // The Attribute Manager on a few kinds of object.
    for (k, name) in ["Sphere", "Light", "Camera", "Group"].into_iter().enumerate() {
        let id = named(h.state(), name);
        h.state_mut().editor.selection.set_single(id);
        h.state_mut().editor.panes.show = Some(Pane::Attributes);
        h.run_steps(3);
        save(&mut h, dir, &format!("{tag}-04-attributes-{k}"));
    }

    // The windows and dialogs.
    let windows: [(&str, Open); 8] = [
        ("about", |e| e.show_about = true),
        ("licences", |e| {
            e.show_about = true;
            e.show_licences = true;
        }),
        ("preferences", |e| e.show_preferences = true),
        ("render-settings", |e| e.show_render_settings = true),
        ("optimize", |e| e.show_optimize = true),
        ("script-manager", |e| e.scripts.show = true),
        ("picture-viewer", |e| e.renders.viewer_open = true),
        ("crash-report", |e| {
            e.crash.new_report = Some(PathBuf::from("crash-2026-11-01T13-41-07-4242.txt"))
        }),
    ];
    for (k, (name, open)) in windows.into_iter().enumerate() {
        open(&mut h.state_mut().editor);
        h.run_steps(3);
        save(&mut h, dir, &format!("{tag}-05-window-{k}-{name}"));
        let editor = &mut h.state_mut().editor;
        editor.show_about = false;
        editor.show_licences = false;
        editor.show_preferences = false;
        editor.show_render_settings = false;
        editor.show_optimize = false;
        editor.scripts.show = false;
        editor.renders.viewer_open = false;
        editor.crash.new_report = None;
        h.run_steps(2);
    }
}

#[test]
#[ignore = "renders the interface for review; needs a graphics adapter"]
fn render_the_interface_for_review() {
    let dir = std::env::var_os("GRID4D_WALKTHROUGH")
        .map_or_else(|| std::env::temp_dir().join("grid4d-walkthrough"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    for language in [Language::English, Language::Greek] {
        for theme in [ColorTheme::Dark, ColorTheme::Light] {
            pass(&dir, language, theme);
        }
    }
    println!("pictures in {}", dir.display());
}
