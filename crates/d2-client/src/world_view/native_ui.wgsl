// The presented picture with the UI drawn at the output resolution
// (`native_ui.rs`). The picture is sharp-scaled exactly as `sharp.wgsl`
// scales the frame: the frame image itself (the scene, without its UI), or
// between ticks the smooth camera's world layer (shifted) under its screen
// layer. The native UI pass's two images are then read at this output
// pixel and applied in the palette's own units: `out = picture ·
// (1 − cover) + add`. Where there is no UI both are zero and the picture
// passes through unchanged.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var frame: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var frame_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var world: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var screen: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var cover: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var add: texture_2d<f32>;
// xy: screen pixels per game pixel on each axis (at least 1).
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<uniform> scale: vec4<f32>;
// The world layer's uv origin (xy) and size (zw).
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var<uniform> world_uv: vec4<f32>;
// x: 1 shows the world and screen layers, 0 the frame.
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var<uniform> flags: vec4<f32>;
// The tick before's world layer and its uv origin (xy) and size (zw;
// zero: no fill). It fills the strip the shifted world layer uncovers.
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var prev: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var<uniform> prev_uv: vec4<f32>;

// `SOLID_SLACK` in sharp.rs.
const SOLID_SLACK: f32 = 0.000244140625;

// `sharp.wgsl`'s sample, premultiplied (a transparent texel is all zero).
fn sharp(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let texel = uv * size;
    let base = floor(texel);
    let d = fract(texel) - 0.5;
    let prescale = ceil(scale.xy);
    let solid = 0.5 - 0.5 / prescale + SOLID_SLACK;
    let f = (d - clamp(d, -solid, solid)) * prescale + 0.5;
    return textureSampleLevel(t, frame_sampler, (base + f) / size, 0.0);
}

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// Defaults are exactly identity; only the presented colour changes.
fn picture_options(rgb: vec3<f32>) -> vec3<f32> {
    return clamp(pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(scale.z + 1.0)) * (scale.w + 1.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var picture: vec3<f32>;
    if flags.x > 0.5 {
        let wuv = world_uv.xy + mesh.uv * world_uv.zw;
        var w: vec4<f32>;
        if prev_uv.z > 0.0 && (any(wuv < vec2<f32>(0.0)) || any(wuv > vec2<f32>(1.0))) {
            w = sharp(prev, prev_uv.xy + mesh.uv * prev_uv.zw);
        } else {
            w = sharp(world, wuv);
        }
        let s = sharp(screen, mesh.uv);
        picture = s.rgb + w.rgb * (1.0 - s.a);
    } else {
        picture = sharp(frame, mesh.uv).rgb;
    }
    let size = vec2<i32>(textureDimensions(cover));
    let at = clamp(vec2<i32>(floor(mesh.uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
    let c = textureLoad(cover, at, 0).rgb;
    let a = textureLoad(add, at, 0).rgb;
    if all(c == vec3<f32>(0.0)) && all(a == vec3<f32>(0.0)) {
        return vec4<f32>(picture_options(picture), 1.0);
    }
    let out = to_gamma(picture) * (1.0 - c) + a;
    return vec4<f32>(picture_options(to_linear(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)))), 1.0);
}
