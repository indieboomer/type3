struct Push { view_projection: mat4x4<f32> }
var<push_constant> push: Push;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
}

@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> VertexOut {
    var output: VertexOut;
    output.position = push.view_projection * vec4<f32>(position, 1.0);
    output.normal = normal;
    return output;
}

@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(input.normal);
    let sun = normalize(vec3<f32>(-0.6, 0.5, 0.8));
    let diffuse = max(dot(n, sun), 0.0);
    let rock = vec3<f32>(0.26, 0.34, 0.40);
    return vec4<f32>(rock * (0.045 + 0.955 * diffuse), 1.0);
}
