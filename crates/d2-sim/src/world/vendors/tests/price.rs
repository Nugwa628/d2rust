// Spec: specs/world/vendors.md §9 (Test vectors, edge cases 5, 7, 13)
use super::*;
use crate::world::vendors::price::{charged_skills, gamble_price, Bonus, PriceFatal};

const TEST_NPC: u16 = 600;

fn ctx(class: u16) -> PriceCtx {
    PriceCtx {
        difficulty: 0,
        npc_class: class,
        reduced_prices: 0,
        player_level: 1,
        quest_slots: [0; 3],
    }
}

fn pi(t: &VendorTables, c: &str) -> PriceItem {
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

fn price(t: &VendorTables, c: &PriceCtx, it: &PriceItem, txn: u32) -> i32 {
    cost(t, c, Some(it), txn).unwrap()
}

// Covers: specs/world/vendors.md §9.2 l2 r2, §9.2 l2 r9, §9.2 l2 r13, §9.3
#[test]
fn recorded_prices() {
    let t = tables();
    // Buy dgr (cost 60) from Charsi: 60·960/1024 = 56.
    assert_eq!(price(&t, &ctx(class::CHARSI), &pi(&t, "dgr"), tx::BUY), 56);
    // Sell skc (cost 1000) to Charsi: 1000·512/1024 = 500.
    assert_eq!(
        price(&t, &ctx(class::CHARSI), &pi(&t, "skc"), tx::SELL),
        500
    );
    // Buy yps (cost 40) from Akara: 40.
    let mut yps = pi(&t, "yps");
    yps.max_durability = 0;
    assert_eq!(price(&t, &ctx(class::AKARA), &yps, tx::BUY), 40);
}

// Covers: specs/world/vendors.md §9.2 l2 r9, §9.3, §edge-cases-original-bugs r6
#[test]
fn quest_multipliers() {
    let t = tables();
    let skc = pi(&t, "skc");
    let mut c = ctx(class::GHEED);
    assert_eq!(price(&t, &c, &skc, tx::BUY), 1062);
    for bits in [1, 2, 3] {
        c.quest_slots[0] = bits;
        assert_eq!(price(&t, &c, &skc, tx::BUY), 956, "slot 4 = {bits}");
    }
    c.quest_slots[0] = 4;
    assert_eq!(price(&t, &c, &skc, tx::BUY), 1062, "bit 2 only");
    // Malah, S = 100: 200; slot 41: 180; slots 41 and 35: 90.
    let lax = pi(&t, "lax");
    let mut c = ctx(class::MALAH);
    assert_eq!(price(&t, &c, &lax, tx::BUY), 200);
    c.quest_slots[0] = 1;
    assert_eq!(price(&t, &c, &lax, tx::BUY), 180);
    c.quest_slots[1] = 2;
    assert_eq!(price(&t, &c, &lax, tx::BUY), 90);
}

// Covers: specs/world/vendors.md §9.2 text, §9.2 l2 r9, §9.2 l2 r12, §edge-cases-original-bugs r5
#[test]
fn buy_guard_and_max_buy() {
    let mut t = tables();
    let skc = index(&t, "skc");
    t.items[skc].cost = 100_000;
    let it = pi(&t, "skc");
    // B = 100000, buy mult 512: (100000/1024)·512 = 49664.
    assert_eq!(price(&t, &ctx(TEST_NPC), &it, tx::SELL), 49664);
    // The guard tests the sell multiplier: sell 0 → 100000·512/1024.
    let row = t.npc.iter_mut().find(|r| r.class == 600).unwrap();
    row.sell = 0;
    assert_eq!(price(&t, &ctx(TEST_NPC), &it, tx::SELL), 50000);
    // B = 20000 at Charsi, Normal: min(10000, 5000).
    t.items[skc].cost = 20_000;
    assert_eq!(price(&t, &ctx(class::CHARSI), &it, tx::SELL), 5000);
    let mut nm = ctx(class::CHARSI);
    nm.difficulty = 1;
    assert_eq!(price(&t, &nm, &it, tx::SELL), 10000);
}

// Covers: specs/world/vendors.md §9.2 l2 r2, §9.2 l2 r10
#[test]
fn arrows_are_not_multiplied() {
    let t = tables();
    let mut aqv = pi(&t, "aqv");
    aqv.quantity = 350;
    aqv.max_durability = 0;
    // S = 256·350/1024 = 87 → 87·960/1024 = 81.
    assert_eq!(price(&t, &ctx(class::CHARSI), &aqv, tx::BUY), 81);
}

// Covers: specs/world/vendors.md §9.4, §9.2 l2 r1
#[test]
fn gamble_prices() {
    let mut t = tables();
    let hax = pi(&t, "hax");
    for (l, want) in [
        (1, 771),
        (5, 771),
        (10, 1656),
        (30, 6896),
        (60, 21552),
        (99, 57986),
    ] {
        let mut c = ctx(class::GHEED);
        c.player_level = l;
        assert_eq!(price(&t, &c, &hax, tx::GAMBLE), want, "L = {l}");
    }
    // Reduced prices, no minimum: 771 − 771·10/100.
    let mut c = ctx(class::GHEED);
    c.reduced_prices = 10;
    assert_eq!(price(&t, &c, &hax, tx::GAMBLE), 771 - 77);
    // Format 0 and rin / amu: the gamble cost.
    let h = index(&t, "hax");
    t.items[h].gamble_cost = 999;
    let mut f0 = hax.clone();
    f0.format = 0;
    assert_eq!(gamble_price(&t, &f0, 60, 0).unwrap(), 999);
    let r = index(&t, "rin");
    t.items[r].gamble_cost = 12345;
    assert_eq!(gamble_price(&t, &pi(&t, "rin"), 60, 0).unwrap(), 12345);
}

// Covers: specs/world/vendors.md §9.2 l2 r8, §9.2 l2 r13
#[test]
fn repair_value() {
    let t = tables();
    let mut lax = pi(&t, "lax");
    lax.durability = 5;
    // R = 100, durability 5 of 20: 75.
    assert_eq!(price(&t, &ctx(TEST_NPC), &lax, tx::REPAIR), 75);
    // Replenishing durability: (max − 1)·R/max while durability < max − 1.
    lax.replenish_durability = 1;
    assert_eq!(price(&t, &ctx(TEST_NPC), &lax, tx::REPAIR), 95);
    lax.durability = 19;
    assert_eq!(price(&t, &ctx(TEST_NPC), &lax, tx::REPAIR), 1, "R 0 → 1");
}

// Covers: specs/world/vendors.md §edge-cases-original-bugs r13, §9.2 r0
#[test]
fn rule_zero() {
    let t = tables();
    let c = ctx(TEST_NPC);
    assert_eq!(cost(&t, &c, None, tx::BUY), Ok(0x7FFF_FFFF));
    let mut lax = pi(&t, "lax");
    lax.durability = 5;
    lax.flags = 0;
    assert_eq!(price(&t, &c, &lax, tx::REPAIR), 0, "unidentified");
    lax.flags = flag::IDENTIFIED | flag::ETHEREAL;
    assert_eq!(price(&t, &c, &lax, tx::REPAIR), 0, "ethereal");
    lax.flags = flag::IDENTIFIED | flag::STARTITEM;
    assert_eq!(price(&t, &c, &lax, tx::BUY), 1);
    // A potion: no repair column, no durability → not repairable.
    let mut yps = pi(&t, "yps");
    yps.max_durability = 0;
    assert_eq!(price(&t, &c, &yps, tx::REPAIR), 0);
}

// Covers: specs/world/vendors.md §9.2 l2 r2
#[test]
fn base_kinds() {
    let mut t = tables();
    let c = ctx(TEST_NPC);
    // Ear: (level & 0xFF)·cost.
    let mut ear = pi(&t, "ear");
    ear.flags |= flag::EAR;
    ear.ear_level = 0x105;
    assert_eq!(price(&t, &c, &ear, tx::BUY), 5);
    // Type 40: cost + 8·monstats level of difficulty d.
    let mut body = item("bod", 20, T_BODY);
    body.durability = 0;
    t.items.push(body);
    let mut b = pi(&t, "bod");
    b.file_index = 1;
    assert_eq!(price(&t, &c, &b, tx::BUY), 20 + 80);
    let mut hell = c;
    hell.difficulty = 2;
    assert_eq!(price(&t, &hell, &b, tx::BUY), 20 + 560);
    b.file_index = 9;
    assert_eq!(price(&t, &c, &b, tx::BUY), 20, "no monstats row");
    // Book: cost + qty·costpercharge of suffix 0; not ×qty.
    let mut book = pi(&t, "ibk");
    book.suffix[0] = 1;
    book.quantity = 20;
    assert_eq!(price(&t, &c, &book, tx::BUY), 100 + 20 * 16);
    book.suffix[0] = 5;
    assert_eq!(
        cost(&t, &c, Some(&book), tx::BUY),
        Err(PriceFatal::NoBook(5))
    );
    // Armor: cost·AC / max AC.
    let qui = index(&t, "qui");
    t.items[qui].minac = 10;
    t.items[qui].maxac = 20;
    let mut a = pi(&t, "qui");
    a.armor_base = 15;
    assert_eq!(price(&t, &c, &a, tx::BUY), 75);
    // Helm is type 50 through the equivalence.
    let cap = index(&t, "cap");
    t.items[cap].maxac = 20;
    let mut h = pi(&t, "cap");
    h.armor_base = 10;
    assert_eq!(price(&t, &c, &h, tx::BUY), 50);
    // max AC − min AC = −1: unchanged.
    t.items[cap].minac = 21;
    assert_eq!(price(&t, &c, &h, tx::BUY), 100);
}

// Covers: specs/world/vendors.md §9.2 l2 r3, §9.2 l2 r4
#[test]
fn quality_deltas() {
    let t = tables();
    let c = ctx(TEST_NPC);
    let base = pi(&t, "lax");
    let with = |q: u8, f: &dyn Fn(&mut PriceItem)| {
        let mut it = base.clone();
        it.quality = q;
        f(&mut it);
        price(&t, &c, &it, tx::BUY)
    };
    // Magic, prefix 1 (mult 2048, add 100): 100 + 100 + 200.
    assert_eq!(with(4, &|it| it.prefix[0] = 1), 400);
    assert_eq!(
        with(4, &|it| {
            it.prefix[0] = 1;
            it.flags = 0;
        }),
        100,
        "unidentified: no deltas"
    );
    // Unique row 0: add 5000 + 100·5120/1024.
    assert_eq!(with(7, &|it| it.file_index = 0), 5600);
    assert_eq!(
        with(7, &|it| {
            it.file_index = 3;
            it.suffix[0] = 2;
        }),
        400,
        "no row: as magic"
    );
    // Set row 0: add 1000 + 100·3072/1024.
    assert_eq!(with(5, &|it| it.file_index = 0), 1400);
    // Rare: three affixes.
    assert_eq!(
        with(6, &|it| {
            it.prefix = [1, 2, 0];
            it.suffix = [3, 0, 0];
        }),
        1000
    );
    // Inferior: −(S/2), replacing the automagic delta.
    assert_eq!(with(1, &|it| it.auto_affix = 1), 50);
    // Automagic on a normal item.
    assert_eq!(with(2, &|it| it.auto_affix = 1), 400);
    // Superior with a bonus stat (encode 0: m 512, a 3, /1024).
    assert_eq!(
        with(3, &|it| it.bonuses = vec![Bonus {
            stat: 5,
            layer: 0,
            value: 4
        }]),
        100 + 512 * 400 / 1024 + 3
    );
}

// Covers: specs/world/vendors.md §9.2 l2 r3
#[test]
fn item_skill_costs() {
    let t = tables();
    let c = ctx(TEST_NPC);
    let mut it = pi(&t, "lax");
    // Skill 0 (m 1024, a 10), value 2 → k = 3: (100 + 10)·3.
    it.item_skills = vec![(0, 2)];
    assert_eq!(price(&t, &c, &it, tx::BUY), 100 + 330);
    // Magic: after the deltas.
    it.quality = 4;
    it.prefix[0] = 1;
    assert_eq!(price(&t, &c, &it, tx::BUY), 400 + (400 + 10) * 3);
    // staffmods 7: no item-skill costs.
    let mut t2 = tables();
    let lax = index(&t2, "lax");
    t2.items[lax].type_ = T_STAFF;
    let mut it = pi(&t2, "lax");
    it.item_skills = vec![(0, 2)];
    assert_eq!(price(&t2, &c, &it, tx::BUY), 100);
}

// Covers: specs/world/vendors.md §edge-cases-original-bugs r7
#[test]
fn negative_deltas_divide_unsigned() {
    let t = tables();
    let mut tkn = pi(&t, "tkn");
    tkn.quantity = 1;
    // value 0 → k = −1: dS = −110; div = max stack 60, unsigned.
    tkn.item_skills = vec![(0, 0)];
    let s = 100 + ((-110i32) as u32 / 60) as i32;
    assert_eq!(s, 71_582_886);
    let want = (s / 1024) * 1024;
    assert_eq!(price(&t, &ctx(TEST_NPC), &tkn, tx::BUY), want);
}

// Covers: specs/world/vendors.md §9.2 l2 r11
#[test]
fn charged_skill_costs() {
    let t = tables();
    let mut it = pi(&t, "lax");
    // Skill 1, level 3; 4 of 10 charges; reqlevel 12: t = 9, x = 90000.
    it.charges = vec![((1 << 6) | 3, (10 << 8) | 4)];
    let c = (90_000 / 1024) * 1024;
    let want = 6 * (c + 10) / 10;
    assert_eq!(charged_skills(&t, &it, CHARGE_BASE), want);
    // Full durability: R = 0 from rule 8, plus the charges.
    assert_eq!(price(&t, &ctx(TEST_NPC), &it, tx::REPAIR), want);
    it.charges = vec![((1 << 6) | 3, (10 << 8) | 10)];
    assert_eq!(charged_skills(&t, &it, CHARGE_BASE), 0, "full");
}

// Covers: specs/world/vendors.md §9.2 l2 r6, §9.2 l2 r7, §9.2 l2 r8
#[test]
fn sockets_ethereal_class() {
    let mut t = tables();
    let c = ctx(TEST_NPC);
    let mut it = pi(&t, "lax");
    it.sockets = vec![index(&t, "isc"), index(&t, "isc")];
    assert_eq!(price(&t, &c, &it, tx::BUY), 100 + 16);
    let mut it = pi(&t, "lax");
    // B = 100·512/1024 = 50; ethereal /4.
    assert_eq!(price(&t, &c, &it, tx::SELL), 50);
    it.flags |= flag::ETHEREAL;
    assert_eq!(price(&t, &c, &it, tx::SELL), (100 / 4) * 512 / 1024);
    it.durability = 0;
    assert_eq!(price(&t, &c, &it, tx::SELL), 1, "broken ethereal: B 0 → 1");
    let wt = T_WEAP as usize;
    t.itemtypes[wt].class = 3;
    let it = pi(&t, "lax");
    assert_eq!(price(&t, &c, &it, tx::SELL), (100 / 4) * 512 / 1024);
}

// Covers: specs/world/vendors.md §9.2 l2 r10
#[test]
fn stacks() {
    let t = tables();
    let c = ctx(TEST_NPC);
    let mut tkn = pi(&t, "tkn");
    tkn.quantity = 30;
    tkn.max_durability = 0;
    // B = 50; qty < M, no replenish: R := 100·30, B := 60·50 − 3000.
    assert_eq!(price(&t, &c, &tkn, tx::SELL), 1);
    tkn.replenish_quantity = 1;
    assert_eq!(price(&t, &c, &tkn, tx::SELL), 3000);
    // Not repairable: B ×= qty.
    tkn.flags = 0;
    assert_eq!(price(&t, &c, &tkn, tx::SELL), 50 * 30);
    // S ×= qty for a plain stack.
    assert_eq!(price(&t, &c, &tkn, tx::BUY), 100 * 30);
}

// Covers: specs/world/vendors.md §9.2 l2 r13
#[test]
fn reduced_prices() {
    let t = tables();
    let mut c = ctx(class::CHARSI);
    c.reduced_prices = 10;
    assert_eq!(price(&t, &c, &pi(&t, "dgr"), tx::BUY), 56 - 5);
    c.reduced_prices = 150;
    assert_eq!(
        price(&t, &c, &pi(&t, "dgr"), tx::BUY),
        1,
        "capped 99, min 1"
    );
    // Sell ignores reduced prices.
    assert_eq!(price(&t, &c, &pi(&t, "skc"), tx::SELL), 500);
}

// Covers: specs/world/vendors.md §edge-cases-original-bugs r2
#[test]
fn no_npc_row_is_fatal() {
    let t = tables();
    let r = cost(&t, &ctx(class::KASHYA), Some(&pi(&t, "skc")), tx::SELL);
    assert_eq!(r, Err(PriceFatal::NoNpcRow(class::KASHYA)));
}
