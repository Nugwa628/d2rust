// Spec: specs/world/vendors.md §1–§4, §6 (Test vectors, edge cases 1, 8,
// 9, 10)
use super::*;
use crate::items::q;
use crate::rng::Seed;
use crate::world::vendors::price::PriceFatal;
use crate::world::vendors::store::{
    clear_record, client_left, generate, level_changed, make_store_item, open, quality_draw, range,
    refresh_act, StoreCtx,
};

fn hires(w: &Fake) -> Vec<String> {
    w.log
        .iter()
        .filter(|l| l.starts_with("hire"))
        .cloned()
        .collect()
}

fn ctx<'a>(t: &'a VendorTables, seed: &'a mut Seed) -> StoreCtx<'a> {
    StoreCtx { tables: t, seed }
}

/// Codes of the record's store items, in order.
fn codes(t: &VendorTables, w: &Fake, rec: &VendorRecord) -> Vec<String> {
    rec.store
        .iter()
        .map(|u| {
            let c = t.items[w.unit(u.0).record].code;
            String::from_utf8_lossy(&c).trim_end().to_string()
        })
        .collect()
}

// Covers: specs/world/vendors.md §1 r1, §1 r2
#[test]
fn column_lists() {
    let mut t = tables();
    let skc = index(&t, "skc");
    t.items[skc].spawnable = 0;
    t.items[skc].columns[CHARSI_COL] = [1, 1, 1, 1, 1];
    let c = Column::build(&t, CHARSI_COL);
    let mut want: Vec<[u8; 4]> = CHARSI_STORE.iter().map(|(c, _)| code(c)).collect();
    want.push(code("axe"));
    assert_eq!(c.items.iter().map(|e| e.code).collect::<Vec<_>>(), want);
    assert_eq!(c.perm, vec![code("aqv"), code("cqv")]);
    let hax = &c.items[0];
    assert_eq!(
        (
            hax.min,
            hax.max,
            hax.magic_min,
            hax.magic_max,
            hax.magic_lvl
        ),
        (2, 1, 0, 1, 0)
    );
    // Max = 0 and MagicMax = 0: not listed.
    let lax = index(&t, "lax");
    t.items[lax].columns[CHARSI_COL] = [5, 0, 5, 0, 5];
    assert!(!Column::build(&t, CHARSI_COL)
        .items
        .iter()
        .any(|e| e.code == code("lax")));
    assert_eq!(Column::build(&t, 0).perm, vec![code("yps")]);
}

// Covers: specs/world/vendors.md §1 r2, §1 r3, §1 r4, §1 r5, §edge-cases-original-bugs r1
#[test]
fn records_and_switches() {
    let mut t = tables();
    // Column 16 for Nihlathak's global build; column 11 is never built.
    let skc = index(&t, "skc");
    t.items[skc].columns[16] = [1, 1, 0, 0, 0];
    t.items[skc].columns[11] = [1, 1, 0, 0, 0];
    t.items[skc].columns[15] = [2, 2, 0, 0, 0];
    let g = GlobalLists::build(&t);
    assert_eq!(g.columns[16].items.len(), 1, "built for nihlathak");
    assert!(g.columns[11].items.is_empty(), "Cain's column");
    assert!(g.columns[15].items.is_empty(), "no larzuk in interact");
    let mut t2 = t.clone();
    t2.interact.push(class::LARZUK);
    let g2 = GlobalLists::build(&t2);
    let n = VendorRecord::new(class::NIHLATHAK, 4, true, &g2);
    assert_eq!(n.items, g2.columns[15].items, "copy is Larzuk's 15");
    assert_eq!(n.items[0].min, 2);
    assert!(n.has_gamble && !n.flag24);
    let gh = VendorRecord::new(class::GHEED, 0, true, &g);
    assert!(gh.has_gamble && gh.flag24 && gh.flag25);
    let ch = VendorRecord::new(class::CHARSI, 0, true, &g);
    assert!(!ch.has_gamble && ch.flag24);
    assert_eq!(ch.perm, vec![code("aqv"), code("cqv")]);
    let not_trader = VendorRecord::new(class::CHARSI, 0, false, &g);
    assert!(not_trader.items.is_empty() && not_trader.perm.is_empty());
    assert!(!not_trader.flag24);
    assert_eq!(column_of(150), None);
    assert_eq!(global_column_of(class::NIHLATHAK), Some(16));
}

// Covers: specs/world/vendors.md §2
#[test]
fn store_levels() {
    assert_eq!(store_level(1, 0, 0), 6);
    assert_eq!(store_level(10, 0, 0), 12);
    assert_eq!(store_level(10, 0, 1), 15);
    assert_eq!(store_level(40, 0, 4), 45);
    assert_eq!(store_level(40, 0, 3), 36);
    assert_eq!(store_level(40, 1, 0), 45, "no cap in Nightmare");
    assert_eq!(store_level(80, 2, 4), 85);
}

// Covers: specs/world/vendors.md §3 r4
#[test]
fn quality_draw_vector() {
    let mut s = Seed::new(1, 666);
    let got: Vec<u8> = (0..5).map(|_| quality_draw(&mut s, 6)).collect();
    assert_eq!(got, vec![2, 2, 2, 3, 2]);
    assert_eq!(s, Seed::new(3_217_527_747, 1_278_238_622));
    // Bands: Seed {0, x} steps to lo' = x.
    let q = |x: u32, ilvl: i32| quality_draw(&mut Seed::new(0, x), ilvl);
    assert_eq!((q(90, 4), q(91, 4)), (q::NORMAL, q::LOW));
    assert_eq!((q(85, 9), q(86, 9)), (q::NORMAL, q::SUPERIOR));
    assert_eq!((q(74, 10), q(75, 10)), (q::NORMAL, q::SUPERIOR));
}

// Covers: specs/world/vendors.md §3 r2
#[test]
fn range_helper() {
    let mut s = Seed::new(1, 666);
    assert_eq!(range(&mut s, 5, 5), 5);
    assert_eq!(range(&mut s, 5, 2), 5);
    assert_eq!(s, Seed::new(1, 666), "no draw for an empty range");
    let mut r = Seed::new(1, 666);
    assert_eq!(range(&mut s, 2, 12), r.roll(10) as i32 + 2);
    assert_eq!(s, r);
}

/// Generation of the recorded Charsi store at character level 1.
fn charsi_store(w: &mut Fake, seed: &mut Seed) -> (VendorTables, VendorRecord) {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 0);
    generate(&mut ctx(&t, seed), &mut rec, w, p(), 77).unwrap();
    (t, rec)
}

// Covers: specs/world/vendors.md §3 text, §3 r1, §3 r2, §3 r3, §3 r4, §3 r5, §3.1 r5, §3.2
#[test]
fn charsi_store_vector() {
    let mut w = Fake::new(class::CHARSI);
    let mut seed = Seed::new(1, 666);
    let (t, rec) = charsi_store(&mut w, &mut seed);
    let want = "hax hax lax lax spc ssd scm scm dgr dgr tkf tkf jav jav spr spr bar bar \
                sbw sbw hbw hbw ktr ktr cap cap skp qui qui lea hla buc buc sml sml lgl lgl \
                lbt lbt lbl lbl aqv cqv";
    assert_eq!(codes(&t, &w, &rec).join(" "), want);
    assert_eq!(rec.store_time, 77);
    // Every request at item level 6; the magic jav second.
    assert!(w.created.iter().all(|&(_, _, ilvl)| ilvl == 6));
    let quals: Vec<u8> = rec.store.iter().map(|u| w.unit(u.0).quality).collect();
    assert_eq!(quals[13], q::MAGIC);
    for (i, &qv) in quals.iter().enumerate() {
        if i != 13 {
            assert!(qv == q::NORMAL || qv == q::SUPERIOR, "item {i}: {qv}");
        }
    }
    // Draws: 40 quality steps and jav's range(1, 2); axe (level 7) and
    // the no-draw ranges draw nothing.
    let mut s = Seed::new(1, 666);
    for _ in 0..41 {
        s.step();
    }
    assert_eq!(seed, s);
    // Marked, shown, permanent stacks full.
    for u in &rec.store {
        assert!(w.unit(u.0).flags & flag::IDENTIFIED != 0);
        assert!(w.unit(u.0).uflags & unit_flag::VENDOR != 0);
    }
    assert_eq!(w.trade_inv.len(), 43);
    let aqv = rec.store[41];
    assert_eq!(w.get(aqv.0, stat::QUANTITY), 500);
    assert_eq!(w.get(rec.store[42].0, stat::QUANTITY), 350);
}

// Covers: specs/world/vendors.md §3 r3, §edge-cases-original-bugs r8
#[test]
fn classic_skips_expansion_items_after_the_draw() {
    let mut t = tables();
    let ktr = index(&t, "ktr");
    t.items[ktr].columns[CHARSI_COL] = [1, 3, 0, 0, 0];
    let mut rec = record(&t, class::CHARSI, 0);
    rec.items.retain(|e| e.code == code("ktr"));
    rec.perm.clear();
    let mut w = Fake::new(class::CHARSI);
    w.expansion = false;
    w.format = 2;
    let mut seed = Seed::new(1, 666);
    generate(&mut ctx(&t, &mut seed), &mut rec, &mut w, p(), 0).unwrap();
    assert!(rec.store.is_empty());
    let mut s = Seed::new(1, 666);
    s.step();
    assert_eq!(seed, s, "the n_norm draw happened");
}

// Covers: specs/world/vendors.md §3 r2, §3 r5
#[test]
fn high_level_store_draws() {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 4);
    let mut w = Fake::new(class::CHARSI);
    w.set(PLAYER, stat::LEVEL, 30);
    let mut seed = Seed::new(5, 666);
    generate(&mut ctx(&t, &mut seed), &mut rec, &mut w, p(), 0).unwrap();
    // ilvl 35: no normal items; jav's k draw and n_mag draw; perms.
    let mut s = Seed::new(5, 666);
    let k = range(&mut s, 1, 3) + 1;
    let n_mag = range(&mut s, 1, k);
    assert_eq!(seed, s);
    assert_eq!(rec.store.len(), n_mag as usize + 2);
    assert!(w.created.iter().all(|&(_, _, ilvl)| ilvl == 35));
}

// Covers: specs/world/vendors.md §3.3, §3.4, §3.1 r4
#[test]
fn failures_stop_normal_items() {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 0);
    let mut w = Fake::new(class::CHARSI);
    w.store_room = [0; 4];
    let mut seed = Seed::new(1, 666);
    generate(&mut ctx(&t, &mut seed), &mut rec, &mut w, p(), 0).unwrap();
    // 33 nulls: the 33rd stops; every one parked as a deferred node.
    assert_eq!(w.created.len(), 33);
    assert_eq!(rec.events.len(), 33);
    assert!(rec.events.iter().all(|e| e.deferred));
    assert!(rec.store.is_empty());
    // Clearing the record destroys deferred items.
    clear_record(&mut rec, &mut w);
    assert_eq!(w.destroyed.len(), 33);
    assert!(rec.events.is_empty());
}

// Covers: specs/world/vendors.md §3 r5, §3.3, §edge-cases-original-bugs r9
#[test]
fn magic_nulls_never_stop() {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 0);
    rec.items = vec![ColumnEntry {
        min: 0,
        max: 0,
        magic_min: 40,
        magic_max: 39,
        code: code("jav"),
        magic_lvl: 1,
    }];
    let mut w = Fake::new(class::CHARSI);
    w.store_room = [0; 4];
    let mut seed = Seed::new(1, 666);
    generate(&mut ctx(&t, &mut seed), &mut rec, &mut w, p(), 0).unwrap();
    // range(0, 1) draws once; 40 magic nulls; then aqv's null (41 > 32)
    // stops before cqv.
    assert_eq!(w.created.len(), 41);
    assert_eq!(w.created[40].0, index(&t, "aqv"));
}

// Covers: specs/world/vendors.md §3.1 text, §3.1 r2, §3.1 r3, §3.1 r4
#[test]
fn store_item_tries() {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 0);
    let mut w = Fake::new(class::CHARSI);
    let mut seed = Seed::new(1, 666);
    let hax = index(&t, "hax");
    // Four cracked tries, then a good one.
    w.create_queue = VecDeque::from([Some(true); 4]);
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 6, 1).unwrap();
    assert!(it.is_some());
    assert_eq!(w.destroyed.len(), 4);
    // Five cracked tries: null.
    w.create_queue = VecDeque::from([Some(true); 5]);
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 6, 1).unwrap();
    assert!(it.is_none());
    // Code mismatch: destroyed, second round with quality 2, whose
    // mismatch also destroys the item: null (§3.1 rule 2, V7).
    w.created.clear();
    let destroyed = w.destroyed.len();
    w.create_as.insert(hax, index(&t, "lax"));
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 4, 6, 1).unwrap();
    assert_eq!(it, None);
    assert_eq!(w.created, vec![(hax, 4, 6), (hax, 2, 6)]);
    assert_eq!(w.destroyed.len(), destroyed + 2);
    w.create_as.clear();
    // A null creation call is the fatal assert (line 0x61A).
    w.create_queue = VecDeque::from([None]);
    assert_eq!(
        make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 6, 1),
        Err(PriceFatal::NullStoreItem)
    );
    // No store page: destroyed, null.
    let mut t2 = t.clone();
    t2.items[hax].type_ = T_NOPAGE;
    let before = w.destroyed.len();
    let it = make_store_item(&mut ctx(&t2, &mut seed), &mut rec, &mut w, hax, 2, 6, 1).unwrap();
    assert!(it.is_none());
    assert_eq!(w.destroyed.len(), before + 1);
    // Page 1 full: page 2.
    w.store_room = [10, 0, 10, 10];
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 6, 1)
        .unwrap()
        .unwrap();
    assert_eq!(w.unit(it.0).page, 2);
    // Page 0 full: parked, null.
    let cap = index(&t, "cap");
    w.store_room = [0, 10, 10, 10];
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, cap, 2, 6, 1).unwrap();
    assert!(it.is_none());
    assert!(rec.events.last().unwrap().deferred);
    // Repaired before placement: durability := max.
    w.store_room = [10; 4];
    w.create_queue = VecDeque::from([Some(false)]);
    let it = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 6, 1)
        .unwrap()
        .unwrap();
    assert!(w.log.iter().any(|l| l == &format!("recharge {}", it.0)));
    assert_eq!(w.get(it.0, stat::DURABILITY), 20);
    assert!(w.stat_msgs.is_empty(), "no player: no 0x3E");
}

// Covers: specs/world/vendors.md §3.1 r1
#[test]
fn upgrades() {
    let t = tables();
    let hax = index(&t, "hax");
    let (uber, ultra) = (index(&t, "9ha"), index(&t, "7ha"));
    let run = |d: u8, lp: i32, x: u32, t: &VendorTables| {
        let mut rec = record(t, class::CHARSI, 0);
        let mut w = Fake::new(class::CHARSI);
        w.difficulty = d;
        let mut seed = Seed::new(0, x);
        make_store_item(&mut ctx(t, &mut seed), &mut rec, &mut w, hax, 2, 35, lp).unwrap();
        (w.created[0].0, seed)
    };
    // Nightmare, ilvl 35: r < 35·64 + 4000 = 6240 → ubercode.
    assert_eq!(run(1, 30, 6239, &t).0, uber);
    assert_eq!(run(1, 30, 6240, &t).0, hax);
    // Hell: r < 35·16 + 1000 = 1560 → ultracode; r < 9480 → ubercode.
    assert_eq!(run(2, 30, 1559, &t).0, ultra);
    assert_eq!(run(2, 30, 9479, &t).0, uber);
    assert_eq!(run(2, 30, 9480, &t).0, hax);
    // HellUpgrade overrides.
    let mut t2 = t.clone();
    t2.items[hax].hell_upgrade = code("lax");
    assert_eq!(run(2, 30, 1559, &t2).0, index(&t, "lax"));
    t2.items[hax].nightmare_upgrade = code("spc");
    assert_eq!(run(1, 30, 9000, &t2).0, index(&t, "spc"));
    // No draw in Normal or with L_p ≤ 25.
    assert_eq!(run(0, 30, 5, &t).1, Seed::new(0, 5));
    assert_eq!(run(2, 25, 5, &t).1, Seed::new(0, 5));
}

// An ubercode that is set but missing from the code map is still chosen
// (rule 1 tests only ≠ 0 / spaces); its lookup gives class 0, which never
// matches the chosen code, so both rounds fail and the item is null.
// Covers: specs/world/vendors.md §3.1 r1, §3.1 r2
#[test]
fn an_unfound_upgrade_code_gives_no_item() {
    let mut t = tables();
    let hax = index(&t, "hax");
    t.items[hax].ubercode = code("zzz");
    let mut rec = record(&t, class::CHARSI, 0);
    let mut w = Fake::new(class::CHARSI);
    w.difficulty = 1;
    // Nightmare, ilvl 35: r = 6239 < 6240 picks the ubercode.
    let mut seed = Seed::new(0, 6239);
    let got = make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 35, 30);
    assert_eq!(got, Ok(None));
    // One creation per round, each of class 0 and destroyed.
    assert_eq!(
        w.created.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![0, 0]
    );
    // Above the threshold the base code is kept.
    let mut w = Fake::new(class::CHARSI);
    w.difficulty = 1;
    let mut seed = Seed::new(0, 6240);
    let mut rec = record(&t, class::CHARSI, 0);
    assert!(
        make_store_item(&mut ctx(&t, &mut seed), &mut rec, &mut w, hax, 2, 35, 30)
            .unwrap()
            .is_some()
    );
}

// Covers: specs/world/vendors.md §4 text, §4 r1, §4 r2, §4 r3, §6 r5
#[test]
fn trade_open() {
    let t = tables();
    let mut rec = record(&t, class::CHARSI, 0);
    let mut w = Fake::new(class::CHARSI);
    let mut seed = Seed::new(1, 666);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        10,
    )
    .unwrap();
    assert!(rec.has_traded && rec.store_generated);
    assert_eq!(rec.last_npc, NPC);
    assert_eq!(rec.chain_node(PLAYER).map(|n| n.gamble_mode), Some(false));
    assert_eq!(w.hire_made, vec![class::CHARSI]);
    assert!(hires(&w).is_empty(), "charsi makes no hire list");
    assert_eq!(w.refreshed, vec![NPC]);
    // Generation marks 43, the open shows 43 more.
    assert_eq!(w.trade_inv.len(), 86);
    // A second open keeps the store: no draws.
    let s = seed;
    let first = rec.store.clone();
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        20,
    )
    .unwrap();
    assert_eq!((seed, &rec.store), (s, &first));
    // Pending refresh, not single: kept.
    rec.refresh_pending = true;
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        false,
        false,
        30,
    )
    .unwrap();
    assert_eq!(rec.store, first);
    // Single: cleared and regenerated.
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        40,
    )
    .unwrap();
    assert!(!rec.refresh_pending);
    assert_ne!(rec.store, first);
    assert_eq!(w.removed.len(), 43);
    assert_eq!(w.new_inventories, vec![(class::CHARSI, None)]);
    assert_eq!(rec.store_time, 40);
}

// Covers: specs/world/vendors.md §4 r2, §4 r3
#[test]
fn trade_open_classes() {
    let t = tables();
    let g = GlobalLists::build(&t);
    let mut seed = Seed::new(1, 666);
    // Asheara makes the hire list; a non-trader generates nothing.
    let mut rec = VendorRecord::new(class::ASHEARA, 2, true, &g);
    let mut w = Fake::new(class::ASHEARA);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        0,
    )
    .unwrap();
    assert_eq!(hires(&w), vec![format!("hire list {}", class::ASHEARA)]);
    // Kashya: no store refresh, no items shown.
    let mut rec = VendorRecord::new(class::KASHYA, 0, false, &g);
    let mut w = Fake::new(class::KASHYA);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        0,
    )
    .unwrap();
    assert!(w.refreshed.is_empty() && !rec.store_generated);
    assert!(hires(&w).is_empty(), "not a trader");
    // Gamble open at a non-gambler: no list.
    let mut rec = VendorRecord::new(class::CHARSI, 0, true, &g);
    let mut w = Fake::new(class::CHARSI);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        true,
        0,
    )
    .unwrap();
    assert!(rec.gamble_lists.is_empty() && rec.chain.is_empty());
}

// Covers: specs/world/vendors.md §6 r3, §6 r4, §edge-cases-original-bugs r10
#[test]
fn refresh_rule() {
    let t = tables();
    let mut seed = Seed::new(1, 666);
    let mut w = Fake::new(class::CHARSI);
    let mut rec = record(&t, class::CHARSI, 0);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        1000,
    )
    .unwrap();
    let mut recs = vec![rec, record(&t, class::GHEED, 0)];
    // Not empty: the 240000 ms timer (unsigned compare).
    refresh_act(&mut recs, &mut w, 0, false, 241_000);
    assert!(!recs[0].refresh_pending);
    refresh_act(&mut recs, &mut w, 0, false, 241_001);
    assert!(recs[0].refresh_pending && recs[0].store_time == 241_001);
    assert!(!recs[1].refresh_pending, "never traded");
    // Other act: untouched.
    recs[0].refresh_pending = false;
    refresh_act(&mut recs, &mut w, 1, true, 0);
    assert!(recs[0].store_generated);
    // Empty, NPC busy: pending.
    refresh_act(&mut recs, &mut w, 0, true, 0);
    assert!(recs[0].refresh_pending && recs[0].store_generated);
    // Empty, interaction list empty: cleared, the NPC gets the inventory.
    w.npcs.insert(NPC, (class::CHARSI, true));
    let stock: Vec<u32> = recs[0].store.iter().map(|u| u.0).collect();
    assert!(!stock.is_empty());
    refresh_act(&mut recs, &mut w, 0, true, 5);
    assert!(!recs[0].store_generated && recs[0].store.is_empty());
    // §6 rule 4 (`0x00536510`): every store item is freed.
    assert!(stock.iter().all(|g| w.destroyed.contains(g)));
    assert_eq!(recs[0].store_time, 0);
    assert_eq!(w.new_inventories, vec![(class::CHARSI, Some(NPC))]);
    // NPC missing: cleared without an NPC.
    let mut rec = record(&t, class::CHARSI, 0);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        0,
    )
    .unwrap();
    w.npcs.clear();
    let mut recs = vec![rec];
    refresh_act(&mut recs, &mut w, 0, true, 0);
    assert!(!recs[0].store_generated);
    assert_eq!(w.new_inventories.last(), Some(&(class::CHARSI, None)));
}

// Covers: specs/world/vendors.md §6 text, §6 r1, §6 r2
#[test]
fn leaving_town() {
    let t = tables();
    let mut seed = Seed::new(1, 666);
    let mut w = Fake::new(class::CHARSI);
    let mut rec = record(&t, class::CHARSI, 0);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        0,
    )
    .unwrap();
    w.npcs.insert(NPC, (class::CHARSI, true));
    let mut recs = vec![rec];
    // Another player in town: no reset.
    w.players_in.insert(1, 1);
    level_changed(&mut recs, &mut w, p(), 1, 2, 0);
    assert!(recs[0].store_generated);
    w.players_in.insert(1, 0);
    level_changed(&mut recs, &mut w, p(), 1, 2, 0);
    assert!(
        !recs[0].store_generated,
        "single player: new store next time"
    );
    assert!(w.intros.is_empty());
    level_changed(&mut recs, &mut w, p(), 2, 40, 0);
    level_changed(&mut recs, &mut w, p(), 2, 103, 0);
    assert_eq!(w.intros, vec![40]);
    // Client leaving: counted minus 1; game type 3 and level 109 skipped.
    let mut rec = record(&t, class::CHARSI, 0);
    open(
        &mut ctx(&t, &mut seed),
        &mut rec,
        &mut w,
        npc(),
        p(),
        true,
        false,
        0,
    )
    .unwrap();
    let mut recs = vec![rec];
    w.players_in.insert(1, 1);
    w.game_type = 3;
    client_left(&mut recs, &mut w, p(), 0);
    assert!(recs[0].store_generated);
    w.game_type = 0;
    w.level_id = 109;
    client_left(&mut recs, &mut w, p(), 0);
    assert!(recs[0].store_generated);
    w.level_id = 1;
    client_left(&mut recs, &mut w, p(), 0);
    assert!(!recs[0].store_generated);
}
