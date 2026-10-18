// OpenC4D — ground grid on the XZ plane.
// A single camera-centred quad; lines are computed analytically per pixel with
// screen-space derivatives, so they stay 1 px wide and anti-aliased at any zoom.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light_dir: vec4<f32>,
    // x: minor spacing, y: major spacing, z: fade distance, w: half extent.
    grid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_xz: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    // Snap the quad centre to the major grid so it never swims under the camera.
    let major = camera.grid.y;
    let center = floor(camera.eye.xz / major) * major;
    let xz = center + corners[index] * camera.grid.w;

    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(xz.x, 0.0, xz.y, 1.0);
    out.world_xz = xz;
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.0031308);
    let lower = c * 12.92;
    let higher = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, cutoff);
}

// Coverage (0..1) of grid lines `width_px` wide, spaced `spacing` world units apart.
fn grid_coverage(coord: vec2<f32>, spacing: f32, width_px: f32) -> f32 {
    let p = coord / spacing;
    let derivative = max(fwidth(p), vec2<f32>(1e-6));
    let dist_px = abs(fract(p - 0.5) - 0.5) / derivative;
    return 1.0 - min(min(dist_px.x, dist_px.y) / width_px, 1.0);
}

fn axis_coverage(distance: f32, derivative: f32, width_px: f32) -> f32 {
    return 1.0 - min(abs(distance) / (max(derivative, 1e-6) * width_px), 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let minor = grid_coverage(in.world_xz, camera.grid.x, 1.0);
    let major = grid_coverage(in.world_xz, camera.grid.y, 1.4);
    let d = fwidth(in.world_xz);
    let x_axis = axis_coverage(in.world_xz.y, d.y, 1.6); // the X axis is the line z = 0
    let z_axis = axis_coverage(in.world_xz.x, d.x, 1.6); // the Z axis is the line x = 0

    // Fade with distance: hides the quad's edge and moiré towards the horizon.
    let dist = distance(vec3<f32>(in.world_xz.x, 0.0, in.world_xz.y), camera.eye.xyz);
    let fade = 1.0 - smoothstep(camera.grid.z * 0.3, camera.grid.z, dist);

    var color = vec3<f32>(0.2);
    var alpha = max(minor * 0.2, major * 0.42);
    color = mix(color, vec3<f32>(0.72, 0.07, 0.06), x_axis);
    alpha = max(alpha, x_axis * 0.85);
    color = mix(color, vec3<f32>(0.05, 0.2, 0.78), z_axis);
    alpha = max(alpha, z_axis * 0.85);
    alpha = alpha * fade;

    if (camera.eye.w > 0.5) {
        color = linear_to_srgb(color);
    }
    // Premultiplied alpha.
    return vec4<f32>(color * alpha, alpha);
}
