//! test vectors, edge cases and the TSV check.

use super::*;
use crate::items::props::{
    apply_craft_list, apply_property, apply_socket_filler, roll_value, runeword_match, set_bonuses,
    PropCtx, FUNCS,
};
use crate::items::tables::{GemRec, RuneRec, SetItemRec, SetRec, SkillRec};
use crate::items::{flag, q, stat};

const CHARGED: u16 = 204;

fn run(t: &ItemTables, it: &mut Item<FakeStats>, r: PropRec) {
    apply_property(t, it, &mut PropCtx::item(0), &r);
}

#[test]
fn value_roll() {
    let mut s = Seed::init_low(1);
    assert_eq!(roll_value(&mut s, 5, 5), 5);
    assert_eq!(s, Seed::init_low(1));
    let lo = Seed::init_low(1).step();
    assert_eq!(roll_value(&mut s, 15, 5), 5 + (lo % 11) as i32);
}

fn charge_tables() -> (ItemTables, usize) {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::STAF, b"sst "));
    t.properties = vec![prop1(19, CHARGED)];
    t.skills = vec![SkillRec::default(); 4];
    (t, i)
}

#[test]
fn func19_charges_vectors() {
    let (t, i) = charge_tables();
    for (min, level, c, value) in [
        (0, 1, 5, 1283),
        (-3, 10, 6, 1541),
        (-12, 20, 42, 10771),
        (300, 1, 255, 65524),
    ] {
        let mut it = item(i, 4242);
        run(&t, &mut it, rec(0, 3, min, level));
        let layer = (3 << 6) + (level as u16 & 0x3F);
        assert_eq!(it.stats.item_list(CHARGED, layer), value, "min {min}");
        let mut s = Seed::init_low(4242);
        s.roll(c - c / 8);
        assert_eq!(it.item_seed, s);
    }
}

/// A `param` outside the skills table charges skill 0.
#[test]
fn func19_skill_outside_the_table_is_skill_0() {
    let (t, i) = charge_tables();
    let mut it = item(i, 4242);
    run(&t, &mut it, rec(0, 99, 0, 1));
    // c = 5, layer (0 << 6) + 1.
    assert_eq!(it.stats.item_list(CHARGED, 1), 1283);
}

#[test]
fn func18_by_time_vectors() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(18, 268)];
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 1, -50, 50));
    assert_eq!(it.stats.item_list(268, 0), 1_254_201);
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 5, -300, 900));
    assert_eq!(it.stats.item_list(268, 0), 4_190_211);
}

/// Functions 18 and 19 write with the plain list set:
/// no `valshift` (the reference vectors stay unshifted).
#[test]
fn func18_func19_set_without_valshift() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(18, 268), prop1(19, CHARGED)];
    t.skills = vec![SkillRec::default(); 4];
    t.valshift[268] = 8;
    t.valshift[usize::from(CHARGED)] = 8;
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 1, -50, 50));
    assert_eq!(it.stats.item_list(268, 0), 1_254_201);
    let mut it = item(i, 4242);
    run(&t, &mut it, rec(1, 3, 0, 1));
    assert_eq!(it.stats.item_list(CHARGED, (3 << 6) + 1), 1283);
}

/// With a foreign owner (owner = the player, I = the set
/// item): I is still an item, so the small-bonus rule applies and the
/// base is reset.
#[test]
fn func7_set_bonus_on_item() {
    let mut t = tables();
    let i = weapon(&mut t, 1, 2);
    t.properties = vec![prop1(7, stat::MAXDAMAGE_PERCENT)];
    let mut partial = [PropRec::NONE; 8];
    partial[0] = rec(0, 0, 10, 10);
    t.sets = vec![SetRec {
        count: 3,
        version: 0,
        partial,
        full: [PropRec::NONE; 8],
    }];
    t.setitems = vec![SetItemRec::default()];
    let mut it = item(i, 1);
    it.file_index = 0;
    let key = ListKey {
        state: 165,
        flags: 0,
    };
    let mut owner = FakeStats::default();
    set_bonuses(&t, &mut it, 0b11, &mut owner, key);
    assert_eq!(owner.lists[&key].get(&(stat::MAXDAMAGE, 0)), Some(&1));
    assert_eq!(owner.lists[&key].get(&(stat::MAXDAMAGE_PERCENT, 0)), None);
    assert_eq!(it.stats.base(stat::MAXDAMAGE, 0), 2, "base reset");
}

#[test]
fn func10_layer() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(10, 83)];
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 7, 2, 2));
    assert_eq!(it.stats.item_list(83, 17), 2);
}

fn weapon(t: &mut ItemTables, mindam: u8, maxdam: u8) -> usize {
    let mut r = item_rec(AXE, b"axe ");
    r.mindam = mindam;
    r.maxdam = maxdam;
    push_item(t, r)
}

#[test]
fn func5_func6_floors() {
    let mut t = tables();
    let i = weapon(&mut t, 3, 3);
    t.properties = vec![prop1(5, stat::MINDAMAGE), prop1(6, stat::MAXDAMAGE)];
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 0, -5, -5));
    run(&t, &mut it, rec(1, 0, -5, -5));
    assert_eq!(it.stats.item_list(stat::MINDAMAGE, 0), -2);
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE, 0), -3);
    // A one-handed weapon: no two-handed or throw damage.
    assert!(!it.stats.lists[&ListKey::ITEM].contains_key(&(stat::SECONDARY_MINDAMAGE, 0)));
    assert!(!it.stats.lists[&ListKey::ITEM].contains_key(&(stat::THROW_MINDAMAGE, 0)));
}

/// Edge case 1: later slots reuse slot 0's value (one roll for a min/max
/// pair).
#[test]
fn slot_zero_value_shared() {
    let mut t = tables();
    let i = weapon(&mut t, 3, 9);
    let mut p = prop1(5, stat::MINDAMAGE);
    p.slots[1] = crate::items::tables::PropSlot {
        func: 6,
        stat: stat::MAXDAMAGE,
        set: 0,
        val: 0,
    };
    t.properties = vec![p];
    let mut it = item(i, 31);
    run(&t, &mut it, rec(0, 0, 1, 20));
    let mut s = Seed::init_low(31);
    let v = s.roll(20) as i32 + 1;
    assert_eq!(it.item_seed, s, "one draw");
    assert_eq!(it.stats.item_list(stat::MINDAMAGE, 0), v);
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE, 0), v);
}

/// Edge cases 2 and 5: a 0 value adds nothing; function 0 in slot 0 ends.
#[test]
fn zero_value_and_function_zero() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, 0), prop1(0, 0)];
    t.properties[1].slots[1] = crate::items::tables::PropSlot {
        func: 1,
        stat: 0,
        set: 0,
        val: 0,
    };
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 0, 0, 0));
    run(&t, &mut it, rec(1, 0, 3, 3));
    assert!(it.stats.lists.is_empty());
    // Out-of-range and negative codes do nothing.
    run(&t, &mut it, rec(9, 0, 3, 3));
    run(&t, &mut it, rec(-1, 0, 3, 3));
    assert!(it.stats.lists.is_empty());
}

/// Edge case 4: enhanced damage rounding to ≤ 0 adds +1 max damage.
#[test]
fn func7_small_bonus() {
    let mut t = tables();
    let i = weapon(&mut t, 1, 2);
    t.properties = vec![prop1(7, stat::MAXDAMAGE_PERCENT)];
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 0, 10, 10));
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE, 0), 1);
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE_PERCENT, 0), 0);
    assert_eq!(it.stats.base(stat::MAXDAMAGE, 0), 2, "base reset");
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 0, 50, 50));
    assert_eq!(it.stats.item_list(stat::MAXDAMAGE_PERCENT, 0), 50);
    assert_eq!(it.stats.item_list(stat::MINDAMAGE_PERCENT, 0), 50);
}

/// Edge case 3: function 14 ignores the generation quality caps.
#[test]
fn func14_no_quality_cap() {
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.invwidth = 2;
    r.invheight = 3;
    r.gemsockets = 6;
    let i = push_item(&mut t, r);
    t.itemtypes[ty::HELM as usize].maxsock40 = 6;
    t.properties = vec![prop1(14, stat::NUMSOCKETS)];
    let mut it = item(i, 1);
    it.quality = q::RARE;
    run(&t, &mut it, rec(0, 0, 5, 5));
    assert_eq!(it.stats.base(stat::NUMSOCKETS, 0), 5);
    assert_ne!(it.flags & flag::SOCKETED, 0);
}

#[test]
fn poison_count_and_valshift() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.valshift[57] = 8;
    t.properties = vec![prop1(1, stat::POISONMAXDAM), prop1(1, 57)];
    let mut it = item(i, 1);
    run(&t, &mut it, rec(0, 0, 4, 4));
    run(&t, &mut it, rec(0, 0, 4, 4));
    run(&t, &mut it, rec(1, 0, 2, 2));
    assert_eq!(it.stats.item_list(stat::POISONMAXDAM, 0), 8);
    assert_eq!(it.stats.item_list(stat::POISON_COUNT, 0), 2);
    assert_eq!(it.stats.item_list(57, 0), 512);
}

/// Vector: mask 0b1011 on a 6-item set → c 3, n 3, records 1–4.
#[test]
fn set_bonus_records() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = (0..16).map(|k| prop1(1, 100 + k)).collect();
    let mut partial = [PropRec::NONE; 8];
    for (k, p) in partial.iter_mut().enumerate() {
        *p = rec(k as i32, 0, 1, 1);
    }
    let mut full = [PropRec::NONE; 8];
    full[0] = rec(15, 0, 1, 1);
    t.sets = vec![SetRec {
        count: 6,
        version: 0,
        partial,
        full,
    }];
    t.setitems = vec![SetItemRec {
        set: 0,
        props: [PropRec::NONE; 9],
        aprops: [PropRec::NONE; 10],
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.quality = q::SET;
    it.file_index = 0;
    let mut owner = FakeStats::default();
    let key = ListKey {
        state: 1,
        flags: 0x40,
    };
    set_bonuses(&t, &mut it, 0b1011, &mut owner, key);
    let got: Vec<u16> = owner.lists[&key].keys().map(|&(s, _)| s).collect();
    assert_eq!(got, vec![100, 101, 102, 103]);
    assert!(it.stats.lists.is_empty());
    let mut owner = FakeStats::default();
    set_bonuses(&t, &mut it, 0b111111, &mut owner, key);
    // n = min(6, 5) = 5: 2n − 2 = 8 partial records, then the full one.
    assert_eq!(owner.lists[&key].len(), 9);
    assert_eq!(crate::items::props::set_mask_count(64), 0);
}

#[test]
fn set_item_partial_states() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(RING, b"rin "));
    t.properties = vec![prop1(1, 7)];
    let mut aprops = [PropRec::NONE; 10];
    aprops[3] = rec(0, 0, 2, 2);
    t.setitems = vec![SetItemRec {
        add_func: 2,
        props: [
            rec(0, 0, 1, 1),
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
            PropRec::NONE,
        ],
        aprops,
        ..Default::default()
    }];
    let mut it = item(i, 1);
    it.file_index = 0;
    crate::items::props::apply_set_item(&t, &mut it);
    assert_eq!(it.stats.item_list(7, 0), 1);
    let k = ListKey {
        state: 166,
        flags: 0x2040,
    };
    assert_eq!(it.stats.list_get(k, 7, 0), 2);
}

#[test]
fn gem_filler_and_runeword() {
    let mut t = tables();
    let mut sword = item_rec(AXE, b"axe ");
    sword.hasinv = 1;
    let s = push_item(&mut t, sword);
    let mut g = item_rec(ty::RUNE, b"r01 ");
    g.gemoffset = 0;
    let r1 = push_item(&mut t, g.clone());
    g.code = *b"r02 ";
    g.gemoffset = 1;
    let r2 = push_item(&mut t, g);
    t.properties = vec![prop1(1, 50), prop1(1, 51), prop1(1, 52)];
    let block = |code| [rec(code, 0, 3, 3), PropRec::NONE, PropRec::NONE];
    t.gems = vec![
        GemRec {
            mods: [block(0), block(1), block(1)],
        },
        GemRec {
            mods: [block(1), block(0), block(0)],
        },
    ];
    let mut f = item(r1, 1);
    apply_socket_filler(&t, &mut f, 0);
    assert_eq!(f.stats.item_list(50, 0), 3);
    let mut f = item(r2, 1);
    apply_socket_filler(&t, &mut f, 2);
    assert_eq!(f.stats.item_list(50, 0), 3);

    let mut props = [PropRec::NONE; 7];
    props[0] = rec(2, 0, 9, 9);
    t.runes = vec![
        RuneRec {
            complete: 1,
            itype: [ty::ARMO as i16, 0, 0, 0, 0, 0],
            runes: [r1 as i32, r2 as i32, 0, 0, 0, 0],
            props,
            ..Default::default()
        },
        RuneRec {
            complete: 1,
            itype: [ty::WEAP as i16, 0, 0, 0, 0, 0],
            runes: [r1 as i32, r2 as i32, 0, 0, 0, 0],
            props,
            ..Default::default()
        },
    ];
    let mut it = item(s, 1);
    it.quality = q::NORMAL;
    it.stats.set_base(stat::NUMSOCKETS, 0, 2);
    assert_eq!(runeword_match(&t, &it, &[r1, r2]), Some(1));
    assert_eq!(runeword_match(&t, &it, &[r2, r1]), None);
    assert_eq!(runeword_match(&t, &it, &[r1]), None);
    // I = the filler (a rune), O = the socketed item's stats.
    let mut filler = item(r2, 1);
    assert!(crate::items::props::activate_runeword(
        &t,
        &mut filler,
        &mut it.stats,
        1,
        false
    ));
    let k = ListKey {
        state: 171,
        flags: 0x40,
    };
    assert_eq!(it.stats.list_get(k, 52, 0), 9);
    assert_eq!(
        filler.stats.list_get(k, 52, 0),
        0,
        "the filler's lists stay"
    );
    assert!(!crate::items::props::activate_runeword(
        &t,
        &mut filler,
        &mut it.stats,
        1,
        false
    ));
    it.quality = q::MAGIC;
    assert_eq!(runeword_match(&t, &it, &[r1, r2]), None);
}

/// A ranged runeword property rolls on the filler's item seed
/// (I), not the socketed item's (O), and lands in O's state-171 list.
#[test]
fn a_runeword_roll_draws_on_the_filler_seed() {
    let mut t = tables();
    let s = push_item(&mut t, item_rec(AXE, b"axe "));
    let r = push_item(&mut t, item_rec(ty::RUNE, b"r01 "));
    t.properties = vec![prop1(1, 52)];
    let mut props = [PropRec::NONE; 7];
    props[0] = rec(0, 0, 1, 100);
    t.runes = vec![RuneRec {
        complete: 1,
        props,
        ..Default::default()
    }];
    let (mut filler, mut socketed) = (item(r, 7), item(s, 9));
    let (fseed, sseed) = (filler.item_seed, socketed.item_seed);
    let mut want = fseed;
    let v = crate::items::props::roll_value(&mut want, 1, 100);
    assert!(crate::items::props::activate_runeword(
        &t,
        &mut filler,
        &mut socketed.stats,
        0,
        false
    ));
    let k = ListKey {
        state: 171,
        flags: 0x40,
    };
    assert_eq!(socketed.stats.list_get(k, 52, 0), v);
    assert_eq!(filler.item_seed, want, "one draw on the filler's seed");
    assert_eq!(socketed.item_seed, sseed, "none on the socketed item's");
}

#[test]
fn craft_list_reapplies_ethereal() {
    let mut t = tables();
    let i = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    t.properties = vec![prop1(1, 7)];
    let mut it = item(i, 1);
    it.flags = flag::ETHEREAL;
    it.stats.set_base(stat::ARMORCLASS, 0, 10);
    apply_craft_list(&t, &mut it, &[rec(0, 0, 5, 5)]);
    assert_eq!(it.stats.item_list(7, 0), 5);
    assert_eq!(it.stats.base(stat::ARMORCLASS, 0), 15);
}

/// The function table equals `property-functions.tsv`.
const TSV: &str = include_str!("../../../../../tables/items/property-functions.tsv");

/// Differences between the TSV text and the code table, as
/// `(line, column, tsv, code)` (line 1 = header).
fn tsv_diff(tsv: &str) -> Vec<(usize, &'static str, String, String)> {
    const COLS: [&str; 8] = [
        "func",
        "address",
        "value",
        "draws",
        "reset_base",
        "writes",
        "layer",
        "returns",
    ];
    let mut out = Vec::new();
    let mut lines = tsv.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    if header != COLS {
        out.push((1, "header", header.join("\t"), COLS.join("\t")));
    }
    let rows: Vec<&str> = lines.collect();
    for k in 0..rows.len().max(FUNCS.len()) {
        let cells: Vec<&str> = rows.get(k).map_or(vec![], |l| l.split('\t').collect());
        let code = FUNCS.get(k).map(|f| {
            [
                f.func.to_string(),
                f.address.into(),
                f.value.into(),
                f.draws.into(),
                f.reset_base.into(),
                f.writes.into(),
                f.layer.into(),
                f.returns.into(),
            ]
        });
        for (c, name) in COLS.iter().enumerate() {
            let a = cells.get(c).copied().unwrap_or("").to_string();
            let b = code.as_ref().map_or(String::new(), |r| r[c].clone());
            if a != b {
                out.push((k + 2, *name, a, b));
            }
        }
    }
    out
}

#[test]
fn funcs_match_tsv() {
    assert_eq!(tsv_diff(TSV), vec![]);
}

/// A changed TSV cell is reported exactly.
#[test]
fn tsv_check_catches_perturbation() {
    let changed = TSV.replacen("param%3+param/3*8", "param%3+param/3*9", 1);
    assert_eq!(
        tsv_diff(&changed),
        vec![(
            11,
            "layer",
            "param%3+param/3*9".to_string(),
            "param%3+param/3*8".to_string()
        )]
    );
    let dropped: String = TSV
        .lines()
        .filter(|l| !l.starts_with("36\t"))
        .map(|l| format!("{l}\n"))
        .collect();
    let d = tsv_diff(&dropped);
    assert_eq!(d.len(), 8, "every column of the missing row: {d:?}");
    assert!(d.iter().all(|x| x.0 == 26));
}
