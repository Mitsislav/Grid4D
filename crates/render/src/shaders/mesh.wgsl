// Grid4D — objects: shaded surfaces (`vs_main` / `fs_main`) and lines along
// their edges (`vs_line` / `fs_line`).
// Instanced: one draw call per mesh key, the instances carry the placement.
// Lighting is done in linear space; the result is sRGB-encoded here when the
// surface format is not an *Srgb format (egui prefers a gamma framebuffer).

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz: eye position, w: 1.0 if this shader must encode to sRGB itself.
    eye: vec4<f32>,
    // xyz: direction towards the key light, w: grid plane + 8 for a parallel camera.
    light_dir: vec4<f32>,
    // x: minor spacing, y: major spacing, z: fade distance, w: half extent.
    grid: vec4<f32>,
    // xy: target size in pixels, z: the fill (0 lit, 1 quick, 2 constant),
    // w: 1 for lines in the object's colour.
    viewport: vec4<f32>,
    // The component overlay's colours and sizes (overlay.wgsl).
    wire: vec4<f32>,
    selected: vec4<f32>,
    hover: vec4<f32>,
    sizes: vec4<f32>,
    alphas: vec4<f32>,
    // x: scene lights to use (0: the default light), y: ambient with scene lights,
    // z: 1 to darken the ambient light by the view's occlusion.
    lighting: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

// Must match `LightRaw` in frame.rs.
struct Light {
    // xyz: world position, w: type (0 omni, 1 spot, 2 infinite).
    position: vec4<f32>,
    // xyz: the direction it shines in, w: cosine of half the outer angle.
    direction: vec4<f32>,
    // rgb: colour × intensity, w: cosine of half the inner angle.
    color: vec4<f32>,
    // x: falloff (0 none, 1 linear, 2 inverse square), y: radius.
    falloff: vec4<f32>,
};

@group(0) @binding(1) var<storage, read> lights: array<Light>;

// Must match `ShadowUniform` in effects.rs.
struct Shadow {
    // One view-projection per face (+X, −X, +Y, −Y, +Z, −Z for an Omni light).
    faces: array<mat4x4<f32>, 6>,
    // x: the casting light's index, y: faces (0: no shadow), z: map size, w: normal offset
    params: vec4<f32>,
    // x: filter radius in texels, y: depth bias
    blur: vec4<f32>,
};

@group(0) @binding(2) var<uniform> shadow: Shadow;
@group(0) @binding(3) var shadow_map: texture_depth_2d_array;
@group(0) @binding(4) var shadow_sampler: sampler_comparison;

// The view's ambient occlusion (1: open), read where `lighting.z` says so.
@group(1) @binding(0) var ao_map: texture_2d<f32>;
@group(1) @binding(1) var ao_sampler: sampler;

// The object's material's textures. Must match `MaterialUniform` in
// textures.rs. A channel without a picture has strength 0 (its value stays)
// and a white picture bound.
struct ChannelSettings {
    // xy: tiling, zw: offset.
    place: vec4<f32>,
    // xy: the range the picture's 0 – 1 maps to, z: strength, w: flags (1: a
    // picture, 2: a grey one, which multiplies a colour).
    range: vec4<f32>,
};

struct MaterialUniform {
    // x: emission strength, y: opacity, z: opacity threshold, w: specular.
    header: vec4<f32>,
    // In `Channel::ALL` order.
    channels: array<ChannelSettings, 10>,
};

@group(2) @binding(0) var<uniform> material: MaterialUniform;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var map_base_color: texture_2d<f32>;
@group(2) @binding(3) var map_metallic: texture_2d<f32>;
@group(2) @binding(4) var map_roughness: texture_2d<f32>;
@group(2) @binding(5) var map_specular: texture_2d<f32>;
@group(2) @binding(6) var map_normal: texture_2d<f32>;
@group(2) @binding(7) var map_bump: texture_2d<f32>;
@group(2) @binding(8) var map_emission: texture_2d<f32>;
@group(2) @binding(9) var map_opacity: texture_2d<f32>;
@group(2) @binding(10) var map_transmission: texture_2d<f32>;
@group(2) @binding(11) var map_clearcoat: texture_2d<f32>;

const BASE_COLOR: u32 = 0u;
const METALLIC: u32 = 1u;
const ROUGHNESS: u32 = 2u;
const SPECULAR: u32 = 3u;
const EMISSION: u32 = 6u;
const OPACITY: u32 = 7u;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(12) uv: vec2<f32>,
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
    // x: selected, y: unlit, z: roughness, w: metallic
    @location(10) params: vec4<f32>,
    // rgb: light the surface gives off
    @location(11) emission: vec4<f32>,
    // The Material tag's change to texture coordinates: a 2 × 2 matrix
    // (columns), then its shift (xy) and z: 1 for a Camera projection.
    @location(13) uv_matrix: vec4<f32>,
    @location(14) uv_offset: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) emission: vec3<f32>,
    // The texture coordinates (the tag's change applied; not used by a
    // Camera projection).
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) uv_matrix: vec4<f32>,
    @location(7) @interpolate(flat) uv_offset: vec4<f32>,
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
    out.emission = instance.emission.rgb;
    out.uv = tag_uv(vertex.uv, instance.uv_matrix, instance.uv_offset);
    out.uv_matrix = instance.uv_matrix;
    out.uv_offset = instance.uv_offset;
    return out;
}

// A Material tag's change to coordinates `uv`.
fn tag_uv(uv: vec2<f32>, m: vec4<f32>, offset: vec4<f32>) -> vec2<f32> {
    return mat2x2<f32>(m.xy, m.zw) * uv + offset.xy;
}

// Where channel `index`'s picture is read: the surface's coordinates through
// the channel's tiling and offset.
fn channel_uv(index: u32, uv: vec2<f32>) -> vec2<f32> {
    let place = material.channels[index].place;
    return uv * place.xy + place.zw;
}

// A number channel: its value, or towards the picture's red mapped onto its range.
fn scalar_channel(index: u32, value: f32, picture: vec4<f32>) -> f32 {
    let r = material.channels[index].range;
    return mix(value, mix(r.x, r.y, picture.r), r.z);
}

// A colour channel: its value, or towards the picture (a grey picture
// multiplies the value).
fn colour_channel(index: u32, value: vec3<f32>, picture: vec4<f32>) -> vec3<f32> {
    let r = material.channels[index].range;
    let grey = (u32(r.w + 0.5) & 2u) != 0u;
    let textured = select(picture.rgb, value * picture.rgb, grey);
    return mix(value, textured, r.z);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cutoff = c <= vec3<f32>(0.0031308);
    let lower = c * 12.92;
    let higher = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, cutoff);
}

const SELECTION_RIM: vec3<f32> = vec3<f32>(1.0, 0.42, 0.06);
// The default light's strength.
const DEFAULT_LIGHT: f32 = 0.62;
// How much the ambient occlusion darkens the lights' direct light.
const AO_ON_DIRECT: f32 = 0.6;

// What one light reflects towards the viewer: diffuse, and a highlight whose size
// follows the roughness (Blinn-Phong with a normalised exponent, Schlick Fresnel).
fn reflected(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>, diffuse: vec3<f32>, f0: vec3<f32>, shininess: f32) -> vec3<f32> {
    let n_dot_l = dot(n, l);
    if (n_dot_l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    let highlight = fresnel * ((shininess + 8.0) / 8.0) * pow(max(dot(n, h), 0.0), shininess);
    return (diffuse + highlight) * radiance * n_dot_l;
}

// The direction towards `light` from `p` (xyz) and how much of it arrives (w):
// its cone and falloff.
fn incoming(light: Light, p: vec3<f32>) -> vec4<f32> {
    let kind = u32(light.position.w + 0.5);
    if (kind == 2u) {
        // Infinite: a direction only.
        return vec4<f32>(-light.direction.xyz, 1.0);
    }
    let to_light = light.position.xyz - p;
    let distance = max(length(to_light), 0.001);
    let l = to_light / distance;
    var amount = 1.0;
    let falloff = u32(light.falloff.x + 0.5);
    let radius = max(light.falloff.y, 0.01);
    if (falloff == 1u) {
        amount = clamp(1.0 - distance / radius, 0.0, 1.0);
    } else if (falloff == 2u) {
        let ratio = radius / distance;
        amount = min(ratio * ratio, 10000.0);
    }
    if (kind == 1u) {
        let cos_angle = dot(-l, light.direction.xyz);
        let cos_outer = light.direction.w;
        let cos_inner = light.color.w;
        if (cos_inner > cos_outer + 0.0001) {
            amount = amount * smoothstep(cos_outer, cos_inner, cos_angle);
        } else if (cos_angle < cos_outer) {
            amount = 0.0;
        }
    }
    return vec4<f32>(l, amount);
}

// How much of the shadow-casting light reaches `p` (normal `n`): 1 lit, 0 in
// shadow; a filtered compare against the shadow map, softer for soft shadows.
fn lit_by_key_light(p: vec3<f32>, n: vec3<f32>, light_position: vec3<f32>) -> f32 {
    let faces = u32(shadow.params.y + 0.5);
    if (faces == 0u) {
        return 1.0;
    }
    var face = 0u;
    if (faces == 6u) {
        let d = p - light_position;
        let a = abs(d);
        if (a.x >= a.y && a.x >= a.z) {
            face = select(1u, 0u, d.x > 0.0);
        } else if (a.y >= a.z) {
            face = select(3u, 2u, d.y > 0.0);
        } else {
            face = select(5u, 4u, d.z > 0.0);
        }
    }
    // Lifted off the surface by about a texel, against self-shadowing.
    let lifted = p + n * shadow.params.w;
    let clip = shadow.faces[face] * vec4<f32>(lifted, 1.0);
    if (clip.w <= 0.0) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    if (abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let step = shadow.blur.x * 0.5 / shadow.params.z;
    let depth = ndc.z - shadow.blur.y;
    var lit = 0.0;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            let at = uv + vec2<f32>(f32(x), f32(y)) * step;
            lit = lit + textureSampleCompareLevel(shadow_map, shadow_sampler, at, face, depth);
        }
    }
    return lit / 25.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The material's pictures, read first (in uniform control flow). A
    // Camera projection reads them where the view shows the point.
    var uv = in.uv;
    if (in.uv_offset.z > 0.5) {
        uv = tag_uv(in.clip_position.xy / camera.viewport.xy, in.uv_matrix, in.uv_offset);
    }
    let base_picture = textureSample(map_base_color, material_sampler, channel_uv(BASE_COLOR, uv));
    let metallic_picture = textureSample(map_metallic, material_sampler, channel_uv(METALLIC, uv));
    let roughness_picture = textureSample(map_roughness, material_sampler, channel_uv(ROUGHNESS, uv));
    let specular_picture = textureSample(map_specular, material_sampler, channel_uv(SPECULAR, uv));
    let emission_picture = textureSample(map_emission, material_sampler, channel_uv(EMISSION, uv));
    let opacity_picture = textureSample(map_opacity, material_sampler, channel_uv(OPACITY, uv));
    // Cut-out: below the threshold the surface is not there.
    if (scalar_channel(OPACITY, material.header.y, opacity_picture) < material.header.z) {
        discard;
    }

    let view_dir = normalize(camera.eye.xyz - in.world_position);
    var n = normalize(in.world_normal);
    // Two-sided lighting that does not depend on winding (planes seen from below).
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }

    // The material: metals reflect their colour and have no diffuse part; a
    // dielectric reflects 8 % × its specular (4 % at the usual 0.5).
    let base = colour_channel(BASE_COLOR, in.color.rgb, base_picture);
    let roughness = clamp(scalar_channel(ROUGHNESS, in.params.z, roughness_picture), 0.02, 1.0);
    let metallic = clamp(scalar_channel(METALLIC, in.params.w, metallic_picture), 0.0, 1.0);
    let specular = clamp(scalar_channel(SPECULAR, material.header.w, specular_picture), 0.0, 1.0);
    let diffuse = base * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.08 * specular), base, metallic);
    let shininess = clamp(2.0 / pow(roughness, 4.0) - 2.0, 1.0, 2048.0);

    // Soft light from all around (stronger from above), and its reflection: a dim
    // studio, brighter overhead, that mostly metals show (smoother ones more).
    let facing = max(dot(n, view_dir), 0.0);
    let sky = 0.5 + 0.5 * n.y;
    let ambient = diffuse * (0.08 + 0.22 * sky + 0.16 * facing);
    let mirrored = reflect(-view_dir, n);
    let studio = f0 * (0.16 + 0.34 * (0.5 + 0.5 * mirrored.y)) * (1.0 - 0.5 * roughness);

    // The view's ambient occlusion darkens the soft light, and the lights' own
    // light in part, so it shows in scenes lit mostly by their lights.
    var open = 1.0;
    if (camera.lighting.z > 0.5) {
        open = textureSampleLevel(ao_map, ao_sampler, in.clip_position.xy / camera.viewport.xy, 0.0).r;
    }
    let direct_open = mix(1.0, open, AO_ON_DIRECT);

    var soft = studio + ambient;
    var direct = vec3<f32>(0.0);
    let count = u32(camera.lighting.x + 0.5);
    if (count == 0u) {
        // The default light, from over the viewer's shoulder.
        let light = normalize(camera.light_dir.xyz);
        direct = reflected(n, view_dir, light, vec3<f32>(DEFAULT_LIGHT), diffuse, f0, shininess);
    } else {
        soft = studio + ambient * camera.lighting.y;
        let caster = u32(shadow.params.x + 0.5);
        for (var i = 0u; i < count; i = i + 1u) {
            let light = lights[i];
            let arriving = incoming(light, in.world_position);
            var amount = arriving.w;
            if (amount > 0.0 && i == caster) {
                amount = amount * lit_by_key_light(in.world_position, n, light.position.xyz);
            }
            if (amount > 0.0) {
                direct = direct + reflected(n, view_dir, arriving.xyz, light.color.rgb * amount, diffuse, f0, shininess);
            }
        }
    }
    var lit = soft * open + direct * direct_open;
    if (u32(camera.viewport.z + 0.5) == 2u) {
        // Constant Shading: the colour itself.
        lit = base;
    }
    // Emission: a grey picture scales the light given off, a coloured one is it.
    let emission_range = material.channels[EMISSION].range;
    let grey_emission = (u32(emission_range.w + 0.5) & 2u) != 0u;
    let pictured = select(emission_picture.rgb * material.header.x, in.emission * emission_picture.rgb, grey_emission);
    let emission = mix(in.emission, pictured, emission_range.z);
    var rgb = mix(lit, base, in.params.y) + emission;

    // Selected objects get a subtle orange Fresnel rim (the UI adds bbox corner brackets).
    let rim = pow(1.0 - facing, 4.0);
    rgb = mix(rgb, SELECTION_RIM, clamp(rim * 0.8, 0.0, 1.0) * in.params.x);

    if (camera.eye.w > 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}

struct LineOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// Lines are pulled towards the viewer by this share of their distance, so an
// edge wins over its own polygons.
const LINE_PULL: f32 = 0.002;

@vertex
fn vs_line(@location(0) position: vec3<f32>, instance: InstanceIn) -> LineOut {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    let p = (model * vec4<f32>(position, 1.0)).xyz;
    var q = p + (camera.eye.xyz - p) * LINE_PULL;
    // A parallel camera's eye is far away: pull a fixed distance instead
    // (light_dir.w: + 8 for a parallel camera).
    if ((u32(camera.light_dir.w) & 8u) != 0u) {
        q = p + normalize(camera.eye.xyz - p) * LINE_PULL * 1000.0;
    }

    var out: LineOut;
    out.clip_position = camera.view_proj * vec4<f32>(q, 1.0);
    // Over surfaces a dark shade of the object's colour; alone, the colour
    // itself — as for a spline's curve (params.y: unlit), which has no surface.
    var color = instance.color.rgb * 0.22;
    if (camera.viewport.w > 0.5 || instance.params.y > 0.5) {
        color = instance.color.rgb;
    }
    if (instance.params.x > 0.5) {
        color = SELECTION_RIM;
    }
    out.color = color;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    var rgb = in.color;
    if (camera.eye.w > 0.5) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
