//! Tests for the rendering facts: the file format, the first-difference compare and the
//! exporter. They run on hand-made fact sets, with no game values, only the shape of the
//! files (every number below is made up for the test).

use std::path::Path;

use super::compare::{compare, compare_dirs, FactSet, Outcome, Stage};
use super::export::{
    add_cycle_rows, draw_rows, frame_rows, ExportContext, FrameInputs, FrameState, Rows,
};
use super::*;
use crate::frames::{FrameAnchor, FramePart, FrameSet, FrameSetKey, FrameStore, IndexFrame};
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, MapId};

const HEAD: &str = "# facts v1; tool: test 0; command: hand-made; game: reference\n";

fn draws_text(rows: &[&str]) -> String {
    let mut s = format!("{HEAD}{}\n", DRAW_COLUMNS.join("\t"));
    for r in rows {
        s.push_str(&r.replace(' ', "\t"));
        s.push('\n');
    }
    s
}

fn frame_text(edit: &[(&str, &str)]) -> String {
    let mut s = format!("{HEAD}key\tvalue\n");
    for (k, _) in FRAME_KEYS {
        let v = edit.iter().find(|(e, _)| *e == k).map_or("1", |(_, v)| *v);
        s.push_str(&format!("{k}\t{v}\n"));
    }
    s
}

fn sprites_text(rows: &[&str]) -> String {
    let mut s = format!("{HEAD}{}\n", SPRITE_COLUMNS.join("\t"));
    for r in rows {
        s.push_str(&r.replace(' ', "\t"));
        s.push('\n');
    }
    s
}

const ROW0: &str = "0 unit - - - - 10 20 - - - - - 0xff - 1:7 0x401000";
const ROW1: &str = "1 CelDraw a/b.dc6 0 2 - 30 40 8 9 -1 8 5 0xff 0 - 0x401010";
const SPRITE: &str = "a/b.dc6 0 2 8 9 -1 8";

fn set(draws: &[&str], frame: &[(&str, &str)], sprites: Option<&[&str]>) -> FactSet {
    FactSet {
        draws: parse("draws", &draws_text(draws), &DRAW_COLUMNS).unwrap(),
        frame: parse("frame", &frame_text(frame), &FRAME_COLUMNS).unwrap(),
        sprites: sprites.map(|s| parse("sprites", &sprites_text(s), &SPRITE_COLUMNS).unwrap()),
    }
}

// Two identical fact sets match (exit code 0).
#[test]
fn identical_sets_match() {
    let a = set(&[ROW0, ROW1], &[], Some(&[SPRITE]));
    assert_eq!(compare(&a, &a.clone(), &[]), Outcome::Match);
    assert_eq!(Outcome::Match.exit_code(), 0);
}

// Changing any compared draw cell diverges at exactly that row and column (exit code 1).
#[test]
fn every_changed_draw_cell_is_reported_at_its_row_and_column() {
    let a = set(&[ROW0, ROW1], &[], Some(&[SPRITE]));
    let base: Vec<&str> = ROW1.split(' ').collect();
    // Change each compared cell of row 1 in turn.
    for (c, name) in DRAW_COLUMNS.iter().enumerate() {
        if matches!(*name, "i" | "at") {
            continue;
        }
        let mut cells = base.clone();
        cells[c] = "77";
        let changed = cells.join(" ");
        let b = set(&[ROW0, &changed], &[], Some(&[SPRITE]));
        match compare(&a, &b, &[]) {
            Outcome::Diverged(d) => {
                assert_eq!(
                    (d.stage, d.row, d.column.as_str()),
                    (Stage::Draws, 1, *name)
                );
                assert_eq!(d.d2rust.unwrap()[c], "77");
            }
            other => panic!("column {name}: {other:?}"),
        }
    }
}

// The informational columns and the ignored ones are never compared.
#[test]
fn info_and_ignored_columns_are_not_compared() {
    let a = set(&[ROW0, ROW1], &[("seq", "5")], Some(&[SPRITE]));
    let b = set(
        &[ROW0, &ROW1.replace("0x401010", "-").replace(" 5 ", " 6 ")],
        &[("seq", "9")],
        Some(&[SPRITE]),
    );
    assert!(matches!(compare(&a, &b, &[]), Outcome::Diverged(d) if d.column == "mode"));
    assert_eq!(compare(&a, &b, &["mode".to_owned()]), Outcome::Match);
}

// A shorter draw list diverges at the first missing row.
#[test]
fn a_shorter_list_diverges_at_the_first_missing_row() {
    let a = set(&[ROW0, ROW1], &[], Some(&[SPRITE]));
    let b = set(&[ROW0], &[], Some(&[SPRITE]));
    match compare(&a, &b, &[]) {
        Outcome::Diverged(d) => {
            assert_eq!((d.stage, d.row), (Stage::Draws, 1));
            assert!(d.original.is_some() && d.d2rust.is_none());
        }
        other => panic!("{other:?}"),
    }
}

// A `?` cell or a missing `sprites.tsv` makes the result partial (exit code 2), not a match.
#[test]
fn unknown_cells_and_missing_sprites_are_partial() {
    let a = set(&[ROW0, ROW1], &[], Some(&[SPRITE]));
    let b = set(&[ROW0, &ROW1.replace(" 5 ", " ? ")], &[], Some(&[SPRITE]));
    let out = compare(&a, &b, &[]);
    assert_eq!(out.exit_code(), 2);
    assert!(matches!(&out, Outcome::Partial(u) if u.get("mode") == Some(&1)));
    let c = set(&[ROW0, ROW1], &[], None);
    assert!(matches!(compare(&a, &c, &[]), Outcome::Partial(u) if u.contains_key("sprites.tsv")));
    // A `?` never hides a later measured difference.
    let d = set(
        &[&ROW0.replace(" 10 ", " ? "), &ROW1.replace(" 30 ", " 31 ")],
        &[],
        Some(&[SPRITE]),
    );
    assert!(matches!(compare(&a, &d, &[]), Outcome::Diverged(x) if x.row == 1 && x.column == "x"));
}

// Minimap cels the reference never rasterizes are left out of the compare.
#[test]
fn unmeasured_maximap_cels_are_left_out() {
    const MAXI: &str = "1 CelDrawClipped data/global/ui/automap/maximap.dc6 0 3 - 283 -29 ? ? ? ? ? ? ? - 0x4596d9";
    let d2rust = "1 CelDrawClipped data/global/ui/automap/maximap.dc6 0 3 - 283 -29 16 32 0 32 5 0xff 0 - automap";
    let a = set(&[ROW0, MAXI], &[], Some(&[SPRITE]));
    let b = set(&[ROW0, d2rust], &[], Some(&[SPRITE]));
    assert_eq!(compare(&a, &b, &[]), Outcome::Match);
    // A D2RUST minimap sprite the reference lacks is not counted either.
    let s = set(
        &[ROW0, d2rust],
        &[],
        Some(&[SPRITE, "data/global/ui/automap/maximap.dc6 0 3 16 32 0 32"]),
    );
    assert_eq!(compare(&a, &s, &[]), Outcome::Match);
    // The measured cells still have to match.
    let c = set(
        &[ROW0, &d2rust.replace(" 283 ", " 284 ")],
        &[],
        Some(&[SPRITE]),
    );
    assert!(matches!(compare(&a, &c, &[]), Outcome::Diverged(x) if x.row == 1 && x.column == "x"));
    // Other files' `?` cells stay unmeasured.
    let other = MAXI.replace("maximap.dc6", "minimap.dc6");
    let e = set(&[ROW0, &other], &[], Some(&[SPRITE]));
    let f = set(
        &[ROW0, &d2rust.replace("maximap.dc6", "minimap.dc6")],
        &[],
        Some(&[SPRITE]),
    );
    assert_eq!(compare(&e, &f, &[]).exit_code(), 2);
}

// A fallback cel's sprite row is checked against the sizes in the reference's draw rows.
#[test]
fn a_fallback_cel_sprite_is_compared_against_its_draw_rows() {
    const SHADOW: &str = "2 CelDrawShadow a/c.dcc 46 0 - 28 40 8 9 -1 8 - - - - 0x401020";
    const FELL: &str = "c/d.dcc 46 0 8 9 -1 8";
    // The reference's sprite table has no key for the fallback cel; its draw row is measured.
    let a = set(&[ROW0, ROW1, SHADOW], &[], Some(&[SPRITE]));
    let b = set(
        &[ROW0, ROW1, SHADOW],
        &[],
        Some(&[SPRITE, "a/c.dcc 46 0 8 9 -1 8"]),
    );
    assert_eq!(compare(&a, &b, &[]), Outcome::Match);
    // Each of the four columns is compared: one changed cell diverges there.
    for (c, name) in ["w", "h", "xoff", "yoff"].iter().enumerate() {
        let mut cells: Vec<String> = "a/c.dcc 46 0 8 9 -1 8"
            .split(' ')
            .map(str::to_owned)
            .collect();
        cells[3 + c] = "77".into();
        let row = cells.join(" ");
        let e = set(&[ROW0, ROW1, SHADOW], &[], Some(&[SPRITE, &row]));
        assert!(
            matches!(compare(&a, &e, &[]), Outcome::Diverged(x)
                if x.stage == Stage::Sprites && x.row == 1 && x.column == *name),
            "{name}"
        );
    }
    // No measured reference row of that key: still unmeasured.
    let f = set(&[ROW0, ROW1, SHADOW], &[], Some(&[SPRITE, FELL]));
    assert!(
        matches!(compare(&a, &f, &[]), Outcome::Partial(u) if u.get("sprite rows") == Some(&1))
    );
    let g = set(
        &[ROW0, ROW1, &SHADOW.replace(" 8 9 -1 8 ", " ? ? ? ? ")],
        &[],
        Some(&[SPRITE]),
    );
    assert_eq!(compare(&g, &b, &[]).exit_code(), 2);
}

// The first difference is the cause: inputs, then sprites, then draws, then outputs.
#[test]
fn causes_are_reported_before_effects() {
    let a = set(&[ROW0, ROW1], &[], Some(&[SPRITE]));
    // Input key, sprite, draw and output all differ: the input wins.
    let b = set(
        &[ROW0, &ROW1.replace(" 30 ", " 31 ")],
        &[("level", "2"), ("index_sha256", "ab")],
        Some(&["a/b.dc6 0 2 8 9 -2 8"]),
    );
    let first = |b: &FactSet| match compare(&a, b, &[]) {
        Outcome::Diverged(d) => (d.stage, d.column),
        other => panic!("{other:?}"),
    };
    assert_eq!(first(&b), (Stage::FrameInput, "level".into()));
    let b = set(
        &b_rows(),
        &[("index_sha256", "ab")],
        Some(&["a/b.dc6 0 2 8 9 -2 8"]),
    );
    assert_eq!(first(&b), (Stage::Sprites, "xoff".into()));
    let b = set(&b_rows(), &[("index_sha256", "ab")], Some(&[SPRITE]));
    assert_eq!(first(&b), (Stage::Draws, "x".into()));
    let b = set(&[ROW0, ROW1], &[("index_sha256", "ab")], Some(&[SPRITE]));
    assert_eq!(first(&b), (Stage::FrameOutput, "index_sha256".into()));
}

fn b_rows() -> Vec<&'static str> {
    vec![
        ROW0,
        "1 CelDraw a/b.dc6 0 2 - 31 40 8 9 -1 8 5 0xff 0 - 0x401010",
    ]
}

// Malformed files are errors.
#[test]
fn malformed_files_are_errors() {
    let bad_header = draws_text(&[ROW0]).replacen("v1", "v2", 1);
    assert!(parse("d", &bad_header, &DRAW_COLUMNS).is_err());
    let mut swapped = DRAW_COLUMNS;
    swapped.swap(6, 7);
    let text = format!(
        "{HEAD}{}\n{}\n",
        swapped.join("\t"),
        ROW0.replace(' ', "\t")
    );
    assert!(parse("d", &text, &DRAW_COLUMNS).is_err());
    let short = draws_text(&["0 unit -"]);
    assert!(parse("d", &short, &DRAW_COLUMNS).is_err());
    assert!(parse("d", draws_text(&[ROW0]).trim_end(), &DRAW_COLUMNS).is_err());
    let frame = frame_text(&[]).replace("rain\t", "snowy\t");
    let t = parse("f", &frame, &FRAME_COLUMNS).unwrap();
    assert!(check_frame_keys("f", &t).is_err());
    let ok = parse("f", &frame_text(&[]), &FRAME_COLUMNS).unwrap();
    assert!(check_frame_keys("f", &ok).is_ok());
}

// The header line round-trips through write and parse.
#[test]
fn header_round_trips() {
    let h = Header {
        tool: "t 1".into(),
        command: "py x.py --a 1; b".into(),
        game: "reference".into(),
    };
    let text = write(&h, &SPRITE_COLUMNS, &[]);
    assert_eq!(parse("s", &text, &SPRITE_COLUMNS).unwrap().header, h);
}

// Directories compare, with the reference side falling back to the shared `sprites.tsv`.
#[test]
fn directories_compare_with_the_shared_sprites_file() {
    let root = std::env::temp_dir().join(format!("facts-compare-{}", std::process::id()));
    let scene = root.join("render/scenes/s1");
    let ours = root.join("d2rust");
    let write = |dir: &Path, name: &str, text: String| {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    };
    write(&scene, DRAWS_FILE, draws_text(&[ROW0, ROW1]));
    write(&scene, FRAME_FILE, frame_text(&[]));
    write(&root.join("render"), SPRITES_FILE, sprites_text(&[SPRITE]));
    write(&ours, DRAWS_FILE, draws_text(&[ROW0, ROW1]));
    write(&ours, FRAME_FILE, frame_text(&[]));
    write(&ours, SPRITES_FILE, sprites_text(&[SPRITE]));
    assert_eq!(
        compare_dirs(&scene, &ours, &[], false).unwrap(),
        Outcome::Match
    );
    assert!(compare_dirs(&scene, &root.join("missing"), &[], false).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}

fn store() -> FrameStore {
    let mut s = FrameStore::new();
    let dc6 = IndexFrame::new(4, 3, -2, 10, vec![1; 12])
        .unwrap()
        .with_anchor(FrameAnchor::Bottom);
    let dcc = IndexFrame::new(2, 5, 3, -6, vec![1; 10]).unwrap();
    let key = |p: &str, part| FrameSetKey::new(p, part).unwrap();
    s.insert(
        key("x/ui.dc6", FramePart::Dir(0)),
        FrameSet { frames: vec![dc6] },
    )
    .unwrap();
    s.insert(
        key("x/unit.dcc", FramePart::Dir(3)),
        FrameSet { frames: vec![dcc] },
    )
    .unwrap();
    let tile = IndexFrame::new(160, 80, 0, -16, vec![1; 160 * 80]).unwrap();
    s.insert(
        key("x/floor.dt1", FramePart::Tile(7)),
        FrameSet { frames: vec![tile] },
    )
    .unwrap();
    let pixel = IndexFrame::new(1, 1, 0, 0, vec![1]).unwrap();
    s.insert(
        key("d2rust/weather/pixel", FramePart::Tile(0)),
        FrameSet {
            frames: vec![pixel],
        },
    )
    .unwrap();
    s
}

// Export inverts placement back to the draw call's position and merges a tile's block draws
// into one row.
#[test]
fn export_rows_invert_placement_and_merge_tile_blocks() {
    use crate::rules::placement::place;
    use crate::scene::order::pass;
    use crate::scene::{FrameId, Rect};
    let s = store();
    let at = |id: u32, x: i32, y: i32| {
        // Items placed by the forward placement rule.
        let f = s.frame(FrameId(id)).unwrap();
        let p = place(f, x, y, Rect::FRAME);
        DrawItem::new(FrameId(id), p.x, p.y)
    };
    let mut floor = at(2, 100, 200);
    floor.key = DrawKey::new(pass::FLOORS, 0, 0, 0).unwrap();
    floor.tag = ItemTag::Tile { x: 1, y: 2 };
    let mut floor_block = floor;
    floor_block.clip = Rect::new(0, 0, 32, 15);
    let mut unit = at(1, 50, 60);
    unit.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    unit.tag = ItemTag::Unit(9);
    let mut ui = at(0, 300, 400);
    ui.key = DrawKey::new(pass::UI, 0, 0, 0).unwrap();
    let unit_type = |g: u32| (g == 9).then_some(1u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(5),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[floor, floor_block, unit, ui], &cx).unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r[..8].join(" ")).collect();
    assert_eq!(
        cols,
        [
            "0 FloorTileDraw x/floor.dt1 - 7 ? 175 200",
            "1 unit - - - - ? ?",
            "2 CelDraw x/unit.dcc 3 0 - 50 60",
            "3 CelDraw x/ui.dc6 0 0 - 300 400",
        ]
    );
    // A DCC box's yoff names its bottom row; a DC6 keeps its own.
    assert_eq!(rows.draws[2][8..12], ["2", "5", "3", "-2"]);
    assert_eq!(rows.draws[1][15], "1:9");
    assert_eq!(
        rows.sprites,
        [
            vec!["x/ui.dc6", "0", "0", "4", "3", "-2", "10"],
            vec!["x/unit.dcc", "3", "0", "2", "5", "3", "-2"],
        ]
    );
    for r in &rows.draws {
        assert_eq!(r.len(), DRAW_COLUMNS.len());
    }
}

// The exported `frame.tsv` has every key, in order.
#[test]
fn exported_frame_rows_have_every_key_in_order() {
    let rows = frame_rows(&FrameState {
        seq: 1,
        tick: 2,
        width: 800,
        height: 600,
        act: Some(0),
        level: None,
        camera: None,
        open_mode: Some(0),
        inputs: FrameInputs::default(),
        draws: 3,
        index_sha256: None,
        palette_sha256: sha256_hex(&[]),
    });
    let h = Header {
        tool: "t".into(),
        command: "c".into(),
        game: "d2rust".into(),
    };
    let t = parse("f", &write(&h, &FRAME_COLUMNS, &rows), &FRAME_COLUMNS).unwrap();
    check_frame_keys("f", &t).unwrap();
    let get = |k: &str| t.rows.iter().find(|r| r[0] == k).unwrap()[1].clone();
    assert_eq!(
        (get("tick"), get("level"), get("draws")),
        ("2".into(), "?".into(), "3".into())
    );
    assert_eq!(
        get("palette_sha256"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

/// StartDraw comes first (x = BlankScreen, y = 0), ClearScreen(0) last only when the plan clears
/// after drawing; rows are renumbered.
#[test]
fn cycle_rows_frame_the_items() {
    let item = |i: &str| {
        let mut r = vec!["-".to_owned(); 17];
        r[0] = i.into();
        r[1] = "CelDraw".into();
        r
    };
    let mut rows = Rows {
        draws: vec![item("0"), item("1")],
        sprites: vec![],
    };
    add_cycle_rows(&mut rows, true, false);
    let ops: Vec<_> = rows
        .draws
        .iter()
        .map(|r| (r[0].as_str(), r[1].as_str()))
        .collect();
    assert_eq!(
        ops,
        [("0", "StartDraw"), ("1", "CelDraw"), ("2", "CelDraw")]
    );
    assert_eq!(
        rows.draws[0].join("\t"),
        "0\tStartDraw\t-\t-\t-\t-\t1\t0\t-\t-\t-\t-\t-\t-\t-\t-\t-"
    );
    let mut rows = Rows {
        draws: vec![item("0")],
        sprites: vec![],
    };
    add_cycle_rows(&mut rows, false, true);
    assert_eq!(rows.draws[0][6], "0");
    assert_eq!(
        rows.draws[2].join("\t"),
        "2\tClearScreen\t-\t-\t-\t-\t0\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-"
    );
}

/// Pass 9's calls are one row each (`DrawLine` x0, y0, color; the flash `DrawBox` color 255)
/// where their pixels stood; the pixel items themselves write no row. With every pixel
/// off-screen the rows stand before the first later pass, and with no later pass at the end.
#[test]
fn sky_calls_replace_their_pixel_items() {
    use crate::rules::draw_order::weather::SkyDraw;
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let item = |id: u32, p: u32| {
        let mut i = DrawItem::new(FrameId(id), 0, 0);
        i.key = DrawKey::new(p, 0, 0, 0).unwrap();
        i
    };
    let line = |x0, y0, color| SkyDraw::Line {
        x0,
        y0,
        x1: x0 + 1,
        y1: y0 + 5,
        color,
        alpha: 127,
    };
    let sky = [
        line(596, 151, 185),
        line(355, -3, 198),
        SkyDraw::Flash {
            x0: 0,
            y0: 0,
            x1: 800,
            y1: 553,
        },
    ];
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: None,
        unit_type: &unit_type,
        sky: &sky,
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    // Item rows by op; call rows with x, y and mode.
    let rows = |items: &[DrawItem]| -> Vec<String> {
        draw_rows(items, &cx)
            .unwrap()
            .draws
            .iter()
            .map(|r| match r[1].as_str() {
                "DrawLine" | "DrawBox" => format!("{} {} {} {} {}", r[0], r[1], r[6], r[7], r[12]),
                _ => format!("{} {}", r[0], r[1]),
            })
            .collect()
    };
    let calls = |at: usize| {
        vec![
            format!("{} DrawLine 596 151 185", at),
            format!("{} DrawLine 355 -3 198", at + 1),
            format!("{} DrawBox 0 0 255", at + 2),
        ]
    };
    let unit = item(1, pass::WALLS_UNITS);
    let ui = item(0, pass::UI);
    // Pixels present: the calls stand where the first pixel was.
    let pixel = item(3, pass::UNIDENTIFIED_9);
    let mut want = vec!["0 CelDraw".to_owned()];
    want.extend(calls(1));
    want.push("4 CelDraw".into());
    assert_eq!(rows(&[unit, pixel, pixel, ui]), want);
    // Every pixel off-screen: the same place, before the first later pass.
    assert_eq!(rows(&[unit, ui]), want);
    // No later pass: at the end.
    let mut want = vec!["0 CelDraw".to_owned()];
    want.extend(calls(1));
    assert_eq!(rows(&[unit]), want);
    // No calls: pixel-free lists are unchanged.
    let cx = ExportContext { sky: &[], ..cx };
    assert_eq!(draw_rows(&[unit, ui], &cx).unwrap().draws.len(), 2);
}

/// The blocks of one tile, each its own frame of the tile's set with the block's offsets, give
/// one row; the next tile's blocks another.
#[test]
fn block_frames_of_one_tile_are_one_row() {
    use crate::scene::order::pass;
    let mut s = FrameStore::new();
    let block = |x_off| IndexFrame::new(32, 15, x_off, 0, vec![1; 32 * 15]).unwrap();
    s.insert(
        FrameSetKey::new("x/f.dt1", FramePart::Tile(3)).unwrap(),
        FrameSet {
            frames: vec![block(0), block(32), block(64)],
        },
    )
    .unwrap();
    let ids: Vec<_> = (0..3)
        .map(|i| {
            s.id(&FrameSetKey::new("x/f.dt1", FramePart::Tile(3)).unwrap(), i)
                .unwrap()
        })
        .collect();
    let item = |id, x: i32, tile: (i32, i32)| {
        let mut i = DrawItem::new(id, x, 40);
        i.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
        i.tag = ItemTag::Tile {
            x: tile.0,
            y: tile.1,
        };
        i
    };
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(
        &[
            item(ids[0], 100, (1, 1)),
            item(ids[1], 132, (1, 1)),
            item(ids[2], 164, (1, 1)),
            item(ids[0], 260, (2, 1)),
            item(ids[1], 292, (2, 1)),
        ],
        &cx,
    )
    .unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r[..8].join(" ")).collect();
    assert_eq!(
        cols,
        [
            "0 TileDrawLit x/f.dt1 - 3 ? 100 40",
            "1 TileDrawLit x/f.dt1 - 3 ? 260 40",
        ]
    );
    // Two records of one cell with the same tile (two draw keys, the record's list position)
    // are two calls: one lower wall is drawn twice.
    let twice = |id, x: i32, minor| {
        let mut i = item(id, x, (1, 1));
        i.key = DrawKey::new(pass::WALLS_UNITS, 0, minor, 0).unwrap();
        i
    };
    let rows = draw_rows(
        &[
            twice(ids[0], 100, 0),
            twice(ids[1], 132, 0),
            twice(ids[0], 100, 1),
            twice(ids[1], 132, 1),
        ],
        &cx,
    )
    .unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r[..8].join(" ")).collect();
    assert_eq!(
        cols,
        [
            "0 TileDrawLit x/f.dt1 - 3 ? 100 40",
            "1 TileDrawLit x/f.dt1 - 3 ? 100 40",
        ]
    );
}

/// A translucent wall's `mode` is its alpha byte, the `TileDrawTrans` alpha argument; `?` when
/// it is not measured.
#[test]
fn translucent_wall_mode_is_its_alpha_byte() {
    use crate::scene::order::pass;
    let mut s = FrameStore::new();
    let key = FrameSetKey::new("x/w.dt1", FramePart::Tile(1)).unwrap();
    s.insert(
        key.clone(),
        FrameSet {
            frames: vec![IndexFrame::new(32, 15, 0, 0, vec![1; 32 * 15]).unwrap()],
        },
    )
    .unwrap();
    let wall = |minor| {
        let mut i = DrawItem::new(s.id(&key, 0).unwrap(), 100, 40);
        i.key = DrawKey::new(pass::WALLS_UNITS, 0, minor, 0).unwrap();
        i.tag = ItemTag::Tile {
            x: 1,
            y: minor as i32,
        };
        i.blend = BlendOp::IndexTableSrcRow(MapId(0));
        i
    };
    let alphas = std::collections::BTreeMap::from([(wall(0).key, 0x80u8)]);
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &alphas,
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[wall(0), wall(1)], &cx).unwrap();
    let modes: Vec<_> = rows
        .draws
        .iter()
        .map(|r| format!("{} {}", r[1], r[12]))
        .collect();
    assert_eq!(modes, ["TileDrawTrans 128", "TileDrawTrans ?"]);
}

/// A unit's shadow (pass 5) writes no unit row (the shadow pass has no unit draw) and names the
/// unit's own cel and its size, not the derived sheared `#shadow` frame; its X, Y are not
/// measured. The unit's own run (pass 6) still starts with its unit row.
#[test]
fn unit_shadows_name_the_cel_and_write_no_unit_row() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let mut s = store();
    let sheared = IndexFrame::new(7, 2, -5, 0, vec![1; 14]).unwrap();
    s.insert(
        FrameSetKey::new("x/unit.dcc#shadow", FramePart::Dir(3)).unwrap(),
        FrameSet {
            frames: vec![sheared],
        },
    )
    .unwrap();
    let shadow_id = s
        .id(
            &FrameSetKey::new("x/unit.dcc#shadow", FramePart::Dir(3)).unwrap(),
            0,
        )
        .unwrap();
    let mut shadow = DrawItem::new(shadow_id, 40, 50);
    shadow.key = DrawKey::new(pass::SHADOWS, 0, 0, 0).unwrap();
    shadow.tag = ItemTag::Unit(9);
    let mut body = DrawItem::new(FrameId(1), 50, 60);
    body.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    body.tag = ItemTag::Unit(9);
    let unit_type = |g: u32| (g == 9).then_some(1u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[shadow, body], &cx).unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r[..12].join(" ")).collect();
    assert_eq!(
        cols,
        [
            // The shadow call's (X, Y) is the unit's X − 2, Y.
            "0 CelDrawShadow x/unit.dcc 3 0 - 43 50 2 5 3 -2",
            "1 unit - - - - ? ? - - - -",
            &format!(
                "2 CelDraw x/unit.dcc 3 0 - {} {} 2 5 3 -2",
                rows.draws[2][6], rows.draws[2][7]
            ),
        ]
    );
    assert_eq!(
        rows.sprites,
        [vec!["x/unit.dcc", "3", "0", "2", "5", "3", "-2"]]
    );
}

/// A cel in the shadow pass that is not a derived `#shadow` (an object drawn under the units,
/// `DrawUnder`, such as a corpse) is a unit draw: its unit row, then `CelDraw`.
#[test]
fn a_unit_cel_drawn_in_the_shadow_pass_is_a_unit_draw() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let mut under = DrawItem::new(FrameId(1), 50, 60);
    under.key = DrawKey::new(pass::SHADOWS, 0, 0, 0).unwrap();
    under.tag = ItemTag::Unit(21);
    let unit_type = |g: u32| (g == 21).then_some(2u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[under], &cx).unwrap();
    let ops: Vec<&str> = rows.draws.iter().map(|r| r[1].as_str()).collect();
    assert_eq!(ops, ["unit", "CelDraw"]);
    assert_eq!(rows.draws[0][15], "2:21");
}

/// A unit cel's `dir` (and its `sprites.tsv` key) is the cel context's `dir64`
/// (`WorldFrame::unit_dirs`), not the file direction of its frame set; a cel of no drawn unit
/// keeps the file direction.
#[test]
fn unit_cel_dir_is_the_context_dir64() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let mut body = DrawItem::new(FrameId(1), 50, 60);
    body.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    body.tag = ItemTag::Unit(9);
    let mut other = body;
    other.tag = ItemTag::Unit(8);
    other.key = DrawKey::new(pass::WALLS_UNITS, 0, 1, 0).unwrap();
    let unit_type = |_: u32| Some(1u8);
    // Keyed by the unit's draw slot (a GUID repeats across unit types).
    let dirs = std::collections::BTreeMap::from([(body.key.slot(), 62u8)]);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &dirs,
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[body, other], &cx).unwrap();
    let dirs: Vec<&str> = rows
        .draws
        .iter()
        .filter(|r| r[1] == "CelDraw")
        .map(|r| r[3].as_str())
        .collect();
    assert_eq!(dirs, ["62", "3"]);
    assert_eq!(rows.sprites[0][1], "3");
    assert_eq!(rows.sprites[1][1], "62");
}

/// A cel call without pixels (a component file in no archive) is a row at its key: the unit's
/// run starts with its unit row even when the call comes first; the shadow call writes no unit
/// row; neither has a size or a `sprites.tsv` row.
#[test]
fn unit_calls_are_rows_at_their_keys() {
    use crate::assets::path::CanonicalPath;
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    use crate::world_view::UnitCall;
    let s = store();
    let mut body = DrawItem::new(FrameId(1), 50, 60);
    body.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 1).unwrap();
    body.tag = ItemTag::Unit(9);
    let call = |pass, sub, shadow| UnitCall {
        key: DrawKey::new(pass, 0, 0, sub).unwrap(),
        tag: ItemTag::Unit(9),
        path: Some(CanonicalPath::new("x/sh.dcc").unwrap()),
        dir64: 0,
        frame: 6,
        shadow,
        call: None,
    };
    let calls = [
        call(pass::SHADOWS, 0, true),
        call(pass::WALLS_UNITS, 0, false),
    ];
    let unit_type = |_: u32| Some(0u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &calls,
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[body], &cx).unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r[..12].join(" ")).collect();
    assert_eq!(
        cols,
        [
            // No cel: no size; the body call takes the draw's (X, Y).
            "0 CelDrawShadow x/sh.dcc 0 6 - ? ? - - - -",
            "1 unit - - - - ? ? - - - -",
            &format!(
                "2 CelDraw x/sh.dcc 0 6 - {} {} - - - -",
                rows.draws[3][6], rows.draws[3][7]
            ),
            &format!(
                "3 CelDraw x/unit.dcc 3 0 - {} {} 2 5 3 -2",
                rows.draws[3][6], rows.draws[3][7]
            ),
        ]
    );
    assert_eq!(rows.sprites.len(), 1);
}

/// A call without pixels takes the (X, Y) of the unit's composite cels, not of an overlay cel
/// drawn before them at its own (X, Y): here the overlay sits at 404 and the body's empty
/// layer must take the body's 400.
#[test]
fn a_no_pixel_call_skips_overlay_cels() {
    use crate::assets::path::CanonicalPath;
    use crate::scene::order::pass;
    use crate::world_view::UnitCall;
    let mut s = store();
    let ov = s
        .insert(
            FrameSetKey::new("data/global/overlays/aura.dcc", FramePart::Dir(0)).unwrap(),
            FrameSet {
                frames: vec![IndexFrame::new(2, 5, 3, -6, vec![1; 10]).unwrap()],
            },
        )
        .unwrap();
    let unit = s
        .id(
            &FrameSetKey::new("x/unit.dcc", FramePart::Dir(3)).unwrap(),
            0,
        )
        .unwrap();
    let item = |frame, x, sub| {
        let mut it = DrawItem::new(frame, x, 60);
        it.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, sub).unwrap();
        it.tag = ItemTag::Unit(9);
        it
    };
    // The overlay first, 4 px right of the body.
    let items = [item(ov, 54, 0), item(unit, 50, 1)];
    let calls = [UnitCall {
        key: DrawKey::new(pass::WALLS_UNITS, 0, 0, 2).unwrap(),
        tag: ItemTag::Unit(9),
        path: Some(CanonicalPath::new("x/sh.dcc").unwrap()),
        dir64: 0,
        frame: 4,
        shadow: false,
        call: None,
    }];
    let unit_type = |_: u32| Some(0u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &calls,
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
        hit_flash: &std::collections::BTreeMap::new(),
    };
    let rows = draw_rows(&items, &cx).unwrap();
    let xy = |file: &str| {
        let r = rows.draws.iter().find(|r| r[2] == file).unwrap();
        (r[6].clone(), r[7].clone())
    };
    assert_eq!(xy("x/sh.dcc"), xy("x/unit.dcc"));
    assert_ne!(xy("x/sh.dcc"), xy("data/global/overlays/aura.dcc"));
}

/// `--skip-weather`: the reference's pass 9 rows (by call site) and D2RUST's (`at` = `pass9`)
/// drop out on both sides; other lines and boxes stay.
#[test]
fn skip_weather_drops_pass9_rows_only() {
    use super::compare::{is_weather_row, PASS9_TAG};
    let row = |op: &str, at: &str| -> Vec<String> {
        let mut r = vec!["-".to_owned(); DRAW_COLUMNS.len()];
        r[1] = op.into();
        r[16] = at.into();
        r
    };
    assert!(is_weather_row(&row("DrawLine", "0x47368e")));
    assert!(is_weather_row(&row("DrawLine", "0x473585")));
    assert!(is_weather_row(&row("DrawBox", "0x473a00")));
    assert!(is_weather_row(&row("DrawLine", PASS9_TAG)));
    // The range's edges, another op, other call sites.
    assert!(!is_weather_row(&row("DrawLine", "0x47346f")));
    assert!(!is_weather_row(&row("DrawBox", "0x473f50")));
    assert!(!is_weather_row(&row("CelDraw", "0x47368e")));
    assert!(!is_weather_row(&row("DrawBox", "0x46efe9")));
    assert!(!is_weather_row(&row("DrawLine", "0x45a841")));
    assert!(!is_weather_row(&row("DrawLine", "-")));

    let rain = "9 DrawLine - - - - 51 336 - - - - 200 - - - 0x47368e";
    let ours = "9 DrawLine - - - - 99 1 - - - - 7 - - - pass9";
    let a = set(&[ROW0, rain, ROW1], &[], Some(&[SPRITE]));
    let b = set(&[ROW0, ROW1, ours, ours], &[], Some(&[SPRITE]));
    assert!(matches!(compare(&a, &b, &[]), Outcome::Diverged(_)));
    let (a, b) = (a.without_weather(), b.without_weather());
    assert_eq!(compare(&a, &b, &[]), Outcome::Match);
    // a non-weather box stays and is compared
    let bx = "9 DrawBox - - - - 273 573 - - - - 109 - - - 0x46efe9";
    let a = set(&[ROW0, bx, ROW1], &[], Some(&[SPRITE])).without_weather();
    let b = set(&[ROW0, ROW1], &[], Some(&[SPRITE])).without_weather();
    assert!(matches!(compare(&a, &b, &[]), Outcome::Diverged(d) if d.row == 1));
}

/// Pass 4's pool cels (the reference's `CelDraw` from the pool draw, D2RUST's `at` = `pools`)
/// drop out with the weather; other cels and pool-range lines stay.
#[test]
fn skip_weather_drops_the_pool_cels() {
    use super::compare::{is_weather_row, POOLS_TAG};
    let row = |op: &str, at: &str| -> Vec<String> {
        let mut r = vec!["-".to_owned(); DRAW_COLUMNS.len()];
        r[1] = op.into();
        r[16] = at.into();
        r
    };
    assert!(is_weather_row(&row("CelDraw", "0x473bd0")));
    assert!(is_weather_row(&row("CelDraw", "0x473a70")));
    assert!(is_weather_row(&row("CelDraw", POOLS_TAG)));
    assert!(!is_weather_row(&row("CelDraw", "0x473c00")));
    assert!(!is_weather_row(&row("CelDraw", "0x473a6f")));
    assert!(!is_weather_row(&row("CelDraw", "0x4713eb")));
    assert!(!is_weather_row(&row("CelDrawShadow", "0x473bd0")));
    assert!(!is_weather_row(&row("DrawLine", POOLS_TAG)));
    let splash = "9 CelDraw ? 0 6 - 41 305 ? ? ? ? 3 0xffffffff 0 - 0x473bd0";
    let ours =
        "9 CelDraw data/global/uncompoverlays/rain3.dc6 0 1 - 720 56 11 5 -5 1 ? ? ? - pools";
    let a = set(&[ROW0, splash, ROW1], &[], Some(&[SPRITE])).without_weather();
    let b = set(&[ROW0, ours, ours, ROW1], &[], Some(&[SPRITE])).without_weather();
    assert_eq!(compare(&a, &b, &[]), Outcome::Match);
}

/// A UI rectangle (a `d2rust/ui/rect/WxH` item) is a `DrawBox` row: left, top and the colour (its
/// colour row minus the first one); without the colour rows the colour is `?`.
#[test]
fn ui_rectangles_are_drawbox_rows() {
    use crate::scene::{MapId, ShadeChain};
    let mut s = FrameStore::new();
    let key = crate::world_view::ui_bind::rect_key(29, 29);
    s.insert(
        key.clone(),
        FrameSet {
            frames: vec![IndexFrame::new(29, 29, 0, 0, vec![1; 29 * 29]).unwrap()],
        },
    )
    .unwrap();
    let mut item = DrawItem::new(s.id(&key, 0).unwrap(), 198, 199);
    item.shade = ShadeChain::new(&[MapId(10 + 234)]).unwrap();
    item.tag = ItemTag::Ui(0);
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: None,
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: Some(MapId(10)),
        ui_calls: &[],
        text_maps: &[],
    };
    let mut next = item;
    next.x = 227;
    let rows = draw_rows(&[item, next], &cx).unwrap();
    let cols: Vec<String> = rows.draws.iter().map(|r| r.join(" ")).collect();
    assert_eq!(
        cols,
        [
            "0 DrawBox - - - - 198 199 - - - - 234 - - - -",
            "1 DrawBox - - - - 227 199 - - - - 234 - - - -",
        ]
    );
    assert!(rows.sprites.is_empty());
    let cx = ExportContext {
        color_rows: None,
        ..cx
    };
    assert_eq!(draw_rows(&[item], &cx).unwrap().draws[0][12], "?");
}

/// A listed unit whose body fails the pre-test is the unit draw call alone: its `unit` row and
/// no cel row.
#[test]
fn a_culled_body_is_its_unit_row_alone() {
    use crate::scene::order::pass;
    let s = store();
    let calls = [crate::world_view::UnitCall {
        key: DrawKey::new(pass::WALLS_UNITS, 3, 0, 0).unwrap(),
        tag: ItemTag::Unit(9),
        path: None,
        dir64: 0,
        frame: 0,
        shadow: false,
        call: None,
    }];
    let unit_type = |_: u32| Some(2u8);
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &calls,
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[], &cx).unwrap();
    let ops: Vec<(&str, &str)> = rows
        .draws
        .iter()
        .map(|r| (r[1].as_str(), r[15].as_str()))
        .collect();
    assert_eq!(ops, [("unit", "2:9")]);
}

/// A UI cel's op is the draw call its UI code made (`WorldFrame::ui_calls` by the item's UI
/// index); a UI cel with no entry, and every world cel, stay `CelDraw`.
#[test]
fn a_ui_cel_writes_its_wrapper_op() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    use crate::ui::draw::CelCall;
    let s = store();
    let ui = |i: u32, x: i32| {
        let mut it = DrawItem::new(FrameId(1), x, 60);
        it.key = DrawKey::new(pass::UI, 0, i, 0).unwrap();
        it.tag = ItemTag::Ui(i);
        it
    };
    let unit_type = |_: u32| None;
    let info = |call| crate::ui::draw::UiCelInfo {
        call,
        mode: 5,
        pal: Some(0),
        text: false,
    };
    let calls = [info(CelCall::Ex), info(CelCall::Color)];
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &calls,
        text_maps: &[],
    };
    let rows = draw_rows(&[ui(0, 10), ui(1, 20), ui(2, 30)], &cx).unwrap();
    let ops: Vec<&str> = rows.draws.iter().map(|r| r[1].as_str()).collect();
    assert_eq!(ops, ["CelDrawEx", "CelDrawColor", "CelDraw"]);
}

/// `--dump-image` writes the index bytes as they are, with the palette as `PLTE` (the capture
/// decoder reads them back unchanged).
#[test]
fn dump_image_keeps_indices_and_palette() {
    let palette: Vec<u8> = (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect();
    let pixels: Vec<u8> = (0..12u8).map(|i| i * 21).collect();
    let png = export::indexed_png(4, 3, &pixels, &palette).unwrap();
    let image = crate::verify::capture::decode_png(&png).unwrap();
    assert_eq!((image.width, image.height), (4, 3));
    assert_eq!(image.indices, pixels);
    assert_eq!(image.palette, palette);
}

/// One tick dumps into DIR, several into DIR/tick-N.
#[test]
fn dump_dirs_per_tick() {
    let req = |at_ticks: Vec<u64>| export::DumpRequest {
        dir: "d".into(),
        at_ticks,
        image: false,
        native: Vec::new(),
        command: String::new(),
    };
    assert_eq!(req(vec![73]).dir_for(0), Path::new("d"));
    let r = req(vec![2, 73]);
    assert_eq!(
        (r.dir_for(0), r.dir_for(1)),
        (Path::new("d/tick-2").into(), Path::new("d/tick-73").into())
    );
}

/// One unit draw is one draw-order slot, so two units sharing a GUID (monster 1:3, object 2:3)
/// in adjacent slots are two runs, each with its `unit` row.
#[test]
fn a_guid_shared_by_two_slots_starts_two_unit_runs() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let mut first = DrawItem::new(FrameId(1), 50, 60);
    first.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    first.tag = ItemTag::Unit(3);
    let mut second = first;
    second.key = DrawKey::new(pass::WALLS_UNITS, 0, 1, 0).unwrap();
    second.x = 70;
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[first, second], &cx).unwrap();
    let ops: Vec<&str> = rows.draws.iter().map(|r| r[1].as_str()).collect();
    assert_eq!(ops, ["unit", "CelDraw", "unit", "CelDraw"]);
}

/// A unit cel wholly off the frame keeps its size: the cel's header is resolved before the
/// driver's screen test.
#[test]
fn an_off_screen_unit_cel_keeps_its_size() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let mut item = DrawItem::new(FrameId(1), 2000, 60);
    item.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    item.tag = ItemTag::Unit(3);
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[item], &cx).unwrap();
    let cel = rows.draws.iter().find(|r| r[1] == "CelDraw").unwrap();
    assert!(cel[8..12].iter().all(|c| c != "-" && c != "?"), "{cel:?}");
}

/// A pass 4 pool cel is a `CelDraw` with draw mode 3, light −1 (unlit) and a null palette,
/// tagged for `--skip-weather`.
#[test]
fn pool_cel_is_unlit_mode_3() {
    use crate::scene::order::pass;
    use crate::scene::{CelArgs, CelPal, FrameId};
    let s = store();
    let mut splash = DrawItem::new(FrameId(0), 50, 60);
    splash.key = DrawKey::new(pass::UNIDENTIFIED_4, 0, 0, 0).unwrap();
    splash.call = Some(CelArgs {
        mode: 3,
        pal: CelPal::Null,
    });
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &std::collections::BTreeMap::new(),
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[splash], &cx).unwrap();
    let r = &rows.draws[0];
    assert_eq!(r[1], "CelDraw");
    assert_eq!(r[12..15], ["3", "0xffffffff", "0"]);
    assert_eq!(r[16], super::compare::POOLS_TAG);
}

/// A unit drawn with its hit flash counter armed gets one `CelDrawHilight` row per body cel
/// right after its `unit` row, before its composite cels; a unit without it does not.
#[test]
fn a_flashing_unit_gets_its_hilight_rows_after_the_unit_row() {
    use crate::scene::order::pass;
    use crate::scene::FrameId;
    let s = store();
    let mut first = DrawItem::new(FrameId(1), 50, 60);
    first.key = DrawKey::new(pass::WALLS_UNITS, 0, 0, 0).unwrap();
    first.tag = ItemTag::Unit(3);
    let mut second = first;
    second.key = DrawKey::new(pass::WALLS_UNITS, 0, 1, 0).unwrap();
    second.x = 70;
    let flash = std::collections::BTreeMap::from([(
        first.key.slot(),
        crate::world_view::HitFlash {
            color: Some(10),
            cels: vec![crate::world_view::HitFlashCel::Frame(FrameId(1))],
        },
    )]);
    let unit_type = |_: u32| None;
    let cx = ExportContext {
        frames: &s,
        view_left: Some(0),
        unit_type: &unit_type,
        sky: &[],
        unit_dirs: &std::collections::BTreeMap::new(),
        unit_rows: &std::collections::BTreeMap::new(),
        tile_ids: &std::collections::BTreeMap::new(),
        tile_lights: &std::collections::BTreeMap::new(),
        tile_alphas: &std::collections::BTreeMap::new(),
        unit_calls: &[],
        hit_flash: &flash,
        color_rows: None,
        ui_calls: &[],
        text_maps: &[],
    };
    let rows = draw_rows(&[first, second], &cx).unwrap();
    let ops: Vec<&str> = rows.draws.iter().map(|r| r[1].as_str()).collect();
    assert_eq!(
        ops,
        ["unit", "CelDrawHilight", "CelDraw", "unit", "CelDraw"]
    );
    let (hl, cel) = (&rows.draws[1], &rows.draws[2]);
    // Same file, dir, frame, (X, Y) and size as the cel; no mode or
    // light; the colour index as `pal`.
    assert_eq!(hl[2..12], cel[2..12]);
    assert_eq!(hl[12..15], ["-", "-", "10"]);
}
