//! Tests of the player handlers: their rules on a logging
//! [`PlayerWorld`] (sizes, result codes, effects and their order), the id
//! table against `client-messages.tsv`, and the action wiring's provider
//! ([`super::action::ActionPlayer`]) behind the dispatcher.

mod wired;

use std::collections::BTreeMap;

use d2_sim::skills::SkillEntry;
use d2_sim::units::{UnitId, UnitType};

use super::*;

const P: UnitId = UnitId(1);
const MERC: UnitId = UnitId(7);
const OTHER: UnitId = UnitId(9);

/// A logging world: answers from its fields, effects in `log`.
#[derive(Default)]
struct Fake {
    frame: i32,
    expansion: bool,
    difficulty: u8,
    modes: BTreeMap<UnitId, u32>,
    flags: BTreeMap<UnitId, u32>,
    stats: BTreeMap<(UnitId, u16), i32>,
    units: BTreeMap<(UnitType, u32), UnitId>,
    room_ok: bool,
    dead: bool,
    used_skill: bool,
    skill_count: u32,
    /// (skill, item) entries the player has.
    owned: Vec<(i32, u32)>,
    monstats: u32,
    text_test: bool,
    hardcore: bool,
    entries: Vec<SkillEntry>,
    passive: BTreeMap<i32, i32>,
    max: BTreeMap<u16, i32>,
    level: u32,
    busy: bool,
    trading: bool,
    staff: Option<u32>,
    hireling: Option<UnitId>,
    busy_flag: bool,
    switch: Option<(u32, bool)>,
    /// (player, the player it interacts with).
    interacting: Vec<(UnitId, UnitId)>,
    log: Vec<String>,
    /// What `send` queued, in order.
    sent: Vec<(UnitId, Vec<u8>)>,
    /// A test mod game takes `!` lines as its commands.
    commands: bool,
}

impl Fake {
    fn new() -> Self {
        Self {
            frame: 100,
            expansion: true,
            ..Self::default()
        }
    }
}

impl PlayerWorld for Fake {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, to: UnitId, bytes: &[u8]) {
        self.sent.push((to, bytes.to_vec()));
    }
    fn relation(&self, _a: UnitId, _b: UnitId, _flag: u32) -> bool {
        false
    }
    /// A test mod's command: a line starting with `!`.
    fn chat_command(&mut self, _p: UnitId, text: &[u8]) -> bool {
        self.commands && text.starts_with(b"!")
    }
    fn interacting_player(&self, u: UnitId) -> Option<UnitId> {
        self.interacting
            .iter()
            .find(|&&(a, _)| a == u)
            .map(|&(_, b)| b)
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn mode(&self, u: UnitId) -> u32 {
        self.modes.get(&u).copied().unwrap_or(1)
    }
    fn set_mode(&mut self, u: UnitId, mode: u32) {
        self.modes.insert(u, mode);
        self.log.push(format!("mode {} {mode}", u.0));
    }
    fn flags(&self, u: UnitId) -> u32 {
        self.flags.get(&u).copied().unwrap_or(0)
    }
    fn or_flags(&mut self, u: UnitId, bits: u32) {
        self.log.push(format!("flags {} |= {bits:#x}", u.0));
    }
    fn or_flags_ex(&mut self, u: UnitId, bits: u32) {
        self.log.push(format!("flags_ex {} |= {bits:#x}", u.0));
    }
    fn stat(&self, u: UnitId, stat: u16) -> i32 {
        self.stats.get(&(u, stat)).copied().unwrap_or(0)
    }
    fn set_state(&mut self, u: UnitId, state: u16, on: bool) {
        self.log.push(format!("state {} {state} {on}", u.0));
    }
    fn queue_update(&mut self, u: UnitId) {
        self.log.push(format!("queue {}", u.0));
    }
    fn schedule_event(&mut self, u: UnitId, event: u32, expire: i32) {
        self.log.push(format!("event {} {event} @{expire}", u.0));
    }
    fn sound(&mut self, u: UnitId, sound: u32, to: Option<UnitId>) {
        self.log
            .push(format!("sound {} {sound} {:?}", u.0, to.map(|t| t.0)));
    }
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.units.get(&(ty, guid)).copied()
    }
    fn room_in_list_of(&self, _: UnitId, _: UnitId) -> bool {
        self.room_ok
    }
    fn is_dead(&self, _: UnitId) -> bool {
        self.dead
    }
    fn has_used_skill(&self, _: UnitId) -> bool {
        self.used_skill
    }
    fn skill_count(&self) -> u32 {
        self.skill_count
    }
    fn has_skill(&self, _: UnitId, skill: i32, item: u32) -> bool {
        self.owned.contains(&(skill, item))
    }
    fn monstats_count(&self) -> u32 {
        self.monstats
    }
    fn overhead_text_test(&self, _: &[u8]) -> bool {
        self.text_test
    }
    fn replace_overhead(&mut self, u: UnitId, text: &[u8], byte8: u8, end: i32) {
        self.log.push(format!(
            "overhead {} {} {byte8} @{end}",
            u.0,
            String::from_utf8_lossy(text)
        ));
    }
    fn highlight_door(&mut self, p: UnitId, guid: u32) {
        self.log.push(format!("door {} {guid}", p.0));
    }
    fn hardcore(&self, _: UnitId) -> bool {
        self.hardcore
    }
    fn drop_client(&mut self, p: UnitId, reason: u32) {
        self.log.push(format!("drop {} {reason}", p.0));
    }
    fn skill_entries(&self, _: UnitId) -> Vec<SkillEntry> {
        self.entries.clone()
    }
    fn passive_state(&self, skill: i32) -> i32 {
        self.passive.get(&skill).copied().unwrap_or(0)
    }
    fn passive_state_apply(&mut self, u: UnitId, e: &SkillEntry) {
        self.log.push(format!("passive {} {}", u.0, e.skill));
    }
    fn stat_max(&self, _: UnitId, stat: u16) -> i32 {
        self.max.get(&stat).copied().unwrap_or(0)
    }
    fn set_stat_send(&mut self, u: UnitId, stat: u16, value: i32) {
        self.log.push(format!("stat {} {stat} = {value}", u.0));
    }
    fn current_level(&self, _: UnitId) -> u32 {
        self.level
    }
    fn warp(&mut self, p: UnitId, level: u32, arg: u32) {
        self.log.push(format!("warp {} {level} {arg}", p.0));
    }
    fn start_mode_skip_gate(&mut self, p: UnitId, mode: u32) {
        self.log.push(format!("start {} {mode}", p.0));
    }
    fn reselect_hand_skills(&mut self, p: UnitId) {
        self.log.push(format!("reselect {}", p.0));
    }
    fn busy(&self, _: UnitId) -> bool {
        self.busy
    }
    fn trading(&self, _: UnitId) -> bool {
        self.trading
    }
    fn staff_in_orifice(&mut self, p: UnitId, o: u32, i: u32, a: u16) -> Option<u32> {
        self.log.push(format!("staff {} {o} {i} {a}", p.0));
        self.staff
    }
    fn hireling(&self, _: UnitId) -> Option<UnitId> {
        self.hireling
    }
    fn replace_ai_commands(&mut self, m: UnitId, c: [i32; 5]) {
        self.log.push(format!("commands {} {c:?}", m.0));
    }
    fn busy_flag(&self, _: UnitId) -> bool {
        self.busy_flag
    }
    fn clear_busy_flag(&mut self, p: UnitId) {
        self.log.push(format!("unbusy {}", p.0));
    }
    fn clear_npc_intro(&mut self, p: UnitId, d: u8, class: u16) {
        self.log.push(format!("intro {} {d} {class}", p.0));
    }
    fn weapon_switch(&mut self, p: UnitId) -> Option<(u32, bool)> {
        self.log.push(format!("switch {}", p.0));
        self.switch
    }
}

fn go(w: &mut Fake, msg: &[u8]) -> Option<u32> {
    go_t(w, msg, 0)
}

fn go_t(w: &mut Fake, msg: &[u8], target: u32) -> Option<u32> {
    run(
        w,
        &Run {
            player: P,
            msg,
            target,
            clients: &[],
        },
    )
    .code
}

fn take(w: &mut Fake) -> Vec<String> {
    std::mem::take(&mut w.log)
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

// ---- sizes -------------------------------------------------------------------------------

/// Every handled id refuses a wrong size with 3 and does nothing
/// (each handler's "size N else 3").
#[test]
fn wrong_sizes_are_3() {
    let sizes = [
        (0x12, 1),
        (0x3D, 5),
        (0x3F, 3),
        (0x41, 1),
        (0x44, 17),
        (0x46, 13),
        (0x47, 13),
        (0x48, 1),
        (0x4B, 9),
        (0x4D, 3),
        (0x51, 9),
        (0x53, 1),
        (0x54, 1),
        (0x60, 1),
    ];
    for (id, n) in sizes {
        let mut w = Fake::new();
        w.modes.insert(P, MODE_DEAD);
        for len in [n - 1, n + 1] {
            let mut m = vec![0u8; len.max(1)];
            m[0] = id;
            if m.len() == n {
                continue;
            }
            assert_eq!(go(&mut w, &m), Some(3), "{id:#04x} len {len}");
        }
        assert!(w.log.is_empty(), "{id:#04x}: {:?}", w.log);
    }
    // 0x14 takes 4..=275.
    let mut w = Fake::new();
    assert_eq!(go(&mut w, &[0x14, 0, 0]), Some(3));
    assert_eq!(go(&mut w, &vec![0x14; 276]), Some(3));
}

// ---- 0x12, 0x14, 0x15, 0x3D, 0x3F --------------------------------------------------------------

#[test]
fn end_inferno_clears_state_12() {
    let mut w = Fake::new();
    assert_eq!(go(&mut w, &[0x12]), Some(0));
    assert_eq!(take(&mut w), ["state 1 12 false"]);
}

#[test]
fn overhead_chat_record_flags_and_event() {
    let mut w = Fake::new();
    // u8@1 type, u8@2 the byte +8, "hi\0", name "me\0".
    let m = [0x14, 7, 9, b'h', b'i', 0, b'm', b'e', 0];
    assert_eq!(go(&mut w, &m), Some(0));
    // d = 8·2 + 0x7D = 141; end = 100 + 141.
    assert_eq!(
        take(&mut w),
        [
            "overhead 1 hi 9 @241",
            "queue 1",
            "flags 1 |= 0x100",
            "event 1 6 @241"
        ]
    );
    // The duration caps at 254 characters.
    let mut m = vec![0x14, 0, 0];
    m.extend(std::iter::repeat_n(b'a', 255));
    m.push(0);
    assert_eq!(go(&mut w, &m), Some(0));
    let end = 100 + 8 * 254 + 0x7D;
    assert_eq!(take(&mut w).last().unwrap(), &format!("event 1 6 @{end}"));
    // n = 0 → 0, nothing; n ≥ 256 → 2.
    assert_eq!(go(&mut w, &[0x14, 0, 0, 0]), Some(0));
    let mut long = vec![0x14, 0, 0];
    long.extend(std::iter::repeat_n(b'a', 256));
    long.push(0);
    assert_eq!(go(&mut w, &long), Some(2));
    assert!(take(&mut w).is_empty());
    // The text test ≠ 0 → 0, nothing else.
    w.text_test = true;
    assert_eq!(go(&mut w, &m), Some(0));
    assert!(take(&mut w).is_empty());
}

#[test]
fn highlight_door_after_the_unit_test() {
    let mut w = Fake::new();
    let mut m = vec![0x3D];
    m.extend(le32(0x55));
    for code in [1, 2] {
        assert_eq!(go_t(&mut w, &m, code), Some(code));
    }
    assert!(take(&mut w).is_empty());
    assert_eq!(go(&mut w, &m), Some(0));
    assert_eq!(take(&mut w), ["door 1 85"]);
}

/// The unit test of 0x3D / 0x46 (range 10 / 50) and the point test of
/// 0x47 on staged positions.
#[test]
fn target_tests_with_their_ranges() {
    let at = |x, y| Pos { x, y };
    let near = UnitTarget::At {
        player: at(100, 100),
        target: at(110, 90),
    };
    let far = UnitTarget::At {
        player: at(100, 100),
        target: at(111, 100),
    };
    assert_eq!(unit_code(near, DOOR_RANGE), 0);
    assert_eq!(unit_code(far, DOOR_RANGE), 1);
    assert_eq!(unit_code(far, MERC_RANGE), 0);
    assert_eq!(unit_code(UnitTarget::Missing, DOOR_RANGE), 1);
    assert_eq!(unit_code(UnitTarget::OtherAct, DOOR_RANGE), 2);
    assert_eq!(unit_code(UnitTarget::OwnedItem, DOOR_RANGE), 0);
    assert_eq!(point_code(Some(at(100, 100)), at(150, 50), MERC_RANGE), 0);
    assert_eq!(point_code(Some(at(100, 100)), at(151, 100), MERC_RANGE), 1);
    assert_eq!(point_code(None, at(100, 100), MERC_RANGE), 1);
}

#[test]
fn play_audio_range_flag_and_sound() {
    let mut w = Fake::new();
    for s in [24u16, 33, 0, 0xFFFF] {
        let [a, b] = s.to_le_bytes();
        assert_eq!(go(&mut w, &[0x3F, a, b]), Some(0), "{s}");
    }
    assert!(take(&mut w).is_empty());
    assert_eq!(go(&mut w, &[0x3F, 25, 0]), Some(0));
    assert_eq!(go(&mut w, &[0x3F, 32, 0]), Some(0));
    assert_eq!(take(&mut w), ["sound 1 25 None", "sound 1 32 None"]);
    w.flags.insert(P, FLAG_NO_AUDIO);
    assert_eq!(go(&mut w, &[0x3F, 25, 0]), Some(1));
    assert!(take(&mut w).is_empty());
}

// ---- 0x41 ------------------------------------------------------------------------------

#[test]
fn resurrect_in_order() {
    let mut w = Fake::new();
    w.modes.insert(P, MODE_DEAD);
    let entry = |skill| SkillEntry {
        skill,
        base: 1,
        level_bonus: 0,
        owner_guid: -1,
        charges: 0,
        has_charges: false,
    };
    w.entries = vec![entry(8), entry(9), entry(10)];
    w.passive.insert(8, 30);
    w.passive.insert(10, -1);
    w.max = BTreeMap::from([
        (STAT_LIFE, 0x5000),
        (STAT_MANA, 0x2000),
        (STAT_STAMINA, 0x100),
    ]);
    // A player in Lut Gholein's act (level 50): town 40.
    w.level = 50;
    assert_eq!(go(&mut w, &[0x41]), Some(0));
    assert_eq!(
        take(&mut w),
        [
            "state 1 30 true",
            "passive 1 8",
            "stat 1 6 = 20480",
            "stat 1 8 = 8192",
            "stat 1 10 = 256",
            "flags 1 |= 0x2",
            "state 1 54 true",
            "state 1 54 false",
            "warp 1 40 0",
            "start 1 1",
            "reselect 1",
        ]
    );
    // Act V (level 109+): town 109; act I: town 1.
    for (level, town) in [(120, 109), (2, 1), (0, 1)] {
        w.level = level;
        go(&mut w, &[0x41]);
        let log = take(&mut w);
        assert!(
            log.contains(&format!("warp 1 {town} 0")),
            "{level}: {log:?}"
        );
    }
}

#[test]
fn resurrect_mode_and_hardcore() {
    let mut w = Fake::new();
    // Not dead (mode ≠ 0x11) → 0, nothing.
    w.modes.insert(P, 1);
    assert_eq!(go(&mut w, &[0x41]), Some(0));
    assert!(take(&mut w).is_empty());
    // Hardcore: the client is dropped (reason 3), nothing else.
    w.modes.insert(P, MODE_DEAD);
    w.hardcore = true;
    assert_eq!(go(&mut w, &[0x41]), Some(0));
    assert_eq!(take(&mut w), ["drop 1 3"]);
}

// ---- 0x44, 0x46, 0x47, 0x48 --------------------------------------------------------------------

fn staff_msg() -> Vec<u8> {
    let mut m = vec![0x44, 0, 0, 0, 0];
    m.extend(le32(0x11));
    m.extend(le32(0x22));
    m.extend(3u16.to_le_bytes());
    m.extend([0, 0]);
    m
}

#[test]
fn staff_entry() {
    let mut w = Fake::new();
    // Busy and trading → 3; one of them alone goes on.
    w.busy = true;
    w.trading = true;
    assert_eq!(go(&mut w, &staff_msg()), Some(3));
    assert!(take(&mut w).is_empty());
    w.trading = false;
    // No provider for the staff-in-orifice call: the id stays a stub.
    assert_eq!(go(&mut w, &staff_msg()), None);
    assert_eq!(take(&mut w), ["staff 1 17 34 3"]);
    w.staff = Some(1);
    assert_eq!(go(&mut w, &staff_msg()), Some(1));
}

fn merc_msg(id: u8, merc: u32, a: u32, b: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(le32(merc));
    m.extend(le32(a));
    m.extend(le32(b));
    m
}

#[test]
fn merc_interact_codes_and_command() {
    let mut w = Fake::new();
    w.units.insert((UnitType::Monster, 0x70), MERC);
    w.units.insert((UnitType::Monster, 0x90), OTHER);
    w.hireling = Some(MERC);
    // Type ≥ 6 → 2, before the unit test.
    assert_eq!(go_t(&mut w, &merc_msg(0x46, 0x70, 5, 6), 1), Some(2));
    // The unit test's code.
    assert_eq!(go_t(&mut w, &merc_msg(0x46, 0x70, 5, 1), 1), Some(1));
    // No monster with the merc GUID → 1; not the player's hireling → 1.
    assert_eq!(go(&mut w, &merc_msg(0x46, 0x71, 5, 1)), Some(1));
    assert_eq!(go(&mut w, &merc_msg(0x46, 0x90, 5, 1)), Some(1));
    assert!(take(&mut w).is_empty());
    assert_eq!(go(&mut w, &merc_msg(0x46, 0x70, 5, 1)), Some(0));
    assert_eq!(
        take(&mut w),
        ["commands 7 [12, 5, 1, 0, 0]", "sound 7 15 Some(1)"]
    );
}

#[test]
fn move_merc_returns_0_whatever_the_command_returns() {
    let mut w = Fake::new();
    w.units.insert((UnitType::Monster, 0x70), MERC);
    // x u16@5, y u16@9.
    let m = merc_msg(0x47, 0x70, 0xAAAA_0123, 0xBBBB_0456);
    assert_eq!(go_t(&mut w, &m, 1), Some(1));
    // No hireling: the command refuses (1), the handler returns 0.
    assert_eq!(go(&mut w, &m), Some(0));
    assert!(take(&mut w).is_empty());
    w.hireling = Some(MERC);
    assert_eq!(go(&mut w, &m), Some(0));
    assert_eq!(
        take(&mut w),
        ["commands 7 [13, 291, 1110, 0, 0]", "sound 7 15 Some(1)"]
    );
}

#[test]
fn turn_off_busy() {
    let mut w = Fake::new();
    assert_eq!(go(&mut w, &[0x48]), Some(2));
    assert!(take(&mut w).is_empty());
    w.busy_flag = true;
    assert_eq!(go(&mut w, &[0x48]), Some(0));
    assert_eq!(take(&mut w), ["unbusy 1"]);
}

// ---- 0x4B, 0x4D, 0x51, 0x53, 0x54, 0x60 --------------------------------------------------------

fn update_msg(ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![0x4B];
    m.extend(le32(ty));
    m.extend(le32(guid));
    m
}

#[test]
fn request_entity_update_codes() {
    let mut w = Fake::new();
    w.units.insert((UnitType::Player, 0x10), P);
    w.units.insert((UnitType::Player, 0x11), OTHER);
    w.units.insert((UnitType::Monster, 0x20), MERC);
    assert_eq!(go(&mut w, &update_msg(6, 0x20)), Some(2));
    // Type 0: another player, or none → 3.
    assert_eq!(go(&mut w, &update_msg(0, 0x11)), Some(3));
    assert_eq!(go(&mut w, &update_msg(0, 0x12)), Some(3));
    // Missing → 1; out of the room list → 1.
    assert_eq!(go(&mut w, &update_msg(1, 0x21)), Some(1));
    assert_eq!(go(&mut w, &update_msg(1, 0x20)), Some(1));
    assert!(take(&mut w).is_empty());
    w.room_ok = true;
    assert_eq!(go(&mut w, &update_msg(1, 0x20)), Some(0));
    assert_eq!(go(&mut w, &update_msg(0, 0x10)), Some(0));
    assert_eq!(
        take(&mut w),
        [
            "queue 7",
            "flags_ex 7 |= 0x10000",
            "queue 1",
            "flags_ex 1 |= 0x10000"
        ]
    );
}

#[test]
fn play_npc_message_bound_and_clear() {
    let mut w = Fake::new();
    w.monstats = 0x100;
    w.difficulty = 2;
    assert_eq!(go(&mut w, &[0x4D, 0x00, 0x01]), Some(2));
    assert!(take(&mut w).is_empty());
    assert_eq!(go(&mut w, &[0x4D, 0xFF, 0x00]), Some(0));
    assert_eq!(take(&mut w), ["intro 1 2 255"]);
}

fn hotkey_msg(skill: u16, left: bool, slot: u16, item: u32) -> Vec<u8> {
    let w = u32::from(skill) | if left { 0x8000 } else { 0 } | (u32::from(slot) << 16);
    let mut m = vec![0x51];
    m.extend(le32(w));
    m.extend(le32(item));
    m
}

#[test]
fn bind_hotkey_checks_and_slot() {
    let mut w = Fake::new();
    w.skill_count = 10;
    w.owned = vec![(4, u32::MAX)];
    let bind = |w: &mut Fake, m: &[u8]| {
        run(
            w,
            &Run {
                player: P,
                msg: m,
                target: 0,
                clients: &[],
            },
        )
    };
    // Slot > 15 → 3.
    assert_eq!(
        bind(&mut w, &hotkey_msg(4, false, 16, u32::MAX)).code,
        Some(3)
    );
    // skill = count → 3; a skill the player lacks → 3 (item is part of
    // the entry).
    assert_eq!(bind(&mut w, &hotkey_msg(10, false, 0, 0)).code, Some(3));
    assert_eq!(bind(&mut w, &hotkey_msg(4, false, 0, 0)).code, Some(3));
    assert_eq!(
        bind(&mut w, &hotkey_msg(5, false, 0, u32::MAX)).code,
        Some(3)
    );
    let o = bind(&mut w, &hotkey_msg(4, true, 15, u32::MAX));
    assert_eq!(o.code, Some(0));
    assert_eq!(
        o.hotkey,
        Some((
            15,
            HotKey {
                skill: 4,
                left: true,
                item: u32::MAX
            }
        ))
    );
    // skill > count → unbind (−1).
    let o = bind(&mut w, &hotkey_msg(11, false, 2, 7));
    assert_eq!(o.code, Some(0));
    assert_eq!(
        o.hotkey,
        Some((
            2,
            HotKey {
                skill: -1,
                left: false,
                item: 7
            }
        ))
    );
    assert!(w.log.is_empty(), "nothing is sent");
}

#[test]
fn stamina_on_off() {
    let mut w = Fake::new();
    // Walk without stamina: nothing.
    w.modes.insert(P, MODE_WALK);
    assert_eq!(go(&mut w, &[0x53]), Some(0));
    assert!(take(&mut w).is_empty());
    w.stats.insert((P, STAT_STAMINA), 1);
    assert_eq!(go(&mut w, &[0x53]), Some(0));
    assert_eq!(take(&mut w), ["mode 1 3"]);
    // Already running, or neutral: nothing.
    assert_eq!(go(&mut w, &[0x53]), Some(0));
    w.modes.insert(P, MODE_NEUTRAL);
    assert_eq!(go(&mut w, &[0x54]), Some(0));
    assert!(take(&mut w).is_empty());
    w.modes.insert(P, MODE_RUN);
    assert_eq!(go(&mut w, &[0x54]), Some(0));
    assert_eq!(take(&mut w), ["mode 1 2"]);
}

#[test]
fn swap_weapons_message_part() {
    let mut w = Fake::new();
    w.expansion = false;
    assert_eq!(go(&mut w, &[0x60]), Some(3));
    w.expansion = true;
    w.used_skill = true;
    assert_eq!(go(&mut w, &[0x60]), Some(0));
    w.used_skill = false;
    w.dead = true;
    assert_eq!(go(&mut w, &[0x60]), Some(0));
    assert!(take(&mut w).is_empty());
    w.dead = false;
    // No provider for the weapon switch: stub.
    assert_eq!(go(&mut w, &[0x60]), None);
    for (sw, code) in [((0, true), 3), ((0, false), 0), ((1, true), 0)] {
        w.switch = Some(sw);
        assert_eq!(go(&mut w, &[0x60]), Some(code), "{sw:?}");
    }
}

// ---- the id table ------------------------------------------------------------------------

const CLIENT_TSV: &str = include_str!("../../../../../../tables/sim/client-messages.tsv");

/// Rows of [`PLAYER_IDS`] that disagree with `client-messages.tsv`:
/// unsorted ids, a different name, or a handled id that is not a `sim`
/// `handler` row.
fn ids_vs_tsv(tsv: &str) -> Vec<String> {
    let rows: BTreeMap<u8, (String, String, String)> = tsv
        .lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let id = u8::from_str_radix(c.first()?.trim_start_matches("0x"), 16).ok()?;
            Some((
                id,
                (
                    c.get(1)?.to_string(),
                    c.get(6)?.to_string(),
                    c.get(9)?.to_string(),
                ),
            ))
        })
        .collect();
    let mut bad = Vec::new();
    for w in PLAYER_IDS.windows(2) {
        if w[0].0 >= w[1].0 {
            bad.push(format!("order {:#04x}", w[1].0));
        }
    }
    for &(id, name, _, status) in PLAYER_IDS {
        match rows.get(&id) {
            Some((n, k, s))
                if n == name && (status != Status::Handled || (k == "handler" && s == "sim")) => {}
            other => bad.push(format!("{id:#04x} {name}: {other:?}")),
        }
    }
    bad
}

#[test]
fn ids_match_client_tsv() {
    assert_eq!(ids_vs_tsv(CLIENT_TSV), Vec::<String>::new());
    // Perturbation (METHODS M08): a renamed row and a stubbed row are
    // reported, and nothing else.
    let renamed = CLIENT_TSV.replace("\tBindHotkey\t", "\tBindHotkeyX\t");
    let bad = ids_vs_tsv(&renamed);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].starts_with("0x51 BindHotkey"));
    let stubbed = CLIENT_TSV.replace("0x0054C590\thandler", "0x0054C590\tstub0");
    let bad = ids_vs_tsv(&stubbed);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].starts_with("0x48"));
}

/// Every id of these handlers is owned by exactly one handler module's table
/// (here, `world`, `skills`, `items`, or `walk` for 0x5F), and no id is
/// routed by two.
#[test]
fn every_section_9_id_has_one_owner() {
    use crate::adapters::handlers::{items, skills, walk, world};
    let ids = [
        0x12, 0x14, 0x15, 0x3D, 0x3E, 0x3F, 0x41, 0x44, 0x46, 0x47, 0x48, 0x4B, 0x4C, 0x4D, 0x51,
        0x53, 0x54, 0x59, 0x5F, 0x60,
    ];
    for id in ids {
        let routes = [
            handled(id),
            skills::handled(id),
            world::system(id).is_some(),
            items::ITEM_IDS.iter().any(|&(i, o)| i == id && o.is_some()),
            id == walk::RESYNC_ID,
        ];
        assert!(
            routes.iter().filter(|&&r| r).count() <= 1,
            "{id:#04x} routed twice"
        );
        let listed = PLAYER_IDS.iter().any(|r| r.0 == id)
            || world::WORLD_IDS.iter().any(|r| r.0 == id)
            || items::ITEM_IDS.iter().any(|r| r.0 == id)
            || id == walk::RESYNC_ID;
        assert!(listed, "{id:#04x} has no row");
    }
    // The pending owners stay stubs here.
    assert!(handled(0x15));
    assert!(!handled(0x59));
}

/// An open chat line a mod game takes as its command goes to no one; a
/// plain line, or any line under D2RUST's own rules, goes to every client.
#[test]
fn a_mod_command_line_goes_to_no_one() {
    let (p, q) = (UnitId(1), UnitId(2));
    let mut name = [0u8; 16];
    name[..2].copy_from_slice(b"Pa");
    let clients = [(p, name), (q, [0u8; 16])];
    let line = |text: &[u8]| {
        let mut m = vec![0x15, 1, 0];
        m.extend_from_slice(text);
        m.extend_from_slice(&[0, 0, 0]);
        m
    };
    let mut w = Fake::new();
    w.commands = true;
    super::chat(&mut w, p, &line(b"!where"), &clients);
    assert!(w.sent.is_empty(), "{:?}", w.sent);
    super::chat(&mut w, p, &line(b"hi"), &clients);
    assert_eq!(w.sent.iter().map(|s| s.0).collect::<Vec<_>>(), [p, q]);
    let mut w = Fake::new();
    super::chat(&mut w, p, &line(b"!where"), &clients);
    assert_eq!(w.sent.len(), 2);
}

#[test]
fn chat_line_layout() {
    // The layout of a plain chat line (form 1).
    let m = super::chat_line(1, 0, 30, b"TestSor", b"hi");
    assert_eq!(
        m,
        [&[0x26, 1, 0, 2, 0, 0, 0, 0, 0, 30][..], b"TestSor\0hi\0"].concat()
    );
}
