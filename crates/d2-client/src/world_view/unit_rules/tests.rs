//! Synthetic fixtures only: invented tokens, COFs and DC6 files.

use std::sync::Arc;

use d2_formats::palette::{Palette, Rgb};

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::world::{UnitKey, MONSTER, OBJECT, PLAYER};
use crate::rules::unit_composite::{code, Code, EMPTY};
use crate::world_view::unit_assets::{unit_cof, MonsterRow, UnitArtLoader, UnitLooks};
use crate::world_view::{build, Unspecified};

/// COF bytes: one direction, `frames` frames, the
/// given layers (component, weapon class), animation rate 256 (one frame
/// per tick), each frame drawing the layers in order.
fn cof_bytes(frames: u8, layers: &[u8]) -> Vec<u8> {
    cof_dirs(1, frames, layers)
}

/// [`cof_bytes`] with `dirs` directions.
fn cof_dirs(dirs: u8, frames: u8, layers: &[u8]) -> Vec<u8> {
    let l = layers.len() as u8;
    let mut v = vec![l, frames, dirs, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&256u32.to_le_bytes());
    for c in layers {
        v.extend_from_slice(&[*c, 0, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
    }
    v.extend(std::iter::repeat_n(0, usize::from(frames)));
    for _ in 0..usize::from(frames) * usize::from(dirs) {
        v.extend_from_slice(layers);
    }
    v
}

/// A DC6 of one direction with `frames` frames of `w` × `h` literal
/// pixels.
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

fn codes(names: &[&[u8]]) -> Vec<Code> {
    names.iter().map(|n| code(n)).collect()
}

/// Invented tokens: player class 0 = `QA`, monster 7 = `QM` (base weapon
/// `qwc`), object 342 (a DC6 object row) = `QO`.
fn looks() -> UnitLooks {
    let mut l = UnitLooks {
        player_tokens: codes(&[b"QA"]),
        player_modes: codes(&[b"QD", b"QN"]),
        monster_modes: codes(&[b"QD", b"QN"]),
        object_modes: codes(&[b"QN"]),
        components: codes(&[b"QH", b"QT"]),
        ..Default::default()
    };
    l.monsters.insert(
        7,
        MonsterRow {
            token: code(b"QM"),
            base_w: Some(code(b"qwc")),
            composite_death: false,
            directions: [0; 16],
        },
    );
    l.objects.insert(342, code(b"QO"));
    l
}

fn unit(unit_type: u8, guid: u32, class: u32, mode: u32) -> ClientUnit {
    let mut u = ClientUnit::new(UnitKey { unit_type, guid });
    u.class = class;
    u.mode = mode;
    u.position = Some((10, 10));
    u
}

fn assets() -> ViewAssets {
    ViewAssets::new(Palette {
        colors: [Rgb::default(); 256],
    })
}

/// [`Unspecified`] with a fixed placement, so a unit can be built
/// without a camera.
struct Placed;

impl ViewRules for Placed {
    fn tiles(&self, w: &ClientWorld, a: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        Unspecified.tiles(w, a)
    }
    fn unit_pose(&self, w: &ClientWorld, u: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Unspecified.unit_pose(w, u)
    }
    fn unit_params(
        &self,
        w: &ClientWorld,
        u: &ClientUnit,
        p: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Unspecified.unit_params(w, u, p)
    }
    fn component_frame(
        &self,
        u: &ClientUnit,
        p: &UnitPose,
        r: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Unspecified.component_frame(u, p, r)
    }
    fn place(
        &self,
        _: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
        _: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Ok((100, 200))
    }
    fn shade(
        &self,
        u: &ClientUnit,
        r: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Unspecified.shade(u, r)
    }
    fn blend(&self, u: &ClientUnit, r: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Unspecified.blend(u, r)
    }
}

impl UiRules for Placed {
    fn ui_image(&self, r: &ImageRequest, a: &ViewAssets) -> Result<UiSprite, ViewError> {
        Unspecified.ui_image(r, a)
    }
    fn ui_text(&self, r: &TextRequest, a: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        Unspecified.ui_text(r, a)
    }
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(pass::UI)
    }
}

fn setup(src: MemorySource) -> (UnitArtLoader, UnitRules<Placed>) {
    setup_with(src, looks())
}

/// [`setup`] with the given looks.
fn setup_with(src: MemorySource, looks: UnitLooks) -> (UnitArtLoader, UnitRules<Placed>) {
    let looks = Arc::new(looks);
    let art = SharedUnitArt::default();
    let loader = UnitArtLoader {
        source: Arc::new(src),
        looks: looks.clone(),
        art: art.clone(),
    };
    let rules = UnitRules {
        frame_source: Default::default(),
        rules: Placed,
        looks,
        art,
    };
    (loader, rules)
}

#[test]
fn cof_and_component_names_from_the_looks() {
    let mut l = looks();
    let p = unit(PLAYER, 1, 0, 1);
    let name = unit_cof(&l, &p).unwrap();
    assert_eq!(name.full(), "DATA\\GLOBAL\\CHARS\\QA\\COF\\QAQNhth.COF");
    let layer = d2_formats::cof::CofLayer {
        component: 1,
        shadow: 0,
        selectable: 1,
        override_translucency: 0,
        new_translucency: 0,
        weapon_class: *b"hth\0",
    };
    // Preview: no items, so the torso is `lit`.
    let c = component_codes(&l, &p, &name, &layer, None).unwrap();
    assert_eq!(c.name(), "QAQTlitQNhth");
    // The file names the layer's own weapon class; an empty
    // layer class names none.
    let one_hand = d2_formats::cof::CofLayer {
        weapon_class: *b"1ht\0",
        ..layer
    };
    let c = component_codes(&l, &p, &name, &one_hand, None).unwrap();
    assert_eq!(c.name(), "QAQTlitQN1ht");
    let empty = d2_formats::cof::CofLayer {
        weapon_class: [0; 4],
        ..layer
    };
    let c = component_codes(&l, &p, &name, &empty, None).unwrap();
    assert_eq!(c.name(), "QAQTlitQN");
    // A monster's choice from the spawn packet picks its `compcode` code;
    // a choice at or past the count stays `lit`.
    let mut mc = [[EMPTY; 12]; 16];
    mc[1][2] = code(b"med");
    let mut counts = [0u8; 16];
    counts[1] = 3;
    l.monster_components.insert(
        7,
        crate::world_view::unit_assets::MonsterComponents { counts, codes: mc },
    );
    let mut m = unit(MONSTER, 2, 7, 1);
    let mname = unit_cof(&l, &m).unwrap();
    let mut data = crate::bridge::world::MonsterData::default();
    data.components[1] = 2;
    m.kind = crate::bridge::world::KindData::Monster(Box::new(data));
    let c = component_codes(&l, &m, &mname, &layer, None).unwrap();
    assert!(c.name().contains("med"), "{}", c.name());
    if let crate::bridge::world::KindData::Monster(d) = &mut m.kind {
        d.components[1] = 3;
    }
    let c = component_codes(&l, &m, &mname, &layer, None).unwrap();
    assert!(c.name().contains("lit"), "{}", c.name());
    // A monster's weapon class is its BaseW.
    let m = unit(MONSTER, 2, 7, 1);
    assert_eq!(
        unit_cof(&l, &m).unwrap().full(),
        "DATA\\GLOBAL\\MONSTERS\\QM\\COF\\QMQNqwc.COF"
    );
    // Unknown class, unknown mode, a unit type without a composite: none.
    assert!(unit_cof(&l, &unit(MONSTER, 3, 8, 1)).is_none());
    assert!(unit_cof(&l, &unit(OBJECT, 4, 342, 5)).is_none());
    assert!(unit_cof(&l, &unit(4, 5, 0, 0)).is_none());
}

#[test]
fn an_object_loads_draws_and_animates() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\objects\\QO\\cof\\QOQNhth.cof",
        cof_bytes(3, &[1]),
    );
    src.insert(
        "data\\global\\objects\\QO\\QT\\QOQTlitQNhth.dc6",
        dc6(3, 4, 2),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let o = unit(OBJECT, 9, 342, 0);
    world.units.insert(o.key, o.clone());
    let mut a = assets();

    // Not loaded yet: not drawn, no error.
    assert_eq!(rules.unit_pose(&world, &o).unwrap(), None);
    assert!(loader.ensure(&world, &mut a).is_empty());
    assert_eq!(a.cofs.len(), 1);
    // One direction of three frames, and its derived shadow set
    // (`unit_shadow`): three more.
    assert_eq!(a.frames.len(), 6, "one direction of three frames + shadows");
    // Loaded once.
    assert!(loader.ensure(&world, &mut a).is_empty());
    assert_eq!(a.frames.len(), 6);

    // The frame is the model's +0x44 >> 8 (the client object
    // update animates it), whatever the tick; frame 0 is a frame.
    let mut o = o;
    for (tick, counter, frame) in [(1, 0, 0), (2, 0, 0), (3, 0x100, 1), (4, 0x2FF, 2)] {
        world.server_ticks = tick;
        o.frame = counter;
        let pose = rules.unit_pose(&world, &o).unwrap().unwrap();
        assert_eq!((pose.dir, pose.frame), (0, frame), "tick {tick}");
        let built = build(&world, &[], &rules, &a).unwrap();
        assert_eq!((built.units_drawn, built.items.len()), (1, 1));
        let item = &built.items[0];
        assert_eq!((item.x, item.y), (100, 200));
        assert_eq!(item.tag, ItemTag::Unit(9));
        assert_eq!(
            (item.shade, item.blend),
            (ShadeChain::EMPTY, BlendOp::Opaque)
        );
    }
    // No `% F`: a frame past the COF's 3 frames is used as is.
    o.frame = 0x300;
    assert_eq!(rules.unit_pose(&world, &o).unwrap().unwrap().frame, 3);
}

#[test]
fn missing_files_are_skipped_with_one_log_line() {
    let mut src = MemorySource::default();
    // The player's COF is there, its torso file is not; the monster has
    // no COF at all.
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_bytes(1, &[0, 1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    for u in [unit(PLAYER, 1, 0, 1), unit(MONSTER, 2, 7, 1)] {
        world.units.insert(u.key, u);
    }
    let mut a = assets();
    let log = loader.ensure(&world, &mut a);
    // The missing COF is logged; the component files in no archive are
    // the normal empty slots (no line).
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].contains("QMQNqwc.COF"), "{log:?}");
    assert_eq!(rules.art.read().unwrap().files.len(), 2, "both remembered");
    assert!(loader.ensure(&world, &mut a).is_empty(), "never retried");
    // The player draws (no component has a file), the monster is hidden.
    let built = build(&world, &[], &rules, &a).unwrap();
    assert_eq!((built.units_drawn, built.units_hidden), (1, 1));
    assert!(built.items.is_empty());
    // Each slot whose file is in no archive is still a draw call,
    // in slot order; no shadow call
    // without the shadow slot of a draw order.
    let calls: Vec<(u8, &str, bool)> = built
        .unit_calls
        .iter()
        .map(|c| {
            (
                c.key.sub(),
                c.path.as_ref().map_or("", |p| p.as_str()),
                c.shadow,
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            (0, "data/global/chars/qa/qh/qaqhlitqnhth.dcc", false),
            (1, "data/global/chars/qa/qt/qaqtlitqnhth.dcc", false),
        ]
    );
    assert!(built.unit_calls.iter().all(|c| c.tag == ItemTag::Unit(1)));
}

#[test]
fn an_unreadable_component_file_is_logged() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_bytes(1, &[1]),
    );
    src.insert(
        "data\\global\\chars\\QA\\QT\\QAQTlitQNhth.dcc",
        vec![1, 2, 3],
    );
    let (loader, _) = setup(src);
    let mut world = ClientWorld::default();
    let p = unit(PLAYER, 1, 0, 1);
    world.units.insert(p.key, p);
    let log = loader.ensure(&world, &mut assets());
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].contains("QAQTlitQNhth.dcc"), "{log:?}");
}

#[test]
fn units_face_their_walk_target_and_keep_the_facing() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_dirs(16, 1, &[1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let mut remote = unit(PLAYER, 1, 0, 1);
    // 0x0F walk to (20, 10): east, dir64 56 (a pathing test case).
    remote.last_path_request = Some((
        crate::bridge::world::ModeRequest {
            code: 0x01,
            record: [20, 10, 0, 0, 0, 0, 0],
        },
        1,
    ));
    let local = unit(PLAYER, 2, 0, 1);
    world.units.insert(remote.key, remote.clone());
    world.units.insert(local.key, local.clone());
    world.local_player = Some(local.key);
    let mut a = assets();
    loader.ensure(&world, &mut a);
    let dir = |w: &ClientWorld, u: &ClientUnit| rules.unit_pose(w, u).unwrap().unwrap().dir;
    // Remote player: n = 8 on a 16-direction COF, 56 snaps to 56 (table
    // 0, 0, 2, 2, … on 56 >> 2 = 14 → 14 << 2), cof_dir 14.
    assert_eq!(dir(&world, &remote), 14);
    // No facing known: 0.
    assert_eq!(dir(&world, &local), 0);
    // The local player's predicted facing (n = 16): dir64 6 → cof_dir 2
    // (test vector).
    rules.art.write().unwrap().pose_dir = Some((local.key, 6));
    assert_eq!(dir(&world, &local), 2);

    // The remote player arrives and its request is spent: the facing
    // stays (the position change (10, 10) → (20, 10) is east as well).
    let r = world.units.get_mut(&remote.key).unwrap();
    r.position = Some((20, 10));
    r.last_path_request = None;
    loader.ensure(&world, &mut a);
    assert_eq!(dir(&world, &remote), 14);
    // Standing still: kept.
    loader.ensure(&world, &mut a);
    assert_eq!(dir(&world, &remote), 14);
    // A move north that is no placement turns it: (20, 10) → (20, 0) is
    // dir64 40, cof_dir 10.
    world.units.get_mut(&remote.key).unwrap().position = Some((20, 0));
    loader.ensure(&world, &mut a);
    assert_eq!(dir(&world, &remote), 10);
}

/// The local player moved by the single-player copy before its 0x15
/// arrives (a placement, measured on edge hops: the base game draws
/// dir64 0 throughout) keeps its facing; so does the 0x15 itself.
#[test]
fn the_local_player_does_not_turn_toward_a_placement() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_dirs(16, 1, &[1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let local = unit(PLAYER, 2, 0, 1);
    world.units.insert(local.key, local.clone());
    world.local_player = Some(local.key);
    let mut a = assets();
    loader.ensure(&world, &mut a);
    let dir = |w: &ClientWorld, u: &ClientUnit| rules.unit_pose(w, u).unwrap().unwrap().dir;
    assert_eq!(dir(&world, &local), 0);
    // The copy moves it south, the 0x15 follows at the same point.
    let at = local.position.unwrap();
    world.units.get_mut(&local.key).unwrap().position = Some((at.0, at.1 + 16));
    loader.ensure(&world, &mut a);
    assert_eq!(dir(&world, &local), 0);
    world.units.get_mut(&local.key).unwrap().placements += 1;
    loader.ensure(&world, &mut a);
    assert_eq!(dir(&world, &local), 0);
}

/// A player's cast and a 0x13 soft hit in the same tick: the base game faces the
/// cast point in the cast request and the soft hit sets no
/// mode and resets no path, so the cast's facing is seen.
#[test]
fn a_soft_hit_after_a_cast_keeps_the_cast_facing() {
    use crate::bridge::modes::mode_request;
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_dirs(16, 1, &[1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let remote = unit(PLAYER, 1, 0, 1);
    let local = unit(PLAYER, 2, 0, 1);
    world.units.insert(remote.key, remote.clone());
    world.units.insert(local.key, local.clone());
    world.local_player = Some(local.key);
    let inputs = crate::bridge::world::ModelInputs::default();
    let out = crate::bridge::output::Outputs::default();
    // Cast at (20, 10): east, dir64 56; then the soft hit.
    mode_request(
        &mut world,
        &inputs,
        remote.key,
        0x15,
        [0, 0, 20, 10, 0, 0, 0],
        &out,
    )
    .unwrap();
    mode_request(&mut world, &inputs, remote.key, 0x13, [0; 7], &out).unwrap();
    let mut a = assets();
    loader.ensure(&world, &mut a);
    let dir = |w: &ClientWorld, k| rules.unit_pose(w, &w.units[&k]).unwrap().unwrap().dir;
    assert_eq!(dir(&world, remote.key), 14);
}

/// A unit in mode 18 draws its sequence frame, whatever the tick
/// (measured on a Barbarian's Whirlwind: the whirl draws
/// A1 0, 1, 2, 3, 3, 4, 5, 6, 3 on consecutive updates).
#[test]
fn a_unit_in_a_sequence_draws_the_sequence_frame() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        cof_dirs(16, 8, &[1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let local = unit(PLAYER, 2, 0, 1);
    world.units.insert(local.key, local.clone());
    world.local_player = Some(local.key);
    let mut a = assets();
    loader.ensure(&world, &mut a);
    let frame = |w: &ClientWorld| {
        rules
            .unit_pose(w, w.local().unwrap())
            .unwrap()
            .unwrap()
            .frame
    };
    world.server_ticks = 3;
    let plain = frame(&world);
    for (t, f) in [0, 1, 2, 3, 3, 4, 5, 6, 3].into_iter().enumerate() {
        world.server_ticks = 40 + t as u64;
        rules.art.write().unwrap().sequence = Some((local.key, f));
        assert_eq!(frame(&world), f, "update {t}");
    }
    rules.art.write().unwrap().sequence = None;
    world.server_ticks = 3;
    assert_eq!(frame(&world), plain, "the plain frame again");
}

/// A monster is drawn with its own +0x44 >> 8, never the
/// player's tick clock, also while its frame count is 0 (no AnimData
/// record in the model).
#[test]
fn a_monster_draws_its_own_frame_whatever_the_tick() {
    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\monsters\\QM\\cof\\QMQNqwc.cof",
        cof_bytes(8, &[1]),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let mut m = unit(MONSTER, 2, 7, 1);
    world.units.insert(m.key, m.clone());
    let mut a = assets();
    loader.ensure(&world, &mut a);
    for (count, counter, tick, frame) in [
        (0, 0, 1, 0),
        (0, 0, 50, 0),
        (0, 0x2FF, 50, 2),
        (8 << 8, 0x500, 3, 5),
        (8 << 8, 0x500, 90, 5),
    ] {
        m.frame_count = count;
        m.frame = counter;
        world.server_ticks = tick;
        let pose = rules.unit_pose(&world, &m).unwrap().unwrap();
        assert_eq!(
            pose.frame, frame,
            "count {count} +0x44 {counter:#x} tick {tick}"
        );
    }
}

/// The measured phase: the draw of tick T has had T − 1 advances
/// (rate 80, 16 frames: tick 42 → 12, 52 → 15,
/// 128 → 7, 174 → 6, 224 → 5, 43 → 13).
#[test]
fn tick_frame_has_one_advance_less_than_the_tick() {
    for (tick, frame) in [
        (42, 12),
        (43, 13),
        (52, 15),
        (128, 7),
        (174, 6),
        (224, 5),
        (274, 5),
        (354, 14),
    ] {
        assert_eq!(
            super::tick_frame(tick, 16, 80, TickRate::BASE),
            frame,
            "tick {tick}"
        );
    }
    // The old phase (T advances) gives 13 at tick 42.
    assert_eq!((42 * 80) >> 8, 13, "the old phase");
    assert_ne!(super::tick_frame(42, 16, 80, TickRate::BASE), 13);
    assert_eq!(super::tick_frame(0, 16, 80, TickRate::BASE), 0);
    assert_eq!(super::tick_frame(1, 16, 256, TickRate::BASE), 0);
    assert_eq!(super::tick_frame(2, 16, 256, TickRate::BASE), 1);
}

/// The town walk's frame counts from the walk request at speed 213
/// (`a1-walk-*`: started at tick 22, the base game frames 0, 3, 7, 3, 6, 2 at
/// ticks 32, 46, 60, 74, 88, 102).
#[test]
fn the_walk_frame_counts_from_the_walk_start() {
    for (tick, frame) in [(32, 0), (46, 3), (60, 7), (74, 3), (88, 6), (102, 2)] {
        assert_eq!(
            super::walk_frame(tick, 22, 8, 213, TickRate::BASE),
            frame,
            "tick {tick}"
        );
    }
    // The server-tick clock (rate 256) gives other frames.
    assert_ne!(super::tick_frame(46, 8, 256, TickRate::BASE), 3);
    assert_eq!(super::walk_frame(22, 22, 8, 213, TickRate::BASE), 0);
    // The run (`a1-run-*`): speed 151 from the first run click, tick 140;
    // The base game frames 5, 6, 6, 6, 6, 7, 7, 7 at ticks 150 … 248.
    let speeds = crate::bridge::predict::Speeds { walk: 6, run: 9 };
    assert_eq!(super::player_run_speed(Some(speeds)), 151);
    for (tick, frame) in [
        (150, 5),
        (164, 6),
        (178, 6),
        (192, 6),
        (206, 6),
        (220, 7),
        (234, 7),
        (248, 7),
    ] {
        assert_eq!(
            super::walk_frame(tick, 140, 8, 151, TickRate::BASE),
            frame,
            "tick {tick}"
        );
    }
    // A w = 101 at p = 100 gives other frames.
    assert_ne!(super::walk_frame(150, 140, 8, 101, TickRate::BASE), 5);
}

/// Fortress braziers #1 (`98`, BlocksLight 1) and #2 (`99`, BlocksLight
/// 0): only #1 casts its shadow (measured in the Act IV town).
#[test]
fn an_object_casts_its_shadow_only_where_it_blocks_light() {
    let mut l = looks();
    l.object_blocks_light.insert(408, [1, 1, 1, 1, 0, 0, 0, 0]);
    l.object_blocks_light.insert(409, [0; 8]);
    assert!(super::object_casts_shadow(&l, 408, 0));
    assert!(!super::object_casts_shadow(&l, 409, 0));
    assert!(!super::object_casts_shadow(&l, 408, 4));
    assert!(!super::object_casts_shadow(&l, 408, 99));
    // No row: as before.
    assert!(super::object_casts_shadow(&l, 7, 0));
}

/// The waypoint `5a` of class 496 (BlocksLight 0 in NU and OP, 1 in ON)
/// opened by a 0x0E into OP: its OP → ON finish keeps flag 0x20, so it
/// casts no shadow in ON (measured in the Frigid Highlands).
#[test]
fn an_object_shadow_follows_the_mode_of_its_last_0x20_write() {
    use crate::bridge::world::{KindData, ObjectData};
    let mut l = looks();
    l.object_blocks_light.insert(496, [0, 0, 1, 1, 1, 1, 1, 1]);
    let mut u = ClientUnit::new(UnitKey {
        unit_type: OBJECT,
        guid: 9,
    });
    u.class = 496;
    u.mode = 2;
    u.kind = KindData::Object(ObjectData {
        shadow_mode: Some(1),
        ..ObjectData::default()
    });
    assert_eq!(super::object_shadow_mode(&u), 1);
    assert!(!super::object_casts_shadow(
        &l,
        496,
        super::object_shadow_mode(&u)
    ));
    // Never written: the current mode.
    u.kind = KindData::Object(ObjectData::default());
    assert!(super::object_casts_shadow(
        &l,
        496,
        super::object_shadow_mode(&u)
    ));
}

#[test]
fn an_object_whose_draw_is_0_keeps_its_pose_but_draws_no_body() {
    let (_, mut rules) = setup(MemorySource::default());
    let o = unit(OBJECT, 9, 342, 0);
    let m = unit(MONSTER, 9, 342, 0);
    assert!(rules.unit_draws_body(&o));
    let mut looks = (*rules.looks).clone();
    looks.object_no_draw.insert(342);
    rules.looks = Arc::new(looks);
    assert!(
        !rules.unit_draws_body(&o),
        "the call alone, its shadow kept"
    );
    assert!(rules.unit_draws_body(&m), "objects only");
}

#[test]
fn frame_source_is_used_by_the_compositor_without_changing_pose_or_world() {
    use std::sync::Mutex;

    type Observation = (u32, u8, u8, usize, u8);
    struct SelectedFrame(Mutex<Vec<Observation>>);
    impl FrameSource for SelectedFrame {
        fn frame(&self, req: &FrameRequest<'_>, mut source: ComponentFrame) -> ComponentFrame {
            assert_eq!(req.codes.unit_token, code(b"QO"));
            assert_eq!(req.codes.mode_token, code(b"QN"));
            self.0.lock().unwrap().push((
                req.unit.key.guid,
                req.component,
                req.direction,
                req.frame,
                req.sub_frame,
            ));
            source.index = 1;
            source
        }
    }

    let mut src = MemorySource::default();
    src.insert(
        "data\\global\\objects\\QO\\cof\\QOQNhth.cof",
        cof_bytes(3, &[1]),
    );
    src.insert(
        "data\\global\\objects\\QO\\QT\\QOQTlitQNhth.dc6",
        dc6(3, 4, 2),
    );
    let (loader, rules) = setup(src);
    let mut world = ClientWorld::default();
    let o = unit(OBJECT, 9, 342, 0);
    world.units.insert(o.key, o.clone());
    let mut a = assets();
    assert!(loader.ensure(&world, &mut a).is_empty());
    let pose = rules.unit_pose(&world, &o).unwrap().unwrap();
    let baseline = build(&world, &[], &rules, &a).unwrap();
    let before = world.clone();
    let custom = UnitRules {
        frame_source: SelectedFrame(Mutex::new(Vec::new())),
        rules: rules.rules,
        looks: rules.looks.clone(),
        art: rules.art.clone(),
    };
    let selected = build(&world, &[], &custom, &a).unwrap();
    assert_eq!(custom.unit_pose(&world, &o).unwrap().unwrap(), pose);
    assert_eq!(world, before);
    assert_ne!(baseline.items[0].frame, selected.items[0].frame);
    assert_eq!(*custom.frame_source.0.lock().unwrap(), [(9, 1, 0, 0, 0)]);
}

#[test]
fn source_frames_retain_the_cel_for_every_fraction_and_direction() {
    let o = unit(OBJECT, 9, 342, 0);
    let name = unit_cof(&looks(), &o).unwrap();
    let cof = d2_formats::cof::Cof::parse(&cof_bytes(3, &[1])).unwrap();
    let codes = component_codes(&looks(), &o, &name, &cof.layers[0], None).unwrap();
    let path = codes
        .path(crate::rules::unit_composite::FileFormat::Dc6)
        .unwrap();
    for direction in 0..64 {
        for frame in 0..3 {
            let source = component_cel(&path, 8, 3, direction, frame).unwrap();
            for sub_frame in 0..=255 {
                let req = FrameRequest {
                    unit: &o,
                    codes: &codes,
                    component: 1,
                    direction,
                    source_direction: direction,
                    frame,
                    sub_frame,
                };
                assert_eq!(SourceFrames.frame(&req, source.clone()), source);
            }
        }
    }
}

#[test]
fn pack_playback_retains_original_hashes_starting_cof_order_and_world() {
    use super::frame_pack::{FrameKey, FramePack};
    use super::playback::{AnimationMode, PackFrames};
    use sha2::{Digest, Sha256};

    let mut src = MemorySource::default();
    let mut cof = cof_bytes(2, &[0, 1]);
    let end = cof.len();
    cof[end - 2..].copy_from_slice(&[1, 0]);
    src.insert("data/global/objects/QO/cof/QOQNhth.cof", cof);
    for component in ["QH", "QT"] {
        src.insert(
            &format!("data/global/objects/QO/{component}/QO{component}litQNhth.dc6"),
            dc6(2, 4, 2),
        );
    }
    let (loader, baseline) = setup(src);
    let mut world = ClientWorld::default();
    let mut object = unit(OBJECT, 9, 342, 0);
    object.frame = 128;
    world.units.insert(object.key, object.clone());
    let mut a = assets();
    a.palette.colors[1] = Rgb { r: 200, g: 0, b: 0 };
    a.palette.colors[200] = Rgb { r: 0, g: 200, b: 0 };
    assert!(loader.ensure(&world, &mut a).is_empty());
    let mut pack = FramePack {
        input_hash: [77; 32],
        frames: Default::default(),
    };
    let name = unit_cof(&baseline.looks, &object).unwrap();
    let cof = &a.cofs[&name.path().unwrap()];
    for layer in &cof.layers {
        let codes = component_codes(&baseline.looks, &object, &name, layer, None).unwrap();
        for fraction in [64, 128, 192] {
            pack.frames.insert(
                FrameKey {
                    kind: 2,
                    token: codes.unit_token,
                    mode: codes.mode_token,
                    component: layer.component,
                    component_token: codes.component_token,
                    armor_class: codes.armor_class,
                    weapon_class: codes.weapon_class,
                    direction: 0,
                    frame: 0,
                    sub_frame: fraction,
                },
                IndexFrame::new(4, 2, 0, 0, vec![200; 8]).unwrap(),
            );
        }
    }
    let mut playback = UnitRules {
        frame_source: PackFrames::default(),
        rules: Placed,
        looks: baseline.looks.clone(),
        art: baseline.art.clone(),
    };
    playback.frame_source.install(&pack, &mut a.frames).unwrap();
    let image_hash = |items: &[DrawItem]| {
        let rgba = crate::scene::compose_rgba(
            items,
            &a.frames,
            &a.maps,
            &a.palette,
            Rect::new(96, 196, 16, 16),
        )
        .unwrap();
        Sha256::digest(rgba)
    };
    for counter in 0..512 {
        world.server_ticks += 1;
        world.units.get_mut(&object.key).unwrap().frame = counter;
        let before = world.clone();
        let expected = build(&world, &[], &baseline, &a).unwrap();
        playback.frame_source.mode = AnimationMode::Original;
        let original = build(&world, &[], &playback, &a).unwrap();
        assert_eq!(original.items, expected.items);
        assert_eq!(image_hash(&original.items), image_hash(&expected.items));
        playback.frame_source.mode = AnimationMode::Smooth;
        let smooth = build(&world, &[], &playback, &a).unwrap();
        let pose = playback
            .unit_pose(&world, &world.units[&object.key])
            .unwrap()
            .unwrap();
        assert_eq!(
            (pose.frame, pose.sub_frame),
            (counter as usize / 256, counter as u8)
        );
        assert_eq!(smooth.items.len(), expected.items.len());
        for (selected, source) in smooth.items.iter().zip(&expected.items) {
            let mut rest = *selected;
            rest.frame = source.frame;
            assert_eq!(
                &rest, source,
                "only the picture changes, including starting COF order"
            );
        }
        if (64..256).contains(&counter) {
            assert_ne!(image_hash(&smooth.items), image_hash(&expected.items));
        } else {
            assert_eq!(
                smooth.items, expected.items,
                "source boundary or absent pack frame"
            );
        }
        assert_eq!(
            world, before,
            "selection cannot move animation events or simulation state"
        );
    }
    // A denied animation keeps its source pictures in Smooth mode, and its
    // frame events and COF order are untouched.
    playback.frame_source.deny = super::animation_deny::AnimationDeny::parse(
        "kind\tunit\tmode\treason\nobject\tQO \tQN \tdenied for the test\n",
    )
    .unwrap();
    playback.frame_source.mode = AnimationMode::Smooth;
    for counter in [64i32, 128, 192] {
        world.units.get_mut(&object.key).unwrap().frame = counter;
        let before = world.clone();
        let expected = build(&world, &[], &baseline, &a).unwrap();
        let denied = build(&world, &[], &playback, &a).unwrap();
        assert_eq!(denied.items, expected.items);
        assert_eq!(image_hash(&denied.items), image_hash(&expected.items));
        assert_eq!(world, before);
    }
    // Installation is transactional, including IDs already referenced by draws.
    let count = a.frames.len();
    assert!(playback.frame_source.install(&pack, &mut a.frames).is_err());
    assert_eq!(a.frames.len(), count);
    // Shared loader handles can overlap components and revisit a palette.
    let shared = super::playback::SharedPackFrames::default();
    shared.set_mode(AnimationMode::Smooth);
    let second_handle = shared.clone();
    shared.install(&pack, &mut a.frames).unwrap();
    second_handle.install(&pack, &mut a.frames).unwrap();
    shared.clear();
    second_handle.install(&pack, &mut a.frames).unwrap();
    assert_eq!(second_handle.mode(), AnimationMode::Smooth);
    assert_eq!(
        a.frames.len(),
        count,
        "reused pictures retain their resident IDs"
    );
}

/// A DC6 of one direction whose frame `f` is `w` × `h` pixels of `values[f]`.
fn dc6_values(values: &[u8], w: u32, h: u32) -> Vec<u8> {
    let frames = values.len() as u32;
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for &value in values {
        let mut rows = Vec::new();
        for _ in 0..h {
            rows.push(w as u8);
            rows.extend(std::iter::repeat_n(value, w as usize));
            rows.push(0x80);
        }
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

/// The install-time bake and the play loader end to end: `bake-anims`
/// writes a pack and `bake.json`; play only loads that pack (checked
/// against its source file) and never generates. A changed source file
/// makes the pack stale: source frames play.
#[test]
fn baked_frames_load_in_play_and_stale_or_missing_packs_play_source_frames() {
    use super::animation_bake::{bake_item, work_items, BakeIndex, BAKE_VERSION, INDEX, PACKS};
    use super::animation_baked::BakedAnimations;
    use super::generation_job::GENERATOR_SETTINGS;
    use super::playback::{AnimationMode, SharedPackFrames};

    // Monster row 242 draws from DC6.
    let mut looks = looks();
    looks.monsters.insert(
        242,
        MonsterRow {
            token: code(b"QM"),
            base_w: Some(code(b"qwc")),
            composite_death: false,
            directions: [0; 16],
        },
    );
    let looks = Arc::new(looks);
    let monster = unit(MONSTER, 2, 242, 1);
    let name = unit_cof(&looks, &monster).unwrap();
    let cof_path = name.path().unwrap();
    let cof = d2_formats::cof::Cof::parse(&cof_bytes(2, &[1])).unwrap();
    let codes = component_codes(&looks, &monster, &name, &cof.layers[0], None).unwrap();
    let file = codes.file(crate::rules::unit_composite::FileFormat::Dc6);
    let source = |values: &[u8]| {
        let mut src = MemorySource::default();
        src.insert(cof_path.as_str(), cof_bytes(2, &[1]));
        src.insert(&file, dc6_values(values, 4, 4));
        Arc::new(src)
    };
    let palette = Palette {
        colors: std::array::from_fn(|i| Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        }),
    };
    let ramps = [0u8; 256];
    let generator = super::generator::Generator::new(&palette, &ramps);
    let tables = generator.tables();

    // The bake.
    let dir = std::env::temp_dir().join(format!("d2rust-bake-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(PACKS)).unwrap();
    let baked = source(&[10, 30]);
    let items = work_items(
        [file.as_str()],
        &looks,
        &super::animation_deny::AnimationDeny::default(),
    );
    assert_eq!(items.len(), 1, "{file}");
    let entry = bake_item(
        baked.as_ref(),
        &items[0],
        &generator,
        &tables,
        &dir.join(PACKS),
        &[],
    )
    .unwrap()
    .expect("generated frames");
    assert!(entry.frames > 0);
    // A rerun keeps the pack (resume) instead of generating it again.
    let again = bake_item(
        baked.as_ref(),
        &items[0],
        &generator,
        &tables,
        &dir.join(PACKS),
        &[],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (again.pack.as_str(), again.rejected),
        (entry.pack.as_str(), None)
    );
    // A mod bake reuses the base bake's pack of the same identity.
    let reuse = dir.join("mod");
    std::fs::create_dir_all(reuse.join(PACKS)).unwrap();
    let copied = bake_item(
        baked.as_ref(),
        &items[0],
        &generator,
        &tables,
        &reuse.join(PACKS),
        std::slice::from_ref(&dir),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (copied.pack.as_str(), copied.rejected),
        (entry.pack.as_str(), None)
    );
    assert_eq!(
        std::fs::read(reuse.join(&copied.pack)).unwrap(),
        std::fs::read(dir.join(&entry.pack)).unwrap()
    );
    std::fs::write(dir.join(super::animation_pack::LUT_FILE), tables.bytes()).unwrap();
    let index = BakeIndex {
        version: BAKE_VERSION,
        game_version: crate::launch::VERSION.into(),
        generator: GENERATOR_SETTINGS.into(),
        stamp: "test".into(),
        mod_id: None,
        palette: String::new(),
        blend_lut: format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(tables.bytes())
        ),
        packs: vec![entry],
    };
    std::fs::write(dir.join(INDEX), serde_json::to_vec(&index).unwrap()).unwrap();

    // Play: load the drawn monster's pack, through the loader thread.
    let play = |src: Arc<MemorySource>, mode: AnimationMode, dir: &std::path::Path| {
        let loader = UnitArtLoader {
            source: src.clone(),
            looks: looks.clone(),
            art: SharedUnitArt::default(),
        };
        let mut world = ClientWorld::default();
        world.units.insert(monster.key, monster.clone());
        let mut a = assets();
        assert!(loader.ensure(&world, &mut a).is_empty());
        assert_eq!(a.cofs.len(), 1);
        let frames = SharedPackFrames::default();
        frames.set_mode(mode);
        let (p, r) = (palette.clone(), ramps);
        let mut baked =
            BakedAnimations::with_inputs(src, looks.clone(), frames.clone(), dir, move || {
                Ok((p, r))
            });
        let start = std::time::Instant::now();
        let before = a.frames.len();
        baked.update(&world, &loader.art, &mut a);
        while baked.waiting() > 0 && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(2));
            baked.update(&world, &loader.art, &mut a);
        }
        assert_eq!(baked.waiting(), 0, "the loader answers every request");
        (baked.listed(), frames.len(), a.frames.len() - before)
    };
    let (listed, selected, resident) = play(source(&[10, 30]), AnimationMode::Smooth, &dir);
    assert_eq!(listed, 1);
    // Each selected picture is resident with its derived `#shadow` set.
    assert!(
        selected > 0 && 2 * selected == resident,
        "{selected} {resident}"
    );
    // Other source pictures: the pack is stale and not used.
    let (_, selected, resident) = play(source(&[10, 50]), AnimationMode::Smooth, &dir);
    assert_eq!((selected, resident), (0, 0));
    // Original asks for nothing.
    let (_, selected, _) = play(source(&[10, 30]), AnimationMode::Original, &dir);
    assert_eq!(selected, 0);
    // No bake: nothing loads, and nothing is generated in its place.
    let empty = dir.join("none");
    let (listed, selected, resident) = play(source(&[10, 30]), AnimationMode::Smooth, &empty);
    assert_eq!((listed, selected, resident), (0, 0, 0));
    assert!(!empty.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A facing between the file's facings draws the source facing's samples.
#[test]
fn a_facing_between_file_facings_uses_the_source_facing_samples() {
    use super::frame_pack::{FrameKey, FramePack};
    use super::playback::{AnimationMode, PackFrames};
    let o = unit(OBJECT, 1, 342, 0);
    let codes = crate::rules::unit_composite::ComponentCodes::new(
        crate::rules::unit_composite::CompositeKind::Object,
        code(b"QO"),
        code(b"QT"),
        Some(code(b"lit")),
        code(b"QN"),
        code(b"hth"),
    )
    .unwrap();
    let mut frames = PackFrames::default();
    frames.mode = AnimationMode::Smooth;
    let mut store = crate::frames::FrameStore::new();
    let key = FrameKey {
        kind: 2,
        token: *b"QO  ",
        mode: *b"QN  ",
        component: 1,
        component_token: *b"QT  ",
        armor_class: *b"LIT ",
        weapon_class: *b"hth ",
        direction: 8,
        frame: 0,
        sub_frame: 64,
    };
    let pack = FramePack {
        input_hash: [5; 32],
        frames: [(key, IndexFrame::new(1, 1, 0, 0, vec![9]).unwrap())].into(),
    };
    frames.install(&pack, &mut store).unwrap();
    let source = ComponentFrame {
        set: crate::frames::FrameSetKey::new(
            "data/global/objects/qo/qt/x.dc6",
            crate::frames::FramePart::Dir(0),
        )
        .unwrap(),
        index: 0,
    };
    let ask = |direction, source_direction, sub_frame| {
        frames.frame(
            &FrameRequest {
                unit: &o,
                codes: &codes,
                component: 1,
                direction,
                source_direction,
                frame: 0,
                sub_frame,
            },
            source.clone(),
        )
    };
    assert_ne!(ask(10, 8, 64), source, "folded keys, source facing");
    assert_eq!(ask(10, 8, 64), ask(8, 8, 100));
    assert_eq!(ask(10, 16, 64), source, "no samples for that facing");
    assert_eq!(ask(10, 8, 10), source, "source quartile");
}

/// F5: the drawn animations of every unit kind advance by speeds stated
/// per base-rate tick, so at the game's 60 Hz they play the same frames a
/// second as at 25 Hz (before the fix the local walk, the run and the
/// tick-clock fallback added the base speed every 60 Hz tick: 2.4× too
/// fast). Two seconds of draws per mode; the expected count is the
/// table speed · 25 · 2 / 256.
#[test]
fn drawn_animations_play_in_real_time_at_every_rate() {
    use crate::bridge::{monster_anim, player_anim};
    const FRAMES: u8 = 200;
    let mut src = MemorySource::default();
    for path in [
        "data\\global\\chars\\QA\\cof\\QAQNhth.cof",
        "data\\global\\chars\\QA\\cof\\QAQWhth.cof",
        "data\\global\\chars\\QA\\cof\\QAQRhth.cof",
        "data\\global\\monsters\\QM\\cof\\QMQNqwc.cof",
    ] {
        src.insert(path, cof_bytes(FRAMES, &[1]));
    }
    let mut l = looks();
    l.player_modes = codes(&[b"QD", b"QN", b"QW", b"QR"]);
    let (loader, rules) = setup_with(src, l);
    // (what, expected frames over 2 s) per mode, measured at each rate.
    let mut results: Vec<(&str, u32, u64)> = Vec::new();
    for hz in [25u32, 60] {
        let rate = d2_sim::tick::TickRate::new(hz).unwrap();
        let mut world = ClientWorld {
            tick_rate: rate,
            ..ClientWorld::default()
        };
        let local = unit(PLAYER, 1, 0, 2);
        let remote = unit(PLAYER, 2, 0, 1);
        let idle = {
            let mut u = unit(PLAYER, 3, 0, 1);
            u.speed = Some(128);
            u.frame_count = i32::from(FRAMES) << 8;
            u
        };
        let monster = {
            let mut u = unit(MONSTER, 4, 7, 1);
            u.speed = Some(160);
            u.frame_count = i32::from(FRAMES) << 8;
            u
        };
        for u in [&local, &remote, &idle, &monster] {
            world.units.insert(u.key, u.clone());
        }
        world.local_player = Some(local.key);
        let mut a = assets();
        loader.ensure(&world, &mut a);
        let frame = |w: &ClientWorld, key| {
            rules.unit_pose(w, &w.units[&key]).unwrap().unwrap().frame as u64
        };
        let since = 10;
        let ticks = 2 * u64::from(hz);
        let count = |w: &mut ClientWorld, key, speed: Option<u64>, step: bool| {
            let mut total = 0;
            w.server_ticks = since;
            rules.art.write().unwrap().pose_since = speed.map(|s| (key, since, s));
            let mut last = frame(w, key);
            for t in 1..=ticks {
                w.server_ticks = since + t;
                let f = since as i32 + t as i32;
                w.server_frame = Some(f);
                if step {
                    if key.unit_type == MONSTER {
                        let u = w.units.get_mut(&key).unwrap();
                        monster_anim::advance_at(u, rate, f);
                    } else {
                        player_anim::step(w, &Default::default(), key).unwrap();
                    }
                }
                let now = frame(w, key);
                total += (now + u64::from(FRAMES) - last) % u64::from(FRAMES);
                last = now;
            }
            total
        };
        // The local player's predicted walk (speed 213) and run (151).
        results.push(("walk", hz, count(&mut world, local.key, Some(213), false)));
        world.units.get_mut(&local.key).unwrap().mode = 3;
        loader.ensure(&world, &mut a);
        results.push(("run", hz, count(&mut world, local.key, Some(151), false)));
        // A player the model has no speed for: the COF's rate (256)
        // on the server tick clock.
        results.push(("tick clock", hz, count(&mut world, remote.key, None, false)));
        // The model's own animation: a neutral player (128) and a monster
        // (160).
        results.push(("idle", hz, count(&mut world, idle.key, None, true)));
        results.push(("monster", hz, count(&mut world, monster.key, None, true)));
    }
    for (what, speed) in [
        ("walk", 213u64),
        ("run", 151),
        ("tick clock", 256),
        ("idle", 128),
        ("monster", 160),
    ] {
        let want = speed * 25 * 2 / 256;
        for hz in [25, 60] {
            let got = results.iter().find(|r| r.0 == what && r.1 == hz).unwrap().2;
            // Within one frame: the phase of the 8.8 counter at the start.
            assert!(
                got.abs_diff(want) <= 1,
                "{what} at {hz} Hz: {got} frames in 2 s, want {want} ({:.1}/s vs {:.1}/s)",
                got as f64 / 2.0,
                want as f64 / 2.0
            );
        }
    }
}

/// Every picture a Smooth pack selects casts its unit shadow: a generated
/// quartile (sub-frames 64, 128, 192) installed without its derived
/// `#shadow` set drew the body and dropped the shadow, so shadows blinked
/// off three quartiles in four while the source frame held (E3).
#[test]
fn generated_frames_cast_the_shadow_on_every_quartile() {
    use super::frame_pack::{FrameKey, FramePack};
    use super::playback::{AnimationMode, PackFrames};
    use crate::composite::{ComponentDraw, ComponentFrame, Slot};
    use crate::rules::draw_order::OrderKey;
    use crate::rules::shading::ShadeTables;
    use crate::world_view::unit_shadow;

    let mut src = MemorySource::default();
    let mut cof = cof_bytes(2, &[1]);
    cof[28 + 1] = 1; // the layer's shadow byte
    src.insert("data/global/objects/QO/cof/QOQNhth.cof", cof);
    src.insert("data/global/objects/QO/QT/QOQTlitQNhth.dc6", dc6(2, 4, 4));
    let (loader, baseline) = setup(src);
    let mut world = ClientWorld::default();
    let object = unit(OBJECT, 9, 342, 0);
    world.units.insert(object.key, object.clone());
    let mut a = assets();
    assert!(loader.ensure(&world, &mut a).is_empty());
    let pl2 =
        d2_formats::palette::Pl2::parse(&crate::world_view::tile_assets::tests::pl2()).unwrap();
    a.shades = Some(ShadeTables::push(&mut a.maps, &pl2));
    let name = unit_cof(&baseline.looks, &object).unwrap();
    let cof = a.cofs[&name.path().unwrap()].clone();
    let codes = component_codes(&baseline.looks, &object, &name, &cof.layers[0], None).unwrap();
    let mut pack = FramePack {
        input_hash: [78; 32],
        frames: Default::default(),
    };
    for frame in 0..2 {
        for fraction in [64, 128, 192] {
            pack.frames.insert(
                FrameKey {
                    kind: 2,
                    token: codes.unit_token,
                    mode: codes.mode_token,
                    component: 1,
                    component_token: codes.component_token,
                    armor_class: codes.armor_class,
                    weapon_class: codes.weapon_class,
                    direction: 0,
                    frame,
                    sub_frame: fraction,
                },
                IndexFrame::new(4, 4, 0, -3, vec![200; 16]).unwrap(),
            );
        }
    }
    let mut playback = UnitRules {
        frame_source: PackFrames::default(),
        rules: Placed,
        looks: baseline.looks.clone(),
        art: baseline.art.clone(),
    };
    playback.frame_source.mode = AnimationMode::Smooth;
    playback.frame_source.install(&pack, &mut a.frames).unwrap();
    let at = OrderKey {
        pass: 5,
        major: 3,
        minor: 1,
    };
    let mut generated = 0;
    for counter in 0..512 {
        world.units.get_mut(&object.key).unwrap().frame = counter;
        let body = build(&world, &[], &playback, &a).unwrap().items;
        assert_eq!(body.len(), 1);
        let (set, index) = a.frames.owner(body[0].frame).unwrap();
        generated += usize::from(set.path().starts_with("generated/"));
        let draw = ComponentDraw {
            slot: Slot {
                slot: 0,
                component: 1,
                layer: 0,
            },
            frame: ComponentFrame {
                set: set.clone(),
                index,
            },
            item: body[0],
        };
        let shadows = unit_shadow::draws(&cof, 9, at, &[draw], &a).unwrap();
        assert_eq!(shadows.len(), 1, "counter {counter}: the shadow is drawn");
        assert!(shadows[0].clip.width > 0, "counter {counter}");
    }
    assert_eq!(generated, 2 * 192, "the generated quartiles were selected");
}
