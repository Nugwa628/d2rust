//! The player handlers behind the dispatcher on the action wiring's provider
//! ([`super::super::action::ActionPlayer`]): the unit records, stat
//! lists, update queue and timers change; the `Pending` seams are called;
//! their messages reach the client; and without a provider the ids stay
//! stubs.

use std::sync::Arc;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::StateMaps;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Itemstatcost, Monstats, Record, Skills, States};
use d2_sim::combat::CombatTables;
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::{SkillEntry, SkillTables};
use d2_sim::stats::{ClassStats, StatData, StatTable, StateTable};
use d2_sim::tick::events::event;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, Pending};

use super::super::{HotKey, FLAG_EX_RESEND, FLAG_OVERHEAD, HOTKEY_SLOTS};
use crate::adapters::handlers::world::tests::waypoints::{field_drlg, field_room};
use crate::adapters::handlers::world::tests::{host, send, TestHost};
use crate::adapters::handlers::world::{ActionWorld, Outbox};
use crate::adapters::{PlayerData, PlayerFields, SimGame, UnitFacts};
use crate::buffers::ClientBuffers;
use crate::seams::{Intents, PlayerGate, Pos, ResultCode};

/// The `Pending` seams of the player handlers, recorded; the stat messages sent as S→C
/// 0x1D (stat u8, value u8) so their relay is visible.
#[derive(Default)]
struct Rec {
    log: Vec<String>,
    sent: Vec<(UnitId, Vec<u8>)>,
    busy: bool,
    skills: Vec<SkillEntry>,
}

impl Pending for Rec {
    fn play_sound(&mut self, _: &mut Game, unit: UnitId, sound: u32, to: Option<UnitId>) {
        self.log
            .push(format!("sound {} {sound} {:?}", unit.0, to.map(|t| t.0)));
    }
    fn replace_overhead(&mut self, player: UnitId, text: &[u8], byte8: u8, end: i32) {
        self.log.push(format!(
            "overhead {} {} {byte8} @{end}",
            player.0,
            String::from_utf8_lossy(text)
        ));
    }
    fn object_player_busy(&self, _: UnitId) -> bool {
        self.busy
    }
    fn clear_player_busy(&mut self, player: UnitId) {
        self.busy = false;
        self.log.push(format!("unbusy {}", player.0));
    }
    fn skill_list(&self, _: UnitId) -> Vec<SkillEntry> {
        self.skills.clone()
    }
    fn stat_sent(&mut self, player: UnitId, stat: u16, value: u32) {
        self.sent
            .push((player, vec![0x1D, stat as u8, value as u8]));
    }
    fn warp(&mut self, _: &mut Game, player: UnitId, level: u32, tile_code: u8) {
        self.log
            .push(format!("warp {} {level} {tile_code}", player.0));
    }
    fn reselect_hand_skills(&mut self, _: &mut Game, player: UnitId) {
        self.log.push(format!("reselect {}", player.0));
    }
}

impl Outbox for Rec {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

type Wired = SimGame<ActionSim<Rec>, ActionWorld>;

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// 359 stats (no ops) and 200 states (no flags).
fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: n,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    let states = BinTable {
        name: "states".into(),
        source: "synthetic".into(),
        count: 200,
        record_size: States::SIZE,
        records: vec![0u8; 200 * States::SIZE],
    };
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        classes: vec![ClassStats::default(); 7],
        states: StateTable::new(
            &states,
            &StateMaps {
                words: 7,
                bitsets: vec![0; 40 * 7],
                ..StateMaps::default()
            },
        )
        .expect("states"),
        damage_regen: vec![0; 8],
        aurastate: vec![0; 8],
        rescale_precision: d2_sim::stats::DEFAULT_RESCALE_PRECISION,
    })
}

struct Fx {
    host: TestHost<Wired>,
    player: UnitId,
    monster: UnitId,
}

/// A player (mode `mode`) and a monster in one field room, on the action
/// wiring with [`Rec`]; 6 skills, 3 monstats rows.
fn fx(mode: u32) -> Fx {
    let tables = ActionTables {
        missiles: Vec::new(),
        skills: SkillTables {
            skills: (0..6).map(|_| blank::<Skills>()).collect(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 0,
            stat_count: 0,
        },
        combat: CombatTables {
            charstats: Vec::new(),
            difficultylevels: Vec::new(),
            monstats: (0..3).map(|_| blank::<Monstats>()).collect(),
            monstats2: Vec::new(),
            hitclass: Vec::new(),
        },
        levels: Vec::new(),
        skill_modes: Vec::new(),
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    };
    let hooks = ActionHooks::new(
        Arc::new(tables),
        field_drlg(),
        Seed::init_low(77),
        Rec::default(),
    );
    let data = UnitData {
        monsters: vec![MonsterInfo {
            mode_chart: false,
            enabled: true,
            aidel: [15; 3],
            moves: 0,
        }],
        ..UnitData::default()
    };
    let mut events = ActionSim::new(stat_data(), data, hooks);
    let mut game = Game::new();
    let room = field_room(&mut events, &mut game);
    let mut alloc = |ty| {
        let req = AllocRequest {
            ty,
            class: 0,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        events
            .with(&mut game, |g, v| v.allocate(g, &req, 20, 20))
            .expect("allocated")
    };
    let player = alloc(UnitType::Player);
    let monster = alloc(UnitType::Monster);
    events.sys.units.get_mut(player).unwrap().mode = mode;
    let mut sim = SimGame::with_world(game, events, ActionWorld::default());
    sim.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    for u in [player, monster] {
        sim.set_unit(
            u,
            UnitFacts {
                act: 0,
                pos: Pos { x: 100, y: 100 },
                owner: None,
            },
        );
    }
    Fx {
        host: host(sim),
        player,
        monster,
    }
}

impl Fx {
    /// [`send`] past the client's duplicate window.
    fn send(&mut self, m: &[u8]) -> (ResultCode, Vec<Vec<u8>>) {
        self.host.clock.0 += 200;
        send(&mut self.host, m)
    }
    /// The handler alone (`Intents::handle`, after the gate and size
    /// checks), without the frame's tick: the state the handler leaves
    /// before the end-of-tick room clean-up clears the update flags.
    fn handle_only(&mut self, m: &[u8]) -> ResultCode {
        let mut out = ClientBuffers::new();
        Intents::handle(&mut self.host.game, 0, m, m.len(), &mut out)
    }
    fn sim(&mut self) -> &mut Wired {
        &mut self.host.game
    }
    fn rec(&mut self) -> &mut Rec {
        &mut self.sim().events.sys.hooks.x
    }
    fn log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rec().log)
    }
    fn has_state(&mut self, s: u32) -> bool {
        let p = self.player;
        self.sim().events.sys.stats.has_state(p, s)
    }
    fn record(&mut self, u: UnitId) -> d2_sim::units::record::UnitRecord {
        self.sim().events.sys.units.get(u).unwrap().clone()
    }
    fn record_mode(&mut self, u: UnitId, mode: u32) {
        self.sim().events.sys.units.get_mut(u).unwrap().mode = mode;
    }
    fn guid(&mut self, u: UnitId) -> u32 {
        self.sim().game.lists.unit(u).unwrap().guid
    }
    fn set_stat(&mut self, s: u16, v: i32) {
        let p = self.player;
        let g = self.sim();
        g.events.with(&mut g.game, |_, view| view.set_base(p, s, v));
    }
    fn stat(&mut self, s: u16) -> i32 {
        let p = self.player;
        let g = self.sim();
        g.events.with(&mut g.game, |_, view| view.stat(p, s))
    }
}

use ResultCode::*;

#[test]
fn end_inferno_on_the_stat_lists() {
    let mut f = fx(1);
    let p = f.player;
    {
        let g = f.sim();
        g.events.with(&mut g.game, |_, v| v.set_state(p, 12, true));
    }
    assert!(f.has_state(12));
    assert_eq!(f.send(&[0x12]), (Done, vec![]));
    assert!(!f.has_state(12));
    assert!(f.sim().unhandled.is_empty());
}

#[test]
fn stamina_switches_the_unit_mode() {
    let mut f = fx(2);
    let p = f.player;
    // No stamina: walk stays.
    assert_eq!(f.send(&[0x53]).0, Done);
    assert_eq!(f.record(p).mode, 2);
    f.set_stat(10, 5 << 8);
    // The handler's mode set flags the unit; the end-of-tick clean-up
    // clears it again.
    assert_eq!(f.handle_only(&[0x53]), Done);
    let r = f.record(p);
    assert_eq!(r.mode, 3);
    assert_ne!(r.flags & 1, 0, "flags |= 1 (units.md §4.1)");
    f.record_mode(p, 2);
    assert_eq!(f.send(&[0x53]).0, Done);
    assert_eq!(f.record(p).mode, 3);
    assert_eq!(f.send(&[0x54]).0, Done);
    assert_eq!(f.record(p).mode, 2);
}

#[test]
fn overhead_chat_timeout_flags_and_event_6() {
    let mut f = fx(1);
    let p = f.player;
    let frame = f.sim().game.frame;
    // The handler alone: the flag is set before the end-of-tick clean-up
    // clears it.
    let code = f.handle_only(&[0x14, 1, 4, b'y', b'o', 0, 0, 0]);
    assert_eq!(code, Done);
    // d = 8·2 + 0x7D.
    let end = frame + 16 + 0x7D;
    assert_eq!(f.log(), [format!("overhead {} yo 4 @{end}", p.0)]);
    let r = f.record(p);
    assert_eq!(r.hover, Some(end));
    assert_ne!(r.flags & FLAG_OVERHEAD, 0);
    let t = &f.sim().game.timers;
    let ev: Vec<(u8, i32)> = t
        .unit_timers(p)
        .into_iter()
        .filter_map(|id| Some((t.event(id)?.0, t.expire(id)?)))
        .filter(|e| e.0 == event::FREE_HOVER)
        .collect();
    assert_eq!(ev, [(event::FREE_HOVER, end)]);
}

#[test]
fn play_audio_goes_to_the_sound_seam() {
    let mut f = fx(1);
    let p = f.player;
    assert_eq!(f.send(&[0x3F, 26, 0]).0, Done);
    assert_eq!(f.log(), [format!("sound {} 26 None", p.0)]);
    f.sim().events.sys.units.get_mut(p).unwrap().flags |= 0x400;
    assert_eq!(f.send(&[0x3F, 26, 0]).0, Refused);
    assert!(f.log().is_empty());
}

#[test]
fn request_entity_update_sets_flag_ex() {
    let mut f = fx(1);
    let (p, m) = (f.player, f.monster);
    let g = f.guid(m);
    let mut msg = vec![0x4B];
    msg.extend(1u32.to_le_bytes());
    msg.extend(g.to_le_bytes());
    // The handler alone: flag-ex 0x10000 set before the end-of-tick
    // clean-up clears it with the update.
    assert_eq!(f.handle_only(&msg), Done);
    assert_ne!(f.record(m).flags2 & FLAG_EX_RESEND, 0);
    // The player itself (type 0).
    let pg = f.guid(p);
    let mut msg = vec![0x4B];
    msg.extend(0u32.to_le_bytes());
    msg.extend(pg.to_le_bytes());
    assert_eq!(f.handle_only(&msg), Done);
    assert_ne!(f.record(p).flags2 & FLAG_EX_RESEND, 0);
    // A missing unit → 1.
    let mut msg = vec![0x4B];
    msg.extend(1u32.to_le_bytes());
    msg.extend((g + 100).to_le_bytes());
    assert_eq!(f.send(&msg).0, Refused);
}

#[test]
fn bind_hotkey_fills_the_client_slot() {
    let mut f = fx(1);
    f.rec().skills = vec![SkillEntry {
        skill: 3,
        owner_guid: -1,
        ..SkillEntry::default()
    }];
    let msg = |w: u32, item: u32| {
        let mut m = vec![0x51];
        m.extend(w.to_le_bytes());
        m.extend(item.to_le_bytes());
        m
    };
    // Skill 3, left, slot 5, item −1.
    assert_eq!(f.send(&msg(3 | 0x8000 | (5 << 16), u32::MAX)).0, Done);
    // Skill 4: the player lacks it → 3; slot 16 → 3.
    assert_eq!(f.send(&msg(4 | (6 << 16), u32::MAX)).0, Malformed);
    assert_eq!(f.send(&msg(3 | (16 << 16), u32::MAX)).0, Malformed);
    // Skill 100 > 6 skills: unbind slot 7.
    assert_eq!(f.send(&msg(100 | (7 << 16), 9)).0, Done);
    let keys = f.sim().hotkeys(0);
    let mut want = [HotKey::UNBOUND; HOTKEY_SLOTS];
    want[5] = HotKey {
        skill: 3,
        left: true,
        item: u32::MAX,
    };
    want[7] = HotKey {
        skill: -1,
        left: false,
        item: 9,
    };
    assert_eq!(keys, want);
    assert!(f.sim().unhandled.is_empty());
}

#[test]
fn turn_off_busy_through_pending() {
    let mut f = fx(1);
    assert_eq!(f.send(&[0x48]).0, Invalid);
    f.rec().busy = true;
    assert_eq!(f.send(&[0x48]).0, Done);
    let p = f.player;
    assert_eq!(f.log(), [format!("unbusy {}", p.0)]);
}

#[test]
fn resurrect_on_the_wiring() {
    let mut f = fx(0x11);
    let p = f.player;
    // Maxima: stats 7, 9, 11.
    f.set_stat(7, 40);
    f.set_stat(9, 30);
    f.set_stat(11, 20);
    {
        let g = f.sim();
        g.events
            .with(&mut g.game, |_, v| v.set_state(p, 0x36, false));
    }
    let (code, got) = f.send(&[0x41]);
    assert_eq!(code, Done);
    // The three stat messages, in order, to the client.
    assert_eq!(
        got,
        [vec![0x1D, 6, 40], vec![0x1D, 8, 30], vec![0x1D, 10, 20]]
    );
    assert_eq!((f.stat(6), f.stat(8), f.stat(10)), (40, 30, 20));
    assert_ne!(f.record(p).flags & 2, 0);
    assert!(!f.has_state(0x36), "on then off");
    // Cold Plains is act I: the town is level 1; no path provider, so
    // the warp goes to `Pending::warp`.
    assert_eq!(
        f.log(),
        [format!("warp {} 1 0", p.0), format!("reselect {}", p.0)]
    );
}

/// Without a world provider (`NoWorld`) every player-handler id stays a stub.
#[test]
fn no_world_keeps_the_stubs() {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    game.lists.activate_room(room).unwrap();
    let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let mut s = SimGame::new(game);
    s.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    s.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    let mut h = host(s);
    for m in [&[0x12][..], &[0x53], &[0x48]] {
        assert_eq!(send(&mut h, m), (Done, vec![]));
    }
    assert_eq!(
        h.game.unhandled,
        vec![(0, 0x12, 1), (0, 0x53, 1), (0, 0x48, 1)]
    );
}
