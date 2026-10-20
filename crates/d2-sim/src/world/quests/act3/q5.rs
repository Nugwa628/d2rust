// Spec: specs/world/quests-act3.md §7 (A3Q5 The Blackened Temple, chain 19); specs/world/quests-act3-2.md §11.3, §11.5
//! A3Q5: events 0, 2, 3, 8, 10, 11, 13, the active function, the council
//! registration, the Compelling Orb (init 60, operate 53), stairs R
//! (init 53) and the Durance warp check.

use super::{
    add_guid, add_state, clear, completion_flag, guid_listed, in_act3, install, list_remove, npc,
    npc_of, pf, quick_remove, refresh, s_ab, sequence, set, sound, status_all, status_silent,
    table_state, DOCKS,
};
use crate::units::UnitId;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

const CHAIN: u8 = 19;
const SLOT: u8 = 21;
/// `0x00740ED8`: message table state by record state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// Kurast Causeway.
const CAUSEWAY: u32 = 82;
/// Durance of Hate 2.
const DURANCE_2: u32 = 101;
/// Unit flags of the orb's monster (+0xC4).
const ORB_FLAGS: u32 = 0x20000;
/// Khalim's five items (§7.3, §7.7).
const KHALIM_ITEMS: [[u8; 4]; 5] = [*b"qey ", *b"qhr ", *b"qbr ", *b"qf1 ", *b"qf2 "];
const FLAIL: [u8; 4] = *b"qf1 ";
const WILL: [u8; 4] = *b"qf2 ";
const CUBE: [u8; 4] = *b"box ";

/// Chain 19's extra data (§7.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the last killed council member's GUID.
    pub last_council: u32,
    /// +0x04: Ormus started the quest (chat end pending).
    pub ormus_started: bool,
    /// +0x05: the council was seen.
    pub council_seen: bool,
    /// +0x08: the starting player had 17.0.
    pub had_lam: bool,
    /// +0x0C: the Compelling Orb is smashed.
    pub orb_smashed: bool,
    /// +0x0D: the flail was dropped.
    pub flail_dropped: bool,
    /// +0x0E: the cube was dropped.
    pub cube_dropped: bool,
    /// +0x10: the council GUIDs (up to 6).
    pub council: Vec<u32>,
    /// +0x28: the orb monster was spawned; +0x2C its GUID.
    pub orb_spawned: bool,
    pub orb_guid: u32,
    /// +0x30: council registered.
    pub registered: i32,
    /// +0x34: council left to kill.
    pub left: i32,
    /// +0x38: orb hits.
    pub hits: i32,
    /// +0x3C / +0x40: flails, cubes to drop.
    pub flails_to_drop: i32,
    pub cubes_to_drop: i32,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q5
}

/// Dispatches the chain's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => {
            // Chat end `0x005BB0C0`.
            if npc_of(w, &args) == Some(npc::ORMUS) && x(ctl, i).ormus_started {
                status_all(ctl, w, i, 2);
                x(ctl, i).ormus_started = false;
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
                flag_iterate(ctl, w, i);
            }
        }
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        // `0x005BB400`.
        event::PLAYER_LEAVES_GAME => list_remove(ctl, w, i, &args),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            if let Some(p) = args.player {
                game_start(ctl, w, i, p);
            }
        }
        _ => return false,
    }
    true
}

/// Flag iterate `0x005BADC0` (§7.2) for every player.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let r = &ctl.records[i];
    let (state, seen) = (r.state, r.extra.act3.q5.council_seen);
    for p in w.players() {
        if pf(w, p).get(SLOT, bit::REWARD_GRANTED) {
            continue;
        }
        if matches!(state, 2 | 3) {
            set(w, p, SLOT, &[2]);
        }
        if seen {
            set(w, p, SLOT, &[3]);
        }
    }
}

/// Event 0 `0x005BAFD0` (§7.3).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let k = if f.get(SLOT, 4) {
        if ctl.records[i].extra.act3.q5.orb_smashed {
            return;
        }
        5
    } else if f.get(SLOT, bit::REWARD_GRANTED) && !guid_listed(ctl, w, i, p) {
        return;
    } else if ctl.records[i].state > 5 {
        if !f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
            return;
        }
        6
    } else if !ctl.records[i].not_intro {
        return;
    } else {
        match table_state(&MSG_STATE, ctl.records[i].state, 7) {
            Some(k) => k,
            None => return,
        }
    };
    add_state(ctl, w, i, list, args.target, k);
}

/// Active `0x005BAD60` (§7.3).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let f = pf(w, player);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return false;
    }
    let r = &ctl.records[i];
    match npc_class {
        npc::ORMUS => r.state == 1,
        npc::CAIN3 => f.get(SLOT, 4) && !r.extra.act3.q5.orb_smashed,
        _ => false,
    }
}

/// Event 11 `0x005BB210` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if pf(w, p).get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    match (args.a, args.b) {
        (a, 594) if a == u32::from(npc::ORMUS) => {
            ctl.records[i].state = s_ab(ctl, 2, 3);
            x(ctl, i).ormus_started = true;
            refresh(ctl, w, p, &args);
        }
        (a, 626) if a == u32::from(npc::CAIN3) && pf(w, p).get(SLOT, 4) => {
            if pf(w, p).get(18, bit::REWARD_GRANTED) {
                for c in KHALIM_ITEMS {
                    w.delete_item(p, c);
                }
            }
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) && ctl.records[i].state != 7 {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                status_silent(ctl, i, 13);
                ctl.records[i].state = 7;
            }
            // Literal reading: the sequence call follows the 21.13 test
            // (§7.3 punctuation), so it runs for every accepted 626.
            sequence(ctl, w, CHAIN);
            set(w, p, SLOT, &[bit::REWARD_GRANTED]);
            clear(w, p, SLOT, &[4]);
            add_guid(ctl, w, i, p);
            refresh(ctl, w, p, &args);
        }
        _ => {}
    }
}

/// Event 3 `0x005BB120` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == CAUSEWAY && ctl.records[i].not_intro && ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
        install(ctl, i, event::NPC_DEACTIVATE);
    }
    if args.a == DOCKS {
        let Some(p) = args.player else { return };
        quick_remove(ctl, w, i, p);
        let f = pf(w, p);
        if matches!(ctl.records[i].state, 2 | 3)
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, 4)
        {
            if ctl.records[i].status < 3 {
                status_silent(ctl, i, 2);
            }
            ctl.records[i].state = s_ab(ctl, 4, 5);
            flag_iterate(ctl, w, i);
        }
    }
}

/// Event 13 `0x005BB480`.
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let f = pf(w, p);
    if f.get(18, bit::REWARD_GRANTED) {
        x(ctl, i).orb_smashed = true;
    }
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, 4) {
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        ctl.records[i].not_intro = false;
        return;
    }
    let had = f.get(17, bit::REWARD_GRANTED);
    x(ctl, i).had_lam = had;
    let r = &mut ctl.records[i];
    if f.get(SLOT, 3) {
        (r.status, r.state) = (3, if had { 4 } else { 5 });
    } else if f.get(SLOT, 2) {
        (r.status, r.state) = (2, if had { 2 } else { 3 });
    }
}

/// `0x00545B50` → `0x005BB550` (§7.5): a council member placed by the
/// preset path.
pub fn council_preset<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    let g = w.guid(unit);
    let e = x(ctl, i);
    e.council_seen = true;
    if e.registered < 6 && !e.council.contains(&g) {
        e.council.push(g);
        let n = e.council.len() as i32;
        e.registered = n;
        e.left = n;
    }
    // Literal reading (§7.5 punctuation): the status test runs whether
    // or not the GUID was new.
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    if (state < 2 && status == 1) || (state >= 2 && status == 3) {
        return;
    }
    status_all(ctl, w, i, 3);
    ctl.records[i].state = s_ab(ctl, 4, 5);
    flag_iterate(ctl, w, i);
}

/// Event 8 `0x005BBC00` (§7.6): victim = target.
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(victim) = args.target else { return };
    if w.monster_class(victim) == Some(npc::COMPELLING_ORB) {
        w.or_unit_flags(victim, ORB_FLAGS);
        return;
    }
    // 1. Count the players still needing a flail or a cube (`0x005BB7B0`).
    if !x(ctl, i).flail_dropped || !x(ctl, i).cube_dropped {
        let (mut flails, mut cubes) = (0, 0);
        for p in w.players() {
            if !in_act3(w, p) {
                continue;
            }
            if !pf(w, p).get(18, bit::REWARD_GRANTED)
                && !w.has_item(p, FLAIL)
                && !w.has_item(p, WILL)
            {
                flails += 1;
            }
            if !w.has_item(p, CUBE) {
                cubes += 1;
            }
        }
        let e = x(ctl, i);
        e.flails_to_drop = flails;
        e.cubes_to_drop = cubes;
    }
    // 2. The flail, else the cube.
    if !x(ctl, i).flail_dropped {
        let mut created = 0;
        for _ in 0..x(ctl, i).flails_to_drop {
            if w.quest_drop(victim, FLAIL, 7, None, false).is_some() {
                created += 1;
            }
        }
        if created > 0 {
            x(ctl, i).flail_dropped = true;
            if let Some(q2) = ctl.record_mut(16) {
                q2.extra.act3.q2.flails += created;
                q2.extra.act3.q2.flail_dropped = true;
            }
        }
    } else if !x(ctl, i).cube_dropped {
        x(ctl, i).cube_dropped = true;
        for _ in 0..x(ctl, i).cubes_to_drop {
            w.quest_drop(victim, CUBE, 2, None, false);
        }
    }
    // 3. The council count.
    if !ctl.records[i].not_intro || x(ctl, i).left <= 0 {
        return;
    }
    x(ctl, i).left -= 1;
    if x(ctl, i).left != 0 {
        return;
    }
    let g = w.guid(victim);
    x(ctl, i).last_council = g;
    ctl.records[i].state = if x(ctl, i).orb_smashed { 7 } else { 6 };
    status_all(ctl, w, i, 4);
    // `0x005BAE20`: players in Act III in the monster's room or adjacent.
    let near = w.players_near(victim);
    for p in w.players() {
        if in_act3(w, p) && near.contains(&p) {
            council_credit(w, p);
        }
    }
    // `0x005BB6C0`: parties of the credited players.
    for p in w.players() {
        if !pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in w.party_members(p).unwrap_or_default() {
            if in_act3(w, m) {
                council_credit(w, m);
            }
        }
    }
    // `0x005BB710`.
    for p in w.players() {
        completion_flag(w, p, SLOT, CHAIN, &[bit::REWARD_GRANTED, 4]);
    }
    // `0x005BB770`.
    for p in w.players() {
        if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, sound::COUNCIL);
        }
    }
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
}

/// `0x005BAE20` / `0x005BB640` body: a player lacking 21.0 and 21.4 gets
/// 21.4 (21.0 with 18.0) and 21.13.
fn council_credit<W: QuestWorld>(w: &mut W, p: UnitId) {
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, 4) {
        return;
    }
    let b = if f.get(18, bit::REWARD_GRANTED) {
        bit::REWARD_GRANTED
    } else {
        4
    };
    set(w, p, SLOT, &[b, bit::PRIMARY_GOAL_DONE]);
}

/// Orb init 60 `0x005BBBA0`.
pub fn orb_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if x(ctl, i).orb_smashed {
        w.set_object_mode(object, 2);
    }
    if !x(ctl, i).orb_spawned {
        if let Some(m) = w.spawn_monster_at_unit(object, npc::COMPELLING_ORB, 1) {
            w.or_unit_flags(m, ORB_FLAGS);
            let g = w.guid(m);
            let e = x(ctl, i);
            e.orb_guid = g;
            e.orb_spawned = true;
        }
    }
}

/// One hand item for [`weapon_in_use`]: its GUID, whether it is of item
/// type 45 `weap` (`0x00629BB0`, equivalent types included) and its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandItem {
    pub guid: u32,
    pub weap: bool,
    pub code: [u8; 4],
}

/// `0x0063BEF0(inventory)` (`quests-act3-2.md` §11.5), what the host's
/// [`QuestWorld::weapon_code`] reads the code of: the weapon in use.
/// `weapon_guid` is inventory +0x1C (−1: none); `left` / `right` are the
/// items at body locations 5 and 4. The left hand is tried first; each
/// must be `weap` with the GUID +0x1C. The swap locations 11 / 12 are
/// never consulted (edge case 2 of part 2).
pub fn weapon_in_use(
    weapon_guid: u32,
    left: Option<HandItem>,
    right: Option<HandItem>,
) -> Option<HandItem> {
    if weapon_guid == u32::MAX {
        return None;
    }
    [left, right]
        .into_iter()
        .flatten()
        .find(|h| h.weap && h.guid == weapon_guid)
}

/// Orb operate 53 `0x005BB980` (returns 0). The weapon test compares
/// the code of the weapon in use ([`weapon_in_use`]) with `qf2 `
/// (`0x005BB9B3`).
pub fn orb_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    if w.object_mode(object) != 0 {
        return;
    }
    if w.weapon_code(player) != Some(WILL) {
        w.attach_sound(player, sound::REFUSED);
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    x(ctl, i).hits += 1;
    if x(ctl, i).hits < 2 {
        return;
    }
    set(
        w,
        player,
        18,
        &[bit::REWARD_GRANTED, bit::PRIMARY_GOAL_DONE],
    );
    w.delete_item(player, WILL);
    let f = pf(w, player);
    if f.get(SLOT, 4) && !f.get(SLOT, bit::REWARD_GRANTED) {
        set(w, player, SLOT, &[bit::REWARD_GRANTED]);
    }
    let g = x(ctl, i).orb_guid;
    if let Some((m, _)) = w.monster_by_guid(g) {
        // `0x005DDFC0(game, M, 0, 0, 0)`, then the orb missile
        // `0x005DFEE0` (`quests-helpers.md` §4.3).
        w.monster_mode_at(m, 0, 0, 0);
        crate::world::quests::helpers::orb_missile(ctl, w, m);
    }
    w.set_object_mode(object, 1);
    let at = w.frame() + (w.object_anim_length(object) >> 8);
    w.schedule_object_event(object, 1, at);
    x(ctl, i).orb_smashed = true;
    ctl.unique_event(w, 10);
    sequence(ctl, w, CHAIN);
    // `0x005BB850`: the operator's party.
    for m in w.party_members(player).unwrap_or_default() {
        let f = pf(w, m);
        if f.get(18, bit::REWARD_GRANTED) {
            for c in KHALIM_ITEMS {
                w.delete_item(m, c);
            }
        } else if in_act3(w, m) {
            set(w, m, 18, &[bit::REWARD_GRANTED, bit::PRIMARY_GOAL_DONE]);
            if !w.is_trading(m) {
                for c in KHALIM_ITEMS {
                    w.delete_item(m, c);
                }
            }
            if f.get(SLOT, 4) && !f.get(SLOT, bit::REWARD_GRANTED) {
                set(w, m, SLOT, &[bit::REWARD_GRANTED]);
            }
        }
    }
}

/// Stairs R init 53 `0x005BBB70` (object 386).
pub fn stairs_r_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    if ctl
        .record(CHAIN)
        .is_some_and(|r| r.extra.act3.q5.orb_smashed)
    {
        w.set_object_mode(object, 2);
    }
}

/// The Durance warp check `0x005BBFA0` (`quests.md` §8.2): open?
/// `from` is the level of the player's room.
pub fn durance_open(ctl: &QuestControl, from: u32) -> bool {
    if from == DURANCE_2 {
        return true;
    }
    ctl.record(CHAIN)
        .is_none_or(|r| r.extra.act3.q5.orb_smashed)
}
