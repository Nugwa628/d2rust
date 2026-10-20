//! Cube tests on a fake world: the op table against `cube-ops.tsv`, record
//! decoding, and the transmute vectors (V1-V22). Synthetic recipes stand in
//! for the game's records.
use std::collections::BTreeMap;

use super::*;
use crate::world::{tsv_num, tsv_rows, TsvError};

const OPS_TSV: &str = include_str!("../../../../../tables/world/cube-ops.tsv");
const OPS_HEADER: &[&str] = &["op", "scope", "subject", "source", "pass_if", "address"];

// ------------------------------------------------------- cube-ops.tsv

/// (scope, source, pass_if) that [`op_info`] implies, in the TSV's words.
fn describe(op: u8) -> (&'static str, &'static str, &'static str) {
    let (scope, test) = op_info(op);
    let scope = match scope {
        OpScope::Recipe => "recipe",
        OpScope::Input0 => "input0",
        OpScope::Inputs => "inputs",
    };
    let (source, pass) = match test {
        OpTest::Always => ("-", "always"),
        OpTest::DayOfMonth => ("local day of month", "param <= day <= value"),
        OpTest::DayOfWeek => ("local day of week + 1", "day == value"),
        OpTest::FileIndexNot => ("file index", "index != value"),
        OpTest::QuestDifficulty => ("stat 356", "stat >= game difficulty"),
        OpTest::Stat(read, cmp) => (
            match read {
                StatRead::Value => "stat value",
                StatRead::Base => "base stat",
                StatRead::Bonus => "stat bonus",
            },
            match cmp {
                Cmp::Ge => "stat >= t",
                Cmp::Le => "stat <= t",
                Cmp::Ne => "stat != t",
                Cmp::Eq => "stat == t",
            },
        ),
    };
    (scope, source, pass)
}

/// One line per TSV row that disagrees with [`op_info`].
fn check_ops(text: &str) -> Result<Vec<String>, TsvError> {
    let mut out = Vec::new();
    let mut ops = Vec::new();
    for (line, c) in tsv_rows("cube-ops.tsv", text, OPS_HEADER)? {
        let op = tsv_num("cube-ops.tsv", line, "op", c[0])?;
        ops.push(op);
        let (scope, source, pass) = describe(op as u8);
        if c[1] != scope || !c[3].starts_with(source) || !c[4].starts_with(pass) {
            out.push(format!(
                "op {op}: tsv {:?}, code {:?}",
                &c[1..5],
                (scope, source, pass)
            ));
        }
    }
    if ops != (0..=29).collect::<Vec<_>>() {
        out.push(format!("ops {ops:?}"));
    }
    Ok(out)
}

#[test]
fn ops_match_tsv() {
    assert_eq!(check_ops(OPS_TSV).unwrap(), Vec::<String>::new());
    // Every op ≥ 29 passes at recipe scope.
    assert_eq!(op_info(200), (OpScope::Recipe, OpTest::Always));
}

/// M08: a changed cell is reported for exactly that op.
#[test]
fn ops_check_catches_perturbations() {
    let bad = OPS_TSV.replacen("stat >= t\t0x005666FB", "stat <= t\t0x005666FB", 1);
    let errs = check_ops(&bad).unwrap();
    assert_eq!(errs.len(), 1);
    assert!(errs[0].starts_with("op 3:"), "{errs:?}");
    let bad = OPS_TSV.replacen("19\tinput0", "19\trecipe", 1);
    let errs = check_ops(&bad).unwrap();
    assert!(errs.len() == 1 && errs[0].starts_with("op 19:"), "{errs:?}");
    let bad = OPS_TSV.replacen("\n27\t", "\n31\t", 1);
    assert!(!check_ops(&bad).unwrap().is_empty());
}

// ------------------------------------------------------------- decode

#[test]
fn recipe_decode_reads_slot_bytes() {
    let mut r = vec![0u8; Cubemain::SIZE];
    r[0] = 1; // enabled
    r[3] = 0xFF; // class
    r[4] = 28; // op
    r[8..12].copy_from_slice(&(-5i32).to_le_bytes());
    r[16] = 3; // numinputs
    r[18..20].copy_from_slice(&100u16.to_le_bytes());
    // input 2 (slot 1): flags 0x0081 item 7 quality 4 qty 2
    r[28..30].copy_from_slice(&0x0081u16.to_le_bytes());
    r[30..32].copy_from_slice(&7u16.to_le_bytes());
    r[34] = 4;
    r[35] = 2;
    // output b: kind 0xFD, flags rep, item 9, pre 3, suf 4
    let o = 76 + 84;
    r[o..o + 2].copy_from_slice(&0x0200u16.to_le_bytes());
    r[o + 2..o + 4].copy_from_slice(&9u16.to_le_bytes());
    r[o + 8] = 0xFD;
    r[o + 12..o + 14].copy_from_slice(&3u16.to_le_bytes());
    r[o + 18..o + 20].copy_from_slice(&4u16.to_le_bytes());
    r[170] = 40; // b plvl
                 // b mod 1 at 184: property -1, chance 50
    r[184..188].copy_from_slice(&u32::MAX.to_le_bytes());
    r[194] = 50;
    let rec = Recipe::decode(&r);
    assert_eq!(
        (rec.enabled, rec.class, rec.op, rec.param),
        (1, 0xFF, 28, -5)
    );
    assert_eq!((rec.numinputs, rec.version), (3, 100));
    assert_eq!(
        rec.inputs[1],
        InputSlot {
            flags: 0x81,
            item: 7,
            special: 0,
            quality: 4,
            quantity: 2
        }
    );
    let b = rec.outputs[1];
    assert_eq!((b.kind, b.flags, b.item, b.plvl), (0xFD, 0x200, 9, 40));
    assert_eq!((b.pre, b.suf), ([3, 0, 0], [4, 0, 0]));
    assert_eq!((b.mods[0].property, b.mods[0].chance), (-1, 50));
    let t = BinTable {
        name: "cubemain".into(),
        source: "t".into(),
        count: 1,
        record_size: Cubemain::SIZE,
        records: r.clone(),
    };
    assert_eq!(recipes(&t).unwrap(), vec![rec]);
    let t = BinTable {
        name: "belts".into(),
        ..t
    };
    assert!(recipes(&t).is_err());
}

#[test]
fn ratio_rule() {
    assert_eq!(ratio(40, 75), 30);
    assert_eq!(ratio(-7, 50), -3); // toward zero
    assert_eq!(ratio(0x10_0000, 200), 0x10_0000 * 200 / 100);
    assert_eq!(ratio(0x10_0001, 200), (0x10_0001 / 100) * 200);
}

// ------------------------------------------------------- fake world

pub(super) const P: UnitId = UnitId(1);

#[derive(Clone, Debug, Default)]
pub(super) struct Item {
    pub(super) guid: u32,
    pub(super) class: Option<u32>,
    pub(super) page: u8,
    pub(super) mode: u8,
    pub(super) quality: u8,
    pub(super) file_index: u32,
    pub(super) level: i32,
    pub(super) flags: u32,
    pub(super) sockets: i32,
    pub(super) max_sockets: i32,
    pub(super) stats: BTreeMap<u16, i32>,
    pub(super) seed: Seed,
    pub(super) socketed: Vec<UnitId>,
}

#[derive(Default)]
pub(super) struct Fake {
    pub(super) expansion: bool,
    pub(super) game_type: u8,
    pub(super) ladder: bool,
    pub(super) difficulty: u8,
    pub(super) date: (u8, u8),
    pub(super) date_reads: std::cell::Cell<u32>,
    pub(super) seed: Seed,
    pub(super) class: u8,
    pub(super) player_stats: BTreeMap<u16, i32>,
    pub(super) interaction: Option<(u8, u32)>,
    pub(super) stash: bool,
    pub(super) trading: bool,
    pub(super) inventory: Vec<UnitId>,
    pub(super) items: BTreeMap<UnitId, Item>,
    pub(super) next: u32,
    /// Item types: (class, type) pairs that match.
    pub(super) types: Vec<(u32, u16)>,
    pub(super) uniques: BTreeMap<u32, bool>,
    pub(super) place_ok: bool,
    pub(super) cow: bool,
    pub(super) tempered: (u16, u16),
    pub(super) requests: Vec<ItemRequest>,
    pub(super) log: Vec<String>,
    pub(super) sent: Vec<Vec<u8>>,
    /// The rules the fake game runs under (D2RUST's by default).
    pub(super) rules: crate::rules::Ruleset,
}

impl Fake {
    pub(super) fn new() -> Self {
        Fake {
            expansion: true,
            place_ok: true,
            next: 100,
            date: (15, 3),
            ..Fake::default()
        }
    }

    pub(super) fn add(&mut self, class: u32, quality: u8) -> UnitId {
        let id = UnitId(self.next);
        self.next += 1;
        self.items.insert(
            id,
            Item {
                guid: id.0,
                class: Some(class),
                page: CUBE_PAGE,
                quality,
                level: 10,
                ..Item::default()
            },
        );
        self.inventory.push(id);
        id
    }

    pub(super) fn new_item(&mut self, mut it: Item) -> UnitId {
        let id = UnitId(self.next);
        self.next += 1;
        it.guid = id.0;
        self.items.insert(id, it);
        id
    }

    pub(super) fn it(&mut self, id: UnitId) -> &mut Item {
        self.items.get_mut(&id).unwrap()
    }

    pub(super) fn cube_contents(&self) -> Vec<Option<u32>> {
        self.inventory
            .iter()
            .filter(|i| self.items[i].page == CUBE_PAGE)
            .map(|i| self.items[i].class)
            .collect()
    }
}

impl CubeWorld for Fake {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn game_type(&self) -> u8 {
        self.game_type
    }
    fn ladder(&self) -> bool {
        self.ladder
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn item_format(&self) -> u16 {
        if self.expansion {
            101
        } else {
            2
        }
    }
    fn local_date(&self) -> (u8, u8) {
        self.date_reads.set(self.date_reads.get() + 1);
        self.date
    }
    fn game_seed(&mut self) -> &mut Seed {
        &mut self.seed
    }
    fn player_class(&self, _: UnitId) -> u8 {
        self.class
    }
    fn stat(&self, unit: UnitId, read: StatRead, stat: u16) -> i32 {
        let stats = if unit == P {
            &self.player_stats
        } else {
            &self.items[&unit].stats
        };
        // The fake keeps one value; Bonus reads as value − 1 so the three
        // readers are distinguishable.
        let v = stats.get(&stat).copied().unwrap_or(0);
        match read {
            StatRead::Bonus => v - 1,
            _ => v,
        }
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.log.push(format!("stat {} {stat} {value}", unit.0));
        self.it(unit).stats.insert(stat, value);
    }
    fn attach_sound(&mut self, _: UnitId, event: u8) {
        self.log.push(format!("sound {event}"));
    }
    fn send(&mut self, _: UnitId, msg: &[u8]) {
        self.sent.push(msg.to_vec());
    }
    fn interaction(&self, _: UnitId) -> Option<(u8, u32)> {
        self.interaction
    }
    fn set_interaction(&mut self, _: UnitId, ty: u8, guid: u32) {
        if self.interaction.is_none() {
            self.interaction = Some((ty, guid));
        }
    }
    fn reset_interaction(&mut self, _: UnitId) {
        self.interaction = None;
        self.log.push("reset".into());
    }
    fn inventory_pass(&mut self, _: UnitId) {
        self.log.push("pass".into());
    }
    fn interacting_with_stash(&self, _: UnitId) -> bool {
        self.stash
    }
    fn trading(&self, _: UnitId) -> bool {
        self.trading
    }
    fn inventory(&self, _: UnitId) -> Vec<UnitId> {
        self.inventory.clone()
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.items
            .contains_key(&UnitId(guid))
            .then_some(UnitId(guid))
    }
    fn item_guid(&self, item: UnitId) -> u32 {
        self.items[&item].guid
    }
    fn item_page(&self, item: UnitId) -> u8 {
        self.items[&item].page
    }
    fn set_item_page(&mut self, item: UnitId, page: u8) {
        self.it(item).page = page;
    }
    fn item_mode(&self, item: UnitId) -> u8 {
        self.items[&item].mode
    }
    fn set_item_mode(&mut self, item: UnitId, mode: u8) {
        self.it(item).mode = mode;
    }
    fn item_class(&self, item: UnitId) -> Option<u32> {
        self.items[&item].class
    }
    fn set_item_class(&mut self, item: UnitId, class: u32) {
        self.it(item).class = Some(class);
    }
    fn class_is_type(&self, class: u32, ty: u16) -> bool {
        self.types.contains(&(class, ty))
    }
    fn item_quality(&self, item: UnitId) -> u8 {
        self.items[&item].quality
    }
    fn item_file_index(&self, item: UnitId) -> u32 {
        self.items[&item].file_index
    }
    fn item_level(&self, item: UnitId) -> i32 {
        self.items[&item].level
    }
    fn set_item_level(&mut self, item: UnitId, level: i32) {
        self.it(item).level = level;
    }
    fn item_flags(&self, item: UnitId) -> u32 {
        self.items[&item].flags
    }
    fn set_item_flag(&mut self, item: UnitId, flag: u32) {
        self.it(item).flags |= flag;
    }
    fn item_sockets(&self, item: UnitId) -> i32 {
        self.items[&item].sockets
    }
    fn max_sockets(&self, item: UnitId) -> i32 {
        self.items[&item].max_sockets
    }
    fn add_sockets(&mut self, item: UnitId, n: i32) {
        self.log.push(format!("sockets {n}"));
        self.it(item).sockets += n;
    }
    fn item_seed(&mut self, item: UnitId) -> &mut Seed {
        &mut self.it(item).seed
    }
    fn socketed(&self, item: UnitId) -> Vec<UnitId> {
        self.items[&item].socketed.clone()
    }
    fn duplicate(&mut self, item: UnitId, fillers: bool) -> Option<UnitId> {
        self.log.push(format!("dup {} {fillers}", item.0));
        let mut it = self.items[&item].clone();
        it.page = 0;
        Some(self.new_item(it))
    }
    fn item_init(&mut self, item: UnitId) -> Option<UnitId> {
        self.log.push(format!("init {}", item.0));
        Some(item)
    }
    fn create_item(&mut self, r: &ItemRequest) -> Option<UnitId> {
        self.requests.push(*r);
        let it = Item {
            class: Some(r.class),
            quality: if r.quality == 0 { 2 } else { r.quality },
            level: r.level,
            max_sockets: 6,
            seed: Seed::init_low(1234),
            ..Item::default()
        };
        if r.quality == 7 {
            self.uniques.insert(u32::from(r.item_index) - 1, true);
        }
        Some(self.new_item(it))
    }
    fn tempered_affix(&mut self, _: UnitId, prefix: bool) -> u16 {
        if prefix {
            self.tempered.0
        } else {
            self.tempered.1
        }
    }
    fn set_tempered(&mut self, item: UnitId, p: u16, s: u16) {
        self.log.push(format!("tempered {p} {s}"));
        self.it(item).quality = 9;
    }
    fn unique_found(&self, index: u32) -> bool {
        self.uniques.get(&index).copied().unwrap_or(false)
    }
    fn set_unique_found(&mut self, index: u32, found: bool) {
        self.uniques.insert(index, found);
    }
    fn drop_runeword_stats(&mut self, item: UnitId) {
        self.log.push(format!("drop rw {}", item.0));
    }
    fn add_craft_property(&mut self, _: UnitId, p: &CraftProperty) {
        self.log.push(format!(
            "prop {} {} {} {}",
            p.property, p.param, p.min, p.max
        ));
    }
    fn repair(&mut self, _: UnitId) {
        self.log.push("repair".into());
    }
    fn recharge(&mut self, _: UnitId) {
        self.log.push("recharge".into());
    }
    fn place(&mut self, _: UnitId, item: UnitId) -> bool {
        if self.place_ok {
            if !self.inventory.contains(&item) {
                self.inventory.push(item);
            }
            self.log.push(format!("place {}", item.0));
        }
        self.place_ok
    }
    fn free_item(&mut self, item: UnitId) {
        self.log.push(format!("free {}", item.0));
        self.items.remove(&item);
    }
    fn remove_cube_item(&mut self, _: UnitId, item: UnitId) {
        self.log.push(format!("remove {}", item.0));
        self.inventory.retain(|&i| i != item);
        self.items.remove(&item);
    }
    fn targeting_reset(&mut self, _: UnitId) {
        self.log.push("targeting".into());
    }
    fn put_item_check(&self, _: UnitId, item: u32) -> u32 {
        if self.items.contains_key(&UnitId(item)) {
            0
        } else {
            1
        }
    }
    fn cube_check(&self, _: UnitId, cube: u32) -> bool {
        self.items.contains_key(&UnitId(cube))
    }
    fn quest_item_hook(&mut self, _: UnitId, _: UnitId, code: [u8; 4]) {
        self.log
            .push(format!("hook {}", String::from_utf8_lossy(&code)));
    }
    fn cow_portal(&mut self, _: UnitId) -> bool {
        self.log.push("cow".into());
        if !self.cow {
            self.log.push(format!("sound {SOUND_COW_REFUSED}"));
        }
        self.cow
    }
}

// ----------------------------------------------------- item table

pub(super) const HAX: u32 = 0;
pub(super) const FHL: u32 = 1;
pub(super) const XHL: u32 = 2;
pub(super) const UHL: u32 = 3;
pub(super) const GCV: u32 = 4;
pub(super) const GFV: u32 = 5;
pub(super) const AQV: u32 = 6;
pub(super) const CQV: u32 = 7;
pub(super) const RIN: u32 = 8;
pub(super) const JEW: u32 = 9;
pub(super) const BOX: u32 = 10;
pub(super) const MSF: u32 = 11;
pub(super) const VIP: u32 = 12;
pub(super) const HST: u32 = 13;
pub(super) const HLM: u32 = 14;
pub(super) const GCR: u32 = 15;
pub(super) const LEG: u32 = 16;
pub(super) const TBK: u32 = 17;
pub(super) const PK1: u32 = 18;
pub(super) const PK2: u32 = 19;
pub(super) const PK3: u32 = 20;
pub(super) const AMU: u32 = 21;

pub(super) fn rec(code: &[u8; 4]) -> ItemRecord {
    ItemRecord {
        code: *code,
        level: 1,
        spawnable: 1,
        ..ItemRecord::default()
    }
}

pub(super) fn items() -> Vec<ItemRecord> {
    let tiered = |c: &[u8; 4]| ItemRecord {
        normcode: *b"fhl ",
        ubercode: *b"xhl ",
        ultracode: *b"uhl ",
        version: if c == b"fhl " { 0 } else { 100 },
        ..rec(c)
    };
    let mut v = vec![
        ItemRecord {
            normcode: *b"hax ",
            ubercode: *b"9ha ",
            ultracode: *b"7ha ",
            ..rec(b"hax ")
        },
        tiered(b"fhl "),
        tiered(b"xhl "),
        tiered(b"uhl "),
        rec(b"gcv "),
        rec(b"gfv "),
        ItemRecord {
            stackable: 1,
            maxstack: 350,
            ..rec(b"aqv ")
        },
        ItemRecord {
            stackable: 1,
            maxstack: 350,
            ..rec(b"cqv ")
        },
        rec(b"rin "),
        rec(b"jew "),
        rec(b"box "),
        ItemRecord {
            quest: 10,
            questdiffcheck: 1,
            ..rec(b"msf ")
        },
        ItemRecord {
            quest: 10,
            questdiffcheck: 1,
            ..rec(b"vip ")
        },
        ItemRecord {
            quest: 10,
            ..rec(b"hst ")
        },
        rec(b"hlm "),
        rec(b"gcr "),
        rec(b"leg "),
        rec(b"tbk "),
        rec(b"pk1 "),
        rec(b"pk2 "),
        rec(b"pk3 "),
        rec(b"amu "),
    ];
    v[AMU as usize].level = 30;
    v
}

pub(super) fn data(recipes: Vec<Recipe>) -> CubeData {
    CubeData {
        recipes,
        items: items(),
        valshift: vec![0; 10],
        max_level: 99,
    }
}

pub(super) fn input(flags: u16, item: u32, qty: u8) -> InputSlot {
    InputSlot {
        flags,
        item: item as u16,
        quantity: qty,
        ..InputSlot::default()
    }
}

pub(super) fn code_in(item: u32) -> InputSlot {
    input(input_flags::USEANY, item, 0)
}

pub(super) fn out(kind: u8, item: u32) -> OutputSlot {
    OutputSlot {
        kind,
        item: item as u16,
        mods: [CraftMod {
            property: -1,
            ..CraftMod::default()
        }; 5],
        ..OutputSlot::default()
    }
}

pub(super) fn recipe(n: u8, ins: &[InputSlot], a: OutputSlot) -> Recipe {
    let mut inputs = [InputSlot::default(); 7];
    inputs[..ins.len()].copy_from_slice(ins);
    Recipe {
        enabled: 1,
        class: 0xFF,
        numinputs: n,
        inputs,
        outputs: [a, OutputSlot::default(), OutputSlot::default()],
        ..Recipe::default()
    }
}

/// Stand-ins for the live records the vectors name.
pub(super) fn gem_recipe() -> Recipe {
    // record 23: "gcv,qty=3" → gfv
    recipe(3, &[input(1, GCV, 3)], out(kind::ITEMCODE, GFV))
}

pub(super) fn arrows_recipe() -> Recipe {
    // record 21: "aqv,qty=2" → cqv
    recipe(2, &[input(1, AQV, 2)], out(kind::ITEMCODE, CQV))
}

pub(super) fn ring_recipe() -> Recipe {
    // record 13: "rin,mag,qty=3" → usetype rin, plvl 75
    let mut i = input(1, RIN, 3);
    i.quality = 4;
    let mut a = out(kind::USETYPE, 0);
    a.plvl = 75;
    recipe(3, &[i], a)
}

fn pairs(f: &Fake) -> Vec<String> {
    f.log.clone()
}

// ------------------------------------------------------------ vectors

#[test]
fn v1_v2_gems() {
    let d = data(vec![ring_recipe(), arrows_recipe(), gem_recipe()]);
    let mut f = Fake::new();
    let g: Vec<_> = (0..3).map(|_| f.add(GCV, 2)).collect();
    let t = d.transmute(&mut f, P);
    assert_eq!(t.record, Some(2));
    assert!(t.committed);
    let r = f.requests[0];
    assert_eq!(
        (r.class, r.level, r.quality, r.item_index, r.flags2),
        (GFV, 1, 0, 0, 0x0A)
    );
    assert_eq!((r.spawn_type, r.init_flags, r.item_format), (4, 1, 101));
    let x = t.outputs[0];
    assert_eq!(
        pairs(&f),
        [
            format!("remove {}", g[0].0),
            format!("remove {}", g[1].0),
            format!("remove {}", g[2].0),
            "sound 4".into(),
            format!("place {}", x.0),
        ]
    );
    assert_eq!(f.cube_contents(), [Some(GFV)]);
    assert!(f.items[&x].flags & item_flags::IDENTIFIED != 0);
    // V2: + gcr → n = 4, nothing.
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    f.add(GCR, 2);
    assert_eq!(d.transmute(&mut f, P), Transmute::default());
    assert!(f.log.is_empty() && f.sent.is_empty());
}

#[test]
fn v3_arrows_and_numinputs() {
    let d = data(vec![arrows_recipe()]);
    let mut f = Fake::new();
    f.add(AQV, 2);
    f.add(AQV, 2);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(AQV, 2);
    }
    assert_eq!(d.transmute(&mut f, P).record, None);
}

#[test]
fn v4_slot_marks_every_match() {
    let r = recipe(
        3,
        &[input(1, AQV, 2), code_in(AQV)],
        out(kind::ITEMCODE, CQV),
    );
    let d = data(vec![r]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(AQV, 2);
    }
    assert_eq!(d.transmute(&mut f, P).record, None);
}

#[test]
fn v5_quantity_is_exact_without_stackables() {
    let d = data(vec![ring_recipe()]);
    let mut f = Fake::new();
    f.add(RIN, 4);
    f.add(RIN, 4);
    f.add(RIN, 6);
    assert_eq!(d.transmute(&mut f, P).record, None);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(RIN, 4);
    }
    f.player_stats.insert(STAT_LEVEL, 40);
    let t = d.transmute(&mut f, P);
    assert_eq!(t.record, Some(0));
    // V8: plvl 75, character level 40 → 30; usetype class.
    assert_eq!((f.requests[0].level, f.requests[0].class), (30, RIN));
}

#[test]
fn v6_upgrade_inputs() {
    // record 64: "fhl,mag,upg" + jew; version 100.
    let mut helm = input(1 | input_flags::UPG, FHL, 0);
    helm.quality = 4;
    let mut r = recipe(2, &[helm, code_in(JEW)], out(kind::USEITEM, 0));
    r.version = 100;
    let d = data(vec![r]);
    for (class, ok) in [(FHL, true), (XHL, true), (UHL, true), (HLM, false)] {
        let mut f = Fake::new();
        f.add(class, 4);
        f.add(JEW, 2);
        assert_eq!(d.transmute(&mut f, P).record.is_some(), ok, "class {class}");
    }
    // `xhl,upg` accepts xhl and uhl, not fhl.
    let mut r2 = r_clone_with_slot(&d.recipes[0], XHL);
    r2.version = 0;
    let d2 = data(vec![r2]);
    for (class, ok) in [(FHL, false), (XHL, true), (UHL, true)] {
        let mut f = Fake::new();
        f.add(class, 4);
        f.add(JEW, 2);
        assert_eq!(
            d2.transmute(&mut f, P).record.is_some(),
            ok,
            "class {class}"
        );
    }
    // Classic game: version 100 skipped.
    let mut f = Fake::new();
    f.expansion = false;
    f.add(XHL, 4);
    f.add(JEW, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
}

fn r_clone_with_slot(r: &Recipe, item: u32) -> Recipe {
    let mut r = r.clone();
    r.inputs[0].item = item as u16;
    r
}

#[test]
fn v7_to_v11_levels() {
    let d = data(vec![]);
    let f = |stats: i32| {
        let mut f = Fake::new();
        f.player_stats.insert(STAT_LEVEL, stats);
        f
    };
    let cap = |level| Capture {
        item: None,
        class: 0,
        level,
    };
    let lv = |lvl, plvl, ilvl| OutputSlot {
        lvl,
        plvl,
        ilvl,
        ..OutputSlot::default()
    };
    assert_eq!(d.output_level(&f(1), P, &lv(30, 0, 60), &cap(50)), 30);
    assert_eq!(d.output_level(&f(40), P, &lv(0, 75, 0), &cap(1)), 30);
    assert_eq!(d.output_level(&f(50), P, &lv(0, 40, 40), &cap(80)), 52);
    assert_eq!(d.output_level(&f(99), P, &lv(0, 66, 66), &cap(99)), 99);
    assert_eq!(d.output_level(&f(1), P, &lv(0, 0, 100), &cap(1)), 1);
    assert_eq!(d.output_level(&f(1), P, &lv(0, 0, 0), &cap(1)), 1);
}

#[test]
fn v11_item_level_stored_back() {
    // ilvl 100 on an item of level 0: stored back as 1, L = 1.
    let a = out(kind::USETYPE, 0);
    let d = data(vec![recipe(1, &[code_in(RIN)], a)]);
    let mut f = Fake::new();
    let ring = f.add(RIN, 4);
    f.it(ring).level = 0;
    d.transmute(&mut f, P);
    assert_eq!(f.requests[0].level, 1);
}

fn mod_recipe(chance: u8) -> Recipe {
    let mut a = out(kind::ITEMCODE, AMU);
    a.mods[0] = CraftMod {
        property: 7,
        param: 0xFFFF,
        min: 5,
        max: 10,
        chance,
    };
    recipe(1, &[code_in(RIN)], a)
}

#[test]
fn v12_v13_mod_chance() {
    // No draw for chance 0 and ≥ 100.
    for c in [0, 100, 200] {
        let d = data(vec![mod_recipe(c)]);
        let mut f = Fake::new();
        f.add(RIN, 2);
        let t = d.transmute(&mut f, P);
        assert!(f.log.contains(&"prop 7 -1 5 10".to_string()));
        assert_eq!(f.items[&t.outputs[0]].seed, Seed::init_low(1234));
    }
    // Chance c passes when lo' mod 100 ≤ c.
    let mut s = Seed::init_low(1234);
    let lo = s.step();
    let roll = (lo % 100) as u8;
    for (c, applied) in [(roll, true), (roll.saturating_sub(1), roll == 0)] {
        if c == 0 || c >= 100 {
            continue;
        }
        let d = data(vec![mod_recipe(c)]);
        let mut f = Fake::new();
        f.add(RIN, 2);
        let t = d.transmute(&mut f, P);
        assert_eq!(
            f.log.iter().any(|l| l.starts_with("prop")),
            applied,
            "c {c}"
        );
        assert_eq!(f.items[&t.outputs[0]].seed, s);
    }
}

#[test]
fn v14_sockets() {
    let sock = |quality: u8, max: i32, have: i32| {
        let mut a = out(kind::USEITEM, 0);
        a.flags = output_flags::SOCK;
        a.quantity = 6;
        let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
        let mut f = Fake::new();
        let h = f.add(HAX, quality);
        f.it(h).max_sockets = max;
        f.it(h).sockets = have;
        d.transmute(&mut f, P);
        f.log.iter().find_map(|l| {
            l.strip_prefix("sockets ")
                .map(|n| n.parse::<i32>().unwrap())
        })
    };
    assert_eq!(sock(4, 4, 0), Some(3));
    assert_eq!(sock(7, 6, 0), Some(1));
    assert_eq!(sock(2, 6, 0), Some(6));
    assert_eq!(sock(2, 6, 2), None);
    // Input test 9: usetype/useitem with `sock` needs max sockets ≠ 0.
    assert_eq!(sock(2, 0, 0), None);
}

/// A seed whose first two rolls are (n1 → r1, n2 → r2).
fn seed_for(n1: u32, r1: u32, n2: u32, r2: u32) -> Seed {
    (1..100_000)
        .map(Seed::init_low)
        .find(|s| {
            let mut s = *s;
            s.step() % n1 == r1 && (n2 == 0 || s.step() % n2 == r2)
        })
        .expect("seed")
}

#[test]
fn v15_v16_type_pick() {
    let mut d = data(vec![]);
    d.items.truncate(5);
    let mut f = Fake::new();
    f.types = vec![(1, 9), (2, 9), (4, 9)];
    f.seed = seed_for(5, 3, 2, 1);
    assert_eq!(d.type_pick(&mut f, 9, 99), 1);
    // Item 2 is never examined: with first roll 3 and only {2}, none.
    f.types = vec![(2, 9)];
    f.seed = seed_for(5, 3, 0, 0);
    assert_eq!(d.type_pick(&mut f, 9, 99), 0);
    // V16: first roll 0, qualifying {4}: stop 4, scan 0–3 → item 0.
    f.types = vec![(4, 9)];
    f.seed = seed_for(5, 0, 0, 0);
    let before = f.seed;
    assert_eq!(d.type_pick(&mut f, 9, 99), 0);
    let mut one = before;
    one.step();
    assert_eq!(f.seed, one); // one draw only
                             // Level, spawnable and version filters.
    f.types = vec![(1, 9), (2, 9), (4, 9)];
    d.items[1].level = 50;
    d.items[4].spawnable = 0;
    f.seed = seed_for(5, 3, 1, 0);
    assert_eq!(d.type_pick(&mut f, 9, 10), 0); // [] after filters → 0
}

#[test]
fn type_pick_output_draws_on_the_game_seed() {
    // record 19 style: item-type output; two game-seed rolls.
    let r = recipe(1, &[code_in(RIN)], out(kind::ITEMTYPE, 9));
    let d = data(vec![r]);
    let mut f = Fake::new();
    f.add(RIN, 2);
    f.types = vec![(AMU, 9), (JEW, 9)];
    let n = d.items.len() as u32;
    f.seed = seed_for(n, 0, 2, 1);
    d.transmute(&mut f, P);
    // Scan 0..n−2 → [JEW] (AMU = n − 1 is the stop item) → roll(1).
    assert_eq!(f.requests[0].class, JEW);
}

#[test]
fn v17_v18_eligibility() {
    let mut r = gem_recipe();
    r.ladder = 1;
    let d = data(vec![r.clone()]);
    let gems = |f: &mut Fake| {
        for _ in 0..3 {
            f.add(GCV, 2);
        }
    };
    let mut f = Fake::new();
    gems(&mut f);
    assert_eq!(d.transmute(&mut f, P).record, None);
    let mut f = Fake::new();
    f.ladder = true;
    gems(&mut f);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    let mut f = Fake::new();
    f.game_type = 1;
    gems(&mut f);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    // V18: disabled.
    let mut r2 = gem_recipe();
    r2.enabled = 0;
    let mut f = Fake::new();
    gems(&mut f);
    assert_eq!(data(vec![r2]).transmute(&mut f, P).record, None);
    // min diff and class.
    let mut r3 = gem_recipe();
    r3.min_diff = 1;
    r3.class = 3;
    let d3 = data(vec![r3]);
    let mut f = Fake::new();
    gems(&mut f);
    f.difficulty = 1;
    assert_eq!(d3.transmute(&mut f, P).record, None); // class 0 ≠ 3
    f.class = 3;
    assert_eq!(d3.transmute(&mut f, P).record, Some(0));
    let mut f = Fake::new();
    gems(&mut f);
    f.class = 3;
    assert_eq!(d3.transmute(&mut f, P).record, None); // Normal < 1
}

#[test]
fn v19_quest_difficulty() {
    let mut r = recipe(2, &[code_in(MSF), code_in(VIP)], out(kind::ITEMCODE, HST));
    r.op = 28;
    let d = data(vec![r]);
    let mut f = Fake::new();
    f.difficulty = 1;
    f.add(MSF, 2);
    f.add(VIP, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.difficulty = 0;
    let t = d.transmute(&mut f, P);
    assert_eq!(t.record, Some(0));
    assert_eq!(f.log.last().unwrap(), "hook hst ");
    // stat 356 ≥ difficulty passes.
    let mut f = Fake::new();
    f.difficulty = 2;
    let a = f.add(MSF, 2);
    let b = f.add(VIP, 2);
    f.it(a).stats.insert(STAT_QUEST_DIFFICULTY, 2);
    f.it(b).stats.insert(STAT_QUEST_DIFFICULTY, 2);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
}

#[test]
fn v20_v21_v22_portals() {
    // V20: Pandemonium stub: matches, success 0, nothing changes.
    let d = data(vec![recipe(
        3,
        &[code_in(PK1), code_in(PK2), code_in(PK3)],
        out(kind::PANDEMONIUM, 0),
    )]);
    let mut f = Fake::new();
    f.add(PK1, 2);
    f.add(PK2, 2);
    f.add(PK3, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!((t.record, t.committed), (Some(0), false));
    assert!(f.log.is_empty() && f.cube_contents().len() == 3);
    // V21: cow portal refused → sound 20, contents kept.
    let d = data(vec![recipe(
        2,
        &[code_in(LEG), code_in(TBK)],
        out(kind::COW_PORTAL, 0),
    )]);
    let mut f = Fake::new();
    f.add(LEG, 2);
    f.add(TBK, 2);
    let t = d.transmute(&mut f, P);
    assert!(!t.committed);
    assert_eq!(f.log, ["cow", "sound 20"]);
    assert_eq!(f.cube_contents().len(), 2);
    // V22: success → contents removed, sound 4.
    let mut f = Fake::new();
    f.cow = true;
    let l = f.add(LEG, 2);
    let b = f.add(TBK, 2);
    assert!(d.transmute(&mut f, P).committed);
    assert_eq!(
        f.log,
        [
            "cow".to_string(),
            format!("remove {}", l.0),
            format!("remove {}", b.0),
            "sound 4".into()
        ]
    );
}

#[test]
fn portal_slot_overwrites_success() {
    // An item in slot a, then a failing portal in slot b: nothing commits
    // and the item is discarded (edge cases 2, 3).
    let mut r = gem_recipe();
    r.outputs[1] = out(kind::PANDEMONIUM, 0);
    let d = data(vec![r]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    let t = d.transmute(&mut f, P);
    assert!(!t.committed);
    assert_eq!(t.outputs.len(), 1);
    assert_eq!(f.cube_contents().len(), 3);
    assert!(f.log.is_empty());
}

#[test]
fn v23_repair() {
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::REP;
    let d = data(vec![recipe(2, &[code_in(HAX), code_in(RIN)], a)]);
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    f.it(h).stats.insert(STAT_DURABILITY, 10);
    f.it(h).stats.insert(STAT_MAX_DURABILITY, 50);
    f.add(RIN, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(f.items[&t.outputs[0]].stats[&STAT_DURABILITY], 50);
    // Broken → repair.
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    f.it(h).flags = item_flags::BROKEN;
    f.add(RIN, 2);
    d.transmute(&mut f, P);
    assert!(f.log.contains(&"repair".to_string()));
}

#[test]
fn rep_refills_stackables() {
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::REP;
    a.quantity = 255;
    let d = data(vec![recipe(2, &[code_in(AQV), code_in(RIN)], a)]);
    let mut f = Fake::new();
    let q = f.add(AQV, 2);
    f.it(q).stats.insert(STAT_EXTRA_STACK, 400);
    f.add(RIN, 2);
    let t = d.transmute(&mut f, P);
    // min(255, min(350 + 400, 511)) = 255
    assert_eq!(f.items[&t.outputs[0]].stats[&STAT_QUANTITY], 255);
}

#[test]
fn v24_v25_routing() {
    let d = data(vec![gem_recipe()]);
    // V24: no interaction → 0x77 0x0C.
    let mut f = Fake::new();
    assert_eq!(d.click_button(&mut f, P, BUTTON_TRANSMUTE), Some(0));
    assert_eq!(f.sent, [vec![0x77, 0x0C]]);
    // Wrong interaction type → 3.
    let mut f = Fake::new();
    f.interaction = Some((2, 9));
    assert_eq!(d.click_button(&mut f, P, BUTTON_CLOSE), Some(3));
    assert_eq!(d.click_button(&mut f, P, 0x05), None);
    // Close.
    f.interaction = Some((INTERACT_CUBE, 9));
    assert_eq!(d.click_button(&mut f, P, BUTTON_CLOSE), Some(0));
    assert_eq!(f.log, ["reset", "pass"]);
    // Transmute: any GUID of type 4.
    let mut f = Fake::new();
    f.interaction = Some((INTERACT_CUBE, 12345));
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    assert_eq!(d.click_button(&mut f, P, BUTTON_TRANSMUTE), Some(0));
    assert_eq!(f.cube_contents(), [Some(GFV)]);
    // Open: from the stash.
    let mut f = Fake::new();
    f.stash = true;
    d.open(&mut f, P, 77);
    assert_eq!(f.sent, [vec![0x77, 0x11], vec![0x77, 0x15]]);
    assert_eq!(f.interaction, Some((INTERACT_CUBE, 77)));
    // V25: 0x2A with a stored item (mode 0) → 3.
    let mut f = Fake::new();
    let cube = f.add(BOX, 2);
    f.it(cube).page = 0;
    let ring = f.add(RIN, 2);
    f.it(ring).page = 0;
    let msg = |item: UnitId, cube: UnitId| {
        let mut m = vec![0x2A];
        m.extend(item.0.to_le_bytes());
        m.extend(cube.0.to_le_bytes());
        m
    };
    assert_eq!(d.put_in(&mut f, P, &msg(ring, cube)), 3);
    assert_eq!(f.log, ["targeting"]);
    // Cursor item → page 3, placed.
    f.it(ring).mode = 4;
    assert_eq!(d.put_in(&mut f, P, &msg(ring, cube)), 0);
    assert_eq!(f.items[&ring].page, CUBE_PAGE);
    // Not a cube → refused; unknown item → 1; trading → sound 19.
    assert_eq!(d.put_in(&mut f, P, &msg(cube, ring)), 3);
    assert_eq!(d.put_in(&mut f, P, &msg(UnitId(999), cube)), 1);
    f.trading = true;
    f.it(cube).page = 1;
    assert_eq!(d.put_in(&mut f, P, &msg(ring, cube)), 0);
    assert_eq!(f.log.last().unwrap(), "sound 19");
    assert_eq!(d.put_in(&mut f, P, &msg(ring, cube)[..8]), 3);
}

#[test]
fn v26_stat_op_guard() {
    let d = data(vec![]); // 10 itemstatcost records
    let f = Fake::new();
    let mut r = gem_recipe();
    r.op = 3;
    r.value = 1;
    r.param = 10; // = count: fails
    assert!(!d.recipe_op(&f, P, &r, (1, 1)));
    r.param = 11; // > count: passes
    assert!(d.recipe_op(&f, P, &r, (1, 1)));
    r.param = -1;
    assert!(d.recipe_op(&f, P, &r, (1, 1)));
    r.param = 4; // stat 4 = 0 < 1: fails; ≥ 1 passes
    assert!(!d.recipe_op(&f, P, &r, (1, 1)));
    let mut f = Fake::new();
    f.player_stats.insert(4, 1);
    assert!(d.recipe_op(&f, P, &r, (1, 1)));
    // ValShift: value 256 >> 8 = 1.
    let mut d2 = data(vec![]);
    d2.valshift[4] = 8;
    r.value = 256;
    assert!(d2.recipe_op(&f, P, &r, (1, 1)));
    // Bonus reader (op 11: bonus ≥ t; the fake's bonus is value − 1).
    r.op = 11;
    assert!(!d2.recipe_op(&f, P, &r, (1, 1)));
}

#[test]
fn date_ops() {
    let d = data(vec![]);
    let f = Fake::new();
    let mut r = gem_recipe();
    r.op = 1;
    (r.param, r.value) = (10, 20);
    assert!(d.recipe_op(&f, P, &r, (10, 1)));
    assert!(d.recipe_op(&f, P, &r, (20, 1)));
    assert!(!d.recipe_op(&f, P, &r, (21, 1)));
    r.op = 2;
    r.value = 7;
    assert!(d.recipe_op(&f, P, &r, (1, 7)));
    assert!(!d.recipe_op(&f, P, &r, (1, 6)));
    // Input-scope ops pass at recipe scope.
    r.op = 15;
    assert!(d.recipe_op(&f, P, &r, (1, 1)));
}

#[test]
fn input_flags_filter() {
    use input_flags as fl;
    let one = |flags: u16, setup: &dyn Fn(&mut Fake, UnitId)| {
        let d = data(vec![recipe(
            1,
            &[input(fl::USEANY | flags, HAX, 0)],
            out(kind::ITEMCODE, RIN),
        )]);
        let mut f = Fake::new();
        let h = f.add(HAX, 2);
        setup(&mut f, h);
        d.transmute(&mut f, P).record.is_some()
    };
    let none = |_: &mut Fake, _: UnitId| {};
    let eth = |f: &mut Fake, h: UnitId| f.it(h).flags |= item_flags::ETHEREAL;
    let socketed = |f: &mut Fake, h: UnitId| f.it(h).sockets = 2;
    let rw = |f: &mut Fake, h: UnitId| f.it(h).flags |= item_flags::RUNEWORD;
    assert!(one(fl::NOS, &none) && !one(fl::NOS, &socketed));
    assert!(!one(fl::SOCK, &none) && one(fl::SOCK, &socketed));
    // Only the first of nos/sock is tested.
    assert!(one(fl::NOS | fl::SOCK, &none));
    assert!(one(fl::NOE, &none) && !one(fl::NOE, &eth));
    assert!(!one(fl::ETH, &none) && one(fl::ETH, &eth));
    assert!(one(fl::NRU, &none) && !one(fl::NRU, &rw));
    // hax is its own normcode, not its ubercode.
    assert!(one(fl::BAS, &none) && !one(fl::EXC, &none) && !one(fl::ELI, &none));
    // Unique / set file index: +4 − 1.
    let special = |idx: u16| {
        let mut i = input(fl::USEANY | fl::SPECIAL, HAX, 0);
        i.special = idx;
        let d = data(vec![recipe(1, &[i], out(kind::ITEMCODE, RIN))]);
        let mut f = Fake::new();
        let h = f.add(HAX, 7);
        f.it(h).file_index = 4;
        d.transmute(&mut f, P).record.is_some()
    };
    assert!(special(5) && !special(4));
    // Item type slot.
    let d = data(vec![recipe(
        1,
        &[input(fl::ITEMCODE, 0, 0)],
        out(kind::ITEMCODE, RIN),
    )]);
    let mut f = Fake::new();
    f.add(HAX, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.types = vec![(HAX, 0)];
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    // `any` accepts every item with a record; one without is skipped
    // but still counted by n.
    let d = data(vec![recipe(
        1,
        &[input(fl::USEANY, 0xFFFF, 0)],
        out(kind::ITEMCODE, RIN),
    )]);
    let mut f = Fake::new();
    f.add(HLM, 2);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    let mut f = Fake::new();
    let x = f.add(HLM, 2);
    f.it(x).class = None;
    assert_eq!(d.transmute(&mut f, P).record, None);
}

#[test]
fn input_op_on_slot0() {
    // op 27: file index != value, on slot-0 candidates only.
    let mut r = recipe(1, &[code_in(RIN)], out(kind::ITEMCODE, AMU));
    r.op = 27;
    r.value = 3;
    let d = data(vec![r]);
    let mut f = Fake::new();
    let ring = f.add(RIN, 2);
    f.it(ring).file_index = 3;
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.it(ring).file_index = 4;
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
}

#[test]
fn capture_last_match_wins_and_upgrade() {
    // usetype,exc with qty 2: class = the last match's ubercode item.
    let mut a = out(kind::USETYPE, 0);
    a.flags = output_flags::EXC;
    let d = data(vec![recipe(2, &[input(1, FHL, 2)], a)]);
    let mut f = Fake::new();
    let h1 = f.add(FHL, 2);
    let h2 = f.add(FHL, 2);
    f.it(h1).level = 5;
    f.it(h2).level = 9;
    d.transmute(&mut f, P);
    assert_eq!(f.requests[0].class, XHL);
    // Classic game: xhl (version 100) not usable → own class.
    let mut f = Fake::new();
    f.expansion = false;
    f.add(FHL, 2);
    f.add(FHL, 2);
    d.transmute(&mut f, P);
    assert_eq!(f.requests[0].class, FHL);
    // useitem without mod ignores the upgrade.
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::EXC;
    let d = data(vec![recipe(1, &[code_in(FHL)], a)]);
    let mut f = Fake::new();
    f.add(FHL, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(f.items[&t.outputs[0]].class, Some(FHL));
    // useitem,mod,exc → copy with the upgraded class, then item init.
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::EXC | output_flags::MOD;
    let d = data(vec![recipe(1, &[code_in(FHL)], a)]);
    let mut f = Fake::new();
    let h = f.add(FHL, 2);
    let t = d.transmute(&mut f, P);
    let x = t.outputs[0];
    assert_eq!(f.items[&x].class, Some(XHL));
    assert!(f
        .log
        .starts_with(&[format!("dup {} true", h.0), format!("init {}", x.0)]));
    // useitem,mod on hax (no `9ha` item) → class 0.
    let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
    let mut f = Fake::new();
    f.add(HAX, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(f.items[&t.outputs[0]].class, Some(0));
}

#[test]
fn reg_keeps_unique_unfound() {
    let mut a = out(kind::USETYPE, 0);
    a.flags = output_flags::REG;
    let d = data(vec![recipe(1, &[code_in(RIN)], a)]);
    let mut f = Fake::new();
    let ring = f.add(RIN, 7);
    f.it(ring).file_index = 41;
    f.it(ring).level = 120;
    d.transmute(&mut f, P);
    let r = f.requests[0];
    assert_eq!((r.quality, r.item_index, r.level), (7, 42, 120));
    assert!(!f.unique_found(41)); // creation set it; cleared again
                                  // Already found: stays found.
    let mut f = Fake::new();
    let ring = f.add(RIN, 7);
    f.it(ring).file_index = 41;
    f.uniques.insert(41, true);
    d.transmute(&mut f, P);
    assert!(f.unique_found(41));
}

#[test]
fn tempered_useitem_turns_off_craft() {
    let mut a = out(kind::USEITEM, 0);
    a.quality = 9;
    a.mods[0] = CraftMod {
        property: 3,
        ..CraftMod::default()
    };
    let d = data(vec![recipe(1, &[code_in(AMU)], a)]);
    let mut f = Fake::new();
    f.add(AMU, 4);
    f.tempered = (5, 6);
    d.transmute(&mut f, P);
    assert!(f.log.contains(&"tempered 5 6".to_string()));
    assert!(f.log.iter().any(|l| l.starts_with("prop 3")));
    let mut f = Fake::new();
    f.add(AMU, 4);
    f.tempered = (5, 0);
    d.transmute(&mut f, P);
    assert!(!f
        .log
        .iter()
        .any(|l| l.starts_with("prop") || l.starts_with("tempered")));
}

#[test]
fn rem_fillers_and_failed_placement() {
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::REM;
    let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    let mut jewel = Item {
        class: Some(JEW),
        ..Item::default()
    };
    jewel.page = 0xFE;
    let j = f.new_item(jewel);
    f.it(h).socketed = vec![j];
    let t = d.transmute(&mut f, P);
    assert!(t.committed);
    // Copy without fillers, runeword stats dropped, socket filler copied.
    assert!(f.log.contains(&format!("dup {} false", h.0)));
    assert!(f.log.contains(&format!("drop rw {}", t.outputs[0].0)));
    assert!(f.log.contains(&format!("dup {} true", j.0)));
    assert_eq!(f.cube_contents(), [Some(HAX), Some(JEW)]);
    // Placement fails: outputs freed after the inputs are gone.
    let mut f = Fake::new();
    f.place_ok = false;
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    let t = data(vec![gem_recipe()]).transmute(&mut f, P);
    assert!(t.committed);
    assert_eq!(f.log.last().unwrap(), &format!("free {}", t.outputs[0].0));
    assert!(f.cube_contents().is_empty());
}

#[test]
fn first_match_wins_even_if_outputs_fail() {
    // Record 0 matches but makes nothing; record 1 would succeed.
    let d = data(vec![
        recipe(3, &[input(1, GCV, 3)], out(kind::NONE, 0)),
        gem_recipe(),
    ]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    let t = d.transmute(&mut f, P);
    assert_eq!((t.record, t.committed), (Some(0), false));
}

#[test]
fn transmute_reads_the_date_once_and_only_with_contents() {
    // Several records are tried (day-of-month op 1: param ≤ day ≤ value).
    let mut late = gem_recipe();
    (late.op, late.param, late.value) = (1, 20, 31);
    let mut early = gem_recipe();
    (early.op, early.param, early.value) = (1, 1, 10);
    let d = data(vec![late, early, gem_recipe()]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(GCV, 2);
    }
    let t = d.transmute(&mut f, P);
    // Day 15 fails records 0 and 1; record 2 (no op) wins; one read.
    assert_eq!(t.record, Some(2));
    assert_eq!(f.date_reads.get(), 1);
    // No page-3 item: nothing, and the date is not read.
    let mut f = Fake::new();
    assert_eq!(d.transmute(&mut f, P), Transmute::default());
    assert_eq!(f.date_reads.get(), 0);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

#[test]
fn slot_item_class_or_any() {
    use input_flags as fl;
    let run = |slot: u32, held: u32| {
        let d = data(vec![recipe(
            1,
            &[input(fl::USEANY, slot, 0)],
            out(kind::ITEMCODE, RIN),
        )]);
        let mut f = Fake::new();
        f.add(held, 2);
        d.transmute(&mut f, P).record.is_some()
    };
    // 0xFFFF: no record fetched, any item passes the class test.
    assert!(run(0xFFFF, HAX));
    assert!(run(0xFFFF, GCV));
    // Otherwise the item's class must be the slot's.
    assert!(run(HAX, HAX));
    assert!(!run(GCV, HAX));
}
