//! Tests for C→S 0x31 message 92 (Kashya, Sisters' Burial Grounds reward)
//! through the desk: the real quest handler grants the reward and the
//! real NPC control block hires the mercenary from Kashya's real hire
//! list.

use super::*;
use crate::world::quests::bit;

/// C→S 0x31: NPC GUID, message index, two padding bytes.
fn quest_msg(guid: u32, index: u16) -> Vec<u8> {
    let mut m = vec![0x31];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&index.to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    m
}

#[test]
fn kashya_reward_hires_from_the_real_hire_list() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::KASHYA);
    let merc = w.spawn(UnitType::Monster, 271);
    w.rest.merc = Some(merc);
    // The talk makes Kashya's hire list on the NPC-control seed.
    let m = msg(0x13, &[1, w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.interact(d, player, &m)), Ok(Some(0)));
    let offered = w
        .ctl
        .record(class::KASHYA)
        .unwrap()
        .hire
        .as_ref()
        .unwrap()
        .slots
        .iter()
        .find(|s| s.offered && !s.hired)
        .copied()
        .expect("an offered slot");
    // Blood Raven's kill gave 2.13 and 2.1.
    let f = &mut w.rest.quests.get_mut(&player).unwrap().flags[0];
    f.set(2, bit::PRIMARY_GOAL_DONE);
    f.set(2, bit::REWARD_PENDING);
    w.rest.sent.clear();
    w.rest.log.clear();
    let m = quest_msg(w.guid(npc), 92);
    assert_eq!(w.desk(|d, ctl| d.quest_message(ctl, player, &m)), 0);
    // The quest side: reward granted, pending cleared, record state 5
    // (2.13 held).
    let f = &w.rest.quests[&player].flags[0];
    assert!(f.get(2, bit::REWARD_GRANTED) && !f.get(2, bit::REWARD_PENDING));
    assert_eq!(w.quests.record(2).unwrap().state, 5);
    // The NPC side: the first offered slot hired, 0x50 with its name, the
    // merc spawned in mode 1 (spread 4) and initialised from the slot.
    let hire = w.ctl.record(class::KASHYA).unwrap().hire.as_ref().unwrap();
    assert!(hire.slots.iter().any(|s| s.name == offered.name && s.hired));
    let sent: Vec<_> = w.rest.sent.iter().map(|(_, m)| m.clone()).collect();
    let mut want = vec![0x50, 2, 0];
    want.extend_from_slice(&offered.name.to_le_bytes());
    want.resize(15, 0);
    assert!(sent.contains(&want));
    // Every Act I callback has a body: only the deferred reward is
    // logged, after the quest call.
    assert_eq!(w.rest.log[0], "spawn merc 271 1");
    // The init: Kashya's `Id` 0 node for the slot.
    let node = w.state.hirelings.list(player).unwrap().nodes[0];
    assert_eq!(
        (node.guid, node.name, node.seed, node.id),
        (w.guid(merc), offered.name, offered.seed, 0)
    );
    w.assert_clean();
}

#[test]
fn kashya_reward_messages_precede_the_text_refresh() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::KASHYA);
    w.rest.merc = Some(w.spawn(UnitType::Monster, 271));
    let m = msg(0x13, &[1, w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.interact(d, player, &m)), Ok(Some(0)));
    let f = &mut w.rest.quests.get_mut(&player).unwrap().flags[0];
    f.set(2, bit::PRIMARY_GOAL_DONE);
    f.set(2, bit::REWARD_PENDING);
    w.rest.trace.clear();
    let m = quest_msg(w.guid(npc), 92);
    assert_eq!(w.desk(|d, ctl| d.quest_message(ctl, player, &m)), 0);
    // 0x28, the GUID, the mercenary (its 0x50, then the hireling's
    // creation: mode 1, spread 4, init), then the text refresh (0x27,
    // 0x29). The hireling's init (`world::hirelings::life::init`, its
    // messages from 0x81 on) runs between the spawn and the refresh.
    let t = &w.rest.trace;
    assert_eq!(t[..3], ["0x28", "0x50", "spawn merc 1"]);
    assert_eq!(t[3], "0x81");
    assert!(t.len() > 5);
    assert_eq!(t[t.len() - 2..], ["0x27", "0x29"]);
    w.assert_clean();
}

#[test]
fn quest_message_without_the_reward_hires_nobody() {
    let mut w = World::new(false);
    let player = w.spawn(UnitType::Player, 0);
    let npc = w.npc(class::KASHYA);
    w.rest.merc = Some(w.spawn(UnitType::Monster, 271));
    let m = msg(0x13, &[1, w.guid(npc)]);
    assert_eq!(w.desk(|d, ctl| ctl.interact(d, player, &m)), Ok(Some(0)));
    // No reward pending: message 92 changes nothing.
    w.rest.log.clear();
    let m = quest_msg(w.guid(npc), 92);
    assert_eq!(w.desk(|d, ctl| d.quest_message(ctl, player, &m)), 0);
    let hire = w.ctl.record(class::KASHYA).unwrap().hire.as_ref().unwrap();
    assert!(hire.slots.iter().all(|s| !s.hired));
    assert!(!w.rest.log.iter().any(|l| l.starts_with("spawn merc")));
    w.assert_clean();
}
