struct Push {
    view_projection: mat4x4<f32>,
    outer: vec4<f32>,
    inner: vec4<f32>,
    params: vec4<f32>,
}
var<push_constant> push: Push;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) relative_position: vec3<f32>,
}

@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> VertexOut {
    var output: VertexOut;
    output.position = push.view_projection * vec4<f32>(position, 1.0);
    output.normal = normal;
    output.color = color;
    output.relative_position = position;
    if dot(normal, normal) < 0.01 {
        output.position.z = output.position.w * 1e-15;
    }
    return output;
}

fn coverage(p: vec3<f32>, bounds: vec4<f32>) -> f32 {
    if bounds.w <= 0.0 { return 0.0; }
    let d = abs(p-bounds.xyz);
    return 1.0-smoothstep(bounds.w*0.75, bounds.w*0.95, max(d.x,max(d.y,d.z)));
}

@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    if dot(input.normal, input.normal) < 0.01 { return vec4<f32>(input.color, 1.0); }
    var outer = 1.0;
    if push.params.x > 0.5 { outer = coverage(input.relative_position,push.outer)*push.params.y; }
    let inner = coverage(input.relative_position,push.inner)*push.params.y;
    let bayer = array<f32,16>(0.0,8.0,2.0,10.0,12.0,4.0,14.0,6.0,3.0,11.0,1.0,9.0,15.0,7.0,13.0,5.0);
    let pixel = vec2<u32>(input.position.xy);
    let threshold = (bayer[(pixel.y%4u)*4u+pixel.x%4u]+0.5)/16.0;
    if threshold >= outer || threshold < inner { discard; }
    let n = normalize(input.normal);
    let sun = normalize(vec3<f32>(-0.6, 0.5, 0.8));
    let diffuse = max(dot(n, sun), 0.0);
    let rock = input.color;
    return vec4<f32>(rock * (0.085 + 0.915 * diffuse), 1.0);
}
