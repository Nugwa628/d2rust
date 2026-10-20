// Spec: specs/world/quests-act1.md §10.8 (A1Q6 Sisters to the Slaughter, chain 6), §8.1
// Spec: specs/world/quests-act1-rest.md §5, §8 items 6, 7
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q6 callback by callback: events 0, 2, 3, 8 (Andariel), 10, 11, 13,
//! the portal timer `0x00596500`, the active function and the credit,
//! with the iterate functions O2–O7 (O1 is the shared status iterate).
//! Slot 6 is a constant in each.

use super::{add_state, broadcast, player_flags, rec, restore, send_completed_now, table_state};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, npc, raise_progression, send_player_flags, EventArgs, GuidList,
    QuestControl, QuestError, QuestWorld, TextList, TimerFn,
};

const SLOT: u8 = 6;
const CHAIN: u8 = 6;
/// Catacombs 1–4, Lut Gholein, the Rogue Encampment.
const CATACOMBS: std::ops::RangeInclusive<u32> = 34..=37;
const CATACOMBS_4: u32 = 37;
const LUT_GHOLEIN: u32 = 40;
const TOWN: u32 = 1;
/// The town portal object O7 opens.
const PORTAL: u16 = 59;
/// `0x007382C4`: message state by quest state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];

/// Chipped gems (`0x007361DC`) and normal gems (`0x00736444`).
pub const CHIPPED_GEMS: [[u8; 4]; 7] = [
    *b"gcv ", *b"gcr ", *b"gcb ", *b"gcy ", *b"gcg ", *b"gcw ", *b"skc ",
];
pub const NORMAL_GEMS: [[u8; 4]; 7] = [
    *b"gsv ", *b"gsr ", *b"gsb ", *b"gsy ", *b"gsg ", *b"gsw ", *b"sku ",
];

/// A1Q6's extra data (§10.8).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra6 {
    /// +0x00, +0x84, +0x108: the Cain, Akara and Kashya lists.
    pub cain: GuidList,
    pub akara: GuidList,
    pub kashya: GuidList,
    /// +0x18C: Andariel's GUID.
    pub victim: u32,
    /// +0x192: the portal timer's counter.
    pub counter: u16,
    /// +0x194: Andariel killed.
    pub killed: bool,
}

fn x6(ctl: &mut QuestControl, i: usize) -> &mut Extra6 {
    &mut ctl.records[i].extra.q6
}

/// The gem a quest-seed draw picks (§10.8).
pub fn gem_code(list: &[[u8; 4]; 7], lo: u32) -> [u8; 4] {
    list[(lo % 7) as usize]
}

/// Dispatches chain 6's callbacks; false = no body (unhandled).
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
            // `0x00595B80`.
            let cain = args.target.and_then(|n| w.monster_class(n)) == Some(npc::CAIN5);
            if cain && ctl.records[i].extra.talked {
                broadcast(ctl, w, i, 1, 0);
                ctl.records[i].extra.talked = false;
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x005961C0`: `0x00545530`, then the three lists.
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            ctl.records[i].guids.remove(g);
            let x = x6(ctl, i);
            x.cain.remove(g);
            x.akara.remove(g);
            x.kashya.remove(g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x00596900`.
            if let Some(p) = args.player {
                restore(ctl, w, i, p);
            }
        }
        _ => return false,
    }
    true
}

/// O2 `0x00595BD0` for every player (as A1Q1's I2).
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 if status == 1 => f.set(SLOT, bit::LEAVE_TOWN),
            3 => f.set(SLOT, bit::ENTER_AREA),
            _ => {}
        }
    }
}

/// Credit `0x00596210`: 6.13, 6.1, then the character progression
/// `0x00538680(P's client, 1, difficulty)` (`quests-act1-rest.md` §5).
/// The client is passed without a test and read at once
/// (`quests-act1-rest.md` §9 item 4, `0x00538684`): a player without a
/// client is an invariant violation, reported as fatal.
fn credit<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: UnitId) {
    if let Some(f) = flags_of(w, p) {
        f.set(SLOT, bit::PRIMARY_GOAL_DONE);
        f.set(SLOT, bit::REWARD_PENDING);
    }
    let d = w.difficulty();
    if !raise_progression(w, p, 1, d) {
        ctl.faults.push(QuestError::Fatal(0x0053_8684));
    }
}

/// Adds the player to the three lists and credits it (O3, O4).
fn list_and_credit<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let g = w.guid(p);
    let x = x6(ctl, i);
    x.cain.add(g);
    x.akara.add(g);
    x.kashya.add(g);
    credit(ctl, w, p);
}

/// Event 0 `0x00595E20`.
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let c = args.target.and_then(|n| w.monster_class(n));
    let g = args.player.map_or(u32::MAX, |p| w.guid(p));
    let r = rec(w, args.player);
    let x = &ctl.records[i].extra.q6;
    let listed = match c {
        Some(npc::CAIN5) => x.cain.contains(g),
        Some(npc::AKARA) => x.akara.contains(g),
        Some(npc::KASHYA) => x.kashya.contains(g),
        _ => false,
    };
    if listed {
        return add_state(ctl, w, i, list, args.target, 3);
    }
    if r.get(SLOT, bit::REWARD_PENDING) {
        let k = if matches!(c, Some(npc::CAIN5 | npc::AKARA | npc::KASHYA)) {
            4
        } else {
            3
        };
        return add_state(ctl, w, i, list, args.target, k);
    }
    if ctl.records[i].guids.contains(g) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    let state = ctl.records[i].state;
    if state == 1 && c == Some(npc::CAIN5) && !r.get(SLOT, bit::REWARD_GRANTED) {
        return add_state(ctl, w, i, list, args.target, 0);
    }
    if state == 0
        || r.get(SLOT, bit::REWARD_GRANTED)
        || (state >= 4 && !r.get(SLOT, bit::PRIMARY_GOAL_DONE))
    {
        return;
    }
    if let Some(m) = table_state(&MSG_STATE, state) {
        add_state(ctl, w, i, list, args.target, m);
    }
}

/// Event 3 `0x00596010` (a = old level, b = new level).
pub(super) fn changed_level<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
) {
    if CATACOMBS.contains(&args.b) && ctl.records[i].not_intro {
        // State 3, 4 or 5 is kept (`quests-act1-rest.md` §8 item 6).
        let changed = ctl.records[i].state < 3;
        if changed {
            ctl.records[i].state = 3;
        }
        let status = ctl.records[i].status;
        if args.b == CATACOMBS_4 {
            if status < 2 {
                broadcast(ctl, w, i, 2, 0);
                iterate_progress(ctl, w, i);
            } else if changed {
                iterate_progress(ctl, w, i);
            }
        } else if status == 0 {
            ctl.records[i].flags = 0;
            ctl.records[i].status = 1;
            iterate_progress(ctl, w, i);
        } else if changed {
            iterate_progress(ctl, w, i);
        }
    } else if ctl.records[i].state == 4 && args.b == LUT_GHOLEIN {
        ctl.records[i].state = 5;
    } else if args.a == TOWN {
        let g = args.player.map_or(u32::MAX, |p| w.guid(p));
        if !ctl.records[i].guids.0.is_empty() {
            ctl.records[i].guids.remove(g);
        }
        let r = rec(w, args.player);
        if ctl.records[i].state == 2
            && !r.get(SLOT, bit::REWARD_GRANTED)
            && !r.get(SLOT, bit::REWARD_PENDING)
        {
            ctl.records[i].state = 3;
            iterate_progress(ctl, w, i);
        }
    }
}

/// Event 8 `0x005965A0`: Andariel's death (victim = target).
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    let not_intro = ctl.records[i].not_intro;
    if not_intro {
        let killer = args.player.filter(|&k| {
            let f = player_flags(w, k);
            !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING)
        });
        if let Some(k) = killer {
            credit(ctl, w, k);
            if let Some(v) = args.target {
                for list in [&CHIPPED_GEMS, &CHIPPED_GEMS, &NORMAL_GEMS] {
                    let lo = ctl.seed.step();
                    w.drop_item_at(v, gem_code(list, lo), 2);
                }
            }
        }
        for p in w.players() {
            // O3 `0x00596260`.
            let f = player_flags(w, p);
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::COMPLETED_BEFORE)
                && w.unit_level(p) == Some(CATACOMBS_4)
            {
                list_and_credit(ctl, w, i, p);
            }
        }
        for p in w.players() {
            // O4 `0x00596440` / `0x00596320`.
            if !player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in w.party_members(p).unwrap_or_default() {
                let f = player_flags(w, m);
                let in_act1 = w.unit_level(m).is_some_and(|l| l != 0) && w.unit_act(m) == Some(0);
                if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) && in_act1
                {
                    list_and_credit(ctl, w, i, m);
                }
            }
        }
        for p in w.players() {
            // O5 `0x005963E0`.
            let Some(f) = flags_of(w, p) else { continue };
            if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
                f.set(SLOT, bit::COMPLETED_NOW);
                send_completed_now(w, p, CHAIN, 0);
            }
        }
    }
    let g = args.target.map_or(u32::MAX, |v| w.guid(v));
    x6(ctl, i).victim = g;
    x6(ctl, i).counter = 1;
    if not_intro {
        if let Err(e) = ctl.add_timer(CHAIN, TimerFn::AndarielPortals, 1) {
            ctl.faults.push(e);
        }
        for p in w.players() {
            // O6 `0x00596170`.
            let f = player_flags(w, p);
            if f.get(SLOT, bit::REWARD_PENDING) && f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                w.attach_sound(p, 33);
            }
        }
    }
    x6(ctl, i).killed = true;
    let r = &mut ctl.records[i];
    r.state = 4;
    r.callbacks |= 1 << event::PLAYER_LEAVES_GAME;
    r.clear_callback(event::MONSTER_KILLED);
}

/// Timer `0x00596500`; true = remove it.
pub(super) fn portal_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    let x = x6(ctl, i);
    x.counter = x.counter.wrapping_add(1);
    match x.counter {
        10 => {
            for p in w.players() {
                // O7 `0x00596490`: returns 1 for the first player in
                // Catacombs 4 whether or not the portal was created
                // (`quests-act1-rest.md` §8 item 7).
                if w.unit_level(p) != Some(CATACOMBS_4) {
                    continue;
                }
                if let Some((px, py, _)) = w.unit_position(p) {
                    w.create_portal(p, px, py, PORTAL, TOWN);
                }
                break;
            }
            false
        }
        12 => {
            if !matches!(ctl.records[i].status, 3 | 13) {
                broadcast(ctl, w, i, 3, 0);
            }
            true
        }
        _ => false,
    }
}

/// Event 11 `0x00595C60` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let g = w.guid(p);
    let refresh = |ctl: &mut QuestControl, w: &mut W| {
        if let Some(n) = args.target {
            ctl.refresh_text(w, p, n);
        }
    };
    match (args.a as u16, args.b) {
        (npc::CAIN5, 166) => {
            ctl.records[i].state = 2;
            ctl.records[i].extra.talked = true;
            iterate_progress(ctl, w, i);
            refresh(ctl, w);
        }
        (npc::CAIN5, 184) => x6(ctl, i).cain.remove(g),
        (npc::WARRIV1, 183) => {
            refresh(ctl, w);
            let r = player_flags(w, p);
            if r.get(SLOT, bit::REWARD_PENDING) {
                if r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                    let rd = &mut ctl.records[i];
                    rd.flags = 0;
                    rd.status = 13;
                    rd.state = 5;
                    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                }
                if let Some(f) = flags_of(w, p) {
                    f.clear(SLOT, bit::REWARD_PENDING);
                    f.set(SLOT, bit::REWARD_GRANTED);
                }
                ctl.records[i].guids.add(g);
                send_player_flags(w, p, 6, 0);
            }
        }
        (npc::AKARA, 179) => x6(ctl, i).akara.remove(g),
        (npc::KASHYA, 181) => x6(ctl, i).kashya.remove(g),
        _ => {}
    }
}

/// Active `0x005967F0` (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let r = player_flags(w, player);
    let g = w.guid(player);
    let rd = &ctl.records[i];
    let x = &rd.extra.q6;
    match npc_class {
        npc::CAIN5 => {
            x.cain.contains(g)
                || (!r.get(SLOT, bit::REWARD_GRANTED)
                    && !r.get(SLOT, bit::REWARD_PENDING)
                    && rd.state == 1)
        }
        npc::WARRIV1 => !r.get(SLOT, bit::REWARD_GRANTED) && r.get(SLOT, bit::REWARD_PENDING),
        npc::AKARA => x.akara.contains(g),
        npc::KASHYA => x.kashya.contains(g),
        _ => false,
    }
}

/// `0x005968C0` (no direct caller found): killed when not-intro ≠ 0,
/// else true.
pub fn killed(ctl: &QuestControl) -> bool {
    ctl.record(CHAIN)
        .is_none_or(|r| !r.not_intro || r.extra.q6.killed)
}
