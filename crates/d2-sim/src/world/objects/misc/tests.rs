// Spec: specs/world/objects.md §10–§13 (Test vectors, Edge cases)
use super::super::fake::{blank_object, Call, Fake};
use super::super::{dispatch, oevent, oflags, Dispatch, ObjectData};
use super::*;
use crate::rng::Seed;
use d2_data::tables::Record;

// Test conventions for the stat map of [`Fake`] (it has no fields for the
// misc seams): keys at or above 0xF000 are not stats.
/// OR of the collision bits of the object's footprint cells.
const CELLS: u16 = 0xFFFF;
/// `iterate_players` result.
const ITERATE: u16 = 0xFFFE;
/// `hostile_time` value.
const HOSTILE: u16 = 0xFFFD;
/// Nonzero: `cure_states` removes something.
const CURABLE: u16 = 0xFFFC;
/// Nonzero: a pet is healed.
const PETS: u16 = 0xFFFB;
/// Nonzero on unit 0: `level_spawn_point` finds (room 9, 50, 60).
const SPAWN: u16 = 0xFFFA;
/// Nonzero: the player has a quest record.
const QREC: u16 = 0xFFF9;
/// The unit's party id (absent: 0xFFFF).
const PARTY: u16 = 0xFFF8;
/// State `s` is present when key 0xF000 + s is nonzero.
const STATE: u16 = 0xF000;
/// The partner portal's unit id of an object.
const PARTNER: u16 = 0xFFF7;
/// `player_portal_guid` of a player.
const PUID: u16 = 0xFFF6;
/// Nonzero on unit 0: an expansion game.
const EXPANSION: u16 = 0xFFF5;
/// Quest bit (q, 0) of a player is set when key 0xE000 + q is nonzero.
const QBIT: u16 = 0xE000;

impl MiscWorld for Fake {
    fn create_level_portal(&mut self, object: UnitId, row: u16) {
        self.calls
            .push(Call::Other(format!("level_portal {object:?} {row}")));
    }
    fn footprint_collides(&self, object: UnitId, mask: u16) -> bool {
        let cells = self.stats.get(&(object, CELLS)).copied().unwrap_or(0) as u16;
        cells & mask != 0
    }
    fn vital_stat(&self, unit: UnitId, id: u16) -> u32 {
        self.stats.get(&(unit, id)).copied().unwrap_or(0) as u32
    }
    fn set_vital_stat(&mut self, unit: UnitId, id: u16, value: u32) {
        self.stats.insert((unit, id), value as i32);
        self.calls.push(Call::Other(format!("stat {id} {value}")));
    }
    fn remove_state_list(&mut self, unit: UnitId, state: u16) -> bool {
        self.calls
            .push(Call::Other(format!("remove state {state}")));
        self.stats.remove(&(unit, STATE + state)).unwrap_or(0) != 0
    }
    fn cure_states(&mut self, unit: UnitId) -> bool {
        self.calls.push(Call::Other("cure".into()));
        self.stats.remove(&(unit, CURABLE)).unwrap_or(0) != 0
    }
    fn well_heal_pets(&mut self, player: UnitId, _object: UnitId) -> bool {
        self.calls.push(Call::Other("pets".into()));
        self.stats.get(&(player, PETS)).copied().unwrap_or(0) != 0
    }
    fn iterate_players(&self, player: UnitId) -> u32 {
        self.stats.get(&(player, ITERATE)).copied().unwrap_or(0) as u32
    }
    fn hostile_time(&self, player: UnitId) -> u32 {
        self.stats.get(&(player, HOSTILE)).copied().unwrap_or(0) as u32
    }
    fn party_id(&self, unit: UnitId) -> u16 {
        self.stats.get(&(unit, PARTY)).map_or(0xFFFF, |&v| v as u16)
    }
    fn has_quest_record(&self, player: UnitId) -> bool {
        self.stats.get(&(player, QREC)).is_some_and(|&v| v != 0)
    }
    fn level_spawn_point(&mut self, level: u32) -> Option<(RoomId, i32, i32)> {
        self.calls.push(Call::Other(format!("spawn {level}")));
        self.stats
            .get(&(UnitId(0), SPAWN))
            .is_some_and(|&v| v != 0)
            .then_some((RoomId(9), 50, 60))
    }
    fn player_mode_xy(&mut self, player: UnitId, mode: u8, x: i32, y: i32) {
        self.calls
            .push(Call::Other(format!("walk {} {mode} {x} {y}", player.0)));
    }
    fn portal_partner(&mut self, object: UnitId) -> Option<UnitId> {
        self.stats
            .get(&(object, PARTNER))
            .map(|&v| UnitId(v as u32))
    }
    fn player_portal_guid(&self, player: UnitId) -> u32 {
        self.stats.get(&(player, PUID)).copied().unwrap_or(0) as u32
    }
    fn expansion(&self) -> bool {
        self.stats
            .get(&(UnitId(0), EXPANSION))
            .is_some_and(|&v| v != 0)
    }
    fn player_quest_bit(&self, player: UnitId, quest: u32, bit: u8) -> bool {
        bit == 0
            && self
                .stats
                .get(&(player, QBIT + quest as u16))
                .is_some_and(|&v| v != 0)
    }
    fn quest_level_change(&mut self, player: UnitId, from: u32, to: u32) {
        self.calls
            .push(Call::Other(format!("qlc {} {from} {to}", player.0)));
    }
    fn remove_portal(&mut self, object: UnitId) {
        self.calls.push(Call::Other(format!("remove {}", object.0)));
    }
    fn portal_act5_hook(&mut self, partner: UnitId) {
        self.calls.push(Call::Other(format!("act5 {}", partner.0)));
    }
    fn just_portaled(&mut self, player: UnitId, expire: i32) {
        self.calls
            .push(Call::Other(format!("portaled {} {expire}", player.0)));
    }
}

const O: UnitId = UnitId(10);
const P: UnitId = UnitId(20);
const M: UnitId = UnitId(30);

const DOOR: u16 = 1;
const WELL: u16 = 2;
const PORTAL: u16 = 3;
const TORCH: u16 = 4;

fn tables() -> ObjectTables {
    let mut objects: Vec<_> = (0..64).map(|_| blank_object()).collect();
    for (c, f) in [(DOOR, 8), (WELL, 22), (PORTAL, 15), (TORCH, 11)] {
        objects[c as usize].operatefn = f;
        // Sync 1: mode changes draw nothing (§4).
        objects[c as usize].sync = 1;
    }
    // Live well values (§11): Parm0 750, Parm1 128, Parm2 1, Parm3 3.
    let w = &mut objects[WELL as usize];
    (w.parm0, w.parm1, w.parm2, w.parm3) = (750, 128, 1, 3);
    ObjectTables {
        objects,
        ..ObjectTables::default()
    }
}

/// Object `O` of `class` in `mode`, player `P`, monster `M`.
fn setup(class: u16, mode: u8) -> (ObjectControl, Fake) {
    let mut ctl = ObjectControl {
        seed: Seed::init(),
        regions: vec![],
        shrine_lists: Default::default(),
        data: Default::default(),
    };
    ctl.data.insert(
        O,
        ObjectData {
            guid: 0x55,
            class,
            ..ObjectData::default()
        },
    );
    let mut f = Fake::default();
    f.modes.insert(O, mode);
    f.guids.insert(O, 0x55);
    f.guids.insert(P, 0x77);
    f.operators.insert(P, Operator::Player(0));
    f.operators.insert(M, Operator::Monster);
    f.tick = 100_000;
    (ctl, f)
}

fn run(ctl: &mut ObjectControl, t: &ObjectTables, f: &mut Fake, op: Option<UnitId>) -> Dispatch {
    dispatch(ctl, t, f, O, op).unwrap()
}

fn modes(f: &Fake) -> Vec<u8> {
    f.calls
        .iter()
        .filter_map(|c| match c {
            Call::Mode(_, m, _) => Some(*m),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ §10

// Covers: specs/world/objects.md §10 text, §10 r1, §10 r3, §edge-cases-original-bugs r9
#[test]
fn door_debounce_host_tick() {
    let t = tables();
    let (mut ctl, mut f) = setup(DOOR, 0);
    ctl.get_mut(O).unwrap().last_tick = 1000;
    f.tick = 1499;
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
    assert!(f.calls.is_empty());
    f.tick = 1500;
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
    assert_eq!(f.calls[..2], [Call::Free(O), Call::Mode(O, 2, true)]);
    assert_eq!(ctl.get(O).unwrap().last_tick, 1500);
}

// Covers: specs/world/objects.md §10 r2
#[test]
fn door_open_and_locked() {
    let t = tables();
    // Closed: free the footprint, mode 2, tick stored.
    let (mut ctl, mut f) = setup(DOOR, 0);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.modes[&O], 2);
    assert_eq!(ctl.get(O).unwrap().last_tick, 100_000);
    // Locked without a key: sound 22 on the operator, nothing else.
    let (mut ctl, mut f) = setup(DOOR, 6);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
    assert_eq!(
        f.calls,
        vec![Call::KeyTest(P), Call::Sound(P, sound::LOCKED, None, false)]
    );
    assert_eq!(ctl.get(O).unwrap().last_tick, 0);
    // With a key: as mode 0.
    f.keys.insert(P);
    f.calls.clear();
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(
        f.calls[..3],
        [Call::KeyTest(P), Call::Free(O), Call::Mode(O, 2, true)]
    );
    assert_eq!(ctl.get(O).unwrap().last_tick, 100_000);
    // Modes 1, 3, 4, 7: nothing.
    for m in [1, 3, 4, 7] {
        let (mut ctl, mut f) = setup(DOOR, m);
        assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
        assert!(f.calls.is_empty());
        assert_eq!(ctl.get(O).unwrap().last_tick, 0);
    }
}

// Covers: specs/world/objects.md §10 r2, §edge-cases-original-bugs r8
#[test]
fn door_close_blocked() {
    let t = tables();
    // Clear doorway: stamp, mode 0.
    let (mut ctl, mut f) = setup(DOOR, 2);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.calls[..2], [Call::Stamp(O), Call::Mode(O, 0, true)]);
    assert_eq!(ctl.get(O).unwrap().last_tick, 100_000);
    // Vector: mode 2, a player (0x80) only → mode 5, tick stored.
    let (mut ctl, mut f) = setup(DOOR, 2);
    f.stats.insert((O, CELLS), 0x80);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(modes(&f), vec![5]);
    assert!(!f.calls.contains(&Call::Stamp(O)));
    assert_eq!(ctl.get(O).unwrap().last_tick, 100_000);
    // Already 5 with a monster: nothing, no tick.
    let (mut ctl, mut f) = setup(DOOR, 5);
    f.stats.insert((O, CELLS), 0x100);
    run(&mut ctl, &t, &mut f, Some(P));
    assert!(f.calls.is_empty());
    assert_eq!(ctl.get(O).unwrap().last_tick, 0);
    // A corpse (0x8000) → mode 4, from 2 or 5.
    for m in [2, 5] {
        let (mut ctl, mut f) = setup(DOOR, m);
        f.stats.insert((O, CELLS), 0x8080);
        run(&mut ctl, &t, &mut f, Some(P));
        assert_eq!(modes(&f), vec![4]);
        assert_eq!(ctl.get(O).unwrap().last_tick, 100_000);
    }
    // Mode 5, doorway clear: closes.
    let (mut ctl, mut f) = setup(DOOR, 5);
    f.stats.insert((O, CELLS), 0x7F);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(modes(&f), vec![0]);
}

// ------------------------------------------------------------------ §11

fn well_fake(charges: u8) -> (ObjectControl, Fake) {
    let (mut ctl, mut f) = setup(WELL, 0);
    ctl.get_mut(O).unwrap().interact = charges;
    f.frame = 1000;
    (ctl, f)
}

/// Life 100/200, mana 50/100, stamina full (8.8 fixed).
fn hurt(f: &mut Fake) {
    for (id, v) in [(6, 100), (7, 200), (8, 50), (9, 100), (10, 30), (11, 30)] {
        f.stats.insert((P, id), v << 8);
    }
}

// Covers: specs/world/objects.md §11 text, §11 r1, §11 r2, §11 r3, §edge-cases-original-bugs r10
#[test]
fn well_heals_and_uses_a_charge() {
    let t = tables();
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    // life + 200·128 >> 8 = +100 → 200 (max); mana + 50 → 100.
    assert_eq!(f.stats[&(P, 6)], 200 << 8);
    assert_eq!(f.stats[&(P, 8)], 100 << 8);
    assert_eq!(
        f.calls[..6],
        [
            Call::Other(format!("stat 6 {}", 200 << 8)),
            Call::Other(format!("stat 8 {}", 100 << 8)),
            Call::Other("remove state 2".into()),
            Call::Other("remove state 1".into()),
            Call::Other("cure".into()),
            Call::Other("pets".into()),
        ]
    );
    assert_eq!(ctl.get(O).unwrap().interact, 1);
    assert_eq!(modes(&f), vec![1]);
    assert_eq!(f.schedules(), vec![(O, oevent::WELL_REFILL, 1751)]);
    // Partial heal: min(life + max·Parm1 >> 8, max).
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    f.stats.insert((P, 6), 10 << 8);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.stats[&(P, 6)], 110 << 8);
}

// Covers: specs/world/objects.md §11 r1, §11 r2, §11 r3
#[test]
fn well_parm3_and_unused() {
    let mut t = tables();
    // Charges 0 → 0, nothing.
    let (mut ctl, mut f) = well_fake(0);
    hurt(&mut f);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert!(f.calls.is_empty());
    // Full player: nothing used, the charge stays.
    let (mut ctl, mut f) = well_fake(2);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(ctl.get(O).unwrap().interact, 2);
    assert!(f.schedules().is_empty());
    assert!(modes(&f).is_empty());
    // Parm3 = 2: life only; stamina always.
    t.objects[WELL as usize].parm3 = 2;
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    f.stats.insert((P, 10), 0);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.stats[&(P, 6)], 200 << 8);
    assert_eq!(f.stats[&(P, 8)], 50 << 8);
    assert_eq!(f.stats[&(P, 10)], 15 << 8);
    // Parm3 = 0, only a poison state removed: still used.
    t.objects[WELL as usize].parm3 = 0;
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    f.stats.insert((P, STATE + 2), 1);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.stats[&(P, 6)], 100 << 8);
    assert_eq!(ctl.get(O).unwrap().interact, 1);
    // Curable states, pets: used.
    for key in [CURABLE, PETS] {
        let (mut ctl, mut f) = well_fake(2);
        f.stats.insert((P, key), 1);
        run(&mut ctl, &t, &mut f, Some(P));
        assert_eq!(ctl.get(O).unwrap().interact, 1);
    }
}

// Covers: specs/world/objects.md §11 r3, §11 text
#[test]
fn well_charge_vector() {
    // Parm2 1: charges 2, use, use, refill, refill → modes 1, 2, 1, 0.
    let t = tables();
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    run(&mut ctl, &t, &mut f, Some(P));
    hurt(&mut f);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(ctl.get(O).unwrap().interact, 0);
    // Empty: no heal.
    hurt(&mut f);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.stats[&(P, 6)], 100 << 8);
    well_refill(&mut ctl, &t, &mut f, O).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 1);
    well_refill(&mut ctl, &t, &mut f, O).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 2);
    assert_eq!(modes(&f), vec![1, 2, 1, 0]);
    assert_eq!(f.modes[&O], 0);
    // Refill: the ordinary mode set (queues), then queue and flag 0x1
    // again (`objects-2.md` §24 rule 7).
    let n = f.calls.len();
    assert_eq!(
        f.calls[n - 3..],
        [
            Call::Mode(O, 0, true),
            Call::Anim(O, 0, 0, 0),
            Call::Queue(O)
        ]
    );
    assert_ne!(f.flags[&O] & oflags::CHANGED, 0);
    // A third refill is fatal (c / Parm2 ≥ 2).
    assert_eq!(
        well_refill(&mut ctl, &t, &mut f, O),
        Err(ObjectError::WellCharges(2))
    );
}

// Covers: specs/world/objects.md §11 r3
#[test]
fn well_larger_parm2_modes() {
    // Parm2 2 (4 charges): mode changes only at multiples of 2.
    let mut t = tables();
    t.objects[WELL as usize].parm2 = 2;
    let (mut ctl, mut f) = well_fake(4);
    for _ in 0..4 {
        hurt(&mut f);
        run(&mut ctl, &t, &mut f, Some(P));
    }
    assert_eq!(modes(&f), vec![1, 2]);
    for _ in 0..4 {
        well_refill(&mut ctl, &t, &mut f, O).unwrap();
    }
    assert_eq!(modes(&f), vec![1, 2, 1, 0]);
    assert_eq!(f.calls.iter().filter(|c| **c == Call::Queue(O)).count(), 4);
    // c / Parm2 = 2 → fatal.
    assert!(well_refill(&mut ctl, &t, &mut f, O).is_err());
}

// ------------------------------------------------------------------ §12

// Covers: specs/world/objects.md §12 text, §12 r1, §12 r5, §edge-cases-original-bugs r28
#[test]
fn portal_busy_and_owner() {
    let t = tables();
    for which in 0..3 {
        let (mut ctl, mut f) = setup(PORTAL, 1);
        // The dispatch refuses these first (§7.2 rule 2); the portal's own
        // busy test is reached by a direct call.
        match which {
            0 => f.interact_active.insert(P),
            1 => f.busy.insert(P),
            _ => f.cursor.insert(P),
        };
        let op = Operate {
            object: O,
            operator: Some(P),
            class: PORTAL,
            operate_fn: 15,
        };
        assert_eq!(portal(&mut ctl, &t, &mut f, &op), Ok(Some(0)));
        assert!(f.calls.is_empty());
    }
    // Iterate-players nonzero: only the owner passes.
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.stats.insert((P, ITERATE), 1);
    ctl.get_mut(O).unwrap().owner = Some(-1);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert!(f.calls.is_empty());
    ctl.get_mut(O).unwrap().owner = Some(0x77);
    f.stats.insert((UnitId(0), SPAWN), 1);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert!(f.calls.contains(&Call::Other("place 20 9 50 60".into())));
    // Iterate-players 0: anyone.
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.stats.insert((UnitId(0), SPAWN), 1);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert!(f.calls.contains(&Call::Other("place 20 9 50 60".into())));
    // §12 rule 5: a monster or no operator is fatal (`0x0058494F`; §7.1
    // stops monsters before the dispatch with live data).
    for op in [Some(M), None] {
        assert_eq!(
            dispatch(&mut ctl, &t, &mut f, O, op),
            Err(ObjectError::PortalOperator)
        );
    }
}

// Covers: specs/world/objects.md §12 r2, §12 r3, §12 r8, §12 r9, §12 r11, §12 r13, §edge-cases-original-bugs r9
#[test]
fn portal_hostile_delay_and_travel_seam() {
    let t = tables();
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.stats.insert((P, HOSTILE), 10_000);
    f.tick = 14_999;
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert_eq!(
        f.calls,
        vec![Call::Sound(P, sound::PORTAL_REFUSED, None, false)]
    );
    // 5000 ms later: the travel (rules 4–13), O without a room and no
    // partner: the spawn point of `InteractType`'s level.
    f.tick = 15_000;
    f.calls.clear();
    ctl.get_mut(O).unwrap().interact = 37;
    f.stats.insert((UnitId(0), SPAWN), 1);
    f.guids.insert(P, 0x77);
    f.frame = 100;
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    let mut stop = vec![0x0D, 0];
    stop.extend_from_slice(&0x77u32.to_le_bytes());
    stop.extend_from_slice(&[1, 55, 0, 65, 0, 0, 0]);
    assert_eq!(
        f.calls,
        vec![
            Call::Other("spawn 37".into()),
            Call::Other("place 20 9 50 60".into()),
            Call::Sound(P, sound::PORTAL, None, false),
            Call::Other("walk 20 2 55 65".into()),
            Call::Other(format!("send 20 {stop:02x?}")),
            Call::Schedule(O, oevent::END_ANIM, 100 + 1),
            Call::Other("portaled 20 175".into()),
        ]
    );
    assert_eq!(ctl.get(O).unwrap().portal_flags, 5);
}

// Covers: specs/world/objects.md §12 r4, §12 r7, §12 r10
#[test]
fn portal_owner_party_and_quest_gates() {
    let t = tables();
    // Owner 0x99 ≠ P, P without a party: refused, sound 19 with target P.
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.guids.insert(P, 0x77);
    ctl.get_mut(O).unwrap().owner = Some(0x99);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert_eq!(
        f.calls,
        vec![Call::Sound(P, sound::PORTAL_REFUSED, Some(P), false)]
    );
    // P in a party, owner not found: goes on (no spawn: error).
    f.calls.clear();
    f.stats.insert((P, PARTY), 3);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, Some(P)),
        Err(ObjectError::NoPortalDestination(O))
    );
    // O in a room: no quest record → refused.
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.rooms.insert(O, RoomId(4));
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert_eq!(
        f.calls,
        vec![Call::Sound(P, sound::PORTAL_REFUSED, Some(P), false)]
    );
    // Record, but no leveldefs row for the destination → refused.
    f.calls.clear();
    f.stats.insert((P, QREC), 1);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(0));
    assert_eq!(
        f.calls,
        vec![Call::Sound(P, sound::PORTAL_REFUSED, Some(P), false)]
    );
}

const L: UnitId = UnitId(11);

/// Object `O` (class `class`, mode `mode`) with a partner portal `L` at
/// (70, 80) in room 5; P at level 1 with its own portal GUID `puid`.
fn partnered(class: u16, mode: u8, puid: u32) -> (ObjectControl, Fake) {
    let (mut ctl, mut f) = setup(class, mode);
    f.stats.insert((O, PARTNER), L.0 as i32);
    f.guids.insert(L, 0x66);
    f.rooms.insert(L, RoomId(5));
    f.positions.insert(L, (70, 80));
    f.levels.insert(P, 1);
    f.rooms.insert(P, RoomId(2));
    f.stats.insert((P, PUID), puid as i32);
    ctl.get_mut(O).unwrap().interact = 37;
    (ctl, f)
}

fn op(class: u16) -> Operate {
    Operate {
        object: O,
        operator: Some(P),
        class,
        operate_fn: 15,
    }
}

// Covers: specs/world/objects.md §12 r6, §12 r8, §12 r9, §12 r10, §12 r11
#[test]
fn portal_partner_destination_and_quest_hook() {
    let t = tables();
    let (mut ctl, mut f) = partnered(PORTAL, 0, 0x99);
    // Partner exists: no spawn lookup; destination is L's room and
    // position; P's room is a town → the quest hook (level of P → level of
    // the destination room, rule 9) runs before the placement.
    assert_eq!(portal(&mut ctl, &t, &mut f, &op(PORTAL)), Ok(Some(0)));
    assert!(!f
        .calls
        .iter()
        .any(|c| matches!(c, Call::Other(s) if s.starts_with("spawn"))));
    let at = |name: &str| {
        f.calls
            .iter()
            .position(|c| matches!(c, Call::Other(s) if s.starts_with(name)))
    };
    assert_eq!(
        f.calls[at("qlc").unwrap()],
        Call::Other("qlc 20 1 0".into())
    );
    assert_eq!(
        f.calls[at("place").unwrap()],
        Call::Other("place 20 5 70 80".into())
    );
    assert!(at("qlc") < at("place"));
    assert_eq!(ctl.get(O).unwrap().portal_flags, 5);
}

// Covers: specs/world/objects.md §12 r12, §edge-cases-original-bugs r29
#[test]
fn portal_removal_rules() {
    let t = tables();
    let removed = |f: &Fake| -> Vec<String> {
        f.calls
            .iter()
            .filter_map(|c| match c {
                Call::Other(s) if s.starts_with("remove") || s.starts_with("act5") => {
                    Some(s.clone())
                }
                _ => None,
            })
            .collect()
    };
    // Class 59, u = L's GUID: O, then the Act V hook, then L.
    let (mut ctl, mut f) = partnered(PORTAL, 1, 0x66);
    portal(&mut ctl, &t, &mut f, &op(59)).unwrap();
    assert_eq!(removed(&f), ["remove 10", "act5 11", "remove 11"]);
    assert!(f.schedules().is_empty());
    // Class 59, u ≠ L's GUID: nothing removed, no ENDANIM.
    let (mut ctl, mut f) = partnered(PORTAL, 1, 0x99);
    portal(&mut ctl, &t, &mut f, &op(59)).unwrap();
    assert!(removed(&f).is_empty());
    assert!(f.schedules().is_empty());
    // Class 60 (even with u = L's GUID): O in mode 1 → ENDANIM.
    let (mut ctl, mut f) = partnered(PORTAL, 1, 0x66);
    f.frame = 100;
    portal(&mut ctl, &t, &mut f, &op(60)).unwrap();
    assert!(removed(&f).is_empty());
    assert_eq!(f.schedules(), [(O, oevent::END_ANIM, 101)]);
    // Class 60, mode 0: nothing.
    let (mut ctl, mut f) = partnered(PORTAL, 0, 0x66);
    portal(&mut ctl, &t, &mut f, &op(60)).unwrap();
    assert!(f.schedules().is_empty());
}

// Covers: specs/world/objects.md §12 r7
#[test]
fn portal_class_59_quest_flag_gate() {
    let mut t = tables();
    let mut row = d2_data::tables::Leveldefs::decode(&[0; d2_data::tables::Leveldefs::SIZE]);
    (row.questflag, row.questflagex) = (5, 9);
    t.leveldefs = vec![row; 40];
    for (expansion, quest) in [(false, 5u16), (true, 9)] {
        // Gate applies: u ∉ {GUID(O), GUID(L)}, flag clear → refused.
        let (mut ctl, mut f) = partnered(PORTAL, 1, 0x99);
        f.rooms.insert(O, RoomId(4));
        f.stats.insert((P, QREC), 1);
        f.stats.insert((UnitId(0), EXPANSION), i32::from(expansion));
        portal(&mut ctl, &t, &mut f, &op(59)).unwrap();
        assert_eq!(
            f.calls,
            vec![Call::Sound(P, sound::PORTAL_REFUSED, Some(P), false)]
        );
        // The same flag set: goes on.
        let (mut ctl, mut f) = partnered(PORTAL, 1, 0x99);
        f.rooms.insert(O, RoomId(4));
        f.stats.insert((P, QREC), 1);
        f.stats.insert((UnitId(0), EXPANSION), i32::from(expansion));
        f.stats.insert((P, QBIT + quest), 1);
        portal(&mut ctl, &t, &mut f, &op(59)).unwrap();
        assert!(f
            .calls
            .iter()
            .any(|c| matches!(c, Call::Other(s) if s.starts_with("place"))));
    }
    // u = O's GUID skips the test; class 60 skips it too.
    for (class, puid) in [(59, 0x55u32), (60, 0x99)] {
        let (mut ctl, mut f) = partnered(PORTAL, 1, puid);
        f.rooms.insert(O, RoomId(4));
        f.stats.insert((P, QREC), 1);
        portal(&mut ctl, &t, &mut f, &op(class)).unwrap();
        assert!(f
            .calls
            .iter()
            .any(|c| matches!(c, Call::Other(s) if s.starts_with("place"))));
    }
    // No partner: the class-59 test is skipped.
    let (mut ctl, mut f) = setup(PORTAL, 1);
    f.rooms.insert(O, RoomId(4));
    f.stats.insert((P, QREC), 1);
    f.stats.insert((UnitId(0), SPAWN), 1);
    ctl.get_mut(O).unwrap().interact = 37;
    portal(&mut ctl, &t, &mut f, &op(59)).unwrap();
    assert!(f
        .calls
        .iter()
        .any(|c| matches!(c, Call::Other(s) if s.starts_with("place"))));
}

// ------------------------------------------------------------------ §13

// Covers: specs/world/objects.md §13
#[test]
fn torch_modes() {
    let t = tables();
    let (mut ctl, mut f) = setup(TORCH, 0);
    f.flags.insert(O, oflags::SELECTABLE);
    assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
    assert_eq!(f.modes[&O], 1);
    assert_eq!(f.flags[&O] & oflags::SELECTABLE, 0);
    for m in [1, 2] {
        let (mut ctl, mut f) = setup(TORCH, m);
        f.flags.insert(O, oflags::SELECTABLE);
        assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
        assert_eq!(f.modes[&O], 0);
        assert_ne!(f.flags[&O] & oflags::SELECTABLE, 0);
    }
    for m in [3, 7] {
        let (mut ctl, mut f) = setup(TORCH, m);
        assert_eq!(run(&mut ctl, &t, &mut f, Some(P)), Dispatch::Done(1));
        assert!(f.calls.is_empty());
    }
}

// Covers: specs/world/objects-2.md §24 r5
#[test]
fn well_used_is_set_by_the_write_not_by_a_changed_value() {
    let mut t = tables();
    // Parm1 0: the written value equals the total, and still counts.
    t.objects[WELL as usize].parm1 = 0;
    let (mut ctl, mut f) = well_fake(2);
    hurt(&mut f);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(f.stats[&(P, 6)], 100 << 8, "life unchanged");
    assert_eq!(f.calls[0], Call::Other(format!("stat 6 {}", 100 << 8)));
    assert_eq!(ctl.get(O).unwrap().interact, 1, "a charge was used");
    // Nothing below its maximum, nothing removed: not used.
    let (mut ctl, mut f) = well_fake(2);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(ctl.get(O).unwrap().interact, 2);
    // A removed poison list alone uses a charge.
    let (mut ctl, mut f) = well_fake(2);
    f.stats.insert((P, STATE + 2), 1);
    run(&mut ctl, &t, &mut f, Some(P));
    assert_eq!(ctl.get(O).unwrap().interact, 1);
}

// Covers: specs/world/objects-2.md §24 r6
#[test]
fn locked_door_with_no_operator_is_fatal() {
    let t = tables();
    let (mut ctl, mut f) = setup(DOOR, 6);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, None),
        Err(ObjectError::KeyTestNoUnit)
    );
    assert!(f.calls.is_empty());
}

// Covers: specs/world/objects-2.md §24 r7
#[test]
fn well_refill_queues_and_flags_even_without_a_mode_set() {
    let mut t = tables();
    // Parm2 4: modes at charges 0, 4, 8 only; c = 2 sets no mode.
    t.objects[WELL as usize].parm2 = 4;
    let (mut ctl, mut f) = well_fake(1);
    f.flags.insert(O, 0);
    well_refill(&mut ctl, &t, &mut f, O).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 2);
    assert!(modes(&f).is_empty());
    assert_eq!(f.calls, vec![Call::Queue(O)]);
    assert_ne!(f.flags[&O] & oflags::CHANGED, 0);
    // c = 4: the mode set (which queues and flags) and the explicit
    // repeat.
    let (mut ctl, mut f) = well_fake(3);
    f.flags.insert(O, 0);
    well_refill(&mut ctl, &t, &mut f, O).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 4);
    assert_eq!(modes(&f), vec![1]);
    assert_eq!(f.calls.iter().filter(|c| **c == Call::Queue(O)).count(), 1);
    assert_ne!(f.flags[&O] & oflags::CHANGED, 0);
}
