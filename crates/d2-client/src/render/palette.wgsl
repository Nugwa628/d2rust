// Spec: specs/render/map-preview.md (Palette shading)
// Draws an 8-bit index image through a 256-color palette. Index 0 is
// transparent. Both textures are read with textureLoad (no filtering), so
// the output is exactly palette[index].

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var indices: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var palette: texture_2d<f32>;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(indices);
    // uv (0,0) is the image's top-left; clamp guards the far edge.
    let texel = min(vec2<u32>(mesh.uv * vec2<f32>(size)), size - vec2<u32>(1u, 1u));
    let index = textureLoad(indices, texel, 0).r;
    if index == 0u {
        discard;
    }
    // The palette texture is sRGB, so this is the linear color; the sRGB
    // render target encodes it back to the exact palette bytes.
    return vec4<f32>(textureLoad(palette, vec2<u32>(index, 0u), 0).rgb, 1.0);
}
