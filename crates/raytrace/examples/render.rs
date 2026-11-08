//! Renders a Grid4D document — or the demo scene — to a PNG file.
//!
//! ```text
//! cargo run --release -p grid4d-raytrace --example render -- [scene.g4d | demo] [out.png] [WIDTHxHEIGHT] [samples]
//! ```
//!
//! The picture is seen through the document's first camera object, else from a
//! view that frames the whole scene. Size, samples, bounces, Auto Light and Alpha
//! Channel come from the document's render settings (Ctrl+B in the app); the
//! size and samples given here override them. Materials show their texture
//! pictures (a picture that cannot be read renders as its channel's value).
//! Defaults: the demo scene, `render.png`.

use std::{error::Error, fs::File, io::BufWriter, path::Path, sync::Arc, time::Instant};

use grid4d_eval::EvaluatedScene;
use grid4d_io::Pixels;
use grid4d_raytrace::{
    CameraProjection, Picture, Pictures, RenderCamera, RenderOptions, RenderScene, Texels, Texturing, render,
};
use grid4d_scene::{ColorSpace, Document, NodeId, ObjectKind, TextureSource};

/// The document's texture pictures, decoded (those that cannot be read are
/// left out, with a note).
fn pictures(doc: &Document) -> Pictures {
    let mut pictures = Pictures::new();
    for (id, texture) in doc.scene.textures().iter() {
        let bytes = match &texture.source {
            TextureSource::Linked(path) => grid4d_io::read_file(path).map(Arc::<[u8]>::from),
            TextureSource::Embedded { bytes, .. } => Ok(Arc::clone(bytes)),
        };
        let image = bytes.and_then(|bytes| grid4d_io::decode_texture(&bytes, texture.color_space));
        let Some(level) = image.as_ref().ok().and_then(|image| image.levels.first()) else {
            println!(
                "texture {:?} cannot be read: its channels' values are used",
                texture.name
            );
            continue;
        };
        let texels = match (&level.pixels, texture.color_space) {
            (Pixels::Rgba8(bytes), ColorSpace::Srgb) => Texels::Srgb8(Arc::clone(bytes)),
            (Pixels::Rgba8(bytes), ColorSpace::Linear) => Texels::Linear8(Arc::clone(bytes)),
            (Pixels::RgbaF16(values), _) => Texels::Half(Arc::clone(values)),
        };
        if let Some(picture) = Picture::new(level.width, level.height, texels) {
            pictures.insert(id, Arc::new(picture));
        }
    }
    pictures
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = args.first().map_or("demo", String::as_str);
    let output = args.get(1).map_or("render.png", String::as_str);
    let doc = if source == "demo" {
        Document::with_demo_scene()
    } else {
        grid4d_io::load(Path::new(source))?
    };
    let mut options = RenderOptions::from(&doc.render);
    if let Some((w, h)) = args.get(2).and_then(|size| size.split_once('x')) {
        (options.width, options.height) = (w.parse()?, h.parse()?);
    }
    if let Some(samples) = args.get(3) {
        options.samples = samples.parse()?;
    }
    let mut evaluated = EvaluatedScene::for_render();
    evaluated.update(&doc.scene);
    let cameras: Vec<(NodeId, RenderCamera)> = doc
        .scene
        .iter()
        .filter_map(|(id, node)| match node.data.kind {
            ObjectKind::Camera { fov_y_deg, .. } => Some((
                id,
                RenderCamera {
                    world: doc.scene.world_matrix(id),
                    projection: CameraProjection::Perspective {
                        fov_y: fov_y_deg.to_radians(),
                    },
                    lens: None,
                    end: None,
                },
            )),
            _ => None,
        })
        .collect();
    let camera = cameras
        .first()
        .map(|&(_, camera)| camera)
        .or_else(|| {
            RenderScene::new(&evaluated)
                .bounds()
                .map(|(min, max)| RenderCamera::framing(min, max, 40f32.to_radians()))
        })
        .ok_or("the scene is empty")?;

    let started = Instant::now();
    let pictures = pictures(&doc);
    let texturing = Texturing {
        materials: doc.scene.materials(),
        pictures: &pictures,
        camera,
        aspect: options.width.max(1) as f32 / options.height.max(1) as f32,
        cameras: &cameras,
    };
    let scene = RenderScene::textured(&evaluated, &texturing);
    let built = started.elapsed();

    let image = render(&scene, &camera, &options);
    println!(
        "{} triangles, built in {:.2?}; {}×{} at {} samples rendered in {:.2?}",
        scene.triangle_count(),
        built,
        image.width,
        image.height,
        options.samples,
        started.elapsed() - built,
    );

    let mut encoder = png::Encoder::new(BufWriter::new(File::create(output)?), image.width, image.height);
    encoder.set_depth(png::BitDepth::Eight);
    if doc.render.alpha {
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header()?.write_image_data(&image.to_rgba8())?;
    } else {
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header()?.write_image_data(&image.to_rgb8())?;
    }
    println!("saved {output}");
    Ok(())
}
