// Spec: specs/world/vendors.md; specs/items/generation.md §1.3
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked. A child of
//! `vendors::tests` for its synthetic tables and fake world.

use super::*;

// From specs/items/generation.md §1.3: `type` equivalent to T, or `type2`
// equivalent to T when `type2` > 0.
#[test]
fn is_type_reads_type2_only_when_positive() {
    let mut t = tables();
    let i = index(&t, "hax");
    // type weap, type2 ring: ring yes, amulet no.
    t.items[i].type2 = T_RING;
    assert!(t.is_type(i, T_RING as u16));
    assert!(!t.is_type(i, T_AMU as u16));
    // type2 0 is not read (type 0 is equivalent to itself).
    t.items[i].type2 = 0;
    assert!(!t.is_type(i, 0));
}

// From specs/world/vendors.md §3 r1, §5: a code names an item only when
// ≠ 0 and ≠ four spaces.
#[test]
fn valid_code_refuses_zero_and_spaces_even_when_listed() {
    let mut t = tables();
    t.items.push(item("", 1, T_WEAP)); // code four spaces
    let mut z = item("x", 1, T_WEAP);
    z.code = [0; 4];
    t.items.push(z);
    assert_eq!(t.valid_code(NO_CODE), None);
    assert_eq!(t.valid_code([0; 4]), None);
    assert_eq!(t.valid_code(code("hax")), Some(index(&t, "hax")));
}

// From specs/world/vendors.md §4 r2: the player's own vendor-chain node,
// created on first use.
#[test]
fn chain_node_is_the_players_own() {
    let t = tables();
    let mut r = record(&t, 154, 0);
    r.chain_node_mut(1).gamble_mode = true;
    r.chain_node_mut(2);
    assert!(r.chain_node_mut(1).gamble_mode);
    assert!(!r.chain_node_mut(2).gamble_mode);
    assert_eq!(r.chain.len(), 2);
}

// ------------------------------------------------------------ §9.2

use crate::world::vendors::price::{
    charges_not_full, durability_applicable, repairable, replenishable_stack, throwable,
};

fn identified(t: &VendorTables, c: &str) -> PriceItem {
    PriceItem {
        record: index(t, c),
        quality: 2,
        flags: flag::IDENTIFIED,
        file_index: -1,
        format: 101,
        durability: 20,
        max_durability: 20,
        ..Default::default()
    }
}

// From specs/world/vendors.md §9.2 rule 0: durability-applicable is
// `nodurability` = 0, `durability` ≠ 0, max durability ≠ 0, stat 152 < 1.
#[test]
fn durability_applicable_needs_all_four() {
    let base = tables();
    let lax = index(&base, "lax");
    let it = identified(&base, "lax");
    assert!(durability_applicable(&base, &it));
    let mut t = base.clone();
    t.items[lax].nodurability = 1;
    assert!(!durability_applicable(&t, &it));
    let mut t = base.clone();
    t.items[lax].durability = 0;
    assert!(!durability_applicable(&t, &it));
    let mut i2 = it.clone();
    i2.max_durability = 0;
    assert!(!durability_applicable(&base, &i2));
    let mut i3 = it.clone();
    i3.indestructible = 1;
    assert!(!durability_applicable(&base, &i3));
}

// From specs/world/vendors.md §9.2 (C): current = value & 0xFF, max =
// value >> 8; "not all full" is current < max.
#[test]
fn charges_current_low_byte_max_high_bits() {
    let with = |v: i32| PriceItem {
        charges: vec![(0, v)],
        ..PriceItem::default()
    };
    assert!(charges_not_full(&with((5 << 8) | 3)));
    assert!(!charges_not_full(&with((5 << 8) | 5)));
    assert!(!charges_not_full(&with((5 << 8) | 6)));
}

/// A stackable throwing item `tax` of type `throw`.
fn throwing() -> (VendorTables, PriceItem) {
    let mut t = tables();
    let mut r = item("tax", 100, T_THROW);
    r.stackable = 1;
    r.maxstack = 50;
    t.items.push(r);
    let it = identified(&t, "tax");
    (t, it)
}

// From specs/world/vendors.md §9.2 rule 0: replenishable stack = `repair`,
// `throwable`, `stackable`, not ethereal.
#[test]
fn replenishable_stack_needs_all_four() {
    let (t, it) = throwing();
    assert!(replenishable_stack(&t, &it));
    assert!(throwable(&t, &it));
    let mut eth = it.clone();
    eth.flags |= flag::ETHEREAL;
    assert!(!replenishable_stack(&t, &eth));
    let mut t2 = t.clone();
    t2.items.last_mut().unwrap().stackable = 0;
    assert!(!replenishable_stack(&t2, &it));
    let mut t3 = t.clone();
    t3.itemtypes[T_THROW as usize].repair = 0;
    assert!(!replenishable_stack(&t3, &it));
    let mut t4 = t.clone();
    t4.itemtypes[T_THROW as usize].throwable = 0;
    assert!(!replenishable_stack(&t4, &it));
}

// From specs/world/vendors.md §9.2 rule 0: a `repair` type alone is not
// enough (needs a replenishable stack or durability).
#[test]
fn repairable_needs_stack_or_durability() {
    let t = tables();
    let mut it = identified(&t, "lax");
    assert!(repairable(&t, &it));
    it.max_durability = 0;
    assert!(!repairable(&t, &it));
}

// From specs/world/vendors.md §9.2 rule 4: guard'(X, mult) for S ≥
// 0x10000 is (X/1024)·mult.
#[test]
fn affix_guard_above_0x10000() {
    let mut t = tables();
    let cost = 0x1_0000 + 1023;
    t.items.push(item("big", cost, T_WEAP));
    let mut it = identified(&t, "big");
    it.quality = 4;
    it.prefix[0] = 1; // mult 2048, add 100
    let x = cost as i32;
    let s = x + 100 + (x / 1024) * 2048;
    // npc 600: sell mult 1024, S ≥ 0x10000 → (S/1024)·1024.
    let want = (s / 1024) * 1024;
    let c = PriceCtx {
        difficulty: 0,
        npc_class: 600,
        reduced_prices: 0,
        player_level: 1,
        quest_slots: [0; 3],
    };
    assert_eq!(cost_of(&t, &c, &it), want);
}

fn cost_of(t: &VendorTables, c: &PriceCtx, it: &PriceItem) -> i32 {
    crate::world::vendors::cost(t, c, Some(it), tx::BUY).unwrap()
}

// From specs/world/vendors.md §9.2 (A): S < 0x10000 → m·S/1024.
#[test]
fn item_skill_cost_small_s_uses_m_times_s() {
    let mut t = tables();
    t.skills[0].cost.mult = 1536;
    let mut it = identified(&t, "lax");
    it.item_skills = vec![(0, 2)];
    let c = PriceCtx {
        difficulty: 0,
        npc_class: 600,
        reduced_prices: 0,
        player_level: 1,
        quest_slots: [0; 3],
    };
    // k = 3: (1536·100/1024 + 10)·3 = 480.
    assert_eq!(cost_of(&t, &c, &it), 100 + 480);
}

// From specs/world/vendors.md §3 step 4: ilvl 5–9 use the 85 band.
#[test]
fn store_quality_band_at_ilvl_5() {
    use crate::items::q;
    use crate::rng::Seed;
    use crate::world::vendors::store::quality_draw;
    // Seed {0, 88}: one step gives lo' = 88.
    assert_eq!(quality_draw(&mut Seed::new(0, 88), 5), q::SUPERIOR);
    assert_eq!(quality_draw(&mut Seed::new(0, 88), 4), q::NORMAL);
}

// ------------------------------------------------------------ §3 store

mod store {
    use super::*;
    use crate::rng::Seed;
    use crate::world::vendors::store::{
        generate, make_store_item, mark, place_store_page, range, StoreCtx,
    };

    fn ctx<'a>(t: &'a VendorTables, seed: &'a mut Seed) -> StoreCtx<'a> {
        StoreCtx { tables: t, seed }
    }

    // From specs/world/vendors.md §3.1 r1: Hell ultracode needs r <
    // ilvl·16 + 1000 (r = 1560 at ilvl 35 falls to the ubercode test).
    #[test]
    fn hell_ultra_boundary() {
        let t = tables();
        let hax = index(&t, "hax");
        let mut rec = record(&t, class::CHARSI, 0);
        let mut w = Fake::new(class::CHARSI);
        w.difficulty = 2;
        let mut seed = Seed::new(0, 1560);
        make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 35, 30).unwrap();
        assert_eq!(w.created[0].0, index(&t, "9ha"));
    }

    // From specs/world/vendors.md §3.1 r5: flag 0x10, plus flag 1 with
    // filled sockets.
    #[test]
    fn mark_keeps_identified_with_sockets() {
        let t = tables();
        let mut w = Fake::new(class::CHARSI);
        let g = w.add_item(index(&t, "hax"), 2, 0);
        w.units.get_mut(&g).unwrap().sockets = true;
        mark(&mut w, class::CHARSI, UnitId(g));
        assert_eq!(w.unit(g).flags, flag::IDENTIFIED | flag::NEW);
    }

    // From specs/world/vendors.md §3.1 r4: only page 1 retries on page 2.
    #[test]
    fn only_page_1_retries_on_page_2() {
        let t = tables();
        let mut w = Fake::new(class::CHARSI);
        w.store_room = [0, 0, 1000, 1000];
        let weap = w.add_item(index(&t, "hax"), 2, 0);
        assert_eq!(
            place_store_page(&t, &mut w, class::CHARSI, UnitId(weap)),
            Some(true)
        );
        assert_eq!(w.unit(weap).page, 2);
        let armo = w.add_item(index(&t, "qui"), 2, 0);
        assert_eq!(
            place_store_page(&t, &mut w, class::CHARSI, UnitId(armo)),
            Some(false)
        );
        assert_eq!(w.store_used[2], 1);
    }

    fn codes(t: &VendorTables, w: &Fake, rec: &VendorRecord) -> Vec<String> {
        rec.store
            .iter()
            .map(|u| {
                let c = t.items[w.unit(u.0).record].code;
                String::from_utf8_lossy(&c).trim_end().to_string()
            })
            .collect()
    }

    // From specs/world/vendors.md §3 r1: only level > ilvl skips.
    #[test]
    fn item_at_store_level_is_made() {
        let mut t = tables();
        let hax = index(&t, "hax");
        t.items[hax].level = 6; // ilvl 6 at L_p 1
        let mut rec = record(&t, class::CHARSI, 0);
        let mut w = Fake::new(class::CHARSI);
        generate(
            &mut ctx(&t, &mut Seed::new(1, 666)),
            &mut rec,
            &mut w,
            p(),
            0,
        )
        .unwrap();
        assert_eq!(codes(&t, &w, &rec)[..2], ["hax", "hax"]);
    }

    // From specs/world/vendors.md §3 r2, r5: ilvl 25 makes no normal
    // items and draws k.
    #[test]
    fn store_level_25_has_no_normal_items() {
        let t = tables();
        let mut rec = record(&t, class::CHARSI, 4);
        let mut w = Fake::new(class::CHARSI);
        w.set(PLAYER, stat::LEVEL, 20);
        let mut seed = Seed::new(5, 666);
        generate(&mut ctx(&t, &mut seed), &mut rec, &mut w, p(), 0).unwrap();
        let mut s = Seed::new(5, 666);
        let k = range(&mut s, 1, 3) + 1;
        let n_mag = range(&mut s, 1, k);
        assert_eq!(seed, s);
        assert_eq!(rec.store.len(), n_mag as usize + 2);
        assert!(w.created.iter().all(|&(_, _, ilvl)| ilvl == 25));
    }

    // From specs/world/vendors.md §3 r3: item format 100 keeps version-100
    // items.
    #[test]
    fn item_format_100_keeps_expansion_items() {
        let mut t = tables();
        let ktr = index(&t, "ktr");
        t.items[ktr].columns[CHARSI_COL] = [1, 3, 0, 0, 0];
        let mut rec = record(&t, class::CHARSI, 0);
        rec.items.retain(|e| e.code == code("ktr"));
        rec.perm.clear();
        let mut w = Fake::new(class::CHARSI);
        w.format = 100;
        generate(
            &mut ctx(&t, &mut Seed::new(1, 666)),
            &mut rec,
            &mut w,
            p(),
            0,
        )
        .unwrap();
        assert!(!rec.store.is_empty());
    }

    /// A store of `n` normal `hax` (no room anywhere), then `aqv`, `cqv`.
    fn failing_store(n: u8) -> Fake {
        let t = tables();
        let mut rec = record(&t, class::CHARSI, 0);
        rec.items = vec![ColumnEntry {
            min: n,
            max: n - 1,
            magic_min: 0,
            magic_max: 0,
            code: code("hax"),
            magic_lvl: 0,
        }];
        let mut w = Fake::new(class::CHARSI);
        w.store_room = [0; 4];
        generate(
            &mut ctx(&t, &mut Seed::new(1, 666)),
            &mut rec,
            &mut w,
            p(),
            0,
        )
        .unwrap();
        w
    }

    // From specs/world/vendors.md §3: a null permanent item counts in
    // fails; fails > 32 after a permanent item stops.
    #[test]
    fn permanent_nulls_count_and_stop_above_32() {
        let t = tables();
        // 32 normal nulls, then aqv's null makes 33: cqv is not made.
        let w = failing_store(32);
        assert_eq!(w.created.len(), 33);
        assert_eq!(w.created[32].0, index(&t, "aqv"));
        // 31 normal nulls, aqv's null makes 32: cqv is still made.
        let w = failing_store(31);
        assert_eq!(w.created.len(), 33);
        assert_eq!(w.created[32].0, index(&t, "cqv"));
    }
}
