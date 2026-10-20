//! Property test of the item-move intent handlers (`handle`) on
//! arbitrary message payloads against the fake world of `tests` (every
//! seam answered from random state and random knobs).
//!
//! Properties, for every message:
//! - no panic (the fake also panics when a handler reads a field of an
//!   item that does not exist, a read the base game makes through a null
//!   unit);
//! - `None` exactly for an id outside `HANDLED` (or an empty message);
//!   otherwise a handler result code 0–3, or a named fatal assert;
//! - a wrong size → 3 with no state change (text);
//! - every rejection ordered before the handler's first effect
//!   returns its code and leaves the state unchanged (seam calls
//!   that only log, such as a sound, are not state; the "can't do that"
//!   0x5A of 0x19 / 0x63 is the one send a rejection makes). The
//!   expected rejections are written from the handler rules, step by step, below
//!   (`early`).

// The oracle keeps each step as its own branch, even where two steps
// give the same code, so it reads against the handler rules line by line.
#![allow(clippy::if_same_then_else)]

use proptest::prelude::*;

use super::seams::InventoryOps;
use super::tests::{me, FInv, FItem, FUnit, Fake, Knobs, P};
use super::{handle, mode, res, Guid, Owner, Spot, HANDLED};
use crate::units::RoomId;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Item GUIDs of the random world; 99 names nothing.
const FIRST: Guid = 10;
const N_ITEMS: u32 = 8;
const MISSING: Guid = 99;
const MERC: Guid = 50;

const TYPES: [u16; 16] = [2, 3, 4, 18, 19, 22, 27, 28, 30, 33, 34, 37, 71, 76, 80, 81];
const CODES: [[u8; 4]; 5] = [*b"box ", *b"gld ", *b"isc ", *b"xxx ", *b"hp1 "];

/// Random setup bytes, read past the end as 0.
struct Dna<'a> {
    b: &'a [u8],
    i: usize,
}

impl Dna<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.i).copied().unwrap_or(0);
        self.i += 1;
        v
    }
    fn below(&mut self, n: u8) -> u8 {
        self.u8() % n
    }
    fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[usize::from(self.u8()) % v.len()]
    }
}

fn world(dna: &[u8]) -> Fake {
    let mut d = Dna { b: dna, i: 0 };
    let mut f = Fake::new();
    f.expansion = d.below(4) != 3;
    let cursor = d.below(2 * N_ITEMS as u8);
    let mut body_used = [false; 13];
    for n in 0..N_ITEMS {
        let g = FIRST + n;
        let m = if u32::from(cursor) == n {
            mode::CURSOR
        } else {
            d.pick(&[mode::STORED, mode::EQUIPPED, mode::BELT, mode::GROUND, 6, 5])
        };
        // A few items belong to someone else (not in the player's lists).
        let foreign = m != mode::GROUND && m != mode::CURSOR && d.below(6) == 5;
        let it = f.item(g, m);
        it.page = match m {
            mode::STORED => d.pick(&[0, 0, 0, 1, 2, 3, 4]),
            _ => d.pick(&[0xFF, 0xFF, 0, 1, 2]),
        };
        it.stored_page = d.below(5);
        it.iflags = u32::from(d.u8()) << d.pick(&[0, 4, 8, 16]);
        let nt = d.below(3);
        it.types = (0..nt).map(|_| d.pick(&TYPES)).collect();
        it.code = d.pick(&CODES);
        it.quality = d.below(8);
        it.file_index = i32::from(d.below(3)) - 1;
        it.quest = u8::from(d.below(5) == 4);
        it.useable = d.bool();
        it.component = d.below(3);
        it.max_stack = d.pick(&[0, 1, 2, 4, 100, i32::MAX, -1]);
        it.filled = d.bool();
        it.filler = d.bool();
        it.sockets = i32::from(d.below(4));
        it.spell = i32::from(d.below(2));
        it.two_handed = d.bool();
        it.beltable = d.bool();
        it.carry_one = d.bool();
        let loc = d.below(13);
        let slot = d.below(16);
        if foreign {
            it.owner = Some(Owner::monster(MERC));
            let inv = f.inv_mut();
            inv.list.retain(|&x| x != g);
        } else if m == mode::EQUIPPED && !body_used[usize::from(loc)] {
            body_used[usize::from(loc)] = true;
            f.items.get_mut(&g).unwrap().body_loc = loc;
            f.inv_mut().body.insert(loc, g);
        }
        let u = f.unit(Owner::item(g));
        u.x = 100 + i32::from(d.below(80)) - 40;
        u.y = 100 + i32::from(d.below(80)) - 40;
        if m == mode::BELT {
            u.x = i32::from(slot);
            u.y = 0;
        }
        // Stats include the extremes a stat list can hold.
        u.stats.insert(70, d.pick(&[0, 1, 2, 3, 5, i32::MAX, -1]));
        u.stats
            .insert(14, d.pick(&[0, 7, 150, 2_000_000_000, i32::MAX, -5]));
        u.stats.insert(72, d.pick(&[0, 10, 49, i32::MAX, -1]));
        u.uflags = u32::from(d.u8());
    }
    if f.inv().cursor.is_some() {
        // The cursor item is never also in the item list.
        let c = f.inv().cursor.unwrap();
        f.inv_mut().list.retain(|&x| x != c);
    }
    f.inv_mut().weapon = d
        .bool()
        .then_some(FIRST + u32::from(d.below(N_ITEMS as u8)));
    let pl = f.unit(me());
    pl.stats
        .insert(14, d.pick(&[0, 50, 5000, 10_000_000, i32::MAX]));
    pl.stats.insert(12, d.pick(&[1, 1, 30, 99, 0]));
    // Positions are subtiles inside a level (never near the i32 limits).
    let (px, py) = d.pick(&[(100, 100), (100, 100), (30_000, 2), (0, 0)]);
    pl.x = px;
    pl.y = py;
    let merc = Owner::monster(MERC);
    let hire = d.below(3) != 2;
    if hire {
        let mut mi = FInv::default();
        for loc in 1..=10u8 {
            if d.below(3) == 2 {
                let g = 60 + u32::from(loc);
                mi.list.push(g);
                mi.body.insert(loc, g);
                f.items.insert(
                    g,
                    FItem {
                        mode: d.pick(&[mode::EQUIPPED, mode::EQUIPPED, mode::STORED]),
                        page: 0xFF,
                        body_loc: loc,
                        owner: Some(merc),
                        types: vec![d.pick(&TYPES)],
                        code: *b"xxx ",
                        file_index: -1,
                        max_stack: 1,
                        ..Default::default()
                    },
                );
                f.units.insert(Owner::item(g), FUnit::default());
            }
        }
        f.invs.insert(merc, mi);
        f.units.insert(
            merc,
            FUnit {
                class: d.pick(&[0x10F, 0x152, 0x167, 0x230, 0x231, 7]),
                ..Default::default()
            },
        );
    }
    let opt_guid = |d: &mut Dna| {
        let v = d.below(N_ITEMS as u8 + 2);
        (u32::from(v) < N_ITEMS).then_some(FIRST + u32::from(v))
    };
    f.k = Knobs {
        busy: d.bool(),
        trading: d.below(4) == 3,
        gate: d.below(4) != 3,
        in_town: d.bool(),
        place_ok: d.below(4) != 3,
        link_ok: d.below(6) != 5,
        fail_body_place: false,
        fail_link_kind: None,
        fail_unlink: false,
        free: d.bool().then_some((0, 0)),
        belt_slot: d.bool().then(|| d.below(16)),
        belt_gate: d.bool(),
        equip_check: d.below(8),
        requirements: d.bool(),
        stack: d.bool(),
        auto_equip: d.bool().then(|| d.below(13)),
        equip_from_cursor: (d.bool(), d.bool()),
        to_remove: opt_guid(&mut d),
        distance: i32::from(d.pick(&[0u8, 3, 4, 5, 50, 51])),
        collides: d.bool(),
        room_at: d.bool(),
        spot: d.bool().then_some(Spot {
            room: RoomId(7),
            x: 10,
            y: 20,
        }),
        in_room: d.bool(),
        quest_flags: Default::default(),
        pile_owner: d.bool().then_some(me()),
        merge: d.bool(),
        consume: d.bool(),
        use_ok: d.bool(),
        equip_picked: d.bool(),
        hireling: hire.then_some(merc),
        alive: d.below(4) != 3,
        used_skill: d.below(4) == 3,
        owns: d.below(4) != 3,
        q44: d.bool(),
        bits: if d.bool() { Vec::new() } else { vec![1, 2, 3] },
        boxes: 16,
        item_skill: -1,
        has_skill: false,
    };
    f
}

/// The message: an id, then u32 words (each an item GUID, the player, a
/// missing GUID, a small number or any value), cut to `len`.
#[derive(Clone, Debug)]
struct Msg {
    id: u8,
    len: usize,
    words: Vec<u32>,
    /// Put the role the message layout names into each item field.
    hinted: bool,
}

/// Item roles of the u32 fields of each id (layouts; `resolve`):
/// 0 cursor, 1 stored, 2 belt, 3 equipped, 4 ground, 5 owned, 6 the
/// player; `N` = not an item.
const N: u8 = 0xFF;
fn hints(id: u8) -> [u8; 4] {
    match id {
        0x16 => [N, 4, N, N],
        0x17 | 0x18 | 0x1A | 0x1B | 0x1D | 0x1E | 0x23 => [0, N, N, N],
        0x19 | 0x20 | 0x63 => [1, N, N, N],
        0x1F => [0, 1, N, N],
        0x21 | 0x27 => [5, 5, N, N],
        0x22 => [5, N, N, N],
        0x24 | 0x26 => [2, N, N, N],
        0x25 => [0, 2, N, N],
        0x28 => [0, 3, N, N],
        0x29 => [4, 1, N, N],
        0x50 => [6, N, N, N],
        _ => [N; 4],
    }
}

impl Msg {
    fn bytes(&self, f: &Fake) -> Vec<u8> {
        let mut b = vec![self.id];
        let h = hints(self.id);
        for (i, &w) in self.words.iter().enumerate() {
            let w = match h[i] {
                role if self.hinted && role != N => SEL | (w & 0xFF00) | u32::from(role),
                _ => w,
            };
            b.extend_from_slice(&resolve(f, w).to_le_bytes());
        }
        b.resize(self.len, 0);
        b
    }
}

/// Words at or above `SEL` name an item by role, resolved against the
/// state the message meets (`resolve`): low byte the role, next byte an
/// index among the matching items.
const SEL: u32 = 0xF000_0000;

fn word() -> impl Strategy<Value = u32> {
    prop_oneof![
        6 => (0u32..6, 0u32..8).prop_map(|(role, k)| SEL | (k << 8) | role),
        2 => FIRST..FIRST + N_ITEMS,
        1 => 60u32..140,
        1 => Just(P),
        1 => Just(MISSING),
        3 => 0u32..14,
        1 => 0u32..200,
        1 => any::<u32>(),
    ]
}

fn msg() -> impl Strategy<Value = Msg> {
    let ids: Vec<(u8, usize)> = HANDLED.to_vec();
    (
        prop_oneof![
            9 => proptest::sample::select(ids),
            1 => (any::<u8>(), 0usize..20).prop_map(|(i, s)| (i, s)),
        ],
        prop_oneof![8 => Just(0i32), 1 => -2i32..3],
        proptest::collection::vec(word(), 4),
        (0u8..4).prop_map(|h| h != 0),
    )
        .prop_map(|((id, size), delta, words, hinted)| Msg {
            id,
            len: (size as i32 + delta).max(0) as usize,
            words,
            hinted,
        })
}

/// Resolves a role word: 0 the cursor item, 1 a stored item of the
/// player, 2 a belt item, 3 an equipped item, 4 a ground item, 5 any item
/// of the player's list, 6 the player (none → the missing GUID).
fn resolve(f: &Fake, w: u32) -> u32 {
    if w < SEL {
        return w;
    }
    let (role, k) = ((w & 0xFF) as u8, ((w >> 8) & 0xFF) as usize);
    match role {
        0 => return f.inv().cursor.unwrap_or(MISSING),
        6 => return P,
        _ => {}
    }
    let list = &f.inv().list;
    let pick: Vec<Guid> = f
        .items
        .iter()
        .filter(|&(g, it)| match role {
            1 => it.mode == mode::STORED && list.contains(g),
            2 => it.mode == mode::BELT && list.contains(g),
            3 => it.mode == mode::EQUIPPED && list.contains(g),
            4 => it.mode == mode::GROUND,
            _ => list.contains(g),
        })
        .map(|(&g, _)| g)
        .collect();
    if pick.is_empty() {
        MISSING
    } else {
        pick[k % pick.len()]
    }
}

/// Everything a handler may change (the call log aside).
fn state(f: &Fake) -> String {
    format!("{:?}", (&f.items, &f.units, &f.invs, &f.sent, f.next_guid))
}

fn valid_loc(v: u32) -> bool {
    (1..=10).contains(&v)
}

/// The rejections orders before any effect: `Some(code)` when the
/// message must be refused with that code and no state change.
fn early(f: &Fake, m: &Msg, b: &[u8]) -> Option<u32> {
    let p = me();
    let w = |i: usize| u32::from_le_bytes([b[1 + 4 * i], b[2 + 4 * i], b[3 + 4 * i], b[4 + 4 * i]]);
    let exists = |g: Guid| f.items.contains_key(&g);
    let cursor_check = |g| f.check_cursor_item(p, g);
    let first = |checks: &[u32]| checks.iter().copied().find(|&r| r != 0);
    let busy = f.k.busy || f.inv().cursor.is_some();
    let body = |loc: u32| {
        u8::try_from(loc)
            .ok()
            .and_then(|l| f.inv().body.get(&l).copied())
    };
    match m.id {
        // R1,.
        0x16 => {
            let (t, g) = (w(0), w(1));
            if t > 5 {
                Some(res::BAD)
            } else if t == 0 && g == P {
                Some(res::REFUSED)
            } else if t == 4 && (!exists(g) || f.it(g).mode != mode::GROUND || f.k.distance > 50) {
                Some(res::RANGE)
            } else {
                None
            }
        }
        0x17 => {
            first(&[cursor_check(w(0))]).or_else(|| (busy && f.k.trading).then_some(res::REFUSED))
        }
        // (the player exists and is a player).
        0x18 => first(&[cursor_check(w(0))]).or_else(|| match w(3) {
            4 if !f.k.in_town => Some(res::REFUSED),
            2 if !f.k.trading => Some(res::REFUSED),
            1 => Some(res::BAD),
            n if n >= 5 => Some(res::BAD),
            _ => None,
        }),
        0x19 => {
            let g = w(0);
            first(&[f.check_stored(p, g)]).or_else(|| {
                if f.inv().cursor.is_some() {
                    Some(res::BAD)
                } else if f.it(g).page == 1 {
                    Some(res::REFUSED)
                } else if !f.k.gate {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        0x1A => first(&[cursor_check(w(0))])
            .or_else(|| (!valid_loc(u32::from(b[5]))).then_some(res::BAD)),
        // Up to the requirement failure (stat refresh and sound only).
        0x1B => {
            let loc = u32::from(b[5]);
            first(&[cursor_check(w(0))]).or_else(|| {
                if !valid_loc(loc) {
                    Some(res::BAD)
                } else if loc != 4 && loc != 5 {
                    Some(res::REFUSED)
                } else if body(9 - loc).is_none() || f.k.equip_check != 2 {
                    Some(res::REFUSED)
                } else if !f.k.requirements {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        // Up to the equip check.
        0x1C => {
            let loc = u32::from(u16::from_le_bytes([b[1], b[2]]));
            if !valid_loc(loc) {
                Some(res::BAD)
            } else if !f.k.gate || (loc == 8/* belt removal defaults to refusal */) {
                Some(res::OK)
            } else if f.inv().cursor.is_some() || body(loc).is_none() {
                Some(res::OK)
            } else if f.k.equip_check != 3 && f.k.equip_check != 4 {
                Some(res::REFUSED)
            } else {
                None
            }
        }
        // Up to the equip check.
        0x1D => {
            let loc = u32::from(b[5]);
            first(&[cursor_check(w(0))]).or_else(|| {
                if !valid_loc(loc) {
                    Some(res::BAD)
                } else if body(loc).is_none() {
                    Some(res::RANGE)
                } else if loc == 8 || !f.k.gate || f.k.equip_check != 5 {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        0x1E => {
            let loc = u32::from(b[5]);
            first(&[cursor_check(w(0))]).or_else(|| {
                if !valid_loc(loc) {
                    Some(res::BAD)
                } else if loc != 4 && loc != 5 {
                    Some(res::REFUSED)
                } else if body(loc).is_none() {
                    Some(res::RANGE)
                } else if !f.k.gate {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        0x1F => {
            let t = w(1);
            first(&[cursor_check(w(0)), f.check_stored(p, t)]).or_else(|| {
                if matches!(f.it(t).page, 1 | 2) {
                    Some(res::REFUSED)
                } else if !f.k.gate {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        // (small coordinates only: the conversion of a u32 beyond
        // i32 is not written).
        0x20 => first(&[f.check_stored(p, w(0))]).or_else(|| {
            let (x, y) = (w(1), w(2));
            let pl = &f.units[&p];
            let far = |v: u32, c: i32| (i64::from(v) - i64::from(c)).abs() > 50;
            (x < 0x8000_0000 && y < 0x8000_0000 && (far(x, pl.x) || far(y, pl.y)))
                .then_some(res::RANGE)
        }),
        0x21 => first(&[f.check_owned(p, w(0)), f.check_owned(p, w(1))])
            .or_else(|| (w(0) == w(1)).then_some(res::REFUSED)),
        // 3 for every owned item.
        0x22 => Some(first(&[f.check_owned(p, w(0))]).unwrap_or(res::REFUSED)),
        0x23 => first(&[cursor_check(w(0))]),
        0x24 => first(&[f.check_belt(p, w(0))]).or_else(|| {
            if !f.k.gate {
                Some(res::OK)
            } else if f.inv().cursor.is_some() {
                Some(res::BAD)
            } else {
                None
            }
        }),
        0x25 => first(&[cursor_check(w(0)), f.check_belt(p, w(1))])
            .or_else(|| (!f.it(w(0)).beltable).then_some(res::OK)),
        0x26 => first(&[f.check_belt(p, w(0))]),
        0x27 => first(&[f.check_owned(p, w(0)), f.check_owned(p, w(1))]),
        0x28 => first(&[cursor_check(w(0)), f.check_stored_or_equipped(p, w(1))]),
        0x29 => first(&[f.check_ground_or_owned(p, w(0)), f.check_stored(p, w(1))]),
        0x50 => {
            let (unit, amount) = (w(0), w(1) as i32);
            let gold = f.units[&p].stats.get(&14).copied().unwrap_or(0);
            let level = f.units[&p].stats.get(&12).copied().unwrap_or(0);
            if (busy && f.k.trading) || unit != P {
                Some(res::REFUSED)
            } else if amount < 0 || amount > gold || i64::from(amount) > i64::from(level) * 10_000 {
                Some(res::REFUSED)
            } else if amount == 0 {
                Some(res::OK)
            } else {
                None
            }
        }
        0x61 => {
            if !f.expansion || (busy && f.k.trading) {
                Some(res::REFUSED)
            } else if !(!f.k.used_skill && f.k.alive && f.k.hireling.is_some() && f.k.owns) {
                Some(res::OK)
            } else {
                None
            }
        }
        0x63 => {
            let g = w(0);
            first(&[f.check_stored(p, g)]).or_else(|| {
                if f.inv().cursor.is_some() {
                    Some(res::BAD)
                } else if !f.it(g).beltable {
                    Some(res::BAD)
                } else if f.it(g).page != 0 {
                    Some(res::REFUSED)
                } else if f.k.belt_slot.is_none() || !f.k.gate {
                    Some(res::OK)
                } else {
                    None
                }
            })
        }
        _ => None,
    }
}

fn run_case(dna: &[u8], msgs: &[Msg]) {
    let mut f = world(dna);
    for m in msgs {
        let b = m.bytes(&f);
        let before = state(&f);
        // The oracle reads the state the message met.
        let pre = f.clone();
        let size = HANDLED.iter().find(|&&(i, _)| i == m.id).map(|&(_, s)| s);
        let got = handle(&mut f, P, &b);
        match (size, got) {
            (None, None) => assert_eq!(state(&f), before, "foreign id {:#x}", m.id),
            (None, Some(_)) => panic!("id {:#x} is not handled", m.id),
            (Some(_), None) => assert!(b.is_empty(), "handled id {:#x} gave None", m.id),
            (Some(size), Some(r)) => {
                if b.len() != size {
                    assert_eq!(r, Ok(res::REFUSED), "size check of {:#x}", m.id);
                    assert_eq!(state(&f), before, "size check changed state");
                    continue;
                }
                if let Ok(code) = r {
                    assert!(code <= res::REFUSED, "{:#x} result {code}", m.id);
                }
                if let Some(want) = early(&pre, m, &b) {
                    assert_eq!(r, Ok(want), "{:#x} early rejection {b:?}", m.id);
                    // 0x19 / 0x63 with a cursor item answer "can't do
                    // that": the one effect
                    // their rejection has.
                    if (m.id == 0x19 || m.id == 0x63)
                        && pre.inv().cursor.is_some()
                        && f.sent.last() == Some(&super::layouts::cant_do_that())
                    {
                        f.sent.pop();
                    }
                    assert_eq!(
                        state(&f),
                        before,
                        "{:#x} rejected with an effect {b:?}",
                        m.id
                    );
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(config(512))]

    #[test]
    fn handlers_on_arbitrary_payloads(
        dna in proptest::collection::vec(any::<u8>(), 150..700),
        msgs in proptest::collection::vec(msg(), 1..12),
    ) {
        run_case(&dna, &msgs);
    }
}
