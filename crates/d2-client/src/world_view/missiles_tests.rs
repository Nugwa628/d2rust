//! Client missiles with synthetic fixtures: a Fire Bolt cast request in the
//! model, missile and overlay DC6 files in a memory source.

use d2_formats::palette::{Palette, Rgb};

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::world::{CastRequest, KindData, PlayerData};
use crate::world_view::model_feed::ModelFeed;
use crate::world_view::NoFeed;

const PLAYER: UnitKey = UnitKey {
    unit_type: 0,
    guid: 1,
};
/// A monster caster (its `cltmissile` is the model's client missile;
/// the layer starts only its cast overlay).
const CASTER: UnitKey = UnitKey {
    unit_type: 1,
    guid: 2,
};
const FIRE_BOLT: usize = 3;

/// A DC6 of one direction, `frames` frames of `w × h` literal pixels.
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

fn rows() -> EffectRows {
    let mut by_skill = vec![0; 8];
    by_skill[FIRE_BOLT] = 1;
    EffectRows {
        skill_missile: by_skill.clone(),
        skill_overlay: by_skill,
        missiles: vec![
            MissileRow::default(),
            MissileRow {
                cel_file: "firebolt".into(),
                vel: 16,
                range: 20,
                anim_rate: 256,
                anim_len: 4,
                loop_anim: true,
                offset: (0, 0, 0),
                explosion: 2,
                init_steps: 0,
                trans: 1,
                clt_do_func: 0,
            },
            MissileRow {
                cel_file: "boom".into(),
                anim_rate: 256,
                anim_len: 3,
                ..MissileRow::default()
            },
        ],
        overlays: vec![
            OverlayRow::default(),
            OverlayRow {
                file: "cast".into(),
                frames: 2,
                anim_rate: 16,
                offset: (0, 0),
                trans: 3,
                pre_draw: false,
            },
        ],
        // Overlay 1 for the model overlay calls.
        overlay_rules: vec![
            OverlayRules::default(),
            OverlayRules {
                anim_rate: 16,
                ..OverlayRules::default()
            },
        ],
        ..EffectRows::default()
    }
}

/// A model overlay call on `unit` at `tick`: a create of overlay 1 of
/// `kind`, or its removal.
fn overlay_call(tick: u64, unit: UnitKey, kind: u8, remove: bool) -> HitOverlay {
    HitOverlay {
        tick,
        unit,
        id: 1,
        kind,
        args: [0; 3],
        remove,
    }
}

fn source() -> Arc<dyn FileSource> {
    let mut src = MemorySource::default();
    src.insert("data\\global\\missiles\\firebolt.dc6", dc6(4, 4, 3));
    src.insert("data\\global\\missiles\\boom.dc6", dc6(3, 6, 5));
    src.insert("data\\global\\overlays\\cast.dc6", dc6(2, 8, 8));
    Arc::new(src)
}

/// View assets with the act's shade tables (the blend modes draw
/// through them).
fn assets() -> ViewAssets {
    let mut a = ViewAssets::new(Palette {
        colors: [Rgb::default(); 256],
    });
    let pl2 = d2_formats::palette::Pl2::parse(&super::super::tile_assets::tests::pl2()).unwrap();
    a.shades = Some(crate::rules::shading::ShadeTables::push(&mut a.maps, &pl2));
    a
}

fn world() -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut p = ClientUnit::new(PLAYER);
    p.position = Some((100, 100));
    p.kind = KindData::Player(PlayerData::default());
    w.units.insert(PLAYER, p);
    w.local_player = Some(PLAYER);
    let mut c = ClientUnit::new(CASTER);
    c.position = Some((100, 100));
    c.mode = 1;
    // On its action frame: a monster's flight starts there, so these
    // tests see it at the request tick.
    c.action_frame = 1;
    w.units.insert(CASTER, c);
    w
}

/// The cast request of S→C 0x4D: skill, point, cast by [`CASTER`].
fn cast(w: &mut ClientWorld, skill: i32, at: (i32, i32)) {
    cast_by(w, CASTER, skill, at);
}

/// The cast request of S→C 0x4D by `key`.
fn cast_by(w: &mut ClientWorld, key: UnitKey, skill: i32, at: (i32, i32)) {
    // The bridge counts the tick before it handles the tick's messages:
    // a cast placed between frames runs in the next frame's tick.
    let tick = w.server_ticks + 1;
    let u = w.units.get_mut(&key).unwrap();
    let request = ModeRequest {
        code: CAST_POINT,
        record: [skill, -1, at.0, at.1, 2, 0, 0],
    };
    u.last_mode_request = Some(request);
    u.mode_requests += 1;
    u.last_cast = Some(CastRequest {
        request,
        count: u.mode_requests,
        late: false,
        tick,
    });
}

/// A built frame of `w`: its one camera,
/// `None` without a local player.
fn framed(w: &ClientWorld) -> WorldFrame {
    let mut feed = ModelFeed::<NoFeed>::default();
    WorldFrame {
        camera: crate::world_view::frame_camera(w, &mut feed).unwrap(),
        ..WorldFrame::default()
    }
}

struct Run {
    m: Missiles,
    a: ViewAssets,
    feed: ModelFeed<NoFeed>,
}

impl Run {
    fn new() -> Self {
        Run {
            m: Missiles::new(source(), rows()),
            a: assets(),
            feed: ModelFeed::<NoFeed>::default(),
        }
    }

    /// One frame at server tick `t`: the draws' positions.
    fn frame(&mut self, w: &mut ClientWorld, t: u64) -> Vec<(i32, i32)> {
        w.server_ticks = t;
        let mut frame = framed(w);
        let log = self.m.add_to_frame(w, &self.feed, &mut self.a, &mut frame);
        assert!(log.is_empty(), "{log:?}");
        frame.items.iter().map(|d| (d.x, d.y)).collect()
    }
}

#[test]
fn a_state_overlay_plays_on_its_unit_and_no_rows_draw_nothing() {
    let mut w = world();
    let mut r = Run::new();
    assert!(r.frame(&mut w, 1).is_empty());
    r.m.hit_overlays([overlay_call(2, PLAYER, 3, false)]);
    assert_eq!(r.frame(&mut w, 2).len(), 1);
    assert_eq!(r.frame(&mut w, 3).len(), 1, "it loops");
    r.m.hit_overlays([overlay_call(4, PLAYER, 0, true)]);
    assert!(r.frame(&mut w, 4).is_empty());
    // The default: no rows, no source.
    let mut frame = framed(&w);
    let mut a = assets();
    let feed = ModelFeed::<NoFeed>::default();
    let log = Missiles::default().add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty() && frame.items.is_empty());
}

#[test]
fn a_missing_art_file_is_logged_once_and_not_drawn() {
    let mut w = world();
    let feed = ModelFeed::<NoFeed>::default();
    let mut m = Missiles::new(Arc::new(MemorySource::default()), rows());
    let mut a = assets();
    m.add_to_frame(&w, &feed, &mut a, &mut framed(&w));
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    w.server_ticks = 2;
    let mut frame = framed(&w);
    let log = m.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert_eq!(log.len(), 1, "the overlay: {log:?}");
    assert!(frame.items.is_empty());
    w.server_ticks = 3;
    assert!(m
        .add_to_frame(&w, &feed, &mut a, &mut framed(&w))
        .is_empty());
}

#[test]
fn the_local_players_overlays_follow_its_predicted_position() {
    use crate::world_view::preview::Preview;
    let mut w = world();
    let mut m = Missiles::new(source(), rows());
    m.hit_overlays([overlay_call(1, PLAYER, 3, false)]);
    let mut a = assets();
    let mut feed = ModelFeed::<NoFeed>::default().with_preview(Preview::default());
    let mut draw = |feed: &mut ModelFeed<NoFeed>, t: u64| {
        w.server_ticks = t;
        let mut frame = WorldFrame {
            camera: crate::world_view::frame_camera(&w, feed).unwrap(),
            ..WorldFrame::default()
        };
        let log = m.add_to_frame(&w, &*feed, &mut a, &mut frame);
        assert!(log.is_empty(), "{log:?}");
        frame.items.iter().map(|d| (d.x, d.y)).collect::<Vec<_>>()
    };
    let at_cell = draw(&mut feed, 1);
    assert_eq!(at_cell.len(), 1);
    // The walk prediction three sub-tiles east of the model cell: the
    // camera follows it, and so does the overlay (same screen point).
    let c = |s: u32| (s << 16) | 0x8000;
    feed.set_local_prediction(Some((PLAYER, (c(103), c(100)))));
    assert_eq!(draw(&mut feed, 1), at_cell);
}

#[test]
fn missiles_and_overlays_draw_in_their_trans_mode() {
    let mut w = world();
    let mut r = Run::new();
    let t = r.a.shades.unwrap();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    let ops = |m: &Missiles| -> Vec<(ShadeChain, BlendOp)> {
        m.last()
            .iter()
            .map(|d| (d.item.shade, d.item.blend))
            .collect()
    };
    // Cast overlay `Trans` 3 (the overlay's mode as is): additive, unlit,
    // no remap.
    let add = cel_ops(&t, 3, None, 0xFF);
    assert_eq!(ops(&r.m), vec![add]);
    assert_ne!(add.1, BlendOp::Opaque);
    // Missile `Trans` 2 → mode 4, anything else → 5 (opaque).
    assert_eq!(missile_mode(2, false), 4);
    assert_eq!(missile_mode(0, false), MODE_OPAQUE);
    let mut rows = rows();
    rows.missiles[1].trans = 2;
    rows.overlays[1].trans = 5;
    let mut r = Run::new();
    r.m = Missiles::new(source(), rows);
    let mut w = world();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    assert_eq!(ops(&r.m), vec![cel_ops(&t, 5, None, 0xFF)]);
    assert_eq!(ops(&r.m)[0], (ShadeChain::EMPTY, BlendOp::Opaque));
}

#[test]
fn missiles_join_their_cell_and_overlays_their_host() {
    use crate::rules::camera::{moving_to_client, FrameSize, OpenMode};
    use crate::rules::draw_order::{tile_of, DrawGrid, OrderKey, UnitSlot};
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    // The player's camera, at the caster's cell centre.
    let at = moving_to_client(cell_centre((100, 100)).0, cell_centre((100, 100)).1);
    let camera = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
    let grid = DrawGrid::of_camera(&camera);
    let ci = grid.cell(tile_of(at.x, at.y)).unwrap() as u32;
    let slot = |minor| {
        UnitSlot::Drawn(OrderKey {
            pass: pass::WALLS_UNITS,
            major: ci,
            minor,
        })
    };
    let other = UnitKey::new(MONSTER, 9);
    let slots = BTreeMap::from([(CASTER, slot(3)), (other, slot(4))]);
    // The cast overlay (`PreDraw` 0) after the caster's slots (sub 255),
    // wherever the cell's units stand.
    let below = |k: UnitKey| (k == CASTER).then_some(i32::MAX);
    for keyed in [
        Missiles::keyed(r.m.last(), &slots, &grid, below),
        Missiles::keyed(r.m.last(), &slots, &grid, |_| Some(i32::MIN)),
    ] {
        let got: Vec<_> = keyed.iter().map(|(d, b)| (d.item.key, *b)).collect();
        assert_eq!(got, vec![(DrawKey::new(6, ci, 3, 255).unwrap(), false)]);
    }
    // A back overlay draws before its host's slot 0; a hidden host hides
    // its overlays.
    let mut rows = rows();
    rows.overlays[1].pre_draw = true;
    let mut r2 = Run::new();
    r2.m = Missiles::new(source(), rows);
    let mut w2 = world();
    r2.frame(&mut w2, 10);
    cast(&mut w2, FIRE_BOLT as i32, (130, 100));
    r2.frame(&mut w2, 11);
    let keyed = Missiles::keyed(r2.m.last(), &slots, &grid, below);
    assert_eq!(
        (keyed[0].0.item.key, keyed[0].1),
        (DrawKey::new(6, ci, 3, 0).unwrap(), true)
    );
    let hidden = BTreeMap::from([(CASTER, UnitSlot::NotDrawn)]);
    assert!(Missiles::keyed(r2.m.last(), &hidden, &grid, below).is_empty());

    // In the frame: the overlay after the caster's item, before the
    // monster's slot.
    let mut w3 = world();
    let mut m = ClientUnit::new(other);
    m.position = Some((100, 102));
    w3.units.insert(other, m);
    let mut frame = WorldFrame {
        camera: Some(camera),
        slots: Some(BTreeMap::from([(CASTER, slot(3)), (other, slot(5))])),
        ..WorldFrame::default()
    };
    for (minor, tag) in [(3, ItemTag::Unit(2)), (5, ItemTag::Unit(9))] {
        let mut item = DrawItem::new(crate::scene::FrameId(0), 0, 0);
        item.key = DrawKey::new(6, ci, minor, 0).unwrap();
        item.tag = tag;
        frame.items.push(item);
    }
    let mut r3 = Run::new();
    r3.frame(&mut w3, 10);
    cast(&mut w3, FIRE_BOLT as i32, (130, 100));
    w3.server_ticks = 11;
    let feed = ModelFeed::<NoFeed>::default();
    let log = r3.m.add_to_frame(&w3, &feed, &mut r3.a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    let got: Vec<_> = frame
        .items
        .iter()
        .map(|i| (i.key.minor(), i.key.sub(), i.tag))
        .collect();
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0], (3, 0, ItemTag::Unit(2)));
    assert_eq!(
        (got[1].0, got[1].1),
        (3, 255),
        "the overlay after the caster"
    );
    assert_eq!(got[2], (5, 0, ItemTag::Unit(9)));
}

#[test]
fn flat_missiles_file_in_their_cells_shadow_list() {
    use crate::rules::camera::{moving_to_client, FrameSize, OpenMode};
    use crate::rules::draw_order::{tile_of, DrawGrid};
    // Functions 2 and 11 set flag 0x10000 at the animation end: frame +
    // speed ≥ length, frame = age · speed.
    assert!(!flat_at_end(11, 3, 256, 1, TickRate::BASE));
    assert!(flat_at_end(11, 3, 256, 2, TickRate::BASE));
    assert!(!flat_at_end(2, 4, 128, 6, TickRate::BASE));
    assert!(flat_at_end(2, 4, 128, 7, TickRate::BASE));
    assert!(
        !flat_at_end(1, 3, 256, 99, TickRate::BASE),
        "other functions never"
    );
    // A flat missile goes to its cell's shadow list (pass 5), after the
    // cell's entries; not into the unit list.
    let at = moving_to_client(cell_centre((100, 100)).0, cell_centre((100, 100)).1);
    let camera = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
    let grid = DrawGrid::of_camera(&camera);
    let ci = grid.cell(tile_of(at.x, at.y)).unwrap() as u32;
    let draw = MissileDraw {
        id: 1,
        item: DrawItem::new(crate::scene::FrameId(0), 0, 0),
        join: Join::Flat { at },
    };
    let keyed = Missiles::keyed(&[draw], &BTreeMap::new(), &grid, |_| None);
    assert_eq!(
        keyed
            .iter()
            .map(|(d, b)| (d.item.key, *b))
            .collect::<Vec<_>>(),
        vec![(
            DrawKey::new(pass::SHADOWS, ci, DrawKey::MINOR_MAX, 255).unwrap(),
            false
        )]
    );
}

#[test]
fn a_ui_overlay_call_creates_advances_draws_on_its_host_and_removes() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            xoffset: -5,
            yoffset: -7,
            height: [-30, 0, 0, -50],
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    // Class 3's `monstats2` row: `overlayHeight` 4 → `Height4`.
    r.m.rows.monster_overlay = vec![(false, None); 3];
    r.m.rows.monster_overlay.push((false, Some(4)));
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    npc.class = 3;
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    let on = OverlayCall {
        unit: k,
        id: 1,
        on: true,
    };
    r.m.overlay_calls(2, [on]);
    // Created before the update's walk: advanced once in the same update.
    assert_eq!(r.frame(&mut w, 2).len(), 1);
    let rec = r.m.unit_overlays(k)[0];
    assert_eq!(
        (rec.kind, rec.rate, rec.frame, rec.frames),
        (3, 144, 144, 512)
    );
    assert_eq!((rec.x, rec.y), (-5, -57));
    let d = r.m.last()[0];
    assert_eq!(d.item.tag, ItemTag::Unit(7), "drawn in its host's unit run");
    assert_eq!(
        d.join,
        Join::Host {
            host: k,
            back: false
        }
    );
    // Two more updates: +0x18 = 432, frame 1; kind 3 wraps at the end.
    r.frame(&mut w, 4);
    assert_eq!(r.m.unit_overlays(k)[0].frame_index(), 1);
    r.frame(&mut w, 6);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 5 * 144 - 512);
    // A second create of the same id replaces the record.
    r.m.overlay_calls(7, [on]);
    r.frame(&mut w, 7);
    assert_eq!(r.m.unit_overlays(k).len(), 1);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 144);
    // Remove by id; then nothing draws.
    r.m.overlay_calls(8, [OverlayCall { on: false, ..on }]);
    assert!(r.frame(&mut w, 8).is_empty());
    assert!(r.m.unit_overlays(k).is_empty());
}

#[test]
fn a_unit_overlay_output_creates_a_kind_2_overlay_at_its_tick() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    r.m.rows.monster_overlay = vec![(false, None); 4];
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    npc.class = 3;
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    let o = Output::UnitOverlay {
        unit: k,
        overlay: 1,
        mode: 2,
        sound: 0,
    };
    assert!(r.m.effect_output(2, &o));
    // Other effect outputs are not this layer's yet.
    let umod = Output::UmodFx {
        unit: k,
        umods: [0; 9],
        flag8: false,
    };
    assert!(!r.m.effect_output(2, &umod));
    assert_eq!(r.frame(&mut w, 2).len(), 1);
    let rec = r.m.unit_overlays(k)[0];
    assert_eq!((rec.kind, rec.frame), (2, 144));
    // A later-queued call of an earlier tick runs first (tick order).
    let on = OverlayCall {
        unit: k,
        id: 1,
        on: false,
    };
    assert!(r.m.effect_output(5, &o));
    r.m.overlay_calls(4, [on]);
    r.frame(&mut w, 4);
    assert!(r.m.unit_overlays(k).is_empty());
    r.frame(&mut w, 5);
    assert_eq!(r.m.unit_overlays(k)[0].kind, 2);
}

#[test]
fn a_hit_overlay_runs_after_the_advance_of_its_tick() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    r.m.rows.monster_overlay = vec![(false, None); 4];
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    npc.class = 3;
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    let hit = |count, tick| crate::bridge::world::HitOverlay { id: 1, count, tick };
    w.units.get_mut(&k).unwrap().last_hit_overlay = Some(hit(1, 3));
    // Created after tick 3's advance: frame 0 draws in tick 3.
    assert_eq!(r.frame(&mut w, 3).len(), 1);
    let rec = r.m.unit_overlays(k)[0];
    assert_eq!((rec.kind, rec.frame), (2, 0));
    r.frame(&mut w, 4);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 144);
    // The same count again creates nothing new.
    r.frame(&mut w, 5);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 288);
    // A new create replaces the record, again after the advance.
    w.units.get_mut(&k).unwrap().last_hit_overlay = Some(hit(2, 6));
    r.frame(&mut w, 6);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 0);
}

#[test]
fn a_shrine_on_mode_output_removes_then_creates_in_mode_0() {
    use crate::bridge::output::ShrineFxKind;
    let obj = UnitKey::new(2, 9);
    let fx = |kind, overlays, mode| Output::ShrineFx {
        kind,
        code: 12,
        object: obj,
        player: None,
        overlays,
        mode,
    };
    let call = |id, on: bool| crate::bridge::hit_overlays::HitOverlay {
        tick: 7,
        unit: obj,
        id,
        kind: 3,
        args: [0; 3],
        remove: !on,
    };
    let mut m = Missiles::default();
    assert!(m.effect_output(7, &fx(ShrineFxKind::OnMode, [0x41, 0x39], 0)));
    assert_eq!(
        m.overlay_calls,
        [
            call(0x41, false),
            call(0x41, true),
            call(0x39, false),
            call(0x39, true)
        ]
    );
    // Out of mode 0: the removes only; −1 is no overlay.
    let mut m = Missiles::default();
    assert!(m.effect_output(7, &fx(ShrineFxKind::OnMode, [0x41, -1], 2)));
    assert_eq!(m.overlay_calls, [call(0x41, false)]);
    // The on-use function is not this layer's.
    assert!(!m.effect_output(7, &fx(ShrineFxKind::OnUse, [0x41, 0x39], 0)));
    assert_eq!(m.overlay_calls.len(), 1);
}

#[test]
fn a_host_whose_body_is_culled_draws_none_of_its_overlays() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    r.m.overlay_calls(
        2,
        [OverlayCall {
            unit: k,
            id: 1,
            on: true,
        }],
    );
    w.server_ticks = 2;
    let mut frame = framed(&w);
    frame.body_culled.insert(k);
    let log = r.m.add_to_frame(&w, &r.feed, &mut r.a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    assert!(frame.items.is_empty(), "culled host: no overlay draw");
    // The record lives on and advances; it draws again with the body.
    assert_eq!(r.m.unit_overlays(k).len(), 1);
    assert_eq!(r.frame(&mut w, 3).len(), 1);
}

/// The one cel the layer draws for client missile `m` (key 3:40, class
/// 1), filed in the unit slot of its drawn point's cell; its unit draw
/// call carries no path (no COF).
fn client_missile_cels(m: crate::bridge::client_missiles::ClientMissile) -> Vec<DrawItem> {
    use crate::bridge::world::view_key;
    use crate::rules::draw_order::{tile_of, DrawGrid, OrderKey, UnitSlot};
    use crate::world_view::UnitCall;
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 1);
    let key = UnitKey::new(MISSILE, 40);
    let mut unit = ClientUnit::new(key);
    unit.class = 1;
    w.objclient.set_c.insert(key, unit);
    w.objclient.missiles.insert(key, m);
    w.server_ticks = 2;
    let mut frame = framed(&w);
    let camera = frame.camera.unwrap();
    let grid = DrawGrid::of_camera(&camera);
    let drawn = m.drawn_at.unwrap_or(m.pos);
    let at = crate::rules::camera::moving_to_client(drawn.0, drawn.1);
    let ci = grid.cell(tile_of(at.x, at.y)).unwrap() as u32;
    let slot = OrderKey {
        pass: pass::WALLS_UNITS,
        major: ci,
        minor: 2,
    };
    frame.slots = Some(BTreeMap::from([(view_key(key), UnitSlot::Drawn(slot))]));
    frame.unit_calls.push(UnitCall {
        call: None,
        key: DrawKey::new(slot.pass, slot.major, slot.minor, 0).unwrap(),
        tag: ItemTag::Unit(view_key(key).guid),
        path: None,
        dir64: 0,
        frame: 0,
        shadow: false,
    });
    let log = r.m.add_to_frame(&w, &r.feed, &mut r.a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    frame.items
}

#[test]
fn a_client_missiles_cel_draws_though_its_unit_call_has_no_path() {
    use crate::bridge::client_missiles::ClientMissile;
    let items = client_missile_cels(ClientMissile {
        pos: cell_centre((101, 100)),
        ..ClientMissile::default()
    });
    assert_eq!(items.len(), 1, "the missile's one cel");
}

/// A client missile's cel takes the light byte of its own unit draw:
/// The base game lights client missiles
/// like any unit (measured: the blood splats in
/// mode 5 are `L[P[s]]`), so the layer asks `host_light` for the missile.
#[test]
fn a_client_missiles_cel_takes_its_own_units_light_byte() {
    use crate::bridge::client_missiles::ClientMissile;
    use std::cell::RefCell;
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 1);
    let key = UnitKey::new(MISSILE, 40);
    let mut unit = ClientUnit::new(key);
    unit.class = 1;
    w.objclient.set_c.insert(key, unit);
    w.objclient.missiles.insert(
        key,
        ClientMissile {
            pos: cell_centre((101, 100)),
            ..ClientMissile::default()
        },
    );
    w.server_ticks = 2;
    // One frame first: it makes the missile's art resident.
    let mut frame = framed(&w);
    let log = r.m.add_to_frame(&w, &r.feed, &mut r.a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    let camera = frame.camera.unwrap();
    let asked = RefCell::new(Vec::new());
    let host_light = |u: &ClientUnit| {
        asked.borrow_mut().push(u.key);
        0x80
    };
    let draws =
        r.m.draws(&w, &camera, &r.a, &|_| None, &|_, _| false, &host_light);
    assert_eq!(draws.len(), 1, "the missile's one cel");
    assert_eq!(*asked.borrow(), vec![key]);
}

#[test]
fn a_trails_cel_draws_at_its_pixel_pair_not_its_copied_precise_point() {
    use crate::bridge::client_missiles::ClientMissile;
    // A function 18 child: created at its cell's centre, then given the
    // spear's precise point (here 6 and 3 px off along the two axes).
    let centre = cell_centre((101, 100));
    let trail = ClientMissile {
        pos: (centre.0 + 0x6000, centre.1 - 0x3000),
        drawn_at: Some(centre),
        ..ClientMissile::default()
    };
    let at_centre = ClientMissile {
        pos: centre,
        ..ClientMissile::default()
    };
    let (a, b) = (client_missile_cels(trail), client_missile_cels(at_centre));
    assert_eq!(a.len(), 1);
    assert_eq!(
        (a[0].x, a[0].y),
        (b[0].x, b[0].y),
        "drawn at the creation centre"
    );
}

#[test]
fn a_call_delivered_on_a_skipped_tick_runs_just_before_that_ticks_step() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    // Tick 2 draws no frame; the call delivered at tick 3 waits for the
    // frame of tick 3, which catches up ticks 2 and 3: one advance.
    r.m.overlay_calls(
        3,
        [OverlayCall {
            unit: k,
            id: 1,
            on: true,
        }],
    );
    r.frame(&mut w, 3);
    assert_eq!(r.m.unit_overlays(k)[0].frame, 144);
}

#[test]
fn a_cast_overlay_plays_at_animrate_times_16_and_leaves() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    assert_eq!(r.m.live(), 1, "the overlay");
    // `AnimRate` 16 = 256 (8.8) = one frame per tick: two frames, gone at
    // the third tick.
    let overlay_frames: Vec<usize> = (12..=14)
        .map(|t| {
            r.frame(&mut w, t);
            r.m.live()
        })
        .collect();
    assert_eq!(overlay_frames, vec![1, 0, 0], "{overlay_frames:?}");
}

#[test]
fn a_cast_from_a_late_scripted_click_ages_from_the_tick_before() {
    // The local player's request from a scripted click the model ran
    // after its update: the base game created the overlay in the update before.
    // Another unit's request comes from a server message: no shift.
    for (key, gone) in [(PLAYER, vec![0, 0, 0]), (CASTER, vec![1, 0, 0])] {
        let mut w = world();
        let mut r = Run::new();
        r.frame(&mut w, 10);
        cast_by(&mut w, key, FIRE_BOLT as i32, (130, 100));
        w.units
            .get_mut(&key)
            .unwrap()
            .last_cast
            .as_mut()
            .unwrap()
            .late = true;
        r.frame(&mut w, 11);
        assert_eq!(r.m.live(), 1, "the overlay");
        let live: Vec<usize> = (12..=14)
            .map(|t| {
                r.frame(&mut w, t);
                r.m.live()
            })
            .collect();
        assert_eq!(live, gone, "{key:?}");
    }
}

#[test]
fn a_cast_first_seen_a_tick_late_ages_from_its_request() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    // The request ran in tick 11; no frame was built until tick 12.
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 12);
    assert_eq!(r.m.live(), 1, "the overlay");
    r.frame(&mut w, 13);
    assert_eq!(r.m.live(), 0, "the overlay played ticks 11 and 12");
}

#[test]
fn a_cast_followed_by_a_soft_hit_in_the_same_tick_still_starts() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    // A 0x13 soft hit after the cast, before the frame (the first Frost
    // Nova cast of `fxfrostnova`, tick 265): the last request is no cast.
    let u = w.units.get_mut(&CASTER).unwrap();
    u.last_mode_request = Some(ModeRequest {
        code: 0x13,
        record: [5164, 4656, 45, 0, 0, 0, 0],
    });
    u.mode_requests += 1;
    r.frame(&mut w, 11);
    assert_eq!(r.m.live(), 1, "the overlay");
}

#[test]
fn the_same_cast_repeated_starts_a_new_overlay() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    assert_eq!(r.m.live(), 1);
    for t in 12..60 {
        r.frame(&mut w, t);
    }
    assert_eq!(r.m.live(), 0, "the first overlay ended");
    // The same skill at the same point again: an identical request, one
    // more request counted.
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 60);
    assert_eq!(r.m.live(), 1, "a new overlay");
}

#[test]
fn rows_take_the_explosion_link_and_the_anim_speed_from_the_columns() {
    use d2_data::tables::Record;
    let blank = || d2_data::tables::Missiles::decode(&vec![0; d2_data::tables::Missiles::SIZE]);
    let mut fire_bolt = blank();
    fire_bolt.explosionmissile = 29;
    fire_bolt.explosion = false;
    fire_bolt.animspeed = 16;
    fire_bolt.initsteps = 1;
    let mut plain = blank();
    plain.explosionmissile = 0xFFFF; // blank link16 = -1
    let rows = EffectRows::from_tables(&[], &[fire_bolt, plain], &[], &[]);
    assert_eq!(rows.missiles[0].explosion, 29);
    assert_eq!(rows.missiles[0].anim_rate, 256);
    assert_eq!(rows.missiles[0].init_steps, 1);
    assert_eq!(rows.missiles[1].explosion, 0);
}

#[test]
fn a_players_cast_starts_its_overlay_but_no_flight_of_the_layers_own() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast_by(&mut w, PLAYER, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    assert_eq!(r.m.live(), 1, "the cast overlay alone");
}

#[test]
fn a_cast_while_its_overlay_plays_starts_no_second_one() {
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast_by(&mut w, PLAYER, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    // finds the first still playing: no create.
    cast_by(&mut w, PLAYER, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 12);
    assert_eq!(r.m.live(), 1);
}

#[test]
fn a_monsters_cast_starts_its_overlay_but_no_flight_of_the_layers_own() {
    // The monster's `cltmissile` is the model's client missile, created by
    // its skill step (`bridge::monster_anim::skill_step`).
    let mut w = world();
    let mut r = Run::new();
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    assert_eq!(r.m.live(), 1, "the cast overlay alone");
}

#[test]
fn a_hit_overlay_is_a_kind_2_record_removed_after_its_frames() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    let k = UnitKey::new(MONSTER, 7);
    let mut npc = ClientUnit::new(k);
    npc.position = Some((102, 100));
    w.units.insert(k, npc);
    r.frame(&mut w, 1);
    r.m.hit_overlays([HitOverlay {
        tick: 2,
        unit: k,
        id: 1,
        kind: 2,
        args: [0; 3],
        remove: false,
    }]);
    r.frame(&mut w, 2);
    let rec = r.m.unit_overlays(k)[0];
    assert_eq!((rec.kind, rec.a, rec.frame), (2, 1, 144));
    // 4 frames of 144 reach the end (running flag cleared), the
    // next update removes the record.
    r.frame(&mut w, 5);
    assert_eq!(r.m.unit_overlays(k)[0].a, 0);
    r.frame(&mut w, 6);
    assert!(r.m.unit_overlays(k).is_empty());
}

#[test]
fn only_composite_hosts_fail_the_cof_box_pre_test() {
    use crate::rules::draw_order::{OrderKey, UnitSlot};
    let slot = |minor| {
        UnitSlot::Drawn(OrderKey {
            pass: pass::WALLS_UNITS,
            major: 7,
            minor,
        })
    };
    let monster = UnitKey::new(MONSTER, 9);
    // A client missile's unit call carries no cel path (this layer draws
    // its cel): the same "no cel" mark a culled composite leaves.
    let missile = view_key(UnitKey::new(MISSILE, 208));
    let slots = BTreeMap::from([(monster, slot(3)), (missile, slot(4))]);
    let culled: std::collections::BTreeSet<u64> = [3, 4]
        .into_iter()
        .map(|m| DrawKey::new(pass::WALLS_UNITS, 7, m, 0).unwrap().slot())
        .collect();
    assert!(host_culled(monster, &slots, &culled));
    assert!(!host_culled(missile, &slots, &culled));
    assert!(!host_culled(UnitKey::new(MONSTER, 10), &slots, &culled));
    assert!(!host_culled(monster, &slots, &Default::default()));
}

#[test]
fn a_landed_items_gleam_plays_its_start_cycle_then_waits() {
    let mut w = world();
    let mut r = Run::new();
    // Row 70 (`itemgleam`): the test's two-frame cast art, a 4000 ms wait.
    let gleam = r.m.rows.overlays[1].clone();
    r.m.rows.overlays.resize(70, OverlayRow::default());
    r.m.rows.overlays.push(gleam);
    r.m.rows.overlay_rules = vec![OverlayRules::default(); 70];
    r.m.rows.overlay_rules.push(OverlayRules {
        anim_rate: 16,
        loop_wait_time: 4000,
        ..OverlayRules::default()
    });
    let k = UnitKey::new(4, 9);
    let mut pile = ClientUnit::new(k);
    pile.position = Some((102, 100));
    pile.mode = 3;
    // Landed at tick 200; the a = 8 draw gave frame 0.
    pile.ground_overlay = Some((200, 0));
    w.units.insert(k, pile);
    assert!(r.frame(&mut w, 199).is_empty(), "not yet landed");
    // Created after the walk with +0x08 = 0 and +0x30 = 0: hidden.
    assert!(r.frame(&mut w, 200).is_empty());
    let rec = r.m.unit_overlays(k)[0];
    assert_eq!((rec.kind, rec.a, rec.frame, rec.wait_start), (6, 0, 0, 0));
    // The next update's clock (8040 ms) is past the 4000 ms wait: it
    // plays at once.
    assert_eq!(r.frame(&mut w, 201).len(), 1);
    assert_eq!(r.m.unit_overlays(k)[0].frame_index(), 1);
    // The cycle wraps: +0x08 := 0, +0x30 := now, hidden while it waits.
    assert!(r.frame(&mut w, 202).is_empty());
    assert_eq!(r.m.unit_overlays(k)[0].wait_start, 202 * 40);
    // 4000 ms is 100 updates on the 40 ms clock.
    assert!(r.frame(&mut w, 301).is_empty());
    assert_eq!(r.frame(&mut w, 302).len(), 1);
    assert_eq!(r.m.unit_overlays(k).len(), 1, "one record per item");
}

#[test]
fn a_client_missiles_cel_adds_its_motion_records_offsets() {
    use crate::bridge::client_missiles::ClientMissile;
    // A lobbed missile 40 px up its arc (`oz` = −40) and 3 px right.
    let flat = ClientMissile {
        pos: cell_centre((101, 100)),
        ..ClientMissile::default()
    };
    let mut lobbed = flat;
    lobbed.motion.offset = [3, 0, -40];
    let (a, b) = (client_missile_cels(flat), client_missile_cels(lobbed));
    assert_eq!((b[0].x - a[0].x, b[0].y - a[0].y), (3, -40));
}

#[test]
fn one_units_cast_and_hit_overlays_draw_newest_first() {
    let mut w = world();
    let mut r = Run::new();
    r.m.rows.overlay_rules = vec![
        OverlayRules::default(),
        OverlayRules {
            anim_rate: 9,
            ..OverlayRules::default()
        },
    ];
    r.m.rows.monster_overlay = vec![(false, None); 4];
    r.frame(&mut w, 10);
    cast(&mut w, FIRE_BOLT as i32, (130, 100));
    r.frame(&mut w, 11);
    let hit = crate::bridge::world::HitOverlay {
        id: 1,
        count: 1,
        tick: 12,
    };
    w.units.get_mut(&CASTER).unwrap().last_hit_overlay = Some(hit);
    r.frame(&mut w, 12);
    let on_caster: Vec<u32> =
        r.m.last
            .iter()
            .filter(|d| matches!(d.join, Join::Host { host, .. } if host == CASTER))
            .map(|d| d.id)
            .collect();
    assert_eq!(on_caster.len(), 2, "{on_caster:x?}");
    // The hit overlay record (newer) before the cast overlay.
    assert!(on_caster[0] & 0x0040_0000 != 0, "{on_caster:x?}");
    assert!(on_caster[1] & 0x0040_0000 == 0, "{on_caster:x?}");
}

/// F5: a cast overlay's `AnimRate` is per base-rate tick, so its pass and
/// its frames last the same real time at 25 Hz and at the game's 60 Hz,
/// and a blood missile turns flat after the same time.
#[test]
fn cast_effects_play_in_real_time_at_every_rate() {
    let r60 = TickRate::new(60).unwrap();
    // 14 frames at one a base tick: 14 ticks (560 ms) at 25 Hz; 33.6 →
    // 34 ticks at 60 Hz.
    assert_eq!(Missiles::ticks_of(14, 256, TickRate::BASE), 14);
    assert_eq!(Missiles::ticks_of(14, 256, r60), 34);
    assert_eq!(Missiles::ticks_of(14, 128, r60), 68);
    // The drawn frame at the end of the pass: the last at both rates.
    assert_eq!(TickRate::BASE.progress(256, 13) >> 8, 13);
    assert_eq!(r60.progress(256, 33) >> 8, 13);
    assert_eq!(r60.progress(256, 34) >> 8, 14);
    // Flat after 3 frames at one a base tick: age 2 at 25 Hz (the third
    // tick, 120 ms), age 7 at 60 Hz (the eighth, 133 ms).
    assert!(!flat_at_end(11, 3, 256, 6, r60));
    assert!(flat_at_end(11, 3, 256, 7, r60));
}
