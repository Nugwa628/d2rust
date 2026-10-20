//! Items ↔ units / stats: creation on a real item unit and stat list,
//! checked against the same pipeline on the map oracle.

use super::*;
use crate::items::{create, q, stat, ItemGame, ItemRequest};
use crate::wiring::economy::item_units::swap_stats;
use crate::wiring::economy::{find_list, EconomyError, ItemSpawn};

const GROUND: ItemSpawn = ItemSpawn {
    room: None,
    mode: 3,
    init_flags: 1,
};

fn unique_cap() -> ItemRequest {
    ItemRequest {
        item: CAP as i32,
        format: 101,
        ilvl: 10,
        quality: q::UNIQUE,
        ..ItemRequest::default()
    }
}

/// The oracle run: the same request and game seed on [`MapStats`].
fn oracle(t: &ItemTables, rq: &ItemRequest) -> (create::Created<MapStats>, GameFields) {
    let mut g = GameFields::new(Seed::init_low(GAME_SEED), true);
    let mut rq = rq.clone();
    let c = create::create_item(t, &mut g, &mut rq, false, MapStats::default(), 0).unwrap();
    (c, g)
}

/// Item creation with quality, unique and property handling (property
/// mode 3): a unique cap's property lands in a real stat list (state 0,
/// flags 0x40) attached to the item; every field, seed and stat equals
/// the oracle's.
#[test]
fn unique_item_on_real_units_and_stat_lists() {
    let mut w = World::new();
    let mut rq = unique_cap();
    let unit = w.econ().create_item(&mut rq, false, GROUND).unwrap();
    let (c, og) = oracle(&w.tables, &unique_cap());

    // Game seed: two steps, as the oracle.
    let mut g = Seed::init_low(GAME_SEED);
    g.step();
    g.step();
    assert_eq!(w.fields.seed, g);
    assert_eq!(w.fields.seed, og.seed);
    assert_eq!(w.fields.uniques, og.uniques);
    assert!(w.fields.uniques.get(0));

    // Item data and the unit record's seeds.
    let (want, ms) = swap_stats(c.item, ());
    assert_eq!(w.items.get(unit), Some(&want));
    assert_eq!(want.quality, q::UNIQUE);
    assert_eq!(want.file_index, 0);
    let r = w.units.get(unit).unwrap();
    assert_eq!((r.ty, r.class, r.mode), (UnitType::Item, CAP as u32, 3));
    assert_eq!(r.seed, want.unit_seed);
    assert_eq!(r.init_seed, want.init_seed);
    assert_eq!(r.item_seed, Some((want.item_seed, want.start_seed)));
    assert_eq!(w.game.lists.unit(unit).unwrap().ty, UnitType::Item);

    // Stats: base entries, the property list, and the item's totals.
    assert!(!ms.base.is_empty());
    for (&(id, layer), &v) in &ms.base {
        assert_eq!(w.stats.unit_base(unit, id, layer), v, "base {id}");
    }
    assert_eq!(
        ms.lists.keys().copied().collect::<Vec<_>>(),
        [ListKey::ITEM]
    );
    let l = find_list(&w.stats, unit, ListKey::ITEM).expect("property list");
    for (&(id, layer), &v) in &ms.lists[&ListKey::ITEM] {
        assert_eq!(w.stats.base(l, id, layer), v, "list {id}");
    }
    let v = ms.list_get(ListKey::ITEM, PROP_STAT, 0);
    assert!((10..=20).contains(&v));
    for s in [
        PROP_STAT,
        stat::DURABILITY,
        stat::MAXDURABILITY,
        stat::ARMORCLASS,
    ] {
        assert_eq!(w.stats.unit_total(unit, s, 0), ms.stat(s, 0), "total {s}");
    }
    assert_eq!(w.stats.owner_type(l), 4);
    assert_eq!(w.stats.state(l), 0);
    assert!(w.game.timers.unit_timers(unit).is_empty());
}

/// Same seed, same world → same item (determinism).
#[test]
fn creation_is_deterministic() {
    let run = || {
        let mut w = World::new();
        let u = w
            .econ()
            .create_item(&mut unique_cap(), false, GROUND)
            .unwrap();
        let l = find_list(&w.stats, u, ListKey::ITEM).unwrap();
        (
            w.items.get(u).cloned(),
            w.units.get(u).cloned(),
            w.stats.base_entries(l),
            w.stats.full_entries(w.stats.unit_list(u).unwrap()),
        )
    };
    assert_eq!(run(), run());
}

/// The item's list attached to a player carries
/// the property into the player's totals; `with_item` re-assembles the
/// same item.
#[test]
fn item_properties_reach_the_equipping_player() {
    let mut w = World::new();
    let player = w.spawn(UnitType::Player, 1);
    let unit = w
        .econ()
        .create_item(&mut unique_cap(), false, GROUND)
        .unwrap();
    let v = w.stats.unit_total(unit, PROP_STAT, 0);
    assert_ne!(v, 0);
    assert_eq!(w.stats.unit_total(player, PROP_STAT, 0), 0);
    let il = w.stats.unit_list(unit);
    w.stats.equip(&mut w.hooks, player, il, false, true);
    assert_eq!(w.stats.unit_total(player, PROP_STAT, 0), v);
    // The player's list has the server callback: writes reach the host.
    let before = w.hooks.callbacks;
    w.set_stat(player, 7, 100);
    assert!(w.hooks.callbacks >= before);
    // `with_item` builds the item from the store, record and lists.
    let seen = w
        .econ()
        .with_item(unit, |s| (s.item.quality, s.item.stats.stat(PROP_STAT, 0)))
        .unwrap();
    assert_eq!(seen, (q::UNIQUE, v));
}

/// Failures in the first two creation steps happen before the allocation:
/// no unit, no seed step.
#[test]
fn early_failures_allocate_nothing() {
    let mut w = World::new();
    let mut rq = ItemRequest {
        item: 99,
        ..unique_cap()
    };
    assert_eq!(
        w.econ().create_item(&mut rq, false, GROUND),
        Err(EconomyError::Create(create::CreateError::BadIndex))
    );
    w.tables.items[CAP].version = 100;
    w.fields.expansion = false;
    assert_eq!(
        w.econ().create_item(&mut unique_cap(), false, GROUND),
        Err(EconomyError::Create(create::CreateError::Classic))
    );
    assert_eq!(w.fields.seed, Seed::init_low(GAME_SEED));
    assert!(w.game.lists.units_of_type(UnitType::Item).is_empty());
}

/// A failure after the allocation removes the unit and its stat list;
/// the seed steps stay made.
#[test]
fn late_failure_removes_the_unit() {
    let mut w = World::new();
    w.tables.itemratio.clear();
    let mut rq = ItemRequest {
        quality: 0,
        ..unique_cap()
    };
    let e = w.econ().create_item(&mut rq, false, GROUND);
    assert!(
        matches!(e, Err(EconomyError::Create(create::CreateError::Fatal(_)))),
        "{e:?}"
    );
    let mut g = Seed::init_low(GAME_SEED);
    g.step();
    g.step();
    assert_eq!(w.fields.seed, g);
    assert!(w.game.lists.units_of_type(UnitType::Item).is_empty());
    assert!(w.units.get(UnitId(0)).is_none() && w.items.is_empty());
}

/// A replenish stat schedules item event 3 at
/// frame + 2500 / r + 1 on the game's timer queue.
#[test]
fn replenish_schedules_event_3() {
    let mut w = World::new();
    w.tables.properties[PROP_ROW as usize].slots[0].stat = stat::REPLENISH_DURABILITY;
    w.tables.uniques[0].props[0].min = 10;
    w.tables.uniques[0].props[0].max = 10;
    w.game.frame = 40;
    let unit = w
        .econ()
        .create_item(&mut unique_cap(), false, GROUND)
        .unwrap();
    let timers = w.game.timers.unit_timers(unit);
    assert_eq!(timers.len(), 1);
    assert_eq!(w.game.timers.expire(timers[0]), Some(40 + 2500 / 10 + 1));
    assert_eq!(w.game.timers.event(timers[0]).map(|e| e.0), Some(3));
    // Freeing the item cancels its timers and drops its list.
    w.econ().free_item(unit).unwrap();
    assert!(w.game.timers.unit_timers(unit).is_empty());
    assert!(w.stats.unit_list(unit).is_none() && w.items.is_empty());
}

/// The request unit from unit and stat fields.
#[test]
fn request_unit_from_unit_fields() {
    let mut w = World::new();
    let p = w.spawn(UnitType::Player, 3);
    w.set_stat(p, 12, 42);
    let name = *b"bob\0\0\0\0\0\0\0\0\0\0\0\0\0";
    let r = w.econ().request_unit(p, Some((name, Some(true)))).unwrap();
    assert_eq!(r.class, 3);
    let pi = r.player.unwrap();
    assert_eq!((pi.name, pi.level, pi.hardcore), (name, 42, Some(true)));
    assert_eq!(ItemGame::item_format(&w.fields), 101);
}
