//! Proto contract tests (q-proto-audit, part c): S→C 0x67–0x9B except
//! 0x94. For each id with a `d2-sim` byte builder: the builder's bytes
//! equal the `d2_proto` (TSV-generated) encode of the same field values,
//! the `d2_proto` decode reads them back, and the client handler (the
//! `HANDLERS` entry, through the dispatch) reads the same values
//! into the model or its outputs. Ids without a sim builder check the
//! `d2_proto` encode against the client handler only.
//!
//! Inputs: the edge values 0, 1, 0x7F / 0x80, 0xFF, 0x7FFF / 0x8000,
//! 0xFFFF, 0x7FFF_FFFF / 0x8000_0000, u32::MAX rotated over the fields,
//! then a fixed xorshift sweep.

use d2_proto::server as gen;
use d2_proto::FixedMessage;
use d2_sim::combat::vitals::sync::life_mana_update2;
use d2_sim::items::moves::layouts;
use d2_sim::monsters::mode_message as mm;
use d2_sim::path::walk::messages::walk_verify;
use d2_sim::units::messages as unit;
use d2_sim::wiring::action::switch::corpse_assign;
use d2_sim::world::cube::trade_action;
use d2_sim::world::hirelings::pets;
use d2_sim::world::npc::resurrect_message;

use super::super::output::{Output, SkillTarget};
use super::super::world::{KindData, ObjectData, RosterRecord, UnitKey, MONSTER, OBJECT, PLAYER};
use super::support::Model;

/// The edge values (cut to each field's width by `as`).
const EDGES: [u32; 11] = [
    0,
    1,
    0x7F,
    0x80,
    0xFF,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    u32::MAX,
];

/// Field value rows: every edge value rotated over `n` fields (stride 3,
/// so neighbouring fields differ), every field at one edge value, then
/// `sweep` rows of a fixed xorshift (test input only).
fn cases(n: usize, sweep: usize) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for k in 0..EDGES.len() {
        out.push((0..n).map(|j| EDGES[(k + 3 * j) % EDGES.len()]).collect());
        out.push(vec![EDGES[k]; n]);
    }
    let mut s = 0x2545_F491_u32;
    for _ in 0..sweep {
        out.push(
            (0..n)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    s
                })
                .collect(),
        );
    }
    out
}

/// `sim` equals the generated encode, and decodes back to `want`.
fn one_layout<M: FixedMessage + PartialEq + std::fmt::Debug>(sim: &[u8], want: &M) {
    let mut proto = vec![0; M::SIZE];
    want.write(&mut proto);
    assert_eq!(sim, &proto[..], "sim bytes vs d2_proto encode of {want:?}");
    assert_eq!(
        &M::decode(sim).unwrap(),
        want,
        "d2_proto decode of the sim bytes"
    );
}

/// A model holding a monster `guid` in mode 1 at `pos`.
fn monster_at(guid: u32, pos: (u16, u16)) -> (Model, UnitKey) {
    let mut m = Model::default();
    let k = UnitKey::new(MONSTER, guid);
    let u = m.put(k);
    u.mode = 1;
    u.position = Some(pos);
    (m, k)
}

/// Drives the client's queued unit handler with `b` for the monster
/// `guid` (placed at `pos`, the message's check point); returns the
/// stored mode request (code, record).
fn client_request(b: &[u8], guid: u32, pos: (u16, u16)) -> (u8, [i32; 7]) {
    let (mut m, k) = monster_at(guid, pos);
    m.recv(b);
    assert_eq!(m.drain(), 1, "{b:02x?}");
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    let r = m.unit(k).last_mode_request.expect("mode request stored");
    (r.code, r.record)
}

/// The skill-mode codes (0x15, 0x16) read the skill tables; the sweep
/// keeps the code byte off them so the monster request is the plain
/// mode change.
fn code(v: u32) -> u8 {
    match v as u8 {
        0x15 | 0x16 => 0x14,
        c => c,
    }
}

/// The 0x67 builders' path byte rewrite.
fn t_point(t: u32) -> u8 {
    match t {
        5 | 6 => 1,
        8 => 11,
        t => t as u8,
    }
}

// ---- 0x67 – 0x6D: the monster mode messages ------------------------------------------

#[test]
fn monster_move_0x67_one_layout() {
    for c in cases(8, 48) {
        let f = mm::Move {
            guid: c[0],
            code: code(c[1]),
            x: c[2] as u16,
            y: c[3] as u16,
            s: c[4] as u8,
            t: c[5] % 16,
            velocity: c[6] as u16,
            max_distance: c[7] as u8,
        };
        let sim = mm::monster_move(f);
        let want = gen::MonsterMove {
            guid: f.guid,
            code: f.code,
            x: f.x,
            y: f.y,
            arg: f.s,
            arg2: 0,
            path: t_point(f.t),
            velocity: f.velocity,
            arg3: f.max_distance,
        };
        one_layout(&sim, &want);
        // row 0x67: u16@6, u16@8, u8@0xA, u8@0xC,
        // i16@0xD, u8@0xF, u8@0xB.
        let r = client_request(&sim, f.guid, (1, 1));
        assert_eq!(
            r,
            (
                f.code,
                [
                    i32::from(f.x),
                    i32::from(f.y),
                    i32::from(f.s),
                    i32::from(want.path),
                    i32::from(f.velocity as i16),
                    i32::from(f.max_distance),
                    0,
                ]
            )
        );
    }
}

#[test]
fn monster_knockback_0x67_one_layout() {
    for c in cases(9, 48) {
        let f = mm::Knockback {
            guid: c[0],
            code: code(c[1]),
            x: c[2] as u16,
            y: c[3] as u16,
            d: c[4] as u8,
            f: c[5] as u8,
            t: c[6] % 16,
            velocity: c[7] as u16,
            e: c[8] as u8,
        };
        let sim = mm::knockback(f);
        let want = gen::MonsterMove {
            guid: f.guid,
            code: f.code,
            x: f.x,
            y: f.y,
            arg: f.d,
            arg2: f.f,
            path: t_point(f.t),
            velocity: f.velocity,
            arg3: f.e,
        };
        one_layout(&sim, &want);
        let r = client_request(&sim, f.guid, (1, 1));
        assert_eq!(r.1[2], i32::from(f.d));
        assert_eq!(r.1[6], i32::from(f.f));
        assert_eq!(r.1[5], i32::from(f.e));
    }
}

#[test]
fn monster_move_to_unit_0x68_one_layout() {
    for c in cases(10, 48) {
        let f = mm::MoveToUnit {
            guid: c[0],
            code: code(c[1]),
            x: c[2] as u16,
            y: c[3] as u16,
            a: c[4] as u8,
            b: c[5],
            s: c[6] as u8,
            t: c[7] % 16,
            velocity: c[8] as u16,
            max_distance: c[9] as u8,
        };
        let sim = mm::move_to_unit(f);
        let path = match f.t {
            5 | 6 => 2,
            8 => 11,
            t => t as u8,
        };
        let want = gen::MonsterMoveToTarget {
            guid: f.guid,
            code: f.code,
            x: f.x,
            y: f.y,
            target_type: f.a,
            target: f.b,
            arg: f.s,
            arg2: 0,
            path,
            velocity: f.velocity,
            arg3: f.max_distance,
        };
        one_layout(&sim, &want);
        // Row 0x68: check (u16@6, u16@8); u8@0xA, u32@0xB, u8@0xF,
        // u8@0x11, i16@0x12, u8@0x14, u8@0x10.
        let r = client_request(&sim, f.guid, (f.x, f.y));
        assert_eq!(
            r,
            (
                f.code,
                [
                    i32::from(f.a),
                    f.b as i32,
                    i32::from(f.s),
                    i32::from(path),
                    i32::from(f.velocity as i16),
                    i32::from(f.max_distance),
                    0,
                ]
            )
        );
    }
}

#[test]
fn monster_knockback_to_unit_0x68_one_layout() {
    for c in cases(11, 48) {
        let f = mm::KnockbackToUnit {
            guid: c[0],
            code: code(c[1]),
            x: c[2] as u16,
            y: c[3] as u16,
            a: c[4] as u8,
            b: c[5],
            d: c[6] as u8,
            f: c[7] as u8,
            t: c[8] % 16,
            velocity: c[9] as u16,
            e: c[10] as u8,
        };
        let sim = mm::knockback_to_unit(f);
        let want = gen::MonsterMoveToTarget {
            guid: f.guid,
            code: f.code,
            x: f.x,
            y: f.y,
            target_type: f.a,
            target: f.b,
            arg: f.d,
            arg2: f.f,
            path: if f.t == 8 { 11 } else { f.t as u8 },
            velocity: f.velocity,
            arg3: f.e,
        };
        one_layout(&sim, &want);
        let r = client_request(&sim, f.guid, (f.x, f.y));
        assert_eq!(r.1[2], i32::from(f.d));
        assert_eq!(r.1[6], i32::from(f.f));
        assert_eq!(r.1[5], i32::from(f.e));
    }
}

#[test]
fn monster_state_0x69_one_layout() {
    for c in cases(6, 48) {
        let (guid, cd, a, b, d, e) = (
            c[0],
            code(c[1]),
            c[2] as u16,
            c[3] as u16,
            c[4] as u8,
            c[5] as u8,
        );
        let sim = mm::monster_state(guid, cd, a, b, d, e);
        one_layout(
            &sim,
            &gen::MonsterState {
                guid,
                code: cd,
                a,
                b,
                d,
                e,
            },
        );
        let r = client_request(&sim, guid, (1, 1));
        assert_eq!(
            r,
            (
                cd,
                [
                    i32::from(a),
                    i32::from(b),
                    i32::from(d),
                    2,
                    0,
                    4,
                    i32::from(e)
                ]
            )
        );
    }
}

#[test]
fn monster_state_to_unit_0x6a_one_layout() {
    for c in cases(5, 48) {
        let (guid, cd, a, b, d) = (c[0], code(c[1]), c[2] as u8, c[3], c[4] as u8);
        let sim = mm::state_to_unit(guid, cd, a, b, d);
        one_layout(
            &sim,
            &gen::Unknown6A {
                guid,
                code: cd,
                target_type: a,
                target: b,
                d,
            },
        );
        let r = client_request(&sim, guid, (1, 1));
        assert_eq!(r, (cd, [i32::from(a), b as i32, i32::from(d), 2, 0, 4, 0]));
    }
}

#[test]
fn monster_action_0x6b_one_layout() {
    for c in cases(8, 48) {
        let (guid, cd, a, b, d, e, x, y) = (
            c[0],
            code(c[1]),
            c[2] as u16,
            c[3] as u16,
            c[4] as u8,
            c[5] as u8,
            c[6] as u16,
            c[7] as u16,
        );
        let sim = mm::monster_action(guid, cd, a, b, d, e, x, y);
        one_layout(
            &sim,
            &gen::MonsterAction {
                guid,
                code: cd,
                a,
                b,
                d,
                e,
                x,
                y,
            },
        );
        // Row 0x6B: check (u16@0xC, u16@0xE).
        let r = client_request(&sim, guid, (x, y));
        assert_eq!(
            r,
            (
                cd,
                [
                    i32::from(a),
                    i32::from(b),
                    i32::from(d),
                    2,
                    0,
                    4,
                    i32::from(e)
                ]
            )
        );
    }
}

#[test]
fn monster_attack_0x6c_one_layout() {
    for c in cases(7, 48) {
        let (guid, cd, a, b, d, x, y) = (
            c[0],
            code(c[1]),
            c[2] as u8,
            c[3],
            c[4] as u8,
            c[5] as u16,
            c[6] as u16,
        );
        let sim = mm::monster_attack(guid, cd, a, b, d, x, y);
        one_layout(
            &sim,
            &gen::MonsterAttack {
                guid,
                code: cd,
                target_type: a,
                target: b,
                d,
                x,
                y,
            },
        );
        let r = client_request(&sim, guid, (x, y));
        assert_eq!(r, (cd, [i32::from(a), b as i32, i32::from(d), 2, 0, 4, 0]));
    }
}

#[test]
fn monster_stop_0x6d_one_layout() {
    for c in cases(4, 48) {
        let (guid, x, y, life) = (c[0], c[1] as u16, c[2] as u16, c[3] as u8);
        let sim = mm::monster_stop(guid, x, y, life);
        one_layout(&sim, &gen::MonsterStop { guid, x, y, life });
        // Row 0x6D: check (u16@5, u16@7), code 7.
        let r = client_request(&sim, guid, (x, y));
        assert_eq!(
            r,
            (7, [i32::from(x), i32::from(y), i32::from(life), 2, 0, 4, 0])
        );
    }
}

// ---- 0x95, 0x96: the bit-packed vitals --------------------------------------------------

/// A model whose local player (0, 1) stands at `pos` in mode 2.
fn local_at(pos: (u16, u16)) -> Model {
    let mut m = Model::default();
    let k = UnitKey::new(PLAYER, 1);
    let u = m.put(k);
    u.position = Some(pos);
    u.mode = 2;
    m.w.local_player = Some(k);
    m
}

#[test]
fn life_mana_update2_0x95_one_layout() {
    for c in cases(7, 96) {
        // The builder cuts each value to its width.
        let (life, mana, stamina) = (c[0] & 0x7FFF, c[1] & 0x7FFF, c[2] & 0x7FFF);
        let (x, y, dx, dy) = (c[3] as u16, c[4] as u16, c[5] as u8, c[6] as u8);
        let sim = life_mana_update2(life as i32, mana as i32, stamina as i32, x, y, dx, dy);
        one_layout(
            &sim,
            &gen::LifeManaUpdate2 {
                life: life as u16,
                mana: mana as u16,
                stamina: stamina as u16,
                x,
                y,
                dx,
                dy,
            },
        );
        // The builder cuts a wider value; the bytes are those of the cut
        // value.
        let wide = life_mana_update2(
            (life | 0x8000) as i32,
            mana as i32,
            stamina as i32,
            x,
            y,
            dx,
            dy,
        );
        assert_eq!(wide, sim);
        // The typed encode cuts the same way (q-fix-proto-packed-cut).
        let typed = gen::LifeManaUpdate2 {
            life: (life | 0x8000) as u16,
            mana: mana as u16,
            stamina: stamina as u16,
            x,
            y,
            dx,
            dy,
        }
        .encode();
        assert_eq!(typed.to_vec(), sim.to_vec());
        // Stats 6, 8, 10 := v << 8; the check at
        // (x, y) stores the server point (x = 0 or y = 0: no check).
        let mut m = local_at((x, y));
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let u = m.unit(UnitKey::new(PLAYER, 1));
        assert_eq!(
            (u.stat(6), u.stat(8), u.stat(10)),
            (
                (life << 8) as i32,
                (mana << 8) as i32,
                (stamina << 8) as i32
            )
        );
        let point = if x == 0 || y == 0 { (0, 0) } else { (x, y) };
        assert_eq!(u.server_point, point);
        assert!(m.w.outgoing.is_empty());
    }
}

#[test]
fn walk_verify_0x96_one_layout() {
    for c in cases(4, 96) {
        let stamina = c[0] & 0x7FFF;
        let (x, y, dx, dy) = (c[1] as u16, c[2] as u16, c[3] as i8, (c[3] >> 8) as i8);
        let sim = walk_verify(stamina, x, y, dx, dy);
        one_layout(
            &sim,
            &gen::WalkVerify {
                stamina: stamina as u16,
                x,
                y,
                dx: dx as u8,
                dy: dy as u8,
            },
        );
        assert_eq!(walk_verify(stamina | 0x8000, x, y, dx, dy), sim);
        // Bit order: id 0–7, stamina 8–22, x 23–38, y 39–54, dx 55–62,
        // dy 63–70, bit 71 clear (LSB first from byte 0).
        let v = sim
            .iter()
            .rev()
            .fold(0u128, |a, &b| (a << 8) | u128::from(b));
        assert_eq!(v & 0xFF, 0x96);
        assert_eq!((v >> 8) & 0x7FFF, u128::from(stamina));
        assert_eq!((v >> 23) & 0xFFFF, u128::from(x));
        assert_eq!((v >> 39) & 0xFFFF, u128::from(y));
        assert_eq!((v >> 55) & 0xFF, u128::from(dx as u8));
        assert_eq!((v >> 63) & 0xFF, u128::from(dy as u8));
        assert_eq!(v >> 71, 0);
        let mut m = local_at((x, y));
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let u = m.unit(UnitKey::new(PLAYER, 1));
        assert_eq!(u.stat(10), (stamina << 8) as i32);
        let point = if x == 0 || y == 0 { (0, 0) } else { (x, y) };
        assert_eq!(u.server_point, point);
        assert!(m.w.outgoing.is_empty());
    }
}

// ---- 0x74 – 0x7E -----------------------------------------------------------------------

#[test]
fn player_corpse_assign_0x74_one_layout() {
    for c in cases(1, 32) {
        let sim = corpse_assign(c[0], c[0]);
        one_layout(
            &sim,
            &gen::PlayerCorpseAssign {
                flag: 1,
                player: c[0],
                corpse: c[0],
            },
        );
        // step 1: flag ≠ 0 and the player present → mode 0.
        let mut m = Model::default();
        let k = UnitKey::new(PLAYER, c[0]);
        m.put(k).mode = 1;
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        assert_eq!(m.unit(k).mode, 0);
    }
}

#[test]
fn player_in_proximity_0x76_one_layout() {
    for c in cases(2, 32) {
        let (ty, guid) = (c[0] as u8, c[1]);
        let sim = unit::unit_ref(0x76, ty, guid);
        one_layout(&sim, &gen::PlayerInProximity { type_: ty, guid });
        let mut m = Model::default();
        m.recv(&sim);
        assert_eq!(
            m.out,
            [Output::OverheadClear {
                unit: UnitKey::new(ty, guid)
            }]
        );
    }
}

#[test]
fn trade_action_0x77_one_layout() {
    for code in 0..=u8::MAX {
        let sim = trade_action(code);
        one_layout(&sim, &gen::TradeAction { code });
        let mut m = Model::default();
        m.recv(&sim);
        assert!(
            matches!(m.out[..], [Output::TradeAction { code: c, .. }] if c == code),
            "{:?}",
            m.out
        );
    }
}

#[test]
fn pet_action_0x7a_one_layout() {
    for c in cases(5, 48) {
        let (action, pet_type, class, owner, pet) =
            (c[0] as u8, c[1] as u8, c[2] as u16, c[3], c[4]);
        // `pets::pet_action(action, pet_type, class, pet, owner)`: pet
        // before owner in the arguments, owner first on the wire.
        let sim = pets::pet_action(action, pet_type, class, pet, owner);
        one_layout(
            &sim,
            &gen::PetAction {
                action,
                pet_type,
                class,
                owner,
                pet,
            },
        );
        // Action ≠ 0 → set (pet u32@9, owner u32@5,
        // type u8@2, class u16@3); 0 → remove.
        let mut m = Model::default();
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        if action != 0 {
            let r = m.w.pets.iter().find(|r| r.pet == pet).expect("record");
            assert_eq!((r.owner, r.pet_type, r.class), (owner, pet_type, class));
        } else {
            assert!(m.w.pets.is_empty());
        }
    }
    // The recorded remove (`hirelings/pets.rs` docs).
    assert_eq!(
        pets::pet_action(0, 0, 0, 0x0D, 0),
        [0x7A, 0, 0, 0, 0, 0, 0, 0, 0, 0x0D, 0, 0, 0]
    );
}

#[test]
fn assign_hotkey_0x7b_one_layout() {
    for c in cases(4, 48) {
        let (slot, skill, flag, item) = (c[0] as u8, c[1] as i16, c[2] & 1 != 0, c[3]);
        let sim = unit::assign_hotkey(slot, skill, flag, item);
        let word = (skill as u16 & 0xFFF) | if flag { 0x8000 } else { 0 };
        one_layout(
            &sim,
            &gen::AssignHotkey {
                slot,
                skill: word,
                item,
            },
        );
        // Skill = word & 0xFFF (past the table count →
        // −1), left = bit 0x8000.
        let mut m = Model::default();
        m.recv(&sim);
        let count = m.inputs.tables.skills.len() as i32;
        let s = i32::from(word & 0xFFF);
        assert_eq!(
            m.out,
            [Output::HotkeyAssign {
                slot,
                skill: if s > count { -1 } else { s },
                left: flag,
                item,
            }]
        );
    }
}

#[test]
fn use_scroll_0x7c_one_layout() {
    for c in cases(2, 32) {
        let (ty, guid) = (c[0] as u8, c[1]);
        let sim = layouts::item_used(ty, guid);
        one_layout(&sim, &gen::UseScroll { type_: ty, guid });
        let mut m = Model::default();
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    }
}

#[test]
fn set_item_state_0x7d_one_layout() {
    for c in cases(5, 48) {
        let (ty, owner, item, flag, state) = (c[0] as u8, c[1], c[2], c[3], c[4]);
        let sim = layouts::item_state(ty, owner, item, flag, state);
        one_layout(
            &sim,
            &gen::SetItemState {
                type_: ty,
                unit: owner,
                item,
                flag,
                state,
            },
        );
        let mut m = Model::default();
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    }
}

#[test]
fn game_entry_done_0x7e_one_layout() {
    // Id only (5 bytes; bytes 1–4 unwritten in the base game, 0 here).
    let sim = unit::GAME_ENTRY_DONE;
    assert_eq!(sim.len(), 5);
    assert!(matches!(
        d2_proto::SERVER_MESSAGES
            .iter()
            .find(|r| r.id == 0x7E)
            .map(|r| r.size),
        Some(d2_proto::schema::SizeRule::Fixed(5))
    ));
    let mut m = Model::default();
    m.recv(&sim);
    assert!(
        matches!(m.out[..], [Output::CommonCof { .. }]),
        "{:?}",
        m.out
    );
}

// ---- 0x81, 0x82 ---------------------------------------------------------------------------

#[test]
fn assign_merc_0x81_one_layout() {
    for c in cases(5, 48) {
        let (class, owner, merc, seed, name) = (c[0] as u16, c[1], c[2], c[3], c[4]);
        let sim = pets::assign_merc(class, owner, merc, seed, name);
        one_layout(
            &sim,
            &gen::AssignMerc {
                pet_type: 7,
                class,
                owner,
                merc,
                seed,
                name,
            },
        );
        // Set (merc u32@8, owner u32@4, type, class),
        // then extra := {u32@0xC, u32@0x10, 0}.
        let mut m = Model::default();
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let r = m.w.pets.iter().find(|r| r.pet == merc).expect("record");
        assert_eq!(
            (r.owner, r.pet_type, r.class, r.extra),
            (owner, 7, class, Some([seed, name, 0]))
        );
    }
}

#[test]
fn portal_ownership_0x82_one_layout() {
    for (i, c) in cases(3, 32).into_iter().enumerate() {
        let (owner, portal, portal2) = (c[0], c[1], c[2]);
        let name: Vec<u8> = (0..i % 20).map(|k| b'a' + k as u8).collect();
        let sim = unit::portal_ownership(owner, &name, portal, portal2);
        let mut want_name = [0u8; 16];
        let n = name.len().min(15);
        want_name[..n].copy_from_slice(&name[..n]);
        one_layout(
            &sim,
            &gen::PortalOwnership {
                owner,
                name: want_name,
                portal,
                portal2,
            },
        );
        // The portal objects get the name; with
        // portal2 ≠ −1 the owner's roster record gets both GUIDs.
        let mut m = Model::default();
        m.put(UnitKey::new(OBJECT, portal)).kind = KindData::Object(ObjectData::default());
        m.w.roster.push(RosterRecord {
            guid: owner,
            ..RosterRecord::default()
        });
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let KindData::Object(d) = &m.unit(UnitKey::new(OBJECT, portal)).kind else {
            unreachable!()
        };
        assert_eq!(d.owner_name, Some(want_name));
        if portal2 != u32::MAX && owner != u32::MAX {
            assert_eq!(m.w.roster[0].portals, (portal, portal2));
        }
    }
}

// ---- 0x89 – 0x93: sim builders inline (no fn), proto vs client ---------------------------

#[test]
fn corpse_assign_0x8e_one_layout() {
    for c in cases(2, 32) {
        let (player, corpse) = (c[0], c[1]);
        if player == u32::MAX {
            continue;
        }
        // The wired sender (`wiring/inventory/pending.rs` `corpse_taken`)
        // writes `8E 00 <player> <corpse>`.
        let mut sim = vec![0x8E, 0];
        sim.extend_from_slice(&player.to_le_bytes());
        sim.extend_from_slice(&corpse.to_le_bytes());
        one_layout(
            &sim,
            &gen::CorpseAssign {
                flag: 0,
                player,
                corpse,
            },
        );
        // Flag 1 adds the corpse to the player's record, 0 removes it.
        let mut m = Model::default();
        m.w.roster.push(RosterRecord {
            guid: player,
            ..RosterRecord::default()
        });
        let add = gen::CorpseAssign {
            flag: 1,
            player,
            corpse,
        }
        .encode();
        m.recv(&add);
        assert!(m.w.roster[0].corpses.contains(&corpse));
        m.recv(&sim);
        assert!(!m.w.roster[0].corpses.contains(&corpse));
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    }
}

#[test]
fn npc_wants_interact_0x8a_one_layout() {
    for c in cases(1, 32) {
        // `world/quests.rs` active-function sender: `8A 01 <npc GUID>`.
        let mut sim = vec![0x8A, 1];
        sim.extend_from_slice(&c[0].to_le_bytes());
        one_layout(
            &sim,
            &gen::NpcWantsInteract {
                type_: 1,
                guid: c[0],
            },
        );
        let mut m = Model::default();
        m.recv(&sim);
        assert!(
            matches!(m.out[..], [Output::NpcInteract { unit, .. }] if unit == UnitKey::new(MONSTER, c[0])),
            "{:?}",
            m.out
        );
    }
}

#[test]
fn npc_gossip_act_0x91_one_layout() {
    for c in cases(13, 32) {
        let slots: [u16; 12] = std::array::from_fn(|k| c[k + 1] as u16);
        let want = gen::NpcGossipAct {
            act: c[0] as u8,
            npc0: slots[0],
            npc1: slots[1],
            npc2: slots[2],
            npc3: slots[3],
            npc4: slots[4],
            npc5: slots[5],
            npc6: slots[6],
            npc7: slots[7],
            npc8: slots[8],
            npc9: slots[9],
            npc10: slots[10],
            npc11: slots[11],
        };
        let b = want.encode();
        assert_eq!(gen::NpcGossipAct::decode(&b).unwrap(), want);
        let mut m = Model::default();
        m.recv(&b);
        assert_eq!(m.out, [Output::NpcIntro { slots }]);
    }
}

#[test]
fn pong_0x8f_one_layout() {
    for c in cases(8, 16) {
        let want = gen::Pong {
            f1: c[0],
            f5: c[1],
            f9: c[2],
            f13: c[3],
            f17: c[4],
            f21: c[5],
            f25: c[6],
            f29: c[7],
        };
        let b = want.encode();
        let mut m = Model::default();
        m.recv(&b);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let mut expect: [u32; 8] = c[..8].try_into().unwrap();
        expect[4] = 0;
        assert_eq!(m.w.ping.pong, expect);
    }
}

// ---- 0x97 – 0x9B --------------------------------------------------------------------------

#[test]
fn weapon_switch_0x97_one_layout() {
    // The sender (`wiring/inventory/swap.rs` `WEAPON_SWITCH`) sends `[0x97]`.
    one_layout(&[0x97], &gen::WeaponSwitch);
}

#[test]
fn unknown98_0x98_one_layout() {
    for c in cases(2, 32) {
        let (guid, v) = (c[0], c[1] as u16);
        let sim = unit::unknown98(guid, v);
        one_layout(&sim, &gen::Unknown98 { guid, f5: v });
        // Monster data +0x40 := u16@5 (0xFFFF → −1).
        let mut m = Model::default();
        let k = UnitKey::new(MONSTER, guid);
        m.put(k).kind = KindData::Monster(Box::default());
        m.recv(&sim);
        let KindData::Monster(d) = &m.unit(k).kind else {
            unreachable!()
        };
        assert_eq!(d.f40, Some(if v == 0xFFFF { -1 } else { i32::from(v) }));
    }
}

#[test]
fn skill_events_0x99_0x9a_one_layout() {
    for c in cases(8, 32) {
        let (ty, guid, skill, level, tt, target, w) = (
            PLAYER,
            c[0],
            c[1] as u16,
            c[2] as u8,
            MONSTER,
            c[3],
            c[4] as u16,
        );
        let b = gen::SkillTriggered {
            type_: ty,
            guid,
            skill,
            f8: level,
            target_type: tt,
            target,
            f14: w,
        }
        .encode();
        // The d2-sim record's sender (`event_records`, flag 1).
        assert_eq!(
            d2_sim::wiring::action::event_records::skill_event_on_unit(
                ty, guid, skill, level, tt, target, w
            )
            .to_vec(),
            b.to_vec()
        );
        let mut m = Model::default();
        m.put(UnitKey::new(ty, guid));
        m.put(UnitKey::new(tt, target));
        m.recv(&b);
        assert_eq!(
            m.out,
            [Output::SkillEvent {
                unit: UnitKey::new(ty, guid),
                skill,
                level,
                target: SkillTarget::Unit(UnitKey::new(tt, target)),
                w,
            }]
        );
        // 0x9A: skill u32@6 (the handler reads the low u16), level u8@10.
        let (x, y) = ((c[5] as u16).max(1), (c[6] as u16).max(1));
        let b = gen::Unknown9A {
            type_: ty,
            guid,
            skill: c[1],
            f10: level,
            x,
            y,
            f15: w,
        }
        .encode();
        assert_eq!(
            d2_sim::wiring::action::event_records::skill_event_on_point(
                ty, guid, c[1], level, x, y, w
            )
            .to_vec(),
            b.to_vec()
        );
        let mut m = Model::default();
        m.put(UnitKey::new(ty, guid));
        m.recv(&b);
        assert_eq!(
            m.out,
            [Output::SkillEvent {
                unit: UnitKey::new(ty, guid),
                skill,
                level,
                target: SkillTarget::Point(x, y),
                w,
            }]
        );
    }
}

#[test]
fn merc_dead_0x9b_one_layout() {
    for c in cases(2, 48) {
        let (name, cost) = (c[0] as u16, c[1]);
        let sim = pets::merc_dead_message(name, cost);
        one_layout(&sim, &gen::Unknown9B { name, cost });
        // The handler reads u16@1 and u16@3 (the
        // sender's u32@3 cut to its low half).
        let mut m = Model::default();
        m.recv(&sim);
        assert_eq!(
            m.out,
            [Output::MercRevive {
                state: name,
                value: cost as u16,
            }]
        );
    }
    // The resurrect form.
    one_layout(
        &resurrect_message(),
        &gen::Unknown9B {
            name: 0xFFFF,
            cost: 0,
        },
    );
}
