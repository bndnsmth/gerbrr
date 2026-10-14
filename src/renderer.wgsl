struct Camera { x: vec4<f32>, y: vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex fn geometry_vertex(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(dot(camera.x.xyz, vec3<f32>(p, 1.0)), dot(camera.y.xyz, vec3<f32>(p, 1.0)), 0.0, 1.0);
}
@fragment fn dark() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }
@fragment fn clear() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }

struct Quad { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn quad(@builtin(vertex_index) i: u32) -> Quad {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: Quad;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(0) @binding(2) var<uniform> tint: vec4<f32>;
@fragment fn composite(in: Quad) -> @location(0) vec4<f32> {
    let a = textureSample(image, image_sampler, in.uv).r * tint.a;
    return vec4<f32>(tint.rgb * a, a);
}
@fragment fn present(in: Quad) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv);
}
