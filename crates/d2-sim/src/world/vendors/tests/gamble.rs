// Spec: specs/world/vendors.md §5 (Test vectors, edge case 8)
use super::*;
use crate::items::q;
use crate::rng::Seed;
use crate::world::vendors::gamble::{
    drop_list, identify_gamble, level_draw, make_list, quality_draw,
};
use crate::world::vendors::store::{open, StoreCtx};

/// Tables with a gamble index [hax, lax, rin, amu, 9ha] and thresholds 3
/// (5 from level 60).
fn gamble_tables() -> VendorTables {
    let mut t = tables();
    let ids = ["hax", "lax", "rin", "amu", "9ha"].map(|c| index(&t, c) as u32);
    t.gamble_index = Some(ids.to_vec());
    t.gamble_thresholds = (0..100).map(|l| if l >= 60 { 5 } else { 3 }).collect();
    t
}

/// The expected list, drawn step by step with the spec's order (§5.1).
fn expected(t: &VendorTables, seed: &mut Seed, lp: i32, expansion: bool) -> Vec<(usize, u8, i32)> {
    let idx = t.gamble_index.clone().unwrap();
    let o = t.difficulty[0];
    let mut out = Vec::new();
    let mut c = 0;
    while c < 14 {
        let lg = ((seed.step() % 10) as i32 - 5 + lp).clamp(5, 99);
        let th = t.gamble_thresholds[lg as usize] as i32;
        let mut id = idx[if th < 1 { 0 } else { seed.roll(th) as usize }] as usize;
        if !expansion && t.items[id].version >= 100 {
            continue;
        }
        if c == 0 {
            id = index(t, "rin");
        } else if c == 1 {
            id = index(t, "amu");
        }
        if expansion && t.items[id].ubercode == code("9ha") {
            let w_u = (lg - 31) * o.uber + 1;
            if w_u > 0 {
                if (seed.roll(10_000) as i32) < w_u {
                    id = index(t, "9ha");
                } else {
                    let w_x = (lg - 54) * o.ultra + 1;
                    if w_x > 0 && (seed.roll(10_000) as i32) < w_x {
                        id = index(t, "7ha");
                    }
                }
            }
        }
        let r = seed.step() % 100_000;
        let quality = if r < 50 {
            q::UNIQUE
        } else if r < 150 {
            q::SET
        } else if r < 10_150 {
            q::RARE
        } else {
            q::MAGIC
        };
        out.push((id, quality, lg));
        c += 1;
    }
    out
}

fn gamble(t: &VendorTables, w: &mut Fake, seed: &mut Seed) -> VendorRecord {
    let mut rec = record(t, class::GHEED, 0);
    make_list(&mut StoreCtx { tables: t, seed }, &mut rec, w, p());
    rec
}

// Covers: specs/world/vendors.md §5.1 r1
#[test]
fn level_draw_vector() {
    let mut s = Seed::new(1, 666);
    let got: Vec<i32> = (0..3).map(|_| level_draw(&mut s, 10)).collect();
    assert_eq!(got, vec![6, 6, 7]);
    assert_eq!(level_draw(&mut Seed::new(0, 0), 1), 5, "≤ 5 → 5");
    assert_eq!(level_draw(&mut Seed::new(0, 9), 120), 99, "≥ 99 → 99");
}

// Covers: specs/world/vendors.md §5.1 r6
#[test]
fn quality_bands() {
    let q = |x: u32| quality_draw(&mut Seed::new(0, x), 10_000, 100, 50);
    assert_eq!(
        [q(49), q(50), q(149), q(150), q(10_149), q(10_150)],
        [q::UNIQUE, q::SET, q::SET, q::RARE, q::RARE, q::MAGIC]
    );
    let mut s = Seed::new(3, 666);
    assert_eq!(quality_draw(&mut s, 0, 0, 0), q::MAGIC);
    assert_eq!(s, Seed::new(3, 666), "H = 0: no draw");
}

// Covers: specs/world/vendors.md §5.1 text, §5.1 r2, §5.1 r4, §5.1 r5, §5.1 r7, §5.1 r8, §3.2, §edge-cases-original-bugs r8
#[test]
fn expansion_list() {
    let t = gamble_tables();
    let mut upgraded = false;
    for lp in [1, 40, 62, 90] {
        let mut w = Fake::new(class::GHEED);
        w.set(PLAYER, stat::LEVEL, lp);
        let mut seed = Seed::new(7, 666);
        let rec = gamble(&t, &mut w, &mut seed);
        let mut s = Seed::new(7, 666);
        let want = expected(&t, &mut s, lp, true);
        assert_eq!(w.created, want, "L_p {lp}");
        assert_eq!(seed, s, "L_p {lp}: draw order");
        assert_eq!(w.created[0].0, index(&t, "rin"));
        assert_eq!(w.created[1].0, index(&t, "amu"));
        upgraded |= w.created.iter().any(|&(id, ..)| t.items[id].version == 100);
        let list = &rec.gamble_lists[0];
        assert_eq!((list.player, list.items.len()), (PLAYER, 14));
        for u in &list.items {
            let unit = w.unit(u.0);
            assert_eq!(unit.flags & flag::IDENTIFIED, 0, "unidentified");
            assert_eq!(unit.page, 0);
        }
    }
    assert!(upgraded, "an uber or ultra pick happened");
}

// Covers: specs/world/vendors.md §5.1 r3
#[test]
fn classic_list() {
    let t = gamble_tables();
    let mut w = Fake::new(class::GHEED);
    w.expansion = false;
    w.format = 2;
    w.set(PLAYER, stat::LEVEL, 70);
    let mut seed = Seed::new(11, 666);
    gamble(&t, &mut w, &mut seed);
    let mut s = Seed::new(11, 666);
    assert_eq!(w.created, expected(&t, &mut s, 70, false));
    assert!(w.created.iter().all(|&(id, ..)| id != index(&t, "9ha")));
    assert_eq!(seed, s);
    // A record missing from the tables: stop.
    let mut t2 = t.clone();
    t2.gamble_index = Some(vec![9999]);
    let mut w = Fake::new(class::GHEED);
    w.expansion = false;
    gamble(&t2, &mut w, &mut Seed::new(1, 666));
    assert!(w.created.is_empty());
}

// Covers: specs/world/vendors.md §5.1 r7
#[test]
fn no_room_stops() {
    let t = gamble_tables();
    let mut w = Fake::new(class::GHEED);
    w.gamble_room = 3;
    let rec = gamble(&t, &mut w, &mut Seed::new(1, 666));
    assert_eq!(w.created.len(), 4);
    assert_eq!(rec.gamble_lists[0].items.len(), 3);
    assert_eq!(w.destroyed.len(), 1);
}

// Covers: specs/world/vendors.md §4 r2, §5.4
#[test]
fn open_and_drop() {
    let t = gamble_tables();
    let mut w = Fake::new(class::GHEED);
    let mut seed = Seed::new(1, 666);
    let mut rec = record(&t, class::GHEED, 0);
    let mut c = StoreCtx {
        tables: &t,
        seed: &mut seed,
    };
    open(&mut c, &mut rec, &mut w, npc(), p(), true, true, 0).unwrap();
    assert_eq!(rec.chain_node(PLAYER).map(|n| n.gamble_mode), Some(true));
    assert!(!rec.store_generated, "gamble open makes no store");
    let list = rec.gamble_lists[0].items.clone();
    // The list items are shown; a second open keeps the list.
    assert!(list.iter().all(|u| w.trade_inv.contains(&u.0)));
    open(&mut c, &mut rec, &mut w, npc(), p(), true, true, 0).unwrap();
    assert_eq!(rec.gamble_lists.len(), 1);
    assert_eq!(rec.gamble_lists[0].items, list);
    drop_list(&mut rec, &mut w, PLAYER);
    assert!(rec.gamble_lists.is_empty());
    assert_eq!(w.removed, list.iter().map(|u| u.0).collect::<Vec<_>>());
}

// Covers: specs/world/vendors.md §5.5
#[test]
fn identify_after_buy() {
    let t = tables();
    let mut w = Fake::new(class::GHEED);
    let item = w.add_item(index(&t, "rin"), q::MAGIC, 0);
    let msg = |g: u32| {
        let mut m = vec![0x37];
        m.extend(g.to_le_bytes());
        m
    };
    assert_eq!(identify_gamble(&mut w, p(), &msg(item)[..4]), 3, "size");
    assert_eq!(identify_gamble(&mut w, p(), &msg(999)), 2, "missing");
    assert_eq!(
        identify_gamble(&mut w, p(), &msg(item)),
        3,
        "not the last bought"
    );
    w.last_bought = item;
    assert_eq!(identify_gamble(&mut w, p(), &msg(item)), 0);
    assert!(w.unit(item).flags & flag::IDENTIFIED != 0);
    assert!(w.sent.is_empty(), "no 0x2A");
}
