//! Cube → items: a transmute whose output is a real item unit with a
//! craft property in its stat list. The cube's input and output slots come
//! from `crate::world::cube`; item creation comes from the economy wiring.

use super::*;
use crate::items::{props, q, ItemRequest, RequestUnit};
use crate::wiring::economy::{find_list, CubeRest, EconomyCube, ItemSpawn};
use crate::world::cube::{
    input_flags, kind, CraftMod, CubeData, CubeWorld, InputSlot, ItemRecord, OutputSlot, Recipe,
    CUBE_PAGE,
};

const INV: ItemSpawn = ItemSpawn {
    room: None,
    mode: 0,
    init_flags: 1,
};

/// The rest of the cube's world: an inventory list and a log.
#[derive(Default)]
struct Rest {
    inventory: Vec<UnitId>,
    log: Vec<String>,
}

impl CubeRest for Rest {
    fn player_info(&self, _: UnitId) -> Option<([u8; 16], Option<bool>)> {
        None
    }
    fn local_date(&self) -> (u8, u8) {
        (15, 3)
    }
    fn attach_sound(&mut self, _: UnitId, event: u8) {
        self.log.push(format!("sound {event}"));
    }
    fn send(&mut self, _: UnitId, msg: &[u8]) {
        self.log.push(format!("send {:02x}", msg[0]));
    }
    fn inventory_pass(&mut self, _: UnitId) {}
    fn inventory(&self, _: UnitId) -> Vec<UnitId> {
        self.inventory.clone()
    }
    fn socketed(&self, _: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    fn place(&mut self, _: UnitId, item: UnitId) -> bool {
        self.inventory.push(item);
        self.log.push(format!("place {}", item.0));
        true
    }
    fn remove_cube_item(&mut self, _: UnitId, item: UnitId) {
        self.inventory.retain(|&i| i != item);
        self.log.push(format!("remove {}", item.0));
    }
    fn targeting_reset(&mut self, _: UnitId) {}
    fn put_item_check(&self, _: UnitId, _: u32) -> u32 {
        0
    }
    fn cube_check(&self, _: UnitId, _: u32) -> bool {
        true
    }
    fn duplicate(&mut self, _: UnitId, _: bool) -> Option<UnitId> {
        None
    }
    fn tempered_affix(&mut self, _: UnitId, _: bool) -> u16 {
        0
    }
    fn drop_runeword_stats(&mut self, _: UnitId) {}
    fn repair(&mut self, _: UnitId) {}
    fn recharge(&mut self, _: UnitId) {}
    fn quest_item_hook(&mut self, _: UnitId, _: UnitId, _: [u8; 4]) {}
    fn cow_portal(&mut self, _: UnitId) -> bool {
        false
    }
}

/// A recipe: "rin" → amulet (quality 2) with mod 1 =
/// property 7, param −1, 5–10, chance `chance`.
fn recipe(chance: u8) -> Recipe {
    let mut inputs = [InputSlot::default(); 7];
    inputs[0] = InputSlot {
        flags: input_flags::USEANY,
        item: RING as u16,
        ..InputSlot::default()
    };
    let mut out = OutputSlot {
        kind: kind::ITEMCODE,
        item: AMULET as u16,
        quality: q::NORMAL,
        mods: [CraftMod {
            property: -1,
            ..CraftMod::default()
        }; 5],
        ..OutputSlot::default()
    };
    out.mods[0] = CraftMod {
        property: PROP_ROW,
        param: 0xFFFF,
        min: 5,
        max: 10,
        chance,
    };
    Recipe {
        enabled: 1,
        class: 0xFF,
        numinputs: 1,
        inputs,
        outputs: [out, OutputSlot::default(), OutputSlot::default()],
        ..Recipe::default()
    }
}

fn cube_data(t: &ItemTables, recipes: Vec<Recipe>) -> CubeData {
    CubeData {
        recipes,
        items: t
            .items
            .iter()
            .map(|r| ItemRecord {
                code: r.code,
                level: r.level,
                spawnable: 1,
                ..ItemRecord::default()
            })
            .collect(),
        valshift: vec![0; N_STATS],
        max_level: 99,
    }
}

/// A player with a ring in the cube (a real normal ring).
fn setup() -> (World, UnitId, UnitId) {
    let mut w = World::new();
    let player = w.spawn(UnitType::Player, 2);
    w.set_stat(player, 12, 9);
    let mut rq = ItemRequest {
        item: RING as i32,
        format: 101,
        ilvl: 5,
        quality: q::NORMAL,
        ..ItemRequest::default()
    };
    let ring = w.econ().create_item(&mut rq, false, INV).unwrap();
    w.items.get_mut(ring).unwrap().inv_page = CUBE_PAGE;
    (w, player, ring)
}

/// The transmute runs on the real world: the ring is matched through
/// the item data, the amulet is created on a real unit, and the craft
/// property lands in its stat list. A direct run of the same calls gives the same item.
#[test]
fn transmute_makes_a_real_item_with_its_craft_property() {
    let (mut w, player, ring) = setup();
    let d = cube_data(&w.tables, vec![recipe(0)]);
    let mut rest = Rest {
        inventory: vec![ring],
        ..Rest::default()
    };
    let t = {
        let mut e = w.econ();
        let mut cube = EconomyCube::new(&mut e, &mut rest);
        let t = d.transmute(&mut cube, player);
        assert!(cube.errors.is_empty(), "{:?}", cube.errors);
        t
    };
    assert_eq!(t.record, Some(0));
    assert!(t.committed);
    let amu = t.outputs[0];
    assert_eq!(
        rest.log,
        [
            format!("remove {}", ring.0),
            "sound 4".into(),
            format!("place {}", amu.0)
        ]
    );
    let it = w.items.get(amu).unwrap().clone();
    assert_eq!((it.record, it.quality), (AMULET, q::NORMAL));
    assert_eq!(w.units.get(amu).unwrap().mode, 4);
    let l = find_list(&w.stats, amu, ListKey::ITEM).expect("craft list");
    let v = w.stats.base(l, PROP_STAT, 0);
    assert!((5..=10).contains(&v), "{v}");
    assert_eq!(w.stats.unit_total(amu, PROP_STAT, 0), v);

    // The same item made directly: the cube's output request, then the
    // craft list.
    let (mut w2, player2, _) = setup();
    let mut rq = ItemRequest {
        unit: Some(RequestUnit {
            class: 2,
            player: None,
        }),
        item: AMULET as i32,
        ilvl: it.ilvl,
        format: 101,
        quality: q::NORMAL,
        flags2: 0x0A,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode: 4,
        init_flags: 1,
    };
    assert_eq!(player2, player);
    let amu2 = w2.econ().create_item(&mut rq, false, spawn).unwrap();
    let rec = PropRec {
        code: PROP_ROW,
        param: -1,
        min: 5,
        max: 10,
    };
    w2.econ()
        .with_item(amu2, |s| props::apply_craft_list(s.tables, s.item, &[rec]))
        .unwrap();
    assert_eq!(
        w2.units.get(amu2).unwrap().seed,
        w.units.get(amu).unwrap().seed
    );
    assert_eq!(
        w2.units.get(amu2).unwrap().item_seed,
        w.units.get(amu).unwrap().item_seed
    );
    let l2 = find_list(&w2.stats, amu2, ListKey::ITEM).unwrap();
    assert_eq!(w2.stats.base_entries(l2), w.stats.base_entries(l));
    assert_eq!(w2.fields.seed, w.fields.seed);
}

/// The item and game fields the cube reads and writes land on the real
/// providers: GUID lookup, page, mode, class, flags, sockets,
/// unique bits, stats, free.
#[test]
fn cube_fields_on_real_providers() {
    let (mut w, player, ring) = setup();
    w.tables.items[RING].gemsockets = 2;
    w.tables.items[RING].invwidth = 1;
    w.tables.items[RING].invheight = 1;
    w.tables.itemtypes[10].maxsock1 = 3;
    let guid = w.units.get(ring).unwrap().guid;
    let mut rest = Rest::default();
    let mut e = w.econ();
    let mut c = EconomyCube::new(&mut e, &mut rest);
    assert_eq!(c.item_by_guid(guid), Some(ring));
    assert_eq!(c.item_guid(ring), guid);
    assert_eq!(c.item_page(ring), CUBE_PAGE);
    c.set_item_mode(ring, 4);
    assert_eq!(c.item_mode(ring), 4);
    assert_eq!(c.item_class(ring), Some(RING as u32));
    assert!(c.class_is_type(RING as u32, crate::items::ty::MISC));
    // Sockets: min(gemsockets 2, maxsock1 3) at ilvl 5.
    assert_eq!(c.max_sockets(ring), 2);
    // Capped by the 1×1 inventory size.
    c.add_sockets(ring, 3);
    assert_eq!(c.item_sockets(ring), 1);
    assert_ne!(c.item_flags(ring) & crate::items::flag::SOCKETED, 0);
    assert!(!c.unique_found(5));
    c.set_unique_found(5, true);
    assert!(c.unique_found(5));
    c.set_unique_found(5, false);
    assert!(!c.unique_found(5));
    assert_eq!(c.player_class(player), 2);
    assert_eq!(c.stat(player, crate::world::cube::StatRead::Value, 12), 9);
    c.set_stat(player, 12, 11);
    assert_eq!(c.stat(player, crate::world::cube::StatRead::Base, 12), 11);
    c.set_tempered(ring, 3, 4);
    assert_eq!(c.item_quality(ring), q::TEMPERED);
    c.free_item(ring);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.item_by_guid(guid), None);
    drop(c);
    assert!(w.items.is_empty());
}
