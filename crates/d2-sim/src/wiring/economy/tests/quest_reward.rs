//! Tests for the quest reward item (create, place, drop or free) on the
//! inventory wiring's model ([`QuestInv`]), with the inventory wiring's
//! fixture.

use crate::items::moves::{self, Owner, Spot};
use crate::items::{flag, stat};
use crate::wiring::economy::quest_reward::{
    create_reward, drop_or_free, item_level, place_reward, Placed,
};
use crate::wiring::economy::{Economy, QuestInv};
use crate::wiring::inventory::tests::World;

/// Runs `f` on the world's economy and its inventory model lent as a
/// [`QuestInv`].
fn lent<T>(
    w: &mut World,
    f: impl FnOnce(
        &mut Economy<'_, crate::wiring::inventory::tests::Hooks>,
        &mut QuestInv<'_, crate::wiring::inventory::tests::Rest>,
    ) -> T,
) -> T {
    let mut econ = Economy {
        game: &mut w.game,
        units: &mut w.units,
        stats: &mut w.stats,
        data: &w.data,
        hooks: &mut w.hooks,
        fields: &mut w.fields,
        tables: &w.tables,
        items: &mut w.items,
    };
    let mut inv = QuestInv::new(&w.inv, &mut w.state, &mut w.rest);
    let out = f(&mut econ, &mut inv);
    assert!(inv.errors.is_empty(), "{:?}", inv.errors);
    out
}

#[test]
fn a_reward_is_created_at_the_players_level_and_placed_identified() {
    // Level 0 → the player's base level (7); the item is
    // created with full durability (stat 72 = 73 > 0), page 0,
    // then placed in the player's inventory and identified.
    let mut w = World::new();
    let p = w.player;
    w.set_stat(p, stat::LEVEL, 7);
    let (item, placed) = lent(&mut w, |econ, inv| {
        let item = create_reward(econ, p, *b"cap ", 0, 2)
            .expect("no economy error")
            .expect("created");
        (item, place_reward(econ, inv, p, item))
    });
    assert_eq!(placed, Placed::Stored(item));
    let it = w.items.get(item).unwrap();
    assert_eq!(it.ilvl, 7);
    assert_eq!(it.quality, 2);
    assert_ne!(it.flags & flag::IDENTIFIED, 0);
    assert_eq!(it.inv_page, 0);
    let max = w.stats.unit_total(item, stat::MAXDURABILITY, 0);
    assert!(max > 0);
    assert_eq!(w.stats.unit_total(item, stat::DURABILITY, 0), max);
    // Created in mode 4, stored by the placement (mode 0).
    assert_eq!(
        w.units.get(item).unwrap().mode,
        u32::from(moves::mode::STORED)
    );
    assert!(w.state.holds(p, item), "in the player's inventory");
}

#[test]
fn an_explicit_level_is_used_and_an_unknown_code_makes_nothing() {
    let mut w = World::new();
    let p = w.player;
    let seed = w.fields.seed;
    // An unknown code → none, before any draw.
    let none = lent(&mut w, |econ, _| create_reward(econ, p, *b"zzz ", 0, 2));
    assert_eq!(none, Ok(None));
    assert!(w.items.is_empty());
    assert_eq!(w.fields.seed, seed);
    // A level ≠ 0 is the item level; a level ≤ 0 becomes 1.
    let a = lent(&mut w, |econ, _| create_reward(econ, p, *b"key ", 5, 2));
    let b = lent(&mut w, |econ, _| create_reward(econ, p, *b"key ", -3, 2));
    assert_eq!(w.items.get(a.unwrap().unwrap()).unwrap().ilvl, 5);
    assert_eq!(w.items.get(b.unwrap().unwrap()).unwrap().ilvl, 1);
    // A player's base level, at least 1.
    w.set_stat(p, stat::LEVEL, 0);
    let econ = Economy {
        game: &mut w.game,
        units: &mut w.units,
        stats: &mut w.stats,
        data: &w.data,
        hooks: &mut w.hooks,
        fields: &mut w.fields,
        tables: &w.tables,
        items: &mut w.items,
    };
    assert_eq!(item_level(&econ, p), Some(1));
}

#[test]
fn a_refused_reward_is_dropped_at_the_spot_or_freed() {
    // No inventory for the player → the placement fails. Droppable with a
    // spot → ground placement there (mode 3, in the room, page 0xFF,
    // never-expiring only for quest items: a `key ` expires); without a
    // spot → freed.
    let mut w = World::new();
    let p = w.player;
    w.state.inventories.remove(&p);
    let room = w.room;
    w.game.frame = 50;
    let (dropped, freed) = lent(&mut w, |econ, inv| {
        let a = create_reward(econ, p, *b"cap ", 0, 2).unwrap().unwrap();
        assert_eq!(place_reward(econ, inv, p, a), Placed::Refused(a));
        let spot = Spot { room, x: 12, y: 13 };
        let dropped = drop_or_free(econ, inv, a, Some(spot));
        let b = create_reward(econ, p, *b"key ", 0, 2).unwrap().unwrap();
        assert_eq!(place_reward(econ, inv, p, b), Placed::Refused(b));
        (dropped, (b, drop_or_free(econ, inv, b, None)))
    });
    let a = dropped.expect("dropped");
    let g = w.units.get(a).unwrap().guid;
    assert_eq!(w.mode(g), 3);
    assert!(w.in_room(g));
    assert_eq!(
        w.desk(|d| moves::MoveUnits::pos(d, Owner::item(g))),
        (12, 13)
    );
    assert_eq!(w.data(g).page, 0xFF);
    assert_eq!(w.state.expiry[&a], 50 + 15000);
    let (b, out) = freed;
    assert_eq!(out, None);
    assert!(w.items.get(b).is_none());
    assert!(w.units.get(b).is_none());
}

#[test]
fn a_reward_never_rolls_ethereal() {
    use crate::wiring::economy::quest_reward::create_reward_flags;
    // The same `cap ` from the same world, once as the reward and once
    // without the never-ethereal bit: only the second draws the
    // ethereal roll from its item seed.
    let made = |reward: bool| {
        let mut w = World::new();
        let p = w.player;
        let item = lent(&mut w, |econ, _| {
            if reward {
                create_reward(econ, p, *b"cap ", 5, 2)
            } else {
                create_reward_flags(econ, p, *b"cap ", 5, 2, 0)
            }
            .expect("no economy error")
            .expect("created")
        });
        let it = w.items.get(item).unwrap();
        (it.item_seed, it.flags & flag::ETHEREAL)
    };
    let (reward, eth) = made(true);
    let (plain, _) = made(false);
    assert_eq!(eth, 0);
    assert_ne!(reward, plain, "the reward skips the ethereal draw");
}
