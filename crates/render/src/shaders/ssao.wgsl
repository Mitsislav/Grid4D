// Grid4D — screen-space ambient occlusion: a pre-pass writes each pixel's
// view-space normal and depth, the occlusion pass compares points around each
// pixel with that depth, and a small blur removes the noise. The viewport's mesh
// shader then darkens the soft light by the result.

struct Ssao {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    // xy: target size in pixels, z: radius (world units), w: power
    params: vec4<f32>,
    // x: depth bias (world units), y: 1 for a parallel camera
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ssao: Ssao;
// The pre-pass result (occlusion pass) or the occlusion (blur pass).
@group(1) @binding(0) var input: texture_2d<f32>;

// The parts of `InstanceRaw` (frame.rs) the pre-pass needs.
struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) normal_0: vec4<f32>,
    @location(7) normal_1: vec4<f32>,
    @location(8) normal_2: vec4<f32>,
    // w: 1 for markers (cameras, lights)
    @location(11) emission: vec4<f32>,
};

struct PrepassOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) view_position: vec3<f32>,
    @location(1) view_normal: vec3<f32>,
};

@vertex
fn vs_prepass(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, instance: InstanceIn) -> PrepassOut {
    var out: PrepassOut;
    out.view_normal = vec3<f32>(0.0, 0.0, 1.0);
    // Markers occlude nothing: beyond the far plane, they are clipped.
    if (instance.emission.w > 0.5) {
        out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        out.view_position = vec3<f32>(0.0);
        return out;
    }
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    let normal_matrix = mat3x3<f32>(instance.normal_0.xyz, instance.normal_1.xyz, instance.normal_2.xyz);
    let view_position = ssao.view * model * vec4<f32>(position, 1.0);
    out.clip_position = ssao.proj * view_position;
    out.view_position = view_position.xyz;
    out.view_normal = (ssao.view * vec4<f32>(normal_matrix * normal, 0.0)).xyz;
    return out;
}

// xyz: the view-space normal (facing the viewer), w: the depth (distance along
// the view direction); the clear value 0 marks empty background.
@fragment
fn fs_prepass(in: PrepassOut) -> @location(0) vec4<f32> {
    var n = normalize(in.view_normal);
    var towards_viewer = normalize(-in.view_position);
    if (ssao.extra.y > 0.5) {
        towards_viewer = vec3<f32>(0.0, 0.0, 1.0);
    }
    if (dot(n, towards_viewer) < 0.0) {
        n = -n;
    }
    return vec4<f32>(n, -in.view_position.z);
}

// A triangle covering the whole target.
@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// Points in the unit hemisphere around +Z, nearer ones first.
var<private> KERNEL: array<vec3<f32>, 16> = array<vec3<f32>, 16>(
    vec3<f32>(0.0168, 0.0000, 0.0986),
    vec3<f32>(-0.0222, 0.0203, 0.0990),
    vec3<f32>(0.0037, -0.0427, 0.1057),
    vec3<f32>(0.0356, 0.0465, 0.1179),
    vec3<f32>(-0.0777, -0.0137, 0.1348),
    vec3<f32>(0.0886, -0.0564, 0.1558),
    vec3<f32>(-0.0358, 0.1332, 0.1798),
    vec3<f32>(-0.0821, -0.1582, 0.2058),
    vec3<f32>(0.2131, 0.0778, 0.2328),
    vec3<f32>(-0.2629, 0.1085, 0.2592),
    vec3<f32>(0.1490, -0.3184, 0.2834),
    vec3<f32>(0.1284, 0.4093, 0.3033),
    vec3<f32>(-0.4477, -0.2595, 0.3158),
    vec3<f32>(0.6034, -0.1327, 0.3164),
    vec3<f32>(-0.4206, 0.5982, 0.2963),
    vec3<f32>(-0.1106, -0.8535, 0.2309),
);

// The view-space point under pixel `pixel` at `depth`.
fn view_position(pixel: vec2<f32>, depth: f32) -> vec3<f32> {
    let size = ssao.params.xy;
    let ndc = vec2<f32>(pixel.x / size.x * 2.0 - 1.0, 1.0 - pixel.y / size.y * 2.0);
    let p = ssao.proj;
    if (ssao.extra.y > 0.5) {
        return vec3<f32>((ndc.x - p[3][0]) / p[0][0], (ndc.y - p[3][1]) / p[1][1], -depth);
    }
    return vec3<f32>(ndc.x * depth / p[0][0], ndc.y * depth / p[1][1], -depth);
}

@fragment
fn fs_ssao(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let size = ssao.params.xy;
    let limit = vec2<i32>(size) - vec2<i32>(1);
    let here = textureLoad(input, vec2<i32>(pixel.xy), 0);
    let depth = here.w;
    if (depth <= 0.0) {
        return vec4<f32>(1.0);
    }
    let n = normalize(here.xyz);
    let p = view_position(pixel.xy, depth);

    // A frame around the normal, turned per pixel (the blur hides the pattern).
    let angle = 6.2831853 * fract(52.9829189 * fract(dot(pixel.xy, vec2<f32>(0.06711056, 0.00583715))));
    let random = vec3<f32>(cos(angle), sin(angle), 0.0);
    var tangent = random - n * dot(random, n);
    if (dot(tangent, tangent) < 0.0001) {
        tangent = vec3<f32>(1.0, 0.0, 0.0) - n * n.x;
    }
    tangent = normalize(tangent);
    let bitangent = cross(n, tangent);

    let radius = ssao.params.z;
    var occlusion = 0.0;
    for (var i = 0u; i < 16u; i = i + 1u) {
        let k = KERNEL[i];
        let s = p + (tangent * k.x + bitangent * k.y + n * k.z) * radius;
        let clip = ssao.proj * vec4<f32>(s, 1.0);
        if (clip.w <= 0.0) {
            continue;
        }
        let ndc = clip.xy / clip.w;
        if (abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0) {
            continue;
        }
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        let at = clamp(vec2<i32>(uv * size), vec2<i32>(0), limit);
        let surface = textureLoad(input, at, 0).w;
        if (surface <= 0.0) {
            continue;
        }
        // Occluded where the visible surface is in front of the sample point;
        // surfaces much farther in front (another object) count less.
        let range = smoothstep(0.0, 1.0, radius / max(abs(depth - surface), 0.0001));
        if (surface < -s.z - ssao.extra.x) {
            occlusion = occlusion + range;
        }
    }
    let open = 1.0 - occlusion / 16.0;
    return vec4<f32>(pow(open, ssao.params.w), 0.0, 0.0, 1.0);
}

// A 4 × 4 average of the occlusion.
@fragment
fn fs_blur(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let limit = vec2<i32>(ssao.params.xy) - vec2<i32>(1);
    let centre = vec2<i32>(pixel.xy);
    var sum = 0.0;
    for (var y = -2; y < 2; y = y + 1) {
        for (var x = -2; x < 2; x = x + 1) {
            let at = clamp(centre + vec2<i32>(x, y), vec2<i32>(0), limit);
            sum = sum + textureLoad(input, at, 0).r;
        }
    }
    return vec4<f32>(sum / 16.0, 0.0, 0.0, 1.0);
}
