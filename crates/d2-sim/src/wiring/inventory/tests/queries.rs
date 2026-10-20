//! Tests for the item queries and socket effects the desk answers itself
//! (`queries.rs`): two-handed, ammo type, spell, the inputs gathered for the
//! level requirement, and the runeword step of socketing (matching a runes
//! row, running its properties, scheduling the replenish event).

use super::*;
use crate::items::tables::{AffixRec, RuneRec, SetItemRec, SkillRec, UniqueRec};
use crate::items::{flag, stat};
use crate::wiring::inventory::queries::{STAT_LEVELREQ, STAT_SINGLESKILL};

/// Two-handed (items `2handed`) and the ammo type (the primary type's
/// `shoots`), from the tables.
#[test]
fn two_handed_and_ammo_from_the_tables() {
    let mut w = World::new();
    let s = w.ground_item(TWO_HANDER, 20, 20);
    let u = w.unit(s).unwrap();
    assert!(!w.desk(|d| d.is_two_handed(u)));
    w.inv.items[TWO_HANDER].twohanded = 1;
    assert!(w.desk(|d| d.is_two_handed(u)));
    assert_eq!(w.desk(|d| d.ammo_of(u)), Some(-1), "the link's miss");
    w.tables.itemtypes[usize::from(T_SWOR)].shoots = 0;
    assert_eq!(w.desk(|d| d.ammo_of(u)), None, "blank shoots: no ammo type");
    w.tables.itemtypes[usize::from(T_SWOR)].shoots = 5;
    assert_eq!(w.desk(|d| d.ammo_of(u)), Some(5));
}

/// The book / scroll spell is the item's suffix slot 0.
#[test]
fn spell_is_suffix_slot_0() {
    let mut w = World::new();
    let k = w.ground_item(KEY, 20, 20);
    let u = w.unit(k).unwrap();
    w.items.get_mut(u).unwrap().suffix[0] = 9;
    assert_eq!(w.desk(|d| d.spell_of(u)), 9);
    assert_eq!(w.desk(|d| moves::MoveUnits::spell(d, k)), 9);
    assert_eq!(w.desk(|d| moves::MoveUnits::spell(d, 0xDEAD)), 0);
}

fn affix(levelreq: i32, class: u8, classlevelreq: i32) -> AffixRec {
    AffixRec {
        levelreq,
        class,
        classlevelreq,
        ..AffixRec::default()
    }
}

/// Level requirement inputs: items `levelreq`, the magic affix rows by id (combined
/// index + 1; the class value for the player's class), the unique and
/// set `lvl req` of the file index, the fillers (recursive), stat 92;
/// the result runs `items::inventory::level_requirement`.
#[test]
fn level_requirement_gathers_its_inputs() {
    let mut w = World::new();
    let p = w.player;
    w.tables.magic = vec![
        affix(7, 0xFF, 0),
        affix(20, CLASS as u8, 15),
        affix(30, 0, 1),
    ];
    let c = w.ground_item(CAP, 20, 20);
    let u = w.unit(c).unwrap();
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 0);
    w.inv.items[CAP].levelreq = 3;
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 3);
    // Magic: prefix 0, suffix 0, automagic; the class value for class 4.
    {
        let it = w.items.get_mut(u).unwrap();
        it.quality = q::MAGIC;
        it.prefix[0] = 2;
        it.suffix[0] = 1;
    }
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 15);
    assert_eq!(w.desk(|d| d.item_level_requirement(u, None)), 20);
    // An unknown id is skipped.
    w.items.get_mut(u).unwrap().prefix[0] = 99;
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 7);
    // Unique and set rows of the file index.
    w.tables.uniques = vec![UniqueRec {
        lvl_req: 41,
        ..UniqueRec::default()
    }];
    w.tables.setitems = vec![SetItemRec {
        lvl_req: -5,
        ..SetItemRec::default()
    }];
    {
        let it = w.items.get_mut(u).unwrap();
        it.quality = q::UNIQUE;
        it.file_index = 0;
    }
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 41);
    w.items.get_mut(u).unwrap().quality = q::SET;
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 3);
    // A filler with a higher requirement, then stat 92 on top.
    w.items.get_mut(u).unwrap().quality = q::NORMAL;
    w.state.add_inventory(u, UnitKind::Item, c);
    let f = w.ground_item(KEY, 21, 20);
    let fu = w.unit(f).unwrap();
    w.inv.items[KEY].levelreq = 25;
    w.desk(|d| {
        let mut inv = d.state.inventories.remove(&u).unwrap();
        inv.link(d, fu, None);
        d.state.inventories.insert(u, inv);
    });
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 25);
    w.set_stat(u, STAT_LEVELREQ, 4);
    assert_eq!(w.desk(|d| d.item_level_requirement(u, Some(p))), 29);
    // The InvWorld seam answers the same for the player.
    assert_eq!(
        w.desk(|d| crate::items::inventory::InvWorld::level_requirement(d, u, p)),
        29
    );
}

/// Stat 107 entries count only from an extended list; the item's plain
/// list gives none (the layer is the skill id).
#[test]
fn single_skill_entries_need_an_extended_list() {
    let mut w = World::new();
    w.tables.skills = vec![SkillRec {
        itypea1: -1,
        reqlevel: 18,
        maxlvl: 20,
        charclass: 0xFF,
    }];
    let c = w.ground_item(CAP, 20, 20);
    let u = w.unit(c).unwrap();
    w.stats.unit_set(&mut w.hooks, u, STAT_SINGLESKILL, 1, 0);
    let extended = w.stats.unit_list(u).is_some_and(|l| w.stats.is_extended(l));
    let want = if extended { 18 } else { 0 };
    assert_eq!(
        w.desk(|d| d.level_req_item(u).single_skills.len()),
        usize::from(extended)
    );
    assert_eq!(w.desk(|d| d.item_level_requirement(u, None)), want);
}

/// A sword with two keys socketed and a runes row naming them.
fn runeword_world(server: u8) -> (World, UnitId) {
    let mut w = World::new();
    w.tables.runes = vec![RuneRec {
        complete: 1,
        server,
        itype: [T_WEAP as i16, 0, 0, 0, 0, 0],
        runes: [KEY as i32, KEY as i32, 0, 0, 0, 0],
        ..RuneRec::default()
    }];
    let s = w.ground_item(SWORD, 20, 20);
    let u = w.unit(s).unwrap();
    w.set_stat(u, stat::NUMSOCKETS, 2);
    w.state.add_inventory(u, UnitKind::Item, s);
    for x in 0..2 {
        let k = w.ground_item(KEY, 30 + x, 20);
        let ku = w.unit(k).unwrap();
        w.desk(|d| {
            let mut inv = d.state.inventories.remove(&u).unwrap();
            inv.link(d, ku, None);
            d.state.inventories.insert(u, inv);
        });
    }
    (w, u)
}

/// The matching row runs, sets item flag 0x4000000 (also in the inventory
/// copy) and, with stat 252 set, schedules the replenish event 3. A
/// `server` row needs a ladder game; a socket count other than the filler
/// count matches no row.
#[test]
fn runeword_activation_on_the_socketed_item() {
    let (mut w, u) = runeword_world(0);
    w.set_stat(u, stat::REPLENISH_DURABILITY, 10);
    assert!(w.desk(|d| d.activate_runeword_on(u)));
    assert_ne!(w.items.get(u).unwrap().flags & flag::RUNEWORD, 0);
    assert_ne!(w.state.items[&u].flags & flag::RUNEWORD, 0);
    let events: Vec<_> = w
        .game
        .timers
        .unit_timers(u)
        .into_iter()
        .filter_map(|id| w.game.timers.event(id))
        .filter(|&(e, _, _)| e == 3)
        .collect();
    assert_eq!(events.len(), 1);
    assert!(w.state.errors.is_empty());

    let (mut w, u) = runeword_world(1);
    assert!(
        !w.desk(|d| d.activate_runeword_on(u)),
        "server row, no ladder"
    );
    assert_eq!(w.items.get(u).unwrap().flags & flag::RUNEWORD, 0);
    w.fields.ladder = true;
    assert!(w.desk(|d| d.activate_runeword_on(u)));
    // Fewer sockets than fillers: no row.
    let (mut w, u) = runeword_world(0);
    w.set_stat(u, stat::NUMSOCKETS, 3);
    let (me, g) = (w.me(), w.units.get(u).unwrap().guid);
    assert!(!w.desk(|d| moves::MovePending::runeword(d, me, g)));
}
