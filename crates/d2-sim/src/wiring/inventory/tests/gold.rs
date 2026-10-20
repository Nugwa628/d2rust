//! Tests for gold: 0x50 DropGold and gold pickup on real stats and real
//! item creation.

use super::*;
use crate::items::moves::stat::GOLD as STAT_GOLD;

fn gold(w: &World, u: UnitId) -> i32 {
    w.stats.unit_total(u, STAT_GOLD, 0)
}

fn drop_gold(w: &World, amount: u32) -> Vec<u8> {
    msg(0x50, &[w.pguid(), amount])
}

/// Amount 0 → 0, no pile, no draw.
#[test]
fn drop_gold_zero_draws_nothing() {
    let mut w = World::new();
    let p = w.player;
    w.set_stat(p, STAT_GOLD, 5000);
    w.rest.gold = true;
    let seed = w.fields.seed;
    let n = w.game.lists.units_of_type(UnitType::Item).len();
    assert_eq!(w.handle(&drop_gold(&w, 0)), Ok(0));
    assert_eq!(w.fields.seed, seed);
    assert_eq!(w.game.lists.units_of_type(UnitType::Item).len(), n);
    assert_eq!(gold(&w, p), 5000);
}

/// 1500 of 5000 gold: one `gld` pile created through the real item
/// creation (the game seed steps exactly twice), pile gold 1500, placed
/// on the ground at the spot (room list, mode 3, ITEMDROPPED hook); owner
/// set (0); player gold 3500. Refusals: more than the gold → 3; another
/// unit's GUID → 3.
#[test]
fn drop_gold_makes_a_real_pile() {
    let mut w = World::new();
    let p = w.player;
    w.set_stat(p, STAT_GOLD, 5000);
    w.rest.gold = true;
    w.rest.spot = Some(Spot {
        room: w.room,
        x: 10,
        y: 10,
    });
    assert_eq!(w.handle(&drop_gold(&w, 5001)), Ok(3));
    assert_eq!(w.handle(&msg(0x50, &[w.pguid() + 1, 10])), Ok(3));
    let mut seed = w.fields.seed;
    seed.step();
    seed.step();
    let before = w.game.lists.units_of_type(UnitType::Item);

    assert_eq!(w.handle(&drop_gold(&w, 1500)), Ok(0));

    assert_eq!(w.fields.seed, seed);
    let piles: Vec<UnitId> = w
        .game
        .lists
        .units_of_type(UnitType::Item)
        .into_iter()
        .filter(|u| !before.contains(u))
        .collect();
    assert_eq!(piles.len(), 1);
    let pile = piles[0];
    let g = w.units.get(pile).unwrap().guid;
    assert_eq!(w.units.get(pile).unwrap().class, GOLD as u32);
    assert_eq!(gold(&w, pile), 1500);
    assert_eq!(w.mode(g), 3);
    assert!(w.in_room(g));
    assert_eq!(gold(&w, p), 3500);
    assert_eq!(
        w.rest.log,
        [
            format!("quest_item_dropped {g}"),
            format!("set_owner {g} {}", w.pguid()),
        ]
    );
    assert!(w.state.errors.is_empty());
}

/// Without a creation request (the seam's default): no pile, the
/// gold stays; result 0.
#[test]
fn drop_gold_without_creation_keeps_the_gold() {
    let mut w = World::new();
    let p = w.player;
    w.set_stat(p, STAT_GOLD, 5000);
    w.rest.spot = Some(Spot {
        room: w.room,
        x: 10,
        y: 10,
    });
    assert_eq!(w.handle(&drop_gold(&w, 1500)), Ok(0));
    assert_eq!(gold(&w, p), 5000);
}

/// Gold pickup through 0x16 (auto pickup): level 1 (limit
/// 10000), gold 9500, pile 1000 → gold 10000, a new pile of the rest 500
/// at the player (seam), the picked pile left the room and was freed
/// (no immediate delete notice: the tick's room pass announces it).
#[test]
fn gold_pickup_caps_at_the_limit() {
    let mut w = World::new();
    let p = w.player;
    w.set_stat(p, STAT_GOLD, 9500);
    let g = w.ground_item(GOLD, 12, 11);
    let u = w.unit(g).unwrap();
    w.set_stat(u, STAT_GOLD, 1000);
    w.rest.log.clear();
    assert_eq!(w.handle(&pick(g, 0)), Ok(0));
    assert_eq!(gold(&w, p), 10000);
    assert_eq!(w.unit(g), None, "freed");
    assert!(w.game.lists.unit(u).is_none());
    assert_eq!(
        w.rest.log,
        [
            format!("pickup_sound {} {g}", w.pguid()),
            format!("free_collision {g}"),
            format!("rest_pile {} 500", w.pguid()),
        ]
    );
    assert!(w.state.errors.is_empty());
}
