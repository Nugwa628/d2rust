//! Gap tests for property rules the `props.rs` tests do not pin.

use super::*;
use crate::items::props::{
    apply_affix, apply_property, apply_quality_row, apply_set_item, apply_socket_filler,
    apply_unique, PropCtx,
};
use crate::items::quality::superior;
use crate::items::tables::{
    GemRec, PropSlot, QualityRec, RuneRec, SetItemRec, SetRec, SkillRec, UniqueRec,
};
use crate::items::{flag, q, stat, ItemRequest};
use d2_data::tables::{
    Automagic, Gems, Magicprefix, Magicsuffix, Properties, Qualityitems, Runes, Setitems, Sets,
    Uniqueitems,
};

fn zero<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

fn run(t: &ItemTables, it: &mut Item<FakeStats>, mode: u8, r: PropRec) {
    apply_property(t, it, &mut PropCtx::item(mode), &r);
}

/// Every source table's record columns project to `{code, param, min,
/// max}`; a code read as < 0 is none.
#[test]
fn record_shape_from_tables() {
    let want = rec(1, 2, 3, 4);
    let none = rec(-1, 0, 0, 0);
    let mut a: Magicprefix = zero();
    (a.mod1code, a.mod1param, a.mod1min, a.mod1max) = (1, 2, 3, 4);
    a.mod3code = u32::MAX;
    let r = AffixRec::from(&a);
    assert_eq!((r.mods[0], r.mods[2].code), (want, -1));
    let mut a: Magicsuffix = zero();
    (a.mod2code, a.mod2param, a.mod2min, a.mod2max) = (1, 2, 3, 4);
    assert_eq!(AffixRec::from(&a).mods[1], want);
    let mut a: Automagic = zero();
    (a.mod3code, a.mod3param, a.mod3min, a.mod3max) = (1, 2, 3, 4);
    assert_eq!(AffixRec::from(&a).mods[2], want);
    let mut qi: Qualityitems = zero();
    (qi.mod2code, qi.mod2param, qi.mod2min, qi.mod2max) = (1, 2, 3, 4);
    qi.mod1code = u32::MAX;
    let r = QualityRec::from(&qi);
    assert_eq!(r.mods, [none, want]);
    let mut u: Uniqueitems = zero();
    (u.prop12, u.par12, u.min12, u.max12) = (1, 2, 3, 4);
    (u.prop1, u.par1, u.min1, u.max1) = (1, 2, 3, 4);
    let r = UniqueRec::from(&u);
    assert_eq!((r.props[0], r.props[11]), (want, want));
    let mut s: Setitems = zero();
    (s.prop9, s.par9, s.min9, s.max9) = (1, 2, 3, 4);
    (s.aprop1a, s.apar1a, s.amin1a, s.amax1a) = (1, 2, 3, 4);
    (s.aprop5b, s.apar5b, s.amin5b, s.amax5b) = (1, 2, 3, 4);
    let r = SetItemRec::from_record(&s, &vec![0u8; Setitems::SIZE]);
    assert_eq!((r.props[8], r.aprops[0], r.aprops[9]), (want, want, want));
    let mut s: Sets = zero();
    (s.pcode2a, s.pparam2a, s.pmin2a, s.pmax2a) = (1, 2, 3, 4);
    (s.fcode8, s.fparam8, s.fmin8, s.fmax8) = (1, 2, 3, 4);
    let r = SetRec::from_record(&s, &vec![0u8; Sets::SIZE]);
    assert_eq!((r.partial[0], r.full[7]), (want, want));
    let mut w: Runes = zero();
    (w.t1code1, w.t1param1, w.t1min1, w.t1max1) = (1, 2, 3, 4);
    (w.t1code7, w.t1param7, w.t1min7, w.t1max7) = (1, 2, 3, 4);
    let r = RuneRec::from(&w);
    assert_eq!((r.props[0], r.props[6]), (want, want));
    let mut g: Gems = zero();
    (
        g.weaponmod1code,
        g.weaponmod1param,
        g.weaponmod1min,
        g.weaponmod1max,
    ) = (1, 2, 3, 4);
    (
        g.helmmod2code,
        g.helmmod2param,
        g.helmmod2min,
        g.helmmod2max,
    ) = (1, 2, 3, 4);
    (
        g.shieldmod3code,
        g.shieldmod3param,
        g.shieldmod3min,
        g.shieldmod3max,
    ) = (1, 2, 3, 4);
    let r = GemRec::from(&g);
    assert_eq!(
        (r.mods[0][0], r.mods[1][1], r.mods[2][2]),
        (want, want, want)
    );
    // A record with code < 0 does nothing.
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, 7)];
    let mut it = item(i, 1);
    run(&t, &mut it, 0, none);
    assert!(it.stats.lists.is_empty());
}

/// Slot k of a properties row is (`funcK+1`, `statK+1`, `setK+1`,
/// `valK+1`).
#[test]
fn property_slots_from_table() {
    let mut p: Properties = zero();
    (p.func1, p.stat1, p.set1, p.val1) = (1, 11, 0, 101);
    (p.func2, p.stat2, p.set2, p.val2) = (2, 12, 1, 102);
    (p.func3, p.stat3, p.set3, p.val3) = (3, 13, 0, 103);
    (p.func4, p.stat4, p.set4, p.val4) = (4, 14, 1, 104);
    (p.func5, p.stat5, p.set5, p.val5) = (5, 15, 0, 105);
    (p.func6, p.stat6, p.set6, p.val6) = (6, 16, 1, 106);
    (p.func7, p.stat7, p.set7, p.val7) = (7, 17, 0, 107);
    let r = PropertyRec::from(&p);
    for (k, s) in r.slots.iter().enumerate() {
        let k = k as u16;
        assert_eq!(
            *s,
            PropSlot {
                func: (k + 1) as u8,
                stat: 11 + k,
                set: (k % 2) as u8,
                val: 101 + k,
            }
        );
    }
}

/// Mode records and stop rules; every mode writes the item's own list
/// (state 0, flags 0x40).
#[test]
fn modes_records_and_stop_rules() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    // Property k writes stat 100 + k.
    t.properties = (0..20).map(|k| prop1(1, 100 + k)).collect();
    let r = |k: i32| rec(k, 0, 1, 1);
    let n = PropRec::NONE;
    let keys = |it: &Item<FakeStats>| -> Vec<u16> {
        assert!(it.stats.lists.keys().all(|&k| k == ListKey::ITEM));
        it.stats
            .lists
            .get(&ListKey::ITEM)
            .map_or(vec![], |l| l.keys().map(|&(s, _)| s).collect())
    };
    // Mode 0: mod1–mod3, first code < 0 ends.
    t.magic = vec![
        AffixRec {
            mods: [r(0), n, r(2)],
            ..Default::default()
        },
        AffixRec {
            mods: [r(0), r(1), r(2)],
            ..Default::default()
        },
    ];
    let mut it = item(i, 1);
    apply_affix(&t, &mut it, 1);
    assert_eq!(keys(&it), vec![100]);
    let mut it = item(i, 1);
    apply_affix(&t, &mut it, 2);
    assert_eq!(keys(&it), vec![100, 101, 102]);
    // Mode 1: mod1, mod2, first code < 0 ends.
    t.qualityitems = vec![
        QualityRec {
            mods: [n, r(1)],
            ..Default::default()
        },
        QualityRec {
            mods: [r(0), r(1)],
            ..Default::default()
        },
    ];
    let mut it = item(i, 1);
    apply_quality_row(&t, &mut it, 0);
    assert_eq!(keys(&it), Vec::<u16>::new());
    let mut it = item(i, 1);
    apply_quality_row(&t, &mut it, 1);
    assert_eq!(keys(&it), vec![100, 101]);
    // Mode 3: all 12 run, code < 0 skipped; file index outside → nothing.
    let mut props = [n; 12];
    props[0] = r(0);
    props[2] = r(2);
    props[11] = r(11);
    t.uniques = vec![UniqueRec {
        props,
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.file_index = 0;
    apply_unique(&t, &mut it);
    assert_eq!(keys(&it), vec![100, 102, 111]);
    for fi in [-1, 1] {
        let mut it = item(i, 1);
        it.file_index = fi;
        apply_unique(&t, &mut it);
        assert!(it.stats.lists.is_empty());
    }
    // Mode 4: prop1–prop9 then aprops, code < 0 skipped.
    let mut props = [n; 9];
    props[1] = r(1);
    props[8] = r(8);
    let mut aprops = [n; 10];
    aprops[9] = r(19);
    t.setitems = vec![SetItemRec {
        props,
        aprops,
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.file_index = 0;
    apply_set_item(&t, &mut it);
    assert_eq!(keys(&it), vec![101, 108, 119]);
    let mut it = item(i, 1);
    it.file_index = 1;
    apply_set_item(&t, &mut it);
    assert!(it.stats.lists.is_empty());
    // Modes 2 / 5: three records per block, first code < 0 ends.
    let mut g = item_rec(ty::GEM, b"gcv ");
    g.gemoffset = 0;
    let gem = push_item(&mut t, g);
    let mut ru = item_rec(ty::RUNE, b"r01 ");
    ru.gemoffset = 0;
    let rune = push_item(&mut t, ru);
    t.gems = vec![GemRec {
        mods: [[r(0), n, r(2)], [r(3), r(4), r(5)], [n, r(6), r(7)]],
    }];
    for f in [gem, rune] {
        let mut it = item(f, 1);
        apply_socket_filler(&t, &mut it, 0);
        assert_eq!(keys(&it), vec![100]);
        let mut it = item(f, 1);
        apply_socket_filler(&t, &mut it, 1);
        assert_eq!(keys(&it), vec![103, 104, 105]);
        let mut it = item(f, 1);
        apply_socket_filler(&t, &mut it, 2);
        assert_eq!(keys(&it), Vec::<u16>::new());
    }
}

/// Base reset: armor defense to max(base + 1, maxac + 1); weapon damage
/// columns (throw damage only when throwable), for reset functions (2
/// always, 1 only in mode 1).
#[test]
fn base_reset() {
    let mut t = tables();
    t.properties = vec![
        prop1(2, stat::ARMORCLASS),
        prop1(2, stat::ARMOR_PERCENT),
        prop1(1, stat::ARMORCLASS),
        prop1(2, stat::MAXDAMAGE_PERCENT),
        prop1(2, stat::MINDAMAGE_PERCENT),
        prop1(2, stat::MAXDAMAGE),
        prop1(2, stat::MINDAMAGE),
    ];
    let mut r = item_rec(ty::HELM, b"cap ");
    r.maxac = 10;
    let h = push_item(&mut t, r.clone());
    r.maxac = 0;
    let h0 = push_item(&mut t, r);
    let v = rec(0, 0, 5, 5);
    for (code, base, want) in [(0, 3, 11), (0, 20, 21), (1, 3, 11)] {
        let mut it = item(h, 1);
        it.stats.set_base(stat::ARMORCLASS, 0, base);
        run(&t, &mut it, 0, PropRec { code, ..v });
        assert_eq!(it.stats.base(stat::ARMORCLASS, 0), want, "{code} {base}");
    }
    let mut it = item(h0, 1);
    it.stats.set_base(stat::ARMORCLASS, 0, 3);
    run(&t, &mut it, 0, v);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 3, "maxac 0");
    // Function 1: only in mode 1.
    let mut it = item(h, 1);
    it.stats.set_base(stat::ARMORCLASS, 0, 3);
    run(&t, &mut it, 0, PropRec { code: 2, ..v });
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 3);
    run(&t, &mut it, 1, PropRec { code: 2, ..v });
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 11);
    // Weapons.
    let wpn = |t: &mut ItemTables, ty: u16| {
        let mut r = item_rec(ty, b"wpn ");
        (r.mindam, r.maxdam, r.mindam2, r.maxdam2) = (2, 9, 0, 0);
        (r.minmisdam, r.maxmisdam) = (4, 12);
        push_item(t, r)
    };
    let axe = wpn(&mut t, AXE);
    let thrown = wpn(&mut t, THROWN);
    let ring = push_item(&mut t, item_rec(RING, b"rin "));
    let fresh = |i: usize| {
        let mut it = item(i, 1);
        for s in [21, 22, 23, 24, 159, 160] {
            it.stats.set_base(s, 0, 100);
        }
        it
    };
    for (i, code, want) in [
        (axe, 3, [100, 9, 100, 100, 100, 100]),
        (axe, 5, [100, 9, 100, 100, 100, 100]),
        (thrown, 3, [100, 9, 100, 100, 100, 12]),
        (axe, 4, [2, 100, 100, 100, 100, 100]),
        (axe, 6, [2, 100, 100, 100, 100, 100]),
        (thrown, 4, [2, 100, 100, 100, 4, 100]),
        (ring, 3, [100; 6]),
    ] {
        let mut it = fresh(i);
        run(&t, &mut it, 0, PropRec { code, ..v });
        let got = [21, 22, 23, 24, 159, 160].map(|s| it.stats.base(s, 0));
        assert_eq!(got, want, "record {i} code {code}");
    }
}

/// Function 11 (skill on event): chance, the three level rules, layer
/// skill × 64 + (level & 63).
#[test]
fn func11_skill_on_event() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(11, 195)];
    t.skills = vec![
        SkillRec::default(),
        SkillRec {
            charclass: 0xFF,
            itypea1: 0,
            reqlevel: 18,
            maxlvl: 20,
        },
        SkillRec {
            charclass: 0xFF,
            itypea1: 0,
            reqlevel: 9,
            maxlvl: 0,
        },
        SkillRec {
            charclass: 0xFF,
            itypea1: 0,
            reqlevel: 1,
            maxlvl: 5,
        },
    ];
    // (skill, ilvl, min, max) → (layer level, value)
    for (skill, ilvl, min, max, level, value) in [
        (1, 50, 0, 5, 5, 5),     // max > 0; chance < 1 → 5
        (1, 50, 12, 0, 9, 12),   // (50 − 18) / 4 + 1
        (1, 10, -3, 0, 1, 5),    // ≤ 0 → 1
        (3, 99, 7, 0, 5, 7),     // capped at maxlvl 5
        (2, 99, 7, 0, 20, 7),    // maxlvl < 1 → 20
        (2, 70, 7, -3, 2, 7),    // d = −(90 / −3) = 30: 61 / 30
        (2, 99, 7, -200, 26, 7), // d = max(0, 1) = 1: 90, & 63
        (2, 5, 7, -3, 1, 7),     // ≤ 0 → 1
    ] {
        let mut it = item(i, 1);
        it.ilvl = ilvl;
        run(&t, &mut it, 0, rec(0, skill, min, max));
        let layer = skill as u16 * 64 + level;
        assert_eq!(
            it.stats.lists[&ListKey::ITEM],
            BTreeMap::from([((195, layer), value)]),
            "skill {skill} ilvl {ilvl} max {max}"
        );
        assert_eq!(it.item_seed, Seed::init_low(1), "no draw");
    }
    // Skill out of the table: skill 0 ("out of skills → 0"), its
    // row's level rules (max 5 → level 5), layer 0 × 64 + 5.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(0, 9, 5, 5));
    assert_eq!(
        it.stats.lists[&ListKey::ITEM],
        BTreeMap::from([((195, 5), 5)])
    );
}

/// Slot 1 = function 8 on stat 9 with the same record (min = max = 0):
/// it shows slot 0's return value, or nothing when that is 0.
fn with_probe(mut p: PropertyRec) -> PropertyRec {
    p.slots[1] = PropSlot {
        func: 8,
        stat: 9,
        set: 0,
        val: 0,
    };
    p
}

#[test]
fn func13_durability() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.properties = vec![prop1(13, stat::MAXDURABILITY), prop1(13, 9)];
    let mut it = item(i, 1);
    it.stats.set_base(stat::MAXDURABILITY, 0, 20);
    it.stats.set_base(stat::DURABILITY, 0, 7);
    run(&t, &mut it, 0, rec(0, 0, 10, 10));
    assert_eq!(it.stats.item_list(stat::MAXDURABILITY, 0), 10);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 30);
    // A zero add: durability untouched.
    let mut it = item(i, 1);
    it.stats.set_base(stat::MAXDURABILITY, 0, 20);
    it.stats.set_base(stat::DURABILITY, 0, 7);
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 7);
    // Max durability 0: durability untouched.
    let mut it = item(i, 1);
    it.stats.set_base(stat::DURABILITY, 0, 7);
    run(&t, &mut it, 0, rec(1, 0, 4, 4));
    assert_eq!(it.stats.item_list(9, 0), 4);
    assert_eq!(it.stats.base(stat::DURABILITY, 0), 7);
}

#[test]
fn func15_16_17() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    (r.mindam, r.maxdam) = (3, 3);
    let i = push_item(&mut t, r);
    t.properties = vec![
        with_probe(prop1(15, stat::MINDAMAGE)), // 0
        with_probe(prop1(15, 7)),               // 1
        with_probe(prop1(16, stat::MAXDAMAGE)), // 2
        with_probe(prop1(16, stat::MINDAMAGE)), // 3
        with_probe(prop1(17, stat::MAXDAMAGE)), // 4
        with_probe(prop1(17, 7)),               // 5
        with_probe(prop1(15, stat::MAXDAMAGE)), // 6
    ];
    let list = |it: &Item<FakeStats>| {
        it.stats
            .lists
            .get(&ListKey::ITEM)
            .cloned()
            .unwrap_or_default()
    };
    // 15 on stat 21 → function 5 (floor: 1 − 3 = −2); returns min.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(0, 0, -5, 0));
    assert_eq!(list(&it), BTreeMap::from([((21, 0), -2), ((9, 0), -5)]));
    assert_eq!(it.item_seed, Seed::init_low(1));
    // 15 on another stat → plain add.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(1, 0, 4, 0));
    assert_eq!(list(&it), BTreeMap::from([((7, 0), 4), ((9, 0), 4)]));
    // 16 on stat 22 → function 6 (floor −3); returns max.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(2, 0, 0, -5));
    assert_eq!(list(&it), BTreeMap::from([((22, 0), -3), ((9, 0), -5)]));
    // 16 on stat 21 → plain add (function 5 is for 15 only).
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(3, 0, 0, -5));
    assert_eq!(list(&it), BTreeMap::from([((21, 0), -5), ((9, 0), -5)]));
    // 15 on stat 22 → plain add.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(6, 0, -5, 0));
    assert_eq!(list(&it), BTreeMap::from([((22, 0), -5), ((9, 0), -5)]));
    // 17: param when ≠ 0 (no draw), on stat 22 → function 6.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(4, -5, 0, 0));
    assert_eq!(list(&it), BTreeMap::from([((22, 0), -3), ((9, 0), -5)]));
    assert_eq!(it.item_seed, Seed::init_low(1));
    // 17: param 0 → roll(min..max).
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(5, 0, 2, 9));
    let mut s = Seed::init_low(1);
    let v = s.roll(8) as i32 + 2;
    assert_eq!(it.item_seed, s);
    assert_eq!(list(&it), BTreeMap::from([((7, 0), v), ((9, 0), v)]));
    // 17: value 0 → return 0, nothing written.
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(5, 0, 0, 0));
    assert!(it.stats.lists.is_empty());
}

#[test]
fn func20_indestructible() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![with_probe(prop1(20, 0))];
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_eq!(it.stats.item_list(stat::INDESTRUCTIBLE, 0), 2, "added");
    assert_eq!(it.stats.item_list(9, 0), 2, "returns 1 each time");
    // itemstatcost with 152 rows: no stat 152, still returns 1.
    t.valshift.truncate(152);
    let mut it = item(i, 1);
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_eq!(it.stats.item_list(stat::INDESTRUCTIBLE, 0), 0);
    assert_eq!(it.stats.item_list(9, 0), 1);
}

#[test]
fn func23_ethereal() {
    let mut t = tables();
    let mut r = item_rec(AXE, b"axe ");
    r.durability = 20;
    let i = push_item(&mut t, r.clone());
    r.nodurability = 1;
    let nd = push_item(&mut t, r);
    t.properties = vec![with_probe(prop1(23, 0))];
    let fresh = |i: usize| {
        let mut it = item(i, 1);
        it.stats.set_base(stat::DURABILITY, 0, 20);
        it.stats.set_base(stat::MINDAMAGE, 0, 10);
        it
    };
    let mut it = fresh(i);
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_ne!(it.flags & flag::ETHEREAL, 0);
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 15);
    assert_eq!(it.stats.item_list(9, 0), 1, "returns 1");
    // Already ethereal: nothing, returns 0.
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 15);
    assert_eq!(it.stats.item_list(9, 0), 1);
    // No durability: nothing.
    let mut it = fresh(nd);
    run(&t, &mut it, 0, rec(0, 0, 0, 0));
    assert_eq!(it.flags & flag::ETHEREAL, 0);
    assert_eq!(it.stats.base(stat::MINDAMAGE, 0), 10);
    assert_eq!(it.stats.item_list(9, 0), 0);
}

#[test]
fn func12_36_rolled_layer() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    let mut p36 = prop1(36, 97);
    p36.slots[0].val = 4;
    t.properties = vec![prop1(12, 97), p36];
    for seed in 0..10 {
        let mut s = Seed::init_low(seed);
        let layer = s.roll(6) as u16 + 3;
        let mut it = item(i, seed);
        run(&t, &mut it, 0, rec(0, 2, 3, 8));
        assert_eq!(it.item_seed, s);
        assert_eq!(
            it.stats.lists[&ListKey::ITEM],
            BTreeMap::from([((97, layer), 2)])
        );
        let mut it = item(i, seed);
        run(&t, &mut it, 0, rec(1, 2, 3, 8));
        assert_eq!(it.item_seed, s);
        assert_eq!(
            it.stats.lists[&ListKey::ITEM],
            BTreeMap::from([((97, layer), 4)])
        );
    }
}

/// Superior applies its row once in mode 1 (reset functions reset); an
/// affix applies in mode 0 (no reset for function 1).
#[test]
fn superior_mode1_affix_mode0() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.maxac = 10;
    let i = push_item(&mut t, r);
    t.properties = vec![prop1(1, stat::ARMORCLASS)];
    t.qualityitems = vec![QualityRec {
        mods: [rec(0, 0, 5, 5), PropRec::NONE],
        armor: 1,
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.quality = q::SUPERIOR;
    it.stats.set_base(stat::ARMORCLASS, 0, 3);
    assert!(superior(&t, &mut it, &ItemRequest::default()));
    assert_eq!(it.file_index, 0);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 11);
    assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 5, "once");
    t.magic = vec![AffixRec {
        mods: [rec(0, 0, 5, 5), PropRec::NONE, PropRec::NONE],
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.stats.set_base(stat::ARMORCLASS, 0, 3);
    apply_affix(&t, &mut it, 1);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 3);
    assert_eq!(it.stats.item_list(stat::ARMORCLASS, 0), 5);
}

/// Mode 3 runs the 12 records in order (seen through the draw order).
#[test]
fn unique_records_in_order() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = (0..12).map(|k| prop1(1, 100 + k)).collect();
    let mut props = [PropRec::NONE; 12];
    for (k, p) in props.iter_mut().enumerate() {
        *p = rec(k as i32, 0, 1, 50);
    }
    t.uniques = vec![UniqueRec {
        props,
        ..Default::default()
    }];
    let mut it = item(i, 77);
    it.file_index = 0;
    apply_unique(&t, &mut it);
    let mut s = Seed::init_low(77);
    let want: BTreeMap<(u16, u16), i32> = (0..12)
        .map(|k| ((100 + k, 0), s.roll(50) as i32 + 1))
        .collect();
    assert_eq!(it.stats.lists[&ListKey::ITEM], want);
    assert_eq!(it.item_seed, s);
}

/// Gem fillers: the gems row by `gemoffset`, block by the socketed item's
/// apply type (0 weapon, 1 helm, 2 shield), into the filler's own list.
#[test]
fn gem_filler_blocks() {
    let mut t = tables();
    t.properties = (0..10).map(|k| prop1(1, 100 + k)).collect();
    let blk = |k: i32| [rec(k, 0, 1, 1), PropRec::NONE, PropRec::NONE];
    t.gems = vec![
        GemRec {
            mods: [blk(9), blk(9), blk(9)],
        },
        GemRec {
            mods: [blk(0), blk(1), blk(2)],
        },
    ];
    let mut g = item_rec(ty::GEM, b"gcv ");
    g.gemoffset = 1;
    let gem = push_item(&mut t, g);
    for (apply, stat_id) in [(0u8, 100u16), (1, 101), (2, 102)] {
        let mut f = item(gem, 1);
        apply_socket_filler(&t, &mut f, apply);
        assert_eq!(
            f.stats.lists,
            BTreeMap::from([(ListKey::ITEM, BTreeMap::from([((stat_id, 0), 1)]))]),
            "apply type {apply}"
        );
    }
}
