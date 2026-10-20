// GPU compute compositor of the game frame. Same formulas as the CPU
// reference (`scene::cpu::compose_binned_frame`): per pixel, start from the
// base (the previous frame, which persists between frames; 0 in rows below
// `clear_rows`), walk the pixel's bin list in order; index 0 of a cel leaves
// the pixel unchanged; otherwise the shade chain maps the index, then the
// item's light gradient map of the pixel if any, and the blend op combines
// it with the pixel's value (`IndexTable` reads row = destination, column =
// source, `IndexTableSrcRow` the transpose); `clear_after` sets the result
// to 0. Integers only; frames are read with textureLoad.
//
// Alongside the frame it keeps two presentation layers for smooth
// scrolling between ticks: the world layer (the same compose with the
// screen-layer items left out) and which pixels still depend on the screen
// layer (UI, the local player) after subsequent translucent blends. They
// never change `indices`.
//
// With the native UI pass on (`params.native_ui`; `ui_pass.wgsl` draws the
// UI at the output resolution) the UI items are kept out of what is
// presented: `rgba` is the scene (the frame without the UI items) and the
// screen layer is the scene's own screen part (the local player, the
// fade). `indices` stays the whole frame either way, UI included: it is
// the next frame's base and what the frame hashes read.
// Buffer layouts: `pack.rs` (all little-endian).

struct Params {
    size: vec2<u32>,   // view width, height
    bins: vec2<u32>,   // bin columns, rows
    item_count: u32,
    map_rows: u32,
    pages: u32,
    clear_rows: u32,   // view rows 0..clear_rows start at 0 (StartDraw clear)
    clear_after: u32,  // 1: every pixel 0 after drawing (ClearScreen)
    native_ui: u32,    // 1: the UI items are presented by the native UI pass
    base_wide: u32,    // 1: base is the previous frame's index output, one pixel per word
    pad2: u32,
}

struct Item {
    area: vec4<u32>,   // x0, y0, x1, y1 in view pixels, ends exclusive
    texel: vec4<u32>,  // atlas x, y of the area's top-left; page; shade length
    shade: vec4<u32>,  // map rows, used in order
    blend: vec4<u32>,  // op (0 opaque, 1 index table, 2 index table src row), table base row, layer (0 world, 1 screen, 2 UI), 0
    light: vec4<u32>,  // gradient kind (0 none, 1 wall, 2 RLE floor), light map 0 row, corners c0..c3 (bytes), dx | dy << 16
}

const BIN_SIZE: u32 = 32u;
const BLEND_OPAQUE: u32 = 0u;
const BLEND_INDEX_TABLE_SRC_ROW: u32 = 2u;
const GRADIENT_WALL: u32 = 1u;
const GRADIENT_RLE_FLOOR: u32 = 2u;
const GRADIENT_ISO_FLOOR: u32 = 3u;
const LAYER_WORLD: u32 = 0u;
const LAYER_UI: u32 = 2u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> items: array<Item>;
@group(0) @binding(2) var<storage, read> bin_ranges: array<u32>;
@group(0) @binding(3) var<storage, read> bin_items: array<u32>;
@group(0) @binding(4) var<storage, read> maps: array<u32>;
@group(0) @binding(5) var atlas: texture_2d_array<u32>;
@group(0) @binding(6) var<storage, read_write> indices: array<u32>;
@group(0) @binding(7) var<storage, read> palette: array<u32, 256>;
@group(0) @binding(8) var<storage, read_write> rgba: array<u32>;
@group(0) @binding(9) var<storage, read> base: array<u32>;
@group(0) @binding(10) var<storage, read_write> layers: array<u32>;
@group(0) @binding(11) var<storage, read_write> world_rgba: array<u32>;
@group(0) @binding(12) var<storage, read_write> screen_rgba: array<u32>;

// Byte `i` of the base framebuffer: four pixels per word, little-endian;
// with `base_wide` one pixel per word (the previous frame's `indices`).
fn base_byte(i: u32) -> u32 {
    if params.base_wide != 0u {
        return base[i] & 0xffu;
    }
    return (base[i >> 2u] >> ((i & 3u) << 3u)) & 0xffu;
}

// Byte `b` of map row `row`: rows are 256 bytes, four per word, little-endian.
fn map_byte(row: u32, b: u32) -> u32 {
    return (maps[(row << 6u) + (b >> 2u)] >> ((b & 3u) << 3u)) & 0xffu;
}

fn shade(it: Item, index: u32) -> u32 {
    var s = index;
    for (var j = 0u; j < it.texel.w; j++) {
        s = map_byte(it.shade[j], s);
    }
    return s;
}

fn corner(it: Item, i: u32) -> i32 {
    return i32((it.light.z >> (8u * i)) & 0xffu);
}

// Light map row of area pixel (ax, ay) of a gradient-lit item: block
// column x, row r; a_r, b_r the row's left and right levels; G[a][b][x].
fn gradient_row(it: Item, ax: u32, ay: u32, run_cut: i32) -> u32 {
    var scale = 16;
    var shift = 7u;
    if it.light.x == GRADIENT_WALL {
        scale = 32;
        shift = 8u;
    }
    var x = i32(ax + (it.light.w & 0xffffu));
    var r = i32(ay + (it.light.w >> 16u));
    if it.light.x == GRADIENT_ISO_FLOOR {
        // An iso floor tile: the light row and column restart at the first
        // drawn row / at the row's own first column s_r.
        let cut_x = i32(it.light.w & 0xffffu);
        let cut_y = i32(it.light.w >> 16u);
        x = x - max(cut_x - abs(14 - 2 * r), 0);
        r = r - cut_y;
    }
    x = x - run_cut;
    let a = (scale * corner(it, 0u) + r * (corner(it, 3u) - corner(it, 0u))) >> shift;
    let b = (scale * corner(it, 1u) + r * (corner(it, 2u) - corner(it, 1u))) >> shift;
    return it.light.y + u32((32 * a + x * (b - a)) >> 5u);
}

// Source index `s` of item `it` over destination index `dst`.
fn blend(it: Item, s: u32, dst: u32) -> u32 {
    if it.blend.x == BLEND_OPAQUE {
        return s;
    } else if it.blend.x == BLEND_INDEX_TABLE_SRC_ROW {
        return map_byte(it.blend.y + s, dst);
    }
    return map_byte(it.blend.y + dst, s);
}

@compute @workgroup_size(16, 16, 1)
fn compose(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.x;
    let py = gid.y;
    if px >= params.size.x || py >= params.size.y {
        return;
    }
    let bin = (py / BIN_SIZE) * params.bins.x + px / BIN_SIZE;
    let at = py * params.size.x + px;
    var value = 0u;
    if py >= params.clear_rows {
        value = base_byte(at);
    }
    var world = value;
    var screen = false;
    // The frame without the UI items, and whether a screen item that is
    // not UI still contributes to it.
    var scene = value;
    var scene_screen = false;
    for (var k = bin_ranges[bin]; k < bin_ranges[bin + 1u]; k++) {
        let it = items[bin_items[k]];
        if px < it.area.x || px >= it.area.z || py < it.area.y || py >= it.area.w {
            continue;
        }
        let t = vec2<u32>(it.texel.x + (px - it.area.x), it.texel.y + (py - it.area.y));
        let src = textureLoad(atlas, t, it.texel.z, 0).r;
        if src == 0u {
            continue;
        }
        var s = shade(it, src);
        if it.light.x != 0u {
            // A wall or RLE floor block clipped on the left: the pixels of
            // the run the clip cut do not advance the light column.
            var run_cut = 0;
            let cut_x = i32(it.light.w & 0xffffu);
            let runs = it.light.x == GRADIENT_WALL || it.light.x == GRADIENT_RLE_FLOOR;
            if runs && cut_x > 0 {
                // The run's start, scanning left across the clipped columns.
                var start = i32(px - it.area.x);
                for (var c = 0; c < 32; c++) {
                    if start <= -cut_x {
                        break;
                    }
                    let tp = vec2<u32>(u32(i32(it.texel.x) + start - 1), t.y);
                    if textureLoad(atlas, tp, it.texel.z, 0).r == 0u {
                        break;
                    }
                    start -= 1;
                }
                run_cut = max(-start, 0);
            }
            s = map_byte(gradient_row(it, px - it.area.x, py - it.area.y, run_cut), s);
        }
        value = blend(it, s, value);
        // A translucent wall still contains the player drawn beneath it.
        // Keep that composite on the screen layer until an opaque item covers it.
        let on_screen = it.blend.z != LAYER_WORLD;
        let translucent = it.blend.x != BLEND_OPAQUE;
        screen = on_screen || (screen && translucent);
        if !on_screen {
            world = blend(it, s, world);
        }
        if it.blend.z != LAYER_UI {
            scene = blend(it, s, scene);
            scene_screen = on_screen || (scene_screen && translucent);
        }
    }
    if params.clear_after != 0u {
        value = 0u;
        world = 0u;
        screen = false;
        scene = 0u;
        scene_screen = false;
    }
    indices[at] = value;
    layers[at] = world | (select(0u, 1u, screen) << 8u) | (scene << 16u)
        | (select(0u, 1u, scene_screen) << 24u);
}

// The index the presented image shows at pixel `i`: the frame, or with the
// native UI pass the scene (the frame without the UI items).
fn shown(i: u32) -> u32 {
    if params.native_ui != 0u {
        return (layers[i] >> 16u) & 0xffu;
    }
    return indices[i] & 0xffu;
}

// Index framebuffer -> RGBA8 bytes through the frame palette: a bit copy of
// the palette word, so no color conversion can change a byte.
@compute @workgroup_size(16, 16, 1)
fn to_rgba(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.size.x || gid.y >= params.size.y {
        return;
    }
    let i = gid.y * params.size.x + gid.x;
    rgba[i] = palette[shown(i)];
}

// The presentation layers through the same palette: the world layer
// everywhere, the screen layer where it still contributes (elsewhere all
// zero, so alpha 0 lets the world layer show through). With the native UI
// pass it is the scene's screen part: the scene's colour where a screen
// item that is not UI still contributes.
@compute @workgroup_size(16, 16, 1)
fn to_layers(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.size.x || gid.y >= params.size.y {
        return;
    }
    let i = gid.y * params.size.x + gid.x;
    let l = layers[i];
    world_rgba[i] = palette[l & 0xffu];
    var mark = 0x100u;
    if params.native_ui != 0u {
        mark = 0x1000000u;
    }
    screen_rgba[i] = select(0u, palette[shown(i)], (l & mark) != 0u);
}
