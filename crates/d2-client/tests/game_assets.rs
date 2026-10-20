//! The client asset path on the user's game archives: canonical paths
//! over every listed name, the Bevy loaders on real files,
//! frame sets and the residency cache on real frame sets, and
//! the CPU reference compositor on real frames, shade maps and blend
//! tables against an independent per-pixel evaluation of the blend rules, with
//! its binned walk and (GPU test) the compute compositor compared to it
//! the way `verify` compares synthetic cases.
//!
//! Expected values unconfirmed: written without game files, so no test
//! here carries a `Covers` claim until its first local run passes.
//!
//! Ignored by default. Run:
//! `D2_GAME_DIR=<install> cargo test --release -p d2-client --test game_assets -- --ignored --nocapture --test-threads 1`

use std::collections::BTreeSet;
use std::sync::Arc;

use bevy::asset::LoadState;
use bevy::prelude::*;
use d2_client::assets::cache::{CacheEvent, Clock, Pool};
use d2_client::assets::path::{read_asset, CanonicalPath};
use d2_client::assets::{
    asset_path, CofAsset, D2AssetsPlugin, Dc6Asset, DccAsset, Ds1Asset, Dt1Asset, MpqSourcePlugin,
    PaletteAsset, Pl2Asset, TblAsset,
};
use d2_client::frames::{FramePart, FrameSet, FrameSetKey};
use d2_client::scene::{
    self, BlendOp, DrawItem, FrameId, FrameImage, MapId, MapTable, Rect, ShadeChain,
};
use d2_client::verify::{self, gpu::Wgpu, GpuCompositor, GpuJob, GpuOutcome};
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;

const PALETTE: &str = r"data\global\palette\act1\pal.dat";
const PL2: &str = r"data\global\palette\act1\pal.pl2";

fn set() -> ArchiveSet {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open")
}

/// Distinct listed names over every archive, as listed (first spelling).
fn listed(set: &ArchiveSet) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for a in set.archives() {
        for n in a.listfile().unwrap().unwrap_or_default() {
            if seen.insert(n.to_ascii_lowercase()) {
                names.push(n);
            }
        }
    }
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names
}

fn with_ext<'a>(names: &'a [String], ext: &'a str) -> impl Iterator<Item = &'a String> + 'a {
    names
        .iter()
        .filter(move |n| n.to_ascii_lowercase().ends_with(ext))
}

fn read(set: &ArchiveSet, name: &str) -> Vec<u8> {
    set.read(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A `.tbl` of printable text only is a text file.
fn is_text(b: &[u8]) -> bool {
    b.iter()
        .all(|&c| c == b'\t' || c == b'\r' || c == b'\n' || (0x20..0x7F).contains(&c))
}

// Canonical paths on every listed name of the install: each
// canonicalizes (expect 0 refused), and reading the canonical path through
// the `mpq://` reader gives the bytes the archive set gives for the listed
// spelling.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_listed_name_canonicalizes_and_reads_back() {
    let set = set();
    let names = listed(&set);
    let mut refused = Vec::new();
    let mut read_back = 0usize;
    for name in &names {
        let canon = match CanonicalPath::new(name) {
            Ok(c) => c,
            Err(e) => {
                refused.push(e.to_string());
                continue;
            }
        };
        // One handle per file: the canonical form of the canonical form is
        // itself, and asset_path agrees with it.
        assert_eq!(
            CanonicalPath::parse_canonical(canon.as_str()).as_ref(),
            Ok(&canon)
        );
        assert_eq!(asset_path(name), canon.asset_path());
        let via_reader = read_asset(&set, canon.as_str()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(via_reader == read(&set, name), "{name}: bytes differ");
        read_back += 1;
    }
    println!(
        "{} listed names, {read_back} read back identical, {} refused: {refused:?}",
        names.len(),
        refused.len()
    );
    assert!(refused.is_empty());
}

/// A windowless app with `mpq://` over the install.
fn app(set: ArchiveSet) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        MpqSourcePlugin {
            archives: Arc::new(set),
        },
        AssetPlugin::default(),
        D2AssetsPlugin,
    ));
    app
}

/// Runs the app until every id has left the loading states.
fn settle(app: &mut App, ids: &[bevy::asset::UntypedAssetId]) {
    let start = std::time::Instant::now();
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        let pending = ids
            .iter()
            .filter(|&&id| {
                matches!(
                    server.load_state(id),
                    LoadState::NotLoaded | LoadState::Loading
                )
            })
            .count();
        if pending == 0 {
            return;
        }
        assert!(
            start.elapsed().as_secs() < 600,
            "{pending} assets never settled"
        );
        std::thread::yield_now();
    }
}

/// Loads `name` through the `AssetServer` and returns its handle once
/// settled; a failed load fails the test, naming the file.
fn load<A: Asset>(app: &mut App, name: &str) -> Handle<A> {
    let server = app.world().resource::<AssetServer>().clone();
    let h: Handle<A> = server.load(asset_path(name));
    settle(app, &[h.id().untyped()]);
    let state = app.world().resource::<AssetServer>().load_state(h.id());
    assert!(state.is_loaded(), "{name}: {state:?}");
    h
}

/// The first listed name with `ext` (sorted) that `parse` accepts.
fn first_parsing<T>(
    set: &ArchiveSet,
    names: &[String],
    ext: &str,
    parse: impl Fn(&[u8]) -> Option<T>,
) -> (String, T) {
    with_ext(names, ext)
        .find_map(|n| parse(&read(set, n)).map(|v| (n.clone(), v)))
        .unwrap_or_else(|| panic!("no {ext} parses"))
}

// The loaders: one real file per loader through the AssetServer equals the
// d2-formats parse of the same bytes (the loaders add no logic).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn each_loader_loads_a_real_file() {
    let set = set();
    let names = listed(&set);
    let (ds1, ds1_v) = first_parsing(&set, &names, ".ds1", |b| Ds1::parse(b).ok());
    let (dt1, dt1_v) = first_parsing(&set, &names, ".dt1", |b| Dt1::parse(b).ok());
    let (dc6, dc6_v) = first_parsing(&set, &names, ".dc6", |b| Dc6::parse(b).ok());
    let (dcc, dcc_v) = first_parsing(&set, &names, ".dcc", |b| Dcc::parse(b).ok());
    let (pl2, pl2_v) = first_parsing(&set, &names, ".pl2", |b| Pl2::parse(b).ok());
    let (cof, cof_v) = first_parsing(&set, &names, ".cof", |b| Cof::parse(b).ok());
    let pal_v = Palette::parse(&read(&set, PALETTE)).unwrap();
    let font = r"data\local\font\latin\font16.tbl";
    let font_v = FontTable::parse(&read(&set, font)).unwrap();
    let strings = r"data\local\lng\eng\string.tbl";
    let strings_v = StringTable::parse(&read(&set, strings)).unwrap();
    println!("loading {ds1}, {dt1}, {dc6}, {dcc}, {pl2}, {cof}, {PALETTE}, {font}, {strings}");

    let mut app = app(set);
    let h = load::<Ds1Asset>(&mut app, &ds1);
    assert_eq!(
        app.world()
            .resource::<Assets<Ds1Asset>>()
            .get(&h)
            .unwrap()
            .0,
        ds1_v
    );
    let h = load::<Dt1Asset>(&mut app, &dt1);
    assert_eq!(
        app.world()
            .resource::<Assets<Dt1Asset>>()
            .get(&h)
            .unwrap()
            .0,
        dt1_v
    );
    let h = load::<Dc6Asset>(&mut app, &dc6);
    assert_eq!(
        app.world()
            .resource::<Assets<Dc6Asset>>()
            .get(&h)
            .unwrap()
            .0,
        dc6_v
    );
    let h = load::<DccAsset>(&mut app, &dcc);
    assert_eq!(
        app.world()
            .resource::<Assets<DccAsset>>()
            .get(&h)
            .unwrap()
            .0,
        dcc_v
    );
    let h = load::<Pl2Asset>(&mut app, &pl2);
    assert_eq!(
        app.world()
            .resource::<Assets<Pl2Asset>>()
            .get(&h)
            .unwrap()
            .0,
        pl2_v
    );
    let h = load::<CofAsset>(&mut app, &cof);
    assert_eq!(
        app.world()
            .resource::<Assets<CofAsset>>()
            .get(&h)
            .unwrap()
            .0,
        cof_v
    );
    let h = load::<PaletteAsset>(&mut app, PALETTE);
    assert_eq!(
        app.world()
            .resource::<Assets<PaletteAsset>>()
            .get(&h)
            .unwrap()
            .0,
        pal_v
    );
    let h = load::<TblAsset>(&mut app, font);
    match app.world().resource::<Assets<TblAsset>>().get(&h).unwrap() {
        TblAsset::Font(f) => assert_eq!(*f, font_v),
        other => panic!("{font}: {other:?}"),
    }
    let h = load::<TblAsset>(&mut app, strings);
    match app.world().resource::<Assets<TblAsset>>().get(&h).unwrap() {
        TblAsset::Strings(t) => assert_eq!(*t, strings_v),
        other => panic!("{strings}: {other:?}"),
    }
}

// Every `.pl2`, `.cof` and `.tbl` through the AssetServer: every one
// loads, except the junk `amblxbow.cof`, which fails naming its path; a
// `.tbl` under `data\local\font\` is `Font`, one under `data\local\lng\`
// is `Strings`. The two plain-text `.tbl` files are not a font or a
// string table and no rule says what the loader does
// with them: their outcome is printed, not asserted.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_pl2_cof_and_tbl_loads() {
    let set = set();
    let names = listed(&set);
    let pl2s: Vec<String> = with_ext(&names, ".pl2").cloned().collect();
    let cofs: Vec<String> = with_ext(&names, ".cof").cloned().collect();
    let tbls: Vec<(String, bool)> = with_ext(&names, ".tbl")
        .map(|n| (n.clone(), is_text(&read(&set, n))))
        .collect();
    let mut app = app(set);
    let server = app.world().resource::<AssetServer>().clone();
    let pl2_h: Vec<Handle<Pl2Asset>> = pl2s.iter().map(|n| server.load(asset_path(n))).collect();
    let cof_h: Vec<Handle<CofAsset>> = cofs.iter().map(|n| server.load(asset_path(n))).collect();
    let tbl_h: Vec<Handle<TblAsset>> = tbls
        .iter()
        .map(|(n, _)| server.load(asset_path(n)))
        .collect();
    let ids: Vec<_> = pl2_h
        .iter()
        .map(|h| h.id().untyped())
        .chain(cof_h.iter().map(|h| h.id().untyped()))
        .chain(tbl_h.iter().map(|h| h.id().untyped()))
        .collect();
    settle(&mut app, &ids);

    let state =
        |id: bevy::asset::UntypedAssetId| app.world().resource::<AssetServer>().load_state(id);
    for (n, h) in pl2s.iter().zip(&pl2_h) {
        assert!(state(h.id().untyped()).is_loaded(), "{n}");
    }
    let mut cof_failed = Vec::new();
    for (n, h) in cofs.iter().zip(&cof_h) {
        match state(h.id().untyped()) {
            LoadState::Loaded => {}
            LoadState::Failed(e) => {
                let canon = CanonicalPath::new(n).unwrap();
                assert!(e.to_string().contains(canon.as_str()), "{n}: {e}");
                cof_failed.push(canon.as_str().to_string());
            }
            other => panic!("{n}: {other:?}"),
        }
    }
    let tbl_assets = app.world().resource::<Assets<TblAsset>>();
    let (mut fonts, mut strings) = (0, 0);
    for ((n, text), h) in tbls.iter().zip(&tbl_h) {
        let lower = n.to_ascii_lowercase();
        let s = state(h.id().untyped());
        if *text {
            println!("text .tbl {n}: {s:?}");
            continue;
        }
        let asset = tbl_assets.get(h).unwrap_or_else(|| panic!("{n}: {s:?}"));
        match asset {
            TblAsset::Font(_) => fonts += 1,
            TblAsset::Strings(_) => strings += 1,
        }
        if lower.starts_with(r"data\local\font\") {
            assert!(matches!(asset, TblAsset::Font(_)), "{n}");
        } else if lower.starts_with(r"data\local\lng\") {
            assert!(matches!(asset, TblAsset::Strings(_)), "{n}");
        } else {
            println!(".tbl outside font and lng folders: {n}");
        }
    }
    println!(
        "loaded {} .pl2, {} .cof (failed {cof_failed:?}), {} .tbl ({fonts} font, {strings} strings)",
        pl2s.len(),
        cofs.len(),
        tbls.len()
    );
    assert_eq!(cof_failed, ["data/global/chars/am/cof/amblxbow.cof"]);
    assert!(fonts > 0 && strings > 0);
}

/// A clock that advances one microsecond per reading.
struct Ticks(u64);

impl Clock for Ticks {
    fn now_micros(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
}

/// The first listed `.dcc` (sorted) with at least `dirs` directions, each
/// with a non-empty frame.
fn dcc_with_dirs(set: &ArchiveSet, names: &[String], dirs: usize) -> (String, Dcc) {
    first_parsing(set, names, ".dcc", |b| {
        Dcc::parse(b).ok().filter(|d| {
            d.directions.len() >= dirs
                && d.directions[..dirs]
                    .iter()
                    .all(|dir| dir.frames.iter().any(|f| !f.pixels.is_empty()))
        })
    })
}

// Frame sets of a real DCC equal its parsed directions; the frame-set
// residency on them: missing keys load synchronously and count as stalls,
// resident keys do not reload, LRU by last frame used with ties by key
// order, and a frame over budget keeps every key it uses (overrun logged).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn frame_sets_and_residency_on_a_real_dcc() {
    let set = set();
    let names = listed(&set);
    let (name, dcc) = dcc_with_dirs(&set, &names, 4);
    let path = CanonicalPath::new(&name).unwrap();
    println!("{name}: {} directions", dcc.directions.len());
    let key = |d: u8| FrameSetKey::new(path.as_str(), FramePart::Dir(d)).unwrap();

    let sets: Vec<FrameSet> = (0..4)
        .map(|d| FrameSet::from_dcc(&dcc, d).unwrap())
        .collect();
    for (d, s) in sets.iter().enumerate() {
        let dir = &dcc.directions[d];
        assert_eq!(s.frames.len(), dir.frames.len(), "dir {d}");
        for (f, src) in s.frames.iter().zip(&dir.frames) {
            assert_eq!(
                (f.width, f.height, f.x_off, f.y_off, &f.pixels),
                (src.width, src.height, src.x_min, src.y_min, &src.pixels),
                "dir {d}"
            );
        }
    }
    let size: Vec<u64> = sets.iter().map(FrameSet::byte_size).collect();
    assert!(size.iter().all(|&s| s > 0));

    // Room for all four but one byte: loading dir 3 must evict exactly
    // dir 0 (frame 1, lowest key of the oldest frame).
    let mut pool: Pool<FrameSetKey, FrameSet> =
        Pool::new("frame sets", size.iter().sum::<u64>() - 1);
    let mut clock = Ticks(0);
    let mut loads = Vec::new();
    let load = |k: &FrameSetKey, loads: &mut Vec<FrameSetKey>| {
        loads.push(k.clone());
        let FramePart::Dir(d) = k.part() else {
            unreachable!()
        };
        FrameSet::from_dcc(&dcc, d).map(|s| {
            let b = s.byte_size();
            (s, b)
        })
    };

    pool.begin_frame(1).unwrap();
    let r = pool
        .resolve(&[key(0), key(1)], &mut clock, |k| load(k, &mut loads))
        .unwrap();
    assert_eq!((r.resident, r.stalls.len()), (0, 2));
    pool.begin_frame(2).unwrap();
    let r = pool
        .resolve(&[key(2), key(3)], &mut clock, |k| load(k, &mut loads))
        .unwrap();
    assert_eq!((r.resident, r.stalls.len()), (0, 2));
    let evicted: Vec<CacheEvent> = pool
        .drain_events()
        .into_iter()
        .filter(|e| matches!(e, CacheEvent::Evicted { .. }))
        .collect();
    assert_eq!(
        evicted,
        [CacheEvent::Evicted {
            pool: "frame sets",
            key: format!("{:?}", key(0)),
            bytes: size[0],
            last_used: 1,
        }]
    );
    assert_eq!(
        pool.keys().cloned().collect::<Vec<_>>(),
        [key(1), key(2), key(3)]
    );
    assert_eq!(pool.used(), size[1] + size[2] + size[3]);

    // Frame 3: everything it needs is resident; nothing loads.
    pool.begin_frame(3).unwrap();
    let before = loads.len();
    let r = pool
        .resolve(&[key(1), key(2), key(3)], &mut clock, |k| {
            load(k, &mut loads)
        })
        .unwrap();
    assert_eq!((r.resident, r.stalls.len(), loads.len()), (3, 0, before));
    assert_eq!(pool.peek(&key(2)), Some(&sets[2]));

    // Frame 4 over a 1-byte budget: the older entries go, dir 0 (used now)
    // stays, and the overrun is logged.
    pool.begin_frame(4).unwrap();
    pool.drain_events();
    pool.resolve(&[key(0)], &mut clock, |k| load(k, &mut loads))
        .unwrap();
    pool.set_budget(1);
    assert_eq!(pool.keys().cloned().collect::<Vec<_>>(), [key(0)]);
    assert_eq!(pool.used(), size[0]);
    let events = pool.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CacheEvent::Overrun { frame: 4, .. })),
        "{events:?}"
    );
    assert_eq!(pool.stalls().count, 5);
    assert_eq!(loads, [key(0), key(1), key(2), key(3), key(0)]);
}

/// Real frames, maps and items for the compositor tests: frames of a DCC
/// direction, a DC6 and DT1 tiles; shade rows from the act 1 PL2 light
/// levels and its three alpha blend tables; items overlapping, partly off
/// screen, some clipped, shaded or blended.
struct RealScene {
    items: Vec<DrawItem>,
    frames: Vec<FrameImage>,
    maps: MapTable,
    palette: Palette,
}

fn real_scene(set: &ArchiveSet) -> RealScene {
    let names = listed(set);
    let mut frames = Vec::new();
    let push = |s: FrameSet, limit: usize, frames: &mut Vec<FrameImage>| {
        for f in s.frames.into_iter().filter(|f| !f.is_empty()).take(limit) {
            frames.push(FrameImage {
                width: f.width,
                height: f.height,
                pixels: f.pixels,
            });
        }
    };
    let (_, dcc) = dcc_with_dirs(set, &names, 1);
    push(FrameSet::from_dcc(&dcc, 0).unwrap(), 12, &mut frames);
    let (_, dc6) = first_parsing(set, &names, ".dc6", |b| {
        Dc6::parse(b)
            .ok()
            .filter(|d| d.frames.iter().any(|f| !f.pixels.is_empty()))
    });
    for d in 0..dc6.header.directions.min(2) as u8 {
        push(FrameSet::from_dc6(&dc6, d).unwrap(), 6, &mut frames);
    }
    let (_, dt1) = first_parsing(set, &names, ".dt1", |b| Dt1::parse(b).ok());
    let mut tiles = 0;
    for t in 0..dt1.tiles.len() as u32 {
        let s = FrameSet::from_dt1(&dt1, t).unwrap();
        if !s.frames.is_empty() && tiles < 6 {
            tiles += 1;
            push(s, 1, &mut frames);
        }
    }
    assert!(frames.len() >= 8, "{} real frames", frames.len());

    let pl2 = Pl2::parse(&read(set, PL2)).unwrap();
    let mut maps = MapTable::new();
    let light: Vec<MapId> = pl2.light_levels.iter().map(|m| maps.push(*m)).collect();
    let hue: Vec<MapId> = pl2
        .hue_variations
        .iter()
        .take(4)
        .map(|m| maps.push(*m))
        .collect();
    let alpha: Vec<MapId> = pl2
        .alpha_blend
        .iter()
        .map(|level| {
            let table: Box<[[u8; 256]; 256]> = level
                .clone()
                .into_boxed_slice()
                .try_into()
                .expect("256 rows per alpha level");
            maps.push_table(&table)
        })
        .collect();

    let mut items = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let n = i as i32;
        let mut item = DrawItem::new(
            FrameId(i as u32),
            (n * 97) % 760 - f.width as i32 / 3,
            (n * 61) % 560 - f.height as i32 / 3,
        );
        item.shade = match i % 3 {
            0 => ShadeChain::EMPTY,
            1 => ShadeChain::new(&[light[i % light.len()]]).unwrap(),
            _ => ShadeChain::new(&[hue[i % hue.len()], light[(i * 7) % light.len()]]).unwrap(),
        };
        if i % 4 == 3 {
            item.blend = BlendOp::IndexTable(alpha[i % alpha.len()]);
        }
        if i % 5 == 4 {
            item.clip = Rect::new(100, 80, 500, 400);
        }
        items.push(item);
    }
    RealScene {
        items,
        frames,
        maps,
        palette: Palette::parse(&read(set, PALETTE)).unwrap(),
    }
}

/// The composition evaluated directly, pixel by pixel: items in list order; frame
/// pixels inside clip ∩ view; index 0 transparent before the chain;
/// `i' = m_k[…m_0[i]]`; `Opaque` or `dest = map[base + dest][i']`
/// (`IndexTable`: row = destination, column = source). Independent of
/// `scene` (no bins, no resolve).
fn evaluate(s: &RealScene, view: Rect) -> Vec<u8> {
    let mut out = vec![0u8; view.width as usize * view.height as usize];
    for item in &s.items {
        let f = &s.frames[item.frame.0 as usize];
        for fy in 0..f.height {
            for fx in 0..f.width {
                let (sx, sy) = (
                    i64::from(item.x) + i64::from(fx),
                    i64::from(item.y) + i64::from(fy),
                );
                if !item.clip.contains(sx, sy) || !view.contains(sx, sy) {
                    continue;
                }
                let src = f.pixels[(fy * f.width + fx) as usize];
                if src == 0 {
                    continue;
                }
                let i = item
                    .shade
                    .maps()
                    .iter()
                    .fold(src, |i, m| s.maps.get(*m).unwrap()[usize::from(i)]);
                let at = (sy - i64::from(view.y)) as usize * view.width as usize
                    + (sx - i64::from(view.x)) as usize;
                out[at] = match item.blend {
                    BlendOp::Opaque => i,
                    BlendOp::IndexTable(base) => {
                        s.maps.get(MapId(base.0 + u32::from(out[at]))).unwrap()[usize::from(i)]
                    }
                    BlendOp::IndexTableSrcRow(base) => {
                        s.maps.get(MapId(base.0 + u32::from(i))).unwrap()[usize::from(out[at])]
                    }
                };
            }
        }
    }
    out
}

// The composition on real frames and real PL2 maps: `scene::compose`
// equals the direct evaluation byte for byte; the binned walk equals it under
// the verify comparison (0 of N bytes, 0 RGBA pixels through the act 1
// palette); `--perturb 7` on the reference reports exactly 7.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn cpu_compositor_on_real_frames() {
    let s = real_scene(&set());
    let view = Rect::FRAME;
    let reference = scene::compose(&s.items, &s.frames, &s.maps, view).unwrap();
    let direct = evaluate(&s, view);
    let drawn = direct.iter().filter(|&&p| p != 0).count();
    println!(
        "{} items, {} map rows, {drawn} non-zero pixels",
        s.items.len(),
        s.maps.len()
    );
    assert!(drawn > 0, "the scene draws something");
    let m = verify::compare_indices(&reference, &direct, view).unwrap();
    assert_eq!(m.mismatched, 0, "compose vs direct: {m}");

    let bins = scene::bin(&s.items, &s.frames, &s.maps, view).unwrap();
    let binned = scene::compose_binned(&s.items, &bins, &s.frames, &s.maps, view).unwrap();
    let m = verify::compare_indices(&binned, &reference, view).unwrap();
    assert_eq!(m.mismatched, 0, "binned: {m}");
    let rgba = scene::to_rgba(&reference, &s.palette);
    let m = verify::compare(&scene::to_rgba(&binned, &s.palette), &rgba, view).unwrap();
    assert_eq!(m.mismatched, 0, "binned rgba: {m}");

    let mut perturbed = reference.clone();
    verify::perturb_indices(&mut perturbed, 7).unwrap();
    let m = verify::compare_indices(&binned, &perturbed, view).unwrap();
    assert_eq!(m.mismatched, 7, "{m}");
}

// The verify GPU half (`verify::gpu::Wgpu`) on the same real scene: index
// framebuffer and RGBA equal the CPU reference (0 differing), and a
// reference perturbed by 7 reports exactly 7 on both. Needs a GPU adapter
// as well as the game files; no adapter fails the test, never passes it.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR and a GPU adapter"]
fn gpu_compositor_on_real_frames() {
    let s = real_scene(&set());
    let view = Rect::FRAME;
    let mut gpu = Wgpu::new();
    let line = gpu.open();
    println!("{line}");
    assert!(line.starts_with("adapter: "), "{line}");
    let reference = scene::compose(&s.items, &s.frames, &s.maps, view).unwrap();
    let rgba = scene::to_rgba(&reference, &s.palette);
    let bins = scene::bin(&s.items, &s.frames, &s.maps, view).unwrap();
    let job = GpuJob {
        case: "real-frames",
        items: &s.items,
        bins: &bins,
        frames: &s.frames,
        maps: &s.maps,
        palette: &s.palette,
        view,
    };
    let GpuOutcome::Image {
        indices,
        rgba: gpu_rgba,
    } = gpu.compose(&job)
    else {
        panic!("GPU half did not produce an image");
    };
    let i = verify::compare_indices(&indices, &reference, view).unwrap();
    let m = verify::compare(&gpu_rgba, &rgba, view).unwrap();
    println!("GPU indices: {i}; GPU: {m}");
    assert_eq!((i.mismatched, m.mismatched), (0, 0));

    let mut perturbed = reference;
    verify::perturb_indices(&mut perturbed, 7).unwrap();
    let i = verify::compare_indices(&indices, &perturbed, view).unwrap();
    let m = verify::compare(&gpu_rgba, &scene::to_rgba(&perturbed, &s.palette), view).unwrap();
    assert_eq!(i.mismatched, 7, "{i}");
    println!("perturb 7 → RGBA {m}");
}
