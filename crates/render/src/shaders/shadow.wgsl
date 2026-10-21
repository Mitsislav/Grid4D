// Grid4D — the key light's shadow map: depth only, seen from the light, one face
// at a time (a dynamic offset picks the face's matrix).

struct Face {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> face: Face;

// The parts of `InstanceRaw` (frame.rs) this pass needs.
struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    // w: 1 for markers (cameras, lights)
    @location(11) emission: vec4<f32>,
};

@vertex
fn vs_shadow(@location(0) position: vec3<f32>, instance: InstanceIn) -> @builtin(position) vec4<f32> {
    // Markers cast no shadow: beyond the far plane, they are clipped.
    if (instance.emission.w > 0.5) {
        return vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    return face.view_proj * model * vec4<f32>(position, 1.0);
}
