// Spec: specs/world/objects.md §9 (Test vectors, Edge cases)
//!
//! [`ShrineWorld`] for the shared [`Fake`]: extra fake state lives in
//! `Fake::stats` under pseudo-stat keys ≥ 0x1000 (below), and every call
//! is recorded as `Call::Other`.

use std::collections::BTreeMap;

use super::super::fake::{blank_level, blank_object, blank_shrine, Call, Fake};
use super::super::{dispatch, object_event, Dispatch, ObjectData, Operator};
use super::*;
use crate::rng::Seed;

/// Text length of the object's hover string: present → `create_hover`
/// succeeds.
const K_HOVER_LEN: u16 = 0x1000;
/// The hover's expiry frame.
const K_HOVER_EXP: u16 = 0x1001;
const K_TO_HIT: u16 = 0x1002;
const K_TWO_HANDED: u16 = 0x1003;
const K_MAX_LIFE: u16 = 0x1004;
const K_MAX_MANA: u16 = 0x1005;
const K_MAX_STAMINA: u16 = 0x1006;
const K_LEVEL: u16 = 0x1007;
const K_SPOT_X: u16 = 0x1008;
const K_SPOT_Y: u16 = 0x1009;
/// Gem i: item id at 0x2000 + i, better code (LE) at 0x2100 + i.
const K_GEM: u16 = 0x2000;
const K_GEM_BETTER: u16 = 0x2100;
/// Unit i in range of the centre at 0x3000 + i.
const K_RANGE: u16 = 0x3000;
/// Next state list id.
const K_NEXT_LIST: u16 = 0x1010;
/// Present on P: `drop_near_player` finds no free spot.
const K_NO_SPOT: u16 = 0x1011;

fn code_str(c: [u8; 4]) -> String {
    String::from_utf8_lossy(&c).trim_end().to_string()
}

fn other(f: &mut Fake, s: String) {
    f.calls.push(Call::Other(s));
}

impl ShrineWorld for Fake {
    fn create_hover(&mut self, obj: UnitId, string_id: u32) -> bool {
        other(self, format!("create_hover {} {string_id}", obj.0));
        let Some(&len) = self.stats.get(&(obj, K_HOVER_LEN)) else {
            return false;
        };
        let e = self.frame + hover_lifetime(len as u32);
        self.stats.insert((obj, K_HOVER_EXP), e);
        true
    }
    fn hover_expiry(&self, obj: UnitId) -> Option<i32> {
        self.stats.get(&(obj, K_HOVER_EXP)).copied()
    }
    fn free_hover(&mut self, obj: UnitId) {
        other(self, format!("free_hover {}", obj.0));
        self.stats.remove(&(obj, K_HOVER_EXP));
    }
    fn stat(&self, unit: UnitId, id: u16) -> i32 {
        self.stats.get(&(unit, id)).copied().unwrap_or(0)
    }
    fn set_stat(&mut self, unit: UnitId, id: u16, value: i32) {
        other(self, format!("set {} {id} {value}", unit.0));
        self.stats.insert((unit, id), value);
    }
    fn add_base_stat(&mut self, unit: UnitId, id: u16, delta: i32) {
        other(self, format!("add {} {id} {delta}", unit.0));
        *self.stats.entry((unit, id)).or_default() += delta;
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.stat(unit, K_MAX_LIFE)
    }
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.stat(unit, K_MAX_MANA)
    }
    fn max_stamina(&self, unit: UnitId) -> i32 {
        self.stat(unit, K_MAX_STAMINA)
    }
    fn apply_state(&mut self, r: StateRequest) -> Option<StateList> {
        let id = self
            .stats
            .get(&(UnitId(0), K_NEXT_LIST))
            .copied()
            .unwrap_or(1);
        self.stats.insert((UnitId(0), K_NEXT_LIST), id + 1);
        other(
            self,
            format!(
                "state {} {} dur {} state {} stat {} v {} {:?} -> {id}",
                r.target.0, r.source.0, r.duration, r.state, r.stat, r.value, r.remove
            ),
        );
        Some(StateList(id as u32))
    }
    fn set_list_stat(&mut self, list: StateList, id: u16, value: i32) {
        other(self, format!("list {} {id} {value}", list.0));
    }
    fn refresh_skills(&mut self, unit: UnitId) {
        other(self, format!("refresh {}", unit.0));
    }
    fn to_hit(&mut self, player: UnitId) -> i32 {
        self.stat(player, K_TO_HIT)
    }
    fn two_handed(&self, player: UnitId) -> bool {
        self.stat(player, K_TWO_HANDED) != 0
    }
    fn backpack_gems(&self, player: UnitId) -> Vec<Gem> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(&item) = self.stats.get(&(player, K_GEM + i)) {
            let b = self.stat(player, K_GEM_BETTER + i);
            out.push(Gem {
                item: UnitId(item as u32),
                better: b.to_le_bytes(),
            });
            i += 1;
        }
        out
    }
    fn remove_item(&mut self, player: UnitId, item: UnitId) {
        other(self, format!("remove {} {}", player.0, item.0));
    }
    fn drop_near_player(&mut self, player: UnitId, code: [u8; 4]) -> bool {
        other(self, format!("drop {} {}", player.0, code_str(code)));
        !self.stats.contains_key(&(player, K_NO_SPOT))
    }
    fn drop_potion_near_player(&mut self, player: UnitId, code: [u8; 4], quantity: i32) {
        other(
            self,
            format!("potion {} {} {quantity}", player.0, code_str(code)),
        );
    }
    fn units_in_range(&mut self, center: UnitId, _range: i32) -> Vec<UnitId> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(&u) = self.stats.get(&(center, K_RANGE + i)) {
            out.push(UnitId(u as u32));
            i += 1;
        }
        out
    }
    fn create_missile(&mut self, m: MissileRequest) {
        other(
            self,
            format!(
                "missile {} owner {} from {} ({}, {}) lvl {} flags {:#x}",
                m.class, m.owner.0, m.from.0, m.offset.0, m.offset.1, m.skill_level, m.flags
            ),
        );
    }
    fn player_level(&self, player: UnitId) -> i32 {
        self.stat(player, K_LEVEL)
    }
    fn free_spot(&mut self, unit: UnitId, size: i32, mask: u32) -> Option<(i32, i32)> {
        other(self, format!("spot {} {size} {mask:#x}", unit.0));
        let x = *self.stats.get(&(unit, K_SPOT_X))?;
        Some((x, self.stat(unit, K_SPOT_Y)))
    }
    fn create_portal(&mut self, player: UnitId, x: i32, y: i32, class: u16, dest: u32) {
        other(
            self,
            format!("portal {} ({x}, {y}) class {class} to {dest}", player.0),
        );
    }
    fn make_nearest_unique(&mut self, player: UnitId) {
        other(self, format!("unique {}", player.0));
    }
    fn reverse_player_name(&mut self, player: UnitId) {
        other(self, format!("reverse {}", player.0));
    }
}

// ------------------------------------------------------------------ setup

const OBJ: UnitId = UnitId(10);
const P: UnitId = UnitId(20);
const P_GUID: u32 = 77;
const CLASS: u16 = 2;
const SHRINE: u16 = 1;

struct Setup {
    ctl: ObjectControl,
    t: ObjectTables,
    w: Fake,
}

/// Object class 2 (operate 2, `SubClass` 1, `Mode2` 1, `CycleAnim1` 0,
/// `Sync` 1, `FrameCnt1` 10 · 256), shrine row 1 with `code`, frame 1000,
/// the object in mode 0 in level 2 (act 0), P a player with GUID 77.
fn setup(code: u8) -> Setup {
    let mut o = blank_object();
    o.operatefn = 2;
    o.subclass = 1;
    o.mode2 = 1;
    o.sync = 1;
    o.framecnt1 = 10 << 8;
    let mut s = blank_shrine();
    s.code = code;
    s.duration_in_frames = 2400;
    let t = ObjectTables {
        objects: vec![blank_object(), blank_object(), o],
        shrines: vec![blank_shrine(), s],
        levels: vec![blank_level(), blank_level(), blank_level()],
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    };
    let mut ctl = ObjectControl {
        seed: Seed::init(),
        regions: Vec::new(),
        shrine_lists: Default::default(),
        data: BTreeMap::new(),
    };
    ctl.data.insert(
        OBJ,
        ObjectData {
            guid: 5,
            class: CLASS,
            interact: SHRINE as u8,
            shrine: Some(SHRINE),
            ..ObjectData::default()
        },
    );
    let mut w = Fake {
        frame: 1000,
        ..Fake::default()
    };
    w.guids.insert(OBJ, 5);
    w.guids.insert(P, P_GUID);
    w.operators.insert(P, Operator::Player(0));
    w.levels.insert(P, 2);
    w.levels.insert(OBJ, 2);
    Setup { ctl, t, w }
}

impl Setup {
    fn shrine(&mut self) -> &mut d2_data::tables::Shrines {
        &mut self.t.shrines[SHRINE as usize]
    }
    fn op(&mut self, operator: Option<UnitId>) -> i32 {
        let op = Operate {
            object: OBJ,
            operator,
            class: CLASS,
            operate_fn: 2,
        };
        operate(&mut self.ctl, &self.t, &mut self.w, &op).unwrap()
    }
    fn others(&self) -> Vec<String> {
        self.w
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Other(s)
                    if !s.starts_with("create_hover") && !s.starts_with("free_hover") =>
                {
                    Some(s.clone())
                }
                _ => None,
            })
            .collect()
    }
}

// ------------------------------------------------------------------ §9.1

// Covers: specs/world/objects.md §9.1 text, §9.1 r2, §9.1 r3, §9.1 r4, §9.1 r5, §9.1 r6
#[test]
fn operate_full_sequence() {
    let mut s = setup(2);
    s.shrine().reset_time_in_minutes = 2;
    s.w.stats.insert((OBJ, K_HOVER_LEN), 10);
    s.w.stats.insert((P, K_MAX_LIFE), 500 << 8);
    assert_eq!(s.op(Some(P)), 1);
    assert_eq!(s.ctl.data[&OBJ].operator, P_GUID + 1);
    assert_eq!(s.w.modes[&OBJ], 1);
    assert_eq!(
        s.w.flags[&OBJ] & (oflags::CHANGED | oflags::HOVER_FREED),
        oflags::CHANGED | oflags::HOVER_FREED
    );
    let mine: Vec<Call> = s.w.calls_of(|c| !matches!(c, Call::Anim(..)));
    assert_eq!(
        mine,
        vec![
            Call::Queue(OBJ),
            Call::Mode(OBJ, 1, true),
            Call::Other("free_hover 10".into()),
            Call::Other(format!("create_hover 10 {}", 3683 + 1)),
            Call::Queue(OBJ),
            Call::Schedule(OBJ, oevent::HOVER, 1300),
            Call::Other(format!("add 20 6 {}", 500 << 8)),
            Call::Schedule(OBJ, oevent::SHRINE_RESET, 1000 + 2400 + 1),
            Call::Schedule(OBJ, oevent::END_ANIM, 1000 + 10 + 1),
        ]
    );
    // The hover lifetime: 8 · 10 + 125 frames.
    assert_eq!(s.w.hover_expiry(OBJ), Some(1000 + 205));
}

// Covers: specs/world/objects.md §9.1 r1
#[test]
fn operate_refusals() {
    let mut s = setup(2);
    s.ctl.data.get_mut(&OBJ).unwrap().operator = 1;
    assert_eq!(s.op(Some(P)), 0);
    assert!(s.w.calls.is_empty());

    let mut s = setup(2);
    s.w.modes.insert(OBJ, 1);
    assert_eq!(s.op(Some(P)), 0);
    assert!(s.w.calls.is_empty());
    assert_eq!(s.ctl.data[&OBJ].operator, 0);
}

// Covers: specs/world/objects.md §9.1 r2, §9.1 r4
#[test]
fn operate_without_operator_keeps_field_zero_and_runs_no_effect() {
    // §9.1 "No guards": rules 1–3 run, then the effect reads the missing
    // operator (fatal; unreachable with live data).
    let mut s = setup(2);
    s.w.stats.insert((P, K_MAX_LIFE), 100);
    let op = Operate {
        object: OBJ,
        operator: None,
        class: CLASS,
        operate_fn: 2,
    };
    assert_eq!(
        operate(&mut s.ctl, &s.t, &mut s.w, &op),
        Err(ObjectError::ShrineNoOperator)
    );
    assert_eq!(s.ctl.data[&OBJ].operator, 0);
    assert_eq!(s.w.modes[&OBJ], 1);
    assert!(s.others().is_empty());
    // Mode 1 now refuses a second use (rule 1).
    assert_eq!(s.op(Some(P)), 0);
}

// Covers: specs/world/objects.md §9.1 r3, §9.1 r5, §9.1 r6
#[test]
fn operate_no_hover_no_reset_no_endanim() {
    // Hover not created, reset 0, CycleAnim1 ≠ 0.
    let mut s = setup(2);
    s.t.objects[CLASS as usize].cycleanim1 = 1;
    assert_eq!(s.op(Some(P)), 1);
    assert!(s.w.schedules().is_empty());
    assert_eq!(s.w.flags[&OBJ] & oflags::HOVER_FREED, 0);

    // Mode2 = 0: no ENDANIM.
    let mut s = setup(2);
    s.t.objects[CLASS as usize].mode2 = 0;
    s.op(Some(P));
    assert!(s.w.schedules().is_empty());
}

// Covers: specs/world/objects.md §9.1 r2, §4 r3
#[test]
fn operate_mode_draw_on_unit_seed() {
    let mut s = setup(2);
    s.t.objects[CLASS as usize].sync = 0;
    s.t.objects[CLASS as usize].framedelta1 = 256;
    s.w.seeds.insert(OBJ, Seed::new(12345, 666));
    s.op(Some(P));
    // §4 test vector: roll(32) = 23, speed 263.
    assert!(s.w.calls.contains(&Call::Anim(OBJ, 10 << 8, 0, 263)));
}

// Covers: specs/world/objects.md §9.1 text, §7.2 r4
#[test]
fn dispatch_runs_operate_2() {
    let mut s = setup(3);
    s.w.stats.insert((P, K_MAX_MANA), 300);
    let r = dispatch(&mut s.ctl, &s.t, &mut s.w, OBJ, Some(P)).unwrap();
    assert_eq!(r, Dispatch::Done(1));
    assert_eq!(s.others(), vec!["add 20 8 300".to_string()]);
}

// Covers: specs/world/objects.md §9.1 text
#[test]
fn reset_event_needs_subclass_bit_0() {
    let mut s = setup(2);
    s.w.modes.insert(OBJ, 1);
    s.ctl.data.get_mut(&OBJ).unwrap().operator = P_GUID + 1;
    object_event(&mut s.ctl, &s.t, &mut s.w, OBJ, oevent::SHRINE_RESET).unwrap();
    assert_eq!(s.w.modes[&OBJ], 0);
    assert_eq!(s.ctl.data[&OBJ].operator, 0);
    assert_eq!(s.w.calls[0], Call::Queue(OBJ));
    assert_eq!(s.w.calls[1], Call::Mode(OBJ, 0, true));
    // The shrine can be used again.
    assert_eq!(s.op(Some(P)), 1);

    let mut s = setup(2);
    s.t.objects[CLASS as usize].subclass = 2;
    s.w.modes.insert(OBJ, 1);
    s.ctl.data.get_mut(&OBJ).unwrap().operator = P_GUID + 1;
    object_event(&mut s.ctl, &s.t, &mut s.w, OBJ, oevent::SHRINE_RESET).unwrap();
    assert_eq!(s.w.modes[&OBJ], 1);
    assert_eq!(s.ctl.data[&OBJ].operator, P_GUID + 1);
    assert!(s.w.calls.is_empty());
}

// Covers: specs/world/objects.md §9.1 text; specs/world/objects-2.md §24 r4
#[test]
fn hover_event_frees_or_reschedules() {
    // Expired (expiry = frame): freed, queued, flag 0x100.
    let mut s = setup(2);
    s.w.stats.insert((OBJ, K_HOVER_EXP), 1000);
    object_event(&mut s.ctl, &s.t, &mut s.w, OBJ, oevent::HOVER).unwrap();
    assert_eq!(s.w.hover_expiry(OBJ), None);
    assert_eq!(s.w.flags[&OBJ] & oflags::HOVER_FREED, oflags::HOVER_FREED);
    assert!(s.w.calls.contains(&Call::Queue(OBJ)));
    assert!(s.w.schedules().is_empty());

    // Not expired: rescheduled at the expiry.
    let mut s = setup(2);
    s.w.stats.insert((OBJ, K_HOVER_EXP), 1001);
    object_event(&mut s.ctl, &s.t, &mut s.w, OBJ, oevent::HOVER).unwrap();
    assert_eq!(s.w.hover_expiry(OBJ), Some(1001));
    assert_eq!(s.w.calls, vec![Call::Schedule(OBJ, oevent::HOVER, 1001)]);

    // No hover: nothing.
    let mut s = setup(2);
    object_event(&mut s.ctl, &s.t, &mut s.w, OBJ, oevent::HOVER).unwrap();
    assert!(s.w.calls.is_empty());
}

// ------------------------------------------------------------------ §9.2

fn run(code: u8, f: impl FnOnce(&mut Setup)) -> Vec<String> {
    let mut s = setup(code);
    f(&mut s);
    assert_eq!(s.op(Some(P)), 1);
    s.others()
}

// Covers: specs/world/objects.md §9.2, §9.1 r4
#[test]
fn refill_codes_0_1_23_and_out_of_table() {
    for code in [0, 1, 23, 24, 200] {
        let got = run(code, |s| {
            s.w.stats.insert((P, stat::LIFE), 100);
            s.w.stats.insert((P, K_MAX_LIFE), 400);
            s.w.stats.insert((P, stat::MANA), 50);
            s.w.stats.insert((P, K_MAX_MANA), 60);
        });
        assert_eq!(got, vec!["add 20 6 300", "add 20 8 10"], "code {code}");
    }
}

// Covers: specs/world/objects.md §9.2, §edge-cases-original-bugs r5
#[test]
fn health_and_mana_shrines_only_fill() {
    let got = run(2, |s| {
        s.w.stats.insert((P, stat::LIFE), 100);
        s.w.stats.insert((P, K_MAX_LIFE), 400);
    });
    // Base-stat adds of max − total (§9.2).
    assert_eq!(got, vec!["add 20 6 300"]);
    let got = run(3, |s| {
        s.w.stats.insert((P, stat::MANA), 0);
        s.w.stats.insert((P, K_MAX_MANA), 256);
    });
    assert_eq!(got, vec!["add 20 8 256"]);
}

// Covers: specs/world/objects.md §9.2
#[test]
fn life_to_mana_code_4() {
    // d = 30 · (1000·256 >> 8) / 100 = 300; life −= 300·256; mana +=
    // 150 · 300 · 256 / 100 = 115200.
    let got = run(4, |s| {
        s.shrine().arg0 = 30;
        s.shrine().arg1 = 150;
        s.w.stats.insert((P, stat::LIFE), 1000 << 8);
        s.w.stats.insert((P, stat::MANA), 7);
    });
    assert_eq!(
        got,
        vec![
            format!("add 20 6 {}", -300 * 256),
            format!("add 20 8 {}", 115_200),
        ]
    );
}

// Covers: specs/world/objects.md §9.2
#[test]
fn mana_to_life_code_5() {
    // d = 30 · 999 / 100 = 299 (toward 0); life += 150 · 299 / 100 = 448.
    let got = run(5, |s| {
        s.shrine().arg0 = 30;
        s.shrine().arg1 = 150;
        s.w.stats.insert((P, stat::MANA), 999);
        s.w.stats.insert((P, stat::LIFE), 10);
    });
    assert_eq!(got, vec!["add 20 8 -299", "add 20 6 448"]);
}

// Covers: specs/world/objects.md §9.2
#[test]
fn generic_state_codes() {
    for (code, st, sv) in [
        (6, 171, 128),
        (8, 39, 131),
        (9, 43, 132),
        (10, 41, 130),
        (11, 45, 133),
        (13, 27, 135),
        (15, 85, 137),
    ] {
        let got = run(code, |s| s.shrine().arg0 = 75);
        assert_eq!(
            got,
            vec![format!(
                "state 20 10 dur 2400 state {sv} stat {st} v 75 Default -> 1"
            )],
            "code {code}"
        );
    }
}

// Covers: specs/world/objects.md §9.2
#[test]
fn combat_shrine_code_7() {
    // v = V(19, 200) = 200 · 333 / 100 = 666.
    let got = run(7, |s| {
        s.shrine().arg0 = 200;
        s.shrine().arg1 = 50;
        s.w.stats.insert((P, K_TO_HIT), 333);
    });
    assert_eq!(
        got,
        vec![
            "state 20 10 dur 2400 state 129 stat 25 v 50 Default -> 1".to_string(),
            "list 1 19 666".to_string(),
        ]
    );
}

// Covers: specs/world/objects.md §9.2
#[test]
fn skill_shrine_code_12() {
    let got = run(12, |s| s.shrine().arg0 = 2);
    assert_eq!(
        got,
        vec![
            "state 20 10 dur 2400 state 134 stat 11 v 0 Skill -> 1".to_string(),
            "refresh 20".to_string(),
        ]
    );
}

// Covers: specs/world/objects.md §9.2, §edge-cases-original-bugs r6
#[test]
fn stamina_shrine_code_14() {
    // Without a stamina bonus V(162, 200) = 0: stamina 0 and stat 162 0 on
    // the list; only the refill and the 1000 recovery act.
    let got = run(14, |s| {
        s.shrine().arg0 = 200;
        s.w.stats.insert((P, K_MAX_STAMINA), 90 << 8);
    });
    assert_eq!(
        got,
        vec![
            format!("set 20 10 {}", 90 << 8),
            "state 20 10 dur 2400 state 136 stat 162 v 0 Stamina -> 1".to_string(),
            "list 1 10 0".to_string(),
            "list 1 28 1000".to_string(),
        ]
    );
    // With stat 162 = 25: v = 50, stamina on the list 100.
    let got = run(14, |s| {
        s.shrine().arg0 = 200;
        s.w.stats.insert((P, stat::STAMINA_BONUS), 25);
    });
    assert_eq!(
        got[1],
        "state 20 10 dur 2400 state 136 stat 162 v 50 Stamina -> 1"
    );
    assert_eq!(got[2], "list 1 10 100");
}

// Covers: specs/world/objects.md §9.2
#[test]
fn value_function() {
    let mut w = Fake::default();
    // Listed stats give a.
    for st in [11, 27, 39, 41, 43, 45, 85, 171] {
        assert_eq!(value(&mut w, P, st, 75), 75);
    }
    w.stats.insert((P, K_TO_HIT), 150);
    assert_eq!(value(&mut w, P, 19, 33), 49);
    w.stats.insert((P, 21), 10);
    w.stats.insert((P, 23), 40);
    assert_eq!(value(&mut w, P, 21, 50), 5);
    w.stats.insert((P, K_TWO_HANDED), 1);
    assert_eq!(value(&mut w, P, 21, 50), 20);
    // Other: signed, toward zero.
    w.stats.insert((P, 31), -7);
    assert_eq!(value(&mut w, P, 31, 50), -3);
}

// Covers: specs/world/objects.md §9.2, §edge-cases-original-bugs r26
#[test]
fn code_16_and_20() {
    assert_eq!(run(16, |_| {}), vec!["reverse 20"]);
    assert_eq!(run(20, |_| {}), vec!["unique 20"]);
}

// Covers: specs/world/objects.md §9.2, §edge-cases-original-bugs r27
#[test]
fn portal_shrine_code_17() {
    let got = run(17, |s| {
        s.w.stats.insert((P, K_SPOT_X), 100);
        s.w.stats.insert((P, K_SPOT_Y), 200);
        s.t.levels[2].act = 1;
    });
    assert_eq!(
        got,
        vec![
            "spot 20 3 0x1c09".to_string(),
            "portal 20 (105, 205) class 59 to 40".to_string(),
        ]
    );
    // No free spot: nothing created.
    assert_eq!(run(17, |_| {}), vec!["spot 20 3 0x1c09"]);
}

// Covers: specs/world/objects.md §9.2, §edge-cases-original-bugs r27
#[test]
fn portal_shrine_in_town_plays_the_refusal_sound() {
    let mut s = setup(17);
    s.w.stats.insert((P, K_SPOT_X), 100);
    s.w.stats.insert((P, K_SPOT_Y), 200);
    s.w.levels.insert(P, 40);
    assert_eq!(s.op(Some(P)), 1);
    // No portal; sound event 0x18 on P, targeted at P, queued (not sent now).
    assert_eq!(s.others(), vec!["spot 20 3 0x1c09"]);
    assert!(s
        .w
        .calls
        .iter()
        .any(|c| matches!(c, Call::Sound(u, 0x18, Some(t), false) if *u == P && *t == P)));
}

// ------------------------------------------------------------------ §9.3

// Covers: specs/world/objects.md §9.3
#[test]
fn gem_shrine_upgrades_first_upgradable_gem() {
    let got = run(18, |s| {
        s.w.stats.insert((P, K_GEM), 50);
        s.w.stats
            .insert((P, K_GEM_BETTER), i32::from_le_bytes(NO_GEM));
        s.w.stats.insert((P, K_GEM + 1), 51);
        s.w.stats
            .insert((P, K_GEM_BETTER + 1), i32::from_le_bytes(*b"gfr "));
        s.w.stats.insert((P, K_GEM + 2), 52);
        s.w.stats
            .insert((P, K_GEM_BETTER + 2), i32::from_le_bytes(*b"gsv "));
        s.w.seeds.insert(P, Seed::new(1, 666));
    });
    assert_eq!(got, vec!["drop 20 gfr", "remove 20 51"]);
}

// Covers: specs/world/objects.md §9.3
#[test]
fn gem_shrine_failed_drop_keeps_gem_and_walks_on() {
    let got = run(18, |s| {
        s.w.stats.insert((P, K_GEM), 51);
        s.w.stats
            .insert((P, K_GEM_BETTER), i32::from_le_bytes(*b"gfr "));
        s.w.stats.insert((P, K_GEM + 1), 52);
        s.w.stats
            .insert((P, K_GEM_BETTER + 1), i32::from_le_bytes(*b"gsv "));
        s.w.stats.insert((P, K_NO_SPOT), 1);
        s.w.seeds.insert(P, Seed::new(1, 666));
    });
    // No gem removed; both upgrades tried, then the chipped fallback.
    assert_eq!(got.len(), 3);
    assert_eq!(got[..2], ["drop 20 gfr", "drop 20 gsv"]);
    assert!(got[2].starts_with("drop 20 gc"));
}

// Covers: specs/world/objects.md §9.3
#[test]
fn gem_shrine_without_gem_rolls_on_player_seed() {
    let p_seed = Seed::new(4321, 666);
    let u_seed = Seed::new(999, 666);
    let mut s = setup(18);
    s.w.stats.insert((P, K_GEM), 50);
    s.w.stats
        .insert((P, K_GEM_BETTER), i32::from_le_bytes(NO_GEM));
    s.w.seeds.insert(P, p_seed);
    s.w.seeds.insert(OBJ, u_seed);
    s.op(Some(P));
    let mut want = p_seed;
    let r = want.roll(6) as usize;
    assert_eq!(
        s.others(),
        vec![format!(
            "drop 20 {}",
            ["gcw", "gcr", "gcg", "gcb", "gcy", "gcv"][r]
        )]
    );
    assert_eq!(s.w.seeds[&P], want);
    // The shrine's seed is not drawn (Sync 1).
    assert_eq!(s.w.seeds[&OBJ], u_seed);
}

// Covers: specs/world/objects.md §9.3; specs/world/objects-2.md §24 r3
#[test]
fn storm_shrine() {
    let got = run(19, |s| {
        s.shrine().arg0 = 50;
        s.shrine().arg1 = 10;
        s.w.stats.insert((OBJ, K_RANGE), 30);
        s.w.stats.insert((OBJ, K_RANGE + 1), 31);
        s.w.stats.insert((UnitId(30), stat::LIFE), (101 << 8) + 7);
        s.w.stats.insert((UnitId(31), stat::LIFE), 255);
        s.w.stats.insert((P, K_LEVEL), 47);
    });
    // 30: (101 · 50 / 100) · 256 = 50 · 256, a base-stat add; 31: life >>
    // 8 = 0, a 0 add writes nothing.
    assert_eq!(got[0], format!("add 30 6 {}", -50 * 256));
    let missiles: Vec<&String> = got[1..].iter().collect();
    assert_eq!(missiles.len(), 16);
    // Level 47 / 5 = 9 → 8.
    assert_eq!(
        missiles[0],
        "missile 62 owner 20 from 10 (5, 5) lvl 8 flags 0x3"
    );
    assert_eq!(
        missiles[1],
        "missile 62 owner 20 from 10 (5, -10) lvl 8 flags 0x3"
    );
    assert_eq!(
        missiles[15],
        "missile 62 owner 20 from 10 (-20, -20) lvl 8 flags 0x3"
    );
    assert_eq!(missile_level(0), 1);
    assert_eq!(missile_level(4), 1);
    assert_eq!(missile_level(10), 2);
}

// Covers: specs/world/objects.md §9.3, §9.1 r2
#[test]
fn exploding_and_poison_shrines_roll_on_shrine_seed_after_mode_draw() {
    for (code, potion, missile) in [(21u8, "opm", 45), (22, "gpm", 48)] {
        let u_seed = Seed::new(12345, 666);
        let mut s = setup(code);
        s.t.objects[CLASS as usize].sync = 0;
        s.t.objects[CLASS as usize].framedelta1 = 256;
        s.shrine().arg0 = 5;
        s.shrine().arg1 = 10;
        s.w.seeds.insert(OBJ, u_seed);
        s.w.seeds.insert(P, Seed::new(7, 666));
        s.w.stats.insert((P, K_LEVEL), 12);
        s.op(Some(P));
        // Order on U: the §4 mode draw roll(32), then roll(10 − 5).
        let mut want = u_seed;
        want.roll(32);
        let n = 5 + want.roll(5) as usize;
        assert_eq!(s.w.seeds[&OBJ], want);
        assert_eq!(s.w.seeds[&P], Seed::new(7, 666));
        let got = s.others();
        assert_eq!(got.len(), n + 6);
        assert!(got[..n]
            .iter()
            .all(|g| *g == format!("potion 20 {potion} 1")));
        let offs: Vec<String> = POTION_OFFSETS
            .iter()
            .map(|(x, y)| {
                format!("missile {missile} owner 20 from 10 ({x}, {y}) lvl 2 flags 0x520")
            })
            .collect();
        assert_eq!(got[n..].to_vec(), offs);
    }
}
