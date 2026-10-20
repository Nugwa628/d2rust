// Spec: specs/world/objects.md §8 (Test vectors, Edge cases)
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::super::fake::{blank_level, blank_object, Call, Fake};
use super::super::{
    dispatch, oflags, Dispatch, MiscWorld, ObjectData, ObjectTables, Operator, Region, ShrineWorld,
};
use super::*;
use crate::rng::Seed;

impl ChestWorld for Fake {
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.room_unit_lists.get(&room).cloned().unwrap_or_default()
    }
    fn unit_type(&self, unit: UnitId) -> Option<u8> {
        self.unit_types.get(&unit).copied()
    }
    fn unit_class(&self, unit: UnitId) -> Option<u32> {
        self.unit_classes
            .get(&unit)
            .copied()
            .or_else(|| match self.operator(unit) {
                Operator::Player(c) => Some(u32::from(c)),
                _ => None,
            })
    }
}

mod mutant_tests;
#[cfg(test)]
mod tests_c2world;

const OBJ: UnitId = UnitId(10);
const PLAYER: UnitId = UnitId(20);
const ROOM: RoomId = RoomId(3);

/// A [`Fake`] plus the chest seams' scripted answers. Every chest seam
/// call is recorded as `Call::Other` in `f.calls`, in call order.
#[derive(Default)]
struct H {
    f: Fake,
    /// Scripted chest drops, in order: `Some(quality)` drops an item,
    /// `None` (or an empty queue) drops nothing.
    drops: VecDeque<Option<u8>>,
    next_item: u32,
    item_q: BTreeMap<UnitId, u8>,
    types: BTreeMap<UnitId, u8>,
    trap_monster: Option<u32>,
    spot_ok: bool,
    free: Option<(RoomId, i32, i32)>,
    room_units: BTreeMap<RoomId, Vec<UnitId>>,
    within: BTreeSet<(UnitId, UnitId, i32)>,
    in_room: bool,
    /// Log of the `&self` seams, merged into `others` in call order.
    calls_log: std::cell::RefCell<Vec<String>>,
}

impl H {
    fn other(&mut self, s: String) {
        self.flush();
        self.f.calls.push(Call::Other(s));
    }
    fn flush(&mut self) {
        let logged: Vec<String> = self.calls_log.borrow_mut().drain(..).collect();
        for l in logged {
            self.f.calls.push(Call::Other(l));
        }
    }
    fn others(&self) -> Vec<String> {
        self.f
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Other(s) => Some(s.clone()),
                _ => None,
            })
            .chain(self.calls_log.borrow().iter().cloned())
            .collect()
    }
    fn drop_qs(&self) -> Vec<String> {
        self.others()
            .into_iter()
            .filter(|s| s.starts_with("drop") || s.starts_with("code"))
            .collect()
    }
}

impl ObjectWorld for H {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.f.frame()
    }
    fn host_tick(&self) -> u32 {
        self.f.host_tick()
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.f.guid(unit)
    }
    fn find_object(&self, guid: u32) -> Option<UnitId> {
        self.f.find_object(guid)
    }
    fn operator(&self, unit: UnitId) -> Operator {
        self.f.operator(unit)
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.f.mode(unit)
    }
    fn write_mode(&mut self, unit: UnitId, mode: u8, queue: bool) {
        self.f.write_mode(unit, mode, queue)
    }
    fn set_anim(&mut self, unit: UnitId, frame_count: i32, frame: i32, speed: i16) {
        self.f.set_anim(unit, frame_count, frame, speed)
    }
    fn unit_seed(&mut self, unit: UnitId) -> Option<&mut Seed> {
        self.f.unit_seed(unit)
    }
    fn flags(&self, unit: UnitId) -> u32 {
        self.f.flags(unit)
    }
    fn set_flags(&mut self, unit: UnitId, flags: u32) {
        self.f.set_flags(unit, flags)
    }
    fn queue_update(&mut self, unit: UnitId) {
        self.f.queue_update(unit)
    }
    fn room(&self, unit: UnitId) -> Option<RoomId> {
        self.f.room(unit)
    }
    fn level(&self, unit: UnitId) -> Option<u32> {
        self.f.level(unit)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.f.position(unit)
    }
    fn schedule(&mut self, unit: UnitId, ev: u8, frame: i32) {
        self.f.schedule(unit, ev, frame)
    }
    fn cancel_timers(&mut self, unit: UnitId) {
        self.f.cancel_timers(unit)
    }
    fn stamp_footprint(&mut self, unit: UnitId, room: Option<RoomId>, x: i32, y: i32) {
        self.f.stamp_footprint(unit, room, x, y)
    }
    fn free_footprint(&mut self, unit: UnitId) {
        self.f.free_footprint(unit)
    }
    fn sound(&mut self, unit: UnitId, id: u8, to: Option<UnitId>, now: bool) {
        self.f.sound(unit, id, to, now)
    }
    fn key_test(&mut self, player: UnitId) -> bool {
        self.f.key_test(player)
    }
    fn in_interact_range(&self, operator: UnitId, object: UnitId) -> bool {
        self.f.in_interact_range(operator, object)
    }
    fn interact_active(&self, player: UnitId) -> bool {
        self.f.interact_active(player)
    }
    fn player_busy(&self, player: UnitId) -> bool {
        self.f.player_busy(player)
    }
    fn cursor_item(&self, player: UnitId) -> bool {
        self.f.cursor_item(player)
    }
    fn allocate_object(
        &mut self,
        room: RoomId,
        class: u16,
        x: i32,
        y: i32,
        mode: u8,
    ) -> Option<UnitId> {
        self.f.allocate_object(room, class, x, y, mode)
    }
    fn staff_tomb_level(&self) -> u32 {
        self.f.staff_tomb_level()
    }
    fn store_mode(&mut self, unit: UnitId, mode: u8) {
        self.f.store_mode(unit, mode)
    }
}

impl ChestWorld for H {
    fn chest_drop(&mut self, op: &Operate, q: u8) -> Option<UnitId> {
        self.other(format!("drop {q} {:?}", op.object));
        let qual = self.drops.pop_front().flatten()?;
        self.next_item += 1;
        let i = UnitId(1000 + self.next_item);
        self.item_q.insert(i, qual);
        self.types.insert(i, unit_type::ITEM);
        Some(i)
    }
    fn unit_type(&self, unit: UnitId) -> Option<u8> {
        self.types.get(&unit).copied()
    }
    fn item_quality(&self, item: UnitId) -> Option<u8> {
        self.item_q.get(&item).copied()
    }
    fn code_drop(&mut self, object: UnitId, code: u32) -> Option<UnitId> {
        let s = String::from_utf8_lossy(&code.to_le_bytes()).into_owned();
        self.other(format!("code {s} {object:?}"));
        None
    }
    fn drop_item_code(&mut self, object: UnitId, code: u32) {
        self.other(format!("dropcode {code:#x} {object:?}"));
    }
    /// The scripted region list: the one class `trap_monster`, read by
    /// the trap monster id (§8.3) on a cache miss.
    fn monster_region_classes(&self, level: u32) -> Option<Vec<i32>> {
        self.calls_log.borrow_mut().push(format!("trapid {level}"));
        Some(
            self.trap_monster
                .map(|m| vec![m as i32])
                .unwrap_or_default(),
        )
    }
    fn monstats_count(&self) -> u32 {
        1000
    }
    fn spawn_trap_monster(&mut self, object: UnitId, monster: u32, arg: u32) {
        self.other(format!("trapmon {monster} {arg} {object:?}"));
    }
    fn spawn_monster_at_spot(
        &mut self,
        room: RoomId,
        monster: u32,
        x: i32,
        y: i32,
        flag: u32,
    ) -> Option<UnitId> {
        self.other(format!("atspot {monster} {room:?} {x} {y} {flag:#x}"));
        self.spot_ok.then_some(UnitId(500))
    }
    fn free_spot(&self, _room: RoomId, _x: i32, _y: i32, mask: u32) -> Option<(RoomId, i32, i32)> {
        assert_eq!(mask, 0x3F11);
        self.free
    }
    fn spawn_monster(&mut self, room: RoomId, monster: u32, x: i32, y: i32) -> Option<UnitId> {
        self.other(format!("spawn {monster} {room:?} {x} {y}"));
        Some(UnitId(501))
    }
    fn start_player_skill_on(&mut self, player: UnitId, object: UnitId) {
        self.other(format!("skill {player:?} {object:?}"));
    }
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.room_units.get(&room).cloned().unwrap_or_default()
    }
    fn within(&self, object: UnitId, unit: UnitId, range: i32) -> bool {
        self.within.contains(&(object, unit, range))
    }
    fn trap_damage(&mut self, object: UnitId, target: UnitId) {
        self.other(format!("damage {object:?} {target:?}"));
    }
    fn spawn_region_monster(&mut self, object: UnitId) {
        self.other(format!("region {object:?}"));
    }
    fn in_room(&self, _room: RoomId, _x: i32, _y: i32) -> bool {
        self.in_room
    }
}
impl ShrineWorld for H {}
impl MiscWorld for H {}
impl super::super::MechWorld for H {}

// ------------------------------------------------------------------ setup

const CHEST: u16 = 5;
const CASKET: u16 = 6;
const URN: u16 = 7;
const BARREL: u16 = 8;
const CORPSE: u16 = 9;
const EVIL_URN: u16 = 12;

/// Tables: 400 blank `Sync` rows (no unit-seed draw); `levels` 0–129 with
/// acts by the town bounds; class rows set per test.
fn tables() -> ObjectTables {
    let mut objects = vec![blank_object(); 400];
    for o in &mut objects {
        o.sync = 1;
        o.framecnt1 = 10 << 8;
    }
    let levels = (0..130u32)
        .map(|i| {
            let mut l = blank_level();
            l.act = match i {
                0..=39 => 0,
                40..=74 => 1,
                75..=102 => 2,
                103..=108 => 3,
                _ => 4,
            };
            l
        })
        .collect();
    ObjectTables {
        objects,
        levels,
        ..Default::default()
    }
}

/// The control with one object region per level 0–129 (acts as
/// [`tables`]).
fn ctl(seed: Seed) -> ObjectControl {
    let regions = (0..130u32)
        .map(|i| {
            Some(Region::new(match i {
                0..=39 => 0,
                40..=74 => 1,
                75..=102 => 2,
                103..=108 => 3,
                _ => 4,
            }))
        })
        .collect();
    ObjectControl {
        seed,
        regions,
        shrine_lists: Default::default(),
        data: BTreeMap::new(),
    }
}

/// An object `OBJ` of `class` in mode 0, room `ROOM`, level 2, frame 100.
fn setup(class: u16, interact: u8, seed: Seed) -> (ObjectControl, H) {
    let mut c = ctl(seed);
    c.data.insert(
        OBJ,
        ObjectData {
            class,
            interact,
            ..Default::default()
        },
    );
    let mut h = H::default();
    h.f.frame = 100;
    h.f.modes.insert(OBJ, 0);
    h.f.flags.insert(OBJ, oflags::SELECTABLE);
    h.f.rooms.insert(OBJ, ROOM);
    h.f.levels.insert(OBJ, 2);
    h.f.positions.insert(OBJ, (40, 50));
    h.f.operators.insert(PLAYER, Operator::Player(1));
    (c, h)
}

fn op(class: u16, f: u8) -> Operate {
    Operate {
        object: OBJ,
        operator: Some(PLAYER),
        class,
        operate_fn: f,
    }
}

/// The first seed `{lo, 666}` (lo from 1) whose first draws satisfy `p`.
fn find_seed(p: impl Fn(&mut Seed) -> bool) -> Seed {
    (1..100_000)
        .map(Seed::init_low)
        .find(|s| p(&mut s.clone()))
        .expect("seed")
}

fn mode_calls(h: &H) -> Vec<(UnitId, u8)> {
    h.f.calls
        .iter()
        .filter_map(|c| match c {
            Call::Mode(u, m, _) => Some((*u, *m)),
            _ => None,
        })
        .collect()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// ------------------------------------------------------------------ §8.1

// Covers: specs/world/objects.md §8.1 r1, §8.1 r5, §8.1 r6, §8.1 r7
#[test]
fn chest_unlocked_drops_once_and_opens() {
    let mut t = tables();
    t.objects[CHEST as usize].mode1 = 1;
    let seed = find_seed(|s| s.roll(100) >= 25);
    let (mut c, mut h) = setup(CHEST, 0, seed);
    c.get_mut(OBJ).unwrap().drop_code = 0x2020_6B6B;
    h.drops.push_back(Some(2));
    let r = operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap();
    assert_eq!(r, 1);
    let mut e = seed;
    e.roll(100);
    assert_eq!(c.seed, e);
    assert_eq!(
        h.others(),
        s(&["drop 0 UnitId(10)", "dropcode 0x20206b6b UnitId(10)"])
    );
    assert_eq!(mode_calls(&h), vec![(OBJ, 1)]);
    // ENDANIM at frame + fc1 + 1.
    assert_eq!(h.f.schedules(), vec![(OBJ, oevent::END_ANIM, 111)]);
    assert_eq!(h.f.flags(OBJ) & oflags::SELECTABLE, 0);
}

// Covers: specs/world/objects.md §8.1 r1
#[test]
fn chest_not_in_mode_0_does_nothing() {
    let t = tables();
    let seed = Seed::init_low(7);
    let (mut c, mut h) = setup(CHEST, 0x81, seed);
    h.f.modes.insert(OBJ, 1);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert!(h.f.calls.is_empty());
    assert_eq!(c.seed, seed);
}

// Covers: specs/world/objects.md §8.1 r5, §8.1 r7
#[test]
fn chest_empty_quarter_still_opens_mode_2_without_mode1() {
    let t = tables();
    let seed = find_seed(|s| s.roll(100) < 25);
    let (mut c, mut h) = setup(CHEST, 0, seed);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert!(h.drop_qs().is_empty());
    assert_eq!(mode_calls(&h), vec![(OBJ, 2)]);
    assert!(h.f.schedules().is_empty());
    assert_eq!(h.f.flags(OBJ) & oflags::SELECTABLE, 0);
}

// Covers: specs/world/objects.md §8.1 r2, §8.1 r5
#[test]
fn chest_locked_with_key_two_picks_and_no_empty_roll() {
    let t = tables();
    // Locked: the 25% empty result does not apply.
    let seed = find_seed(|s| s.roll(100) < 25);
    let (mut c, mut h) = setup(CHEST, 0x80, seed);
    h.f.keys.insert(PLAYER);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert_eq!(h.f.calls[0], Call::KeyTest(PLAYER));
    assert_eq!(h.f.calls[1], Call::Sound(PLAYER, sound::UNLOCK, None, true));
    assert_eq!(h.drop_qs(), s(&["drop 0 UnitId(10)", "drop 0 UnitId(10)"]));
    assert_eq!(mode_calls(&h), vec![(OBJ, 2)]);
}

// Covers: specs/world/objects.md §8.1 r2
#[test]
fn chest_locked_without_key_stays_closed() {
    let t = tables();
    let seed = Seed::init_low(9);
    let (mut c, mut h) = setup(CHEST, 0x80, seed);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert_eq!(
        h.f.calls,
        vec![
            Call::KeyTest(PLAYER),
            Call::Sound(PLAYER, sound::LOCKED, None, false)
        ]
    );
    assert_eq!(c.seed, seed);
    assert_eq!(h.f.mode(OBJ), 0);
}

// Covers: specs/world/objects.md §8.1 r2, §edge-cases-original-bugs r23
#[test]
fn chest_locked_assassin_needs_no_key() {
    let t = tables();
    let seed = find_seed(|s| s.roll(100) >= 25);
    let (mut c, mut h) = setup(CHEST, 0x80, seed);
    h.f.operators.insert(PLAYER, Operator::Player(ASSASSIN));
    assert_eq!(operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap(), 1);
    assert!(!h.f.calls.contains(&Call::KeyTest(PLAYER)));
    assert_eq!(h.f.calls[0], Call::Sound(PLAYER, sound::UNLOCK, None, true));
    assert_eq!(h.drop_qs().len(), 2);
}

// Covers: specs/world/objects.md §8.1 r3, §8.1 r5
#[test]
fn chest_sparkle_q6_retries_until_magic() {
    let t = tables();
    // Sparkle roll < 5 → Q 6; the second roll(100) result does not matter
    // (sparkling chests are never empty).
    let seed = find_seed(|s| s.roll(100) < 5 && s.roll(100) < 25);
    let (mut c, mut h) = setup(CHEST, 0, seed);
    c.get_mut(OBJ).unwrap().spark = 1;
    // Pick: non-magic; retries: none, rare (6, magic) → stop.
    h.drops.extend([Some(2), None, Some(6), Some(7)]);
    operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs(), vec!["drop 6 UnitId(10)".to_string(); 3]);
    let mut e = seed;
    e.roll(100);
    e.roll(100);
    assert_eq!(c.seed, e);
}

// Covers: specs/world/objects.md §8.1 r3, §8.1 r5
#[test]
fn chest_sparkle_q4_magic_pick_stops_and_retry_cap_is_10() {
    let t = tables();
    let seed = find_seed(|s| s.roll(100) >= 5);
    // Magic first pick: no retries.
    let (mut c, mut h) = setup(CHEST, 0, seed);
    c.get_mut(OBJ).unwrap().spark = 1;
    h.drops.push_back(Some(4));
    operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs(), s(&["drop 4 UnitId(10)"]));
    // Never magic: 1 pick + 10 retries. Quality 3 (superior) and 10 are
    // not magic.
    let (mut c, mut h) = setup(CHEST, 0, seed);
    c.get_mut(OBJ).unwrap().spark = 1;
    h.drops.extend([Some(3), Some(10), Some(1)]);
    operate(&mut c, &t, &mut h, &op(CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs().len(), 11);
}

// Covers: specs/world/objects.md §8.1 r4, §edge-cases-original-bugs r7
#[test]
fn class_397_low_bands_magic_opens_and_lock_still_takes_key() {
    let mut t = tables();
    t.objects[SPECIAL_CHEST as usize].mode1 = 1;
    for (lo, hi, q) in [(0, 200, 7), (200, 600, 5), (600, 1200, 6)] {
        // Sparkling and locked: the sparkle roll is drawn, Q and picks are
        // ignored.
        let seed = find_seed(|s| {
            s.roll(100);
            let r = s.roll(10000);
            (lo..hi).contains(&r)
        });
        let (mut c, mut h) = setup(SPECIAL_CHEST, 0x80, seed);
        c.get_mut(OBJ).unwrap().spark = 1;
        h.f.keys.insert(PLAYER);
        h.drops.extend([Some(2), Some(5)]);
        assert_eq!(
            operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap(),
            1
        );
        assert_eq!(h.f.calls[0], Call::KeyTest(PLAYER));
        let d = format!("drop {q} UnitId(10)");
        assert_eq!(h.drop_qs(), vec![d.clone(), d]);
        let mut e = seed;
        e.roll(100);
        e.roll(10000);
        assert_eq!(c.seed, e);
        assert_eq!(mode_calls(&h), vec![(OBJ, 1)]);
    }
}

fn tail(k: usize) -> Vec<String> {
    let mut v = Vec::new();
    for _ in 0..4usize.saturating_sub(k) {
        v.push("drop 0 UnitId(10)".to_string());
    }
    v.extend(vec!["code gld  UnitId(10)".to_string(); 5]);
    v.extend(vec!["code hp3  UnitId(10)".to_string(); 2]);
    v.extend(vec!["code mp3  UnitId(10)".to_string(); 2]);
    v
}

// Covers: specs/world/objects.md §8.1 r4
#[test]
fn class_397_low_band_null_or_two_plain_go_to_tail() {
    let t = tables();
    let seed = find_seed(|s| s.roll(10000) < 200);
    // Null first → tail; tail: plain, plain, magic → k = 2.
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops.extend([None, Some(2), Some(2), Some(4)]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    let mut want = s(&["drop 7 UnitId(10)"]);
    want.extend(vec!["drop 4 UnitId(10)".to_string(); 3]);
    want.extend(tail(2));
    assert_eq!(h.drop_qs(), want);
    // Two plain → tail; tail: 10 nulls → k = 0.
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops.extend([Some(2), Some(2)]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    let mut want = vec!["drop 7 UnitId(10)".to_string(); 2];
    want.extend(vec!["drop 4 UnitId(10)".to_string(); 10]);
    want.extend(tail(0));
    assert_eq!(h.drop_qs(), want);
}

// Covers: specs/world/objects.md §8.1 r4
#[test]
fn class_397_band_3200_stops_at_three_magic_or_tails_on_nothing() {
    let t = tables();
    let seed = find_seed(|s| (1200..3200).contains(&s.roll(10000)));
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops
        .extend([Some(4), None, Some(2), Some(5), Some(9), Some(4)]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs(), vec!["drop 4 UnitId(10)".to_string(); 5]);
    // Nothing dropped in 10 → tail (10 more D(4), all null → k = 0).
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    let mut want = vec!["drop 4 UnitId(10)".to_string(); 20];
    want.extend(tail(0));
    assert_eq!(h.drop_qs(), want);
}

// Covers: specs/world/objects.md §8.1 r4
#[test]
fn class_397_band_6200_counts_plain_and_pays_gold() {
    let t = tables();
    let seed = find_seed(|s| (3200..6200).contains(&s.roll(10000)));
    // m = 2 after 3 plain: n = 3 → 4 gold.
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops
        .extend([Some(2), Some(5), Some(2), Some(2), Some(6)]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    let mut want = vec!["drop 4 UnitId(10)".to_string(); 5];
    want.extend(vec!["code gld  UnitId(10)".to_string(); 4]);
    assert_eq!(h.drop_qs(), want);
    // Nothing in 10: one D(0) that drops → n = 1 → 6 gold.
    let mut q = VecDeque::from(vec![None; 10]);
    q.push_back(Some(2));
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops = q;
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    let mut want = vec!["drop 4 UnitId(10)".to_string(); 10];
    want.push("drop 0 UnitId(10)".to_string());
    want.extend(vec!["code gld  UnitId(10)".to_string(); 6]);
    assert_eq!(h.drop_qs(), want);
    // Nothing at all: n stays 0 → 7 gold.
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs().iter().filter(|x| x.contains("gld")).count(), 7);
    // 7 plain → no gold.
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops.extend(vec![Some(2); 10]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    assert_eq!(h.drop_qs(), vec!["drop 4 UnitId(10)".to_string(); 10]);
}

// Covers: specs/world/objects.md §8.1 r4, §8.1 r7
#[test]
fn class_397_high_band_is_tail() {
    let t = tables();
    let seed = find_seed(|s| s.roll(10000) >= 6200);
    let (mut c, mut h) = setup(SPECIAL_CHEST, 0, seed);
    h.drops
        .extend([Some(2), Some(1), Some(3), Some(2), Some(2)]);
    operate(&mut c, &t, &mut h, &op(SPECIAL_CHEST, 4)).unwrap();
    // k = 5 plain (≥ 4: no D(0)) then 5 nulls; then the fixed codes.
    let mut want = vec!["drop 4 UnitId(10)".to_string(); 10];
    want.extend(tail(5));
    assert_eq!(h.drop_qs(), want);
    assert_eq!(mode_calls(&h), vec![(OBJ, 2)]);
}

// ------------------------------------------------------------------ §8.2

// Covers: specs/world/objects.md §8.2, §edge-cases-original-bugs r3
#[test]
fn casket_no_item_changes_nothing() {
    let t = tables();
    let seed = Seed::init_low(5);
    for (class, f) in [(CASKET, 1), (EVIL_URN, 68)] {
        let (mut c, mut h) = setup(class, 1, seed);
        assert_eq!(operate(&mut c, &t, &mut h, &op(class, f)).unwrap(), 1);
        assert_eq!(h.others(), s(&["drop 0 UnitId(10)"]));
        assert_eq!(c.seed, seed);
        assert_eq!(h.f.mode(OBJ), 0);
        assert_eq!(h.f.flags(OBJ), oflags::SELECTABLE);
    }
}

// Covers: specs/world/objects.md §8.2, §8.3
#[test]
fn casket_item_trap_monster_footprint_and_arm() {
    let t = tables();
    let seed = find_seed(|s| s.roll(10000) >= 8192);
    let (mut c, mut h) = setup(CASKET, 1, seed);
    h.drops.push_back(Some(2));
    h.trap_monster = Some(170);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CASKET, 1)).unwrap(), 1);
    assert_eq!(
        h.others(),
        s(&["drop 0 UnitId(10)", "trapid 2", "trapmon 170 8 UnitId(10)"])
    );
    let mut e = seed;
    e.roll(10000);
    assert_eq!(c.seed, e);
    assert_eq!(mode_calls(&h), vec![(OBJ, 1)]);
    assert!(h.f.calls.contains(&Call::Free(OBJ)));
    // ENDANIM then the trap event at frame + 35, sound 13.
    assert_eq!(
        h.f.schedules(),
        vec![(OBJ, oevent::END_ANIM, 111), (OBJ, oevent::TRAP, 135)]
    );
    assert!(h
        .f
        .calls
        .contains(&Call::Sound(OBJ, sound::TRAP_ARMED, None, false)));
    // Below 8192: no monster; HasCollision1 set: footprint kept.
    let mut t2 = tables();
    t2.objects[CASKET as usize].hascollision1 = 1;
    let seed = find_seed(|s| s.roll(10000) < 8192);
    let (mut c, mut h) = setup(CASKET, 0, seed);
    h.drops.push_back(Some(2));
    h.trap_monster = Some(170);
    operate(&mut c, &t2, &mut h, &op(CASKET, 1)).unwrap();
    assert_eq!(h.others(), s(&["drop 0 UnitId(10)"]));
    assert!(!h.f.calls.contains(&Call::Free(OBJ)));
    assert_eq!(h.f.schedules(), vec![(OBJ, oevent::END_ANIM, 111)]);
}

// Covers: specs/world/objects.md §8.2
#[test]
fn urn_rolls_20_inclusive() {
    let t = tables();
    for (want_drop, seed) in [
        (true, find_seed(|s| s.roll(100) == 20)),
        (false, find_seed(|s| s.roll(100) == 21)),
    ] {
        let (mut c, mut h) = setup(URN, 0, seed);
        assert_eq!(operate(&mut c, &t, &mut h, &op(URN, 3)).unwrap(), 1);
        assert_eq!(!h.drop_qs().is_empty(), want_drop);
        // Mode change happens before the roll's drop.
        assert!(matches!(h.f.calls[0], Call::Mode(OBJ, 1, true)));
        assert_eq!(h.f.flags(OBJ) & oflags::SELECTABLE, 0);
        assert!(h.f.calls.contains(&Call::Free(OBJ)));
        let mut e = seed;
        e.roll(100);
        assert_eq!(c.seed, e);
    }
}

// Covers: specs/world/objects.md §8.2, §edge-cases-original-bugs r24; specs/world/objects-2.md §24 r2
#[test]
fn barrel_skill_draw_order_and_no_trap_arm() {
    let t = tables();
    let seed = find_seed(|s| s.roll(10000) >= 8192 && s.roll(100) <= 20);
    let (mut c, mut h) = setup(BARREL, 3, seed);
    h.trap_monster = Some(5);
    h.drops.push_back(Some(2));
    assert_eq!(operate(&mut c, &t, &mut h, &op(BARREL, 5)).unwrap(), 0);
    assert_eq!(
        h.others(),
        s(&[
            "skill UnitId(20) UnitId(10)",
            "trapid 2",
            "trapmon 5 8 UnitId(10)",
            "drop 0 UnitId(10)"
        ])
    );
    let mut e = seed;
    e.roll(10000);
    e.roll(100);
    assert_eq!(c.seed, e);
    // Footprint freed whatever HasCollision1; ENDANIM only (trap 3 not
    // armed).
    assert!(h.f.calls.contains(&Call::Free(OBJ)));
    assert_eq!(h.f.schedules(), vec![(OBJ, oevent::END_ANIM, 111)]);
    // A monster operator starts no skill.
    let (mut c, mut h) = setup(BARREL, 0, seed);
    h.f.operators.insert(PLAYER, Operator::Monster);
    operate(&mut c, &t, &mut h, &op(BARREL, 5)).unwrap();
    assert!(!h.others().iter().any(|x| x.starts_with("skill")));
}

// Covers: specs/world/objects.md §8.2, §edge-cases-original-bugs r11
#[test]
fn exploding_barrel_damage_and_chain_recursion() {
    let t = tables();
    let seed = Seed::init_low(3);
    let (mut c, mut h) = setup(EXPLODING_BARREL_CLASS, 0, seed);
    let (dead_p, live_p, mon, far_mon, b2, b3) = (
        UnitId(30),
        UnitId(31),
        UnitId(32),
        UnitId(33),
        UnitId(34),
        UnitId(35),
    );
    for (u, ty, m) in [
        (dead_p, unit_type::PLAYER, 17),
        (live_p, unit_type::PLAYER, 1),
        (mon, unit_type::MONSTER, 1),
        (far_mon, unit_type::MONSTER, 1),
        (b2, unit_type::OBJECT, 0),
        (b3, unit_type::OBJECT, 0),
    ] {
        h.types.insert(u, ty);
        h.f.modes.insert(u, m);
        h.f.rooms.insert(u, ROOM);
    }
    h.types.insert(OBJ, unit_type::OBJECT);
    for b in [b2, b3] {
        c.data.insert(
            b,
            ObjectData {
                class: EXPLODING_BARREL_CLASS,
                ..Default::default()
            },
        );
    }
    h.room_units
        .insert(ROOM, vec![OBJ, dead_p, b2, live_p, mon, far_mon, b3]);
    for u in [dead_p, live_p, mon] {
        h.within.insert((OBJ, u, 3));
    }
    // b2 is within 2 of the first barrel; b3 only of b2; b2 hurts `mon`.
    h.within.insert((OBJ, b2, 2));
    h.within.insert((b2, b3, 2));
    h.within.insert((b2, mon, 3));
    // b3 lists the first barrel (mode 1 by then): not chained back.
    h.within.insert((b3, OBJ, 2));
    assert_eq!(
        operate(&mut c, &t, &mut h, &op(EXPLODING_BARREL_CLASS, 7)).unwrap(),
        0
    );
    assert_eq!(
        h.others(),
        s(&[
            "damage UnitId(34) UnitId(32)",
            "damage UnitId(10) UnitId(31)",
            "damage UnitId(10) UnitId(32)",
        ])
    );
    // Chained barrels finish first (b3 inside b2 inside OBJ).
    let fin: Vec<Call> =
        h.f.calls
            .iter()
            .filter(|c| matches!(c, Call::Free(_) | Call::Schedule(..)))
            .cloned()
            .collect();
    assert_eq!(
        fin,
        vec![
            Call::Free(b3),
            Call::Schedule(b3, oevent::END_ANIM, 111),
            Call::Free(b2),
            Call::Schedule(b2, oevent::END_ANIM, 111),
            Call::Free(OBJ),
            Call::Schedule(OBJ, oevent::END_ANIM, 111),
        ]
    );
    assert_eq!(mode_calls(&h), vec![(OBJ, 1), (b2, 1), (b3, 1)]);
    assert_eq!(c.seed, seed);
    // Not in mode 0: nothing.
    let mut h2 = H::default();
    h2.f.modes.insert(OBJ, 1);
    operate(&mut c, &t, &mut h2, &op(EXPLODING_BARREL_CLASS, 7)).unwrap();
    assert!(h2.f.calls.is_empty());
}

// Covers: specs/world/objects.md §8.2
#[test]
fn corpse_always_drops_and_arms() {
    let t = tables();
    let seed = Seed::init_low(11);
    let (mut c, mut h) = setup(CORPSE, 2, seed);
    assert_eq!(operate(&mut c, &t, &mut h, &op(CORPSE, 14)).unwrap(), 1);
    assert_eq!(h.others(), s(&["drop 0 UnitId(10)"]));
    assert_eq!(mode_calls(&h), vec![(OBJ, 1)]);
    assert_eq!(h.f.flags(OBJ) & oflags::SELECTABLE, 0);
    assert_eq!(
        h.f.schedules(),
        vec![(OBJ, oevent::END_ANIM, 111), (OBJ, oevent::TRAP, 135)]
    );
    assert_eq!(c.seed, seed);
}

// Covers: specs/world/objects.md §8.2
#[test]
fn evil_urn_parm7_threshold() {
    let mut t = tables();
    t.objects[EVIL_URN as usize].parm7 = 100;
    for (want, seed) in [
        (true, find_seed(|s| s.roll(255) == 100)),
        (false, find_seed(|s| s.roll(255) == 101)),
    ] {
        let (mut c, mut h) = setup(EVIL_URN, 0, seed);
        h.drops.push_back(Some(2));
        operate(&mut c, &t, &mut h, &op(EVIL_URN, 68)).unwrap();
        assert_eq!(h.others().contains(&"region UnitId(10)".to_string()), want);
        assert_eq!(mode_calls(&h), vec![(OBJ, 1)]);
        let mut e = seed;
        e.roll(255);
        assert_eq!(c.seed, e);
    }
}

// ------------------------------------------------------------------ §8.3

// Covers: specs/world/objects.md §8.3
#[test]
fn trap_arm_bounds_and_monster_234_in_act_1() {
    let t = tables();
    let seed = Seed::init_low(1);
    // t = 0 and t ≥ 10: nothing.
    for it in [0u8, 0x80, 10, 0x7F] {
        let (mut c, mut h) = setup(CHEST, it, seed);
        trap_arm(&mut c, &t, &mut h, OBJ).unwrap();
        assert!(h.f.calls.is_empty(), "{it}");
    }
    // t = 1 (lock bit masked off).
    let (mut c, mut h) = setup(CHEST, 0x81, seed);
    trap_arm(&mut c, &t, &mut h, OBJ).unwrap();
    assert_eq!(
        h.f.calls,
        vec![
            Call::Schedule(OBJ, oevent::TRAP, 135),
            Call::Sound(OBJ, sound::TRAP_ARMED, None, false)
        ]
    );
    // t = 8, 9 with monster 234: nothing in act I, armed in act II.
    for ty in [8u8, 9] {
        for (level, armed) in [(2, false), (39, false), (40, true)] {
            let (mut c, mut h) = setup(CHEST, ty, seed);
            h.f.levels.insert(OBJ, level);
            h.trap_monster = Some(234);
            trap_arm(&mut c, &t, &mut h, OBJ).unwrap();
            assert_eq!(!h.f.schedules().is_empty(), armed, "{ty} {level}");
        }
        // Another monster in act I: armed.
        let (mut c, mut h) = setup(CHEST, ty, seed);
        h.trap_monster = Some(0);
        trap_arm(&mut c, &t, &mut h, OBJ).unwrap();
        assert_eq!(h.f.schedules(), vec![(OBJ, oevent::TRAP, 135)]);
    }
    assert_eq!(c.seed, seed);
}

// Covers: specs/world/objects.md §8.3, §edge-cases-original-bugs r12
#[test]
fn trap_handler_substitutions() {
    assert_eq!(trap_handler(0, 50), None);
    assert_eq!(trap_handler(10, 50), None);
    assert_eq!(trap_handler(9, 100), Some(9));
    assert_eq!(trap_handler(8, 74), Some(8));
    assert_eq!(trap_handler(8, 75), Some(2));
    assert_eq!(trap_handler(3, 39), Some(2));
    assert_eq!(trap_handler(3, 25), Some(3));
    assert_eq!(trap_handler(3, 40), Some(3));
    for ty in [1, 4] {
        assert_eq!(trap_handler(ty, 39), Some(2));
        assert_eq!(trap_handler(ty, 40), Some(ty));
    }
    for ty in [2, 5, 6, 7] {
        assert_eq!(trap_handler(ty, 1), Some(ty));
    }
}

// Covers: specs/world/objects.md §8.3
#[test]
fn trap_event_monsters_at_spot_or_free_spot() {
    let t = tables();
    let seed = Seed::init_low(1);
    for (ty, level, mon) in [
        (1u8, 50, 330),
        (1, 10, 326),
        (2, 10, 326),
        (6, 10, 326),
        (3, 50, 329),
        (3, 25, 329),
        (4, 50, 369),
        (0x84, 50, 369),
    ] {
        let (mut c, mut h) = setup(CHEST, ty, seed);
        h.f.levels.insert(OBJ, level);
        h.spot_ok = true;
        object_event_trap(&mut c, &t, &mut h);
        assert_eq!(
            h.others(),
            vec![format!("atspot {mon} RoomId(3) 40 50 0x88")],
            "{ty} {level}"
        );
        assert_eq!(c.seed, seed);
    }
    // Spot refused: free spot.
    let (mut c, mut h) = setup(CHEST, 1, seed);
    h.f.levels.insert(OBJ, 50);
    h.free = Some((RoomId(4), 41, 52));
    object_event_trap(&mut c, &t, &mut h);
    assert_eq!(
        h.others(),
        s(&[
            "atspot 330 RoomId(3) 40 50 0x88",
            "spawn 330 RoomId(4) 41 52"
        ])
    );
    // No free spot: nothing.
    let (mut c, mut h) = setup(CHEST, 1, seed);
    h.f.levels.insert(OBJ, 50);
    object_event_trap(&mut c, &t, &mut h);
    assert_eq!(h.others().len(), 1);
}

fn object_event_trap(c: &mut ObjectControl, t: &ObjectTables, h: &mut H) {
    trap_event(c, t, h, OBJ).unwrap();
}

// Covers: specs/world/objects.md §8.3
#[test]
fn trap_event_fire_objects() {
    let t = tables();
    let seed = Seed::init_low(1);
    for ty in [5u8, 7] {
        for inside in [true, false] {
            let (mut c, mut h) = setup(CHEST, ty, seed);
            h.in_room = inside;
            h.f.next_alloc = Some(60);
            c.data.insert(UnitId(60), ObjectData::default());
            c.data.insert(UnitId(61), ObjectData::default());
            object_event_trap(&mut c, &t, &mut h);
            let allocs = h.f.calls_of(|x| matches!(x, Call::Allocate(..)));
            let mut want = vec![Call::Allocate(ROOM, 162, 40, 50, 1)];
            if inside {
                want.push(Call::Allocate(ROOM, 160, 41, 50, 1));
            }
            assert_eq!(allocs, want);
            assert_eq!(c.get(UnitId(60)).unwrap().spark, 2);
            assert_eq!(c.get(UnitId(61)).unwrap().spark, u8::from(inside) * 2);
            assert_eq!(c.seed, seed);
        }
    }
}

// Covers: specs/world/objects.md §8.3
#[test]
fn trap_event_8_9_spawn_one_or_two_by_step() {
    let t = tables();
    for ty in [8u8, 9] {
        for bit in [0, 1] {
            let seed = find_seed(|s| s.step() & 1 == bit);
            let (mut c, mut h) = setup(CHEST, ty, seed);
            h.f.levels.insert(OBJ, 50);
            h.trap_monster = Some(274);
            object_event_trap(&mut c, &t, &mut h);
            let n = h
                .others()
                .iter()
                .filter(|x| x.starts_with("trapmon"))
                .count();
            assert_eq!(n as u32, 1 + bit);
            assert!(h.others().contains(&"trapmon 274 8 UnitId(10)".to_string()));
            let mut e = seed;
            e.step();
            assert_eq!(c.seed, e);
        }
    }
    // t = 8 at level ≥ 75 runs handler 2 (no step).
    let seed = Seed::init_low(1);
    let (mut c, mut h) = setup(CHEST, 8, seed);
    h.f.levels.insert(OBJ, 80);
    h.spot_ok = true;
    object_event_trap(&mut c, &t, &mut h);
    assert_eq!(h.others(), s(&["atspot 326 RoomId(3) 40 50 0x88"]));
    assert_eq!(c.seed, seed);
}

// ------------------------------------------------------------------ dispatch

// Covers: specs/world/objects.md §8.1 r7, §8.3
#[test]
fn chest_through_dispatch_opens_and_arms_trap() {
    let mut t = tables();
    t.objects[CHEST as usize].operatefn = 4;
    let seed = find_seed(|s| s.roll(100) >= 25);
    let (mut c, mut h) = setup(CHEST, 0x01, seed);
    h.drops.push_back(Some(2));
    let d = dispatch(&mut c, &t, &mut h, OBJ, Some(PLAYER)).unwrap();
    assert_eq!(d, Dispatch::Done(1));
    assert_eq!(h.drop_qs(), s(&["drop 0 UnitId(10)"]));
    assert_eq!(h.f.schedules(), vec![(OBJ, oevent::TRAP, 135)]);
}

// ------------------------------------------------------------------ §8 common

// Covers: specs/world/objects.md §8 text
#[test]
fn magic_test_is_item_type_4_quality_4_to_9() {
    let mut h = H::default();
    for q in 0..=12u8 {
        let i = UnitId(2000 + u32::from(q));
        h.types.insert(i, unit_type::ITEM);
        h.item_q.insert(i, q);
        assert_eq!(is_magic(&h, Some(i)), (4..=9).contains(&q), "quality {q}");
    }
    // Not an item (type 1, a monster) with a magic quality: not magic.
    let m = UnitId(3000);
    h.types.insert(m, 1);
    h.item_q.insert(m, 4);
    assert!(!is_magic(&h, Some(m)));
    // No item (the chest drop returned none).
    assert!(!is_magic(&h, None));
}

// Covers: specs/world/objects.md §8.1 r8, §edge-cases-original-bugs r28
#[test]
fn locked_chest_without_operator_is_fatal() {
    let t = tables();
    let (mut c, mut h) = setup(CHEST, 0x80, Seed::init_low(1));
    let mut o = op(CHEST, 4);
    o.operator = None;
    assert_eq!(
        operate(&mut c, &t, &mut h, &o),
        Err(ObjectError::KeyTestNoUnit)
    );
}
