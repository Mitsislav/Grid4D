// OpenC4D — shaded mesh pass.
// Instanced: one draw call per primitive mesh kind (cube, sphere, cylinder, plane).
// Lighting is done in linear space; the result is sRGB-encoded here when the
// surface format is not an *Srgb format (egui prefers a gamma framebuffer).

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz: eye position, w: 1.0 if this shader must encode to sRGB itself.
    eye: vec4<f32>,
    // xyz: direction towards the key light.
    light_dir: vec4<f32>,
    // x: minor spacing, y: major spacing, z: fade distance, w: half extent.
    grid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

// Must match `InstanceRaw` in frame.rs.
struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) normal_0: vec4<f32>,
    @location(7) normal_1: vec4<f32>,
    @location(8) normal_2: vec4<f32>,
    @location(9) color: vec4<f32>,
    // x: selected, y: unlit
    @location(10) params: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
};

@vertex
fn vs_main(vertex: VertexIn, instance: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    let normal_matrix = mat3x3<f32>(instance.normal_0.xyz, instance.normal_1.xyz, instance.normal_2.xyz);
    let world = model * vec4<f32>(vertex.position, 1.0);

    var out: VertexOut;
    out.clip_position = camera.view_proj * world;
    out.world_position = world.xyz;
    out.world_normal = normal_matrix * vertex.normal;
    out.color = instance.color;
    out.params = instance.params;
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.0031308);
    let lower = c * 12.92;
    let higher = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, cutoff);
}

const SELECTION_RIM: vec3<f32> = vec3<f32>(1.0, 0.42, 0.06);

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let view_dir = normalize(camera.eye.xyz - in.world_position);
    var n = normalize(in.world_normal);
    // Two-sided lighting that does not depend on winding (planes seen from below).
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }

    let light = normalize(camera.light_dir.xyz);
    let key = max(dot(n, light), 0.0);
    let facing = max(dot(n, view_dir), 0.0);
    let sky = 0.5 + 0.5 * n.y;
    let half_vec = normalize(light + view_dir);
    let specular = pow(max(dot(n, half_vec), 0.0), 48.0) * 0.08;

    let base = in.color.rgb;
    let lit = base * (0.08 + 0.22 * sky + 0.62 * key + 0.16 * facing) + vec3<f32>(specular);
    var rgb = mix(lit, base, in.params.y);

    // Selected objects get a subtle orange Fresnel rim (the UI adds bbox corner brackets).
    let rim = pow(1.0 - facing, 4.0);
    rgb = mix(rgb, SELECTION_RIM, clamp(rim * 0.8, 0.0, 1.0) * in.params.x);

    if (camera.eye.w > 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
