//! Unit tests for the save appearance bytes in `d2s::appearance`.
//!
//! They use synthetic item tables built in memory and shaped like the
//! stock tables (the same token positions for the items they name); no
//! game file is read.

use super::*;
use crate::d2s::Header;

/// Synthetic item types (itemtypes indices below 80).
mod sty {
    pub const AXE: i32 = 60;
    pub const JAV: i32 = 61;
    pub const SHIE: i32 = 62;
    pub const RING: i32 = 63;
    pub const TPOT: i32 = 64;
    pub const STAF: i32 = 65;
    pub const XBOW: i32 = 66;
}

/// Is-a: every type is itself; weapons are `weap`; `tors`, `helm`,
/// `shld` and `circ` are `armo`; shields are `shld`; circlets are `helm`.
fn types() -> IsA {
    use sty::*;
    IsA::from_fn(80, |i, j| {
        let (i, j) = (i as i32, j as i32);
        if i == j {
            return true;
        }
        let parents: &[i32] = match i {
            AXE | JAV | TPOT | STAF | XBOW => &[ty::WEAP],
            SHIE => &[ty::SHLD, ty::ARMO],
            x if x == ty::SHLD || x == ty::TORS || x == ty::HELM => &[ty::ARMO],
            x if x == ty::CIRC => &[ty::HELM, ty::ARMO],
            _ => &[],
        };
        parents.contains(&j)
    })
}

fn code(s: &str) -> [u8; 4] {
    let mut c = [b' '; 4];
    c[..s.len()].copy_from_slice(s.as_bytes());
    c
}

fn row(c: &str, t: i32, component: u8, wclass: &str) -> ItemGfx {
    ItemGfx {
        code: code(c),
        item_type: t,
        component,
        wclass: code(wclass),
        ..ItemGfx::default()
    }
}

/// Weapons, armor and misc in record order, laid out so that with the
/// fixture reference table they land on the stock token positions:
/// `hax` 4, `wnd` 9, `clb` 12, `ssd` 17, `jav` 27, `bst` 37, `ktr` 45
/// (weapons 4–56), potions 125–134, `cap` 57, body armours from 64, `buc`
/// 79.
fn rows() -> Vec<ItemGfx> {
    let named: &[(usize, &str, i32, &str)] = &[
        (4, "hax", sty::AXE, "1hs"),
        (9, "wnd", sty::AXE, "1hs"),
        (12, "clb", sty::AXE, "1hs"),
        (17, "ssd", sty::AXE, "1hs"),
        (27, "jav", sty::JAV, "1ht"),
        (37, "bst", sty::STAF, "stf"),
        (45, "ktr", sty::AXE, "ht1"),
        (50, "lxb", sty::XBOW, "xbw"),
    ];
    let mut v = Vec::new();
    for slot in 4..=56 {
        v.push(match named.iter().find(|n| n.0 == slot) {
            Some(&(_, c, t, w)) => row(c, t, 5, w),
            None => row(&format!("w{slot:02}"), sty::AXE, 5, "1hs"),
        });
    }
    // `sst` draws as `bst`.
    v.push(ItemGfx {
        alternategfx: code("bst"),
        ..row("sst", sty::STAF, 5, "stf")
    });
    for k in 0..10 {
        v.push(row(&format!("p{k}"), sty::TPOT, 5, "1ht"));
    }
    // armor: 7 helms, 15 body armours (`qui` among them), shields.
    v.push(row("cap", ty::HELM, 0, ""));
    for k in 1..7 {
        v.push(row(&format!("h{k}"), ty::HELM, 0, ""));
    }
    v.push(ItemGfx {
        arm: [0, 0, 0, 0, 1, 1],
        ..row("qui", ty::TORS, 1, "")
    });
    for k in 1..15 {
        v.push(row(&format!("t{k:02}"), ty::TORS, 1, ""));
    }
    v.push(row("buc", sty::SHIE, 7, ""));
    v.push(ItemGfx {
        alternategfx: code("lit"),
        ..row("ci0", ty::CIRC, 0, "")
    });
    // misc: a ring and a gem, never eligible.
    v.push(row("rin", sty::RING, 0, ""));
    v.push(ItemGfx {
        gemoffset: 2,
        ..row("gsr", ty::GEM, 0, "")
    });
    v
}

fn tables_with(
    colours: ColourTables,
    mut items: Vec<ItemGfx>,
    f: impl Fn(&mut ItemGfx),
) -> AppearanceTables {
    for r in &mut items {
        f(r);
    }
    AppearanceTables::new(
        items,
        vec![code("lit"), code("med"), code("hvy")],
        types(),
        colours,
        &fixture_reference(),
    )
}

/// The reference slots the synthetic `rows()` are shaped for: slots
/// 57–124 `weap`, nothing else reserved. With these rows it gives the
/// stock token positions; the game's table (`ReferenceSlots::game`) is
/// checked on the player's own item tables by `d2-server`'s
/// `character_save` test `token_positions_on_the_users_install`.
fn fixture_reference() -> ReferenceSlots {
    let mut s = ReferenceSlots::none();
    for slot in &mut s.0[57..=124] {
        slot.weap = true;
    }
    s
}

fn tables() -> AppearanceTables {
    tables_with(ColourTables::default(), rows(), |_| {})
}

fn rec(t: &AppearanceTables, c: &str) -> usize {
    t.items.iter().position(|r| r.code == code(c)).unwrap()
}

/// An equipped item of record `c` at body location `loc`, normal quality.
fn item(t: &AppearanceTables, c: &str, loc: u8) -> EquippedItem {
    EquippedItem {
        record: rec(t, c),
        mode: MODE_EQUIPPED,
        body_loc: loc,
        quality: 2,
        file_index: -1,
        ..EquippedItem::default()
    }
}

/// The items, each also in the body grid at its body location.
fn equip(items: Vec<EquippedItem>, in_use: Option<usize>) -> Equipment {
    let mut grid = [None; BODY_SLOTS];
    for (i, it) in items.iter().enumerate() {
        if usize::from(it.body_loc) < BODY_SLOTS && it.mode == MODE_EQUIPPED {
            grid[usize::from(it.body_loc)] = Some(i);
        }
    }
    Equipment {
        items,
        body_grid: grid,
        weapon_in_use: in_use,
        ..Equipment::default()
    }
}

fn run(eq: &Equipment, t: &AppearanceTables) -> ([u8; 16], [u8; 16]) {
    let (mut c, mut k) = ([0xFF; 16], [0xFF; 16]);
    fill(eq, t, &mut c, &mut k);
    (c, k)
}

#[test]
fn table_starts_with_armtype_and_skips_duplicates_and_ineligible_rows() {
    let m = types();
    let rows = vec![
        row("aaa", sty::AXE, 5, ""),
        // Same gfx as `aaa` through `alternategfx`: skipped.
        ItemGfx {
            alternategfx: code("aaa"),
            ..row("bbb", sty::AXE, 5, "")
        },
        row("rin", sty::RING, 0, ""),
        row("ci0", ty::CIRC, 0, ""),
        row("cap", ty::HELM, 0, ""),
        // `lit ` is already entry 1.
        row("lit", ty::TORS, 1, ""),
    ];
    let t = TokenTable::build(&rows, &m, &ReferenceSlots::none());
    assert_eq!(t.entry(0), Some(([0; 4], 0)));
    assert_eq!(t.entry(1), Some((code("lit"), 3)));
    assert_eq!(t.entry(2), Some((code("med"), 3)));
    assert_eq!(t.entry(3), Some((code("hvy"), 3)));
    assert_eq!(t.entry(4), Some((code("aaa"), sty::AXE)));
    assert_eq!(t.entry(5), Some((code("cap"), ty::HELM)));
    assert_eq!(t.entry(6), Some(([0; 4], 0)));
    assert_eq!(t.count(), 6);
}

#[test]
fn reserved_slots_push_entries_above_n_where_the_duplicate_test_misses_them() {
    let m = types();
    let mut r = ReferenceSlots::none();
    // Slot 4 is a `weap` slot, slot 5 an `armo` slot.
    r.0[4].weap = true;
    r.0[5].armo = true;
    let rows = vec![
        // A weapon skips 4, lands at 5 (above n = 4).
        row("ktr", sty::AXE, 5, "ht1"),
        // A helm takes n = 4.
        row("cap", ty::HELM, 0, ""),
        // Slot 5 is an `armo` slot (and occupied): 6, above n = 5.
        row("skp", ty::HELM, 0, ""),
        // `ktr` at 5 is not below n = 5, so the duplicate test misses it:
        // placed again (slot 5 `armo`, 6 occupied → 7).
        ItemGfx {
            alternategfx: code("ktr"),
            ..row("kt2", ty::TORS, 1, "")
        },
    ];
    let t = TokenTable::build(&rows, &m, &r);
    assert_eq!(t.entry(4).unwrap().0, code("cap"));
    assert_eq!(t.entry(5).unwrap().0, code("ktr"));
    assert_eq!(t.entry(6).unwrap().0, code("skp"));
    assert_eq!(t.entry(7).unwrap().0, code("ktr"));
    assert_eq!(t.count(), 5);
    // A code held twice: the lookup finds the lower entry.
    assert_eq!(t.lookup(code("ktr"), code("ktr")), 5);
}

#[test]
fn a_search_past_the_table_falls_back_to_n() {
    let m = types();
    let mut r = ReferenceSlots::none();
    for s in &mut r.0 {
        s.weap = true;
    }
    let rows = vec![row("aaa", sty::AXE, 5, ""), row("bbb", sty::AXE, 5, "")];
    let t = TokenTable::build(&rows, &m, &r);
    assert_eq!(t.entry(4).unwrap().0, code("aaa"));
    assert_eq!(t.entry(5).unwrap().0, code("bbb"));
    assert_eq!(t.count(), 6);
}

#[test]
fn the_fixture_reference_gives_the_anchor_positions() {
    let t = tables();
    let at = |c: &str| t.tokens.lookup(code(c), code(c));
    assert_eq!(at("hax"), 4);
    assert_eq!(at("wnd"), 9);
    assert_eq!(at("clb"), 12);
    assert_eq!(at("ssd"), 17);
    assert_eq!(at("jav"), 27);
    assert_eq!(at("bst"), 37);
    assert_eq!(at("ktr"), 45);
    assert_eq!(at("cap"), 57);
    assert_eq!(at("qui"), 64);
    assert_eq!(at("buc"), 79);
    assert_eq!(t.tokens.entry(125).unwrap().0, code("p0"));
    assert_eq!(t.tokens.entry(134).unwrap().0, code("p9"));
    assert_eq!(t.tokens.entry(0).unwrap().0, [0; 4]);
}

#[test]
fn lookup_takes_the_first_entry_holding_either_code() {
    let t = tables();
    // a = `alternategfx`, b = `code`.
    assert_eq!(t.tokens.lookup(code("bst"), code("sst")), 37);
    assert_eq!(t.tokens.lookup(code("zzz"), code("jav")), 27);
    assert_eq!(t.tokens.lookup(code("jav"), code("hax")), 4);
    assert_eq!(t.tokens.lookup(code("zzz"), code("yyy")), 0);
}

#[test]
fn vector_fresh_amazon() {
    let t = tables();
    let eq = equip(
        vec![
            item(&t, "jav", body::RIGHT_HAND),
            item(&t, "buc", body::LEFT_HAND),
        ],
        Some(0),
    );
    let (c, k) = run(&eq, &t);
    let mut want = [0xFF; 16];
    want[part::RH] = 0x1B;
    want[part::SH] = 0x4F;
    assert_eq!(c, want);
    assert_eq!(k, [0xFF; 16]);
}

#[test]
fn vector_class_start_weapons() {
    let t = tables();
    let one = |w: &str, shield: bool| {
        let mut items = vec![item(&t, w, body::RIGHT_HAND)];
        if shield {
            items.push(item(&t, "buc", body::LEFT_HAND));
        }
        run(&equip(items, Some(0)), &t).0
    };
    assert_eq!(one("sst", false)[part::RH], 0x25);
    assert_eq!(one("wnd", false)[part::RH], 0x09);
    for (w, v) in [("ssd", 0x11), ("hax", 0x04), ("clb", 0x0C), ("ktr", 0x2D)] {
        let c = one(w, true);
        assert_eq!((c[part::RH], c[part::SH]), (v, 0x4F), "{w}");
    }
}

#[test]
fn vector_broken_axe_not_in_use_draws_as_the_left_hand() {
    let t = tables();
    let eq = equip(
        vec![
            item(&t, "hax", body::RIGHT_HAND),
            item(&t, "buc", body::LEFT_HAND),
        ],
        None,
    );
    let (c, _) = run(&eq, &t);
    assert_eq!(c[part::LH], 0x04);
    assert_eq!(c[part::SH], 0x4F);
    assert_eq!(c[part::RH], 0xFF);
}

#[test]
fn vector_quilted_armour_parts() {
    let t = tables();
    let eq = equip(vec![item(&t, "qui", body::TORSO)], None);
    let (c, k) = run(&eq, &t);
    let mut want = [0xFF; 16];
    for p in [part::TR, part::LG, part::RA, part::LA] {
        want[p] = 0x01;
    }
    want[part::S1] = 0x02;
    want[part::S2] = 0x02;
    assert_eq!(c, want, "parts 0, 5, 6, 7 and 10–15 untouched");
    assert_eq!(k, [0xFF; 16]);
}

#[test]
fn body_armour_of_class_id_0_or_without_a_token_clears_its_parts() {
    // Record 0 never draws as body armour.
    let t = tables_with(ColourTables::default(), rows(), |r| {
        if r.code == code("hax") {
            r.arm = [0; 6];
        }
    });
    let mut it = item(&t, "hax", body::TORSO);
    it.record = 0;
    let (mut c, mut k) = ([7; 16], [7; 16]);
    fill(&equip(vec![it], None), &t, &mut c, &mut k);
    for p in [part::TR, part::LG, part::RA, part::LA, part::S1, part::S2] {
        assert_eq!((c[p], k[p]), (0xFF, 0xFF));
    }
    assert_eq!(c[part::HD], 7);
    // An `armtype` row past the table: no token.
    let t = tables_with(ColourTables::default(), rows(), |r| {
        if r.code == code("qui") {
            r.arm = [9, 0, 0, 0, 0, 0];
        }
    });
    let (c, _) = run(&equip(vec![item(&t, "qui", body::TORSO)], None), &t);
    assert_eq!(c[part::TR], 0xFF);
    assert_eq!(c[part::LG], 0x01);
}

fn dblu() -> ColourTables {
    // Combined magic array: 3 suffixes, 3 prefixes, 1 automagic.
    ColourTables {
        affix: vec![-1, -1, 9, 5, -1, -1, 2],
        set: vec![-1, 4],
        unique: vec![3, -1],
        gem: vec![-1, -1, 11],
        states: Vec::new(),
    }
}

#[test]
fn vector_magic_buckler_with_a_transform_colour() {
    let t = tables_with(dblu(), rows(), |r| r.transform = 8);
    let mut b = item(&t, "buc", body::LEFT_HAND);
    b.quality = quality::MAGIC;
    // Prefix id 4 → combined index 3 → colour 5 (`dblu`).
    b.prefix = [4, 0, 0];
    let (c, k) = run(&equip(vec![b], None), &t);
    assert_eq!(c[part::SH], 0x4F);
    assert_eq!(k[part::SH], 0x06);
}

#[test]
fn magic_and_rare_take_suffixes_first_then_prefixes_then_automagic() {
    let t = tables_with(dblu(), rows(), |r| r.transform = 1);
    let colour = |q: u8, prefix: [u16; 3], suffix: [u16; 3], auto: u16| {
        let mut b = item(&t, "buc", body::LEFT_HAND);
        b.quality = q;
        b.prefix = prefix;
        b.suffix = suffix;
        b.auto_affix = auto;
        run(&equip(vec![b], None), &t).1[part::SH]
    };
    // Suffix 3 (colour 9) before prefix 4 (colour 5): 32 + 9 + 1.
    assert_eq!(colour(quality::RARE, [4, 0, 0], [1, 3, 0], 0), 42);
    assert_eq!(colour(quality::MAGIC, [5, 4, 0], [1, 2, 0], 0), 38);
    // No affix with a colour → the automagic one (id 7: colour 2).
    assert_eq!(colour(quality::MAGIC, [5, 0, 0], [1, 0, 0], 7), 35);
    // None at all → 0xFF.
    assert_eq!(colour(quality::MAGIC, [5, 0, 0], [1, 0, 0], 0), 0xFF);
}

#[test]
fn set_unique_and_gem_colours() {
    let t = tables_with(dblu(), rows(), |r| r.transform = 2);
    let colour = |f: &dyn Fn(&mut EquippedItem)| {
        let mut b = item(&t, "buc", body::LEFT_HAND);
        f(&mut b);
        run(&equip(vec![b], None), &t).1[part::SH]
    };
    let set = |row: i32| {
        move |b: &mut EquippedItem| {
            b.quality = quality::SET;
            b.file_index = row;
        }
    };
    assert_eq!(colour(&set(1)), 64 + 4 + 1);
    assert_eq!(colour(&set(0)), 0xFF, "chrtransform < 0");
    assert_eq!(
        colour(&|b| {
            b.quality = quality::UNIQUE;
            b.file_index = 0;
        }),
        64 + 3 + 1
    );
    // Gem-socketed normal item: hasinv, socket limit, flag 0x800 and a
    // `gem` first child (gems row 2: transform 11).
    let t = tables_with(dblu(), rows(), |r| {
        r.transform = 2;
        r.hasinv = 1;
    });
    let gem = rec(&t, "gsr");
    let socketed = |flags: u32, limit: i32, child: Option<usize>| {
        let mut b = item(&t, "buc", body::LEFT_HAND);
        b.flags = flags;
        b.max_sockets = limit;
        b.first_child = child;
        b.auto_affix = 7;
        run(&equip(vec![b], None), &t).1[part::SH]
    };
    assert_eq!(socketed(FLAG_SOCKETED, 3, Some(gem)), 64 + 11 + 1);
    // Any condition missing → the automagic affix (colour 2).
    assert_eq!(socketed(0, 3, Some(gem)), 64 + 2 + 1);
    assert_eq!(socketed(FLAG_SOCKETED, 0, Some(gem)), 64 + 2 + 1);
    let ring = rec(&t, "rin");
    assert_eq!(socketed(FLAG_SOCKETED, 3, Some(ring)), 64 + 2 + 1);
}

#[test]
fn a_state_colour_does_not_write_the_byte() {
    let mut col = dblu();
    col.states = vec![StateColour {
        state: 16,
        item_type: ty::WEAP,
        itemtrans: 1,
    }];
    let t = tables_with(col, rows(), |r| r.transform = 5);
    let mut w = item(&t, "jav", body::RIGHT_HAND);
    w.quality = quality::MAGIC;
    w.prefix = [4, 0, 0];
    let mut eq = equip(vec![w], Some(0));
    // Without the state: the affix colour.
    assert_eq!(run(&eq, &t).1[part::RH], 160 + 5 + 1);
    // With it: not written, 0xFF + 1.
    eq.states = vec![16];
    assert_eq!(run(&eq, &t).1[part::RH], 0x00);
    // A shield is not a `weap`: the state does not apply.
    let mut b = item(&t, "buc", body::LEFT_HAND);
    b.quality = quality::MAGIC;
    b.prefix = [4, 0, 0];
    let mut eq = equip(vec![b], None);
    eq.states = vec![16];
    assert_eq!(run(&eq, &t).1[part::SH], 160 + 5 + 1);
}

#[test]
fn colour_byte_and_palette_bounds() {
    assert_eq!(colour_byte(8, 5), 5);
    assert_eq!(colour_byte(1, 20), 52);
    assert_eq!(colour_byte(0, 5), 0);
    assert_eq!(colour_byte(9, 5), 0);
    assert_eq!(colour_byte(2, 21), 0);
    assert_eq!(colour_byte(2, -1), 0);
    for t in [1, 2, 5, 6, 7, 8] {
        assert!(colour_ok(t, 20));
        assert!(!colour_ok(t, 21));
    }
    for t in [0, 3, 4, 9] {
        assert!(!colour_ok(t, 0));
    }
    assert_eq!(link8_signed(0xFF), -1);
    // `Transform` 3 saves 0xFF.
    let t = tables_with(dblu(), rows(), |r| r.transform = 3);
    let mut b = item(&t, "buc", body::LEFT_HAND);
    b.quality = quality::MAGIC;
    b.prefix = [4, 0, 0];
    assert_eq!(run(&equip(vec![b], None), &t).1[part::SH], 0xFF);
}

#[test]
fn vector_circlet_and_other_locations_change_nothing() {
    let t = tables();
    let mut ring = item(&t, "rin", 6);
    ring.mode = MODE_EQUIPPED;
    let mut stored = item(&t, "cap", body::HEAD);
    stored.mode = 0;
    let eq = equip(vec![item(&t, "ci0", body::HEAD), ring, stored], None);
    let (c, k) = run(&eq, &t);
    assert_eq!((c, k), ([0xFF; 16], [0xFF; 16]));
    // A helm writes its own component (`cap` component 0).
    let (c, _) = run(&equip(vec![item(&t, "cap", body::HEAD)], None), &t);
    assert_eq!(c[part::HD], 57);
}

#[test]
fn later_items_overwrite_earlier_bytes() {
    let t = tables();
    let a = item(&t, "cap", body::HEAD);
    let b = item(&t, "h1", body::HEAD);
    // Both in the list (the fill walks the list, not the grid).
    let eq = Equipment {
        items: vec![a, b],
        ..Equipment::default()
    };
    assert_eq!(run(&eq, &t).0[part::HD], 58);
}

#[test]
fn an_item_without_a_token_clears_its_component() {
    let t = tables();
    // `rin` (drawn as `zzz`, in no entry) has no token; with component
    // 3 it clears part 3.
    let t2 = tables_with(ColourTables::default(), t.items.clone(), |r| {
        if r.code == code("rin") {
            r.component = 3;
            r.alternategfx = code("zzz");
        }
    });
    let it = item(&t2, "rin", body::RIGHT_HAND);
    let (mut c, mut k) = ([7; 16], [7; 16]);
    fill(&equip(vec![it], None), &t2, &mut c, &mut k);
    assert_eq!((c[3], k[3]), (0xFF, 0xFF));
    assert_eq!(c[part::RH], 7, "no hand owner byte written");
}

#[test]
fn a_component_past_the_parts_writes_nothing() {
    let t = tables_with(ColourTables::default(), rows(), |r| {
        if r.code == code("cap") {
            r.component = 16;
        }
    });
    let (c, k) = run(&equip(vec![item(&t, "cap", body::HEAD)], None), &t);
    assert_eq!((c, k), ([0xFF; 16], [0xFF; 16]));
}

#[test]
fn a_crossbow_class_copies_the_right_hand_token_to_part_6() {
    let t = tables();
    let mut eq = equip(
        vec![
            item(&t, "lxb", body::RIGHT_HAND),
            item(&t, "buc", body::LEFT_HAND),
        ],
        Some(0),
    );
    eq.weapon_class = wclass::XBW;
    let (c, _) = run(&eq, &t);
    // `lxb` (component 5, `xbw`, not 1hs/1ht/ht1): R by its component.
    assert_eq!(c[part::RH], 50);
    assert_eq!(c[part::LH], 50);
    // `buc` component 7: no copy from it; other classes copy nothing.
    eq.weapon_class = wclass::BOW;
    let (c, _) = run(&eq, &t);
    assert_eq!(c[part::LH], 0xFF);
}

#[test]
fn rebuild_ignores_the_loaded_bytes() {
    let t = tables();
    let mut h = Header {
        components: [1; 16],
        colours: [2; 16],
        ..Header::default()
    };
    let eq = equip(vec![item(&t, "jav", body::RIGHT_HAND)], Some(0));
    h.rebuild_appearance(&eq, &t);
    let mut want = [0xFF; 16];
    want[part::RH] = 0x1B;
    assert_eq!(h.components, want);
    assert_eq!(h.colours, [0xFF; 16]);
    h.rebuild_appearance(&Equipment::default(), &t);
    assert_eq!(h.components, [0xFF; 16]);
}

#[test]
fn wclass_indices() {
    assert_eq!(wclass_index(code("ht1")), 12);
    assert_eq!(wclass_index(code("xbw")), 7);
    assert_eq!(wclass_index(code("stf")), 4);
    assert_eq!(wclass_index(code("")), 0);
}

#[test]
fn reference_types_cover_the_256_slots_in_order() {
    let mut next = 0usize;
    for &(a, b, _) in &REFERENCE_TYPES {
        assert_eq!(usize::from(a), next, "runs are contiguous");
        assert!(b >= a);
        next = usize::from(b) + 1;
    }
    assert_eq!(next, 256);
    let t = reference_types();
    // Spot values of the table.
    assert_eq!((t[0], t[3], t[4], t[10], t[11]), (1, 1, 37, 37, 3));
    assert_eq!((t[47], t[117], t[134], t[198], t[255]), (43, 1, 3, 42, 3));
}
