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
    harness_with(Document::with_demo_scene(), language, theme)
}

fn harness_with(doc: Document, language: Language, theme: ColorTheme) -> Harness<'static, Shell> {
    let render_state = egui_kittest::wgpu::create_render_state(
        egui_kittest::wgpu::default_wgpu_setup(),
        egui_wgpu::RendererOptions::PREDICTABLE,
    );
    let viewport = ViewportPanel::new(Some(&render_state));
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

/// A scene that shows what Grid4D does at a glance: clones moved by an
/// effector, extruded text, a smoothed cage, a bent column, a light with
/// shadows on a floor.
fn showcase() -> Document {
    use glam::Vec3;
    use grid4d_geom::cloner::{Cloner, GridArray, Layout};
    use grid4d_scene::{LightShadow, Node, NodeData, ObjectKind, ObjectKindTag};

    let mut doc = Document::new("Showcase");
    let place = |kind: ObjectKind, name: &str, at: Vec3, color: [f32; 3]| {
        let mut data = NodeData::new(name, kind);
        data.transform.position = at;
        data.display_color = color;
        Node::from_data(data)
    };
    let floor = ObjectKind::Plane {
        width: 40000.0,
        depth: 40000.0,
        width_segments: 1,
        depth_segments: 1,
    };
    doc.scene.insert(place(floor, "Floor", Vec3::ZERO, [0.42, 0.42, 0.45]));

    // Clones, lifted and scaled at random.
    let random = doc.scene.insert(place(
        ObjectKindTag::RandomEffector.default_kind(),
        "Random",
        Vec3::new(-60.0, 0.0, 260.0),
        [0.6, 0.6, 0.6],
    ));
    let cloner = ObjectKind::Cloner {
        cloner: Cloner {
            layout: Layout::Grid(GridArray {
                count: [5, 1, 5],
                size: Vec3::new(95.0, 0.0, 95.0),
                ..GridArray::default()
            }),
            ..Cloner::default()
        },
        object: None,
        effectors: vec![random],
    };
    let cloner = doc
        .scene
        .insert(place(cloner, "Cloner", Vec3::new(-60.0, 45.0, 260.0), [0.2, 0.45, 0.9]));
    let sphere = ObjectKind::Sphere {
        radius: 38.0,
        segments: 24,
    };
    doc.scene
        .insert_child(cloner, place(sphere, "Sphere", Vec3::ZERO, [0.2, 0.45, 0.9]))
        .unwrap();

    // Extruded text.
    let mut extrude = ObjectKindTag::Extrude.default_kind();
    if let ObjectKind::Extrude { extrusion, .. } = &mut extrude {
        extrusion.offset = 60.0;
    }
    let extrude = doc.scene.insert(place(
        extrude,
        "Extrude",
        Vec3::new(-380.0, 0.0, -320.0),
        [0.95, 0.55, 0.15],
    ));
    let text = ObjectKind::Text(grid4d_geom::spline::Text {
        height: 200.0,
        ..grid4d_geom::spline::Text::new("Grid4D")
    });
    doc.scene
        .insert_child(extrude, place(text, "Text", Vec3::ZERO, [0.95, 0.55, 0.15]))
        .unwrap();

    // A smoothed cage.
    let sds = doc.scene.insert(place(
        ObjectKindTag::SubdivisionSurface.default_kind(),
        "Subdivision Surface",
        Vec3::new(420.0, 95.0, 120.0),
        [0.3, 0.75, 0.45],
    ));
    let cage = ObjectKind::Cube {
        size: Vec3::splat(190.0),
        segments: [1, 1, 1],
    };
    doc.scene
        .insert_child(sds, place(cage, "Cube", Vec3::ZERO, [0.3, 0.75, 0.45]))
        .unwrap();

    // A bent column.
    let column = ObjectKind::Cylinder {
        radius: 35.0,
        height: 360.0,
        rotation_segments: 32,
        height_segments: 24,
    };
    let column = doc.scene.insert(place(
        column,
        "Column",
        Vec3::new(-560.0, 180.0, 40.0),
        [0.85, 0.3, 0.35],
    ));
    let mut bend = ObjectKindTag::Bend.default_kind();
    if let ObjectKind::Bend(deformer) = &mut bend {
        deformer.size = Vec3::new(70.0, 360.0, 70.0);
        deformer.strength = 1.1;
    }
    doc.scene
        .insert_child(column, place(bend, "Bend", Vec3::ZERO, [0.6, 0.6, 0.6]))
        .unwrap();

    // A light with soft shadows.
    let mut light = ObjectKindTag::Light.default_kind();
    if let ObjectKind::Light { shadow, intensity, .. } = &mut light {
        *shadow = LightShadow::ShadowMaps;
        *intensity = 1.1;
    }
    doc.scene
        .insert(place(light, "Light", Vec3::new(-700.0, 1100.0, 900.0), [1.0, 1.0, 1.0]));
    doc
}

/// The pictures of the README: the workspace with the showcase scene (shadows
/// and ambient occlusion on), and the same scene path-traced.
///
/// `GRID4D_WALKTHROUGH=<folder> cargo test --release -p grid4d-ui --test walkthrough -- --ignored readme`
#[test]
#[ignore = "renders the README's pictures; needs a graphics adapter"]
fn readme_pictures() {
    let dir = std::env::var_os("GRID4D_WALKTHROUGH")
        .map_or_else(|| std::env::temp_dir().join("grid4d-walkthrough"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness_with(showcase(), Language::English, ColorTheme::Dark);
    {
        let shell = h.state_mut();
        // Framed on the objects, so the floor fills the background.
        let objects: Vec<NodeId> = ["Cloner", "Extrude", "Subdivision Surface", "Column"]
            .into_iter()
            .map(|name| named(shell, name))
            .collect();
        shell.editor.selection.set_all(objects);
        shell
            .panels
            .viewport
            .frame_selection(&shell.doc.scene, &shell.editor.selection);
        let extrude = named(shell, "Extrude");
        shell.editor.selection.set_single(extrude);
    }
    // Options ▸ Shadows and SSAO.
    for effect in ["Shadows", "SSAO"] {
        let options = buttons_in_row(&h, 66.0, 90.0, 40.0, SIZE[0] * 0.65)[3];
        click(&mut h, options);
        let at = h.get_by_label(effect).rect().center();
        click(&mut h, at);
        close_popups(&mut h);
    }
    h.run_steps(6);
    save(&mut h, &dir, "readme-workspace");

    // The same view, path-traced.
    let shell = h.state();
    let mut evaluated = grid4d_eval::EvaluatedScene::for_render();
    evaluated.update(&shell.doc.scene);
    let scene = grid4d_raytrace::RenderScene::new(&evaluated);
    let view = shell.panels.viewport.camera();
    let camera = grid4d_raytrace::RenderCamera {
        world: view.view().inverse(),
        projection: grid4d_raytrace::CameraProjection::Perspective { fov_y: view.fov_y },
    };
    let options = grid4d_raytrace::RenderOptions {
        width: 1280,
        height: 720,
        samples: 96,
        ..grid4d_raytrace::RenderOptions::from(&shell.doc.render)
    };
    let image = grid4d_raytrace::render(&scene, &camera, &options);
    grid4d_io::save_png(
        &dir.join("readme-render.png"),
        image.width,
        image.height,
        grid4d_io::Channels::Rgba,
        &image.to_rgba8(),
    )
    .unwrap();
}
