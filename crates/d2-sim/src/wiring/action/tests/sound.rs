//! Tests for the sound-event slot on the action wiring: the object seam queues
//! it, the per-client object and monster updates flush it as S→C 0x2C, and the
//! room clean-up clears it.

use std::sync::Arc;

use d2_data::tables::Objects;

use super::*;
use crate::tick::TickHooks;
use crate::units::lists::client_state::IN_GAME;
use crate::units::sound::play_sound_message;
use crate::units::ClientId;
use crate::world::objects::{ObjectTables, ObjectWorld};

/// One plain object row.
fn tables() -> Arc<ObjectTables> {
    Arc::new(ObjectTables {
        objects: vec![blank::<Objects>()],
        shrines: Vec::new(),
        levels: vec![blank(); 150],
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    })
}

fn guid(fx: &Fx, u: UnitId) -> u32 {
    fx.game.lists.unit(u).unwrap().guid
}

/// The fixture with an object and two players in room A, each player
/// with an in-game client in A; the object's unit flags cleared (no 0x0E).
fn setup() -> (Fx, UnitId, [(UnitId, ClientId); 2]) {
    let mut fx = Fx::new();
    fx.sim.create_objects(tables());
    let a = fx.a;
    let o = fx
        .sim
        .with(&mut fx.game, |g, v| v.create_object(g, a, 0, 20, 20, 0))
        .expect("allocated");
    fx.sim.sys.units.get_mut(o).unwrap().flags = 0;
    let mut players = [(o, ClientId(0)); 2];
    for (i, slot) in players.iter_mut().enumerate() {
        let p = fx.spawn(UnitType::Player, 0, a, 22 + i as i32, 20);
        let c = fx.game.lists.add_client(Some(p), Some(a), IN_GAME);
        *slot = (p, c);
    }
    (fx, o, players)
}

/// Runs the object seam's sound call.
fn sound(fx: &mut Fx, unit: UnitId, id: u8, to: Option<UnitId>, now: bool) {
    fx.sim
        .objects(&mut fx.game, |_, _, w| w.sound(unit, id, to, now))
        .expect("object state");
}

/// The seam stores event and target and queues the unit for
/// update; a second call overwrites both. The object's per-client update
/// sends one 8-byte 0x2C (type 2, GUID, the last event) to each client
/// while the target is none.
#[test]
fn object_sound_is_flushed_per_client() {
    let (mut fx, o, [(p, c), (q, d)]) = setup();
    let a = fx.a;
    let _ = fx.game.lists.unqueue_update(o);
    sound(&mut fx, o, 22, Some(p), false);
    sound(&mut fx, o, 13, None, false);
    assert!(fx.game.lists.update_queue(a).contains(&o));
    assert!(fx.sim.hooks().x.sent.is_empty());
    fx.sim.send_unit_update(&mut fx.game, c, o);
    fx.sim.send_unit_update(&mut fx.game, d, o);
    let want = play_sound_message(UnitType::Object as u8, guid(&fx, o), 13);
    assert_eq!(&want[..2], &[0x2C, 2]);
    assert_eq!(
        fx.sim.hooks().x.sent,
        [(p, want.to_vec()), (q, want.to_vec())]
    );
    fx.assert_clean();
}

/// A target that is not the client's player: that client gets
/// nothing.
#[test]
fn targeted_sound_only_to_the_target_client() {
    let (mut fx, o, [(p, c), (_, d)]) = setup();
    sound(&mut fx, o, 19, Some(p), false);
    fx.sim.send_unit_update(&mut fx.game, d, o);
    assert!(fx.sim.hooks().x.sent.is_empty());
    fx.sim.send_unit_update(&mut fx.game, c, o);
    let want = play_sound_message(2, guid(&fx, o), 19);
    assert_eq!(fx.sim.hooks().x.sent, [(p, want.to_vec())]);
}

/// The room clean-up clears flag 0x400 of
/// an object (no second send); a player's slot is left to the server's
/// player update pass.
#[test]
fn room_cleanup_clears_the_slot() {
    let (mut fx, o, [(p, c), _]) = setup();
    sound(&mut fx, o, 13, None, false);
    sound(&mut fx, p, 22, None, false);
    fx.sim.unit_update(&mut fx.game, o);
    fx.sim.unit_update(&mut fx.game, p);
    assert_eq!(fx.game.sounds.get(o), None);
    assert!(fx.game.sounds.get(p).is_some());
    fx.sim.send_unit_update(&mut fx.game, c, o);
    assert!(fx.sim.hooks().x.sent.is_empty());
}

/// The key sound 11 is sent at once to the
/// operator's client (called by the chest code).
#[test]
fn chest_key_sound_is_sent_at_once() {
    let (mut fx, _, [(p, _), _]) = setup();
    sound(&mut fx, p, 11, None, true);
    let want = play_sound_message(UnitType::Player as u8, guid(&fx, p), 11);
    assert_eq!(fx.sim.hooks().x.sent, [(p, want.to_vec())]);
    assert_eq!(fx.game.sounds.get(p).map(|s| s.event), Some(11));
}

/// The monster update sends the
/// 0x2C (type 1) to the client's player.
#[test]
fn monster_update_flushes_the_sound() {
    let (mut fx, _, [(p, c), _]) = setup();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 25, 25);
    let r = fx.sim.sys.units.get_mut(m).unwrap();
    r.flags = 0;
    r.flags2 = 0;
    crate::units::sound::queue_sound(&mut fx.game, m, 16, None).unwrap();
    fx.sim.send_unit_update(&mut fx.game, c, m);
    let want = play_sound_message(UnitType::Monster as u8, guid(&fx, m), 16);
    assert_eq!(fx.sim.hooks().x.sent, [(p, want.to_vec())]);
}

/// A monster world holding only monster data (what the 0x57 case reads).
pub(super) struct DataOnly(
    pub(super) std::collections::BTreeMap<UnitId, crate::monsters::init::MonsterData>,
);

impl<X> crate::wiring::action::monsters::MonsterWorld<X> for DataOnly {
    fn type_init(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
    ) {
    }
    fn umods(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
        _: Option<UnitId>,
        _: u8,
    ) {
    }
    fn assign_umod(
        &mut self,
        _: &mut crate::units::hooks::Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
        _: u8,
    ) {
    }
    fn forget(&mut self, _: UnitId) {}
    fn monster(&self, unit: UnitId) -> Option<&crate::monsters::init::MonsterData> {
        self.0.get(&unit)
    }
    fn monster_mut(&mut self, unit: UnitId) -> Option<&mut crate::monsters::init::MonsterData> {
        self.0.get_mut(&unit)
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// A monster with unit flag
/// 0x800 and the monster data's `data_flag1` gets S→C 0x57 (GUID,
/// type 1, name seed, umod bytes 0–2, type flag 4); without that flag,
/// or without unit flag 0x800, nothing.
#[test]
fn monster_update_sends_npc_enchants() {
    let (mut fx, _, [(p, c), _]) = setup();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 25, 25);
    let mut d = crate::monsters::init::MonsterData {
        name_seed: 0x1234,
        type_flags: 4 | 8,
        data_flag1: true,
        ..Default::default()
    };
    d.umods[..4].copy_from_slice(&[5, 17, 30, 9]);
    let world = DataOnly([(m, d.clone())].into_iter().collect());
    fx.sim.sys.hooks.monster_world = Some(Box::new(world));
    let set = |fx: &mut Fx, flags: u32| {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.flags = flags;
        r.flags2 = 0;
    };
    set(&mut fx, 0x800);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    let g = guid(&fx, m).to_le_bytes();
    let want = [
        0x57, g[0], g[1], g[2], g[3], 1, 0x34, 0x12, 5, 17, 30, 0, 1, 0,
    ];
    assert_eq!(
        crate::units::messages::npc_enchants(guid(&fx, m), 0x1234, [5, 17, 30], true),
        want
    );
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().x.sent),
        [(p, want.to_vec())]
    );
    // No unit flag 0x800: nothing.
    set(&mut fx, 0);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    assert_eq!(fx.sim.hooks().x.sent, []);
    // `data_flag1` clear: nothing.
    d.data_flag1 = false;
    fx.sim.sys.hooks.monster_world = Some(Box::new(DataOnly([(m, d)].into_iter().collect())));
    set(&mut fx, 0x800);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    assert_eq!(fx.sim.hooks().x.sent, []);
}

/// The 0x99 / 0x9A / 0xA5 / 0xAB records
/// carry the unit they sit on; 0xAB is not sent for a unit with flag 0x8000
/// or one the receiver may not attack; the
/// 0x99 record of a target absent from the receiver's rooms is sent as the
/// 17-byte 0x9A.
#[test]
fn pending_skill_event_records_0x99_0x9a_0xa5_0xab() {
    use crate::wiring::action::event_records::{
        landing, npc_heal, skill_event_on_point, skill_event_on_unit, EventRecord,
    };
    let (mut fx, _, [(p, c), _]) = setup();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 25, 25);
    let reset = |fx: &mut Fx, flags: u32| {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.flags = flags;
        r.flags2 = 0;
    };
    reset(&mut fx, 0);
    let g = guid(&fx, m);
    let recs = &mut fx.sim.sys.hooks.event_records;
    // The target (0, 0xDEAD) does not exist: the 16-byte 0x99 form.
    recs.push(
        m,
        EventRecord::CastOnUnit {
            skill: 54,
            level: 3,
            target: (0, 0xDEAD),
            w: 1,
        },
    );
    recs.push(
        m,
        EventRecord::CastOnPoint {
            skill: 54,
            level: 3,
            x: 30,
            y: 31,
            w: 0,
        },
    );
    recs.push(m, EventRecord::Landing { skill: 96 });
    recs.push(m, EventRecord::NpcHeal { life: 0x30 });
    fx.sim.send_unit_update(&mut fx.game, c, m);
    let want = [
        skill_event_on_unit(1, g, 54, 3, 0, 0xDEAD, 1).to_vec(),
        skill_event_on_point(1, g, 54, 3, 30, 31, 0).to_vec(),
        landing(1, g, 96).to_vec(),
        npc_heal(1, g, 0x30).to_vec(),
    ];
    assert_eq!(want[0][0], 0x99);
    assert_eq!(want[1][0], 0x9A);
    let got: Vec<_> = std::mem::take(&mut fx.sim.hooks().x.sent)
        .into_iter()
        .map(|(to, b)| {
            assert_eq!(to, p);
            b
        })
        .collect();
    assert_eq!(got, want);
    // Flag 0x8000 (soft hit): no 0xAB; the others are sent.
    reset(&mut fx, 0x8000);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    let got: Vec<_> = std::mem::take(&mut fx.sim.hooks().x.sent)
        .into_iter()
        .map(|(_, b)| b[0])
        .collect();
    assert!(!got.contains(&0xAB));
    assert!([0x99, 0x9A, 0xA5].iter().all(|id| got.contains(id)));
    // A unit the receiver may not attack (own hireling): no 0xAB.
    reset(&mut fx, 0);
    fx.sim.hooks().x.peaceful.push(m);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    let got: Vec<_> = std::mem::take(&mut fx.sim.hooks().x.sent)
        .into_iter()
        .map(|(_, b)| b[0])
        .collect();
    assert!(!got.contains(&0xAB));
    assert!([0x99, 0x9A, 0xA5].iter().all(|id| got.contains(id)));
}

/// A unit whose list has the
/// remove-overlay flag sends 0x11 with the overlay list's stat 178 when
/// it is in 0..=the overlay count (inclusive); otherwise nothing.
#[test]
fn monster_update_sends_the_overlay_0x11() {
    use crate::stats::lists::{flag, owner};
    let (mut fx, _, [(p, c), _]) = setup();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 25, 25);
    let count = 200;
    Arc::make_mut(&mut fx.sim.sys.hooks.tables).overlay_count = count;
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.flags = 0;
        r.flags2 = 0;
    }
    let g = guid(&fx, m);
    let want = |v: u16| {
        let b = g.to_le_bytes();
        [0x11, 1, b[0], b[1], b[2], b[3], v as u8, (v >> 8) as u8]
    };
    let o = fx.sim.sys.stats.alloc(flag::OVERLAY, 0, owner::MONSTER, g);
    let l = fx.sim.sys.stats.unit_list(m).expect("monster stat list");
    for (v, sent) in [(7, true), (count, true), (count + 1, false), (-1, false)] {
        fx.sim
            .sys
            .stats
            .set(&mut fx.sim.sys.hooks, o, 178, v, 0, None);
        fx.sim.sys.stats.attach(&mut fx.sim.sys.hooks, m, o, true);
        fx.sim.sys.stats.set_flags(l, flag::REMOVE_OVERLAY, true);
        fx.sim.send_unit_update(&mut fx.game, c, m);
        let got = std::mem::take(&mut fx.sim.hooks().x.sent);
        if sent {
            let w = want(v as u16);
            assert_eq!(crate::units::messages::report_kill(1, g, v as u16), w);
            assert_eq!(got, [(p, w.to_vec())], "overlay {v}");
        } else {
            assert_eq!(got, [], "overlay {v}");
        }
    }
    // No remove-overlay flag: nothing.
    fx.sim.sys.stats.set_flags(l, flag::REMOVE_OVERLAY, false);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    assert_eq!(fx.sim.hooks().x.sent, []);
}

/// The player's update sends
/// 0x11 with type 0 when its list has the remove-overlay flag and stat
/// 178 of the overlay list is in 0..=the overlay count (for example
/// `11 00 01000000 3600`).
#[test]
fn player_update_sends_the_overlay_0x11() {
    use crate::stats::lists::{flag, owner};
    let (mut fx, _, [(p, c), _]) = setup();
    Arc::make_mut(&mut fx.sim.sys.hooks.tables).overlay_count = 200;
    let g = guid(&fx, p);
    let b = g.to_le_bytes();
    let o = fx.sim.sys.stats.alloc(flag::OVERLAY, 0, owner::PLAYER, g);
    let l = fx.sim.sys.stats.unit_list(p).expect("player stat list");
    let reports = |fx: &mut Fx| -> Vec<Vec<u8>> {
        std::mem::take(&mut fx.sim.hooks().x.sent)
            .into_iter()
            .filter(|(_, m)| m[0] == 0x11)
            .map(|(_, m)| m)
            .collect()
    };
    fx.sim
        .sys
        .stats
        .set(&mut fx.sim.sys.hooks, o, 178, 0x36, 0, None);
    fx.sim.sys.stats.attach(&mut fx.sim.sys.hooks, p, o, true);
    fx.sim.sys.stats.set_flags(l, flag::REMOVE_OVERLAY, true);
    fx.sim.send_unit_update(&mut fx.game, c, p);
    assert_eq!(
        reports(&mut fx),
        [vec![0x11, 0, b[0], b[1], b[2], b[3], 0x36, 0]]
    );
    fx.sim.sys.stats.set_flags(l, flag::REMOVE_OVERLAY, false);
    fx.sim.send_unit_update(&mut fx.game, c, p);
    assert_eq!(reports(&mut fx), Vec::<Vec<u8>>::new());
}

/// A unit's pending event records go to
/// each client's player in list order in the per-client update (monster
/// update first; an object's update always, last) and the room clean-up
/// frees them.
#[test]
fn pending_event_records_are_sent_then_freed() {
    use crate::wiring::action::event_records::{preload, progressive, EventRecord};
    let (mut fx, o, [(p, c), _]) = setup();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let a = fx.a;
    let m = fx.spawn(UnitType::Monster, 0, a, 25, 25);
    let r = fx.sim.sys.units.get_mut(m).unwrap();
    r.flags = 0;
    r.flags2 = 0;
    let gm = guid(&fx, m);
    let a3 = EventRecord::Progressive {
        charges: 3,
        skill: 24,
        level: 5,
        unit: (1, gm),
        target: (0, 7),
        x: 0x1234,
        y: 0,
    };
    let recs = &mut fx.sim.sys.hooks.event_records;
    recs.push(m, a3);
    recs.push(m, EventRecord::Preload { class: 300 });
    recs.push(o, EventRecord::Preload { class: 9 });
    fx.sim.send_unit_update(&mut fx.game, c, m);
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().x.sent),
        [
            (
                p,
                progressive(3, 24, 5, (1, gm), (0, 7), 0x1234, 0).to_vec()
            ),
            (p, preload(300).to_vec()),
        ]
    );
    let r = fx.sim.sys.units.get_mut(o).unwrap();
    r.flags = 0;
    fx.sim.send_unit_update(&mut fx.game, c, o);
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().x.sent),
        [(p, preload(9).to_vec())]
    );
    fx.sim.unit_update(&mut fx.game, m);
    fx.sim.unit_update(&mut fx.game, o);
    assert_eq!(fx.sim.sys.hooks.event_records.of(m), []);
    fx.sim.send_unit_update(&mut fx.game, c, m);
    fx.sim.send_unit_update(&mut fx.game, c, o);
    assert_eq!(fx.sim.hooks().x.sent, []);
}
