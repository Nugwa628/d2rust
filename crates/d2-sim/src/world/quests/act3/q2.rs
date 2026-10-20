// Spec: specs/world/quests-act3.md §4 (A3Q2 Khalim's Will, chain 16); specs/world/quests-act3-2.md §11.3
//! A3Q2: events 0, 2, 3, 4, 10, 11, 13, the status and active functions,
//! Khalim's chests (operate 57 / 59 / 58), the sewer lever and stairs and
//! the cube hook `0x005B86E0`.

use super::{add_state, install, npc, npc_of, orb_intact, pf, set, status_all, status_silent};
use crate::units::UnitId;
use crate::world::quests::GuidList;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};

const CHAIN: u8 = 16;
const SLOT: u8 = 18;
/// Khalim's Eye, Heart, Brain, Flail and the cubed Will (§4.1).
pub const EYE: [u8; 4] = *b"qey ";
pub const HEART: [u8; 4] = *b"qhr ";
pub const BRAIN: [u8; 4] = *b"qbr ";
pub const FLAIL: [u8; 4] = *b"qf1 ";
pub const WILL: [u8; 4] = *b"qf2 ";
/// Great Marsh.
const GREAT_MARSH: u32 = 77;
/// Bits told about (§4.1).
const TOLD_EYE: u8 = 3;
const TOLD_BRAIN: u8 = 4;
const TOLD_FLAIL: u8 = 5;
const TOLD_HEART: u8 = 6;
const TOLD_WILL: u8 = 7;
/// The sewer object mode once open.
const OPEN: i32 = 2;
/// FX byte of the sewer lever (§Constants).
const FX_LEVER: u8 = 9;
/// Frames from the lever's operate to its event 7.
const LEVER_DELAY: i32 = 30;

/// Chain 16's extra data (§4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the sewer stairs are initialised.
    pub stairs_known: bool,
    /// +0x01: Cain started the quest (chat end pending).
    pub cain_started: bool,
    /// +0x04: the stairs' GUID.
    pub stairs_guid: u32,
    /// +0x08: the stairs' mode.
    pub stairs_mode: i32,
    /// +0x0C: drop count scratch.
    pub drop_count: i32,
    /// +0x10 / +0x14 / +0x18 / +0x1C: eyes, brains, hearts, flails
    /// dropped (live counts).
    pub eyes: i32,
    pub brains: i32,
    pub hearts: i32,
    pub flails: i32,
    /// +0x20: Wills cubed.
    pub wills: i32,
    /// +0x24 / +0x25 / +0x26 / +0x27: eye, brain, heart, flail dropped
    /// once.
    pub eye_dropped: bool,
    pub brain_dropped: bool,
    pub heart_dropped: bool,
    pub flail_dropped: bool,
    /// +0x2C: player list (reset only).
    pub players: GuidList,
}

/// Khalim's chests (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KhalimChest {
    /// Object 405, operate 57 `0x005B8860`, `qhr `.
    Heart,
    /// Object 406, operate 59 `0x005B8A20`, `qbr `.
    Brain,
    /// Object 407, operate 58 `0x005B8940`, `qey `.
    Eye,
}

/// The parts the player holds: (flail, eye, heart, brain).
fn parts<W: QuestWorld>(w: &W, p: UnitId) -> (bool, bool, bool, bool) {
    (
        w.has_item(p, FLAIL),
        w.has_item(p, EYE),
        w.has_item(p, HEART),
        w.has_item(p, BRAIN),
    )
}

/// §4.3 step 2: the first held item whose "told" bit is clear (the Will
/// also needs the orb not smashed): its table state.
fn untold<W: QuestWorld>(ctl: &QuestControl, w: &W, p: UnitId, f: &QuestFlags) -> Option<u8> {
    if w.has_item(p, WILL) && !f.get(SLOT, TOLD_WILL) && orb_intact(ctl) {
        Some(5)
    } else if w.has_item(p, FLAIL) && !f.get(SLOT, TOLD_FLAIL) {
        Some(4)
    } else if w.has_item(p, EYE) && !f.get(SLOT, TOLD_EYE) {
        Some(1)
    } else if w.has_item(p, HEART) && !f.get(SLOT, TOLD_HEART) {
        Some(2)
    } else if w.has_item(p, BRAIN) && !f.get(SLOT, TOLD_BRAIN) {
        Some(3)
    } else {
        None
    }
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
            // `0x005B8020`.
            if npc_of(w, &args) == Some(npc::CAIN3) && ctl.records[i].extra.act3.q2.cain_started {
                status_all(ctl, w, i, 1);
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
        event::CHANGED_LEVEL => {
            // `0x005B7FF0`.
            let r = &mut ctl.records[i];
            if args.b == GREAT_MARSH && r.not_intro && r.state < 3 {
                r.state = 1;
            }
        }
        event::ITEM_PICKED_UP => picked_up(ctl, w, i, args),
        // `0x005B7FE0`: a bare `ret`.
        event::PLAYER_LEAVES_GAME => {}
        event::SCROLL_MESSAGE => {
            // `0x005B8060`.
            let Some(p) = args.player else { return true };
            if args.a != u32::from(npc::CAIN3) {
                return true;
            }
            match args.b {
                543 => {
                    ctl.records[i].state = 2;
                    set(w, p, SLOT, &[bit::STARTED]);
                    ctl.records[i].extra.act3.q2.cain_started = true;
                    install(ctl, i, event::NPC_DEACTIVATE);
                }
                544 => set(w, p, SLOT, &[TOLD_HEART]),
                545 => set(w, p, SLOT, &[TOLD_EYE]),
                546 => set(w, p, SLOT, &[TOLD_BRAIN]),
                547 => set(w, p, SLOT, &[TOLD_FLAIL]),
                548 => set(w, p, SLOT, &[TOLD_WILL]),
                _ => {}
            }
        }
        event::PLAYER_STARTED_GAME => {
            // `0x005B8470`.
            let Some(p) = args.player else { return true };
            let f = pf(w, p);
            if f.get(SLOT, bit::REWARD_GRANTED) {
                return true;
            }
            if f.get(SLOT, TOLD_WILL) {
                status_silent(ctl, i, 7);
                ctl.records[i].state = 2;
            } else {
                if f.get(SLOT, bit::STARTED) {
                    ctl.records[i].state = 2;
                    status_silent(ctl, i, 7);
                }
                let x = &ctl.records[i].extra.act3.q2;
                if x.eyes != 0 || x.brains != 0 || x.hearts != 0 || x.flails != 0 {
                    status_silent(ctl, i, 7);
                    ctl.records[i].state = 2;
                }
            }
        }
        _ => return false,
    }
    true
}

/// Event 0 `0x005BD630` (§4.3; only cain3).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    if npc_of(w, &args) != Some(npc::CAIN3) {
        return;
    }
    let f = pf(w, p);
    let k = if ctl.records[i].state != 0 && !f.get(SLOT, bit::STARTED) {
        Some(0)
    } else if let Some(k) = untold(ctl, w, p, &f) {
        Some(k)
    } else {
        let (flail, eye, heart, brain) = parts(w, p);
        if !(flail || eye || heart || brain) {
            if w.has_item(p, WILL) {
                Some(11)
            } else if f.get(SLOT, bit::STARTED) {
                Some(6)
            } else {
                None
            }
        } else if flail {
            Some(10)
        } else if heart {
            Some(8)
        } else if eye {
            Some(7)
        } else {
            Some(9)
        }
    };
    if let Some(k) = k {
        add_state(ctl, w, i, list, args.target, k);
    }
}

/// Pick-up `0x005B8210` (§4.5): `5D 10 <flags> <v> 0000` to the picker
/// (`0x005B81C0`); the status byte is not written.
fn picked_up<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if !f.get(15, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    let Some(code) = args.target.and_then(|it| w.item_code(it)) else {
        return;
    };
    let (flail, eye, heart, brain) = parts(w, p);
    let v = match code {
        FLAIL => {
            if !eye {
                1
            } else if !brain {
                2
            } else if !heart {
                4
            } else if !f.get(SLOT, TOLD_FLAIL) {
                5
            } else {
                7
            }
        }
        EYE => {
            if !brain {
                2
            } else if !flail {
                3
            } else if !heart {
                4
            } else {
                7
            }
        }
        HEART => {
            if !brain {
                2
            } else if !flail {
                3
            } else if !eye {
                1
            } else {
                7
            }
        }
        BRAIN => {
            if !heart {
                4
            } else if !flail {
                3
            } else if !eye {
                1
            } else {
                7
            }
        }
        WILL => {
            // Edge case 2: 15.0 is re-tested inverted, so nothing is
            // sent; only the status byte moves.
            let r = &mut ctl.records[i];
            if r.status != 0 {
                r.status = 7;
            }
            return;
        }
        _ => return,
    };
    let flags = ctl.records[i].flags;
    w.send(p, &[0x5D, CHAIN, flags, v, 0, 0]);
}

/// Active function `0x005BD500` ("wants to talk", §4.3).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    if npc_class != npc::CAIN3 {
        return false;
    }
    let f = pf(w, player);
    if ctl.records[i].state != 0 && !f.get(SLOT, bit::STARTED) {
        return true;
    }
    if untold(ctl, w, player, &f).is_some() {
        return true;
    }
    let (flail, eye, heart, brain) = parts(w, player);
    flail && eye && heart && brain && !f.get(SLOT, TOLD_FLAIL)
}

/// Status function `0x005BD850` (§4.9, always reports; 18.0 is never
/// tested, edge case 16).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    r: &QuestFlags,
) -> u8 {
    if !r.get(15, bit::REWARD_GRANTED) {
        return 0;
    }
    // The initial out := 1 (18.2, or not-intro with state ≥ 2) is
    // overwritten by every branch below.
    if w.has_item(player, WILL) {
        return if orb_intact(ctl) { 6 } else { 12 };
    }
    let (flail, eye, heart, brain) = parts(w, player);
    let n = [flail, eye, heart, brain].iter().filter(|&&h| h).count();
    if n == 4 {
        if r.get(SLOT, TOLD_FLAIL) {
            5
        } else {
            7
        }
    } else if n > 0 {
        if !eye {
            1
        } else if !brain {
            2
        } else if !heart {
            4
        } else if flail {
            7
        } else {
            3
        }
    } else if r.get(SLOT, bit::STARTED) || ctl.records[i].state > 1 {
        1
    } else {
        0
    }
}

/// Khalim's chest operate (§4.6; returns 0). Gold first, then the parts,
/// then the chest's treasure (edge case 3).
pub fn chest_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    chest: KhalimChest,
) {
    if !w.quest_chest_gate(object, player) {
        return;
    }
    let piles = ctl.seed.step() % 5 + 5;
    for _ in 0..piles {
        w.drop_gold(object);
    }
    let code = match chest {
        KhalimChest::Heart => HEART,
        KhalimChest::Brain => BRAIN,
        KhalimChest::Eye => EYE,
    };
    if let Some(i) = ctl.find(CHAIN) {
        ctl.records[i].extra.act3.q2.drop_count = 0;
        // `0x005B8810` / `0x005B87C0` / `0x005B8770` for every player in
        // the game.
        for p in w.players() {
            if !w.has_item(p, code) && !w.has_item(p, WILL) {
                ctl.records[i].extra.act3.q2.drop_count += 1;
            }
        }
        // Item level: the area level of the chest's level (§11.3; the
        // `&level` slot is an output, no incoming value).
        for _ in 0..ctl.records[i].extra.act3.q2.drop_count {
            if w.quest_drop(object, code, 2, None, true).is_some() {
                let x = &mut ctl.records[i].extra.act3.q2;
                match chest {
                    KhalimChest::Heart => {
                        x.hearts += 1;
                        x.heart_dropped = true;
                    }
                    KhalimChest::Brain => {
                        x.brains += 1;
                        x.brain_dropped = true;
                    }
                    KhalimChest::Eye => {
                        x.eyes += 1;
                        x.eye_dropped = true;
                    }
                }
            }
        }
    }
    w.object_treasure(object, player, 4);
}

/// The end-animation event `0x005417D0` type 1 at frame +
/// (`FrameCnt1` >> 8).
fn end_animation<W: QuestWorld>(w: &mut W, object: UnitId) {
    let at = w.frame() + (w.object_anim_length(object) >> 8);
    w.schedule_object_event(object, 1, at);
}

/// Stairs init 41 `0x005B8660`.
pub fn stairs_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let r = &mut ctl.records[i];
    r.extra.act3.q2.stairs_known = true;
    r.extra.act3.q2.stairs_guid = g;
    let mode = if r.not_intro {
        r.extra.act3.q2.stairs_mode
    } else {
        OPEN
    };
    w.set_object_mode(object, mode);
}

/// Stairs operate 44 `0x005B84E0`: only in mode 2, the stairs' warp.
pub fn stairs_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let _ = ctl;
    if w.object_mode(object) == OPEN {
        w.stairs_warp(object, player);
    }
}

/// Lever init 42 `0x005B86B0`: intro → mode 2.
pub fn lever_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    if ctl.record(CHAIN).is_some_and(|r| !r.not_intro) {
        w.set_object_mode(object, OPEN);
    }
}

/// Lever operate 45 `0x005B8530` (returns 0).
pub fn lever_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let _ = player;
    if w.object_mode(object) != 0 {
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = &ctl.records[i].extra.act3.q2;
    if !x.stairs_known || w.object_by_guid(x.stairs_guid).is_none() {
        return;
    }
    w.set_object_mode(object, 1);
    end_animation(w, object);
    ctl.records[i].extra.act3.q2.stairs_mode = OPEN;
    let at = w.frame() + w.tick_rate().ticks(LEVER_DELAY);
    w.schedule_quest_event(object, at);
    ctl.unique_event(w, FX_LEVER);
}

/// Lever object event 7 `0x005B85E0` (class 367).
pub fn lever_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let _ = object;
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = &ctl.records[i].extra.act3.q2;
    if !x.stairs_known {
        return;
    }
    let (guid, mode) = (x.stairs_guid, x.stairs_mode);
    match w.object_by_guid(guid) {
        None => ctl.records[i].extra.act3.q2.stairs_mode = OPEN,
        Some((stairs, _)) => {
            w.set_object_mode(stairs, mode);
            if mode != OPEN {
                ctl.records[i].extra.act3.q2.stairs_mode = OPEN;
                // The stairs' own `FrameCnt1` >> 8, no + 1
                // (`0x005B862A`–`0x005B864A`).
                end_animation(w, stairs);
            }
        }
    }
}

/// Cubing the Will `0x005B86E0` (`world/cube.md` §8): reads 18.0 / 18.1
/// to no effect; the part counts −1 each, Wills +1; nothing sent.
pub fn will_cubed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let _ = (w, player);
    if let Some(r) = ctl.record_mut(CHAIN) {
        let x = &mut r.extra.act3.q2;
        x.eyes -= 1;
        x.hearts -= 1;
        x.brains -= 1;
        x.flails -= 1;
        x.wills += 1;
    }
}
