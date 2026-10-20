//! The automap draw sink with synthetic fixtures: a revealed cell, the
//! cel file in a memory source, the player marker.

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::world::{ActLoad, ClientUnit, UnitKey, PLAYER};
use crate::rules::camera::ClientPos;
use crate::rules::draw_order::{
    Dt1Facts, Fade, LevelFacts, NearRooms, Room, TileRecord, TileRect, REC_DRAWN,
};
use crate::ui::automap::options::MemoryStore;
use crate::ui::automap::picker::{CelPicker, PickRecord};
use crate::ui::automap::session::AutomapSource;
use crate::ui::automap::{Automap, AutomapLevels, UnitCels, SEED};
use d2_formats::palette::{Palette, Rgb};

struct Levels;

impl AutomapLevels for Levels {
    fn layer(&self, level: u32) -> Option<u32> {
        Some(level)
    }
    fn level_type(&self, _: u32) -> Option<u32> {
        Some(1)
    }
    fn act(&self, _: u32) -> Option<u8> {
        Some(0)
    }
}

struct Cels;

impl UnitCels for Cels {
    fn monster_cel(&self, _: u32) -> Option<u32> {
        None
    }
    fn object_cel(&self, _: u32) -> Option<u32> {
        None
    }
}

/// LevelType 1, orientation 1: one record that always gives cel 40.
fn source() -> AutomapSource {
    let mut ranges = vec![(-1, -1); 36];
    ranges[1] = (0, 1);
    AutomapSource {
        picker: CelPicker {
            records: vec![PickRecord {
                level: 1,
                tile: 1,
                style: 0xFF,
                start: 0xFF,
                end: 0,
                cels: [40, -1, -1, -1],
                count: 1,
            }],
            ranges,
        },
        levels: Box::new(Levels),
        cels: Box::new(Cels),
    }
}

fn world(cell: (u16, u16)) -> ClientWorld {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some(cell);
    w.units.insert(p, u);
    w.local_player = Some(p);
    w.act = Some(ActLoad {
        act: 0,
        init_seed: 0x1234,
        town_level: 1,
        f8: 9,
    });
    w
}

/// The local player's client position (the frame's one position).
fn at(w: &ClientWorld) -> ClientPos {
    let (x, y) = w.local().unwrap().cell();
    crate::rules::camera::moving_to_client(
        (u32::from(x) << 16) | 0x8000,
        (u32::from(y) << 16) | 0x8000,
    )
}

/// A built frame whose camera stands on the local player.
fn framed(w: &ClientWorld) -> WorldFrame {
    WorldFrame {
        camera: Some(crate::rules::camera::Camera::new(
            FrameSize::D2RUST,
            OpenMode::NONE,
            at(w),
            (0, 0),
        )),
        ..WorldFrame::default()
    }
}

fn near() -> NearRooms {
    let rec = TileRecord {
        tile: (0, 0),
        flags: REC_DRAWN,
        ty: 0,
        dt1: Dt1Facts {
            orientation: 1,
            sub: 5,
            ..Dt1Facts::default()
        },
        fade: Fade::OPAQUE,
        logical: None,
    };
    NearRooms {
        rooms: vec![Room {
            level: 2,
            tiles: TileRect {
                x: 10,
                y: 0,
                w: 8,
                h: 8,
            },
            floors: vec![rec],
            ..Room::default()
        }],
        player_tile: (0, 0),
        player_logical: 0,
        player_subtile: (0, 0),
        edge: None,
        level: LevelFacts {
            id: 2,
            ..LevelFacts::default()
        },
    }
}

/// A DC6 of one direction, `frames` frames of `w × h` literal pixels,
/// offsets 0, bottom-up.
fn dc6(frames: u32, w: u32, h: u32) -> Vec<u8> {
    let mut rows = Vec::new();
    for _ in 0..h {
        rows.push(w as u8);
        rows.extend((0..w).map(|i| 1 + i as u8));
        rows.push(0x80);
    }
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for _ in 0..frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, w, h, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

fn session() -> AutomapSession {
    AutomapSession::new(Automap::new(&MemoryStore::default(), SEED), source(), None)
}

fn assets() -> ViewAssets {
    let mut colors = [Rgb::default(); 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        };
    }
    ViewAssets::new(Palette { colors })
}

fn view() -> AutomapView {
    let mut src = MemorySource::default();
    for p in cel_paths(false, true).into_iter().flatten() {
        src.insert(&p, dc6(64, 8, 4));
    }
    AutomapView::new(Arc::new(src), true)
}

#[test]
fn an_open_automap_draws_its_cells_and_the_player_marker() {
    let w = world((200, 200));
    let mut s = session();
    s.frame(&w, Some(&mut near())).unwrap();
    let mut v = view();
    let mut a = assets();
    let mode = OpenMode::NONE;
    // Closed: nothing.
    let mut frame = framed(&w);
    assert!(v
        .add_to_frame(&mut s, &w, mode, at(&w), None, None, &mut a, &mut frame)
        .is_empty());
    assert!(frame.items.is_empty());
    // Open (Tab): the cell's cel and the marker's lines.
    let f = FrameFacts {
        width: 800,
        height: 600,
        open_mode: 0,
        mini_down: false,
        unit_origin: ClientPos::default(),
    };
    s.toggle(&f);
    let log = v.add_to_frame(&mut s, &w, mode, at(&w), None, None, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    let cels = v
        .last
        .iter()
        .filter(|d| matches!(d, AutomapDraw::Cel { .. }))
        .count();
    let lines = v
        .last
        .iter()
        .filter(|d| matches!(d, AutomapDraw::Line { .. }))
        .count();
    assert!(cels >= 1, "{:?}", v.last);
    assert!(lines >= 1, "{:?}", v.last);
    assert!(frame.items.len() > cels);
    // Each cell is a `CelDrawClipped`
    // call; each line has one first-pixel item (one `DrawLine` row).
    let clipped = frame
        .ui_calls
        .iter()
        .filter(|c| c.call == crate::ui::draw::CelCall::Clipped)
        .count();
    assert_eq!(clipped, cels);
    let starts = frame
        .items
        .iter()
        .filter(|i| {
            a.frames
                .owner(i.frame)
                .is_some_and(|(k, _)| k.path() == crate::facts::export::UI_LINE_START)
        })
        .count();
    assert_eq!(starts, lines);
    assert!(frame
        .items
        .iter()
        .all(|i| i.key.pass() == pass::UI && i.key.major() == pass::UI_AUTOMAP_MAJOR));
    // Closed again: nothing.
    s.toggle(&f);
    let mut frame = framed(&w);
    v.add_to_frame(&mut s, &w, mode, at(&w), None, None, &mut a, &mut frame);
    assert!(frame.items.is_empty());
}

#[test]
fn a_missing_cel_file_is_logged_once_and_not_drawn() {
    let w = world((200, 200));
    let mut s = session();
    s.frame(&w, Some(&mut near())).unwrap();
    s.toggle(&FrameFacts {
        width: 800,
        height: 600,
        open_mode: 0,
        mini_down: false,
        unit_origin: ClientPos::default(),
    });
    let mut v = AutomapView::new(Arc::new(MemorySource::default()), true);
    let mut a = assets();
    let mut frame = framed(&w);
    let log = v.add_to_frame(
        &mut s,
        &w,
        OpenMode::NONE,
        at(&w),
        None,
        None,
        &mut a,
        &mut frame,
    );
    assert!(log.iter().any(|l| l.contains("in no archive")), "{log:?}");
    let log = v.add_to_frame(
        &mut s,
        &w,
        OpenMode::NONE,
        at(&w),
        None,
        None,
        &mut a,
        &mut frame,
    );
    assert!(log.is_empty());
}

#[test]
fn the_automap_draws_after_every_world_pass_and_before_the_panels() {
    // The rain / snow / flash items of pass 9 and the screen fade (10)
    // are world passes; the panel draws are the UI root's list.
    let automap = DrawKey::new(pass::UI, pass::UI_AUTOMAP_MAJOR, 0, 0).unwrap();
    let last_automap =
        DrawKey::new(pass::UI, pass::UI_AUTOMAP_MAJOR, DrawKey::MINOR_MAX, 0xFF).unwrap();
    let panels = DrawKey::new(pass::UI, pass::UI_PANELS_MAJOR, 0, 0).unwrap();
    for world in [
        DrawKey::new(pass::UNIDENTIFIED_9, 0, DrawKey::MINOR_MAX, 0xFF).unwrap(),
        DrawKey::new(
            pass::UNIDENTIFIED_9,
            DrawKey::MAJOR_MAX,
            DrawKey::MINOR_MAX,
            0xFF,
        )
        .unwrap(),
        DrawKey::new(
            pass::SCREEN_FADE,
            DrawKey::MAJOR_MAX,
            DrawKey::MINOR_MAX,
            0xFF,
        )
        .unwrap(),
    ] {
        assert!(world < automap, "{world:?}");
    }
    assert!(last_automap < panels);
}

#[test]
fn automap_cel_modes_map_to_the_alpha_tables() {
    use crate::scene::MapId;
    let mut a = assets();
    // No act tables: opaque whatever the mode.
    assert_eq!(cel_blend(&a, 0), (ShadeChain::EMPTY, BlendOp::Opaque));
    a.shades = Some(crate::rules::shading::ShadeTables {
        light0: MapId(0),
        highlight: MapId(32),
        red: MapId(33),
        zero: MapId(34),
        remap0: MapId(35),
        alpha: [MapId(200), MapId(456), MapId(712)],
        additive: MapId(968),
        multiplicative: MapId(1224),
        max_component: MapId(1480),
    });
    // m = 0 → A2 (25 %), 1 → A1 (50 %), 2 → A0 (75 %), 5 → opaque; unlit.
    for (m, table) in [(0, 712), (1, 456), (2, 200)] {
        let (shade, blend) = cel_blend(&a, m);
        assert_eq!(blend, BlendOp::IndexTable(MapId(table)), "mode {m}");
        assert_eq!(shade, ShadeChain::EMPTY, "mode {m}");
    }
    assert_eq!(cel_blend(&a, 5), (ShadeChain::EMPTY, BlendOp::Opaque));
}

// The header text of an open automap: the version line and the facts' lines,
// font 1 colour 4, right-aligned at W − 16 from y 24 stepping 16; nothing
// while the automap is closed (playtest B020).
#[test]
fn an_open_automap_draws_the_header_lines_at_the_top_right() {
    use crate::ui::automap::header::{HeaderFacts, DIFFICULTY, NIGHTMARE};
    use crate::ui::UiDraw;

    let w = world((200, 200));
    let mut s = session();
    s.frame(&w, Some(&mut near())).unwrap();
    let u = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
    let facts = HeaderFacts {
        level_name: u("Blood Moor"),
        difficulty: 1,
        ..HeaderFacts::default()
    };
    let strings = |id: u16| match id {
        DIFFICULTY => Some(u("Difficulty: ")),
        NIGHTMARE => Some(u("Nightmare")),
        _ => None,
    };
    let width = |_: u16, t: &[u16]| Some(6 * t.len() as i32);
    let draw = |s: &AutomapSession| header_draws(s, &w, OpenMode::NONE, &facts, &strings, &width);
    assert!(draw(&s).is_empty(), "closed: nothing");
    s.toggle(&FrameFacts {
        width: 800,
        height: 600,
        open_mode: 0,
        mini_down: false,
        unit_origin: ClientPos::default(),
    });
    let lines: Vec<(String, i32, i32, u16, u16)> = draw(&s)
        .into_iter()
        .map(|d| match d {
            UiDraw::Text(t) => (
                String::from_utf16_lossy(&t.text),
                t.at.x,
                t.at.y,
                t.style.font,
                t.style.color,
            ),
            other => panic!("{other:?}"),
        })
        .collect();
    let right = |t: &str| 800 - 16 - 6 * t.chars().count() as i32;
    assert_eq!(
        lines,
        vec![
            ("Blood Moor".into(), right("Blood Moor"), 24, 1, 4),
            (
                format!("D2RUST {}", crate::launch::VERSION),
                right(&format!("D2RUST {}", crate::launch::VERSION)),
                40,
                1,
                4,
            ),
            (
                "Difficulty: Nightmare".into(),
                right("Difficulty: Nightmare"),
                56,
                1,
                4
            ),
        ]
    );
    // Open mode 3 (both panels open) draws no automap, so no header.
    let hidden = header_draws(&s, &w, OpenMode::new(3).unwrap(), &facts, &strings, &width);
    assert!(hidden.is_empty());
}

// The header lines against a recorded automap (the Rogue Encampment on the
// expansion, 800 x 600): the first glyph of "Rogue Encampment" and
// "EXPANSION" is drawn at x 643 and 700 on rows 24 and 56 (font 1, colour
// 4), with the real font tables. The D2RUST version line is right-aligned
// on row 40 by the same rule.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_header_lines_sit_at_their_right_edges() {
    use crate::ui::automap::header::{HeaderFacts, EXPANSION};
    use crate::ui::original::{FontMeasure, CHARACTER_FONTS};
    use crate::ui::UiDraw;

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let set = d2_formats::mpq::ArchiveSet::open_dir(&dir).unwrap();
    let source = crate::assets::game_files::GameFiles::archives(std::sync::Arc::new(set));
    let fonts = FontMeasure::load(source.source().as_ref(), &CHARACTER_FONTS).unwrap();

    let w = world((200, 200));
    let mut s = session();
    s.frame(&w, Some(&mut near())).unwrap();
    s.toggle(&FrameFacts {
        width: 800,
        height: 600,
        open_mode: 0,
        mini_down: false,
        unit_origin: ClientPos::default(),
    });
    let u = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
    let facts = HeaderFacts {
        level_name: u("Rogue Encampment"),
        expansion: true,
        ..HeaderFacts::default()
    };
    let strings = |id: u16| (id == EXPANSION).then(|| u("EXPANSION"));
    let width = |font: u16, t: &[u16]| fonts.width_a(font, t);
    let d = header_draws(&s, &w, OpenMode::NONE, &facts, &strings, &width);
    let at: Vec<(String, i32, i32)> = d
        .iter()
        .map(|d| match d {
            UiDraw::Text(t) => (String::from_utf16_lossy(&t.text), t.at.x, t.at.y),
            o => panic!("{o:?}"),
        })
        .collect();
    let version = format!("D2RUST {}", crate::launch::VERSION);
    let right = |t: &str| 800 - 16 - fonts.width_a(1, &u(t)).unwrap();
    assert_eq!(
        at,
        vec![
            ("Rogue Encampment".to_string(), 643, 24),
            (version.clone(), right(&version), 40),
            ("EXPANSION".to_string(), 700, 56),
        ]
    );
}

/// The full clip (0, 0, 799, 599) covers columns 0..799 (right
/// exclusive) and rows 0..=599 (bottom inclusive), as the base game's clipped draw.
#[test]
fn clip_rect_right_exclusive_bottom_inclusive() {
    let b = crate::ui::automap::view::Bounds {
        left: 0,
        top: 0,
        right: 799,
        bottom: 599,
    };
    assert_eq!(rect(&b), Rect::new(0, 0, 799, 600));
}

#[test]
fn cel_limits_are_each_files_frames_per_direction() {
    let mut src = MemorySource::default();
    let paths = cel_paths(false, false);
    src.insert(paths[0].as_deref().unwrap(), dc6(64, 8, 4));
    src.insert(paths[1].as_deref().unwrap(), dc6(64, 8, 4));
    // Act4Map missing; ExTnMap has no file outside the expansion.
    let l = cel_limits(&src, false, false);
    assert_eq!(l.maxi, 64);
    assert_eq!(l.town, [64, 64, 0, 0]);
}
