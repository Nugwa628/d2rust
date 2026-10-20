// The native UI pass: the UI items of a game frame (panels, the HUD, text,
// tooltips, labels, the cursor) drawn at the output resolution, on top of
// the scaled world, instead of in the frame's own pixels.
//
// Every item keeps its place: its image covers the output pixels the
// presentation maps its frame pixels to (`place`: the output position of
// the image's top-left and the output pixels per frame pixel on each
// axis), so its edges sit on output pixels and the mouse lands on it
// exactly where it lands on the frame. Inside the item every frame pixel
// is a solid block and only the output pixel where two of them meet is
// blended (the sharp rule of the presented frame, applied per item).
//
// The cel is read from the frame atlas, shaded through the item's shade
// chain and coloured through the frame palette; index 0 is transparent.
// An item can instead draw a replacement image (HD Text's glyphs): a
// premultiplied RGBA image in the replacement atlas at its own pixels per
// frame pixel, covering the same output pixels, clipped to the same
// drawable part and sampled bilinearly; a tinted one is coloured by one
// palette index through the shade chain, so its colour is the cel's.
// The result is not a colour but what the UI does to the picture under it,
// per output pixel and channel: `out = under · keep + add`, in the
// palette's own (gamma) units. Opaque art sets keep to 0; the
// translucent draw modes keep part of the picture (alpha), add to it
// (additive) or scale it (multiplicative). Composing items in order keeps
// that form, so the whole UI is two images: `cover = 1 − keep` (zero where
// there is no UI, so an untouched target is "no UI") and `add`.
// Buffer layouts: `ui_pass.rs` (all little-endian).

struct Params {
    size: vec2<u32>,   // output width, height
    bins: vec2<u32>,   // bin columns, rows
    item_count: u32,
    map_rows: u32,
    pages: u32,
    rep_pages: u32,     // layers of the replacement atlas
}

struct Item {
    area: vec4<u32>,   // x0, y0, x1, y1 in output pixels, ends exclusive
    texel: vec4<u32>,  // atlas x, y of the image's top-left; page; shade length
    src: vec4<u32>,    // the drawable part of the image, image pixels: x0, y0, x1, y1
    shade: vec4<u32>,  // map rows, used in order
    blend: vec4<u32>,  // kind (0 opaque, 1 alpha, 2 additive, 3 multiplicative), alpha (0..=256), source (0 frame atlas, 1 replacement), 0
    place: vec4<f32>,  // output x, y of the image's top-left; output pixels per image pixel x, y
    rep: vec4<u32>,    // replacement: width, height, its pixels per frame pixel (f32 bits), tint (0x100 | index, or 0)
}

const BIN_SIZE: u32 = 32u;
const BLEND_OPAQUE: u32 = 0u;
const BLEND_ALPHA: u32 = 1u;
const BLEND_ADD: u32 = 2u;
const BLEND_MUL: u32 = 3u;
const SOURCE_REPLACEMENT: u32 = 1u;
const REP_TINTED: u32 = 0x100u;
const REP_INDEXED: u32 = 0x200u;
// `SOLID_SLACK` in `world_view/sharp.rs`.
const SOLID_SLACK: f32 = 0.000244140625;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> items: array<Item>;
@group(0) @binding(2) var<storage, read> bin_ranges: array<u32>;
@group(0) @binding(3) var<storage, read> bin_items: array<u32>;
@group(0) @binding(4) var<storage, read> maps: array<u32>;
@group(0) @binding(5) var atlas: texture_2d_array<u32>;
@group(0) @binding(6) var<storage, read> palette: array<u32, 256>;
@group(0) @binding(7) var cover_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(8) var add_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(9) var rep_atlas: texture_2d_array<f32>;

fn map_byte(row: u32, b: u32) -> u32 {
    return (maps[(row << 6u) + (b >> 2u)] >> ((b & 3u) << 3u)) & 0xffu;
}

// One image pixel of item `it`, premultiplied: its colour and 1, or all 0
// where it is transparent or outside the drawable part.
fn texel(it: Item, x: i32, y: i32) -> vec4<f32> {
    if x < i32(it.src.x) || y < i32(it.src.y) || x >= i32(it.src.z) || y >= i32(it.src.w) {
        return vec4<f32>(0.0);
    }
    let t = vec2<u32>(it.texel.x + u32(x), it.texel.y + u32(y));
    let index = textureLoad(atlas, t, it.texel.z, 0).r;
    if index == 0u {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(colour(it, index), 1.0);
}

// Palette index `index` through item `it`'s shade chain, as a colour.
fn colour(it: Item, index: u32) -> vec3<f32> {
    var s = index;
    for (var j = 0u; j < it.texel.w; j++) {
        s = map_byte(it.shade[j], s);
    }
    let c = palette[s];
    return vec3<f32>(
        f32(c & 0xffu),
        f32((c >> 8u) & 0xffu),
        f32((c >> 16u) & 0xffu),
    ) / 255.0;
}

// One pixel of item `it`'s replacement image, premultiplied; 0 outside it.
fn rep_texel(it: Item, x: i32, y: i32) -> vec4<f32> {
    if x < 0 || y < 0 || x >= i32(it.rep.x) || y >= i32(it.rep.y) || it.texel.z >= params.rep_pages {
        return vec4<f32>(0.0);
    }
    let t = vec2<i32>(i32(it.texel.x) + x, i32(it.texel.y) + y);
    let c = textureLoad(rep_atlas, t, it.texel.z, 0);
    if (it.rep.w & REP_INDEXED) != 0u {
        return vec4<f32>(colour(it, u32(round(c.r * 255.0))) * c.a, c.a);
    }
    return c;
}

// Item `it`'s replacement at image position `u` (frame pixels): clipped to
// the drawable part, sampled bilinearly in its own pixels, tinted.
fn rep_pixel(it: Item, u: vec2<f32>) -> vec4<f32> {
    let f = floor(u);
    if f.x < f32(it.src.x) || f.y < f32(it.src.y) || f.x >= f32(it.src.z) || f.y >= f32(it.src.w) {
        return vec4<f32>(0.0);
    }
    let r = u * bitcast<f32>(it.rep.z) - 0.5;
    let b = floor(r);
    let w = r - b;
    let x = i32(b.x);
    let y = i32(b.y);
    let top = mix(rep_texel(it, x, y), rep_texel(it, x + 1, y), w.x);
    let bottom = mix(rep_texel(it, x, y + 1), rep_texel(it, x + 1, y + 1), w.x);
    var c = mix(top, bottom, w.y);
    if (it.rep.w & REP_TINTED) != 0u {
        c = vec4<f32>(c.rgb * colour(it, it.rep.w & 0xffu), c.a);
    }
    return c;
}

// The sharp rule on one axis: `u` is the output pixel's centre in image
// pixels, `s` the output pixels per image pixel. Returns the left image
// pixel to blend and the right one's weight (0 inside a solid block).
fn sharp_axis(u: f32, s: f32) -> vec2<f32> {
    let scale = max(ceil(s), 1.0);
    let base = floor(u);
    let d = u - base - 0.5;
    let solid = 0.5 - 0.5 / scale + SOLID_SLACK;
    let p = base + (d - clamp(d, -solid, solid)) * scale + 0.5;
    let left = floor(p - 0.5);
    return vec2<f32>(left, p - 0.5 - left);
}

@compute @workgroup_size(16, 16, 1)
fn draw_ui(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.x;
    let py = gid.y;
    if px >= params.size.x || py >= params.size.y {
        return;
    }
    let bin = (py / BIN_SIZE) * params.bins.x + px / BIN_SIZE;
    var keep = vec3<f32>(1.0);
    var plus = vec3<f32>(0.0);
    for (var k = bin_ranges[bin]; k < bin_ranges[bin + 1u]; k++) {
        let it = items[bin_items[k]];
        if px < it.area.x || px >= it.area.z || py < it.area.y || py >= it.area.w {
            continue;
        }
        let u = (vec2<f32>(f32(px), f32(py)) + 0.5 - it.place.xy) / it.place.zw;
        var c: vec4<f32>;
        if it.blend.z == SOURCE_REPLACEMENT {
            c = rep_pixel(it, u);
        } else {
            let ax = sharp_axis(u.x, it.place.z);
            let ay = sharp_axis(u.y, it.place.w);
            let x = i32(ax.x);
            let y = i32(ay.x);
            let top = mix(texel(it, x, y), texel(it, x + 1, y), ax.y);
            let bottom = mix(texel(it, x, y + 1), texel(it, x + 1, y + 1), ax.y);
            c = mix(top, bottom, ay.y);
        }
        if c.a <= 0.0 {
            continue;
        }
        var m = vec3<f32>(1.0 - c.a);
        var a = c.rgb;
        let kind = it.blend.x;
        if kind == BLEND_ALPHA {
            let alpha = f32(it.blend.y) / 256.0;
            m = vec3<f32>(1.0 - alpha * c.a);
            a = c.rgb * alpha;
        } else if kind == BLEND_ADD {
            m = vec3<f32>(1.0);
        } else if kind == BLEND_MUL {
            m = vec3<f32>(1.0 - c.a) + c.rgb;
            a = vec3<f32>(0.0);
        }
        keep = keep * m;
        plus = plus * m + a;
    }
    let at = vec2<i32>(i32(px), i32(py));
    textureStore(cover_out, at, vec4<f32>(clamp(1.0 - keep, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
    textureStore(add_out, at, vec4<f32>(clamp(plus, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
