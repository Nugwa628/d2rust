//! Tests for NPC ↔ units, stats, timers, quests and the vendors' pay: the
//! real NPC handlers on real unit records, stat lists, the timer queue
//! and the game's quests.

use super::*;
use crate::tick::events::event;
use crate::world::hirelings::{flags as hflags, PetNode};
use crate::world::npc::hire::{hire_init, resurrect_cost};
use crate::world::npc::{code, talk, AI_PARAM, SOUND_HEAL};

/// Talk 0x13 to `npc`.
fn talk_to(w: &mut World, player: UnitId, npc: UnitId) {
    let m = msg(0x13, &[1, w.guid(npc)]);
    let r = w.desk(|d, ctl| ctl.interact(d, player, &m));
    assert_eq!(r, Ok(Some(0)));
}

fn timers(w: &World, u: UnitId) -> Vec<(u8, i32)> {
    let t = &w.game.timers;
    let mut v: Vec<_> = t
        .unit_timers(u)
        .into_iter()
        .filter_map(|id| Some((t.event(id)?.0, t.expire(id)?)))
        .collect();
    v.sort();
    v
}

/// A plain stat list for `state` on the unit, with the state on.
fn state_list(w: &mut World, u: UnitId, state: u16) {
    let guid = w.guid(u);
    let l = w.stats.alloc(0, 0, 0, guid);
    w.stats.set_state(l, u32::from(state));
    w.stats.attach(&mut w.hooks, u, l, true);
    w.stats.toggle_state(u, u32::from(state), true);
}

fn has_list(w: &World, u: UnitId, state: u16) -> bool {
    let r = w.stats.unit_list(u).unwrap();
    w.stats.list_of_state(r, u32::from(state)).is_some()
}

#[test]
fn talk_starts_on_real_units_timers_and_quests() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::AKARA);
    w.game.frame = 40;
    let guid = w.guid(npc);
    talk_to(&mut w, player, npc);
    // Path cleared, AI parameter 0x28, the NPC's think rescheduled on the
    // real queue at frame + 1.
    assert_eq!(
        w.rest.log[..3],
        [
            format!("clear path {}", npc.0),
            format!("ai param {} {AI_PARAM:#x}", npc.0),
            format!("clear path {}", player.0),
        ]
    );
    assert_eq!(timers(&w, npc), [(event::AI_THINK, 41)]);
    // The interaction start: the player's node, the interact unit, then
    // 0x27, 0x29 (game flags, real QuestControl) and 0x28 (the player's
    // flags).
    assert_eq!(w.state.lists[&npc].nodes, [(player, talk::TALKING)]);
    assert_eq!(w.units.get(player).unwrap().interact.get(), Some((1, guid)));
    let ids: Vec<u8> = w.rest.sent.iter().map(|(_, m)| m[0]).collect();
    assert_eq!(ids, [0x27, 0x29, 0x28]);
    assert_eq!(w.rest.sent[0].1[2..6], guid.to_le_bytes());
    assert_eq!(w.rest.sent[1].1[1..], w.quests.game.0);
    w.assert_clean();
}

#[test]
fn chat_open_heals_on_real_stats_and_state_lists() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::AKARA);
    w.set(
        player,
        &[
            (st::MAXHP, 25600),
            (st::LIFE, 1000),
            (st::MAXMANA, 5120),
            (st::MANA, 0),
            (st::MAXSTAMINA, 7680),
            (st::STAMINA, 7680),
        ],
    );
    // Poison (state 2) and a curable state come off; a plain one stays.
    for s in [2, S_CURABLE, S_PLAIN] {
        state_list(&mut w, player, s);
    }
    talk_to(&mut w, player, npc);
    let m = msg(0x2F, &[1, w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.chat_open(d, player, &m)), 0);
    assert_eq!(w.state.lists[&npc].nodes, [(player, talk::CHATTING)]);
    assert_eq!(w.stat(player, st::LIFE), w.stats.max_life(player));
    assert_eq!(w.stat(player, st::MANA), 5120);
    assert_eq!(w.stat(player, st::STAMINA), 7680);
    assert!(!has_list(&w, player, 2));
    assert!(!has_list(&w, player, S_CURABLE));
    assert!(has_list(&w, player, S_PLAIN));
    // The life and mana sets went out as SetStat; stamina was full.
    let sent: Vec<&[u8]> = w.rest.sent.iter().map(|(_, m)| m.as_slice()).collect();
    assert!(sent.contains(&&[0x1E, 6, 0x00, 0x64][..]));
    assert!(sent.contains(&&[0x1E, 8, 0x00, 0x14][..]));
    assert!(!sent.iter().any(|m| m[0] >= 0x1D && m[1] == 10));
    // The heal sound is queued on the NPC for every client (target 0).
    let slot = w.game.sounds.get(npc).expect("the heal sound is queued");
    assert_eq!((slot.event, slot.target), (SOUND_HEAL, None));
    w.assert_clean();
}

#[test]
fn cain_identify_pays_from_real_gold() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::CAIN2);
    w.set(player, &[(st::GOLD, PLAYER_GOLD)]);
    let items = [w.spawn(UnitType::Item, 0), w.spawn(UnitType::Item, 0)];
    w.rest.inventory = items.iter().map(|&i| backpack(i)).collect();
    talk_to(&mut w, player, npc);
    let m = msg(0x34, &[w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.identify(d, player, &m)), 0);
    // 100 gold per item through the vendors' pay.
    assert_eq!(w.stat(player, st::GOLD), PLAYER_GOLD - 200);
    let ids: Vec<String> = items.iter().map(|i| format!("identify {}", i.0)).collect();
    let log: Vec<&String> = w
        .rest
        .log
        .iter()
        .filter(|l| l.starts_with("identify"))
        .collect();
    assert_eq!(log, ids.iter().collect::<Vec<_>>());
    assert_eq!(
        npc_transactions(&w.rest),
        [(0, code::IDENTIFIED, u32::MAX, (PLAYER_GOLD - 200) as u32)]
    );
    w.assert_clean();
}

#[test]
fn hire_at_asheara_pays_real_gold_for_the_offer() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::ASHEARA);
    let merc = w.spawn(UnitType::Monster, 359);
    w.rest.merc = Some(merc);
    w.set(player, &[(st::GOLD, PLAYER_GOLD), (st::LEVEL, 10)]);
    talk_to(&mut w, player, npc);
    // The talk made the hire list on the NPC-control seed and sent it.
    let slots = w.ctl.record(class::ASHEARA).unwrap().hire.clone().unwrap();
    let offered: Vec<_> = w
        .rest
        .sent
        .iter()
        .filter(|(_, m)| m[0] == 0x4E)
        .map(|(_, m)| u16::from_le_bytes([m[1], m[2]]))
        .collect();
    assert!(!offered.is_empty());
    let name = offered[0];
    let slot = slots
        .slots
        .iter()
        .find(|s| s.name == name)
        .copied()
        .unwrap();
    let want = hire_init(&w.ctl.hirelings, 0, slot.seed, 2, 0, 10).unwrap();
    let mut m = msg(0x36, &[w.guid(npc)]);
    m.extend_from_slice(&name.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    assert_eq!(w.desk(|d, ctl| ctl.hire(d, player, &m)), Ok(0));
    assert_eq!(w.stat(player, st::GOLD), PLAYER_GOLD - want.price as i32);
    let rec = w.ctl.record(class::ASHEARA).unwrap();
    assert!(rec
        .hire
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .any(|s| s.name == name && s.hired));
    // The init: the pet node, the flags, the offer's level, the 0x81
    // broadcast.
    let node = PetNode {
        dead: false,
        guid: w.guid(merc),
        seed: slot.seed,
        name,
        id: 15,
    };
    assert_eq!(w.state.hirelings.list(player).unwrap().nodes, vec![node]);
    let f = w.units.get(merc).unwrap().flags;
    assert_eq!(
        f & (hflags::INIT | hflags::OWNED),
        hflags::INIT | hflags::OWNED
    );
    assert_eq!(w.stats.unit_base(merc, st::LEVEL, 0), want.level);
    assert!(w
        .rest
        .sent
        .iter()
        .any(|(p, m)| *p == player && m[0] == 0x81));
    let last = *npc_transactions(&w.rest).last().unwrap();
    assert_eq!(last.1, code::MERC);
    assert_eq!(last.2, w.guid(merc));
    w.assert_clean();
}

#[test]
fn resurrect_revives_a_real_mercenary() {
    let mut w = World::new(true);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::ASHEARA);
    let merc = w.spawn(UnitType::Monster, 359);
    w.set(player, &[(st::GOLD, PLAYER_GOLD)]);
    w.set(merc, &[(st::LEVEL, 10), (st::MAXHP, 5120), (st::LIFE, 0)]);
    w.units.get_mut(merc).unwrap().flags |= 0x10000;
    // A dead hireling node.
    let node = PetNode {
        dead: true,
        guid: w.guid(merc),
        seed: 7,
        name: 10,
        id: 15,
    };
    w.state.hirelings.list_mut(player).nodes.push(node);
    w.state.hirelings.list_mut(player).max = 1;
    talk_to(&mut w, player, npc);
    let m = msg(0x62, &[w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.resurrect(d, player, &m)), 0);
    let cost = resurrect_cost(10);
    assert_eq!(cost, 750);
    assert_eq!(w.stat(player, st::GOLD), PLAYER_GOLD - 750);
    assert_eq!(w.units.get(merc).unwrap().flags & 0x10000, 0);
    assert_eq!(w.stat(merc, st::LIFE), w.stats.max_life(merc));
    assert!(w.rest.log.contains(&format!("mode {} 1", merc.0)));
    // The node living again (0x81), the hireling warped to the player.
    assert!(!w.state.hirelings.list(player).unwrap().nodes[0].dead);
    assert!(w.rest.sent.iter().any(|(_, m)| m[0] == 0x81));
    assert!(w.rest.sent.iter().any(|(_, m)| m[0] == 0x9B));
    let last = *npc_transactions(&w.rest).last().unwrap();
    assert_eq!((last.1, last.2), (code::MERC, w.guid(merc)));
    w.assert_clean();
}

#[test]
fn a_hireling_level_up_queues_event_0x5b_on_the_merc_for_its_player() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let merc = w.spawn(UnitType::Monster, 0);
    w.desk(|d, _| {
        d.with_hirelings(|v, _, _| {
            crate::world::hirelings::HirelingWorld::level_events(v, player, merc)
        })
    })
    .expect("the hireling tables are loaded");
    let slot = w
        .game
        .sounds
        .get(merc)
        .expect("the level-up event is queued");
    assert_eq!((slot.event, slot.target), (0x5B, Some(player)));
    assert!(w.game.sounds.get(player).is_none());
    w.assert_clean();
}
