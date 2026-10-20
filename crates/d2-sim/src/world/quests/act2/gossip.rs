// Spec: specs/world/quests-act2.md §9 (A2Q0, A2Q7, A2Q8, the Act II intro)
// Spec: specs/world/quests-act2-2.md §1 items 2, 3, 15, 18
//! The Act II gossip and intro records: A2Q0 Jerhyn (chain 7, slot 8),
//! the A2Q7 guard (chain 26, slot 30), the A2Q8 guard (chain 27, slot
//! 31) and the Act II intro record (chain 38). Their status functions
//! return false (`act2::status_fn`, `quests-act2-2.md` §1 item 18).

use super::{add_state, guid_of, pf, rec, set_bit};
use crate::units::UnitId;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

/// jerhyn (start).
pub const JERHYN: u16 = 201;
/// cain2.
pub const CAIN2: u16 = 244;
/// act2guard4.
pub const GUARD4: u16 = 377;
/// act2guard5.
pub const GUARD5: u16 = 378;
/// Jerhyn's gossip line.
const MSG_JERHYN: u32 = 253;
/// Cain's line about Act I.
const MSG_CAIN: u32 = 125;
/// act2guard5's line.
const MSG_GUARD5: u32 = 303;
/// `0x005940A0`: the Act I hook chain 7's Cain line calls
/// (`quests-act2-2.md` §1 item 18).
const CAIN_HOOK: u32 = 0x0059_40A0;
/// Chain 26's messages that set 30.13 and install the chat end.
const MSG_GUARD4_INTRO: [u32; 2] = [59, 60];

/// Chain 38: the special class per NPC (meshif1 amazon, drognan
/// sorceress, elzix necromancer, fara paladin, geglash barbarian;
/// warriv2, greiz, lysander none).
const INTRO_NPCS: [(u16, Option<u8>); 8] = [
    (210, Some(0)),
    (177, Some(1)),
    (199, Some(2)),
    (178, Some(3)),
    (200, Some(4)),
    (175, None),
    (198, None),
    (202, None),
];

/// Chain 38 event 11: (message, NPC) pairs that set the NPC's intro bit
/// (the NPC of each message from `quest-messages.tsv` table
/// `0x00738D60`).
const INTRO_MSGS: [(u32, u16); 13] = [
    (190, 198),
    (203, 199),
    (204, 199),
    (215, 175),
    (230, 200),
    (231, 200),
    (241, 210),
    (242, 210),
    (263, 178),
    (264, 178),
    (274, 202),
    (285, 177),
    (286, 177),
];

/// Dispatches one callback of chains 7, 26, 27, 38; false = no body.
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match (ctl.records[i].chain, args.event) {
        (7, event::NPC_ACTIVATE) => jerhyn_text(ctl, w, i, args, list),
        (7, event::PLAYER_LEAVES_GAME) => {
            // `0x005987B0` (`quests-act2-2.md` §1 item 2): the extra list,
            // no bit test and no count test.
            let g = guid_of(w, args.player);
            ctl.records[i].extra.a2.q0.remove(g);
        }
        (7, event::SCROLL_MESSAGE) => jerhyn_scroll(ctl, w, i, args),
        (7, event::PLAYER_STARTED_GAME) => {
            // `0x005987D0`.
            if rec(w, args.player).get(8, bit::REWARD_GRANTED) {
                ctl.game.set(8, bit::PRIMARY_GOAL_DONE);
            }
        }
        (26, event::NPC_ACTIVATE) => guard4_text(ctl, w, i, args, list),
        (26, event::SCROLL_MESSAGE) => {
            // `0x0059E0E0` (`quests-act2-2.md` §1 item 3): NPC 377 only;
            // 59 / 60 → 30.13 and callback 2 := `0x0059E0B0`; any other
            // message → 30.0.
            if let (Some(p), true) = (args.player, args.a == u32::from(GUARD4)) {
                if MSG_GUARD4_INTRO.contains(&args.b) {
                    set_bit(w, p, 30, bit::PRIMARY_GOAL_DONE);
                    ctl.records[i].callbacks |= 1 << event::NPC_DEACTIVATE;
                } else {
                    set_bit(w, p, 30, bit::REWARD_GRANTED);
                }
            }
        }
        (26, event::NPC_DEACTIVATE) => {
            // `0x0059E0B0` (edge case 1 of `quests-act2-2.md`): extra
            // +0x00 is never written with 1, so the body never runs.
            let class = args.target.and_then(|n| w.monster_class(n));
            if class == Some(GUARD4) && ctl.records[i].extra.a2.q7_chat == 1 {
                ctl.records[i].extra.a2.q7_chat = 0;
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
        // `0x0059E2A0`: a bare `ret`.
        (26, event::MONSTER_KILLED) => {}
        (27, event::NPC_ACTIVATE) => {
            // `0x0059E3F0`.
            let class = args.target.and_then(|n| w.monster_class(n));
            if class == Some(GUARD5) && guard5_open(ctl, w, args.player) {
                add_state(ctl, w, i, list, args.target, 0);
            }
        }
        (27, event::SCROLL_MESSAGE) => {
            // `0x0059E3C0` (`quests-act2-2.md` §1 item 3): NPC 378 and
            // message 303 → 31.0.
            if let (Some(p), MSG_GUARD5, true) = (args.player, args.b, args.a == u32::from(GUARD5))
            {
                set_bit(w, p, 31, bit::REWARD_GRANTED);
            }
        }
        (38, event::NPC_ACTIVATE) => intro_text(ctl, w, i, args, list),
        (38, event::SCROLL_MESSAGE) => {
            // `0x005983E0`; `0x00572360` sets the bit in intro field A
            // (`quests-act2-2.md` §1 item 15), apart from the 0x91 field B.
            let hit = INTRO_MSGS
                .iter()
                .any(|&(m, c)| m == args.b && u32::from(c) == args.a);
            if let (true, Some(p)) = (hit, args.player) {
                w.set_npc_intro(p, args.a as u16);
            }
        }
        _ => return false,
    }
    true
}

/// Active functions: chain 7 `0x00598780`, 26 `0x0059E2C0`, 27
/// `0x0059E4B0`; chain 38's `0x005985C0` returns false
/// (`quests-act2-2.md` §1 item 18).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    let f = pf(w, player);
    match ctl.records[i].chain {
        7 => npc == JERHYN && !f.get(8, bit::REWARD_GRANTED),
        26 => {
            npc == GUARD4
                && !f.get(30, bit::REWARD_GRANTED)
                && (!f.get(30, bit::PRIMARY_GOAL_DONE) || f.get(9, bit::PRIMARY_GOAL_DONE))
        }
        27 => {
            npc == GUARD5
                && ctl.record(13).is_some_and(|r| r.not_intro && r.state < 2)
                && ctl.game.get(11, bit::PRIMARY_GOAL_DONE)
                && !f.get(14, bit::REWARD_PENDING)
                && !f.get(14, bit::REWARD_GRANTED)
        }
        _ => false,
    }
}

/// Chain 7 event 0 `0x005986B0`.
fn jerhyn_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let f = rec(w, args.player);
    match args.target.and_then(|n| w.monster_class(n)) {
        Some(JERHYN) if !f.get(8, bit::REWARD_GRANTED) => {
            add_state(ctl, w, i, list, args.target, 0);
        }
        Some(CAIN2)
            if f.get(4, bit::COMPLETED_NOW)
                && !ctl.records[i].extra.a2.q0.contains(guid_of(w, args.player)) =>
        {
            add_state(ctl, w, i, list, args.target, 1);
            cain_hook(ctl, w, args.player);
        }
        _ => {}
    }
}

/// `0x005940A0(game, player)` (`quests-act2-2.md` §1 item 18): remove
/// the player's GUID (−1 without one) from chain 4's +0xB4 list. The
/// original does not test that chain 4 exists; without its record the
/// call is reported.
fn cain_hook<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: Option<UnitId>) {
    let g = guid_of(w, p);
    match ctl.record_mut(4) {
        Some(r) => r.extra.q4.credited.remove(g),
        None => w.unhandled(7, CAIN_HOOK),
    }
}

/// Chain 7 event 11 `0x00598640` (a = NPC class, b = message).
fn jerhyn_scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a == u32::from(JERHYN) && args.b == MSG_JERHYN {
        set_bit(w, p, 8, bit::REWARD_GRANTED);
        ctl.game.set(8, bit::PRIMARY_GOAL_DONE);
    } else if args.a == u32::from(CAIN2) && args.b == MSG_CAIN {
        let g = w.guid(p);
        ctl.records[i].extra.a2.q0.add(g);
    }
}

/// Chain 26 event 0 `0x0059E140`: draws from the player unit seed.
fn guard4_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return;
    };
    if w.monster_class(n) != Some(GUARD4) {
        return;
    }
    let f = pf(w, p);
    let inline = |w: &mut W| (w.unit_seed(p).step() % 3) as u8 + 2;
    // 30.0 set, or Radament's quest under way (9.0, 9.13, 9.1, game
    // 9.13): the inline step.
    let state = if f.get(30, bit::REWARD_GRANTED)
        || f.get(9, bit::REWARD_GRANTED)
        || f.get(9, bit::PRIMARY_GOAL_DONE)
        || f.get(9, bit::REWARD_PENDING)
        || ctl.game.get(9, bit::PRIMARY_GOAL_DONE)
    {
        inline(w)
    } else if ctl.record(8).is_some_and(|r| !r.not_intro) {
        w.unit_seed(p).roll(3) as u8 + 2
    } else if !f.get(30, bit::PRIMARY_GOAL_DONE) {
        w.unit_seed(p).roll(2) as u8
    } else {
        inline(w)
    };
    add_state(ctl, w, i, list, args.target, state);
}

/// Chain 27's event-0 test (`0x0059E3F0`).
fn guard5_open<W: QuestWorld>(ctl: &QuestControl, w: &mut W, p: Option<UnitId>) -> bool {
    let f = rec(w, p);
    ctl.record(13).is_some_and(|r| r.not_intro && r.state < 2)
        && ctl.record(10).is_none_or(|r| !r.not_intro || r.state >= 4)
        && !f.get(14, bit::REWARD_PENDING)
        && !f.get(14, bit::REWARD_GRANTED)
}

/// Chain 38 event 0 `0x005984C0`.
fn intro_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return;
    };
    let Some(class) = w.monster_class(n) else {
        return;
    };
    let Some(&(_, special)) = INTRO_NPCS.iter().find(|e| e.0 == class) else {
        return;
    };
    if w.npc_intro_heard(p, class) {
        return;
    }
    let state = u8::from(special == Some(w.player_class(p)));
    add_state(ctl, w, i, list, args.target, state);
}
