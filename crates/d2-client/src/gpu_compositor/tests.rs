//! CI half of the GPU compositor (no GPU): buffer layouts, little-endian
//! packing, shader validity (naga), and the shader's algorithm run on the
//! packed bytes against the CPU reference, with perturbations (M08). The
//! byte-exact GPU run is the ignored `gpu_*` tests (local, needs a GPU).

use wgpu::naga;

use super::harness::{self, cases, diff, perturb, prepare, Case};
use super::pack::{self, emulate, GpuItem, Params, BASE_WIDE_WORD, ITEM_SIZE, PARAMS_SIZE};
use super::*;
use crate::frames::AtlasSlot;
use crate::scene::{self, DrawItem, FrameImage, MapTable, Rect};

fn module() -> naga::Module {
    naga::front::wgsl::parse_str(SHADER).expect("WGSL parses")
}

fn cpu(case: &Case) -> Vec<u8> {
    case.reference().expect("valid case")
}

fn case_named(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.name == name)
        .expect("case exists")
}

// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn shader_validates_with_naga() {
    let module = module();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("WGSL validates");
    let entries: Vec<(&str, [u32; 3])> = module
        .entry_points
        .iter()
        .map(|e| (e.name.as_str(), e.workgroup_size))
        .collect();
    assert_eq!(
        entries,
        [
            ("compose", [WORKGROUP, WORKGROUP, 1]),
            ("to_rgba", [WORKGROUP, WORKGROUP, 1]),
            ("to_layers", [WORKGROUP, WORKGROUP, 1])
        ]
    );
    const { assert!(WORKGROUP * WORKGROUP <= 256, "default invocation limit") };
    const { assert!(scene::BIN_SIZE.is_multiple_of(WORKGROUP)) };
}

/// The WGSL structs have exactly the byte layout `pack` writes.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn shader_struct_layouts_match_packing() {
    let module = module();
    let mut layouter = naga::proc::Layouter::default();
    layouter.update(module.to_ctx()).expect("layout");
    let layout = |name: &str| {
        let (handle, ty) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .expect("struct exists");
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("{name} is a struct");
        };
        assert_eq!(layouter[handle].size, *span);
        let offsets: Vec<(String, u32)> = members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect();
        (*span, offsets)
    };
    let owned = |v: &[(&str, u32)]| -> Vec<(String, u32)> {
        v.iter().map(|&(n, o)| (n.to_string(), o)).collect()
    };
    assert_eq!(
        layout("Item"),
        (
            ITEM_SIZE as u32,
            owned(&[
                ("area", 0),
                ("texel", 16),
                ("shade", 32),
                ("blend", 48),
                ("light", 64)
            ])
        )
    );
    assert_eq!(
        layout("Params"),
        (
            PARAMS_SIZE as u32,
            owned(&[
                ("size", 0),
                ("bins", 8),
                ("item_count", 16),
                ("map_rows", 20),
                ("pages", 24),
                ("clear_rows", 28),
                ("clear_after", 32),
                ("native_ui", 36),
                ("base_wide", (BASE_WIDE_WORD * 4) as u32),
                ("pad2", 44)
            ])
        )
    );
}

/// Bindings in the shader are the ones `device.rs` binds and `pack.rs`
/// documents.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn shader_bindings() {
    let module = module();
    let mut globals: Vec<(u32, u32, String)> = module
        .global_variables
        .iter()
        .filter_map(|(_, g)| {
            let b = g.binding.as_ref()?;
            Some((b.group, b.binding, g.name.clone().unwrap_or_default()))
        })
        .collect();
    globals.sort();
    let names: Vec<(u32, u32, &str)> = globals
        .iter()
        .map(|(g, b, n)| (*g, *b, n.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (0, 0, "params"),
            (0, 1, "items"),
            (0, 2, "bin_ranges"),
            (0, 3, "bin_items"),
            (0, 4, "maps"),
            (0, 5, "atlas"),
            (0, 6, "indices"),
            (0, 7, "palette"),
            (0, 8, "rgba"),
            (0, 9, "base"),
            (0, 10, "layers"),
            (0, 11, "world_rgba"),
            (0, 12, "screen_rgba"),
        ]
    );
}

// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn item_and_params_bytes_are_little_endian() {
    let item = GpuItem {
        area: [1, 2, 3, 0x0102_0304],
        texel: [5, 6],
        page: 7,
        shade_len: 2,
        shade: [8, 9, 0, 0],
        blend: 1,
        blend_base: 0xAABB_CCDD,
        light: [2, 0x0000_0300, 0x0403_0201, 0x0005_0006],
        layer: pack::LAYER_SCREEN,
    };
    let b = item.to_le_bytes();
    assert_eq!(&b[0..4], &[1, 0, 0, 0]);
    assert_eq!(&b[12..16], &[4, 3, 2, 1]);
    assert_eq!(
        &b[16..32],
        &[5, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 2, 0, 0, 0]
    );
    assert_eq!(&b[32..36], &[8, 0, 0, 0]);
    assert_eq!(&b[48..56], &[1, 0, 0, 0, 0xDD, 0xCC, 0xBB, 0xAA]);
    assert_eq!(&b[56..64], &[1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        &b[64..80],
        &[2, 0, 0, 0, 0, 3, 0, 0, 1, 2, 3, 4, 6, 0, 5, 0]
    );
    assert_eq!(GpuItem::from_le_bytes(&b), item);

    let params = Params {
        width: 800,
        height: 600,
        cols: 25,
        rows: 19,
        item_count: 3,
        map_rows: 513,
        pages: 2,
        clear_rows: 553,
        clear_after: 1,
        native_ui: 0,
    };
    let b = params.to_le_bytes();
    assert_eq!(
        b,
        [
            0x20, 3, 0, 0, 0x58, 2, 0, 0, 25, 0, 0, 0, 19, 0, 0, 0, 3, 0, 0, 0, 1, 2, 0, 0, 2, 0,
            0, 0, 0x29, 2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(Params::from_le_bytes(&b), params);
}

/// Exact packed buffers of a small list: areas in view coordinates, atlas
/// texels of the area's corner, bin prefix sums, map bytes unchanged.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn pack_vector() {
    let frames = vec![FrameImage {
        width: 40,
        height: 3,
        pixels: vec![1; 120],
    }];
    let mut maps = MapTable::new();
    let m = maps.push(harness::map_with(&[(1, 2)]));
    let view = Rect::new(-10, 0, 64, 40);
    let mut a = DrawItem::new(scene::FrameId(0), -20, 30);
    a.shade = scene::ShadeChain::new(&[m]).unwrap();
    a.clip = Rect::new(-100, -100, 1000, 1000);
    let b = DrawItem::new(scene::FrameId(0), 500, 0); // off the view
    let items = vec![a, b];
    let slots = vec![AtlasSlot {
        page: 0,
        x: 100,
        y: 200,
        w: 40,
        h: 3,
    }];
    let bins = scene::bin(&items, &frames, &maps, view).unwrap();
    let p = pack(&items, &bins, &frames, &slots, &maps, 1).unwrap();
    // Item a: image x −20..20, clip wider, view x −10..54 → area x 0..30, y 30..33;
    // texel x = 100 + (−10 − −20) = 110.
    assert_eq!(
        p.items[0],
        GpuItem {
            area: [0, 30, 30, 33],
            texel: [110, 200],
            page: 0,
            shade_len: 1,
            shade: [0, 0, 0, 0],
            blend: pack::BLEND_OPAQUE,
            blend_base: 0,
            light: [0; 4],
            layer: pack::LAYER_WORLD,
        }
    );
    assert_eq!(p.items[1].area, [0; 4]);
    // 2 × 2 bins; a touches bin (0, 0) (y 30..32) and (0, 1) (y 32..33).
    assert_eq!(p.bin_ranges, [0, 1, 1, 2, 2]);
    assert_eq!(p.bin_items, [0, 0]);
    assert_eq!(p.maps_bytes(), maps.rows()[0].to_vec());
    assert_eq!(p.params.map_rows, 1);
    assert_eq!(p.items_bytes().len(), 2 * ITEM_SIZE);
    assert_eq!(p.bin_ranges_bytes().len(), 5 * 4);
    // No frame given: a zero base of the view, no clears.
    assert_eq!(p.base, vec![0; 64 * 40]);
    assert_eq!(p.plan(), scene::FramePlan::NONE);
}

/// The base is one byte per view pixel, four per word, little-endian,
/// zero-padded; the plan goes into the params; bad sizes are rejected.
// Covers: specs/render/composition.md §3 text, §6
#[test]
fn pack_frame_base_and_plan() {
    let frames = vec![FrameImage {
        width: 1,
        height: 1,
        pixels: vec![1],
    }];
    let items = vec![DrawItem::new(scene::FrameId(0), 0, 0)];
    let maps = MapTable::new();
    let view = Rect::new(0, 0, 3, 2);
    let slots = vec![AtlasSlot {
        page: 0,
        x: 0,
        y: 0,
        w: 1,
        h: 1,
    }];
    let bins = scene::bin(&items, &frames, &maps, view).unwrap();
    let p = pack(&items, &bins, &frames, &slots, &maps, 1).unwrap();
    let plan = scene::FramePlan {
        clear_rows: 1,
        clear_after: true,
    };
    let p = p.with_frame(&[1, 2, 3, 4, 5, 6], plan).unwrap();
    assert_eq!(p.base_bytes(), [1, 2, 3, 4, 5, 6, 0, 0]);
    assert_eq!((p.params.clear_rows, p.params.clear_after), (1, 1));
    assert_eq!(p.plan(), plan);
    assert!(matches!(
        p.clone().with_frame(&[0; 5], plan),
        Err(GpuError::Scene(scene::SceneError::BaseSize { .. }))
    ));
    let rows = scene::FramePlan {
        clear_rows: 3,
        clear_after: false,
    };
    assert!(matches!(
        p.with_frame(&[0; 6], rows),
        Err(GpuError::Scene(scene::SceneError::FramePlan(_)))
    ));
}

/// Storage buffers are never empty.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
fn empty_list_pads_buffers() {
    let case = case_named("empty");
    let (atlas, p) = prepare(&case).unwrap();
    assert_eq!(atlas.page_count(), 0);
    assert_eq!(p.items_bytes(), vec![0; ITEM_SIZE]);
    assert_eq!(p.bin_items_bytes(), vec![0; 4]);
    assert_eq!(p.maps_bytes(), vec![0; 256]);
    assert_eq!(p.base_bytes(), vec![0; 800 * 600]);
    assert_eq!(p.bin_ranges, vec![0; 25 * 19 + 1]);
    assert_eq!(emulate(&p, atlas.pages()).unwrap(), vec![0; 800 * 600]);
}

/// The shader's algorithm on the packed bytes equals the CPU reference
/// (and the binned CPU model) on every synthetic case, the frame-cycle and
/// blend-op cases of `composition.md` included.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor, §a8-cpu-reference-compositor
// Covers: specs/render/composition.md §3 text, §5
// Covers: specs/render/shading.md §4 r4; specs/render/blend-modes.md §2; specs/client/render-pipeline.md §a5-blend-ops
// Covers: specs/render/blend-modes.md §8 r1, §8 r2
#[test]
fn emulated_shader_matches_cpu_on_all_cases() {
    let all = cases();
    assert_eq!(all.len(), 21);
    for case in &all {
        let reference = cpu(case);
        let bins = scene::bin(&case.items, &case.frames, &case.maps, case.view).unwrap();
        let binned = scene::compose_binned_frame(
            &case.items,
            &bins,
            &case.frames,
            &case.maps,
            case.view,
            &case.base(),
            case.plan,
        )
        .unwrap();
        assert_eq!(binned, reference, "{}", case.name);
        let (atlas, packed) = prepare(case).unwrap();
        let out = emulate(&packed, atlas.pages()).unwrap();
        let d = diff(&reference, &out, case.view.width, 1);
        assert_eq!(d.differing, 0, "{}: {d:?}", case.name);
    }
}

/// The presentation layers never change the frame: with every other item
/// on the screen layer, the frame's indices are still the CPU reference,
/// and the world layer is the reference of the list without the screen
/// items over the same base.
#[test]
fn layers_split_the_frame_without_changing_it() {
    for case in cases() {
        let reference = cpu(&case);
        let (atlas, mut packed) = prepare(&case).unwrap();
        // All on the world layer: the world layer is the frame.
        let all_world = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        for (p, &r) in all_world.iter().zip(&reference) {
            assert_eq!((p.index, p.world, p.screen), (r, r, false), "{}", case.name);
        }
        let screen = |i: usize| i % 2 == 1;
        packed.set_screen_layer(screen);
        let layers = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        let index: Vec<u8> = layers.iter().map(|p| p.index).collect();
        assert_eq!(index, reference, "{}: the frame is unchanged", case.name);
        let mut world_case = case.clone();
        world_case.items = case
            .items
            .iter()
            .enumerate()
            .filter(|&(i, _)| !screen(i))
            .map(|(_, it)| *it)
            .collect();
        let world_ref = cpu(&world_case);
        let world: Vec<u8> = layers.iter().map(|p| p.world).collect();
        assert_eq!(world, world_ref, "{}: world layer", case.name);
        // Recombining the presentation layers must reproduce every pixel,
        // including world items blended over screen items.
        for (p, &r) in layers.iter().zip(&reference) {
            assert_eq!(if p.screen { p.index } else { p.world }, r, "{}", case.name);
        }
    }
}

/// A room's translucent foreground wall must preserve the player in both
/// tick frames and the layers shown between ticks, standing or walking along it.
#[test]
fn translucent_wall_keeps_the_player_in_presentation_layers() {
    use crate::scene::{BlendOp, FrameId};

    let mut case = case_named("vector-opaque");
    case.view = Rect::new(0, 0, 12, 8);
    case.base = None;
    case.plan = scene::FramePlan::NONE;
    case.maps = MapTable::new();
    let mut table = Box::new([[0u8; 256]; 256]);
    for (a, row) in table.iter_mut().enumerate() {
        for (b, value) in row.iter_mut().enumerate() {
            *value = ((a + b) / 2) as u8;
        }
    }
    let blend = case.maps.push_table(&table);
    case.frames = [
        (12, 8, 11),
        (1, 2, 201),
        (12, 3, 99),
        (12, 1, 55),
        (1, 1, 241),
    ]
    .into_iter()
    .map(|(width, height, index)| FrameImage {
        width,
        height,
        pixels: vec![index; (width * height) as usize],
    })
    .collect();
    for op in [BlendOp::IndexTable(blend), BlendOp::IndexTableSrcRow(blend)] {
        for with_ui in [false, true] {
            let mut visible = Vec::new();
            // Stand for twelve frames, then walk along the same wall.
            for frame in 0..24 {
                let x = if frame < 12 { 2 } else { frame - 12 };
                case.items = vec![
                    DrawItem::new(FrameId(0), 0, 0),
                    DrawItem::new(FrameId(1), x, 4),
                    DrawItem::new(FrameId(2), 0, 4),
                    DrawItem::new(FrameId(3), 0, 5),
                ];
                case.items[2].blend = op;
                if with_ui {
                    // An opaque UI pixel must not erase the scene's dependency
                    // on the player when the native UI pass omits that UI pixel.
                    case.items.insert(2, DrawItem::new(FrameId(4), x, 4));
                }
                for item in &mut case.items {
                    item.clip = case.view;
                }
                let (atlas, mut packed) = prepare(&case).unwrap();
                packed.set_layers(|i| match i {
                    1 => pack::LAYER_SCREEN,
                    2 if with_ui => pack::LAYER_UI,
                    _ => pack::LAYER_WORLD,
                });
                let layers = pack::emulate_layers(&packed, atlas.pages()).unwrap();
                let reference = cpu(&case);
                let at = 4 * 12 + x as usize;
                assert_ne!(layers[at].scene, layers[at].world);
                visible.push(layers[at].on_screen_layer(true));
                for (p, &r) in layers.iter().zip(&reference) {
                    assert_eq!(p.index, r, "tick frame unchanged");
                    for native_ui in [false, true] {
                        let presented = if p.on_screen_layer(native_ui) {
                            p.shown(native_ui)
                        } else {
                            p.world
                        };
                        assert_eq!(presented, p.shown(native_ui), "frame {frame}");
                    }
                }
                assert!(layers[60..72].iter().all(|p| !p.screen && !p.scene_screen));
                assert!(
                    layers[60..72].iter().all(|p| p.world == 55),
                    "roof stays solid"
                );
            }
            assert!(visible.into_iter().all(|v| v), "player never flickers");
        }
    }
}

/// One screen item drawn last marks exactly the pixels it drew.
#[test]
fn the_screen_layer_marks_the_pixels_it_wrote_last() {
    let case = case_named("vector-opaque");
    let (atlas, mut packed) = prepare(&case).unwrap();
    let last = packed.items.len() - 1;
    packed.set_screen_layer(|i| i == last);
    let layers = pack::emulate_layers(&packed, atlas.pages()).unwrap();
    let mut only_last = case.clone();
    only_last.items = vec![case.items[last]];
    only_last.base = None;
    only_last.plan = scene::FramePlan::NONE;
    let drawn = cpu(&only_last);
    for (p, &d) in layers.iter().zip(&drawn) {
        assert_eq!(p.screen, d != 0);
    }
}

/// The cases exercise what they claim: something drawn, two pages, every
/// blend op, chains up to 4, off-screen and empty items.
#[test]
fn cases_are_not_trivial() {
    for case in cases() {
        let drawn = cpu(&case).iter().filter(|&&i| i != 0).count();
        let black = matches!(case.name, "empty" | "frame-post-clear");
        assert_eq!(drawn == 0, black, "{}", case.name);
    }
    // The frame cases start from a non-zero base, and use both clears.
    let frame = |n| case_named(n).plan;
    assert!(frame("frame-blank-screen").clear_rows == 553);
    assert!(frame("frame-no-clear") == scene::FramePlan::NONE);
    assert!(frame("frame-post-clear").clear_after);
    assert!(case_named("frame-stress").base.is_some());
    assert_eq!(prepare(&case_named("two-pages")).unwrap().0.page_count(), 2);
    let stress = case_named("stress");
    let (_, p) = prepare(&stress).unwrap();
    assert!(p.items.iter().any(|i| i.blend == pack::BLEND_INDEX_TABLE));
    assert!(p.items.iter().any(|i| i.shade_len == 4));
    assert!(p.items.iter().any(|i| i.area == [0; 4]));
    assert!(stress.items.iter().any(|i| i.frame == scene::FrameId(0)));
    // The shading cases use both gradient kinds, both table orientations
    // and areas that start inside their block.
    let (_, p) = prepare(&case_named("shading-stress")).unwrap();
    for code in [1, 2] {
        assert!(p.items.iter().any(|i| i.light[0] == code));
    }
    assert!(p.items.iter().any(|i| i.light[0] == 0));
    assert!(p.items.iter().any(|i| i.blend == pack::BLEND_INDEX_TABLE));
    assert!(p
        .items
        .iter()
        .any(|i| i.blend == pack::BLEND_INDEX_TABLE_SRC_ROW));
    assert!(p
        .items
        .iter()
        .any(|i| i.light[0] != 0 && i.light[3] != 0 && i.area != [0; 4]));
}

/// M08: the comparison reports exactly the bytes changed, in the CPU
/// reference (what `--perturb N` does) or in the packed GPU input.
// Covers: specs/client/render-pipeline.md §a10-verify-harness-extension
#[test]
fn perturbations_are_reported_exactly() {
    let case = case_named("stress");
    let reference = cpu(&case);
    let (atlas, mut packed) = prepare(&case).unwrap();
    let out = emulate(&packed, atlas.pages()).unwrap();
    for n in [1, 7, 1000] {
        let mut bad = reference.clone();
        assert_eq!(perturb(&mut bad, n), n);
        let d = diff(&bad, &out, case.view.width, 1);
        assert_eq!(d.differing, n as u64);
        assert_eq!(d.first.map(|f| (f.0, f.1)), Some((0, 0)));
        let rgba = diff(
            &scene::to_rgba(&bad, &case.palette),
            &scene::to_rgba(&out, &case.palette),
            case.view.width,
            4,
        );
        assert_eq!(rgba.differing, n as u64);
    }
    // A wrong shade row in the packed table must show in the image.
    let row = packed.items.iter().find(|i| i.shade_len > 0).unwrap().shade[0] as usize;
    for b in &mut packed.maps[row * 256..(row + 1) * 256] {
        *b = b.wrapping_add(1);
    }
    let broken = emulate(&packed, atlas.pages()).unwrap();
    assert!(diff(&reference, &broken, case.view.width, 1).differing > 0);
}

// Covers: specs/client/render-pipeline.md §a10-verify-harness-extension
#[test]
fn diff_counts_length_mismatch_and_first() {
    let d = diff(&[1, 2, 3, 4], &[1, 9, 3], 2, 1);
    assert_eq!(d.differing, 2);
    assert_eq!(d.first, Some((1, 0, 2, 9)));
    assert_eq!(perturb(&mut [0u8; 3], 10), 3);
    assert_eq!(perturb(&mut [], 1), 0);
}

/// Strict inputs (M07): everything the CPU rejects, `pack` rejects, plus
/// missing, mis-sized and out-of-atlas slots and foreign bins.
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor, §edge-cases-original-bugs
#[test]
fn pack_rejects_bad_input() {
    let case = case_named("vector-key-order");
    let atlas = AtlasFrames::from_images(&case.frames, 1).unwrap();
    let bins = scene::bin(&case.items, &case.frames, &case.maps, case.view).unwrap();
    let ok = |slots: &Vec<AtlasSlot>, pages| {
        pack(&case.items, &bins, &case.frames, slots, &case.maps, pages)
    };
    assert!(ok(&atlas.slots, 1).is_ok());
    assert!(matches!(
        ok(&atlas.slots[..1].to_vec(), 1),
        Err(GpuError::SlotMissing { .. })
    ));
    let mut wrong = atlas.slots.clone();
    wrong[1].w += 1;
    assert!(matches!(ok(&wrong, 1), Err(GpuError::SlotSize { .. })));
    assert!(matches!(
        ok(&atlas.slots, 0),
        Err(GpuError::SlotPage { .. })
    ));
    let mut edge = atlas.slots.clone();
    edge[0].x = crate::frames::PAGE_SIZE - 2;
    assert!(matches!(ok(&edge, 1), Err(GpuError::SlotPage { .. })));

    // Bins carry their view: bins of another view pack that view.
    let other = scene::bin(&case.items, &case.frames, &case.maps, Rect::FRAME).unwrap();
    let p = pack(
        &case.items,
        &other,
        &case.frames,
        &atlas.slots,
        &case.maps,
        1,
    )
    .unwrap();
    assert_eq!(
        (p.view, p.params.cols, p.params.rows),
        (Rect::FRAME, 25, 19)
    );
    // Bins of another list of the same length.
    let mut moved = case.items.clone();
    moved[0].x = 100; // off the view: no longer in bin 0
    assert!(matches!(
        pack(&moved, &bins, &case.frames, &atlas.slots, &case.maps, 1),
        Err(GpuError::BinsMismatch)
    ));
    // An item the CPU rejects.
    let mut flipped = case.items.clone();
    flipped[0].flip_x = true;
    assert!(matches!(
        pack(&flipped, &bins, &case.frames, &atlas.slots, &case.maps, 1),
        Err(GpuError::Scene(scene::SceneError::Item { index: 0, .. }))
    ));
    // Pages given at dispatch must be the packed count.
    let p = ok(&atlas.slots, 1).unwrap();
    assert!(matches!(
        emulate(&p, &[]),
        Err(GpuError::PageCount {
            pages: 0,
            packed: 1
        })
    ));
}

/// The 16:9 frame composes all 1068 columns: the packing sizes its bins
/// and buffers from the view (34 bin columns, the last 12 px wide), and the
/// shader's algorithm fills every pixel exactly as the CPU reference does.
#[test]
fn wide_frame_fills_every_column() {
    let case = harness::wide_case();
    let reference = cpu(&case);
    assert_eq!(reference.len(), 1068 * 600);
    let (atlas, packed) = prepare(&case).unwrap();
    assert_eq!((packed.params.width, packed.params.height), (1068, 600));
    assert_eq!((packed.params.cols, packed.params.rows), (34, 19));
    let out = emulate(&packed, atlas.pages()).unwrap();
    assert_eq!(diff(&reference, &out, 1068, 1).differing, 0);
    for (y, row) in out.chunks(1068).enumerate() {
        for (x, &v) in row.iter().enumerate() {
            let want = if (1050..1068).contains(&x) && (290..310).contains(&y) {
                251
            } else {
                (x % 250 + 1) as u8
            };
            assert_eq!(v, want, "pixel ({x}, {y})");
        }
    }
}

/// Local, needs a GPU: the 16:9 frame byte-identical on the GPU.
#[test]
#[ignore = "needs a GPU"]
fn gpu_wide_frame_matches_cpu() {
    let report = harness::compare(&gpu(), &harness::wide_case(), 0).unwrap();
    assert!(report.is_match(), "{report}");
}

fn gpu() -> Gpu {
    let (gpu, info) = Gpu::headless().expect("a GPU adapter");
    println!(
        "adapter: {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );
    gpu
}

/// Local, needs a GPU: every case byte-identical on the GPU.
/// `cargo test -p d2-client --lib gpu_compositor::tests::gpu_matches_cpu -- --ignored --nocapture`
// Covers: specs/client/render-pipeline.md §a9-gpu-compute-compositor
#[test]
#[ignore = "needs a GPU"]
fn gpu_matches_cpu() {
    let gpu = gpu();
    for case in cases() {
        let report = harness::compare(&gpu, &case, 0).unwrap();
        println!("{report}");
        assert!(report.is_match(), "{report}");
    }
}

/// Local, needs a GPU: every case, the shading and light-gradient cases
/// included, byte-identical on every adapter that can run the compositor,
/// software and fallback adapters too (WARP, lavapipe, llvmpipe), on the
/// adapter's own limits as the game's device takes them. A low-end or
/// fallback adapter must light and shade exactly as the CPU reference.
/// `cargo test -p d2-client --lib gpu_compositor::tests::gpu_matches_cpu_on_every_adapter -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_matches_cpu_on_every_adapter() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapters = bevy::tasks::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let mut ran = 0;
    for adapter in adapters {
        let info = adapter.get_info();
        let name = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let compute = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS);
        if !compute {
            println!("{name}: no compute shaders, skipped");
            continue;
        }
        let device = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("d2 compositor"),
            required_limits: adapter.limits(),
            ..Default::default()
        }));
        let Ok((device, queue)) = device else {
            println!("{name}: no device, skipped");
            continue;
        };
        let gpu = Gpu::from_device(device, queue);
        for case in cases() {
            let report = harness::compare(&gpu, &case, 0).unwrap();
            assert!(report.is_match(), "{name}: {report}");
        }
        println!("{name}: every case matches");
        ran += 1;
    }
    assert!(ran > 0, "no adapter ran the compositor");
}

/// Local, needs a GPU (M08): with 7 reference bytes changed, every case
/// reports exactly 7 differing bytes and 7 differing pixels.
// Covers: specs/client/render-pipeline.md §a10-verify-harness-extension
#[test]
#[ignore = "needs a GPU"]
fn gpu_perturb_reports_exactly_n() {
    let gpu = gpu();
    for case in cases() {
        let report = harness::compare(&gpu, &case, 7).unwrap();
        println!("{report}");
        assert_eq!(report.indices.differing, 7, "{report}");
        assert_eq!(report.rgba.differing, 7, "{report}");
    }
}

/// Local, needs a GPU: the layer outputs equal the emulated layers through
/// the palette, with every other item on the screen layer.
/// `cargo test -p d2-client --lib gpu_compositor::tests::gpu_layers_match_emulation -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_layers_match_emulation() {
    let gpu = gpu();
    for case in cases() {
        let (atlas, mut packed) = prepare(&case).unwrap();
        packed.set_screen_layer(|i| i % 2 == 1);
        let (rgba, world, screen) = gpu
            .compose_layers(&packed, atlas.pages(), &case.palette)
            .unwrap();
        let layers = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        let colour = |i: u8| {
            let c = case.palette.colors[usize::from(i)];
            [c.r, c.g, c.b, 255]
        };
        let want_rgba: Vec<u8> = layers.iter().flat_map(|p| colour(p.index)).collect();
        let want_world: Vec<u8> = layers.iter().flat_map(|p| colour(p.world)).collect();
        let want_screen: Vec<u8> = layers
            .iter()
            .flat_map(|p| if p.screen { colour(p.index) } else { [0; 4] })
            .collect();
        assert_eq!(rgba, want_rgba, "{}", case.name);
        assert_eq!(world, want_world, "{}", case.name);
        assert_eq!(screen, want_screen, "{}", case.name);
    }
}

/// The case's items with every third one moved by `(3, -2)`, as the
/// display frames between ticks move a unit's items (`world_view::unit_motion`).
fn moved(case: &Case) -> Vec<DrawItem> {
    let mut items = case.items.clone();
    for it in items.iter_mut().step_by(3) {
        it.x += 3;
        it.y -= 2;
    }
    items
}

/// A draw list over a packed frame packs exactly as the frame of those
/// items would (items, bins, size, clears), without its map table and
/// base, which the compositor reads from the frame's own upload.
#[test]
fn a_draw_list_over_the_frame_packs_like_a_frame_of_its_items() {
    for case in cases() {
        let (atlas, frame) = prepare(&case).unwrap();
        let items = moved(&case);
        let bins = scene::bin(&items, &case.frames, &case.maps, case.view).unwrap();
        let list = frame
            .with_items(&items, &bins, &case.frames, &atlas.slots)
            .unwrap();
        let mut whole = case.clone();
        whole.items = items;
        let (_, want) = prepare(&whole).unwrap();
        assert_eq!(list.items, want.items, "{}", case.name);
        assert_eq!(list.bin_ranges, want.bin_ranges, "{}", case.name);
        assert_eq!(list.bin_items, want.bin_items, "{}", case.name);
        assert_eq!(list.params, want.params, "{}", case.name);
        assert!(list.maps.is_empty() && list.base.is_empty());
        // The frame itself is untouched.
        assert_eq!(frame, prepare(&case).unwrap().1, "{}", case.name);
    }
}

/// Bins over another view are refused.
#[test]
fn a_draw_list_over_another_view_is_refused() {
    let case = case_named("stress");
    let (atlas, frame) = prepare(&case).unwrap();
    let other = scene::Rect::new(0, 0, 64, 64);
    let bins = scene::bin(&case.items, &case.frames, &case.maps, other).unwrap();
    assert!(frame
        .with_items(&case.items, &bins, &case.frames, &atlas.slots)
        .is_err());
}

/// Local, needs a GPU: the layers of a moved draw list composed over the
/// tick frame's upload equal the emulation of a whole frame of those
/// items over the same base.
/// `cargo test -p d2-client --lib gpu_compositor::tests::gpu_layers_over_a_frame_match_emulation -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_layers_over_a_frame_match_emulation() {
    let gpu = gpu();
    for case in cases() {
        let (atlas, frame) = prepare(&case).unwrap();
        let items = moved(&case);
        let bins = scene::bin(&items, &case.frames, &case.maps, case.view).unwrap();
        let mut list = frame
            .with_items(&items, &bins, &case.frames, &atlas.slots)
            .unwrap();
        list.set_screen_layer(|i| i % 2 == 1);
        let (world, screen, _) = gpu
            .layers_over(&frame, &list, atlas.pages(), &case.palette)
            .unwrap();
        let mut whole = case.clone();
        whole.items = items;
        let (_, mut want) = prepare(&whole).unwrap();
        want.set_screen_layer(|i| i % 2 == 1);
        let layers = pack::emulate_layers(&want, atlas.pages()).unwrap();
        let colour = |i: u8| {
            let c = case.palette.colors[usize::from(i)];
            [c.r, c.g, c.b, 255]
        };
        let want_world: Vec<u8> = layers.iter().flat_map(|p| colour(p.world)).collect();
        let want_screen: Vec<u8> = layers
            .iter()
            .flat_map(|p| if p.screen { colour(p.index) } else { [0; 4] })
            .collect();
        assert_eq!(world, want_world, "{}", case.name);
        assert_eq!(screen, want_screen, "{}", case.name);
    }
}

/// Local, needs a GPU: the next frame composed over the frame before's
/// index output still on the GPU (`encode_rgba_over`, live play's frame
/// chain, `world_view::node`) is byte-identical to the same frame composed
/// over that output read back and packed as its base, and to the CPU
/// emulation; the packed base is not read then (it is all wrong here).
/// `cargo test -p d2-client --lib gpu_compositor::tests::gpu_frame_over_the_gpu_base_matches_the_read_back_base -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_frame_over_the_gpu_base_matches_the_read_back_base() {
    let gpu = gpu();
    for case in cases() {
        let (atlas, first) = prepare(&case).unwrap();
        let pages = atlas.pages();
        let texture = gpu.upload_atlas(pages);
        let n = pages.len() as u32;
        let size = first.pixel_count() as u64 * 4;
        let mut encoder = gpu.device().create_command_encoder(&Default::default());
        let one = gpu
            .encode_rgba(&mut encoder, &first, &texture, n, &case.palette)
            .unwrap()
            .expect("a view");
        gpu.queue().submit([encoder.finish()]);
        let words =
            |bytes: Vec<u8>| -> Vec<u8> { bytes.as_chunks::<4>().0.iter().map(|w| w[0]).collect() };
        let encoder = gpu.device().create_command_encoder(&Default::default());
        let back = words(gpu.read_back(encoder, &one.indices, size).unwrap());
        // The next frame: the items moved, over the first frame.
        let mut next_case = case.clone();
        next_case.items = moved(&case);
        let (_, next) = prepare(&next_case).unwrap();
        let plan = scene::FramePlan::NONE;
        let over_read_back = next.clone().with_frame(&back, plan).unwrap();
        let wrong_base = vec![77; back.len()];
        let over_gpu = next.with_frame(&wrong_base, plan).unwrap();
        let compose = |packed: &Packed, base: Option<&wgpu::Buffer>| {
            let mut encoder = gpu.device().create_command_encoder(&Default::default());
            let out = gpu
                .encode_rgba_over(&mut encoder, packed, &texture, n, &case.palette, base)
                .unwrap()
                .expect("a view");
            gpu.queue().submit([encoder.finish()]);
            let encoder = gpu.device().create_command_encoder(&Default::default());
            words(gpu.read_back(encoder, &out.indices, size).unwrap())
        };
        let want = compose(&over_read_back, None);
        let got = compose(&over_gpu, Some(&one.indices));
        assert_eq!(
            want,
            emulate(&over_read_back, pages).unwrap(),
            "{}",
            case.name
        );
        assert_eq!(got, want, "{}", case.name);
        // A base of another size is refused.
        let mut encoder = gpu.device().create_command_encoder(&Default::default());
        let small = gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        assert!(gpu
            .encode_rgba_over(
                &mut encoder,
                &over_gpu,
                &texture,
                n,
                &case.palette,
                Some(&small)
            )
            .is_err());
    }
}

/// Local, needs a GPU: the cost of one display frame between ticks, on
/// every adapter: a busy 16:9 game frame (1068x600, 3000 items), its
/// draw list repacked on the CPU and composed into the two layers on the
/// GPU (submitted and waited for, so an upper bound). The compose is at
/// the game frame's size: the screen resolution does not enter it.
/// `cargo test --release -p d2-client --lib gpu_compositor::tests::gpu_motion_compose_cost -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_motion_compose_cost() {
    let case = harness::stress("motion-cost", 11, 3000, scene::Rect::new(0, 0, 1068, 600));
    let (atlas, frame) = prepare(&case).unwrap();
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapters = bevy::tasks::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    for adapter in adapters {
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            continue;
        }
        let Ok((device, queue)) =
            bevy::tasks::block_on(adapter.request_device(&Default::default()))
        else {
            continue;
        };
        let gpu = Gpu::from_device(device, queue);
        let runs = 200;
        let (mut cpu, mut gpu_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut gpu_min = std::time::Duration::MAX;
        for n in 0..runs + 10 {
            let start = std::time::Instant::now();
            let mut items = case.items.clone();
            for it in items.iter_mut().step_by(10) {
                it.x += 1 + n % 5;
            }
            let bins = scene::bin(&items, &case.frames, &case.maps, case.view).unwrap();
            let mut list = frame
                .with_items(&items, &bins, &case.frames, &atlas.slots)
                .unwrap();
            list.set_screen_layer(|i| i % 50 == 0);
            let packed_in = start.elapsed();
            let (_, _, took) = gpu
                .layers_over(&frame, &list, atlas.pages(), &case.palette)
                .unwrap();
            if n >= 10 {
                cpu += packed_in;
                gpu_time += took;
                gpu_min = gpu_min.min(took);
            }
        }
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / f64::from(runs);
        println!(
            "{} ({:?}, {:?}): CPU repack {:.3} ms, GPU compose {:.3} ms (min {:.3} ms) per display frame",
            info.name,
            info.backend,
            info.device_type,
            ms(cpu),
            ms(gpu_time),
            gpu_min.as_secs_f64() * 1000.0,
        );
    }
}

/// Local, needs a GPU: the cost of presenting a display frame between
/// ticks at 2160p, on every adapter: the world and the screen layer
/// (1068x600 each) drawn over a 3840x2160 target, one full-screen
/// triangle each, sharp-bilinear sampled and blended (submitted and
/// waited for, so an upper bound). With the compose cost
/// ([`gpu_motion_compose_cost`]) this is all a display frame adds.
/// `cargo test --release -p d2-client --lib gpu_compositor::tests::gpu_present_cost_2160p -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_present_cost_2160p() {
    const SHADER: &str = r"
@group(0) @binding(0) var layer: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs(o: Out) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(layer));
    let texel = o.uv * size;
    let scale = 3.6;
    let cell = floor(texel - 0.5) + 0.5;
    let f = clamp((texel - 0.5 - floor(texel - 0.5)) * scale - (scale - 1.0) * 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    return textureSample(layer, smp, (cell + f) / size);
}
";
    let (w, h) = (3840u32, 2160u32);
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapters = bevy::tasks::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    for adapter in adapters {
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            continue;
        }
        let Ok((device, queue)) =
            bevy::tasks::block_on(adapter.request_device(&Default::default()))
        else {
            continue;
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present cost"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present cost"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let texture = |width, height, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let target = texture(w, h, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let target_view = target.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let groups: Vec<wgpu::BindGroup> = (0..2)
            .map(|_| {
                let layer = texture(1068, 600, wgpu::TextureUsages::TEXTURE_BINDING);
                let view = layer.create_view(&Default::default());
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                })
            })
            .collect();
        let runs = 200;
        let (mut total, mut min) = (std::time::Duration::ZERO, std::time::Duration::MAX);
        for n in 0..runs + 10 {
            let start = std::time::Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                for g in &groups {
                    pass.set_bind_group(0, g, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
            queue.submit([encoder.finish()]);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            let took = start.elapsed();
            if n >= 10 {
                total += took;
                min = min.min(took);
            }
        }
        println!(
            "{} ({:?}, {:?}): 2160p layer present {:.3} ms (min {:.3} ms) per display frame",
            info.name,
            info.backend,
            info.device_type,
            total.as_secs_f64() * 1000.0 / f64::from(runs),
            min.as_secs_f64() * 1000.0,
        );
    }
}

#[test]
fn the_atlas_array_keeps_spare_layers_within_the_memory_budget() {
    assert_eq!(device::atlas_layers(0, 256), 0);
    assert_eq!(device::atlas_layers(1, 256), 5);
    assert_eq!(device::atlas_layers(20, 256), 30);
    // Spare capacity cannot push the game atlas beyond 256 MiB.
    assert_eq!(device::atlas_layers(60, 256), 64);
    assert_eq!(device::atlas_layers(64, 256), 64);
    // Within the device's limit, never fewer than the requested pages.
    assert_eq!(device::atlas_layers(60, 64), 64);
    assert_eq!(device::atlas_layers(70, 64), 70);
}

#[test]
fn only_changed_pages_are_written() {
    use std::sync::Arc;
    let page = |b: u8| {
        Arc::new(crate::frames::atlas::AtlasPage {
            pixels: vec![b],
            dirty: false,
            generation: 0,
        })
    };
    let held = vec![page(1), page(2)];
    // The same pages: nothing to write.
    assert!(device::changed_pages(&held, &held.clone()).is_empty());
    // A replaced page and a new one are written; a page with equal bytes
    // but not shared counts as changed.
    let next = vec![held[0].clone(), page(2), page(3)];
    assert_eq!(device::changed_pages(&held, &next), [1, 2]);
    // An empty array writes every page.
    assert_eq!(device::changed_pages(&[], &next), [0, 1, 2]);
}
