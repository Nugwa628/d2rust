//! Integration tests for the write side of character storage
//! (`adapters::character::save`), which turns a live character into a `.d2s`
//! file when the server saves. Covered here: the item list order and skips,
//! the hireling's `jf` list written and read back by the loader (on the
//! synthetic install's item reader), the appearance provider and the
//! appearance tables of `world_data::tables` (synthetic install; the token
//! positions of the game's own tables on the user's install, `#[ignore]`,
//! `D2_GAME_DIR`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use d2_formats::d2s::appearance::{body, part, EquippedItem, ReferenceSlots, MODE_EQUIPPED};
use d2_formats::d2s::{self, Body, D2s, Header, Hireling, ReadOptions};
use d2_formats::mpq::ArchiveSet;
use d2_server::adapters::character::save::{
    equipment, item_list, list_order, rebuild, SaveContext, SaveItems, FLAGS2_NO_SAVE,
};
use d2_server::world_data::game::GameTables;
use d2_server::world_data::tables::{appearance_tables, SaveData};
use d2_sim::items::bitstream::{write_save, StreamItem, WriteBack};
use d2_sim::items::inventory::node;
use d2_sim::units::UnitId;
use test_fixtures::{install, synth};

fn synthetic(tag: &str) -> (install::Install, GameTables) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "synthetic-character-save-{tag}-{}",
        std::process::id()
    ));
    let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
    let t = GameTables::load(&i.archives).expect("the synthetic install loads");
    (i, t)
}

/// A hand-built view of the units and items.
#[derive(Default)]
struct Fake {
    lists: BTreeMap<UnitId, Vec<UnitId>>,
    body: BTreeMap<(UnitId, u8), UnitId>,
    in_use: BTreeMap<UnitId, UnitId>,
    cursor: BTreeMap<UnitId, UnitId>,
    nodes: BTreeMap<UnitId, (u8, u8)>,
    flags2: BTreeMap<UnitId, u32>,
    streams: BTreeMap<UnitId, StreamItem>,
    looks: BTreeMap<UnitId, EquippedItem>,
    write_backs: std::cell::RefCell<Vec<(UnitId, WriteBack)>>,
}

impl Fake {
    /// Links `item` into `owner`'s list (and its body slot for a body node).
    fn link(&mut self, owner: u32, item: u32, kind: u8, loc: u8, s: StreamItem) {
        let (o, u) = (UnitId(owner), UnitId(item));
        self.lists.entry(o).or_default().push(u);
        self.nodes.insert(u, (kind, loc));
        if kind == node::BODY {
            self.body.insert((o, loc), u);
        }
        self.streams.insert(u, s);
    }
}

impl SaveItems for Fake {
    fn items(&self, owner: UnitId) -> Vec<UnitId> {
        self.lists.get(&owner).cloned().unwrap_or_default()
    }
    fn body_item(&self, owner: UnitId, loc: u8) -> Option<UnitId> {
        self.body.get(&(owner, loc)).copied()
    }
    fn weapon_in_use(&self, owner: UnitId) -> Option<UnitId> {
        self.in_use.get(&owner).copied()
    }
    fn cursor(&self, owner: UnitId) -> Option<UnitId> {
        self.cursor.get(&owner).copied()
    }
    fn node(&self, item: UnitId) -> Option<(u8, u8)> {
        self.nodes.get(&item).copied()
    }
    fn flags2(&self, item: UnitId) -> u32 {
        self.flags2.get(&item).copied().unwrap_or(0)
    }
    fn stream(&self, item: UnitId) -> Option<StreamItem> {
        self.streams.get(&item).cloned()
    }
    fn appearance(&self, item: UnitId) -> Option<EquippedItem> {
        self.looks.get(&item).copied()
    }
    fn write_back(&self, item: UnitId, wb: WriteBack) {
        self.write_backs.borrow_mut().push((item, wb));
    }
}

/// A compact record of `code` (identified).
fn compact(code: [u8; 4]) -> StreamItem {
    StreamItem {
        flags: 0x10,
        compact: true,
        version: 101,
        code,
        page: 0xFF,
        ..StreamItem::default()
    }
}

/// A full record of `code` holding `filled` children.
fn full(code: [u8; 4], filled: u32) -> StreamItem {
    StreamItem {
        flags: 0x810,
        version: 101,
        code,
        page: 0xFF,
        filled,
        ilvl: 1,
        quality: 2,
        file_index: -1,
        runeword: 0xFFFF,
        main: Some(Vec::new()),
        ..StreamItem::default()
    }
}

// The hands go after the list, then the cursor item.
#[test]
fn list_order_puts_the_hands_after_the_list_then_the_cursor() {
    let p = UnitId(1);
    let mut f = Fake::default();
    f.link(1, 10, node::BELT, 0, compact(*b"pt1 "));
    f.link(1, 11, node::BODY, body::LEFT_HAND, compact(*b"ar1 "));
    f.link(1, 12, node::BODY, body::RIGHT_HAND, compact(*b"sb1 "));
    f.link(1, 13, node::BODY, body::HEAD, compact(*b"ar2 "));
    f.cursor.insert(p, UnitId(14));
    let ids = |v: Vec<UnitId>| v.into_iter().map(|u| u.0).collect::<Vec<_>>();
    // No weapon in use: right, then left.
    assert_eq!(ids(list_order(&f, p)), [10, 13, 12, 11, 14]);
    // The right hand in use: right, then left.
    f.in_use.insert(p, UnitId(12));
    assert_eq!(ids(list_order(&f, p)), [10, 13, 12, 11, 14]);
    // The left hand in use: left, then right.
    f.in_use.insert(p, UnitId(11));
    assert_eq!(ids(list_order(&f, p)), [10, 13, 11, 12, 14]);
    // A missing hand is skipped.
    f.body.remove(&(p, body::LEFT_HAND));
    f.lists.get_mut(&p).unwrap().retain(|u| u.0 != 11);
    f.cursor.clear();
    assert_eq!(ids(list_order(&f, p)), [10, 13, 12]);
}

// Items flagged no-save are skipped and not counted; children follow their parent.
#[test]
fn item_list_skips_unsaved_items_and_appends_children() {
    let (_, t) = synthetic("list");
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    let mut f = Fake::default();
    f.link(1, 10, node::PAGE, 0, full(*b"sb1 ", 1));
    f.link(10, 20, node::NONE, 0, compact(*b"pt1 "));
    f.link(1, 11, node::PAGE, 0, compact(*b"pt1 "));
    f.flags2.insert(UnitId(11), FLAGS2_NO_SAVE);
    let e = item_list(&f, UnitId(1), &s.items.isc).unwrap();
    assert_eq!(
        e.len(),
        1,
        "the 0x8000 item writes nothing and is not counted"
    );
    let mut want = full(*b"sb1 ", 1);
    want.children = vec![compact(*b"pt1 ")];
    assert_eq!(e[0].bytes, write_save(&want, &s.items.isc).unwrap().0);
    // The loader's entry length covers the child.
    use d2s::SaveTables;
    assert_eq!(s.item_entry_len(&e[0].bytes), Ok(e[0].bytes.len()));
}

/// The item writer's changes (item level < 1 → 1, quality outside 1–9 → 2)
/// reach every written item, the children too, in write order.
#[test]
fn item_list_hands_back_every_items_write_back() {
    let (_, t) = synthetic("wb");
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    let mut f = Fake::default();
    let mut parent = full(*b"sb1 ", 1);
    parent.ilvl = 0;
    f.link(1, 10, node::PAGE, 0, parent);
    let mut child = full(*b"sb1 ", 0);
    child.quality = 0;
    child.ilvl = 9;
    f.link(10, 20, node::NONE, 0, child);
    item_list(&f, UnitId(1), &s.items.isc).unwrap();
    let got: Vec<(u32, i32, u8)> = f
        .write_backs
        .borrow()
        .iter()
        .map(|(u, w)| (u.0, w.ilvl, w.quality))
        .collect();
    assert_eq!(got, [(10, 1, 2), (20, 9, 2)]);
}

fn base_save(expansion: bool) -> D2s {
    let mut h = Header::default();
    h.set_name(b"Merc").unwrap();
    h.class = 1;
    h.status = if expansion { d2s::status::EXPANSION } else { 0 };
    h.hireling = Hireling {
        flags: 0,
        seed: 7,
        name_index: 0,
        id: 0,
        experience: 10,
        rest: [0; 16],
    };
    let body = Body {
        skills: vec![0; 30],
        hireling_items: Some(None),
        golem: expansion.then(Default::default),
        ..Body::default()
    };
    D2s {
        header: h,
        body: Some(body),
    }
}

// The hireling's `jf` item list survives a write and a read by the loader.
#[test]
fn hireling_items_round_trip_through_the_loader() {
    let (_, t) = synthetic("jf");
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    let a = appearance_tables(&t.fixed, &slots_1_14d()).unwrap();
    let id = s
        .hirelings
        .rows
        .iter()
        .map(|r| r.id)
        .find(|&id| s.hirelings.row_at(true, id, 1).is_some())
        .expect("an expansion row at level 1");
    let (player, merc) = (UnitId(1), UnitId(2));
    let mut f = Fake::default();
    f.link(1, 9, node::PAGE, 0, compact(*b"pt1 "));
    f.link(2, 10, node::BODY, body::RIGHT_HAND, full(*b"sb1 ", 1));
    f.link(10, 20, node::NONE, 0, compact(*b"pt1 "));
    f.link(2, 11, node::BODY, body::HEAD, compact(*b"pt1 "));
    let ctx = SaveContext {
        expansion: true,
        ..SaveContext::default()
    };
    let mut save = base_save(true);
    save.header.hireling.id = u16::try_from(id).unwrap();
    rebuild(&mut save, &f, player, Some(merc), &ctx, &a, &s.items.isc).unwrap();
    let list = save.body.as_ref().unwrap().hireling_items.clone();
    let entries = list.clone().unwrap().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "the hands come after the list: head, then right"
    );
    assert_eq!(
        entries[0].bytes,
        write_save(&compact(*b"pt1 "), &s.items.isc).unwrap().0
    );
    let bytes = d2s::write(&save, &s).unwrap();
    let opts = ReadOptions {
        expansion: true,
        game: None,
    };
    let back = d2s::read(&bytes, &opts, &s).unwrap();
    assert_eq!(back.body.unwrap().hireling_items, list);

    // A hireling with no items: the empty list; none: the
    // marker alone; classic: no section.
    let empty = Fake::default();
    rebuild(
        &mut save,
        &empty,
        player,
        Some(merc),
        &ctx,
        &a,
        &s.items.isc,
    )
    .unwrap();
    assert_eq!(
        save.body.as_ref().unwrap().hireling_items,
        Some(Some(Vec::new()))
    );
    rebuild(&mut save, &empty, player, None, &ctx, &a, &s.items.isc).unwrap();
    assert_eq!(save.body.as_ref().unwrap().hireling_items, Some(None));
    let classic = SaveContext::default();
    let mut save = base_save(false);
    rebuild(
        &mut save,
        &f,
        player,
        Some(merc),
        &classic,
        &a,
        &s.items.isc,
    )
    .unwrap();
    assert_eq!(save.body.as_ref().unwrap().hireling_items, None);
}

// The saved header's appearance components come from the equipped items.
#[test]
fn rebuild_fills_the_appearance_from_the_equipped_items() {
    let (_, t) = synthetic("look");
    let s = SaveData::from_fixed(&t.fixed, true).unwrap();
    let mut a = appearance_tables(&t.fixed, &slots_1_14d()).unwrap();
    // The synthetic `armtype` tokens are upper case (`LIT`), so no entry
    // holds them; with the game's own tokens the armour `ar1` (no armour
    // columns: `armtype` row 0 for all six parts) takes entry 1 (`lit`).
    assert_eq!(a.tokens.lookup(a.armtype[0], a.armtype[0]), 0);
    a.armtype = vec![*b"lit ", *b"med ", *b"hvy "];
    let ar1 = a.items.iter().position(|r| r.code == *b"ar1 ").unwrap();
    let mut f = Fake::default();
    f.link(1, 10, node::BODY, body::TORSO, compact(*b"ar1 "));
    f.looks.insert(
        UnitId(10),
        EquippedItem {
            record: ar1,
            mode: MODE_EQUIPPED,
            body_loc: body::TORSO,
            quality: 2,
            file_index: -1,
            ..EquippedItem::default()
        },
    );
    let ctx = SaveContext {
        expansion: true,
        ..SaveContext::default()
    };
    let eq = equipment(&f, UnitId(1), &ctx);
    assert_eq!(eq.body_grid[usize::from(body::TORSO)], Some(0));
    assert_eq!(eq.items[0].record, ar1);
    let mut save = base_save(true);
    save.header.components = [1; 16];
    rebuild(&mut save, &f, UnitId(1), None, &ctx, &a, &s.items.isc).unwrap();
    let mut want = [0xFF; 16];
    for p in [part::TR, part::LG, part::RA, part::LA, part::S1, part::S2] {
        want[p] = 1;
    }
    assert_eq!(save.header.components, want);
    assert_eq!(save.header.colours, [0xFF; 16]);
}

// The appearance tables built from the synthetic install.
#[test]
fn appearance_tables_project_the_synthetic_install() {
    let (_, t) = synthetic("tables");
    let a = appearance_tables(&t.fixed, &ReferenceSlots::none()).unwrap();
    assert_eq!(a.items.len(), 6, "2 weapons + 2 armor + 2 misc");
    assert_eq!(a.armtype.len(), t.table("armtype").unwrap().count);
    assert_eq!(
        a.colours.affix.len(),
        4,
        "2 suffixes, 1 prefix, 1 automagic"
    );
    // The three `armtype` tokens head the table.
    assert_eq!(a.tokens.entry(1).unwrap().0, *b"lit ");
    assert_eq!(a.tokens.entry(3).unwrap().0, *b"hvy ");
}

/// The user's install (`$D2_GAME_DIR`).
fn live() -> GameTables {
    let dir = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR is set");
    let archives = ArchiveSet::open_dir(PathBuf::from(dir)).expect("archives open");
    GameTables::load(&archives).expect("the live set loads")
}

// The appearance token table on the user's install.
#[test]
#[ignore = "needs the game files (D2_GAME_DIR)"]
fn token_positions_on_the_users_install() {
    let t = live();
    let a = appearance_tables(&t.fixed, &slots_1_14d()).unwrap();
    let at = |c: &[u8; 4]| a.tokens.lookup(*c, *c);
    for (c, v) in [
        (b"hax ", 4),
        (b"axe ", 5),
        (b"lax ", 6),
        (b"wnd ", 9),
        (b"clb ", 12),
        (b"ssd ", 17),
        (b"jav ", 27),
        (b"bst ", 37),
        (b"ktr ", 45),
        (b"cap ", 57),
        (b"buc ", 79),
    ] {
        assert_eq!(at(c), v, "{}", String::from_utf8_lossy(c));
    }
    // `sst` draws as `bst` (Test vectors).
    assert_eq!(a.tokens.lookup(*b"bst ", *b"sst "), 0x25);
    assert_eq!(a.tokens.entry(0).unwrap().0, [0; 4]);
    // Edge case 4: a second `ktr` above n, never returned (OQ3 answer).
    assert_eq!(a.tokens.entry(243).unwrap().0, *b"ktr ");
}

/// The reference table of the image (`ReferenceSlots::game`, the
/// slot types) under the game's own itemtypes: the reserved slots
/// ([`slots_1_14d`]), and the second `ktr` at 243.
#[test]
#[ignore = "needs the game files (D2_GAME_DIR)"]
fn the_image_reference_table_on_the_users_install() {
    use d2_formats::d2s::appearance::{ty, IsA};
    let t = live();
    let eq = &t.fixed.itemtypes_equiv;
    let m = IsA::from_fn(eq.n, |i, j| eq.get(i, j));
    let types = d2_formats::d2s::appearance::reference_types();
    let class = |i: usize| (m.is_a(types[i], ty::WEAP), m.is_a(types[i], ty::ARMO));
    for i in 0..256 {
        let want = match i {
            43..=116 | 130..=133 | 135..=234 => (true, false),
            4..=42 | 118..=121 | 124..=129 | 134 | 235..=255 => (false, true),
            _ => (false, false),
        };
        assert_eq!(class(i), want, "slot {i}");
    }
    // The slots from the image's types equal the Constants' summary.
    assert_eq!(ReferenceSlots::game(&m), slots_1_14d());
    let a = appearance_tables(&t.fixed, &ReferenceSlots::game(&m)).unwrap();
    // The katar's second copy (edge case 4); the lookup returns 45.
    assert_eq!(a.tokens.entry(243).unwrap().0, *b"ktr ");
    assert_eq!(a.tokens.lookup(*b"ktr ", *b"ktr "), 45);
}

/// The slot classes of the reference table under the game's is-a relation
/// between item types: `weap` 43–116, 130–133, 135–234; `armo` 4–42, 118–121,
/// 124–129, 134, 235–255; the rest neither. The expected value of
/// `ReferenceSlots::game` on the user's tables, and the slots the
/// synthetic tables' tests use.
fn slots_1_14d() -> ReferenceSlots {
    let mut s = ReferenceSlots::none();
    for r in [43..=116, 130..=133, 135..=234] {
        for slot in &mut s.0[r] {
            slot.weap = true;
        }
    }
    for r in [4..=42, 118..=121, 124..=129, 134..=134, 235..=255] {
        for slot in &mut s.0[r] {
            slot.armo = true;
        }
    }
    s
}
