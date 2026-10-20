//! Checks item, quality and affix tables from the player's install in
//! `D2_GAME_DIR`. Sweeps exercise every item and requested quality, forced
//! unique and preferred set rows, and affix picks. Successful results must
//! satisfy the quality, affix and socket checks below.
//!
//! `D2_GAME_DIR=<install> cargo test -p d2-sim --test game_items -- --ignored`

mod items_treasure_live;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use d2_data::tables::Uniqueitems;
use d2_sim::items::affixes::{affix, alvl, roll_affix};
use d2_sim::items::create::max_sockets;
use d2_sim::items::tables::AffixRec;
use d2_sim::items::{
    create_item, flag, q, req, stat, ty, CreateError, Fatal, Item, ItemGame, ItemRequest,
    ItemStats, ItemTables, ListKey, UniqueBits,
};
use d2_sim::rng::Seed;
use items_treasure_live::{code, fixed, typed};

fn tables() -> &'static ItemTables {
    static T: OnceLock<ItemTables> = OnceLock::new();
    T.get_or_init(|| ItemTables::from_fixed(fixed()).expect("item tables project"))
}

/// Stats holder: base stats and keyed lists in ordered maps.
#[derive(Clone, Debug, Default)]
struct Stats {
    any: bool,
    base: BTreeMap<(u16, u16), i32>,
    lists: BTreeMap<ListKey, BTreeMap<(u16, u16), i32>>,
}

impl ItemStats for Stats {
    fn has_stats(&self) -> bool {
        self.any
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        let lists: i32 = self
            .lists
            .values()
            .map(|l| l.get(&(id, layer)).copied().unwrap_or(0))
            .sum();
        self.base(id, layer) + lists
    }
    fn base(&self, id: u16, layer: u16) -> i32 {
        self.base.get(&(id, layer)).copied().unwrap_or(0)
    }
    fn set_base(&mut self, id: u16, layer: u16, value: i32) {
        self.any = true;
        self.base.insert((id, layer), value);
    }
    fn has_list(&self, key: ListKey) -> bool {
        self.lists.contains_key(&key)
    }
    fn list_set(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        self.any = true;
        self.lists
            .entry(key)
            .or_default()
            .insert((id, layer), value);
    }
    fn list_add(&mut self, key: ListKey, id: u16, layer: u16, value: i32) {
        self.any = true;
        *self
            .lists
            .entry(key)
            .or_default()
            .entry((id, layer))
            .or_default() += value;
    }
    fn list_get(&self, key: ListKey, id: u16, layer: u16) -> i32 {
        self.lists
            .get(&key)
            .and_then(|l| l.get(&(id, layer)))
            .copied()
            .unwrap_or(0)
    }
}

/// Game fields creation reads; a fresh one per item, so no unique is
/// marked dropped before.
struct Game {
    seed: Seed,
    difficulty: u8,
    expansion: bool,
    uniques: UniqueBits,
}

impl Game {
    fn new(seed: u32, difficulty: u8, expansion: bool) -> Self {
        Self {
            seed: Seed::init_low(seed),
            difficulty,
            expansion,
            uniques: UniqueBits::default(),
        }
    }
}

impl ItemGame for Game {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn seed(&mut self) -> &mut Seed {
        &mut self.seed
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn ladder_flags(&self) -> (bool, bool) {
        (false, false)
    }
    fn uniques(&mut self) -> &mut UniqueBits {
        &mut self.uniques
    }
}

fn request(game: &Game, item: usize, ilvl: i32, quality: u8) -> ItemRequest {
    ItemRequest {
        ilvl,
        item: item as i32,
        format: game.item_format(),
        quality,
        ..ItemRequest::default()
    }
}

/// `item` is any of `types` (list stops at the first < 1).
fn any_type(t: &ItemTables, item: usize, types: &[i16]) -> bool {
    types
        .iter()
        .take_while(|&&x| x >= 1)
        .any(|&x| t.is_type(item, x))
}

/// `affixes.md` §4.1 steps 3–4: no `etype` matches, an `itype` matches.
fn types_fit(t: &ItemTables, item: usize, row: &AffixRec) -> bool {
    !any_type(t, item, &row.etype) && any_type(t, item, &row.itype)
}

// ------------------------------------------------------------ table facts

/// The combined affix table has suffixes 0–746, prefixes 747–1,415,
/// and automagic rows 1,416–1,451; affix ids are one-based.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_affix_parts() {
    let t = tables();
    assert_eq!((t.n_suffix, t.n_prefix), (747, 669));
    assert_eq!(t.first_auto(), 1_416);
    assert_eq!(t.magic.len(), 1_452);
    // Id = combined index + 1.
    assert!(std::ptr::eq(affix(t, 1).unwrap(), &t.magic[0]));
    assert!(std::ptr::eq(affix(t, 1_452).unwrap(), &t.magic[1_451]));
    assert!(affix(t, 0).is_none() && affix(t, 1_453).is_none());
}

/// The loaded qualityitems table contains eight rows.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_qualityitems_count() {
    assert_eq!(tables().qualityitems.len(), 8);
}

/// The bytes above each unique row's u16 rarity are zero, and D2RUST's
/// projected u32 rarity equals the typed column.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_unique_rarity_32_bits() {
    let raw = fixed().table("uniqueitems").unwrap();
    let typed_rows = typed::<Uniqueitems>();
    let t = tables();
    assert_eq!(t.uniques.len(), typed_rows.len());
    for (i, (r, u)) in raw.iter().zip(&typed_rows).enumerate() {
        assert_eq!(r[0x32..0x34], [0, 0], "uniqueitems {i}");
        assert_eq!(t.uniques[i].rarity, u32::from(u.rarity), "uniqueitems {i}");
    }
}

/// `generation.md` §1.3 and `quality.md` §7.1: the itemtypes row indices
/// the specs name, by code.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn live_type_numbers() {
    let t = tables();
    let named: [(u16, &str); 24] = [
        (ty::WEAP, "weap"),
        (ty::ARMO, "armo"),
        (ty::MISC, "misc"),
        (ty::TORS, "tors"),
        (ty::HELM, "helm"),
        (ty::CHAR, "char"),
        (ty::BODY, "body"),
        (ty::PLAY, "play"),
        (ty::SCRO, "scro"),
        (ty::BOOK, "book"),
        (ty::GOLD, "gold"),
        (ty::ELIX, "elix"),
        (ty::JEWL, "jewl"),
        (ty::GEM, "gem "),
        (ty::RUNE, "rune"),
        (ty::STAF, "staf"),
        (ty::BOW, "bow "),
        (ty::XBOW, "xbow"),
        (ty::SCEP, "scep"),
        (ty::WAND, "wand"),
        (ty::SHIE, "shie"),
        (ty::BOOT, "boot"),
        (ty::GLOV, "glov"),
        (ty::BELT, "belt"),
    ];
    for (i, c) in named {
        assert_eq!(code(t.itemtypes[usize::from(i)].code), c, "itemtypes {i}");
    }
}

// ------------------------------------------------------------ sweeps

/// Checks a created item against the invariants of the quality, affix and
/// socket rules; returns a description of the first violation.
fn check_item(t: &ItemTables, it: &Item<Stats>, expansion: bool) -> Result<(), String> {
    let rec = &t.items[it.record];
    let ity = t.itype_of(it.record).expect("a type row");
    // quality.md §4 step 5: only qualities 1–9 finish.
    if !(1..=9).contains(&it.quality) {
        return Err(format!("quality {}", it.quality));
    }
    let fi = it.file_index;
    match it.quality {
        q::UNIQUE if fi == -1 => {
            // §8 step 4 (Edge case 6): only an items `unique` base stays
            // unique without a row.
            if rec.unique == 0 {
                return Err("unique without a row".into());
            }
        }
        q::UNIQUE => {
            // §8 step 6: an accepted row has the item's code.
            let u = t.uniques.get(fi as usize).ok_or("unique row")?;
            if u.code != rec.code {
                return Err(format!("unique row {fi} code {}", code(u.code)));
            }
        }
        q::SET => {
            // §9 step 1.
            let s = t.setitems.get(fi as usize).ok_or("set row")?;
            if s.item != rec.code || i32::from(s.lvl) > it.ilvl || s.set == 29 {
                return Err(format!("set row {fi}"));
            }
        }
        q::SUPERIOR => {
            // §7 step 1: the first 4 rows for throwable / nodurability.
            let n = if ity.throwable != 0 || rec.nodurability != 0 {
                4
            } else {
                t.qualityitems.len()
            };
            if !(0..n as i32).contains(&fi) {
                return Err(format!("superior row {fi} of {n}"));
            }
        }
        q::LOW if !(0..t.n_lowquality as i32).contains(&fi) => {
            return Err(format!("low quality row {fi}"));
        }
        _ => {}
    }
    // affixes.md §1, §3 step 4: slots hold ids of their part, the row
    // fits the item, and no two affixes share a group.
    // §1 rule 4: suffix slot 0 of a `scro` / `book` item holds a `books`
    // row index, not an affix id.
    let books_slot = [i32::from(ty::SCRO), i32::from(ty::BOOK)].contains(&i32::from(rec.type_));
    let mut groups = Vec::new();
    for (slots, lo, hi, part) in [
        (&it.prefix, t.n_suffix, t.first_auto(), 0),
        (&it.suffix, 0, t.n_suffix, 1),
    ] {
        for (k, &id) in slots.iter().enumerate() {
            if id == 0 || (books_slot && part == 1 && k == 0) {
                continue;
            }
            let i = usize::from(id) - 1;
            if !(lo..hi).contains(&i) {
                return Err(format!("affix {id} outside its part"));
            }
            let row = &t.magic[i];
            if !types_fit(t, it.record, row) {
                return Err(format!("affix {id} does not fit"));
            }
            if row.version >= 100 && it.format < 100 {
                return Err(format!("affix {id} version"));
            }
            groups.push(row.group);
        }
    }
    let n = groups.len();
    groups.sort_unstable();
    groups.dedup();
    if groups.len() != n {
        return Err(format!(
            "affix groups repeat: {:?} {:?}",
            it.prefix, it.suffix
        ));
    }
    if matches!(it.quality, q::RARE | q::CRAFTED) {
        let ns = t.n_rare_suffix;
        let p = usize::from(it.rare_prefix);
        let s = usize::from(it.rare_suffix);
        if (p != 0 && !(ns + 1..=t.rare.len()).contains(&p)) || s > ns {
            return Err(format!("rare names {p} {s}"));
        }
    }
    // quality.md §4 step 5.3: automagic only in expansion, never on set
    // or unique, from the automagic part, for an `auto prefix` base.
    if it.auto_affix != 0 {
        let i = usize::from(it.auto_affix) - 1;
        if !expansion
            || matches!(it.quality, q::SET | q::UNIQUE)
            || rec.auto_prefix == 0
            || !(t.first_auto()..t.magic.len()).contains(&i)
        {
            return Err(format!("auto affix {}", it.auto_affix));
        }
    }
    // generation.md §8: ethereal only in expansion games.
    if !expansion && it.flags & flag::ETHEREAL != 0 {
        return Err("ethereal in a classic game".into());
    }
    // generation.md §7.3 for qualities 1–3: at most min(w × h, 6) and the
    // base's `gemsockets`.
    if matches!(it.quality, q::LOW | q::NORMAL | q::SUPERIOR) {
        let sockets = it.stats.stat(stat::NUMSOCKETS, 0);
        let cap = (i32::from(rec.invwidth) * i32::from(rec.invheight)).min(6);
        if sockets > cap || sockets > i32::from(rec.gemsockets) {
            return Err(format!("{sockets} sockets"));
        }
    }
    Ok(())
}

/// Creates every item at requested qualities 0–9, four item levels, and
/// Normal/Hell difficulty in expansion and classic games. Classic games
/// must refuse expansion items; successful items must pass [`check_item`].
/// Crafted `Fatal::NullAffixGroup` results are counted and printed rather
/// than failed. This does not establish why those data combinations have
/// no usable affix group. Ear requests have no player unit, so their
/// `CreateError::NotPlayer` results are also counted separately.
/// [`check_item`] treats suffix slot 0 of scrolls and books as a books
/// index rather than an affix id.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_create_every_item_every_quality() {
    let t = tables();
    let mut failures = Vec::new();
    let mut created = 0usize;
    // Null affix-group refusals, by (expansion, difficulty, ilvl).
    let mut null_group: BTreeMap<(bool, u8, i32), usize> = BTreeMap::new();
    let mut ears = 0usize;
    let mut s = 0u32;
    for (i, rec) in t.items.iter().enumerate() {
        for expansion in [true, false] {
            for difficulty in [0u8, 2] {
                for quality in 0..=9u8 {
                    for ilvl in [1, 30, 60, 99] {
                        s = s.wrapping_add(0x9E37_79B9);
                        let mut game = Game::new(s, difficulty, expansion);
                        let mut rq = request(&game, i, ilvl, quality);
                        let at = format!(
                            "item {i} {} exp {expansion} d {difficulty} q {quality} ilvl {ilvl}",
                            code(rec.code)
                        );
                        match create_item(t, &mut game, &mut rq, false, Stats::default(), 0) {
                            Err(CreateError::Classic) if !expansion && rec.version >= 100 => {}
                            // affixes.md §8 step 3.2, Edge case 3: only the
                            // crafted routine raises it.
                            Err(CreateError::Fatal(Fatal::NullAffixGroup))
                                if quality == q::CRAFTED =>
                            {
                                *null_group.entry((expansion, difficulty, ilvl)).or_default() += 1;
                            }
                            // §9 step 5: an unforced ear needs a player
                            // request unit; the sweep's request has none.
                            Err(CreateError::NotPlayer) if rec.type_ == ty::PLAY as _ => {
                                ears += 1;
                            }
                            Err(e) => failures.push(format!("{at}: {e}")),
                            Ok(_) if !expansion && rec.version >= 100 => {
                                failures.push(format!("{at}: created in a classic game"))
                            }
                            Ok(c) => {
                                created += 1;
                                if let Err(e) = check_item(t, &c.item, expansion) {
                                    failures.push(format!("{at}: {e}"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "crafted requests ending in affixes.md Edge case 3 (expansion, difficulty, ilvl) → count: {} total {null_group:?}",
        null_group.values().sum::<usize>()
    );
    eprintln!("ear requests without a player (§9 step 5): {ears}");
    assert!(created > 0);
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
}

/// `quality.md` §8 step 2 on every uniqueitems row whose `code` is an
/// item: a forced request with that index gives a unique with that file
/// index (or normal quality when the base's itemtype has `normal`, §4
/// step 3.4).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_forced_every_unique() {
    let t = tables();
    let mut tried = 0;
    for (r, u) in t.uniques.iter().enumerate() {
        let Some(i) = t.items.iter().position(|it| it.code == u.code) else {
            continue;
        };
        let mut game = Game::new(r as u32 + 1, 2, true);
        let mut rq = ItemRequest {
            force: true,
            index: r as i32,
            seed: 0x1234_5678 ^ r as u32,
            item_seed: 0x8765_4321 ^ r as u32,
            ..request(&game, i, 99, q::UNIQUE)
        };
        let c = create_item(t, &mut game, &mut rq, false, Stats::default(), 0)
            .unwrap_or_else(|e| panic!("unique {r}: {e}"));
        tried += 1;
        if t.itype_of(i).unwrap().normal != 0 {
            assert_eq!(c.item.quality, q::NORMAL, "unique {r}");
        } else {
            assert_eq!(
                (c.item.quality, c.item.file_index),
                (q::UNIQUE, r as i32),
                "unique {r}"
            );
        }
    }
    assert!(tried > 0);
}

/// `quality.md` §9 steps 1–2 on every setitems row whose `item` is an
/// item: a set request preferring that row (index = row + 1, item level
/// 99, `hellbovine` request flag so set 29 qualifies) picks it, unless
/// §4 step 3 overrides the quality (itemtype `normal`, items `unique`).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_preferred_every_set_item() {
    let t = tables();
    let mut tried = 0;
    for (r, s) in t.setitems.iter().enumerate() {
        let Some(i) = t.items.iter().position(|it| it.code == s.item) else {
            continue;
        };
        let mut game = Game::new(r as u32 + 1, 2, true);
        let mut rq = ItemRequest {
            index: r as i32 + 1,
            flags2: req::HELLBOVINE,
            ..request(&game, i, 99, q::SET)
        };
        let c = create_item(t, &mut game, &mut rq, false, Stats::default(), 0)
            .unwrap_or_else(|e| panic!("set item {r}: {e}"));
        tried += 1;
        if t.itype_of(i).unwrap().normal != 0 {
            assert_eq!(c.item.quality, q::NORMAL, "set item {r}");
        } else if t.items[i].unique != 0 {
            assert_eq!(c.item.quality, q::UNIQUE, "set item {r}");
        } else {
            assert_eq!(
                (c.item.quality, c.item.file_index),
                (q::SET, r as i32),
                "set item {r}"
            );
        }
    }
    assert!(tried > 0);
}

/// `affixes.md` §3 on every item (expansion format, quality 4) for the
/// prefix, suffix and (when the base has one) automagic part, at every
/// item level 1–99, forced past the coin: a pick is an id of the part
/// that passes the step 4 tests (spawnable, version, level window,
/// fit, group, frequency, class), and no pick happens only when no row
/// passes them (step 5).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn sweep_every_affix_pick() {
    let t = tables();
    let mut failures = Vec::new();
    for (i, rec) in t.items.iter().enumerate() {
        let class = t.itype_of(i).map_or(7, |it| it.class.min(7));
        let parts: Vec<(&str, bool, i32, usize, usize)> = {
            let mut v = vec![
                ("prefix", true, 0, t.n_suffix, t.first_auto()),
                ("suffix", false, 0, 0, t.n_suffix),
            ];
            if rec.auto_prefix != 0 {
                let g = i32::from(rec.auto_prefix);
                v.push(("auto", false, g, t.first_auto(), t.magic.len()));
            }
            v
        };
        let mut seen = BTreeSet::new();
        for ilvl in 1..=99 {
            let mut item = Item::new(i, 101, Stats::default());
            item.ilvl = ilvl;
            item.quality = q::MAGIC;
            item.item_seed = Seed::init_low(ilvl as u32);
            let a = alvl(ilvl, i32::from(rec.level), i32::from(rec.magic_lvl));
            let sockets_ok = rec.hasinv != 0 && max_sockets(t, &item) != 0;
            // The picks depend on the item level only through these two.
            if !seen.insert((a, sockets_ok)) {
                continue;
            }
            let passes = |row: &AffixRec, g: i32| {
                let numsockets = row.mods[0].code >= 0
                    && t.properties
                        .get(row.mods[0].code as usize)
                        .is_none_or(|p| p.slots[0].stat == stat::NUMSOCKETS);
                row.spawnable != 0
                    && row.level <= a
                    && (row.maxlevel == 0 || row.maxlevel >= a)
                    && (sockets_ok || !numsockets)
                    && types_fit(t, i, row)
                    && (g == 0 || row.group == g)
                    && row.frequency != 0
                    && (row.classspecific == 0xFF || class == 7 || class == row.classspecific)
            };
            for &(pn, prefix, g, lo, hi) in &parts {
                let any = t.magic[lo..hi].iter().any(|r| passes(r, g));
                let id = roll_affix(t, &mut item, true, true, false, prefix, 0, g);
                let at = format!("item {i} {} ilvl {ilvl} {pn}", code(rec.code));
                if id == 0 {
                    if any {
                        failures.push(format!("{at}: no pick, but a row passes"));
                    }
                    continue;
                }
                let k = usize::from(id) - 1;
                if !(lo..hi).contains(&k) || !passes(&t.magic[k], g) {
                    failures.push(format!("{at}: picked {id}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
}
