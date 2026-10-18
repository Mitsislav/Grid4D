// Grid4D — component overlay: points as screen-space squares, edges as
// screen-space quads of a fixed pixel width, and tinted polygons. Positions are
// world space; every vertex is pulled slightly towards the eye so components on
// a surface win the depth test against it without showing through the far side.

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz: eye position, w: 1.0 if this shader must encode to sRGB itself.
    eye: vec4<f32>,
    light_dir: vec4<f32>,
    grid: vec4<f32>,
    // xy: target size in pixels.
    viewport: vec4<f32>,
    // Linear RGBA.
    wire: vec4<f32>,
    selected: vec4<f32>,
    hover: vec4<f32>,
    // x: point size px, y: edge width px, z: selected edge width px, w: pull towards the eye.
    sizes: vec4<f32>,
    // x: selected face alpha, y: hovered face alpha, z: context edge alpha.
    alphas: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

const SELECTED: u32 = 1u;
const HOVERED: u32 = 2u;
const CONTEXT: u32 = 4u;

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Two triangles covering [-1, 1]².
fn quad_corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return corners[index];
}

fn pulled(p: vec3<f32>, amount: f32) -> vec4<f32> {
    let q = p + (camera.eye.xyz - p) * amount;
    return camera.view_proj * vec4<f32>(q, 1.0);
}

fn flag_color(flags: u32) -> vec4<f32> {
    if ((flags & HOVERED) != 0u) {
        return camera.hover;
    }
    if ((flags & SELECTED) != 0u) {
        return camera.selected;
    }
    if ((flags & CONTEXT) != 0u) {
        return vec4<f32>(camera.wire.rgb, camera.wire.a * camera.alphas.z);
    }
    return camera.wire;
}

// Behind the camera: a position the rasteriser clips away.
const CULLED: vec4<f32> = vec4<f32>(0.0, 0.0, -2.0, 1.0);

struct PointIn {
    @location(0) position: vec3<f32>,
    @location(1) flags: u32,
};

@vertex
fn vs_point(@builtin(vertex_index) index: u32, point: PointIn) -> VertexOut {
    var out: VertexOut;
    out.color = flag_color(point.flags);
    let clip = pulled(point.position, camera.sizes.w);
    if (clip.w <= 0.0) {
        out.clip_position = CULLED;
        return out;
    }
    var size = camera.sizes.x;
    if ((point.flags & (SELECTED | HOVERED)) != 0u) {
        size = size + 2.0;
    }
    // Half the size in pixels, as an NDC offset: px · 2 / viewport.
    let offset = quad_corner(index) * size / camera.viewport.xy;
    out.clip_position = vec4<f32>(clip.xy + offset * clip.w, clip.z, clip.w);
    return out;
}

struct EdgeIn {
    @location(0) a: vec3<f32>,
    @location(1) flags: u32,
    @location(2) b: vec3<f32>,
};

@vertex
fn vs_edge(@builtin(vertex_index) index: u32, edge: EdgeIn) -> VertexOut {
    var out: VertexOut;
    out.color = flag_color(edge.flags);
    var ca = pulled(edge.a, camera.sizes.w);
    var cb = pulled(edge.b, camera.sizes.w);
    // Clip to the near plane (z = 0) so an edge crossing it stays on screen.
    if (ca.z < 0.0 && cb.z < 0.0) {
        out.clip_position = CULLED;
        return out;
    }
    if (ca.z < 0.0) {
        ca = mix(ca, cb, ca.z / (ca.z - cb.z));
    } else if (cb.z < 0.0) {
        cb = mix(cb, ca, cb.z / (cb.z - ca.z));
    }
    let half_px = camera.viewport.xy * 0.5;
    let sa = ca.xy / ca.w * half_px;
    let sb = cb.xy / cb.w * half_px;
    var direction = sb - sa;
    if (dot(direction, direction) < 1e-8) {
        direction = vec2<f32>(1.0, 0.0);
    }
    let normal = normalize(vec2<f32>(-direction.y, direction.x));
    var width = camera.sizes.y;
    if ((edge.flags & (SELECTED | HOVERED)) != 0u) {
        width = camera.sizes.z;
    }
    let corner = quad_corner(index);
    var clip = ca;
    if (corner.x > 0.0) {
        clip = cb;
    }
    // Half the width on each side, as an NDC offset.
    let offset = normal * corner.y * width * 0.5 / half_px;
    out.clip_position = vec4<f32>(clip.xy + offset * clip.w, clip.z, clip.w);
    return out;
}

struct FaceIn {
    @location(0) position: vec3<f32>,
    @location(1) flags: u32,
};

@vertex
fn vs_face(face: FaceIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = pulled(face.position, camera.sizes.w * 0.5);
    if ((face.flags & HOVERED) != 0u) {
        out.color = vec4<f32>(camera.hover.rgb, camera.alphas.y);
    } else {
        out.color = vec4<f32>(camera.selected.rgb, camera.alphas.x);
    }
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.0031308);
    let lower = c * 12.92;
    let higher = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, cutoff);
}

// Premultiplied alpha, like the rest of the target.
@fragment
fn fs_overlay(in: VertexOut) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb;
    if (camera.eye.w > 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb * in.color.a, in.color.a);
}
