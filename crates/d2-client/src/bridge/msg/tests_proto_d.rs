//! Proto contract tests (q-proto-audit, part d): S→C 0x3E, 0x94 and
//! 0x9C–0xB4. For each id with a `d2-sim` (or `d2-server`) builder the
//! builder's bytes are checked against the one layout: the generated
//! `d2_proto::server` encode for the fixed-size ids, the
//! `server-messages.tsv` field offsets and size rule for the variable
//! ids; then the client handler of `HANDLERS` reads the same values
//! back into the model. Ids without a builder are checked from the
//! d2-proto encode (or the documented layout) to the client.
//!
//! Inputs: edge values (0, 1, 0x7F / 0x80, 0xFF / 0xFFFF / 0x8000,
//! u32::MAX) and a fixed xorshift sweep.

use std::collections::BTreeMap;

use d2_proto::item_bits::{self, CodeFacts, IscSave, ItemLookup, Location};
use d2_proto::s2c::{parse, Message as S2c, WardenRequest};
use d2_proto::schema::{FieldType, Size};
use d2_proto::server as gen;
use d2_proto::transport::{server_size, split_server_buffer};
use d2_proto::SERVER_MESSAGES;
use d2_sim::items::bitstream::{self, header_flags, Isc, StatEntry, StreamItem};
use d2_sim::items::moves::layouts::{item_owned, item_world};
use d2_sim::monsters::init::type_flag;
use d2_sim::units::messages::{
    base_skill_levels, clamp_send, set_state, state_ref, unit_states, SendStat, SentState,
};
use d2_sim::wiring::action::monster_add::{
    assign_monster, AssignMonster, StatEntry as MonStat, TypeBlock,
};
use d2_sim::world::hirelings::level::{exp_delta_message, stat_message};

use super::super::output::Output;
use super::super::world::{
    KindData, MonsterClass, PlayerData, SkillRow, StatSend, StateRow, UnitKey, ITEM, MONSTER,
    PLAYER,
};
use super::support::{hex, Model};

const U8S: [u8; 5] = [0, 1, 0x7F, 0x80, 0xFF];
const U16S: [u16; 5] = [0, 1, 0x7FFF, 0x8000, 0xFFFF];
const U32S: [u32; 7] = [0, 1, 0xFE, 0xFF, 0x7FFF_FFFF, 0x8000_0000, u32::MAX];

/// Fixed xorshift sweep (test input only).
fn sweep(n: usize) -> impl Iterator<Item = u32> {
    let mut s = 0x2545_F491_u32;
    (0..n).map(move |_| {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    })
}

/// The `server-messages.tsv` field `name` of message `b` (by its id),
/// little-endian at the TSV offset.
fn tsv(b: &[u8], name: &str) -> u32 {
    let m = &SERVER_MESSAGES[usize::from(b[0])];
    let f = m
        .layout
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("0x{:02X} has no TSV field {name}", b[0]));
    let at = usize::from(f.offset.expect("offset"));
    match f.ty {
        FieldType::U8 => u32::from(b[at]),
        FieldType::U16 => u32::from(u16::from_le_bytes([b[at], b[at + 1]])),
        FieldType::U32 => u32::from_le_bytes(b[at..at + 4].try_into().unwrap()),
        t => panic!("0x{:02X} {name}: {t:?} is not a number", b[0]),
    }
}

/// The bytes from the TSV tail field `name` on.
fn tsv_tail<'a>(b: &'a [u8], name: &str) -> &'a [u8] {
    let m = &SERVER_MESSAGES[usize::from(b[0])];
    let f = m.layout.iter().find(|f| f.name == name).unwrap();
    assert_eq!(f.ty, FieldType::Tail, "0x{:02X} {name} is the tail", b[0]);
    &b[usize::from(f.offset.unwrap())..]
}

/// The size rule gives exactly the message, and a split of it
/// twice in one buffer yields both whole.
fn one_size(b: &[u8]) {
    assert_eq!(
        server_size(b),
        Size::Bytes(b.len()),
        "size rule of {b:02x?}"
    );
    let two = [b, b].concat();
    let s = split_server_buffer(&two).unwrap();
    assert_eq!(s.messages, vec![b, b]);
    assert!(s.discarded.is_empty());
}

/// LSB-first packer of (value, bits) fields.
fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for &(v, n) in fields {
        for i in 0..n {
            if at.is_multiple_of(8) {
                out.push(0);
            }
            if (v >> i) & 1 != 0 {
                *out.last_mut().unwrap() |= 1 << (at % 8);
            }
            at += 1;
        }
    }
    out
}

/// `n` bits of `v` read signed.
fn signed(v: u32, n: u32) -> i32 {
    let v = if n >= 32 { v } else { v & ((1u32 << n) - 1) };
    if n > 0 && n < 32 && v & (1 << (n - 1)) != 0 {
        (v | (u32::MAX << n)) as i32
    } else {
        v as i32
    }
}

// ---- 0x9E–0xA2 hireling stats

#[test]
fn merc_stat_0x9e_0xa2_one_layout() {
    let guids = U32S.iter().copied().chain(sweep(4));
    for guid in guids {
        for stat in [0u16, 1, 12, 0x7F, 0x80, 0xFE] {
            let values = U32S
                .iter()
                .copied()
                .chain([0xFFFE, 0xFFFF, 0x1_0000])
                .chain(sweep(6));
            for value in values {
                let sim = stat_message(stat, guid, value).unwrap();
                let s = stat as u8;
                let (proto, want): (Vec<u8>, S2c) = if value < 0xFF {
                    let g = gen::MercStatByte {
                        stat: s,
                        merc: guid,
                        value: value as u8,
                    };
                    (g.encode().to_vec(), S2c::MercStatByte(g))
                } else if value < 0xFFFF {
                    let g = gen::MercStatWord {
                        stat: s,
                        merc: guid,
                        value: value as u16,
                    };
                    (g.encode().to_vec(), S2c::MercStatWord(g))
                } else {
                    let g = gen::MercStatDword {
                        stat: s,
                        merc: guid,
                        value,
                    };
                    (g.encode().to_vec(), S2c::MercStatDword(g))
                };
                assert_eq!(sim, proto, "sim vs d2_proto encode");
                assert_eq!(parse(&sim).unwrap(), want);
                one_size(&sim);
                let key = UnitKey::new(MONSTER, guid);
                let mut m = Model::default();
                m.put(key);
                m.recv(&sim);
                assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
                assert_eq!(m.unit(key).stat(stat), value as i32);
            }
        }
        // Stat id > 0xFE: a fatal assertion in the base game.
        assert!(stat_message(0xFF, guid, 1).is_err());
    }
}

#[test]
fn merc_add_exp_0xa1_0xa2_one_layout() {
    let olds = U32S.iter().copied().chain(sweep(5));
    for old in olds {
        let deltas = [
            0u32,
            1,
            0x7F,
            0xFE,
            0xFF,
            0x8000,
            0xFFFE,
            0xFFFF,
            0x1_0000,
            u32::MAX,
        ];
        for delta in deltas.into_iter().chain(sweep(4)) {
            let new = old.wrapping_add(delta);
            let (stat, guid) = (13u8, 0x8000_0001 ^ old);
            let sim = exp_delta_message(stat, guid, old, new);
            let (proto, want): (Vec<u8>, S2c) = if delta < 0xFF {
                let g = gen::MercAddExpByte {
                    stat,
                    merc: guid,
                    delta: delta as u8,
                };
                (g.encode().to_vec(), S2c::MercAddExpByte(g))
            } else if delta < 0xFFFF {
                let g = gen::MercAddExpWord {
                    stat,
                    merc: guid,
                    delta: delta as u16,
                };
                (g.encode().to_vec(), S2c::MercAddExpWord(g))
            } else {
                // Edge case 7: 0xA0 carries the old value.
                let g = gen::MercStatDword {
                    stat,
                    merc: guid,
                    value: old,
                };
                (g.encode().to_vec(), S2c::MercStatDword(g))
            };
            assert_eq!(sim, proto);
            assert_eq!(parse(&sim).unwrap(), want);
            one_size(&sim);
            let key = UnitKey::new(MONSTER, guid);
            let mut m = Model::default();
            m.put(key).stats.insert(13, old as i32);
            m.recv(&sim);
            let want = if delta < 0xFFFF { new } else { old };
            assert_eq!(m.unit(key).stat(13), want as i32);
        }
    }
}

// ---- 0xA7 / 0xA8 / 0xA9 / 0xAA states

#[test]
fn delayed_end_state_0xa7_0xa9_one_layout() {
    for ty in [0u8, 1, 2, 3, 4, 5] {
        for guid in U32S.iter().copied().chain(sweep(4)) {
            for state in U8S.iter().copied().chain([9, 105, 0xFE]) {
                let a7 = state_ref(0xA7, ty, guid, state);
                let g7 = gen::DelayedState {
                    type_: ty,
                    guid,
                    state,
                };
                assert_eq!(a7, g7.encode());
                assert_eq!(parse(&a7).unwrap(), S2c::DelayedState(g7));
                let a9 = state_ref(0xA9, ty, guid, state);
                let g9 = gen::EndState {
                    type_: ty,
                    guid,
                    state,
                };
                assert_eq!(a9, g9.encode());
                assert_eq!(parse(&a9).unwrap(), S2c::EndState(g9));
                one_size(&a7);
                one_size(&a9);
                let key = UnitKey::new(ty, guid);
                let mut m = Model::default();
                m.inputs.tables.states = vec![StateRow::default(); 256];
                m.put(key);
                m.recv(&a7);
                assert!(m.unit(key).states.contains(&state));
                m.recv(&a9);
                assert!(!m.unit(key).states.contains(&state));
                assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
            }
        }
    }
}

/// The stat send columns both sides use: id → (bits, param bits,
/// signed). Stat 7 has `Send Bits` 0 (skipped by the writer).
const SEND: [(u16, u8, u8, bool); 7] = [
    (1, 3, 0, true),
    (2, 8, 4, false),
    (3, 32, 0, false),
    (4, 16, 0, true),
    (5, 6, 2, true),
    (7, 0, 0, false),
    (172, 2, 0, false),
];

fn send_row(id: u16) -> Option<SendStat> {
    SEND.iter()
        .find(|r| r.0 == id)
        .map(|&(_, bits, param_bits, signed)| SendStat {
            bits,
            param_bits,
            signed,
        })
}

fn client_stats() -> Vec<StatSend> {
    let mut t = vec![StatSend::default(); 360];
    for (id, bits, param_bits, signed) in SEND {
        t[usize::from(id)] = StatSend {
            bits,
            param_bits,
            signed,
            update_anim_rate: false,
        };
    }
    t
}

/// The state list the client should hold after one state's entries:
/// the writer's first 16 entries with send bits, each clamped
/// and read back with a signed param; value 0
/// removes.
fn expect_list(entries: &[(u16, u16, i32)]) -> BTreeMap<(u16, u16), i32> {
    let mut l = BTreeMap::new();
    for &(param, id, value) in entries.iter().take(16) {
        let Some(r) = send_row(id).filter(|r| r.bits != 0) else {
            continue;
        };
        let p = if r.param_bits > 0 {
            signed(u32::from(param), u32::from(r.param_bits)) as u16
        } else {
            0
        };
        let v = clamp_send(value, r.bits, r.signed);
        let v = if r.bits < 32 && r.signed {
            signed(v as u32, u32::from(r.bits))
        } else if r.bits < 32 {
            (v as u32 & ((1u32 << r.bits) - 1)) as i32
        } else {
            v
        };
        if v == 0 {
            l.remove(&(id, p));
        } else {
            l.insert((id, p), v);
        }
    }
    l
}

/// Entry lists (param, id, value) the state tests send.
fn entry_cases() -> Vec<Vec<(u16, u16, i32)>> {
    let mut cases = vec![
        vec![(0, 172, 2)],
        vec![(0, 1, 3), (0, 1, -4)],
        vec![(0, 1, 100), (0, 1, -100)],
        vec![(5, 2, 0xFF), (0xF, 2, 0x80), (7, 2, 300)],
        vec![
            (0, 3, -1),
            (0, 3, i32::MIN),
            (0, 4, 0x7FFF),
            (0, 4, -0x8000),
        ],
        vec![(1, 5, -32), (3, 5, 31), (0, 7, 9), (0, 9, 9)],
        vec![(0, 4, 1); 20],
    ];
    let mut rng = sweep(1 << 12);
    for _ in 0..8 {
        let n = (rng.next().unwrap() % 6) as usize + 1;
        let e = (0..n)
            .map(|_| {
                let r = rng.next().unwrap();
                let id = SEND[(r % SEND.len() as u32) as usize].0;
                (
                    (r >> 8) as u16 & 0xF,
                    id,
                    rng.next().unwrap() as i32 >> (r % 31),
                )
            })
            .collect();
        cases.push(e);
    }
    cases
}

#[test]
fn set_state_0xa8_one_layout() {
    for (i, entries) in entry_cases().iter().enumerate() {
        let guid = [0, 1, 0x8000_0000, u32::MAX][i % 4];
        let state = [0u8, 9, 0x7F, 0xFE][i % 4];
        let sim = set_state(PLAYER, guid, state, entries, send_row);
        assert_eq!(tsv(&sim, "type"), u32::from(PLAYER));
        assert_eq!(tsv(&sim, "guid"), guid);
        assert_eq!(tsv(&sim, "size"), sim.len() as u32);
        assert_eq!(tsv(&sim, "state"), u32::from(state));
        assert_eq!(tsv_tail(&sim, "data").len(), sim.len() - 8);
        one_size(&sim);
        let key = UnitKey::new(PLAYER, guid);
        let mut m = Model::default();
        m.inputs.tables.stats = client_stats();
        m.inputs.tables.states = vec![StateRow::default(); 256];
        m.put(key);
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let u = m.unit(key);
        assert!(u.states.contains(&state));
        let got = u.state_lists.get(&state).cloned().unwrap_or_default();
        assert_eq!(got, expect_list(entries), "case {i}: {entries:?}");
    }
}

#[test]
fn set_state_0xa8_param_with_the_top_bit_is_signed_and_keeps_16_bits() {
    // Stat 2 sends 4 param bits: raw 0xF reads as -1, kept as its low 16
    // bits, 0x10000 - 2^4 + 0xF; raw 0x8 reads as -8 (0xFFF8).
    let key = UnitKey::new(PLAYER, 5);
    let mut m = Model::default();
    m.inputs.tables.stats = client_stats();
    m.inputs.tables.states = vec![StateRow::default(); 256];
    m.put(key);
    let sim = set_state(
        PLAYER,
        5,
        9,
        &[(0xF, 2, 7), (0x8, 2, 9), (0x7, 2, 11)],
        send_row,
    );
    m.recv(&sim);
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    let l = m.unit(key).state_lists.get(&9).cloned().unwrap_or_default();
    let key = |low: u32| (2u16, (0x10000u32 - 16 + low) as u16);
    assert_eq!(l.get(&key(0xF)), Some(&7));
    assert_eq!(l.get(&key(0x8)), Some(&9));
    assert_eq!(l.get(&(2, 7)), Some(&11));
    assert_eq!(l.len(), 3);
}

#[test]
fn add_unit_states_0xaa_one_layout() {
    let cases = entry_cases();
    let mut rng = sweep(1 << 12);
    for (i, _) in cases.iter().enumerate() {
        // Up to four distinct states, each with or without a list.
        let n = i % 5;
        let mut states = Vec::new();
        for k in 0..n {
            let has = rng.next().unwrap() % 3;
            states.push(SentState {
                state: (k * 50 + i) as u16 % 255,
                entries: match has {
                    0 => None,
                    1 => Some(Vec::new()),
                    _ => Some(cases[(i + k) % cases.len()].clone()),
                },
            });
        }
        states.sort_by_key(|s| s.state);
        states.dedup_by_key(|s| s.state);
        let guid = [0, 1, 0x7FFF_FFFF, u32::MAX, 0x1234_5678][i % 5];
        let ty = [PLAYER, MONSTER, 2, 3][i % 4];
        let sim = unit_states(ty, guid, &states, send_row);
        assert_eq!(tsv(&sim, "type"), u32::from(ty));
        assert_eq!(tsv(&sim, "guid"), guid);
        assert_eq!(tsv(&sim, "size"), sim.len() as u32);
        assert_eq!(tsv_tail(&sim, "data").len(), sim.len() - 7);
        one_size(&sim);
        let key = UnitKey::new(ty, guid);
        let mut m = Model::default();
        m.inputs.tables.stats = client_stats();
        m.inputs.tables.states = vec![StateRow::default(); 256];
        m.put(key);
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "case {i}: {:?}", m.rejected());
        let u = m.unit(key);
        for s in &states {
            let st = s.state as u8;
            assert!(u.states.contains(&st), "case {i}: state {st}");
            let want = match s.entries.as_deref() {
                None | Some([]) => BTreeMap::new(),
                Some(e) => expect_list(e),
            };
            let got = u.state_lists.get(&st).cloned().unwrap_or_default();
            assert_eq!(got, want, "case {i}: state {st}");
        }
        assert_eq!(u.states.len(), states.len());
    }
}

// ---- 0xAC AssignMonster

#[test]
fn assign_monster_0xac_one_layout() {
    let mut rng = sweep(1 << 16);
    let mut next = || rng.next().unwrap();
    // Choice counts (monstats2 +0x15 + i) shared by both sides.
    let counts: [u8; 16] = [0, 1, 2, 3, 4, 5, 8, 9, 16, 17, 255, 2, 1, 0, 7, 33];
    let width = |c: u8| -> u32 {
        if c < 3 {
            1
        } else {
            u32::BITS - u32::from(c - 1).leading_zeros()
        }
    };
    let stats_t = client_stats();
    for case in 0..40u32 {
        let r = next();
        let mut components = [0u8; 16];
        if r & 1 != 0 {
            for (c, &n) in components.iter_mut().zip(&counts) {
                *c = (next() & ((1 << width(n)) - 1)) as u8;
            }
        }
        let type_block = (r & 2 != 0).then(|| {
            let flags = (next() as u16)
                & (type_flag::CHAMPION
                    | type_flag::UNIQUE
                    | type_flag::SUPERUNIQUE
                    | type_flag::MINION
                    | type_flag::GHOSTLY);
            let n = (next() % 10) as usize;
            TypeBlock {
                flags,
                hc_idx: [0, 1, 0x7FFF, 0x8000, 0xFFFF][case as usize % 5],
                umods: (0..n).map(|_| (next() % 255) as u8 + 1).collect(),
                name_seed: next() as u16,
                hireling_owner: (r & 4 != 0).then(&mut next),
            }
        });
        let source = (r & 8 != 0).then(&mut next);
        let stats = match (r >> 4) % 3 {
            0 => None,
            1 => Some(Vec::new()),
            _ => Some(
                (0..(next() % 18))
                    .map(|_| {
                        let id = SEND[(next() % SEND.len() as u32) as usize].0;
                        let row = send_row(id).unwrap();
                        // Values inside the field (the writer does not clamp).
                        let v = if row.bits >= 32 {
                            next() as i32
                        } else {
                            signed(next(), u32::from(row.bits))
                        };
                        let v = if row.signed || row.bits >= 32 {
                            v
                        } else {
                            v & ((1 << row.bits) - 1)
                        };
                        MonStat {
                            stat: id,
                            param: (next() as u16) & ((1 << row.param_bits) - 1),
                            value: v,
                            send: Some((next() % 4 != 0, row.bits, row.param_bits)),
                        }
                    })
                    .collect(),
            ),
        };
        let a = AssignMonster {
            guid: [1, 0x7F, 0x80, 0xFFFF, 0x8000_0000, u32::MAX - 1][case as usize % 6],
            class: 0,
            x: U16S[case as usize % 5],
            y: U16S[(case as usize + 2) % 5],
            life: U8S[case as usize % 5],
            mode: [0, 1, 3, 8, 9, 12, 15][case as usize % 7],
            components,
            counts,
            type_block,
            source,
            stats,
        };
        let sim = assign_monster(&a);
        // Header by the TSV.
        assert_eq!(tsv(&sim, "guid"), a.guid);
        assert_eq!(tsv(&sim, "class"), u32::from(a.class));
        assert_eq!(tsv(&sim, "x"), u32::from(a.x));
        assert_eq!(tsv(&sim, "y"), u32::from(a.y));
        assert_eq!(tsv(&sim, "life"), u32::from(a.life));
        assert_eq!(tsv(&sim, "size"), sim.len() as u32);
        assert_eq!(tsv_tail(&sim, "data").len(), sim.len() - 13);
        one_size(&sim);
        // The client.
        let mut m = Model::default();
        m.inputs.tables.monsters = vec![Some(MonsterClass {
            components: counts,
            ..MonsterClass::default()
        })];
        m.inputs.tables.stats = stats_t.clone();
        m.recv(&sim);
        assert!(m.log.rejected.is_empty(), "case {case}: {:?}", m.rejected());
        let key = UnitKey::new(MONSTER, a.guid);
        let u = m.unit(key);
        let mode = match a.mode {
            0 | 8 | 9 | 12 => a.mode,
            _ => 1,
        };
        assert_eq!((u.class, u.mode), (0, mode), "case {case}");
        assert_eq!(u.stat(6), i32::from(a.life) << 8);
        assert_eq!(u.stat(7), 0x8000);
        assert_eq!(u.stat(328), i32::from(a.x.wrapping_add(a.y)));
        let KindData::Monster(d) = &u.kind else {
            panic!("monster data");
        };
        assert_eq!(d.components, components, "case {case}");
        match &a.type_block {
            None => {
                assert_eq!((d.flags, d.hc_idx, d.name_seed, d.value), (0, 0, 0, -1));
                assert_eq!(d.umods, [0; 9]);
            }
            Some(t) => {
                assert_eq!(u16::from(d.flags), t.flags, "case {case}");
                let hc = if t.flags & type_flag::SUPERUNIQUE != 0 {
                    t.hc_idx
                } else {
                    0
                };
                assert_eq!(d.hc_idx, hc);
                let mut um = [0u8; 9];
                um[..t.umods.len()].copy_from_slice(&t.umods);
                assert_eq!(d.umods, um);
                assert_eq!(d.name_seed, t.name_seed & 0xFF);
                assert_eq!(d.value, t.hireling_owner.map_or(-1, |g| g as i32));
            }
        }
        assert_eq!(d.v31, a.source.map(|v| v & 0x0FFF_FFFF), "case {case}");
        let mut want = BTreeMap::new();
        for e in a.stats.iter().flatten().take(16) {
            if let Some((true, bits, _)) = e.send {
                if bits > 0 {
                    want.insert((e.stat, e.param), e.value);
                }
            }
        }
        let got = d.stat_list.clone().unwrap_or_default();
        assert_eq!(got, want, "case {case}");
    }
}

// ---- 0x94 BaseSkillLevels

#[test]
fn base_skill_levels_0x94_one_layout() {
    assert_eq!(
        base_skill_levels(1, &[]),
        None,
        "n = 0 is below the TSV minimum"
    );
    assert_eq!(
        base_skill_levels(1, &vec![(0, 1); 256]),
        None,
        "n is a byte"
    );
    let mut rng = sweep(1 << 12);
    for n in [1usize, 2, 7, 64, 168] {
        let entries: Vec<(u16, u8)> = (0..n)
            .map(|i| ((i * 3) as u16 % 221, (rng.next().unwrap() % 20) as u8 + 1))
            .collect();
        for guid in [1u32, 0x80, 0xFFFF] {
            let sim = base_skill_levels(guid, &entries).unwrap();
            assert_eq!(tsv(&sim, "count"), n as u32);
            assert_eq!(tsv(&sim, "guid"), guid);
            let data = tsv_tail(&sim, "data");
            assert_eq!(data.len(), 3 * n);
            for (i, &(s, l)) in entries.iter().enumerate() {
                assert_eq!(u16::from_le_bytes([data[3 * i], data[3 * i + 1]]), s);
                assert_eq!(data[3 * i + 2], l);
            }
            one_size(&sim);
            if guid != 1 {
                continue;
            }
            // The client: player 1 (0x59) with 221 skills rows.
            let mut m = Model::default();
            m.inputs.tables.skills = vec![SkillRow::default(); 221];
            let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72");
            b.resize(0x1A, 0);
            m.recv(&b);
            m.recv(&sim);
            assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
            let l = m.unit(UnitKey::new(PLAYER, 1)).skills.as_ref().unwrap();
            let mut want: BTreeMap<u16, i32> = BTreeMap::new();
            for &(s, lv) in &entries {
                want.insert(s, i32::from(lv));
            }
            for (s, lv) in want {
                let e = &l.entries[l.native(s).expect("assigned")];
                assert_eq!(e.base, lv, "skill {s}");
            }
        }
    }
}

// ---- 0x9C / 0x9D item actions

/// Itemstatcost save columns the item streams use: (stat, save bits,
/// save add, save param bits).
const ISC: [(u16, u8, u32, u32); 3] = [(9, 8, 32, 0), (107, 3, 0, 9), (194, 4, 0, 0)];

fn isc_table() -> Vec<Isc> {
    let mut t = vec![Isc::default(); 360];
    for (s, bits, add, param) in ISC {
        t[usize::from(s)] = Isc {
            valshift: 0,
            save_bits: bits,
            save_add: add,
            save_param_bits: param,
        };
    }
    t
}

struct Lookup;

impl ItemLookup for Lookup {
    fn code(&self, code: [u8; 4]) -> Option<CodeFacts> {
        (code == *b"hax " || code == *b"gld ").then(|| CodeFacts {
            gold: code == *b"gld ",
            ..CodeFacts::default()
        })
    }
    fn isc(&self, stat: u16) -> Option<IscSave> {
        ISC.iter()
            .find(|r| r.0 == stat)
            .map(|&(_, save_bits, save_add, save_param_bits)| IscSave {
                save_bits,
                save_add,
                save_param_bits,
            })
    }
}

/// One stream item at (mode, body, x, y, page): a full identified `hax `
/// with one property, or a compact gold pile.
fn stream_item(mode: u32, body: u8, x: i32, y: i32, page: u8, gold: Option<i32>) -> StreamItem {
    let mut it = StreamItem {
        flags: 0x10,
        version: 101,
        mode,
        x,
        y,
        body_loc: body,
        page,
        code: *b"hax ",
        ilvl: 5,
        quality: 2,
        main: Some(vec![StatEntry {
            stat: 9,
            param: 0,
            value: 20,
        }]),
        ..StreamItem::default()
    };
    if let Some(g) = gold {
        it.compact = true;
        it.code = *b"gld ";
        it.kind.gold = true;
        it.total_gold = g;
        it.main = None;
    }
    it
}

/// A placement written: (mode, body, x, y, page).
type Place = (u32, u8, i32, i32, u8);
/// The head read back: (mode, body, page, x, y).
type Head = (u8, u8, u8, u16, u16);

/// Each placement: (mode, body, x, y, page) and the head the client
/// reads (mode, body, page, x, y): grid / slot values are capped at 15
/// and the page is sent + 1.
fn placements() -> Vec<(Place, Head)> {
    let mut v = Vec::new();
    for (x, y) in [(0, 0), (1, 2), (9, 3), (15, 15), (20, -1)] {
        let c = |n: i32| (n as u32).min(15) as u16;
        for page in [0u8, 1, 4, 0xFF] {
            let p = page.wrapping_add(1).min(7);
            let rp = if p == 0 { 0xFF } else { p - 1 };
            v.push(((0, 0, x, y, page), (0, 0, rp, c(x), c(y))));
        }
        v.push(((2, 0, x, y, 0xFF), (2, 0, 0xFF, c(x), c(y))));
    }
    for body in [1u8, 4, 5, 11, 12] {
        v.push(((1, body, 0, 0, 0xFF), (1, body, 0xFF, 0, 0)));
    }
    for g in U16S {
        let (x, y) = (i32::from(g), i32::from(g ^ 0x5A5A));
        v.push(((3, 0, x, y, 0), (3, 0, 0xFF, g, g ^ 0x5A5A)));
        v.push(((5, 0, x, y, 0), (5, 0, 0xFF, g, g ^ 0x5A5A)));
    }
    v.push(((3, 0, -1, 0x1_0000, 0), (3, 0, 0xFF, 0xFFFF, 0xFFFF)));
    v
}

#[test]
fn item_action_0x9c_0x9d_one_layout() {
    let t = isc_table();
    let p1 = UnitKey::new(PLAYER, 1);
    let mut guids = U32S.iter().copied().filter(|&g| g != 1).chain(sweep(3));
    for (n, (place, head)) in placements().into_iter().enumerate() {
        let gold = (n % 3 == 0).then_some([0, 1, 0xFFF, 0x1000, i32::MAX][n % 5]);
        let (mode, body, x, y, page) = place;
        let it = stream_item(mode, body, x, y, page, gold);
        let (bits, _) = bitstream::write(&it, &t).unwrap();
        let guid = guids.next().unwrap_or(0x100 + n as u32);
        let category = U8S[n % 5];
        for owned in [false, true] {
            // 0x9C action 0 (a 0x9C handler without a cursor write);
            // 0x9D action 0x14 (a 0x9D handler without one).
            let (sim, action) = if owned {
                (item_owned(0x14, category, guid, 0, 1, &bits).unwrap(), 0x14)
            } else {
                (item_world(0x00, category, guid, &bits).unwrap(), 0x00)
            };
            assert_eq!(tsv(&sim, "action"), action);
            assert_eq!(tsv(&sim, "size"), sim.len() as u32);
            assert_eq!(tsv(&sim, "category"), u32::from(category));
            assert_eq!(tsv(&sim, "item"), guid);
            if owned {
                assert_eq!((tsv(&sim, "owner_type"), tsv(&sim, "owner")), (0, 1));
            }
            let stream = tsv_tail(&sim, "data");
            assert_eq!(stream, bits.as_slice());
            one_size(&sim);
            // d2-proto's reader of the same stream.
            let d = item_bits::decode(stream, &Lookup).unwrap();
            assert_eq!(d.flags, header_flags(&it));
            assert_eq!((d.version, d.mode), (101, head.0));
            let loc = match d.location.unwrap() {
                Location::Ground { x, y } => (0, 0xFF, x, y),
                Location::Slot { body, x, y, page1 } => (
                    body,
                    if page1 == 0 { 0xFF } else { page1 - 1 },
                    u16::from(x),
                    u16::from(y),
                ),
            };
            assert_eq!(loc, (head.1, head.2, head.3, head.4), "{place:?}");
            assert_eq!(d.code, it.code);
            if let Some(g) = gold {
                assert_eq!(d.gold, Some(g as u32));
            } else {
                let l = d.lists[0].as_ref().unwrap();
                assert_eq!((l[0].stat, l[0].value()), (9, 20));
            }
            // The client handler and its item view.
            let mut m = Model::default();
            m.put(p1).kind = KindData::Player(PlayerData::default());
            m.w.local_player = Some(p1);
            m.recv(&sim);
            assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
            let key = UnitKey::new(ITEM, guid);
            let KindData::Item(data) = &m.unit(key).kind else {
                panic!("item data");
            };
            let r = data.last.as_ref().unwrap();
            assert_eq!(
                (r.id, r.action, r.category, r.owner),
                (sim[0], action as u8, category, owned.then_some(p1))
            );
            assert_eq!(r.stream, bits);
            let v = super::super::items::item(&m.w, key).unwrap();
            assert_eq!(
                (v.flags, v.mode, v.body, v.page, v.x, v.y, v.code),
                (
                    header_flags(&it),
                    head.0,
                    head.1,
                    head.2,
                    head.3,
                    head.4,
                    Some(it.code)
                ),
                "{place:?}"
            );
            assert_eq!(v.gold, gold.map(|g| g as u32));
            assert_eq!(v.owner, Some(p1));
            let pos = matches!(head.0, 3 | 5).then_some((head.3, head.4));
            assert_eq!(m.unit(key).position, pos);
        }
    }
}

// ---- 0x3E UpdateItemStats (no d2-sim builder: the client layout)

#[test]
fn update_item_stats_0x3e_one_layout() {
    // The width choice: 1 bit a; 0 → 8; else 1 bit b: 16 / 32.
    let sized = |v: u32, w: u32| -> Vec<(u32, u32)> {
        match w {
            8 => vec![(0, 1), (v, 8)],
            16 => vec![(1, 1), (0, 1), (v, 16)],
            _ => vec![(1, 1), (1, 1), (v, 32)],
        }
    };
    for (gw, guid) in [
        (8, 0x7Fu32),
        (8, 0xFF),
        (16, 0x8000),
        (16, 0xFFFF),
        (32, u32::MAX),
        (32, 1),
    ] {
        for (vw, value) in [
            (8, 0u32),
            (8, 0x80),
            (16, 0xFFFF),
            (32, 0x8000_0000),
            (32, u32::MAX),
        ] {
            for (set, stat) in [(true, 72u16), (false, 73), (true, 0x1FF), (true, 70)] {
                let mut f = sized(guid, gw);
                f.push((u32::from(set), 1));
                f.push((u32::from(stat), 9));
                f.extend(sized(value, vw));
                f.extend([(1, 1), (0xABCD, 16)]);
                let stream = pack(&f);
                let mut b = vec![0x3E, (2 + stream.len()) as u8];
                b.extend(stream);
                assert_eq!(tsv(&b, "size"), b.len() as u32);
                one_size(&b);
                let key = UnitKey::new(ITEM, guid);
                let mut m = Model::default();
                m.put(key).kind = KindData::Item(Default::default());
                m.recv(&b);
                assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
                let want = set.then_some(value as i32);
                assert_eq!(m.unit(key).stats.get(&stat).copied(), want);
            }
        }
    }
}

/// The d2-sim builder (`units::messages::update_item_stat`, widths:
/// the narrowest that holds each field) equals the client
/// layout packed by hand, and the client sets every stat back (param
/// unused off stat 204).
#[test]
fn update_item_stats_0x3e_sim_builder_to_client() {
    let width = |v: u32| {
        if v <= 0xFF {
            8
        } else if v <= 0xFFFF {
            16
        } else {
            32
        }
    };
    let sized = |v: u32| -> Vec<(u32, u32)> {
        match width(v) {
            8 => vec![(0, 1), (v, 8)],
            16 => vec![(1, 1), (0, 1), (v, 16)],
            _ => vec![(1, 1), (1, 1), (v, 32)],
        }
    };
    let guids = [0u32, 0xFF, 0x100, 0xFFFF, 0x1_0000, u32::MAX];
    let values = [0i32, 1, 0xFF, 0x100, 0xFFFF, 0x1_0000, i32::MAX, -1];
    for (i, guid) in guids.iter().copied().chain(sweep(6)).enumerate() {
        for (j, &value) in values.iter().enumerate() {
            let stat = [70u16, 72, 73, 0x1FF, 0][(i + j) % 5];
            let param = [0u16, 0xFF, 0x100, 0xFFFF][(i + j) % 4];
            let b = d2_sim::units::messages::update_item_stat(guid, stat, value, param);
            let mut f = sized(guid);
            f.push((1, 1));
            f.push((u32::from(stat), 9));
            f.extend(sized(value as u32));
            f.extend(if param <= 0xFF {
                [(0, 1), (u32::from(param), 8)]
            } else {
                [(1, 1), (u32::from(param), 16)]
            });
            let stream = pack(&f);
            let mut want = vec![0x3E, (2 + stream.len()) as u8];
            want.extend(stream);
            // The base game sends the 0x3E in a zero-padded 0x22-byte buffer.
            if want.len() < 0x22 {
                want.resize(0x22, 0);
            }
            assert_eq!(b, want, "guid {guid:#x} value {value:#x}");
            // The wire message is the length byte's size; the rest is the
            // sender's buffer padding.
            let b = &b[..usize::from(b[1])];
            assert_eq!(tsv(b, "size"), b.len() as u32);
            one_size(b);
            let key = UnitKey::new(ITEM, guid);
            let mut m = Model::default();
            m.put(key).kind = KindData::Item(Default::default());
            m.recv(b);
            assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
            assert_eq!(m.unit(key).stats.get(&stat).copied(), Some(value));
        }
    }
}

// ---- fixed ids without a d2-sim builder: d2-proto encode → client

#[test]
fn skill_npc_baal_0xa3_0xa4_0xa5_0xab_one_layout() {
    for (i, guid) in U32S.iter().copied().chain(sweep(4)).enumerate() {
        // 0xA3: every field read back in the SkillDo output.
        let g = gen::UnknownA3 {
            v: U8S[i % 5],
            skill: (i % 3) as u16,
            level: U16S[i % 5],
            type_: MONSTER,
            guid,
            target_type: PLAYER,
            target: guid ^ 1,
            x: U32S[i % 7],
            y: U32S[(i + 3) % 7],
        };
        let b = g.encode();
        // The d2-sim record's sender (`wiring::action::event_records`).
        let sim = d2_sim::wiring::action::event_records::progressive(
            g.v,
            g.skill,
            g.level,
            (g.type_, g.guid),
            (g.target_type, g.target),
            g.x,
            g.y,
        );
        assert_eq!(sim.to_vec(), b.to_vec());
        assert_eq!(parse(&b).unwrap(), S2c::UnknownA3(g));
        one_size(&b);
        let mut m = Model::default();
        m.inputs.tables.skills = vec![SkillRow::default(); 3];
        let unit = UnitKey::new(MONSTER, guid);
        let target = UnitKey::new(PLAYER, guid ^ 1);
        m.put(unit);
        m.put(target);
        m.recv(&b);
        assert_eq!(
            m.out,
            [Output::SkillDo {
                unit,
                target: Some(target),
                skill: g.skill,
                level: g.level as i16,
                x: g.x,
                y: g.y,
                v: g.v,
            }]
        );
        // 0xA5: state 18 off; srvdofunc 67 → SkillEndFx.
        let g = gen::UnknownA5 {
            type_: MONSTER,
            guid,
            skill: 1,
        };
        let b = g.encode();
        assert_eq!(
            d2_sim::wiring::action::event_records::landing(MONSTER, guid, 1).to_vec(),
            b.to_vec()
        );
        assert_eq!(parse(&b).unwrap(), S2c::UnknownA5(g));
        one_size(&b);
        m.out.clear();
        m.inputs.tables.skills[1].srvdofunc = 67;
        m.w.units.get_mut(&unit).unwrap().states.insert(18);
        m.recv(&b);
        assert!(!m.unit(unit).states.contains(&18));
        assert_eq!(
            m.out,
            [Output::SkillEndFx {
                unit,
                skill: 1,
                srvdofunc: 67
            }]
        );
        // 0xAB: monster life byte → stat 6.
        let life = U8S[i % 5];
        let g = gen::NpcHeal {
            type_: MONSTER,
            guid,
            life,
        };
        let b = g.encode();
        assert_eq!(
            d2_sim::wiring::action::event_records::npc_heal(MONSTER, guid, life).to_vec(),
            b.to_vec()
        );
        assert_eq!(parse(&b).unwrap(), S2c::NpcHeal(g));
        one_size(&b);
        m.recv(&b);
        assert_eq!(m.unit(unit).stat(6), i32::from(life) << 8);
        // 0xA4: the class u16@1.
        let class = U16S[i % 5];
        let g = gen::BaalWave { class };
        let b = g.encode();
        assert_eq!(
            d2_sim::wiring::action::event_records::preload(class).to_vec(),
            b.to_vec()
        );
        assert_eq!(parse(&b).unwrap(), S2c::BaalWave(g));
        one_size(&b);
        m.out.clear();
        m.inputs.tables.monsters = vec![None; 0x8001];
        m.recv(&b);
        let want: Vec<Output> = (usize::from(class) < 0x8001)
            .then_some(Output::MonsterPreload { class })
            .into_iter()
            .collect();
        assert_eq!(m.out, want);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    }
}

#[test]
fn item_table_entry_0xa6_one_layout() {
    for index in [0u16, 1, 0x7F, 0x80, 0x1FF] {
        let record: Vec<u8> = sweep(0x120).map(|v| v as u8 ^ index as u8).collect();
        let size = 6 + record.len();
        let mut b = vec![0xA6, 0];
        b.extend((size as u16).to_le_bytes());
        b.extend(index.to_le_bytes());
        b.extend(&record);
        assert_eq!(tsv(&b, "size"), size as u32);
        assert_eq!(tsv(&b, "index"), u32::from(index));
        assert_eq!(tsv_tail(&b, "record"), record.as_slice());
        one_size(&b);
        let mut m = Model::default();
        m.recv(&b);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        assert_eq!(m.w.item_table_ext[usize::from(index)], record);
    }
}

// ---- 0xAE–0xB4 transport / session ids

#[test]
fn session_0xae_0xb4_one_layout() {
    // 0xAE: u16 length + 3, capped at 0x1FD.
    for n in [0usize, 1, 0x7F, 0x80, 0x1FD] {
        let w = WardenRequest {
            data: sweep(n).map(|v| v as u8).collect(),
        };
        let b = w.encode();
        assert_eq!(tsv(&b, "len"), n as u32);
        one_size(&b);
        assert_eq!(parse(&b).unwrap(), S2c::WardenRequest(w));
        let mut m = Model::default();
        m.recv(&b);
        assert!(m.log.rejected.is_empty());
    }
    assert_eq!(
        server_size(&[0xAE, 0xFE, 0x01]),
        Size::Bytes(3),
        "over the cap: 0 + 3"
    );
    // 0xAF: u8 at 1 = 0 → 2, else u8 + 1; the client marks the link up.
    for flag in [0u8, 1, 0x7F, 0x80, 0xFF] {
        let mut b = vec![0xAF, flag];
        b.resize(if flag == 0 { 2 } else { usize::from(flag) + 1 }, 0);
        one_size(&b);
        let mut m = Model::default();
        m.recv(&b);
        assert!(m.w.connected && m.log.rejected.is_empty());
        // 0xB0 (the server's direct one-byte message) clears it.
        let g = gen::ConnectionTerminated;
        assert_eq!(g.encode(), [0xB0]);
        m.recv(&[0xB0]);
        assert!(!m.w.connected);
    }
    // 0xB2: d2-server's builder vs d2-proto; the client ignores it.
    for (players, id) in [(0u16, 0u16), (1, 1), (0x7FFF, 0x8000), (0xFFFF, 0xFFFF)] {
        let mut name = [0u8; 16];
        name[..5].copy_from_slice(b"game1");
        let server = d2_server::adapters::session_flow::game_list_entry(&name, players, id);
        let g = gen::GameList {
            name,
            f49: players,
            f51: id,
        };
        assert_eq!(server, g.encode());
        assert_eq!(parse(&server).unwrap(), S2c::GameList(g));
        one_size(&server);
        let mut m = Model::default();
        m.recv(&server);
        assert!(m.log.rejected.is_empty() && m.out.is_empty());
    }
    // 0xB3: u8 at 1 + 7 (no client effect). The rule needs 8 bytes
    // (`min=8`), so a 7-byte 0xB3 (len 0) splits only when more bytes
    // follow it (findings_d: 0xB3 min).
    assert_eq!(server_size(&[0xB3, 0, 1, 0, 0, 0, 0]), Size::Incomplete);
    assert_eq!(server_size(&[0xB3, 0, 1, 0, 0, 0, 0, 0xB0]), Size::Bytes(7));
    for len in [1u8, 0x7F, 0xFF] {
        let mut b = vec![0xB3, len, 1];
        b.extend(u32::MAX.to_le_bytes());
        b.resize(usize::from(len) + 7, 0);
        assert_eq!(tsv(&b, "len"), u32::from(len));
        one_size(&b);
        let mut m = Model::default();
        m.recv(&b);
        assert!(m.log.rejected.is_empty());
    }
    // 0xB4: code u32@1 → JoinRefused.
    for code in U32S.iter().copied().chain([22, 23, 26, 27]) {
        let g = gen::ConnectionRefused { code };
        let b = g.encode();
        assert_eq!(parse(&b).unwrap(), S2c::ConnectionRefused(g));
        one_size(&b);
        let mut m = Model::default();
        m.recv(&b);
        assert_eq!(
            m.out,
            [Output::JoinRefused {
                error: super::session::join_refused_error(code)
            }]
        );
    }
    // 0xAD and 0xB1 have size 0: never valid, the split discards them.
    for id in [0xADu8, 0xB1] {
        assert_eq!(server_size(&[id, 0, 0]), Size::Invalid);
        let buf = [0xB0, id, 0];
        let s = split_server_buffer(&buf).unwrap();
        assert_eq!((s.messages.len(), s.discarded.len()), (1, 2));
    }
}

#[test]
fn client_missile_0x73_one_layout() {
    for (i, g) in sweep(24).enumerate() {
        let (class, pos, first) = (g as u16, (g, g.rotate_left(7)), (g ^ 5, g.rotate_left(13)));
        let (frame, owner, level, pierce) = (
            U16S[i % 5],
            (U8S[i % 5], g.rotate_left(3)),
            U8S[(i + 1) % 5],
            U8S[(i + 2) % 5],
        );
        let sim =
            d2_sim::units::messages::client_missile(class, pos, first, frame, owner, level, pierce);
        let p = gen::Unknown73 {
            class,
            f7: pos.0,
            f11: pos.1,
            f15: first.0,
            f19: first.1,
            f23: frame,
            source_type: owner.0,
            source: owner.1,
            f30: level,
            f31: pierce,
        };
        let b = p.encode();
        // Bytes 1–4 are not written by the sender and not read.
        assert_eq!(sim.to_vec(), b.to_vec());
        assert_eq!(parse(&b).unwrap(), S2c::Unknown73(p));
        one_size(&b);
        let mut m = Model::default();
        m.put(UnitKey::new(PLAYER, 1));
        m.w.local_player = Some(UnitKey::new(PLAYER, 1));
        m.recv(&b);
        assert_eq!(
            m.out,
            [Output::ClientMissile {
                owner: Some(UnitKey::new(PLAYER, 1)),
                class,
                f07: pos.0,
                f0b: pos.1,
                f0f: first.0,
                f13: first.1,
                f17: frame,
                source: UnitKey::new(owner.0, owner.1),
                f1e: level,
                f1f: pierce,
            }]
        );
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    }
}

#[test]
fn skill_bonus_0x93_one_layout() {
    let guid = 7;
    for (bonus, element, page) in [(1u8, 0u8, 4u8), (0x7F, 3, 0), (0x80, 0, 4), (0xFF, 1, 2)] {
        let sim = d2_sim::units::messages::skill_bonus(guid, bonus, element, page);
        let p = gen::Unknown93 {
            guid,
            bonus,
            element,
            page,
        };
        let b = p.encode();
        assert_eq!(sim.to_vec(), b.to_vec());
        assert_eq!(parse(&b).unwrap(), S2c::Unknown93(p));
        one_size(&b);
        // The client reads the same fields: a player without a skill list
        // is the fatal 0x96D after the bonus 0 test (0x96B).
        let mut m = Model::default();
        m.put(UnitKey::new(PLAYER, guid));
        m.recv(&b);
        assert_eq!(
            m.rejected(),
            [(0x93, "fatal assert 0x96D".to_string())],
            "bonus {bonus}"
        );
    }
    let mut m = Model::default();
    m.put(UnitKey::new(PLAYER, guid));
    m.recv(&d2_sim::units::messages::skill_bonus(guid, 0, 0, 4));
    assert_eq!(m.rejected(), [(0x93, "fatal assert 0x96B".to_string())]);
}

#[test]
fn remove_items_display_0x92_unlinks_every_item_of_the_owner() {
    use super::super::item_lists::ItemProp;
    let t = isc_table();
    let p1 = UnitKey::new(PLAYER, 1);
    let prop = ItemProp {
        stat: 9,
        layer: 0,
        value: 20,
    };
    let mut m = Model::default();
    m.put(p1).kind = KindData::Player(PlayerData::default());
    m.put(UnitKey::new(PLAYER, 2)).kind = KindData::Player(PlayerData::default());
    m.w.local_player = Some(p1);
    // (guid, owner, mode, body, page): P1's two body items, P1's grid
    // item (page 0, not a charm), P1's belt item, another player's body
    // item.
    let rows: [(u32, u32, u32, u8, u8); 5] = [
        (10, 1, 1, 4, 0xFF),
        (11, 1, 1, 5, 0xFF),
        (12, 1, 0, 0, 0),
        (14, 1, 2, 0, 0xFF),
        (13, 2, 1, 4, 0xFF),
    ];
    let record = |m: &mut Model, (guid, owner, mode, body, page): (u32, u32, u32, u8, u8)| {
        let it = stream_item(mode, body, 0, 0, page, None);
        let (bits, _) = bitstream::write(&it, &t).unwrap();
        let b = item_owned(0x14, 0, guid, 0, owner, &bits).unwrap();
        m.recv(&b);
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let KindData::Item(d) = &mut m.w.units.get_mut(&UnitKey::new(ITEM, guid)).unwrap().kind
        else {
            panic!("item data");
        };
        d.props = vec![prop];
    };
    for r in rows {
        record(&mut m, r);
    }
    let total = |m: &Model, p: u32| m.w.item_lists_total(UnitKey::new(PLAYER, p), 9, 0);
    // Body items count; the grid item is not attached.
    assert_eq!((total(&m, 1), total(&m, 2)), (40, 20));
    // The bytes: d2-proto encode against the client handler.
    let p = gen::RemoveItemsDisplay {
        type_: PLAYER,
        guid: 1,
    };
    let b = p.encode();
    assert_eq!(parse(&b).unwrap(), S2c::RemoveItemsDisplay(p));
    one_size(&b);
    m.recv(&b);
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    // Every item of P1's inventory is unlinked (empties it:
    // body, grid and belt); P2's are not; no unit leaves S.
    assert_eq!((total(&m, 1), total(&m, 2)), (0, 20));
    assert_eq!(m.w.units.len(), 7);
    let unlinked = |m: &Model, g: u32| match &m.w.units[&UnitKey::new(ITEM, g)].kind {
        KindData::Item(d) => d.unlinked,
        _ => panic!("item data"),
    };
    assert_eq!(
        [10, 11, 12, 14, 13].map(|g| unlinked(&m, g)),
        [true, true, true, true, false]
    );
    // The next record about an item re-adds only that one.
    record(&mut m, rows[0]);
    assert_eq!(total(&m, 1), 20);
    assert_eq!(
        [10, 11, 12, 14].map(|g| unlinked(&m, g)),
        [false, true, true, true]
    );
    // An absent unit and a non-inventory type change nothing; a wrong
    // size is invalid.
    m.recv(&[0x92, PLAYER, 9, 0, 0, 0]);
    m.recv(&[0x92, 2, 1, 0, 0, 0]);
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
    assert_eq!(total(&m, 1), 20);
}

#[test]
fn attached_item_base_array_counts_in_the_owner_total() {
    use super::super::item_lists::{ItemFields, ItemProp};
    let t = isc_table();
    let p1 = UnitKey::new(PLAYER, 1);
    let mut m = Model::default();
    m.put(p1).kind = KindData::Player(PlayerData::default());
    m.w.local_player = Some(p1);
    // A body item and a grid item, each with base array {31: 20, 68: -5}
    // and list {31: 7}.
    for (guid, mode, body, page) in [(10u32, 1u32, 4u8, 0xFFu8), (12, 0, 0, 0)] {
        let it = stream_item(mode, body, 0, 0, page, None);
        let (bits, _) = bitstream::write(&it, &t).unwrap();
        m.recv(&item_owned(0x14, 0, guid, 0, 1, &bits).unwrap());
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let KindData::Item(d) = &mut m.w.units.get_mut(&UnitKey::new(ITEM, guid)).unwrap().kind
        else {
            panic!("item data");
        };
        let prop = |stat, value| ItemProp {
            stat,
            layer: 0,
            value,
        };
        d.props = vec![prop(31, 7)];
        d.fields = Some(ItemFields {
            base: vec![prop(31, 20), prop(68, -5)],
            ..ItemFields::default()
        });
    }
    // Only the attached body item counts: list and base array.
    assert_eq!(m.w.item_lists_total(p1, 31, 0), 27);
    assert_eq!(m.w.item_lists_total(p1, 68, 0), -5);
}
