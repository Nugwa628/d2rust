//! Proto contract tests (q-proto-audit, part a): S→C 0x00–0x2C. For each
//! id with a `d2-sim` (or `d2-server`) byte builder, the builder's bytes
//! equal the `d2_proto` encode of the same field values, the `d2_proto`
//! parse reads them back, and the client handler of [`super::HANDLERS`]
//! (driven through the receive path) reads the same values into the
//! model or its outputs. Ids without a builder check `d2_proto` encode
//! against the client handler only.
//!
//! Inputs: the edge values of each field, then a fixed xorshift sweep.

use d2_proto::s2c::{parse, Message as S2c};
use d2_proto::server as gen;
use d2_sim::combat::vitals::sync::{exp_message, life_mana_update};
use d2_sim::items::moves::layouts::gold;
use d2_sim::path::walk::messages::{player_move, player_stop, player_to_target, reassign_player};
use d2_sim::units::messages as sim;
use d2_sim::units::sound::play_sound_message;
use d2_sim::wiring::action::unit_update::skill_message::monster_hit;
use d2_sim::wiring::path::act_change::load_act as sim_load_act;
use d2_sim::wiring::path::place::map_reveal as sim_map_reveal;
use d2_sim::world::hirelings::level::speech_message;
use d2_sim::world::npc::transaction;
use d2_sim::world::objects::state_message;
use d2_sim::world::quests::helpers::msg_scroll_text;

use super::super::dispatch::Dispatch;
use super::super::output::Output;
use super::super::receive::receive_chunk;
use super::super::skills::{SkillEntry, SkillList, NATIVE};
use super::super::update::update_pass;
use super::super::world::{
    ActLoad, ClientUnit, KindData, PlayerData, RoomSight, SkillRow, UnitKey, ITEM, MONSTER, OBJECT,
    PLAYER,
};
use super::support::Model;

const U8S: [u8; 5] = [0, 1, 0x7F, 0x80, 0xFF];
const U16S: [u16; 5] = [0, 1, 0x7FFF, 0x8000, 0xFFFF];
const U32S: [u32; 6] = [0, 1, 0x1122_3344, 0x7FFF_FFFF, 0x8000_0000, u32::MAX];

/// Fixed xorshift sweep (test input only).
fn sweep(n: usize) -> Vec<u32> {
    let mut s = 0x2545_F491_u32;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        })
        .collect()
}

/// A model with one dispatch table built once (the support `recv`
/// rebuilds it per message).
struct Rig {
    m: Model,
    d: Dispatch,
}

impl Rig {
    fn new() -> Self {
        Self {
            m: Model::default(),
            d: Dispatch::from_spec().unwrap(),
        }
    }

    /// Receives one message; returns its outputs and asserts the handler
    /// accepted it.
    fn recv(&mut self, b: &[u8]) -> Vec<Output> {
        let r = self.try_recv(b);
        assert!(
            self.m.log.rejected.is_empty(),
            "{:02x?}: {:?}",
            b,
            self.m.log.rejected
        );
        r
    }

    /// Receives one message; the rejection log is left to the caller.
    fn try_recv(&mut self, b: &[u8]) -> Vec<Output> {
        let m = &mut self.m;
        let mut out = Vec::new();
        receive_chunk(&mut m.w, &m.inputs, &self.d, &mut m.log, &mut out, b).unwrap();
        out
    }

    /// Runs the update pass (the queued unit handlers).
    fn drain(&mut self) -> Vec<Output> {
        let m = &mut self.m;
        let mut out = Vec::new();
        update_pass(&mut m.w, &m.inputs, &self.d, &mut m.log, &mut out);
        assert!(m.log.rejected.is_empty(), "{:?}", m.log.rejected);
        out
    }

    fn put(&mut self, key: UnitKey) -> &mut ClientUnit {
        self.m.put(key)
    }

    fn unit(&self, key: UnitKey) -> &ClientUnit {
        self.m.unit(key)
    }

    /// The local player (0, `guid`) with player data.
    fn local(&mut self, guid: u32) -> UnitKey {
        let k = UnitKey::new(PLAYER, guid);
        let u = self.put(k);
        u.kind = KindData::Player(PlayerData::default());
        u.mode = 1;
        self.m.w.local_player = Some(k);
        k
    }
}

// ---------------------------------------------------------------- session

#[test]
fn session_bytes_0x00_0x02_0x04_0x05_0x06_one_layout() {
    assert_eq!(sim::GAME_LOADING, gen::GameLoading.encode());
    assert_eq!(sim::LOAD_SUCCESSFUL, gen::LoadSuccessful.encode());
    assert_eq!(sim::LOAD_COMPLETE, gen::LoadComplete.encode());
    // act_change.rs / session_flow.rs send `[0x05]`, `[0x06]` inline.
    assert_eq!([0x05], gen::UnloadComplete.encode());
    assert_eq!([0x06], gen::GameExit.encode());
    for b in [[0x00], [0x02], [0x04], [0x05], [0x06]] {
        assert!(parse(&b).is_ok(), "{b:02x?}");
    }
    let mut r = Rig::new();
    r.recv(&sim::GAME_LOADING);
    r.recv(&sim::LOAD_SUCCESSFUL);
    assert_eq!(r.m.w.outgoing, [vec![0x6B]]);
    r.recv(&[0x05]);
    assert!(r.m.w.unloaded && !r.m.w.in_game);
    r.recv(&[0x06]);
    assert!(r.m.w.exit_requested);
    // 0x04 needs a placed local player (fatal 0x527 otherwise).
    let k = r.local(1);
    r.m.w.units.get_mut(&k).unwrap().position = Some((10, 10));
    r.recv(&sim::LOAD_COMPLETE);
    assert!(r.m.w.in_game);
}

#[test]
fn game_flags_0x01_one_layout() {
    let mut r = Rig::new();
    let mut check = |d: u8, f: u32, e: bool, l: bool| {
        let s = sim::game_flags(d, f, e, l);
        let p = gen::GameFlags {
            difficulty: d,
            unk: f,
            expansion: u8::from(e),
            ladder: u8::from(l),
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::GameFlags(p));
        r.recv(&s);
        let w = &r.m.w;
        assert_eq!(
            (w.difficulty, w.game_flags, w.expansion, w.ladder),
            (d, f, u32::from(e), u8::from(l))
        );
    };
    for &d in &U8S {
        for &f in &U32S {
            for e in [false, true] {
                for l in [false, true] {
                    check(d, f, e, l);
                }
            }
        }
    }
    for c in sweep(600).chunks(2) {
        check(c[0] as u8, c[1], c[0] & 0x100 != 0, c[0] & 0x200 != 0);
    }
}

#[test]
fn load_act_0x03_one_layout() {
    let mut r = Rig::new();
    let mut check = |act: u8, seed: u32, town: u16, obj: u32| {
        let s = sim_load_act(act, seed, town, obj);
        let p = gen::LoadAct {
            act,
            f2: seed,
            f6: town,
            f8: obj,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::LoadAct(p));
        r.recv(&s);
        assert_eq!(
            r.m.w.act,
            Some(ActLoad {
                act,
                init_seed: seed,
                town_level: town,
                f8: obj
            })
        );
    };
    for &a in &U8S {
        for &s in &U32S {
            for &t in &U16S {
                check(a, s, t, s.rotate_left(7));
            }
        }
    }
    for c in sweep(900).chunks(3) {
        check(c[0] as u8, c[1], (c[0] >> 8) as u16, c[2]);
    }
    // The join's 0x03 (d2-server `session::load_act`, a d2_proto type)
    // carries the act's town level at u16@6.
    for act in 0..5u8 {
        let m = d2_server::adapters::session::load_act(act, 0x1234, 0x5678);
        let town = d2_sim::drlg::TOWN_LEVELS[usize::from(act)] as u16;
        assert_eq!(m.encode(), sim_load_act(act, 0x1234, town, 0x5678));
    }
}

#[test]
fn map_reveal_hide_0x07_0x08_one_layout() {
    let mut r = Rig::new();
    r.m.w.act = Some(ActLoad {
        act: 0,
        init_seed: 0,
        town_level: 0,
        f8: 0,
    });
    let mut check = |x: u16, y: u16, level: u8| {
        let (s7, s8) = (sim_map_reveal(x, y, level), sim::map_hide(x, y, level));
        let p7 = gen::MapReveal { x, y, level };
        let p8 = gen::MapHide { x, y, level };
        assert_eq!((s7, s8), (p7.encode(), p8.encode()));
        assert_eq!(parse(&s7).unwrap(), S2c::MapReveal(p7));
        assert_eq!(parse(&s8).unwrap(), S2c::MapHide(p8));
        for (b, show) in [(s7, true), (s8, false)] {
            r.recv(&b);
            assert_eq!(
                r.m.w.rooms_in_sight.last(),
                Some(&RoomSight { show, level, x, y })
            );
        }
    };
    for &x in &U16S {
        for &y in &U16S {
            for &l in &U8S {
                check(x, y, l);
            }
        }
    }
    for c in sweep(400).chunks(2) {
        check(c[0] as u16, (c[0] >> 16) as u16, c[1] as u8);
    }
}

// ------------------------------------------------------------- unit add / remove

#[test]
fn assign_level_warp_0x09_one_layout() {
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32, class: u8, x: u16, y: u16| {
        let s = sim::assign_warp(ty, guid, class, x, y);
        let p = gen::AssignLevelWarp {
            type_: ty,
            guid,
            class,
            x,
            y,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::AssignLevelWarp(p));
        let out = r.try_recv(&s);
        if ty > 5 {
            // The handler refuses a type past 5.
            assert_eq!(r.m.log.rejected.len(), 1);
            r.m.log.rejected.clear();
            return;
        }
        // A repeated key frees the old unit first (`UnitFreed` outputs).
        assert!(out.iter().all(|o| matches!(o, Output::UnitFreed { .. })));
        assert!(r.m.log.rejected.is_empty());
        let u = r.unit(UnitKey::new(ty, guid));
        let at = ((x, y) != (0, 0)).then_some((x, y));
        assert_eq!((u.class, u.position), (u32::from(class), at));
    };
    for ty in [0, 1, 2, 4, 5, 6, 0xFF] {
        for &g in &U32S {
            for &c in &U8S {
                for &v in &U16S {
                    check(ty, g, c, v, v ^ 0x00F0);
                }
            }
        }
    }
    for c in sweep(900).chunks(3) {
        check(
            (c[0] % 6) as u8,
            c[1],
            c[2] as u8,
            c[0] as u16,
            (c[2] >> 16) as u16,
        );
    }
}

#[test]
fn remove_unit_0x0a_one_layout() {
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32| {
        let s = sim::remove_unit(ty, guid);
        let p = gen::RemoveUnit { type_: ty, guid };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::RemoveUnit(p));
        let k = UnitKey::new(ty, guid);
        let other = UnitKey::new(ty, guid ^ 1);
        r.put(k);
        r.put(other);
        r.recv(&s);
        assert!(!r.m.w.units.contains_key(&k));
        assert!(r.m.w.units.contains_key(&other));
        r.m.w.remove(other);
    };
    // No local player: the hireling GUID is u32::MAX (never removed).
    for ty in [0, 1, 2, 3, 4, 5, 0x80, 0xFF] {
        for &g in &U32S[..5] {
            check(ty, g);
        }
    }
    for c in sweep(300).chunks(2) {
        check(c[0] as u8, c[1] & 0x7FFF_FFFF);
    }
}

#[test]
fn game_handshake_0x0b_one_layout() {
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32| {
        let s = sim::unit_ref(0x0B, ty, guid);
        // act_change.rs:135 builds it inline.
        let g = guid.to_le_bytes();
        assert_eq!(s, [0x0B, ty, g[0], g[1], g[2], g[3]]);
        let p = gen::GameHandshake { type_: ty, guid };
        let b = d2_proto::s2c::GameHandshake {
            unit_type: ty,
            unit_guid: guid,
        };
        assert_eq!((s, s), (p.encode(), b.encode()));
        assert_eq!(parse(&s).unwrap(), S2c::GameHandshake(b));
        let k = UnitKey::new(ty, guid);
        r.put(k);
        r.m.w.local_player = None;
        r.recv(&s);
        assert_eq!(r.m.w.local_player, Some(k));
        r.m.w.remove(k);
    };
    for &t in &U8S {
        for &g in &U32S {
            check(t, g);
        }
    }
    for c in sweep(200) {
        check(c as u8, c.rotate_left(9));
    }
}

// ------------------------------------------------------------- queued unit messages

/// The stored mode request of `k` after the update pass.
fn request(r: &Rig, k: UnitKey) -> (u8, [i32; 7]) {
    let q = r.unit(k).last_mode_request.expect("a mode request");
    (q.code, q.record)
}

#[test]
fn monster_hit_0x0c_one_layout() {
    let mut r = Rig::new();
    let mut check = |guid: u32, b0: u8, life: u8, flag: bool| {
        let s = monster_hit(guid, b0, life, flag);
        let h = if life > 1 { life - 1 } else { life } | if flag { 0x80 } else { 0 };
        let p = gen::MonsterHit {
            f1: MONSTER,
            f2: guid,
            f6: 0x13,
            f7: b0,
            f8: h,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::MonsterHit(p));
        let k = UnitKey::new(MONSTER, guid);
        r.put(k);
        r.recv(&s);
        r.drain();
        assert_eq!(
            request(&r, k),
            (0x13, [i32::from(b0), i32::from(h), 0, 0, 0, 0, 0])
        );
    };
    for &g in &U32S {
        for &b in &U8S {
            for &l in &U8S {
                check(g, b, l, false);
                check(g, l, b, true);
            }
        }
    }
    for c in sweep(200) {
        let [a, b, f, _] = c.to_le_bytes();
        check(c.rotate_left(3), a, b, f & 1 != 0);
    }
}

#[test]
fn player_stop_0x0d_one_layout() {
    // The builder's own sweep is `conformance/tests/s2c_builders.rs`
    // `player_stop_0x0d_one_maker`; here the two d2_proto types and the
    // client record. Item type 4 (the update pass drains types 0–4;
    // no player / object side effects).
    let mut r = Rig::new();
    let mut check = |guid: u32, a: u8, x: u16, y: u16, b: u8, life: u8| {
        let s = player_stop(ITEM, guid, a, x, y, b, life);
        let p = gen::PlayerStop {
            type_: ITEM,
            guid,
            a,
            x,
            y,
            b,
            life_pct: life,
        };
        let h = d2_proto::s2c::PlayerStop {
            unit_type: ITEM,
            unit_guid: guid,
            f6: a,
            x,
            y,
            f11: b,
            f12: life,
        };
        assert_eq!((s, s), (p.encode(), h.encode()));
        let k = UnitKey::new(ITEM, guid);
        r.put(k);
        r.recv(&s);
        r.drain();
        assert_eq!(
            request(&r, k),
            (a, [i32::from(x), i32::from(y), i32::from(b), 0, 0, 0, 0])
        );
    };
    for &g in &U32S {
        for &v in &U8S {
            for &c in &U16S {
                check(g, v, c, c ^ 0x0F0F, v ^ 0x55, v);
            }
        }
    }
    for c in sweep(400).chunks(2) {
        let [a, b, l, _] = c[1].to_le_bytes();
        check(c[0], a, c[1] as u16, (c[0] >> 16) as u16, b, l);
    }
}

#[test]
fn object_state_0x0e_one_layout() {
    let mut r = Rig::new();
    let mut check = |guid: u32, sel: bool, mode: u32| {
        let s = state_message(guid, sel, mode);
        let p = gen::ObjectState {
            type_: OBJECT,
            guid,
            kind: 3,
            selectable: u8::from(sel),
            mode,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::ObjectState(p));
        // The client record (code u8@6, u8@7, u32@8) on an item unit:
        // the same bytes with u8@1 = 4 (no object shrine side effects).
        let mut b = s;
        b[1] = ITEM;
        let k = UnitKey::new(ITEM, guid);
        r.put(k);
        r.recv(&b);
        r.drain();
        assert_eq!(
            request(&r, k),
            (3, [i32::from(sel), mode as i32, 0, 0, 0, 0, 0])
        );
    };
    for &g in &U32S {
        for &m in &U32S {
            check(g, false, m);
            check(g, true, m);
        }
    }
    for c in sweep(300).chunks(2) {
        check(c[0], c[1] & 1 != 0, c[1]);
    }
}

#[test]
fn player_move_to_target_0x0f_0x10_one_layout() {
    let mut r = Rig::new();
    let mut check = |guid: u32, code: u8, tx: u16, ty: u16, x: u16, y: u16, tt: u8, tg: u32| {
        let s = player_move(ITEM, guid, code, tx, ty, x, y);
        let p = gen::PlayerMove {
            type_: ITEM,
            guid,
            code,
            target_x: tx,
            target_y: ty,
            zero: 0,
            x,
            y,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::PlayerMove(p));
        let s2 = player_to_target(ITEM, guid, code, tt, tg, x, y);
        let p2 = gen::PlayerToTarget {
            type_: ITEM,
            guid,
            code,
            target_type: tt,
            target_guid: tg,
            x,
            y,
        };
        assert_eq!(s2, p2.encode());
        assert_eq!(parse(&s2).unwrap(), S2c::PlayerToTarget(p2));
        let k = UnitKey::new(ITEM, guid);
        for (b, rec) in [
            (&s[..], [i32::from(tx), i32::from(ty), 0, 0, 0, 0, 0]),
            (&s2[..], [i32::from(tt), tg as i32, 0, 0, 0, 0, 0]),
        ] {
            let u = r.put(k);
            u.position = Some((x, y));
            r.recv(b);
            r.drain();
            assert_eq!(request(&r, k), (code, rec));
            // The position check reads x u16@12, y u16@14 (step 3).
            let sp = if x == 0 || y == 0 { (0, 0) } else { (x, y) };
            assert_eq!(r.unit(k).server_point, sp);
        }
    };
    for &g in &U32S {
        for &c in &U8S {
            for &v in &U16S {
                check(
                    g,
                    c,
                    v,
                    !v,
                    v ^ 0x1234,
                    v.wrapping_add(7),
                    c,
                    g.rotate_left(5),
                );
            }
        }
    }
    for c in sweep(400).chunks(4) {
        check(
            c[0],
            c[1] as u8,
            c[1] as u16,
            (c[1] >> 16) as u16,
            c[2] as u16,
            (c[2] >> 16) as u16,
            (c[3] >> 8) as u8,
            c[3],
        );
    }
}

#[test]
fn report_kill_0x11_one_layout() {
    // d2-sim `report_kill` (monster update step 9) == d2_proto encode ==
    // what the client handler reads.
    let mut r = Rig::new();
    r.m.inputs.tables.overlay_count = 0x1_0000;
    let mut check = |ty: u8, guid: u32, overlay: u16| {
        let p = gen::ReportKill {
            type_: ty,
            guid,
            overlay,
        };
        let b = p.encode();
        assert_eq!(
            d2_sim::units::messages::report_kill(ty, guid, overlay)[..],
            b[..]
        );
        assert_eq!(parse(&b).unwrap(), S2c::ReportKill(p));
        let k = UnitKey::new(ty, guid);
        r.put(k);
        let out = r.recv(&b);
        let sound = match overlay {
            151 => 396,
            152 => 397,
            _ => 0,
        };
        assert_eq!(
            out,
            [Output::UnitOverlay {
                unit: k,
                overlay,
                mode: 2,
                sound
            }]
        );
    };
    for &t in &U8S {
        for &g in &U32S {
            for &o in &U16S {
                check(t, g, o);
            }
        }
    }
    check(1, 7, 151);
    check(1, 7, 152);
}

#[test]
fn reassign_player_0x15_one_layout() {
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32, x: u16, y: u16, flag: u8| {
        let s = reassign_player(ty, guid, x, y, flag);
        let p = gen::ReassignPlayer {
            type_: ty,
            guid,
            x,
            y,
            flag,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::ReassignPlayer(p));
        let k = UnitKey::new(ty, guid);
        // Mode 1: not dead (a dead unit stays, step 4.3).
        r.put(k).mode = 1;
        r.try_recv(&s);
        if (x, y) == (0, 0) {
            // Step 2: fatal 0x538.
            assert_eq!(r.m.log.rejected.len(), 1);
            r.m.log.rejected.clear();
        } else {
            assert!(r.m.log.rejected.is_empty());
            assert_eq!(r.unit(k).position, Some((x, y)));
        }
    };
    for t in [0, 1, 2, 5, 0xFF] {
        for &g in &U32S {
            for &v in &U16S {
                for &f in &U8S {
                    check(t, g, v, v ^ 0x0101, f);
                }
            }
        }
    }
    for c in sweep(600).chunks(3) {
        check(
            c[0] as u8,
            c[1],
            c[2] as u16,
            (c[2] >> 16) as u16,
            (c[0] >> 8) as u8,
        );
    }
}

// ------------------------------------------------------------- 0x18 (bit-packed)

#[test]
fn life_mana_update_0x18_one_layout() {
    let mut r = Rig::new();
    let k = r.local(1);
    let mut check =
        |life: i32, mana: i32, st: i32, lp: u8, mp: u8, x: u16, y: u16, dx: u8, dy: u8| {
            let s = life_mana_update(life, mana, st, lp, mp, x, y, dx, dy);
            // Each value is cut to its width, LSB first from bit 0.
            let p = gen::LifeManaUpdate {
                life: (life & 0x7FFF) as u16,
                mana: (mana & 0x7FFF) as u16,
                stamina: (st & 0x7FFF) as u16,
                life_pred: lp & 0x7F,
                mana_pred: mp & 0x7F,
                x,
                y,
                dx,
                dy,
            };
            assert_eq!(
                s,
                p.encode(),
                "{life} {mana} {st} {lp} {mp} {x} {y} {dx} {dy}"
            );
            assert_eq!(parse(&s).unwrap(), S2c::LifeManaUpdate(p));
            // Bits 115–119 are never written.
            assert_eq!(s[14] & 0xF8, 0);
            // The client: at the stated point (no correction), stats set.
            let u = r.put(k);
            u.kind = KindData::Player(PlayerData::default());
            u.mode = 1;
            u.position = Some((x, y));
            r.recv(&s);
            let u = r.unit(k);
            assert_eq!(
                (u.stat(6), u.stat(8), u.stat(10), u.stat(74), u.stat(26)),
                (
                    i32::from(p.life) << 8,
                    i32::from(p.mana) << 8,
                    i32::from(p.stamina) << 8,
                    i32::from(p.life_pred),
                    i32::from(p.mana_pred)
                )
            );
            let sp = if x == 0 || y == 0 { (0, 0) } else { (x, y) };
            assert_eq!(u.server_point, sp);
        };
    let ints = [0, 1, 0x7FFF, 0x8000, 0xFFFF, -1, i32::MAX];
    for &l in &ints {
        for &v in &U8S {
            for &c in &U16S {
                check(
                    l,
                    l ^ 0x1555,
                    l.wrapping_add(3),
                    v,
                    !v,
                    c,
                    c ^ 0x00FF,
                    v,
                    !v,
                );
            }
        }
    }
    for c in sweep(1500).chunks(3) {
        let [lp, mp, dx, dy] = c[2].to_le_bytes();
        check(
            c[0] as i32,
            (c[0] >> 15) as i32,
            c[1] as i32,
            lp,
            mp,
            (c[1] >> 8) as u16,
            c[2] as u16,
            dx,
            dy,
        );
    }
}

#[test]
fn life_mana_update_0x18_dx_mirrors_the_path_target() {
    // By design:
    // the server sends dx = (X - target) & 0xFF and the client
    // forms the check point X + dx, the target reflected through the
    // server point.
    let dx_of = |x: u16, t: u16| x.wrapping_sub(t) as u8;
    // (a) The local player (mode 1) at (100, 100); the server's point
    // (104, 100) is past the tolerance, its target (100, 100): dx 4,
    // point (108, 100), d2 >= d1 -> corrected (the 0x5F goes out).
    let mut r = Rig::new();
    let k = r.local(1);
    r.m.w.units.get_mut(&k).unwrap().position = Some((100, 100));
    let (x, y, tx, ty) = (104u16, 100u16, 100u16, 100u16);
    assert_eq!(dx_of(x, tx), 4);
    r.recv(&life_mana_update(
        0x40,
        0x40,
        0x40,
        1,
        1,
        x,
        y,
        dx_of(x, tx),
        dx_of(y, ty),
    ));
    assert!(
        r.m.w.outgoing.iter().any(|m| m.first() == Some(&0x5F)),
        "{:02x?}",
        r.m.w.outgoing
    );
    // (b) Server (96, 100), target (90, 100): dx 6, point (102, 100),
    // d2 < d1 -> the position is kept (nothing goes out).
    let mut r = Rig::new();
    let k = r.local(1);
    r.m.w.units.get_mut(&k).unwrap().position = Some((100, 100));
    let (x, y, tx, ty) = (96u16, 100u16, 90u16, 100u16);
    assert_eq!(dx_of(x, tx), 6);
    r.recv(&life_mana_update(
        0x40,
        0x40,
        0x40,
        1,
        1,
        x,
        y,
        dx_of(x, tx),
        dx_of(y, ty),
    ));
    assert!(r.m.w.outgoing.is_empty(), "{:02x?}", r.m.w.outgoing);
}

// ------------------------------------------------------------- 0x19–0x20 stats

#[test]
fn gold_0x19_0x1d_0x1e_0x1f_one_layout() {
    let mut r = Rig::new();
    let k = r.local(1);
    let mut check = |old: u32, new: u32| {
        let Some(s) = gold(new, old) else {
            assert_eq!(new, old);
            return;
        };
        let proto = match s[0] {
            0x19 => gen::SmallGoldPickup { delta: s[1] }.encode().to_vec(),
            0x1D => gen::SetStatByte {
                stat: 14,
                value: new as u8,
            }
            .encode()
            .to_vec(),
            0x1E => gen::SetStatWord {
                stat: 14,
                value: new as u16,
            }
            .encode()
            .to_vec(),
            0x1F => gen::SetStatDword {
                stat: 14,
                value: new,
            }
            .encode()
            .to_vec(),
            id => panic!("gold sent 0x{id:02X}"),
        };
        assert_eq!(s, proto);
        assert!(parse(&s).is_ok());
        r.put(k).stats.insert(14, old as i32);
        r.m.w.local_player = Some(k);
        r.recv(&s);
        assert_eq!(r.unit(k).stat(14), new as i32, "{old} -> {new}: {s:02x?}");
    };
    let edges = [
        0,
        1,
        0xFE,
        0xFF,
        0x100,
        0xFFFE,
        0xFFFF,
        0x1_0000,
        2_500_000,
        u32::MAX,
    ];
    for &o in &edges {
        for &n in &edges {
            check(o, n);
            check(o, o.wrapping_add(n & 0xFF));
        }
    }
    for c in sweep(1000).chunks(2) {
        check(c[0] >> (c[1] % 32), c[1] >> (c[0] % 32));
    }
}

#[test]
fn experience_0x1a_0x1b_0x1c_one_layout() {
    let mut r = Rig::new();
    let k = r.local(1);
    let mut check = |old: u32, new: u32| {
        let Some(s) = exp_message(new, old) else {
            assert_eq!(new, old);
            return;
        };
        let d = new.wrapping_sub(old);
        let proto = match s[0] {
            0x1A => gen::AddExpByte { value: d as u8 }.encode().to_vec(),
            0x1B => gen::AddExpWord { value: d as u16 }.encode().to_vec(),
            0x1C => gen::AddExpDword { value: new }.encode().to_vec(),
            id => panic!("experience sent 0x{id:02X}"),
        };
        assert_eq!(s, proto);
        assert!(parse(&s).is_ok());
        r.put(k).stats.insert(13, old as i32);
        r.m.w.local_player = Some(k);
        r.recv(&s);
        assert_eq!(r.unit(k).stat(13), new as i32, "{old} -> {new}: {s:02x?}");
    };
    let edges = [0, 1, 0xFE, 0xFF, 0x100, 0xFFFE, 0xFFFF, 0x1_0000, u32::MAX];
    for &o in &edges {
        for &n in &edges {
            check(o, n);
            check(o, o.wrapping_add(n));
        }
    }
    for c in sweep(1000).chunks(2) {
        check(c[0], c[0].wrapping_add(c[1] >> (c[1] % 32)));
    }
}

#[test]
fn set_stat_0x1d_0x1e_0x1f_one_layout() {
    // as d2-server `session::stat_message` (the d2-sim copy
    // in `wiring/action/vitals_sync.rs` is private and the same code).
    use d2_server::adapters::session::stat_message;
    let mut r = Rig::new();
    let k = r.local(1);
    let mut check = |stat: u16, value: i32| {
        let Some(s) = stat_message(stat, value) else {
            assert!(stat > 0xFE);
            return;
        };
        let (st, v) = (stat as u8, value as u32);
        let proto = match s[0] {
            0x1D => gen::SetStatByte {
                stat: st,
                value: v as u8,
            }
            .encode()
            .to_vec(),
            0x1E => gen::SetStatWord {
                stat: st,
                value: v as u16,
            }
            .encode()
            .to_vec(),
            0x1F => gen::SetStatDword { stat: st, value: v }.encode().to_vec(),
            id => panic!("stat sent 0x{id:02X}"),
        };
        assert_eq!(s, proto);
        assert!(parse(&s).is_ok());
        r.m.w.local_player = Some(k);
        r.recv(&s);
        assert_eq!(r.unit(k).stat(stat), value, "{stat} {value}: {s:02x?}");
    };
    let values = [
        0,
        1,
        0xFE,
        0xFF,
        0xFFFE,
        0xFFFF,
        0x1_0000,
        -1,
        i32::MIN,
        i32::MAX,
    ];
    // Stats 6 (life on a dead unit) and 12 (level) run hooks; the unit is
    // not dead and stat 12 only refreshes requirements.
    for stat in [0u16, 1, 7, 12, 13, 14, 0x7F, 0x80, 0xFE, 0xFF, 0x1FF] {
        for &v in &values {
            check(stat, v);
        }
    }
    for c in sweep(600).chunks(2) {
        check((c[0] % 0x100) as u16, (c[1] >> (c[0] % 32)) as i32);
    }
}

#[test]
fn stat_update_0x20_one_layout() {
    // d2-sim `stat_update` (sent by `View::player_part_b`) ==
    // d2_proto encode == the client handler.
    let mut r = Rig::new();
    let mut check = |guid: u32, stat: u8, value: u32| {
        let p = gen::StatUpdate { guid, stat, value };
        let b = p.encode();
        assert_eq!(sim::stat_update(guid, stat, value).to_vec(), b.to_vec());
        assert_eq!(parse(&b).unwrap(), S2c::StatUpdate(p));
        let k = UnitKey::new(PLAYER, guid);
        r.put(k);
        r.recv(&b);
        assert_eq!(r.unit(k).stat(u16::from(stat)), value as i32);
        r.m.w.remove(k);
    };
    for &g in &U32S {
        for &s in &U8S {
            for &v in &U32S {
                check(g, s, v);
            }
        }
    }
    for c in sweep(400).chunks(2) {
        check(c[0], c[1] as u8, c[1].rotate_left(11));
    }
}

// ------------------------------------------------------------- 0x21–0x23 skills

fn rows(n: usize) -> Vec<SkillRow> {
    vec![SkillRow::default(); n]
}

#[test]
fn update_item_oskill_0x21_one_layout() {
    let mut r = Rig::new();
    r.m.inputs.tables.skills = rows(0x200);
    let mut check = |ty: u8, remove: bool, guid: u32, skill: u16, base: u8, bonus: u8| {
        let s = sim::update_oskill(ty, remove, guid, skill, base, bonus);
        let p = gen::UpdateItemOSkill {
            type_: ty,
            remove: u8::from(remove),
            guid,
            skill,
            level: base,
            bonus,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::UpdateItemOSkill(p));
        // The d2-server copy (`handlers/skills/world.rs` add_skill_level):
        // type 0, remove 0, bonus 0, the same offsets.
        let mut inline = vec![0x21, 0, 0];
        inline.extend_from_slice(&guid.to_le_bytes());
        inline.extend_from_slice(&skill.to_le_bytes());
        inline.extend_from_slice(&[base, 0, 0]);
        assert_eq!(inline, sim::update_oskill(0, false, guid, skill, base, 0));
        // The client: (u8@1, GUID u32@3) with a list; skill u16@7, level
        // u8@9 assigned (remove 0, level ≠ 0: the native entry's base).
        let k = UnitKey::new(ty, guid);
        r.put(k).skills = Some(SkillList::default());
        r.m.w.skill_tree_flag = None;
        r.recv(&s);
        assert_eq!(r.m.w.skill_tree_flag, Some(0));
        let list = r.unit(k).skills.as_ref().unwrap();
        if !remove && base != 0 && usize::from(skill) < 0x200 {
            let i = list.native(skill).expect("assigned");
            assert_eq!(list.entries[i].base, i32::from(base));
        }
        r.m.w.remove(k);
    };
    for t in [0u8, 1] {
        for &g in &U32S {
            for sk in [0u16, 1, 0x7F, 0x80, 0x1FF] {
                for &b in &U8S {
                    check(t, false, g, sk, b, !b);
                }
            }
        }
    }
    check(0, true, 5, 6, 0, 0);
    for c in sweep(400).chunks(2) {
        let [b, n, _, _] = c[1].to_le_bytes();
        check(
            (c[0] & 1) as u8,
            false,
            c[0],
            (c[1] >> 16) as u16 % 0x200,
            b,
            n,
        );
    }
}

#[test]
fn update_item_skill_0x22_one_layout() {
    // The d2-sim builder (`sim::update_item_skill`, sent by the inventory
    // desk's `send_skill_quantity`) == the d2_proto encode == the client.
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32, skill: u16, q: u8, flag: u8| {
        let p = gen::UpdateItemSkill {
            type_: ty,
            unit: guid,
            skill,
            quantity: q,
            body_state: flag,
        };
        let b = p.encode();
        assert_eq!(
            sim::update_item_skill(ty, guid, skill, q, flag != 0).to_vec(),
            b
        );
        assert_eq!(parse(&b).unwrap(), S2c::UpdateItemSkill(p));
        let k = UnitKey::new(PLAYER, guid);
        r.put(k).skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill,
                owner: NATIVE,
                quantity: -7,
                ..SkillEntry::default()
            }],
            ..SkillList::default()
        });
        r.recv(&b);
        let want = if flag != 0 { -7 } else { i32::from(q) };
        assert_eq!(r.unit(k).skills.as_ref().unwrap().entries[0].quantity, want);
    };
    for &t in &U8S {
        for &g in &U32S {
            for &s in &U16S {
                for &q in &U8S {
                    check(t, g, s, q, 0);
                    check(t, g, s, q, 1);
                }
            }
        }
    }
}

#[test]
fn set_skill_0x23_one_layout() {
    let mut r = Rig::new();
    r.m.inputs.tables.skills = rows(0x1_0000);
    let mut check = |guid: u32, hand: u8, skill: u16, item: u32| {
        let s = sim::set_skill(0, guid, hand, skill, item);
        let p = gen::SetSkill {
            type_: 0,
            guid,
            hand,
            skill,
            item,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::SetSkill(p));
        // The client selects (skill u16@7, owner u32@9): left when hand
        // u8@6 ≠ 0, else right.
        let k = UnitKey::new(PLAYER, guid);
        r.put(k).skills = Some(SkillList {
            entries: vec![
                SkillEntry {
                    skill: skill ^ 1,
                    owner: item,
                    ..SkillEntry::default()
                },
                SkillEntry {
                    skill,
                    owner: item,
                    ..SkillEntry::default()
                },
            ],
            ..SkillList::default()
        });
        r.recv(&s);
        let l = r.unit(k).skills.as_ref().unwrap();
        let want = if hand != 0 {
            (Some(1), None)
        } else {
            (None, Some(1))
        };
        assert_eq!((l.left, l.right), want);
        r.m.w.remove(k);
    };
    for &g in &U32S {
        for &h in &U8S {
            for &s in &U16S {
                for &i in &U32S {
                    check(g, h, s, i);
                }
            }
        }
    }
    for c in sweep(600).chunks(2) {
        check(c[0], c[1] as u8, (c[1] >> 8) as u16, c[0].rotate_left(13));
    }
}

#[test]
fn set_skill_0x23_fatal_0x668_is_the_missing_skill_rows() {
    // The join's 0x23 (`d2-server` session.rs: hand 1 then hand 0, and
    // the stub load's own hand 0) has the documented layout; the fatal
    // 0x668 of the join / the synthetic play is `select` finding the skill outside
    // the client's `skills` rows (none bound in the test, one in the
    // synthetic play), not a byte disagreement.
    let s = sim::set_skill(0, 1, 0, 6, u32::MAX);
    let mut r = Rig::new();
    let k = UnitKey::new(PLAYER, 1);
    r.put(k).skills = Some(SkillList::default());
    r.try_recv(&s);
    assert_eq!(r.m.rejected(), [(0x23, "fatal assert 0x668".to_owned())]);
    let mut r = Rig::new();
    r.m.inputs.tables.skills = rows(7);
    r.put(k).skills = Some(SkillList::default());
    r.recv(&s);
}

// ------------------------------------------------------------- 0x26–0x2C UI, sound

#[test]
fn chat_0x26_one_layout() {
    // d2_proto types no 0x26 (variable size); its size rule splits the
    // sim bytes whole and the client reads the TSV fields.
    let mut r = Rig::new();
    let mut check = |byte8: u8, ty: u8, guid: u32, text: &[u8]| {
        let s = sim::overhead_chat(byte8, ty, guid, text);
        assert_eq!(
            d2_proto::transport::server_size(&s),
            d2_proto::schema::Size::Bytes(s.len())
        );
        let k = UnitKey::new(ty, guid);
        let out = r.recv(&s);
        assert_eq!(
            out,
            [Output::ChatLine {
                kind: 5,
                lang: byte8,
                unit: k,
                b8: 0,
                b9: 0,
                name: Vec::new(),
                text: text.to_vec(),
                present: false,
                player_name: None,
            }]
        );
    };
    for &b in &U8S {
        for &t in &U8S {
            for &g in &U32S {
                check(b, t, g, b"");
                check(b, t, g, b"hi");
            }
        }
    }
    for c in sweep(300).chunks(3) {
        let n = (c[2] % 60) as usize;
        let text: Vec<u8> = sweep(n + 1)
            .iter()
            .skip(1)
            .map(|v| (v ^ c[2]) as u8 | 1)
            .collect();
        check(c[0] as u8, c[1] as u8, c[1], &text);
    }
}

#[test]
fn npc_info_0x27_one_layout() {
    let mut r = Rig::new();
    let mut check = |s: [u8; 40], p: gen::NpcInfo| {
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::NpcInfo(p));
        let out = r.recv(&s);
        assert_eq!(
            out,
            [Output::NpcText {
                bytes: s,
                present: false,
                object_class: 0
            }]
        );
    };
    let base = |ty: u8, guid: u32, count: u8, kind0: u8, str0: u16| gen::NpcInfo {
        type_: ty,
        guid,
        count,
        kind0,
        str0,
        ..gen::NpcInfo::default()
    };
    for &g in &U32S {
        // Type 1, count 1, kind 3, the level speech.
        let s = speech_message(g);
        let str0 = u16::from_le_bytes([s[10], s[11]]);
        check(s, base(1, g, 1, 3, str0));
        for &st in &U16S {
            // Type 2, count 1, kind 0, the string.
            check(msg_scroll_text(g, st), base(2, g, 1, 0, st));
        }
    }
    // (`world/npc.rs` 0x27 type 1 + the 34 list bytes of
    // `encode_text_list` in `app/npc_seams.rs`: newest entry first).
    for c in sweep(700).chunks(7) {
        let n = (c[0] % 8) as usize;
        let list: Vec<(u16, u32)> = c.iter().take(n).map(|&v| (v as u16, v >> 24)).collect();
        let mut s = [0u8; 40];
        s[0] = 0x27;
        s[1] = 1;
        s[2..6].copy_from_slice(&c[6].to_le_bytes());
        s[6..].copy_from_slice(&crate::app::npc_seams::encode_text_list(&list));
        let mut p = base(1, c[6], n.min(7) as u8, 0, 0);
        let mut e = list.iter().rev().take(7).map(|&(st, k)| (k as u8, st));
        let mut next = || e.next().unwrap_or_default();
        (p.kind0, p.str0) = next();
        (p.kind1, p.str1) = next();
        (p.kind2, p.str2) = next();
        (p.kind3, p.str3) = next();
        (p.kind4, p.str4) = next();
        (p.kind5, p.str5) = next();
        (p.kind6, p.str6) = next();
        check(s, p);
    }
}

#[test]
fn quest_info_0x28_one_layout() {
    // `world/quests.rs` `send_player_flags` builds it inline (28, type,
    // GUID, 0, the record); the two d2_proto types and the client.
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32, rec: [u8; 96]| {
        let mut s = vec![0x28, ty];
        s.extend_from_slice(&guid.to_le_bytes());
        s.push(0);
        s.extend_from_slice(&rec);
        let g = gen::QuestInfo {
            type_: ty,
            guid,
            f6: 0,
            flags: rec,
        };
        let b = d2_proto::s2c::QuestInfo {
            unit_type: ty,
            unit_guid: guid,
            record: rec,
        };
        assert_eq!((&s[..], &s[..]), (&g.encode()[..], &b.encode()[..]));
        assert_eq!(parse(&s).unwrap(), S2c::QuestInfo(b));
        r.m.w.outgoing.clear();
        let out = r.recv(&s);
        if ty == 6 {
            assert_eq!(out, [Output::QuestFlags { record: rec }]);
        } else {
            // No such unit: C→S 0x30 (u32 R = u8@6, u32 G).
            let mut m = vec![0x30, 0, 0, 0, 0];
            m.extend_from_slice(&guid.to_le_bytes());
            assert_eq!(r.m.w.outgoing, [m]);
            assert_eq!(out, [Output::NpcGone { guid }]);
        }
    };
    for t in [1u8, 2, 6] {
        for &g in &U32S {
            for &v in &U8S {
                check(t, g, [v; 96]);
            }
        }
    }
    for c in sweep(97 * 20).chunks(97) {
        let mut rec = [0u8; 96];
        for (b, v) in rec.iter_mut().zip(&c[1..]) {
            *b = *v as u8;
        }
        check(if c[0] & 1 == 0 { 6 } else { 1 }, c[0], rec);
    }
}

#[test]
fn game_quest_info_0x29_one_layout() {
    let mut r = Rig::new();
    for c in sweep(96 * 30).chunks(96) {
        let mut rec = [0u8; 96];
        for (b, v) in rec.iter_mut().zip(c) {
            *b = *v as u8;
        }
        // `QuestControl::send_game_flags`: 29 + the record.
        let mut s = vec![0x29];
        s.extend_from_slice(&rec);
        let g = gen::GameQuestInfo { record: rec };
        let b = d2_proto::s2c::GameQuestInfo { record: rec };
        assert_eq!((&s[..], &s[..]), (&g.encode()[..], &b.encode()[..]));
        assert_eq!(parse(&s).unwrap(), S2c::GameQuestInfo(b));
        assert_eq!(r.recv(&s), [Output::GameQuestFlags { record: rec }]);
    }
}

#[test]
fn npc_transaction_0x2a_one_layout() {
    // The sim / built-type sweep is `conformance/tests/s2c_builders.rs`
    // `npc_transaction_0x2a_one_maker`; here the generated type and the
    // client output.
    let mut r = Rig::new();
    let k = r.local(1);
    r.put(k).stats.insert(14, 1234);
    for &kind in &U8S {
        for &code in &U8S {
            for &g in &U32S {
                let s = transaction(kind, code, g, g ^ 0xFFFF);
                let p = gen::NpcTransaction {
                    kind,
                    code,
                    guid: g,
                    gold: g ^ 0xFFFF,
                };
                assert_eq!(s, p.encode());
                let gold = (g ^ 0xFFFF) as i32;
                assert_eq!(r.recv(&s), [Output::NpcTransaction { bytes: s, gold }]);
                assert_eq!(r.unit(k).stats.get(&14), Some(&gold));
            }
        }
    }
}

#[test]
fn play_sound_0x2c_one_layout() {
    let mut r = Rig::new();
    let mut check = |ty: u8, guid: u32, event: u16| {
        let s = play_sound_message(ty, guid, event);
        let p = gen::PlaySound {
            type_: ty,
            guid,
            event,
        };
        assert_eq!(s, p.encode());
        assert_eq!(parse(&s).unwrap(), S2c::PlaySound(p));
        let k = UnitKey::new(ty, guid);
        let u = r.put(k);
        u.class = guid ^ 3;
        u.position = Some((7, 9));
        assert_eq!(
            r.recv(&s),
            [Output::ServerSound {
                unit: k,
                class: guid ^ 3,
                at: Some((7, 9)),
                event
            }]
        );
    };
    for &t in &U8S {
        for &g in &U32S {
            for &e in &U16S {
                check(t, g, e);
            }
        }
    }
    for c in sweep(400).chunks(2) {
        check(c[0] as u8, c[1], (c[0] >> 16) as u16);
    }
}
