//! What the client receives while its player walks from the town into
//! the Blood Moor on the Act I-shaped synthetic set (first playable
//! scope G17 / G18, server half): the room switch's S→C 0x07 / 0x08 for
//! rooms of both levels, and the Blood Moor's monsters added to the
//! client (S→C 0xAC) once they spawn.
//!
//! The town gets one NPC preset here (DS1 type 1, a `keeper` row added
//! to `monpreset` act 1), as the Rogue Encampment's NPCs are (G17).
//!
//! The set's `monstats` rows get `isSpawn` here (the shared set leaves it
//! empty, so population draws no region entry and no monster
//! ever spawns). No `Covers:` claim on the data: it is made up.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use d2_formats::ds1::Ds1Object;
use d2_sim::units::UnitType;
use test_fixtures::act1::{self, BLOOD_MOOR, COLD_PLAINS, TOWN};
use test_fixtures::drlg::archive_name;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{border_goals, Session, Setup};
use test_fixtures::install;
use test_fixtures::synth::Synthetic;

/// The Act I set with `isSpawn` on every monster that has a rarity (the
/// non-NPC rows).
fn spawning_act1() -> Synthetic {
    let mut s = act1::act1();
    let f = s.tables.file("monstats.txt");
    let col = |name: &str| {
        f.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .unwrap()
    };
    let (rarity, id) = (col("Rarity"), col("Id"));
    let rows: Vec<usize> = f
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r[rarity].is_empty() && !r[id].is_empty())
        .map(|(i, _)| i)
        .collect();
    assert!(!rows.is_empty(), "the set has spawnable monsters");
    for i in rows {
        s.tables.set("monstats", i, "isSpawn", "1");
    }
    // Random bosses: every outdoor level asks for
    // champions / uniques (made up: 4..=6 per level).
    let f = s.tables.file("levels.txt");
    let den = f.columns.iter().position(|c| c == "MonDen").unwrap();
    let outdoor: Vec<usize> = (0..f.rows.len())
        .filter(|&i| !f.rows[i][den].is_empty())
        .collect();
    // The champion chance, monumod row 0 `constants` (20 in the game files).
    s.tables.set("monumod", 0, "constants", "20");
    for i in outdoor {
        s.tables.set("levels", i, "MonUMin", "4");
        s.tables.set("levels", i, "MonUMax", "6");
    }
    // The town NPC: act 1's `monpreset` rows are beast1, Warden 0, then
    // this one (DS1 id 2).
    s.tables
        .row("monpreset", &[("Act", "1"), ("Place", "keeper")]);
    let mut town = act1::town();
    let (_, _, x, y) = act1::TOWN_WAYPOINT;
    town.objects.push(Ds1Object {
        kind: 1,
        id: NPC_DS1_ID,
        x: x + 5,
        y: y + 5,
        flags: 0,
    });
    // A superunique (Bishibosh's kind: `monpreset` act 1 row "Warden 0",
    // DS1 id 1 → superuniques row 0).
    town.objects.push(Ds1Object {
        kind: 1,
        id: SUPERUNIQUE_DS1_ID,
        x: x + 9,
        y: y + 9,
        flags: 0,
    });
    let name = archive_name(act1::TOWN_DS1);
    let f = s
        .files
        .iter_mut()
        .find(|f| f.0 == name)
        .expect("the town file");
    f.1 = test_fixtures::ds1::write(&town);
    s
}

/// The NPC's DS1 id (its `monpreset` act 1 row).
const NPC_DS1_ID: u32 = 2;
/// The superunique's DS1 id (`monpreset` act 1 row "Warden 0").
const SUPERUNIQUE_DS1_ID: u32 = 1;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act1-stream-{}", std::process::id()));
        let i = install::build(&dir, &spawning_act1()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn setup() -> Setup {
    // `act1_game.rs`'s reference Act I creation seeds and fixture choices.
    Setup {
        creation: ActCreation::Full,
        init_seed: 644_409_375,
        town: TOWN,
        game_seed: 1234,
        class: 3,
        known_waypoints: Vec::new(),
    }
}

/// The session walked out of the town until its player stands in a
/// Blood Moor room.
fn walked() -> Session {
    let mut fx = Session::new(data(), &setup());
    let town = fx.level_rect(TOWN);
    let moor = fx.level_rect(BLOOD_MOOR);
    let p = fx.pos();
    let goals = border_goals(town, moor, p);
    fx.walk(
        &goals,
        |f| f.unit_level(f.player) == Some(BLOOD_MOOR),
        "Blood Moor",
    );
    fx.assert_clean("Blood Moor");
    fx
}

#[test]
fn the_walk_into_the_blood_moor_streams_its_rooms_and_monsters() {
    let mut fx = walked();
    let ids: Vec<u8> = fx.transcript.iter().map(|m| m[0]).collect();
    assert!(ids.contains(&0x07), "room adds: {ids:02X?}");
    assert!(ids.contains(&0x08), "room removes: {ids:02X?}");

    // Every monster the server holds was spawned by the Blood Moor's
    // population (the town has none); each one the client was told of
    // came as an S→C 0xAC with the monster's GUID (u32 at 1).
    let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
    assert!(!mons.is_empty(), "the Blood Moor populated");
    let guids: BTreeSet<u32> = mons
        .iter()
        .map(|&u| fx.sim().game.lists.unit(u).unwrap().guid)
        .collect();
    let added: BTreeSet<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(!added.is_empty(), "monsters reached the client: {ids:02X?}");
    assert!(
        added.is_subset(&guids),
        "0xAC for unknown monsters: {added:?} vs {guids:?}"
    );
}

#[test]
fn the_town_npc_reaches_the_client() {
    let mut fx = Session::new(data(), &setup());
    for _ in 0..10 {
        fx.frame();
    }
    fx.assert_clean("town");
    // The class is the `monstats` row (the compiled `.bin` keeps the
    // `.txt` order).
    let set = spawning_act1();
    let f = set.tables.file("monstats.txt");
    let id = f.columns.iter().position(|c| c == "Id").unwrap();
    let keeper = f
        .rows
        .iter()
        .position(|r| r[id] == "keeper")
        .expect("keeper row") as u32;
    let sim = &fx.host.game;
    let npcs: Vec<u32> = sim
        .game
        .lists
        .units_of_type(UnitType::Monster)
        .into_iter()
        .filter(|&u| sim.events.action.sys.units.get(u).unwrap().class == keeper)
        .map(|u| sim.game.lists.unit(u).unwrap().guid)
        .collect();
    assert_eq!(npcs.len(), 1, "the preset NPC spawned in the town");
    let added: Vec<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(added.contains(&npcs[0]), "0xAC of the NPC: {added:?}");
}

/// Determinism: two walks give the same transcript.
#[test]
fn the_stream_is_deterministic() {
    assert_eq!(walked().transcript, walked().transcript);
}

const STONY_FIELD: u32 = 4;

/// The monster type flags (`type_flag`).
const SUPERUNIQUE: u16 = 0x02;
const CHAMPION: u16 = 0x04;
const UNIQUE: u16 = 0x08;
const MINION: u16 = 0x10;

/// The session walked on, level by level along the A1W chain, until its
/// player stands in `to`.
fn walked_on(fx: &mut Session, from: u32, to: u32) {
    let a = fx.level_rect(from);
    let b = fx.level_rect(to);
    let p = fx.pos();
    let goals = border_goals(a, b, p);
    fx.walk(&goals, |f| f.unit_level(f.player) == Some(to), "next level");
    fx.assert_clean("level walk");
}

/// Monster counts per level id and the type flags seen.
fn census(fx: &mut Session) -> (BTreeMap<u32, usize>, u16, Vec<u32>) {
    let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
    let mut per = BTreeMap::new();
    let mut flags = 0u16;
    let mut bosses = Vec::new();
    for u in mons {
        let f = fx
            .sim()
            .events
            .world
            .monsters
            .get(u)
            .map_or(0, |m| m.type_flags);
        flags |= f;
        if f & (UNIQUE | CHAMPION) != 0 {
            bosses.push(fx.sim().game.lists.unit(u).unwrap().guid);
        }
        if let Some(l) = fx.unit_level(u) {
            *per.entry(l).or_insert(0) += 1;
        }
    }
    (per, flags, bosses)
}

/// Monsters spawn in every Act I outdoor level of the A1W chain as the
/// player walks into it (room activation), the
/// random bosses among them: uniques with their minions and champions,
/// all announced to the client (S→C 0xAC with the type flags).
#[test]
fn every_outdoor_level_spawns_monsters_with_champions_and_uniques() {
    let mut fx = walked();
    let mut per_level = Vec::new();
    let mut flags = 0u16;
    let mut bosses = Vec::new();
    for (from, to) in [(BLOOD_MOOR, COLD_PLAINS), (COLD_PLAINS, STONY_FIELD)] {
        let (n, f, b) = census(&mut fx);
        per_level.push(n);
        flags |= f;
        bosses.extend(b);
        walked_on(&mut fx, from, to);
    }
    let (n, f, b) = census(&mut fx);
    per_level.push(n);
    flags |= f;
    bosses.extend(b);
    for (lvl, n) in [BLOOD_MOOR, COLD_PLAINS, STONY_FIELD]
        .into_iter()
        .zip(&per_level)
    {
        assert!(n.get(&lvl).copied().unwrap_or(0) > 0, "level {lvl}: {n:?}");
    }
    for (bit, what) in [
        (UNIQUE, "unique"),
        (CHAMPION, "champion"),
        (MINION, "minion"),
    ] {
        assert!(flags & bit != 0, "no {what} spawned: flags {flags:#x}");
    }
    // The client was told of a boss (S→C 0xAC, GUID at 1).
    let added: BTreeSet<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(bosses.iter().any(|g| added.contains(g)), "{bosses:?}");
}

/// A DS1 preset superunique (Bishibosh, Corpsefire's class of spawn):
/// the room pass creates it with the superunique type flag and its
/// group, and the client is told of it.
#[test]
fn a_preset_superunique_spawns_with_its_group_and_reaches_the_client() {
    let mut fx = Session::new(data(), &setup());
    for _ in 0..10 {
        fx.frame();
    }
    fx.assert_clean("town");
    let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
    let flags_of = |fx: &mut Session, u| {
        fx.sim()
            .events
            .world
            .monsters
            .get(u)
            .map_or(0, |m| m.type_flags)
    };
    let su: Vec<_> = mons
        .iter()
        .copied()
        .filter(|&u| flags_of(&mut fx, u) & SUPERUNIQUE != 0)
        .collect();
    assert_eq!(su.len(), 1, "one superunique in the town");
    let minions = mons
        .iter()
        .filter(|&&u| flags_of(&mut fx, u) & MINION != 0)
        .count();
    assert!(minions >= 2, "its MinGrp..MaxGrp group: {minions}");
    let guid = fx.sim().game.lists.unit(su[0]).unwrap().guid;
    let added: Vec<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(added.contains(&guid), "0xAC of the superunique");
}

/// Steps a `goto` once per frame until it is not pending; after every
/// step the player's room is populated (its preset monsters placed).
fn goto_walk(fx: &mut Session, text: &str) -> (d2_sim::poke::PokeResult, BTreeSet<u32>) {
    use d2_sim::poke::{self, Directive, Env, GotoWalk, PokeResult};
    let Directive::Goto(t) = poke::parse_directive_text(text).unwrap() else {
        panic!("{text}: not a goto");
    };
    let mut w = GotoWalk::default();
    let mut rooms = BTreeSet::new();
    loop {
        let player = fx.player;
        let sim = fx.sim();
        let r = poke::goto_step(
            &mut sim.game,
            &mut sim.events,
            &Env::new(player),
            &t,
            &mut w,
        );
        let room = sim
            .game
            .lists
            .unit(player)
            .and_then(|e| e.room())
            .expect("the player's room");
        // The first step of a walk to another level only starts the warp
        // (the player still stands in the town room, which the game loads
        // its own way); every step after it walks into the level.
        if w.steps > 1 {
            assert!(
                sim.game.lists.room(room).is_some_and(|r| r.populated),
                "{text}: step {}: the player's room is not populated",
                w.steps
            );
        }
        rooms.insert(room.0);
        if r != PokeResult::Pending {
            return (r, rooms);
        }
        fx.frame();
    }
}

// A room's preset walk is taken once. `goto` used to take it for the room's
// objects and tiles only, so the preset monsters (superuniques, act bosses)
// of every room the walk passed through were never placed. Each room the
// walk reaches is now populated in the same step, as a portal arrival does.
#[test]
fn goto_populates_each_room_it_walks_into() {
    let mut fx = Session::new(data(), &setup());
    let (r, rooms) = goto_walk(&mut fx, &format!("goto preset {BLOOD_MOOR} 2:4000"));
    assert_eq!(
        r,
        d2_sim::poke::PokeResult::FailedWith(format!("{} 2", d2_sim::poke::GOTO_NONE_IN_LEVEL))
    );
    assert!(rooms.len() > 1, "the walk never left its first room");
    fx.assert_clean("goto walk");
}

// `goto superunique ROW` lands next to the monster spawned as that
// superuniques row, not next to its minions of the same class.
#[test]
fn goto_superunique_lands_next_to_that_row() {
    use d2_sim::poke::PokeResult;
    let mut fx = Session::new(data(), &setup());
    let (r, _) = goto_walk(&mut fx, "goto superunique 0");
    let PokeResult::Ok(Some(guid)) = r else {
        panic!("goto superunique 0: {r:?}");
    };
    let unit = fx
        .sim()
        .game
        .lists
        .find_unit(UnitType::Monster, guid)
        .expect("the superunique");
    let m = fx
        .sim()
        .events
        .world
        .monsters
        .get(unit)
        .expect("its monster data");
    assert!(m.type_flags & SUPERUNIQUE != 0, "flags {:#x}", m.type_flags);
    assert_eq!(m.boss_hc_idx, 0);
    let at = fx.sim().events.action.hooks().path_position(unit);
    let p = fx.pos();
    assert!(
        test_fixtures::host::cheb(p, at) <= 4,
        "player at {p:?}, superunique at {at:?}"
    );
    // A row nothing was spawned as: the walk explores, then fails (none in level N).
    let (r, _) = goto_walk(&mut fx, "goto superunique 5");
    assert!(
        matches!(&r, PokeResult::FailedWith(w) if w.starts_with(d2_sim::poke::GOTO_NONE_IN_LEVEL)),
        "{r:?}"
    );
}
