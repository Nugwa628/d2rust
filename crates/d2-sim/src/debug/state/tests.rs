//! Tests for the `state-1` snapshot: the lines of hand-built snapshots, and snapshots
//! of the action fixture's game with the path provider on.

use super::*;
use crate::rng::Seed;
use crate::wiring::action::tests::{Fx, LEVEL};

fn unit(ut: u8, g: u32) -> UnitState {
    UnitState {
        ut,
        g,
        ..UnitState::default()
    }
}

/// A unit with every field set (the values tell the keys apart).
fn full_unit() -> UnitState {
    UnitState {
        path_copy: None,
        ut: 1,
        g: 7,
        cl: Some(2),
        m: Some(3),
        x: Some(4),
        y: Some(5),
        xf: Some(6),
        yf: Some(7),
        tx: Some(8),
        ty: Some(9),
        d: Some(10),
        fr: Some(-11),
        fc: Some(12),
        sp: Some(-13),
        s: Some([14, 4_000_000_000]),
        act: Some(15),
        lv: Some(16),
        hp: Some(17),
        hpx: Some(18),
        mp: Some(19),
        mpx: Some(20),
        st: Some(21),
        stx: Some(22),
        str: Some(23),
        ene: Some(24),
        dex: Some(25),
        vit: Some(26),
        lvl: Some(27),
        own: Some(28),
        iq: Some(29),
        ifl: Some(30),
        fi: Some(-31),
        il: Some(32),
        aa: Some(33),
        pf: Some([34, 35, 36]),
        sf: Some([37, 38, 39]),
        rp: Some(40),
        rs: Some(41),
        ik: Some([42, 4_000_000_001]),
        ss: Some(43),
        is: Some(vec![[44, 0, 45], [46, 1, -47]]),
        fs: Some(vec![[0, 0, 48], [7, 0, 49]]),
        sts: Some(vec![28, 33]),
        q: None,
    }
}

// A player's line carries its non-zero quest words as `[slot, word]` pairs.
#[test]
fn quest_record_words_on_the_player_line() {
    use crate::world::quests::QuestFlags;
    let mut f = QuestFlags::default();
    f.set(1, 13);
    f.set(1, 0);
    f.set(7, 0);
    f.set(41, 15);
    assert_eq!(quest_words(&f), vec![[1, 0x2001], [7, 1], [41, 0x8000]]);
    assert!(quest_words(&QuestFlags::default()).is_empty());
    let mut s = StateSnapshot {
        frame: 1,
        seed: [0, 0],
        units: vec![unit(0, 1), unit(1, 1)],
    };
    s.set_quests(1, quest_words(&f));
    s.set_quests(9, vec![[2, 2]]);
    assert_eq!(
        s.to_json_line(),
        r#"{"k":"snap","f":1,"seed":[0,0],"units":[{"ut":0,"g":1,"q":[[1,8193],[7,1],[41,32768]]},{"ut":1,"g":1}]}"#
    );
    assert!(!FIELDS.contains(&"q") && HOST_FIELDS == ["q"]);
}

// A snap line holds every key, in `FIELDS` order.
#[test]
fn a_snap_line_has_every_key_in_table_order() {
    let s = StateSnapshot {
        frame: 3,
        seed: [1234, 666],
        units: vec![full_unit()],
    };
    assert_eq!(
        s.to_json_line(),
        concat!(
            r#"{"k":"snap","f":3,"seed":[1234,666],"units":[{"ut":1,"g":7,"cl":2,"m":3,"#,
            r#""x":4,"y":5,"xf":6,"yf":7,"tx":8,"ty":9,"d":10,"fr":-11,"fc":12,"sp":-13,"#,
            r#""s":[14,4000000000],"act":15,"lv":16,"hp":17,"hpx":18,"mp":19,"mpx":20,"#,
            r#""st":21,"stx":22,"str":23,"ene":24,"dex":25,"vit":26,"lvl":27,"own":28,"#,
            r#""iq":29,"if":30,"fi":-31,"il":32,"aa":33,"pf":[34,35,36],"sf":[37,38,39],"#,
            r#""rp":40,"rs":41,"ik":[42,4000000001],"ss":43,"is":[[44,0,45],[46,1,-47]],"#,
            r#""fs":[[0,0,48],[7,0,49]],"sts":[28,33]}]}"#
        )
    );
    // The key order of the line is the order of `FIELDS`.
    let line = s.to_json_line();
    let mut at = 0;
    for k in FIELDS {
        let p = line[at..].find(&format!("\"{k}\":")).expect(k) + at;
        at = p;
    }
}

// A key a unit does not have is left out of its line, never written as null.
#[test]
fn absent_fields_are_omitted_never_null() {
    let item = UnitState {
        cl: Some(500),
        m: Some(0),
        s: Some([1, 2]),
        ..unit(4, 9)
    };
    let s = StateSnapshot {
        frame: 0,
        seed: [0, 0],
        units: vec![unit(5, 1), item],
    };
    let line = s.to_json_line();
    assert_eq!(
        line,
        r#"{"k":"snap","f":0,"seed":[0,0],"units":[{"ut":5,"g":1},{"ut":4,"g":9,"cl":500,"m":0,"s":[1,2]}]}"#
    );
    assert!(!line.contains("null"));
}

// Units sort by type, then by GUID.
#[test]
fn units_sort_by_type_then_guid() {
    let mut s = StateSnapshot {
        frame: 1,
        seed: [1, 666],
        units: vec![unit(4, 1), unit(1, 3), unit(0, 1), unit(1, 2), unit(5, 0)],
    };
    s.sort_units();
    let keys: Vec<(u8, u32)> = s.units.iter().map(|u| (u.ut, u.g)).collect();
    assert_eq!(keys, [(0, 1), (1, 2), (1, 3), (4, 1), (5, 0)]);
}

// The header and footer lines have the documented keys.
#[test]
fn header_and_footer_lines() {
    let h = Header {
        side: "d2rust".into(),
        tool: "d2-client state-dump 0.1.0".into(),
        date: "2026-10-09".into(),
        command: "d2-client state-dump --save \"a b\\c.d2s\"".into(),
        fields: vec!["ut".into(), "g".into()],
        gaps: vec!["own: why\n".into()],
        save: Some("ScnAma.d2s".into()),
        seed: Some(1234),
        tick_rate: crate::tick::TickRate::GAME,
    };
    assert_eq!(
        h.to_json_line(),
        concat!(
            r#"{"k":"header","format":"state-1","side":"d2rust","tool":"d2-client state-dump 0.1.0","#,
            r#""date":"2026-10-09","command":"d2-client state-dump --save \"a b\\c.d2s\"","#,
            r#""fields":["ut","g"],"gaps":["own: why\n"],"save":"ScnAma.d2s","seed":1234,"tick_rate":60}"#
        )
    );
    let bare = Header::default().to_json_line();
    assert!(
        bare.ends_with(r#""fields":[],"gaps":[],"tick_rate":25}"#),
        "{bare}"
    );
    assert_eq!(
        footer_line(5, &["a"]),
        r#"{"k":"footer","snaps":5,"notes":["a"]}"#
    );
    assert_eq!(
        footer_line::<&str>(0, &[]),
        r#"{"k":"footer","snaps":0,"notes":[]}"#
    );
    assert_eq!(json_string("\u{1}"), r#""\u0001""#);
}

/// The action fixture with the path provider on: a player and a monster
/// in room A with stats, one tick run.
fn small_game() -> (Fx, UnitId, UnitId) {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 1, a, 3, 4);
    let m = fx.spawn(UnitType::Monster, 0, a, 6, 5);
    fx.stats(p, &[(0, 25), (3, 20), (6, 50 << 8), (7, 60 << 8), (12, 4)]);
    fx.stats(m, &[(6, 9 << 8)]);
    fx.tick();
    (fx, p, m)
}

/// Everything the snapshot reads, as text.
fn state_of(fx: &Fx) -> String {
    let s = &fx.sim.sys;
    format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
        fx.game, s.units, s.stats, s.hooks.paths, s.hooks.game_seed
    )
}

// Taking a snapshot reads the game and changes nothing.
#[test]
fn a_snapshot_reads_the_game_and_changes_nothing() {
    let (fx, _, _) = small_game();
    let before = state_of(&fx);
    let one = snapshot(&fx.game, &fx.sim.sys).to_json_line();
    let two = snapshot(&fx.game, &fx.sim.sys).to_json_line();
    assert_eq!(one, two);
    assert_eq!(state_of(&fx), before);
}

// Each snapshot key reads from its home in the wired game.
#[test]
fn a_snapshot_maps_the_d2rust_homes() {
    let (mut fx, p, m) = small_game();
    fx.sim.hooks().game_seed = Seed::new(0xDEAD_BEEF, 777);
    fx.seed(p, Seed::new(11, 22));
    let s = snapshot(&fx.game, &fx.sim.sys);
    assert_eq!(s.frame, fx.game.frame);
    assert_eq!(s.frame, 1);
    assert_eq!(s.seed, [0xDEAD_BEEF, 777]);
    let pg = fx.game.lists.unit(p).unwrap().guid;
    let mg = fx.game.lists.unit(m).unwrap().guid;
    let pu = s.units.iter().find(|u| (u.ut, u.g) == (0, pg)).unwrap();
    let mu = s.units.iter().find(|u| (u.ut, u.g) == (1, mg)).unwrap();
    let rec = fx.sim.sys.units.get(p).unwrap().clone();
    assert_eq!(pu.cl, Some(1));
    assert_eq!(pu.m, Some(rec.mode));
    assert_eq!(pu.s, Some([11, 22]));
    assert_eq!(pu.act, Some(0));
    assert_eq!(
        (pu.fr, pu.fc, pu.sp),
        (
            Some(rec.anim.frame),
            Some(rec.anim.frame_count),
            Some(rec.anim.speed)
        )
    );
    let path = fx
        .sim
        .hooks()
        .paths
        .as_ref()
        .unwrap()
        .dynamic(p)
        .unwrap()
        .clone();
    assert_eq!((pu.x, pu.y), (Some(3), Some(4)));
    assert_eq!(pu.xf, Some(path.precise_x as u16));
    assert_eq!((pu.tx, pu.ty), (Some(path.target_x), Some(path.target_y)));
    assert_eq!(pu.d, Some(path.direction));
    assert_eq!(pu.lv, Some(LEVEL));
    // Raw stats: base 0, 3, 12; full 6, 7; absent keys read 0.
    assert_eq!(
        (pu.str, pu.ene, pu.dex, pu.vit, pu.lvl),
        (Some(25), Some(0), Some(0), Some(20), Some(4))
    );
    assert_eq!(
        (pu.hp, pu.hpx, pu.mp),
        (Some(50 << 8), Some(60 << 8), Some(0))
    );
    assert_eq!(mu.hp, Some(9 << 8));
    assert_eq!((mu.x, mu.y), (Some(6), Some(5)));
    assert_eq!(pu.own, None);
    // Sorted, and every unit of the lists is there.
    let n: usize = UnitType::ALL
        .iter()
        .map(|&t| fx.game.lists.units_of_type(t).len())
        .sum();
    assert_eq!(s.units.len(), n);
    assert!(s
        .units
        .windows(2)
        .all(|w| (w[0].ut, w[0].g) < (w[1].ut, w[1].g)));
}

// `coverage` lists every unit key except the gaps.
#[test]
fn coverage_lists_every_key_but_the_gaps() {
    let (fx, _, _) = small_game();
    let (fields, gaps) = coverage(&fx.sim.sys);
    let want: Vec<String> = FIELDS.iter().map(|k| (*k).to_owned()).collect();
    assert_eq!(fields, want);
    assert!(gaps.is_empty());
    // Without the path provider the path keys are gaps too.
    let bare = Fx::new();
    let (fields, gaps) = coverage(&bare.sim.sys);
    assert!(PATH_FIELDS.iter().all(|k| !fields.iter().any(|f| f == k)));
    assert_eq!(gaps.len(), 1);
}

/// AnimData with one record per name: (name, frames, speed), no events.
fn anim_data(rows: &[(&[u8; 8], u32, u32)]) -> d2_formats::animdata::AnimData {
    use d2_formats::animdata::{self, AnimData, AnimRecord};
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    for &(name, frames, speed) in rows {
        let len = name.iter().position(|&b| b == 0).unwrap_or(8);
        a.buckets[animdata::hash(&name[..len])].push(AnimRecord {
            name: *name,
            frames,
            speed,
            events: [0; animdata::EVENTS],
        });
    }
    a
}

// A joined player's line reads the frame count and speed of its neutral mode.
#[test]
fn a_joined_player_snapshots_the_frame_count_and_speed_of_its_mode() {
    // The Amazon's town and field neutral animations (AMTNHTH 16 frames at
    // speed 80, AMNUHTH 8 at 128): a player standing in town reads frame
    // count 16 << 8 and speed 80 with no animated start run.
    const TN: &[u8; 8] = b"AMTNHTH\0";
    const NU: &[u8; 8] = b"AMNUHTH\0";
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    {
        let h = fx.sim.hooks();
        h.anim_data = Some(std::sync::Arc::new(anim_data(&[
            (TN, 16, 80),
            (NU, 8, 128),
        ])));
        h.x.names.insert((UnitType::Player, 5), *TN);
        h.x.names.insert((UnitType::Player, 1), *NU);
    }
    let a = fx.a;
    let p = fx.spawn(UnitType::Player, 1, a, 3, 4);
    // The rate stats every loaded player has (stats 67–69 = 100), read when
    // the animation rate is computed.
    fx.stats(p, &[(67, 100), (68, 100), (69, 100)]);
    // A fresh unit starts in mode 0, then the join sets its neutral mode,
    // whose re-init fills the frame count and speed.
    fx.sim.sys.units.get_mut(p).unwrap().mode = 0;
    fx.sim
        .sys
        .with(&mut fx.game, |sim, hooks| {
            crate::units::modes::player_join(sim, hooks, p)
        })
        .unwrap();
    let mode = fx.sim.sys.units.get(p).unwrap().mode;
    let (fc, sp) = if mode == 5 {
        (16 << 8, 80)
    } else {
        (8 << 8, 128)
    };
    let s = snapshot(&fx.game, &fx.sim.sys);
    let g = fx.game.lists.unit(p).unwrap().guid;
    let u = s.units.iter().find(|u| (u.ut, u.g) == (0, g)).unwrap();
    assert_eq!((u.m, u.fc, u.sp), (Some(mode), Some(fc), Some(sp)));
}

// A spawned monster's path target is its spawn point.
#[test]
fn a_spawned_monster_snapshots_its_spawn_point_as_the_path_target() {
    // A monster spawned at a point and idle in mode 1 has that point as its
    // path target: the creation mode request's target point is the spawn
    // point.
    let (fx, _, m) = small_game();
    let s = snapshot(&fx.game, &fx.sim.sys);
    let g = fx.game.lists.unit(m).unwrap().guid;
    let u = s.units.iter().find(|u| (u.ut, u.g) == (1, g)).unwrap();
    assert_eq!((u.x, u.y), (Some(6), Some(5)));
    assert_eq!((u.tx, u.ty), (Some(6), Some(5)));
}

// A walking monster's animation speed comes from its velocity, not from AnimData.
#[test]
fn a_walking_monster_snapshots_the_velocity_mode_speed() {
    // A town monster such as Charsi (class 154, its own base, walk speed
    // 256) walking reads speed 192: w (the walk speed, 256) * p / 100 with
    // p = 75, not the AnimData speed.
    const WL: &[u8; 8] = b"M0WLHTH\0";
    let (mut fx, _, m) = small_game();
    {
        let h = fx.sim.hooks();
        h.anim_data = Some(std::sync::Arc::new(anim_data(&[(WL, 8, 256)])));
        h.x.names.insert((UnitType::Monster, 2), *WL);
        h.x.names.insert((UnitType::Monster, 1), *WL);
    }
    // A monster's initial rate stats (velocity percent 75, attack rate 100,
    // other animation rate 100).
    fx.stats(m, &[(67, 75), (68, 100), (69, 100)]);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 2;
    let sp = fx.sim.sys.with(&mut fx.game, |sim, hooks| {
        crate::units::hooks::UnitHooks::anim_rate(hooks, sim, m)
    });
    assert_eq!(sp, 192);
    // Mode 1 (no velocity modifier): the other animation rate is 100, so the
    // speed is the AnimData speed.
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    let sp = fx.sim.sys.with(&mut fx.game, |sim, hooks| {
        crate::units::hooks::UnitHooks::anim_rate(hooks, sim, m)
    });
    assert_eq!(sp, 256);
}

// A monster's `own` is its control's minion owner.
#[test]
fn a_monsters_own_is_its_control_minion_owner() {
    use crate::monsters::ai::{AiControl, AiStore, UnitRef};
    let (mut fx, p, m) = small_game();
    let mut store = AiStore::new();
    store.entry(m).control = Some(AiControl {
        minion_owner: Some(UnitRef {
            ty: UnitType::Player,
            guid: 77,
        }),
        ..AiControl::default()
    });
    fx.sim.hooks().ai = Some(store);
    let s = snapshot(&fx.game, &fx.sim.sys);
    let unit = |t: UnitType, id| {
        let g = fx.game.lists.unit(id).unwrap().guid;
        s.units
            .iter()
            .find(|u| u.ut == t as u8 && u.g == g)
            .unwrap()
            .own
    };
    assert_eq!(unit(UnitType::Monster, m), Some(77));
    assert_eq!(unit(UnitType::Player, p), None);
    // A released pack (GUID -1) has none.
    fx.sim
        .hooks()
        .ai
        .as_mut()
        .unwrap()
        .control_mut(m)
        .unwrap()
        .minion_owner = Some(UnitRef {
        ty: UnitType::Player,
        guid: u32::MAX,
    });
    let s = snapshot(&fx.game, &fx.sim.sys);
    let g = fx.game.lists.unit(m).unwrap().guid;
    assert_eq!(
        s.units.iter().find(|u| u.ut == 1 && u.g == g).unwrap().own,
        None
    );
}

// State ids are the set bits of the list's state words, ascending.
#[test]
fn state_ids_are_the_set_bits_ascending() {
    assert_eq!(set_bits(&[0b1001, 0, 1 << 31]), vec![0, 3, 95]);
    assert!(set_bits(&[0, 0]).is_empty());
}
