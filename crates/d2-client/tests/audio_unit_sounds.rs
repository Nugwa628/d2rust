//! The unit sounds of the audio driver on the real install: the
//! `monsounds` rows, the animation of each mode and the sounds table of
//! the game drive hand-built client worlds; each test asserts the cue (id,
//! unit, tick, position) the driver requests. Run with `D2_GAME_DIR`.

use std::sync::{Arc, OnceLock};

use d2_client::app::sound::sound_table_live;
use d2_client::assets::game_files::GameFiles;
use d2_client::assets::FileSource;
use d2_client::audio::driver::{SoundDriver, SoundRequest};
use d2_client::audio::sound_table::{SoundSystem, SoundTableData};
use d2_client::audio::unit_feed::UnitSoundRows;
use d2_client::audio::{Sound, SoundBank, SoundId};
use d2_client::bridge::skills::{SkillEntry, SkillList};
use d2_client::bridge::world::{ClientUnit, ClientWorld, UnitKey, MISSILE, MONSTER, PLAYER};
use d2_client::world_view::unit_assets::UnitLooks;
use d2_data::bin::TableFiles;
use d2_formats::mpq::ArchiveSet;

struct Bank;

impl SoundBank for Bank {
    fn file(&self, id: SoundId) -> Option<Arc<str>> {
        Some(format!("s{}.wav", id.0).into())
    }
    fn samples(&self, _: SoundId) -> Option<Arc<Sound>> {
        Some(Arc::new(Sound::new(10_000, 1, vec![0; 10_000]).unwrap()))
    }
}

struct Real {
    files: Arc<GameFiles>,
    rows: Arc<UnitSoundRows>,
    classes: Vec<String>,
}

fn real() -> &'static Real {
    static REAL: OnceLock<Real> = OnceLock::new();
    REAL.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let files = Arc::new(GameFiles::archives(Arc::new(
            ArchiveSet::open_dir(dir).expect("archives open"),
        )));
        let looks = Arc::new(UnitLooks::live(files.as_ref()).expect("unit looks"));
        let anim = match files.read_native(
            &d2_client::assets::path::CanonicalPath::new(d2_formats::animdata::PATH).unwrap(),
        ) {
            Some(Ok(d2_native::source::NativeAsset::AnimData(a))) => a,
            other => panic!("AnimData.d2: {:?}", other.map(|r| r.is_ok())),
        };
        let rows = UnitSoundRows::live(files.as_ref(), looks, Arc::new(anim)).expect("rows");
        let (_, txt) = files
            .read_excel("monstats.txt")
            .unwrap()
            .expect("monstats.txt");
        let text = String::from_utf8_lossy(&txt).into_owned();
        let classes = text
            .lines()
            .skip(1)
            .map(|l| l.split('\t').next().unwrap_or("").to_string())
            .collect();
        Real {
            files,
            rows: Arc::new(rows),
            classes,
        }
    })
}

fn class(name: &str) -> u32 {
    real()
        .classes
        .iter()
        .position(|c| c == name)
        .unwrap_or_else(|| panic!("monster {name}")) as u32
}

fn table() -> SoundTableData {
    sound_table_live(real().files.as_ref()).expect("sounds.txt")
}

fn driver() -> SoundDriver {
    let mut d = SoundDriver::new(SoundSystem::new(table(), Box::new(Bank)));
    d.set_unit_rows(real().rows.clone());
    d
}

/// A cue as the request holds it: the group base of the requested id, the
/// units, the tick it starts at and its point relative to the listener.
#[derive(Debug, PartialEq)]
struct Cue {
    base: i32,
    units: Vec<UnitKey>,
    start_tick: u32,
    pos: [f32; 3],
}

fn cues(d: &SoundDriver, unit: UnitKey) -> Vec<Cue> {
    let t = d.system().table();
    d.system()
        .requests()
        .filter(|r| r.units.contains(&unit))
        .map(|r| Cue {
            base: t.base(r.id),
            units: r.units.clone(),
            start_tick: r.start_tick,
            pos: r.pos,
        })
        .collect()
}

/// The group base of `id` in the real sounds table.
fn base(d: &SoundDriver, id: i32) -> i32 {
    d.system().table().base(id)
}

fn cue(d: &SoundDriver, unit: UnitKey, id: i32, start_tick: u32) -> Cue {
    Cue {
        base: base(d, id),
        units: vec![unit],
        start_tick,
        pos: [0.0; 3],
    }
}

fn walker(w: &mut ClientWorld, key: UnitKey, class: u32, at: (u16, u16)) -> &mut ClientUnit {
    let mut u = ClientUnit::new(key);
    u.class = class;
    u.position = Some(at);
    u.mode = 1;
    w.units.insert(key, u);
    w.units.get_mut(&key).unwrap()
}

const P: UnitKey = UnitKey::new(PLAYER, 1);
const M: UnitKey = UnitKey::new(MONSTER, 2);

/// A world at server tick `tick`: the local player at (100, 100) and a
/// monster of `class` at (104, 102).
fn world(tick: u64, class: u32) -> ClientWorld {
    let mut w = ClientWorld {
        server_ticks: tick,
        ..ClientWorld::default()
    };
    walker(&mut w, P, 0, (100, 100));
    walker(&mut w, M, class, (104, 102));
    w.local_player = Some(P);
    w
}

/// The point of a unit 4 subtiles along x and 2 along y of the listener:
/// ((dx − dy) · 16, (dx + dy) · 16, 640).
const NEAR: [f32; 3] = [32.0, 96.0, 640.0];

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn event_16_requests_the_monsters_taunt() {
    let andariel = class("andariel");
    let mut d = driver();
    let w = world(1, andariel);
    let ev = SoundRequest::Server {
        unit: M,
        class: andariel,
        at: None,
        event: 16,
    };
    d.frame(&w, &[], &[ev]).unwrap();
    // `monsounds` andariel: `Taunt` 4,629 (read from the install).
    let taunt = real().rows.record(andariel, 0, 0).unwrap().taunt as i32;
    assert_eq!(taunt, 4629);
    let got = cues(&d, M);
    let mut want = cue(&d, M, 4629, 0);
    want.pos = NEAR;
    assert_eq!(got, [want]);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn event_17_requests_the_flee_voice_after_the_voice_gap() {
    let fallen = class("fallen1");
    let mut d = driver();
    let w = world(10, fallen);
    let ev = SoundRequest::Server {
        unit: M,
        class: fallen,
        at: None,
        event: 17,
    };
    d.frame(&w, &[], &[ev]).unwrap();
    // `Flee` 917; delay 6 + roll(3) (`start_tick` = T + delay).
    let got = cues(&d, M);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].base, base(&d, 917));
    assert!((6..=8).contains(&got[0].start_tick), "{got:?}");
    assert_eq!(got[0].pos, NEAR);
    // A second flee within 4 updates is silent.
    let before = d.system().requests().count();
    let w2 = world(11, fallen);
    d.frame(&w2, &[], &[ev]).unwrap();
    assert_eq!(d.system().requests().count(), before);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn event_12_requests_the_dodge_skill_sound() {
    let mut d = driver();
    let mut w = world(1, 0);
    {
        let u = w.units.get_mut(&P).unwrap();
        // State 68 (`evade`) with stat 350 = skill 13 (Dodge).
        u.state_lists.entry(68).or_default().insert((350, 0), 13);
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 13,
                ..SkillEntry::default()
            }],
            ..SkillList::default()
        });
    }
    let ev = SoundRequest::Server {
        unit: P,
        class: 0,
        at: None,
        event: 12,
    };
    d.frame(&w, &[], &[ev]).unwrap();
    // Live: Dodge's `stsound` is 2,236 `amazon_dodge_1`.
    let got = cues(&d, P);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].base, base(&d, 2236));
    // The listener's own point: x = y = 0, z = 640.
    assert_eq!((got[0].start_tick, got[0].pos), (0, [0.0, 0.0, 640.0]));
    // Without the skill: nothing.
    let mut d = driver();
    w.units.get_mut(&P).unwrap().skills = None;
    d.frame(&w, &[], &[ev]).unwrap();
    assert!(cues(&d, P).is_empty());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_zombie_swinging_requests_its_attack_voice() {
    let zombie = class("zombie1");
    let mut d = driver();
    let mut w = world(5, zombie);
    d.frame(&w, &[], &[]).unwrap();
    assert!(
        cues(&d, M).iter().all(|c| c.base != base(&d, 2203)),
        "standing: no attack voice"
    );
    // Mode NU → A1 (4): `Attack1` 2,203 at probability 100, delay 0.
    w.units.get_mut(&M).unwrap().mode = 4;
    w.server_ticks = 6;
    d.frame(&w, &[], &[]).unwrap();
    let a = cues(&d, M)
        .into_iter()
        .find(|c| c.base == base(&d, 2203))
        .expect("attack voice");
    assert_eq!((a.units, a.pos), (vec![M], NEAR));
    assert_eq!(a.start_tick, d.tick().saturating_sub(1));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_zombie_hit_and_dying_requests_its_voices() {
    let zombie = class("zombie1");
    let mut d = driver();
    let mut w = world(5, zombie);
    d.frame(&w, &[], &[]).unwrap();
    w.units.get_mut(&M).unwrap().mode = 3;
    w.server_ticks = 6;
    d.frame(&w, &[], &[]).unwrap();
    assert!(cues(&d, M).iter().any(|c| c.base == base(&d, 2209)), "hit");
    w.units.get_mut(&M).unwrap().mode = 0;
    w.server_ticks = 7;
    d.frame(&w, &[], &[]).unwrap();
    assert!(
        cues(&d, M).iter().any(|c| c.base == base(&d, 2215)),
        "death"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_local_player_hit_requests_its_class_voice() {
    let mut d = driver();
    let mut w = world(5, 0);
    d.frame(&w, &[], &[]).unwrap();
    // Player mode GH (4): the class `hit` line of the amazon, 2,878, d 2.
    w.units.get_mut(&P).unwrap().mode = 4;
    w.server_ticks = 6;
    d.frame(&w, &[], &[]).unwrap();
    let got = cues(&d, P);
    let hit = got
        .iter()
        .find(|c| c.base == base(&d, 2878))
        .expect("hit voice");
    assert_eq!(hit.start_tick, d.tick().saturating_sub(1) + 2);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_swarm_requests_its_init_voice() {
    let swarm = class("swarm1");
    let mut d = driver();
    let w = world(5, swarm);
    d.frame(&w, &[], &[]).unwrap();
    // `Init` 1,913 on the first sight.
    let got = cues(&d, M);
    assert!(got.iter().any(|c| c.base == base(&d, 1913)), "{got:?}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn an_idle_zombie_groans_after_its_neutral_time() {
    let zombie = class("zombie1");
    let mut d = driver();
    let mut first = None;
    for tick in 1..=2000u64 {
        d.frame(&world(tick, zombie), &[], &[]).unwrap();
        if cues(&d, M).iter().any(|c| c.base == base(&d, 2219)) {
            first = Some(tick);
            break;
        }
    }
    // `Neutral` 2,219, `NeuTime` 300 (C), idle gap 90 at game start, then
    // roll(15) = 0: not before C = 300, and within a few hundred updates.
    let t = first.expect("a groan within 2000 updates");
    assert!(t >= 300, "NeuTime: first groan at C = {t}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_walking_zombie_steps_in_time_with_its_walk_animation() {
    let zombie = class("zombie1");
    let mut d = driver();
    let mut w = world(1, zombie);
    d.frame(&w, &[], &[]).unwrap();
    w.units.get_mut(&M).unwrap().mode = 2;
    // F and s of the walk animation (AnimData): FsCnt 2 steps
    // per cycle, one every F / 2 / s updates.
    let (frames, speed) = real().rows.animation(&w.units[&M]).expect("WL animation");
    let rec = real().rows.record(zombie, 0, 0).unwrap();
    assert_eq!((rec.fscnt, rec.fsoff, rec.fsprb), (2, 0, 100));
    let (n, s) = (rec.fscnt, speed);
    let (f_all, step) = (frames as i32, (frames / n) as i32);
    // The footstep rule written out for f = j · s mod F (f = 0 at the
    // mode set): dist = distance to a multiple of
    // `step` (FsOff 0); fires on a strict local minimum; no step within
    // ⌊2 · period / 3⌋ of the last one (period = F / (n · s)).
    let period = frames / (n * s as u32);
    let dist = |x: i32| {
        let a = x % step;
        a.abs().min((a - step).abs()).min((a + step).abs())
    };
    let mut want = Vec::new();
    let (mut last, window) = (0u32, period * 2 / 3);
    for j in 1..=(8 * period + 8) {
        let f = ((j as i32) * s) % f_all;
        if dist(f) < dist(f + s) && dist(f) < dist(f - s) {
            // Client update C = tick = j + 1 (the mode set is at tick 2).
            let c = j + 1;
            if last == 0 || c - last >= window {
                want.push(u64::from(c));
                last = c;
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut steps = Vec::new();
    for tick in 2..=(8 * u64::from(period) + 9) {
        w.server_ticks = tick;
        d.frame(&w, &[], &[]).unwrap();
        // `Footstep` 2,720: the group of the dirt steps (k = 1).
        let b = base(&d, 2720);
        for r in d.system().requests() {
            if r.units.contains(&M) && d.system().table().base(r.id) == b && seen.insert(r.handle) {
                steps.push((tick, r.start_tick, r.pos));
            }
        }
    }
    let ticks: Vec<u64> = steps.iter().map(|s| s.0).collect();
    assert!(want.len() >= 7, "oracle {want:?}");
    assert_eq!(ticks, want);
    // Every step is at the monster's point and starts the tick requested.
    assert!(steps
        .iter()
        .all(|s| s.2 == NEAR && u64::from(s.1) + 1 == s.0));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_state_turning_on_requests_its_on_sound() {
    let zombie = class("zombie1");
    let mut d = driver();
    let mut w = world(5, zombie);
    d.frame(&w, &[], &[]).unwrap();
    // State 1 `freeze`: `onsound` 365 (read from the install).
    assert_eq!(real().rows.state_sounds(1), Some((365, 0)));
    w.units.get_mut(&M).unwrap().states.insert(1);
    w.server_ticks = 6;
    d.frame(&w, &[], &[]).unwrap();
    let got = cues(&d, M);
    assert!(got.iter().any(|c| c.base == base(&d, 365)), "{got:?}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_missile_requests_its_travel_sound() {
    let mut d = driver();
    let mut w = world(5, 0);
    let m = UnitKey::new(MISSILE, 9);
    // Missiles row 12: `TravelSound` 2,242.
    assert_eq!(real().rows.missile_travel(12), Some(2242));
    walker(&mut w, m, 12, (104, 102)).mode = 0;
    d.frame(&w, &[], &[]).unwrap();
    let got = cues(&d, m);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].base, base(&d, 2242));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_dialog_line_plays_its_speech_on_the_player() {
    let akara = class("akara");
    let mut d = driver();
    let w = world(7, akara);
    // Dialog text key 506 is listed twice in `npc-speech.tsv`; the first
    // row (3,533) wins. Delay 5.
    let line = SoundRequest::NpcDialogLine {
        npc: M,
        class: akara,
        key: 506,
    };
    d.frame(&w, &[], &[line]).unwrap();
    let got = cues(&d, P);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].base, base(&d, 3533));
    assert_eq!(got[0].pos, [0.0, 0.0, 640.0]);
    assert_eq!(got[0].start_tick, 5, "T 0 + delay 5");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn interacting_with_akara_plays_her_greeting_on_the_player() {
    let akara = class("akara");
    assert_eq!(akara, 148);
    let mut d = driver();
    let w = world(7, akara);
    d.frame(
        &w,
        &[],
        &[SoundRequest::NpcGreeting {
            npc: M,
            class: akara,
        }],
    )
    .unwrap();
    // Akara: `akara_greeting_1`, `_inactive_1`, `_time_1`: mode 0 picks
    // the greeting, the inactive line or the time line.
    let rec = d2_client::audio::triggers::npc::GreetingRecords::spec()
        .for_class(148)
        .map(|g| (g.greet, g.inactive, g.time))
        .unwrap();
    let got = cues(&d, P);
    assert_eq!(got.len(), 1, "{got:?}");
    let ok = [rec.0, rec.1, rec.2, rec.2 + 1, rec.2 + 2]
        .iter()
        .any(|&id| base(&d, id) == got[0].base);
    assert!(ok, "{got:?} vs {rec:?}");
    assert_eq!(got[0].start_tick, 0);
}
