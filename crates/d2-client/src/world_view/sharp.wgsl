// Sharp scaling for the presented game frame: every game pixel stays a
// solid block, and only the screen pixel where two game pixels meet is
// blended. That keeps pixel widths even at a fractional scale without the
// blur of plain linear filtering. At a whole-number scale every screen
// pixel falls inside one game pixel, so the result is exactly the nearest
// game pixel. One texture read with a linear sampler; `sharp.rs` holds the
// same rule in Rust (`sharp_offset`) with its tests.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var frame: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var frame_sampler: sampler;
// xy: screen pixels per game pixel on each axis (at least 1).
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> scale: vec4<f32>;
// The part of the image shown: xy its uv origin, zw its uv size.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> uv_rect: vec4<f32>;
// The tick before's image and its uv origin (xy) and size (zw; zero: no
// fill): read where `uv` is outside the image (the smooth camera's world
// layer, `smooth.rs`).
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var prev: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> prev_uv: vec4<f32>;

// `SOLID_SLACK` in sharp.rs.
const SOLID_SLACK: f32 = 0.000244140625;

// Defaults are exactly identity; only the presented colour changes.
fn picture_options(rgb: vec3<f32>) -> vec3<f32> {
    return clamp(pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(scale.z + 1.0)) * (scale.w + 1.0), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(frame));
    let uv = uv_rect.xy + mesh.uv * uv_rect.zw;
    let texel = uv * size;
    let base = floor(texel);
    // Distance from the game pixel's centre; inside the solid region the
    // sample snaps to the centre, within half a screen pixel of an edge it
    // ramps to the boundary with the neighbour.
    let d = fract(texel) - 0.5;
    let prescale = ceil(scale.xy);
    let solid = 0.5 - 0.5 / prescale + SOLID_SLACK;
    let f = (d - clamp(d, -solid, solid)) * prescale + 0.5;
    let c_here = textureSample(frame, frame_sampler, (base + f) / size);
    // Both reads stay in uniform control flow; the choice is per pixel.
    let puv = prev_uv.xy + mesh.uv * prev_uv.zw;
    let ptexel = puv * size;
    let pd = fract(ptexel) - 0.5;
    let pf = (pd - clamp(pd, -solid, solid)) * prescale + 0.5;
    let c_prev = textureSample(prev, frame_sampler, (floor(ptexel) + pf) / size);
    let outside = any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0));
    let c = select(c_here, c_prev, prev_uv.z > 0.0 && outside);
    // A transparent texel is all zero, so where an opaque pixel meets a
    // transparent one the filter gives a premultiplied colour; divide it
    // back for straight-alpha blending (a no-op where alpha is 1).
    return vec4<f32>(picture_options(c.rgb / max(c.a, 1e-6)), c.a);
}
