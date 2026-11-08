// Grid4D — what surrounds the scene, behind everything: a Sky (the daylight
// model per pixel, its sun as a disc) or an Environment picture
// (equirectangular), or the plain grey of one without a picture.
// One full-screen triangle, drawn first; nothing else is tested against it.

// Must match `SkyUniform` in sky.rs.
struct Sky {
    // Clip space → world space.
    inverse_view_proj: mat4x4<f32>,
    // xyz: eye position, w: 1.0 if this shader must encode to sRGB itself.
    eye: vec4<f32>,
    // x: 1 sky, 2 picture, 3 plain; y: the picture's intensity, z: its mip level.
    mode: vec4<f32>,
    // The Perez coefficients A … E for luminance, x and y (xyz each).
    perez_a: vec4<f32>,
    perez_b: vec4<f32>,
    perez_c: vec4<f32>,
    perez_d: vec4<f32>,
    perez_e: vec4<f32>,
    // xyz: the zenith values over the Perez value there (Y, x, y), w: gain.
    scale: vec4<f32>,
    // xyz: towards the sun the model is evaluated with.
    model_sun: vec4<f32>,
    // rgb: below the horizon.
    ground: vec4<f32>,
    // xyz: towards the sun, w: cosine of the disc's radius (2: no sun).
    sun: vec4<f32>,
    // rgb: the disc's colour as shown.
    sun_color: vec4<f32>,
    // Rows: world → the picture's own frame.
    to_picture: array<vec4<f32>, 3>,
    // rgb: the plain surround.
    plain: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    // A triangle that covers the screen.
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    var out: VertexOut;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

const PI: f32 = 3.14159265;

fn perez(cos_theta: f32, gamma: f32) -> vec3<f32> {
    let cos_gamma = cos(gamma);
    return (vec3<f32>(1.0) + sky.perez_a.xyz * exp(sky.perez_b.xyz / max(cos_theta, 0.01)))
        * (vec3<f32>(1.0) + sky.perez_c.xyz * exp(sky.perez_d.xyz * gamma) + sky.perez_e.xyz * cos_gamma * cos_gamma);
}

// CIE xyY → linear sRGB (as `xyy_to_rgb` in grid4d-eval's sky.rs).
fn xyy_to_rgb(x: f32, y: f32, luminance: f32) -> vec3<f32> {
    if (y <= 1e-6 || luminance <= 0.0) {
        return vec3<f32>(0.0);
    }
    let big_x = x / y * luminance;
    let big_z = (1.0 - x - y) / y * luminance;
    let rgb = vec3<f32>(
        3.2406 * big_x - 1.5372 * luminance - 0.4986 * big_z,
        -0.9689 * big_x + 1.8758 * luminance + 0.0415 * big_z,
        0.0557 * big_x - 0.2040 * luminance + 1.0570 * big_z,
    );
    return max(rgb, vec3<f32>(0.0));
}

fn sky_colour(d: vec3<f32>) -> vec3<f32> {
    if (d.y <= 0.0) {
        return sky.ground.rgb;
    }
    let gamma = acos(clamp(dot(d, sky.model_sun.xyz), -1.0, 1.0));
    let values = sky.scale.xyz * perez(d.y, gamma);
    var rgb = xyy_to_rgb(values.y, values.z, values.x) * sky.scale.w;
    if (dot(d, sky.sun.xyz) >= sky.sun.w) {
        rgb = sky.sun_color.rgb;
    }
    return rgb;
}

fn picture_colour(d_world: vec3<f32>) -> vec3<f32> {
    let d = vec3<f32>(
        dot(sky.to_picture[0].xyz, d_world),
        dot(sky.to_picture[1].xyz, d_world),
        dot(sky.to_picture[2].xyz, d_world),
    );
    // The centre column along −Z, u eastwards (+X), v from the zenith down.
    let u = atan2(d.x, -d.z) / (2.0 * PI) + 0.5;
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return textureSampleLevel(picture, picture_sampler, vec2<f32>(u, v), sky.mode.z).rgb * sky.mode.y;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.0031308);
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, cutoff);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let far = sky.inverse_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let d = normalize(far.xyz / far.w - sky.eye.xyz);
    let mode = u32(sky.mode.x + 0.5);
    var rgb = sky.plain.rgb;
    if (mode == 1u) {
        rgb = sky_colour(d);
    } else if (mode == 2u) {
        rgb = picture_colour(d);
    }
    // No tone mapping in the viewport: brighter than white shows white.
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if (sky.eye.w > 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
