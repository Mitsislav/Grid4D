//! Reference-image test: a small render of the demo scene through its camera,
//! compared with a committed picture within a tolerance (floating-point details
//! differ a little between machines).
//!
//! After an intended change to the renderer, write a new reference with
//! `GRID4D_UPDATE_REFERENCE=1 cargo test -p grid4d-raytrace --test reference`
//! and look at it before committing.

use std::{fs::File, io::BufWriter};

use grid4d_eval::EvaluatedScene;
use grid4d_raytrace::{CameraProjection, Image, RenderCamera, RenderOptions, RenderScene, render};
use grid4d_scene::{Document, ObjectKind};

const REFERENCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/demo_reference.png");

fn demo() -> Image {
    let doc = Document::with_demo_scene();
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let (id, fov) = doc
        .scene
        .iter()
        .find_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg } => Some((id, fov_y_deg)),
            _ => None,
        })
        .expect("the demo scene has a camera");
    let camera = RenderCamera {
        world: doc.scene.world_matrix(id),
        projection: CameraProjection::Perspective {
            fov_y: fov.to_radians(),
        },
    };
    let options = RenderOptions {
        width: 96,
        height: 54,
        samples: 8,
        bounces: 1,
        seed: 0,
        auto_light: true,
    };
    render(&RenderScene::new(&evaluated), &camera, &options)
}

#[test]
fn the_demo_scene_matches_its_reference() {
    let image = demo();
    let bytes = image.to_rgba8();
    if std::env::var_os("GRID4D_UPDATE_REFERENCE").is_some() {
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(REFERENCE).unwrap()),
            image.width,
            image.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
        return;
    }
    let decoder = png::Decoder::new(std::io::BufReader::new(
        File::open(REFERENCE).expect("the reference picture"),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut reference = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut reference).unwrap();
    assert_eq!((info.width, info.height), (image.width, image.height));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let reference = &reference[..info.buffer_size()];

    let differences: Vec<u8> = bytes.iter().zip(reference).map(|(a, b)| a.abs_diff(*b)).collect();
    let mean = differences.iter().map(|&d| f64::from(d)).sum::<f64>() / differences.len() as f64;
    let large = differences.iter().filter(|&&d| d > 32).count();
    assert!(mean <= 2.0, "mean difference {mean:.2} of 255");
    assert!(
        large * 100 <= differences.len(),
        "{large} of {} values differ by more than 32",
        differences.len()
    );
}
