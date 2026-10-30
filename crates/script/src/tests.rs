use glam::Vec3;
use grid4d_geom::PolyMesh;
use grid4d_scene::{Document, Material, MeshHandle, Node, NodeId, ObjectKind, ObjectKindTag, PluginValue, Scene};

use crate::{Outcome, run};

/// The plugins of these tests, installed once.
fn plugins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut registry = grid4d_plugin::Registry::default();
        registry.add_generator(grid4d_plugin_gear::Gear);
        registry.add_command(grid4d_plugin_scatter::Scatter);
        grid4d_plugin::install(registry);
    });
}

/// Polygon objects' meshes only (enough for these tests).
fn shapes(scene: &Scene, id: NodeId, _: f64) -> Option<PolyMesh> {
    match &scene.get(id)?.data.kind {
        ObjectKind::Polygon { mesh, .. } => Some(mesh.mesh().clone()),
        _ => None,
    }
}

fn script(doc: &Document, source: &str) -> Result<Outcome, crate::ScriptError> {
    plugins();
    run(doc, &[], 0.0, shapes, source)
}

fn named(doc: &Document, name: &str) -> NodeId {
    doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0
}

#[test]
fn a_grid_of_coloured_cubes_undoes_in_one_go() {
    let doc = Document::new("grid");
    let outcome = script(
        &doc,
        r#"
        for x in 0..3 {
            for z in 0..3 {
                let cube = create("Cube");
                cube.name = `Cube ${x}-${z}`;
                cube.position = [x * 250, 0, z * 250.0];
                cube.color = [x / 2.0, 0.5, z / 2.0];
                cube.set("Size . Y", 50);
            }
        }
        print(`made ${objects().len()}`);
        "#,
    )
    .unwrap();
    let mut made = outcome.document;
    assert_eq!(made.scene.len(), 9);
    assert_eq!(outcome.output, ["made 9"]);
    let corner = made.scene.get(named(&made, "Cube 2-1")).unwrap();
    assert_eq!(corner.data.transform.position, Vec3::new(500.0, 0.0, 250.0));
    assert_eq!(corner.data.display_color, [1.0, 0.5, 0.5]);
    assert!(matches!(corner.data.kind, ObjectKind::Cube { size, .. } if size.y == 50.0));
    assert!(outcome.selection.is_none());
    // Its inverses take all of it back.
    for inverse in outcome.inverses {
        made.apply(inverse).unwrap();
    }
    assert_eq!(made.scene.len(), 0);
    assert_eq!(doc.scene.len(), 0, "the caller's document never changes");
}

#[test]
fn a_failing_script_changes_nothing_and_says_where() {
    let doc = Document::with_demo_scene();
    let err = script(
        &doc,
        "create(\"Cube\");\nprint(\"so far\");\nlet t = create(\"Teapot\");",
    )
    .unwrap_err();
    assert_eq!((err.line, err.column), (Some(3), Some(9)));
    assert!(err.message.contains("Teapot"), "{}", err.message);
    assert_eq!(err.output, ["so far"]);
    assert_eq!(err.to_string(), format!("Line 3, column 9: {}", err.message));
    // A syntax error, before anything runs.
    let err = script(&doc, "let x = ;").unwrap_err();
    assert_eq!(err.line, Some(1));
    // Using a deleted object.
    let err = script(&doc, "let c = find(\"Cube\"); delete(c); c.name = \"x\";").unwrap_err();
    assert!(err.message.contains("no longer exists"), "{}", err.message);
}

#[test]
fn an_endless_loop_is_stopped() {
    let doc = Document::new("loop");
    let err = script(&doc, "create(\"Null\"); let n = 0; loop { n += 1; }").unwrap_err();
    assert!(err.message.contains("stopped"), "{}", err.message);
}

#[test]
fn a_script_runs_a_plugin_command_on_its_selection() {
    let mut doc = Document::new("scatter");
    let floor = PolyMesh::from_faces(
        vec![
            Vec3::new(-100.0, 0.0, 100.0),
            Vec3::new(100.0, 0.0, 100.0),
            Vec3::new(100.0, 0.0, -100.0),
            Vec3::new(-100.0, 0.0, -100.0),
        ],
        [[0, 1, 2, 3]],
    )
    .unwrap();
    doc.scene.insert(Node::new(
        "Floor",
        ObjectKind::Polygon {
            mesh: MeshHandle::new(floor),
            components: grid4d_scene::Components::default(),
        },
    ));
    doc.scene.insert(Node::new("Rock", ObjectKind::Null));
    let outcome = script(
        &doc,
        r#"
        select([find("Rock"), find("Floor")]);
        let made = run_command("Scatter", #{ Count: 4, Seed: 3, "Align to Surface": false });
        print(made.len());
        select(made);
        "#,
    )
    .unwrap();
    let group = named(&outcome.document, "Scatter");
    assert_eq!(outcome.document.scene.children(group).len(), 4);
    assert_eq!(outcome.output, ["1"]);
    assert_eq!(outcome.selection, Some(vec![group]));
    let err = script(&doc, "run_command(\"Scatter\")").unwrap_err();
    assert!(err.message.contains("surface last"), "{}", err.message);
    let err = script(&doc, "run_command(\"Scatter\", #{ Colour: 1 })").unwrap_err();
    assert!(err.message.contains("no option"), "{}", err.message);
}

#[test]
fn parameters_placement_hierarchy_materials_by_name() {
    let mut doc = Document::new("params");
    doc.scene
        .insert(Node::new("Ball", ObjectKindTag::Sphere.default_kind()));
    doc.scene.materials_mut().insert(Material::new("Red"));
    let outcome = script(
        &doc,
        r#"
        let ball = find("Ball");
        print(ball.get("Radius"));
        ball.set("radius", 50);
        ball.rotation = [90, 0, 0];
        ball.material = "Red";
        let gear = create("Gear");
        gear.set("Teeth", 20);
        gear.set("Tooth Width", 50);
        print(gear.get("Tooth Width"));
        print(gear.kind);
        let group = create("Null");
        group.position = [0, 100, 0];
        ball.parent = group;
        print(ball.parent == group);
        print(group.children.len());
        print(ball.material);
        print(find("Nobody") == ());
        "#,
    )
    .unwrap();
    assert_eq!(outcome.output, ["100.0", "50.0", "Gear", "true", "1", "Red", "true"]);
    let doc = &outcome.document;
    let ball = doc.scene.get(named(doc, "Ball")).unwrap();
    assert!(matches!(ball.data.kind, ObjectKind::Sphere { radius, .. } if radius == 50.0));
    assert!((ball.data.transform.rotation.x - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert_eq!(
        ball.data.transform.position.y, -100.0,
        "kept in place under its new parent"
    );
    let ObjectKind::Plugin(gear) = &doc.scene.get(named(doc, "Gear")).unwrap().data.kind else {
        panic!("a gear")
    };
    assert_eq!(gear.get("Teeth"), Some(PluginValue::Int(20)));
    assert_eq!(gear.get("Tooth Width"), Some(PluginValue::Float(0.5)));

    for wrong in [
        "find(\"Ball\").set(\"Teeth\", 3)",
        "find(\"Ball\").material = \"Blue\"",
        "create(\"Scatter\")",
    ] {
        assert!(script(&outcome.document, wrong).is_err(), "{wrong}");
    }
}
