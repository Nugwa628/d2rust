// Spec: specs/world/quests-act1.md §10.6 (A1Q4 The Search for Cain, chain 4), §9.4, §4.6
// Spec: specs/world/quests-act1-rest.md §1–§3, §6, §7, §8 items 1–3, 5
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q4 callback by callback: events 0, 2, 3, 4, 6, 8 (the Cow King), 9,
//! 10, 11, 13, 14, the active function, the tree reset, the Cain cleanup
//! and its timer, the town Cain spawn, the act-change hook, the class-61
//! link, the Cairn stone order and its 0x50, Wirt's body, the tree and
//! stone operate functions, the gibbet operate and quest function, the
//! stone init and its Tristram-portal timer, the town-Cain marker init
//! and "Cain leaves Tristram", with the iterate functions L2–L5 (L1 is
//! the shared status iterate). Slot 4 is a constant in each.

use super::{add_state, broadcast, player_flags, rec, send_completed_now, sequence};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};
use crate::world::quests::{
    bit, event, flags_of, npc, send_player_flags, EventArgs, GuidList, QuestControl, QuestError,
    QuestWorld, TextList, TimerFn,
};

const SLOT: u8 = 4;
const CHAIN: u8 = 4;
/// The Tristram Cain (monster class 146) and Cain in town (cain5).
const CAIN_TRISTRAM: u16 = 146;
/// Tristram, Moo Moo Farm, Lut Gholein, the Rogue Encampment.
const TRISTRAM: u32 = 38;
const COW_LEVEL: u32 = 39;
const LUT_GHOLEIN: u32 = 40;
const TOWN: u32 = 1;
/// Object classes: Cain's gibbet, the Inifuss tree, a Cairn stone, the
/// object linked to chain 4 (§4.6), the Tristram portal.
const GIBBET: u16 = 26;
const TREE: u16 = 30;
const STONE_21: u16 = 21;
pub const LINKED_OBJECT: u16 = 61;
/// The cairnstones missile that opens the Tristram portal.
const CAIRN_MISSILE: u16 = 288;
/// Cain in Tristram leaves through this object (`cain portal`).
const CAIN_PORTAL: u16 = 189;
/// Portal objects: to town, to Tristram.
const PORTAL_TO_TOWN: u16 = 59;
const PORTAL_TO_TRISTRAM: u16 = 60;
/// The first Cairn stone class (`StoneAlpha`); the stones are 17–21.
const STONE_17: u16 = 17;
const SCROLL: [u8; 4] = *b"bks ";
const DECIPHERED: [u8; 4] = *b"bkd ";
/// `0x00737648`: message state by quest state 1–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];

/// A1Q4's extra data (§10.6) beyond the stone order and Wirt's piles.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra4 {
    /// +0x0C: stones touched in order; +0x10 + 4k: their GUIDs.
    pub stones: u16,
    pub stone_guids: [u32; 5],
    /// +0x2C: touch counter without the deciphered scroll.
    pub touches: u16,
    /// +0x28, +0x49: the linked class-61 object.
    pub linked_guid: u32,
    pub linked: bool,
    /// +0x30, +0x47: the Inifuss tree.
    pub tree_guid: u32,
    pub tree_known: bool,
    /// +0x34, +0x48: Cain's gibbet.
    pub gibbet_guid: u32,
    pub gibbet_known: bool,
    /// +0x3C: the player who opened the gibbet (−1 when none).
    pub gibbet_player: u32,
    /// +0x40: the class-17 stone that carries the Tristram portal.
    pub portal_stone: u32,
    /// +0x44: the Tristram-portal timer is pending; +0x45: the portal
    /// was created.
    pub portal_timer: bool,
    pub portal_made: bool,
    /// +0x38: the scroll made by message 112.
    pub scroll_guid: u32,
    /// +0x46: the Cain-removal timer is pending.
    pub removal_timer: bool,
    /// +0x4B–+0x4F progress flags.
    pub b4b: bool,
    pub b4c: bool,
    pub b4d: bool,
    pub b4e: bool,
    pub b4f: bool,
    /// +0x50: Cain gone from Tristram.
    pub cain_gone: bool,
    /// +0x51, +0x52: Cain spawned in town; Cain still to spawn there.
    pub town_cain: bool,
    pub town_cain_due: bool,
    /// +0x54: 3 when the gibbet is open; +0x58: progress marker.
    pub gibbet_open: i32,
    pub progress: i32,
    /// +0x5C–+0x60: per-stone reset bytes (index = class − 17; zeroed
    /// at init, never set).
    pub stone_reset: [bool; 5],
    /// +0x61: the Tristram Cain was removed.
    pub cain_removed: bool,
    /// +0x62: Cain could not be spawned in Tristram.
    pub cain_failed: bool,
    /// +0x63: the reward message sets game 4.13.
    pub game_done_due: bool,
    /// +0x64: scroll deciphered, chat end not yet handled.
    pub deciphered: bool,
    /// +0x66: the town portal out of Tristram made by the gibbet.
    pub out_portal: bool,
    /// +0x68: the town Cain's GUID.
    pub town_cain_guid: u32,
    /// +0x6C, +0x70: the town-Cain marker object (class 385).
    pub marker_guid: u32,
    pub marker_known: bool,
    /// +0x74: scratch, the player found in Tristram (§1.2 step 4).
    pub found_player: Option<UnitId>,
    /// +0x84, +0x88: the marker's position.
    pub marker_pos: (i32, i32),
    /// +0x91 (set by "Cain leaves Tristram"; read by the cain portal's
    /// event 7 outside the Rogue Encampment, `quests-act1-rest.md` §9
    /// item 10).
    pub b91: bool,
    /// +0x80: the cain portals' event-7 count in the Rogue Encampment
    /// (shared by every cain portal of the game, never reset); +0x92:
    /// set once it passes 5 (§9 item 10).
    pub town_portal_count: u32,
    pub b92: bool,
    /// +0x96, +0xA4: the Cain portal object and its GUID.
    pub cain_portal: bool,
    pub cain_portal_guid: u32,
    /// +0x67: the out-of-town portal point was set (`0x005944B0`).
    pub portal_point_set: bool,
    /// +0x95, +0x98: the out-of-town Cain portal object and its GUID
    /// (`0x005943B0`).
    pub out_cain_portal: bool,
    pub out_cain_portal_guid: u32,
    /// +0x9C, +0xA0: the out-of-town portal point.
    pub portal_point: (i32, i32),
    /// +0x78.
    pub b78: bool,
    /// +0x7C: `bks ` / `bkd ` items in the game.
    pub scrolls: i32,
    /// +0x93.
    pub b93: bool,
    /// +0xB4: players credited when Cain reached Act II without them.
    pub credited: GuidList,
    /// +0x138: players who heard Cain's message 123 or 126.
    pub heard: GuidList,
}

fn x4(ctl: &mut QuestControl, i: usize) -> &mut Extra4 {
    &mut ctl.records[i].extra.q4
}

fn scrolls_of<W: QuestWorld>(w: &W, p: UnitId) -> i32 {
    i32::from(w.has_item(p, DECIPHERED)) + i32::from(w.has_item(p, SCROLL))
}

/// Dispatches chain 4's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::ITEM_PICKED_UP => {
            // `0x00592E20`.
            if ctl.records[i].not_intro {
                iterate_progress(ctl, w, i);
            }
        }
        event::EVENT6 => {
            // `0x00592E60` (never raised, §4.1).
            if !x4(ctl, i).b4f && ctl.records[i].state <= 5 {
                ctl.records[i].state = 3;
                tree_reset(ctl, w, i);
            }
        }
        event::MONSTER_KILLED => cow_king(w, args),
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x00592C80` (target = the item).
            let r = &ctl.records[i];
            if r.not_intro && r.state != 6 {
                let code = args.target.and_then(|t| w.item_code(t));
                if matches!(code, Some(SCROLL | DECIPHERED)) {
                    x4(ctl, i).scrolls -= 1;
                }
                let (st, x) = (ctl.records[i].state, &ctl.records[i].extra.q4);
                if x.scrolls == 0 && st <= 5 && !x.b4f && x.tree_known {
                    ctl.records[i].state = 3;
                    tree_reset(ctl, w, i);
                }
            }
        }
        event::PLAYER_LEAVES_GAME => {
            // `0x00592CF0`.
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            let r = rec(w, args.player);
            if r.get(SLOT, bit::REWARD_PENDING)
                && r.get(SLOT, bit::REWARD_GRANTED)
                && !ctl.records[i].guids.0.is_empty()
            {
                ctl.records[i].guids.remove(g);
            }
            x4(ctl, i).credited.remove(g);
            x4(ctl, i).heard.remove(g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x00592B90`.
            if let Some(p) = args.player {
                let n = scrolls_of(w, p);
                x4(ctl, i).scrolls += n;
            }
        }
        _ => return false,
    }
    true
}

/// L2 `0x00592130` for every player.
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3..=5 => f.set(SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// L3 `0x00592B10` for every player.
fn iterate_credit<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        f.set(SLOT, bit::COMPLETED_NOW);
        let g = w.guid(p);
        x4(ctl, i).credited.add(g);
    }
}

/// L4 `0x00593130` for one player: the rescue credit in Tristram, then
/// the party step `0x005930B0` for each member.
fn rescued<W: QuestWorld>(w: &mut W, p: UnitId) {
    let level = w.unit_level(p);
    let Some(f) = flags_of(w, p) else { return };
    if f.get(SLOT, bit::REWARD_GRANTED)
        || f.get(SLOT, bit::REWARD_PENDING)
        || level != Some(TRISTRAM)
    {
        return;
    }
    f.set(SLOT, bit::PRIMARY_GOAL_DONE);
    f.set(SLOT, bit::REWARD_PENDING);
    send_player_flags(w, p, 6, 0);
    party_rescued(w, p);
}

/// For each member of the player's party (`quests-act1-rest.md` §6):
/// `0x005930B0`.
fn party_rescued<W: QuestWorld>(w: &mut W, p: UnitId) {
    for m in w.party_members(p).unwrap_or_default() {
        let in_act1 = w.unit_level(m).is_some_and(|l| l != 0) && w.unit_act(m) == Some(0);
        let Some(f) = flags_of(w, m) else { continue };
        if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) && in_act1 {
            f.set(SLOT, bit::PRIMARY_GOAL_DONE);
            f.set(SLOT, bit::REWARD_PENDING);
            send_player_flags(w, m, 6, 0);
        }
    }
}

/// L5 `0x005931C0` for one player (after L4 in the gibbet function).
pub fn completed_now<W: QuestWorld>(w: &mut W, p: UnitId) {
    let Some(f) = flags_of(w, p) else { return };
    if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
        f.set(SLOT, bit::COMPLETED_NOW);
        send_completed_now(w, p, CHAIN, 0);
    }
}

/// Event 0 `0x00592580`.
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    mut list: Option<&mut TextList>,
) {
    // `quests-act1-rest.md` §8 item 5: the player's data is read first
    // (`0x006221A0` at `0x005925BB`), an internal error without one.
    let Some(p) = args.player else {
        return ctl.faults.push(QuestError::Fatal(0x0059_25BB));
    };
    let c = args.target.and_then(|n| w.monster_class(n));
    let mut add = |ctl: &QuestControl, w: &W, k: u8| {
        add_state(ctl, w, i, list.as_deref_mut(), args.target, k)
    };
    if c == Some(CAIN_TRISTRAM) {
        add(ctl, w, 9);
    }
    if x4(ctl, i).b4f && w.has_item(p, DECIPHERED) {
        w.delete_item(p, DECIPHERED);
        x4(ctl, i).scrolls -= 1;
    }
    let r = player_flags(w, p);
    let g = w.guid(p);
    let cain = c == Some(npc::CAIN5);
    let heard = x4(ctl, i).heard.contains(g);
    if cain && !heard && r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        return add(ctl, w, 5);
    }
    if r.get(SLOT, bit::REWARD_PENDING) {
        return add(ctl, w, if cain && heard { 7 } else { 5 });
    }
    if x4(ctl, i).credited.contains(g) {
        return add(ctl, w, 6);
    }
    if ctl.records[i].guids.contains(g) {
        if cain && !heard {
            add(ctl, w, 5);
        } else if r.get(SLOT, bit::COMPLETED_NOW) {
            add(ctl, w, 8);
        } else if r.get(SLOT, bit::REWARD_GRANTED) {
            add(ctl, w, 7);
        }
        return;
    }
    let state = ctl.records[i].state;
    if r.get(SLOT, bit::COMPLETED_NOW)
        || state == 0
        || r.get(SLOT, bit::REWARD_GRANTED)
        || r.get(SLOT, bit::COMPLETED_BEFORE)
    {
        return;
    }
    if w.has_item(p, SCROLL) {
        add(ctl, w, 3);
    } else if state == 4 {
        add(ctl, w, 2);
    } else if let Some(m) = super::table_state(&MSG_STATE, state) {
        add(ctl, w, m);
    }
}

/// Event 2 `0x005921B0`.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    match args.target.and_then(|n| w.monster_class(n)) {
        Some(npc::AKARA) => {
            if ctl.records[i].extra.talked {
                broadcast(ctl, w, i, 1, 0);
                ctl.records[i].extra.talked = false;
                iterate_progress(ctl, w, i);
            }
            if x4(ctl, i).deciphered {
                broadcast(ctl, w, i, 3, 0);
                iterate_progress(ctl, w, i);
                x4(ctl, i).deciphered = false;
            }
        }
        Some(CAIN_TRISTRAM) => x4(ctl, i).b93 = true,
        _ => {}
    }
}

/// Event 3 `0x00596DE0` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let x = &ctl.records[i].extra.q4;
    if args.b == TRISTRAM && !x.town_cain && !x.cain_gone && ctl.records[i].state >= 6 {
        let r = &mut ctl.records[i];
        r.state = 5;
        r.flags = 0;
        r.status = 4;
        iterate_progress(ctl, w, i);
    }
    let r = rec(w, args.player);
    if args.a == TOWN {
        let g = args.player.map_or(u32::MAX, |p| w.guid(p));
        if !ctl.records[i].guids.0.is_empty() {
            ctl.records[i].guids.remove(g);
        }
        x4(ctl, i).credited.remove(g);
        if r.get(SLOT, bit::REWARD_GRANTED) || r.get(SLOT, bit::REWARD_PENDING) {
            return;
        }
        if ctl.records[i].state == 2 {
            ctl.records[i].state = 3;
        }
    }
    if args.b == TOWN {
        let x = &ctl.records[i].extra.q4;
        if x.marker_known && x.town_cain_due && !x.town_cain {
            if let Some((m, _)) = w.object_by_guid(x.marker_guid) {
                spawn_town_cain_at_marker(ctl, w, i, m);
            }
        }
        return;
    }
    if args.b == LUT_GHOLEIN
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && !r.get(SLOT, bit::REWARD_PENDING)
        && !x4(ctl, i).cain_gone
        && ctl.records[i].state < 6
    {
        to_act2(ctl, w, i);
        x4(ctl, i).progress = 1;
    }
}

/// The common part of event 3's Lut Gholein step and `0x00597310`.
fn to_act2<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    cain_cleanup(ctl, w, i, false, true);
    ctl.records[i].state = 7;
    broadcast(ctl, w, i, 5, 0);
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    iterate_credit(ctl, w, i);
}

/// Act change `0x00597310(game, player)` (§8.1, after the Warriv travel).
pub fn act_change<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let r = player_flags(w, player);
    if !r.get(SLOT, bit::REWARD_GRANTED)
        && !r.get(SLOT, bit::REWARD_PENDING)
        && !x4(ctl, i).cain_gone
        && ctl.records[i].state < 6
    {
        to_act2(ctl, w, i);
    }
}

/// Event 8 `0x00593E70`: the Cow King's death (force dispatch, §4.4).
fn cow_king<W: QuestWorld>(w: &mut W, args: EventArgs) {
    if let Some(k) = args.player {
        let expansion = w.expansion();
        let r = player_flags(w, k);
        if r.get(SLOT, 10)
            || (expansion && !r.get(40, bit::REWARD_GRANTED))
            || (!expansion && !r.get(26, bit::REWARD_GRANTED))
        {
            return;
        }
        if let Some(f) = flags_of(w, k) {
            f.set(SLOT, 10);
        }
    }
    for p in w.players() {
        if w.unit_level(p) == Some(COW_LEVEL) {
            if let Some(f) = flags_of(w, p) {
                f.set(SLOT, 10);
            }
        }
    }
    if let Some(v) = args.target {
        for _ in 0..8 {
            w.drop_item_at(v, *b"vps ", 0);
        }
    }
}

/// Event 11 `0x00592250` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let refresh = |ctl: &mut QuestControl, w: &mut W| {
        if let Some(n) = args.target {
            ctl.refresh_text(w, p, n);
        }
    };
    let r = player_flags(w, p);
    let g = w.guid(p);
    match (args.a as u16, args.b) {
        (npc::AKARA, 97) => {
            ctl.records[i].extra.talked = true;
            ctl.records[i].state = 2;
            refresh(ctl, w);
        }
        (npc::AKARA, 112) if w.has_item(p, SCROLL) => {
            w.delete_item(p, SCROLL);
            match w.reward_item(p, DECIPHERED, 0, 2, true) {
                Some(item) => {
                    let ig = w.guid(item);
                    let x = x4(ctl, i);
                    x.deciphered = true;
                    x.b4b = true;
                    x.b4e = true;
                    x.scroll_guid = ig;
                    let rd = &mut ctl.records[i];
                    rd.state = 5;
                    rd.flags = 0;
                    rd.status = 3;
                }
                None => x4(ctl, i).scrolls -= 1,
            }
        }
        (npc::AKARA, 118) if r.get(SLOT, bit::REWARD_PENDING) => {
            if let Some(f) = flags_of(w, p) {
                f.set(SLOT, bit::REWARD_GRANTED);
                f.clear(SLOT, bit::REWARD_PENDING);
            }
            ctl.records[i].guids.add(g);
            send_player_flags(w, p, 6, 0);
            let (level, quality) = match w.difficulty() {
                0 => (7, 4),
                1 => (30, 6),
                _ => (60, 6),
            };
            w.reward_item(p, *b"rin ", level, quality, true);
            // `0x005458E0`.
            w.send(p, &[0x5D, CHAIN, 2, 0, 0, 0]);
            refresh(ctl, w);
            if r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                ctl.records[i].status = 13;
                ctl.records[i].state = 6;
                if !ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                    sequence(ctl, w, CHAIN);
                }
            }
            if x4(ctl, i).game_done_due {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            }
        }
        (npc::CAIN5, 125) => {
            ctl.records[i].guids.add(g);
            x4(ctl, i).credited.remove(g);
            refresh(ctl, w);
        }
        (npc::CAIN5, 123 | 126) => {
            x4(ctl, i).heard.add(g);
            refresh(ctl, w);
        }
        _ => {}
    }
}

/// Event 13 `0x00597030`.
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let r = player_flags(w, p);
    if r.get(SLOT, bit::REWARD_GRANTED) || r.get(SLOT, bit::COMPLETED_BEFORE) {
        let x = x4(ctl, i);
        x.town_cain_due = true;
        x.gibbet_open = 3;
        x.b4c = true;
        x.b4d = true;
        if r.get(SLOT, bit::REWARD_GRANTED) {
            ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        } else {
            x.game_done_due = true;
        }
    } else if r.get(SLOT, bit::ENTER_AREA) {
        let rd = &mut ctl.records[i];
        (rd.status, rd.state) = (4, 5);
        let x = &mut rd.extra.q4;
        (x.b4c, x.b4d, x.b4e, x.b78) = (true, true, true, true);
        x.progress = 1;
    } else if r.get(SLOT, bit::LEAVE_TOWN) {
        let rd = &mut ctl.records[i];
        (rd.state, rd.status) = (3, 1);
    } else if r.get(SLOT, bit::STARTED) {
        let rd = &mut ctl.records[i];
        (rd.state, rd.status) = (2, 1);
    }
    let n = scrolls_of(w, p);
    let (bkd, bks) = (w.has_item(p, DECIPHERED), w.has_item(p, SCROLL));
    let rd = &mut ctl.records[i];
    rd.extra.q4.scrolls += n;
    if bkd {
        (rd.state, rd.status) = (5, 3);
        let x = &mut rd.extra.q4;
        (x.b4e, x.b78, x.progress) = (true, true, 1);
    } else if bks {
        (rd.state, rd.status) = (4, 2);
        let x = &mut rd.extra.q4;
        (x.b78, x.progress) = (true, 1);
    }
}

/// Active `0x00592FB0` (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let r = player_flags(w, player);
    let rd = &ctl.records[i];
    let g = w.guid(player);
    match npc_class {
        npc::AKARA => {
            r.get(SLOT, bit::REWARD_PENDING)
                || (!r.get(SLOT, bit::REWARD_GRANTED)
                    && (rd.state == 1
                        || (rd.state == 4 && w.has_item(player, SCROLL))
                        || (rd.state == 6 && r.get(SLOT, bit::PRIMARY_GOAL_DONE))))
        }
        npc::CAIN5 => {
            rd.extra.q4.credited.contains(g)
                || (!rd.extra.q4.heard.contains(g) && r.get(SLOT, bit::PRIMARY_GOAL_DONE))
        }
        _ => false,
    }
}

/// Tree reset `0x00592BD0`.
fn tree_reset<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let known = x4(ctl, i).tree_known;
    if known {
        if let Some((o, TREE)) = w.object_by_guid(x4(ctl, i).tree_guid) {
            w.set_object_mode(o, 0);
        }
    }
    broadcast(ctl, w, i, 1, 0);
    ctl.records[i].state = 3;
    x4(ctl, i).progress = 0;
    if known {
        iterate_progress(ctl, w, i);
    }
}

/// Cain cleanup `0x00596CA0(record, timer, remove)`.
fn cain_cleanup<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    timer: bool,
    remove: bool,
) {
    let x = x4(ctl, i).clone();
    let gibbet = x
        .gibbet_known
        .then(|| w.object_by_guid(x.gibbet_guid))
        .flatten()
        .filter(|o| o.1 == GIBBET);
    match gibbet {
        Some((o, _)) if w.object_mode(o) == 3 => {}
        Some((o, _)) => {
            w.set_object_mode(o, 3);
            x4(ctl, i).gibbet_open = 3;
        }
        None => x4(ctl, i).gibbet_open = 3,
    }
    if x.tree_known {
        if let Some((o, _)) = w.object_by_guid(x.tree_guid) {
            w.set_object_mode(o, 1);
        }
    }
    if timer {
        if !x.removal_timer {
            x4(ctl, i).removal_timer = true;
            if let Err(e) = ctl.add_timer(CHAIN, TimerFn::CainRemoval, 1) {
                ctl.faults.push(e);
            }
        }
    } else if remove {
        removal_walk(ctl, w, i);
    }
    let x = x4(ctl, i);
    if remove {
        if !x.cain_removed {
            w.drop_preset_monster(0, CAIN_TRISTRAM);
        }
        x.cain_gone = true;
        if !x.town_cain {
            x.town_cain_due = true;
        }
    } else {
        x.cain_gone = true;
    }
}

/// `0x005928C0` over every monster, stopping at the first class-146 one.
fn removal_walk<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let Some(m) = w
        .monsters()
        .into_iter()
        .find(|&m| w.monster_class(m) == Some(CAIN_TRISTRAM))
    else {
        return;
    };
    match w.npc_chat_clients(m) {
        Some(players) => {
            for p in players {
                w.send(p, &[0x5D, CHAIN, 1, 0, 0, 0]);
            }
        }
        None => {
            w.remove_monster(m);
            x4(ctl, i).cain_removed = true;
        }
    }
}

/// Timer `0x00593260`: the removal walk; returns 1.
pub(super) fn removal_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    removal_walk(ctl, w, i);
    x4(ctl, i).removal_timer = false;
}

/// The town Cain spawn at the marker object `m` (§10.6 step 3.3;
/// `quests-act1-rest.md` §8 item 3: (x, y) and R0 are the object's). A
/// marker left in a freed room has no room (§9 item 2): every spawn try
/// finds none and nothing spawns or draws.
fn spawn_town_cain_at_marker<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    m: UnitId,
) {
    let room = w.unit_position(m).map(|p| p.2);
    match w.unit_xy(m) {
        Some((x, y)) => spawn_town_cain(ctl, w, i, x, y, room),
        // A marker found by GUID always has a static path.
        None => ctl.faults.push(QuestError::Fatal(0x0059_2960)),
    }
}

/// Town Cain spawn `0x00592960(game, x, y)` in room R0 (§10.6 step 15).
/// R0 null (`quests-act1-rest.md` §9 items 1–2): the point test reads an
/// all-zero box and every spawn try returns null before any allocation
/// or draw, so nothing happens.
fn spawn_town_cain<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    x: i32,
    y: i32,
    r0: Option<RoomId>,
) {
    let Some(r0) = r0 else { return };
    // 21 points, i = 0 through 20 (`quests-act1-rest.md` §8 item 3).
    let (mut px, mut py) = (0..=20)
        .map(|k| (x + k, y + k))
        .find(|&(a, b)| w.room_contains(r0, a, b))
        .unwrap_or((y, y + 21)); // bug kept
    let (sx, sy, sr) = w
        .free_spot_at(r0, px, py, 2, 0x100, 1, 100)
        .unwrap_or((x, y, r0));
    let mut cain = w.spawn_monster(sr, sx, sy, npc::CAIN5, 1, 5);
    let mut tries = 0;
    while cain.is_none() && tries < 20 {
        tries += 1;
        px += 1;
        py += 1;
        let (sx, sy, sr) = match w.room_at(r0, px, py) {
            Some(room) => w
                .free_spot_at(room, px, py, 2, 0x100, 2, 100)
                .unwrap_or((x, y, r0)),
            None => (x, y, r0),
        };
        cain = w.spawn_monster(sr, sx, sy, npc::CAIN5, 1, 10);
    }
    if cain.is_none() {
        cain = w.spawn_monster(r0, x, y, npc::CAIN5, 1, 15);
    }
    if let Some(c) = cain {
        w.or_unit_flags(c, 0x0300_0000);
        let g = w.guid(c);
        let x = x4(ctl, i);
        x.town_cain = true;
        x.town_cain_due = false;
        x.town_cain_guid = g;
    }
}

/// add_link `0x00592F80` (§4.6): the class-61 object linked to chain 4.
pub fn link_object<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: Option<UnitId>) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = object.map_or(u32::MAX, |o| w.guid(o));
    let x = x4(ctl, i);
    x.linked = true;
    x.linked_guid = g;
}

// ------------------------------------------------- stones, scroll, tree

/// The Cairn stone order from successive draws (§10.6).
pub fn stone_order_from(mut lo: impl FnMut() -> u32) -> [u8; 5] {
    let mut order = [0u8; 5];
    let mut i = 0;
    while i < 5 {
        let k = (lo() % 5) as usize;
        if order[k] == 0 {
            order[k] = 17 + i;
            i += 1;
        }
    }
    order
}

/// `0x00592E90`: the stone order on the quest seed, computed once (the
/// +0x4A guard).
pub fn stone_order(ctl: &mut QuestControl) -> [u8; 5] {
    let Some(i) = ctl.find(CHAIN) else {
        return [0; 5];
    };
    if let Some(o) = ctl.records[i].extra.stone_order {
        return o;
    }
    let seed: &mut Seed = &mut ctl.seed;
    let o = stone_order_from(|| seed.step());
    ctl.records[i].extra.stone_order = Some(o);
    o
}

/// `0x00593CB0` (§9.4): the stone order's 0x50 (15 bytes). Bytes 13–14
/// are never written by the original (`quests-act1-rest.md` §7: stack
/// leftovers, masked by the comparison); 0 here.
pub fn send_stone_order<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    if ctl.find(CHAIN).is_none() {
        return;
    }
    let order = stone_order(ctl);
    let mut m = [0u8; 15];
    m[0] = 0x50;
    m[1..3].copy_from_slice(&4u16.to_le_bytes());
    for (k, &v) in order.iter().enumerate() {
        let n = u16::from(v.wrapping_sub(17));
        if n >= 5 {
            ctl.faults.push(QuestError::Fatal(0x0059_3CB0));
            return;
        }
        m[3 + 2 * k..5 + 2 * k].copy_from_slice(&n.to_le_bytes());
    }
    w.send(player, &m);
}

/// Wirt's body, object event 7 (`0x00594630`, §10.6).
pub fn wirt_body<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let mut piles = match ctl.records[i].extra.wirt_piles {
        Some(n) => n,
        None => ctl.seed.roll_range(10, 10),
    };
    if piles > 0 && w.drop_item_at(object, *b"gld ", 2) {
        piles -= 1;
        if piles > 0 {
            let at = w.frame() + w.tick_rate().ticks(10);
            w.schedule_quest_event(object, at);
        }
    }
    ctl.records[i].extra.wirt_piles = Some(piles);
}

/// Tree operate `0x00593AF0` (operate pointer `0x00732D48`).
pub fn tree_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let Some(i) = ctl.find(CHAIN) else {
        return ctl.faults.push(QuestError::Fatal(0x0059_3AF0));
    };
    if !ctl.records[i].not_intro {
        return w.set_object_mode(object, 1);
    }
    let r = player_flags(w, player);
    if ctl.records[i].state >= 6
        || w.object_mode(object) != 0
        || r.get(SLOT, bit::REWARD_GRANTED)
        || r.get(SLOT, bit::REWARD_PENDING)
    {
        return;
    }
    if w.has_item(player, DECIPHERED) || w.has_item(player, SCROLL) {
        return w.attach_sound(player, 19);
    }
    w.attach_sound(player, 45);
    ctl.records[i].state = 4;
    if w.drop_item_at(object, SCROLL, 2) {
        ctl.records[i].callbacks |= 1 << event::PLAYER_DROPPED_WITH_QUEST_ITEM;
        broadcast(ctl, w, i, 2, 0);
        let x = x4(ctl, i);
        x.b4b = true;
        // +0x38 := the scroll's GUID in 1.14d (`quests-act1-rest.md` §8
        // item 1); it has no reader, so it is not kept here and the drop
        // seam returns no item.
        x.scrolls += 1;
        x.progress = 1;
        x.b78 = true;
        w.set_object_mode(object, 1);
    }
    let g = w.guid(object);
    let x = x4(ctl, i);
    x.tree_known = true;
    x.tree_guid = g;
}

/// Stone operate `0x00593710` (operate pointer `0x00732D3C`): `value` is
/// the stone's value, the operated object's class 17–21 (args +0x10,
/// `quests-act1-rest.md` §2.1).
pub fn stone_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    value: u16,
) {
    let Some(i) = ctl.find(CHAIN) else {
        return ctl.faults.push(QuestError::Fatal(0x0059_3710));
    };
    let order = stone_order(ctl);
    let r = player_flags(w, player);
    if r.get(SLOT, bit::REWARD_GRANTED) || r.get(SLOT, bit::REWARD_PENDING) {
        return w.attach_sound(player, 19);
    }
    if !w.has_item(player, DECIPHERED) {
        let x = x4(ctl, i);
        if x.touches.is_multiple_of(64)
            && !r.get(SLOT, bit::LEAVE_TOWN)
            && !r.get(SLOT, bit::ENTER_AREA)
        {
            w.attach_sound(player, 39);
        }
        x4(ctl, i).touches = x4(ctl, i).touches.wrapping_add(1);
        return;
    }
    if !ctl.records[i].not_intro || ctl.records[i].state >= 6 {
        return;
    }
    let x = x4(ctl, i).clone();
    let linked = x
        .linked
        .then(|| w.object_by_guid(x.linked_guid))
        .flatten()
        .map(|o| o.0);
    if !x.b4e && linked.is_some() && ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
    }
    if x.b4f {
        return;
    }
    if ctl.records[i].state != 5 {
        ctl.records[i].state = 5;
    }
    let k = usize::from(x.stones);
    if order.get(k).map(|&v| u16::from(v)) != Some(value) {
        return;
    }
    let g = w.guid(object);
    let x = x4(ctl, i);
    x.stone_guids[k] = g;
    x.stones += 1;
    let n = x.stones;
    if w.object_mode(object) != 0 {
        return;
    }
    w.set_object_mode(object, 1);
    if n <= 4 {
        if let Some(l) = linked {
            w.set_object_mode(l, i32::from(n) + 1);
        }
        return;
    }
    if let Some(l) = linked {
        w.set_object_mode(l, 6);
    }
    x4(ctl, i).b4f = true;
    w.delete_item(player, DECIPHERED);
    x4(ctl, i).scrolls -= 1;
    let last = order
        .iter()
        .position(|&v| v == 21)
        .map(|k| x4(ctl, i).stone_guids[k])
        .and_then(|g| w.object_by_guid(g))
        .filter(|o| o.1 == STONE_21)
        .map(|o| o.0)
        .or_else(|| w.find_object_near(object, STONE_21));
    if let Some((sx, sy, _)) = last.and_then(|s| w.unit_position(s)) {
        // The cairnstones missile (owner the player, skill 0, level 1)
        // opens the Tristram portal (`quests-act1-rest.md` §2.3, §4.1).
        if let Some(m) = w.create_missile(player, 0, 1, CAIRN_MISSILE, sx + 6, sy - 3) {
            w.refresh_room(m);
        }
    }
    if ctl.records[i].status < 4 {
        broadcast(ctl, w, i, 4, 0);
        if let Some(members) = w.party_members(player) {
            for m in members {
                // `0x005936B0`.
                let in_act1 = w.unit_level(m).is_some_and(|l| l != 0) && w.unit_act(m) == Some(0);
                let Some(f) = flags_of(w, m) else { continue };
                if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) && in_act1
                {
                    f.set(SLOT, bit::ENTER_AREA);
                }
            }
        }
        if let Some(f) = flags_of(w, player) {
            f.set(SLOT, bit::ENTER_AREA);
        }
    }
    ctl.unique_event(w, 1);
}

// ------------------------------------- gibbet, stone init, town-Cain marker

/// Gibbet operate `0x00593480` (operate pointer `0x00732D40`,
/// `quests-act1-rest.md` §1.1).
pub fn gibbet_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let i = ctl.find(CHAIN);
    if let Some(i) = i {
        let r = &ctl.records[i];
        if !r.not_intro || r.extra.q4.cain_gone || r.state >= 6 {
            return;
        }
    }
    let r = player_flags(w, player);
    if r.get(SLOT, bit::REWARD_PENDING) || r.get(SLOT, bit::REWARD_GRANTED) {
        return w.attach_sound(player, 19);
    }
    if w.object_mode(object) != 0 {
        return;
    }
    w.set_object_mode(object, 1);
    let at = w.frame() + (w.object_anim_length(object) >> 8);
    w.schedule_object_event(object, 1, at);
    if let Some(i) = i {
        let g = w.guid(player);
        let x = x4(ctl, i);
        x.gibbet_open = 3;
        x.gibbet_player = g;
    }
    // Event 7 runs `gibbet_event` (§9.5).
    let at = w.frame() + w.tick_rate().ticks(17);
    w.schedule_quest_event(object, at);
    w.refresh_room(object);
    if let Some(f) = flags_of(w, player) {
        f.set(SLOT, bit::PRIMARY_GOAL_DONE);
        f.set(SLOT, bit::REWARD_PENDING);
    }
    send_player_flags(w, player, 6, 0);
    party_rescued(w, player);
}

/// Gibbet quest function `0x00593290(game, object)`, object event 7
/// (§9.5 class 26, `quests-act1-rest.md` §1.2).
pub fn gibbet_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        return ctl.faults.push(QuestError::Fatal(0x0059_3290));
    };
    if !ctl.records[i].not_intro || x4(ctl, i).cain_gone {
        return;
    }
    x4(ctl, i).gibbet_open = 3;
    w.set_object_mode(object, 3);
    // The object's position (static path) and room (static path +0x00,
    // read without a test): a gibbet left in a freed room has none, and
    // both spawns and the free-spot search find nothing
    // (`quests-act1-rest.md` §9 items 1–2).
    let Some((ox, oy)) = w.unit_xy(object) else {
        return ctl.faults.push(QuestError::Fatal(0x0059_3290));
    };
    let room = w.unit_position(object).map(|p| p.2);
    let (x, y) = (ox + 3, oy + 3);
    let cain = room.and_then(|room| {
        w.spawn_monster(room, x, y, CAIN_TRISTRAM, 1, u32::MAX)
            .or_else(|| {
                let (fx, fy, fr) = w.free_spot_at(room, x, y, 2, 0x100, 3, 100)?;
                w.spawn_monster(fr, fx, fy, CAIN_TRISTRAM, 1, u32::MAX)
            })
    });
    match cain {
        None => {
            // `0x00593220` over every player: the first in Tristram.
            let found = w
                .players()
                .into_iter()
                .find(|&p| w.unit_level(p) == Some(TRISTRAM));
            x4(ctl, i).found_player = found;
            if let Some(p) = found {
                if !x4(ctl, i).out_portal {
                    // `0x0056D130` exits with an internal error on a null
                    // room (`0x0056D147`, §9 item 2): the game ends here.
                    let Some(room) = room else {
                        return ctl.faults.push(QuestError::Fatal(0x0056_D147));
                    };
                    if w.open_portal(Some(p), room, x + 3, y + 3, TOWN, PORTAL_TO_TOWN, false)
                        .is_some()
                    {
                        x4(ctl, i).out_portal = true;
                    }
                }
            }
            let x = x4(ctl, i);
            if !x.town_cain {
                x.town_cain_due = true;
            }
            x.cain_failed = true;
        }
        Some(c) => {
            w.or_unit_flags(c, 0x0300_0000);
            let g = x4(ctl, i).gibbet_player;
            if let Some(p) = w.player_by_guid(g) {
                w.attach_sound(p, 48);
            }
        }
    }
    for p in w.players() {
        rescued(w, p);
    }
    for p in w.players() {
        completed_now(w, p);
    }
    ctl.records[i].flags = 0;
    broadcast(ctl, w, i, 6, 0);
}

/// Cairn stone init `0x005935E0` (init pointer `0x00731BD8`, objects
/// 17–21; `quests-act1-rest.md` §2.2).
pub fn stone_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, class: u16) {
    let Some(i) = ctl.find(CHAIN) else {
        if w.object_mode(object) != 2 {
            w.set_object_mode(object, 2);
        }
        return;
    };
    if ctl.records[i].not_intro && !x4(ctl, i).b4c {
        let x = x4(ctl, i);
        if x.b4d || x.cain_gone {
            return w.set_object_mode(object, 2);
        }
        // The byte +0x4B + c: +0x5C–+0x60 for the stones (never set).
        let k = usize::from(class.wrapping_sub(STONE_17));
        if let Some(b) = x.stone_reset.get_mut(k) {
            if *b {
                *b = false;
                w.set_object_mode(object, 0);
            }
        }
        return;
    }
    x4(ctl, i).b4c = false;
    if !x4(ctl, i).portal_made && class == STONE_17 {
        let g = w.guid(object);
        x4(ctl, i).portal_stone = g;
        if !x4(ctl, i).portal_timer {
            x4(ctl, i).portal_timer = true;
            if let Err(e) = ctl.add_timer(CHAIN, TimerFn::TristramPortal, 1) {
                ctl.faults.push(e);
            }
        }
    }
    w.set_object_mode(object, 2);
}

/// Tristram-portal timer `0x00592D50` (`quests-act1-rest.md` §2.3); true
/// = remove it.
pub(super) fn tristram_portal_timer<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
) -> bool {
    let stone = w
        .object_by_guid(x4(ctl, i).portal_stone)
        .and_then(|(s, _)| w.unit_position(s));
    let Some((x, y, room)) = stone else {
        x4(ctl, i).portal_timer = false;
        return true;
    };
    if w.open_portal(None, room, x + 4, y + 4, TRISTRAM, PORTAL_TO_TRISTRAM, true)
        .is_none()
    {
        return false;
    }
    let x = x4(ctl, i);
    x.portal_made = true;
    x.portal_timer = false;
    true
}

/// Town-Cain marker init `0x005940E0` (object 385, `InitFn` 54;
/// `quests-act1-rest.md` §3) with the init args' room and position. A
/// null room spawns nothing (§9 item 2).
pub fn marker_init<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let xd = x4(ctl, i);
    xd.marker_guid = g;
    xd.marker_known = true;
    xd.marker_pos = (x, y);
    if xd.town_cain_due && !xd.town_cain {
        spawn_town_cain(ctl, w, i, x, y, room);
    }
}

/// Cain leaves Tristram `0x005944F0(game, unit)`, the town-portal call of
/// `cain1`'s NpcOutOfTown AI (`quests-act1-rest.md` §3).
pub fn cain_leaves_tristram<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = x4(ctl, i);
    x.b91 = true;
    x.town_cain_due = true;
    if !x.marker_known {
        return;
    }
    let Some((m, _)) = w.object_by_guid(x.marker_guid) else {
        return;
    };
    // A marker left in a freed room passes a null room: the town Cain
    // spawn finds nothing (§9 item 2).
    let room = w.unit_position(m).map(|p| p.2);
    match w.unit_xy(m) {
        Some((mx, my)) => marker_init(ctl, w, m, room, mx, my),
        // A marker found by GUID always has a static path.
        None => return ctl.faults.push(QuestError::Fatal(0x0059_44F0)),
    }
    if !x4(ctl, i).town_cain {
        return;
    }
    let Some((c, _)) = w.monster_by_guid(x4(ctl, i).town_cain_guid) else {
        return;
    };
    // Cain's room is its dynamic path +0x1C; without one the cell
    // lookup `0x00463740(null)` is null: no portal object.
    let Some((cx, cy, croom)) = w.unit_position(c) else {
        return;
    };
    let Some(room) = w.room_at(croom, cx, cy) else {
        return;
    };
    if let Some(o) = w.spawn_object(room, cx, cy, CAIN_PORTAL, 1) {
        let g = w.guid(o);
        let x = x4(ctl, i);
        x.cain_portal = true;
        x.cain_portal_guid = g;
    }
}

/// Set up Cain's portal point `0x005944B0(game, unit)` (the NpcOutOfTown
/// portal set-up, `monsters/ai-bodies.md` §9.32 step 1;
/// `quests-act1-rest.md` §3): no chain 4 record or a point already set →
/// false; else the point := the unit's position (`0x00620870`), X +0x67
/// := 1, true.
pub fn cain_portal_setup<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) -> bool {
    let Some(i) = ctl.find(CHAIN) else {
        return false;
    };
    if x4(ctl, i).portal_point_set {
        return false;
    }
    let p = w.unit_xy(unit).unwrap_or((0, 0));
    let x = x4(ctl, i);
    x.portal_point = p;
    x.portal_point_set = true;
    true
}

/// Spawn Cain's portal out of town `0x005943B0(game, unit)` (§9.32 step
/// 8; `quests-act1-rest.md` §3): with chain 4's record and no portal yet,
/// the room holding the point, searched from the unit's room
/// (`0x00463740`): none → the point moves by (+3, +3); else object 189
/// at the point, flags 1, mode 1 (`0x00555230`), and on success X +0x95
/// := 1, X +0x98 := its GUID. Always true (1.14d returns 1 on every
/// path).
pub fn cain_spawn_outside_portal<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    unit: UnitId,
) -> bool {
    let Some(i) = ctl.find(CHAIN) else {
        return true;
    };
    if x4(ctl, i).out_cain_portal {
        return true;
    }
    let (px, py) = x4(ctl, i).portal_point;
    let room = w
        .unit_position(unit)
        .and_then(|(_, _, r)| w.room_at(r, px, py));
    let Some(room) = room else {
        x4(ctl, i).portal_point = (px.wrapping_add(3), py.wrapping_add(3));
        return true;
    };
    if let Some(o) = w.spawn_object(room, px, py, CAIN_PORTAL, 1) {
        let g = w.guid(o);
        let x = x4(ctl, i);
        x.out_cain_portal = true;
        x.out_cain_portal_guid = g;
    }
    true
}

/// Cain's portal point `0x00594450(game, unit, &xy)` (§9.32 step 7): no
/// chain 4 record → none; no out-of-town portal yet → the unit's own
/// position (`0x00620870`); else the point.
pub fn cain_portal_coords<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    unit: UnitId,
) -> Option<(i32, i32)> {
    let i = ctl.find(CHAIN)?;
    let x = x4(ctl, i);
    if x.out_cain_portal {
        return Some(x.portal_point);
    }
    Some(w.unit_xy(unit).unwrap_or((0, 0)))
}

/// Gibbet init `0x00544990` → `0x00594060` (object 26, `InitFn` 7;
/// `quests-act1-rest.md` §9 item 8): no chain 4 record → mode 2 unless it
/// is 2; else mode := X +0x54 (3 once opened), X +0x48 := 1, X +0x34 :=
/// the object's GUID. No draw, no message.
pub fn gibbet_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        if w.object_mode(object) != 2 {
            w.set_object_mode(object, 2);
        }
        return;
    };
    let mode = x4(ctl, i).gibbet_open;
    w.set_object_mode(object, mode);
    let g = w.guid(object);
    let x = x4(ctl, i);
    x.gibbet_known = true;
    x.gibbet_guid = g;
}

/// Inifuss tree init `0x00593FC0` (object 30, `InitFn` 9;
/// `quests-act1-rest.md` §9 item 9): no chain 4 record → mode 2 unless it
/// is 2; else X +0x47 := 1 (the GUID at +0x30 is the operate's), X +0x58
/// := 1 when not-intro = 0 or Cain is gone (+0x50), then mode := X +0x58.
pub fn tree_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        if w.object_mode(object) != 2 {
            w.set_object_mode(object, 2);
        }
        return;
    };
    let not_intro = ctl.records[i].not_intro;
    let x = x4(ctl, i);
    x.tree_known = true;
    if !not_intro || x.cain_gone {
        x.progress = 1;
    }
    let mode = x.progress;
    w.set_object_mode(object, mode);
}

/// Cain portal init (object 189): mode 1, object event 7 25 base ticks
/// (1 s) later.
pub fn cain_portal_init<W: QuestWorld>(w: &mut W, object: UnitId) {
    w.set_object_mode(object, 1);
    let at = w.frame() + w.tick_rate().ticks(25);
    w.schedule_quest_event(object, at);
}

/// The Act I cain portal's event 7, `0x005942C0(record, object)`
/// (`quests.md` §9.5 class 189 in Act I; `quests-act1-rest.md` §9 item
/// 10): by the object's mode, 1 → 2; 2 → in the Rogue Encampment count
/// +0x80 and set +0x92 past 5, then 3 when +0x92 is set, elsewhere 3 when
/// +0x91 is set; 3 → 4. Event 7 again at frame + 25 in every case.
pub fn cain_portal_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    match w.object_mode(object) {
        1 => w.set_object_mode(object, 2),
        2 => {
            if w.unit_level(object) == Some(TOWN) {
                let x = x4(ctl, i);
                x.town_portal_count = x.town_portal_count.wrapping_add(1);
                if x.town_portal_count > 5 {
                    x.b92 = true;
                }
                if x.b92 {
                    w.set_object_mode(object, 3);
                }
            } else if x4(ctl, i).b91 {
                w.set_object_mode(object, 3);
            }
        }
        3 => w.set_object_mode(object, 4),
        _ => {}
    }
    let at = w.frame() + w.tick_rate().ticks(25);
    w.schedule_quest_event(object, at);
}

/// Wirt's body operate `0x00583E70` (object 268, `OperateFn` 33;
/// `quests-act1-rest.md` §9 item 11; returns 1 in every case): only in
/// mode 0; drop code := `leg ` and one drop at the object
/// (`0x00559A30(game, object, 2, &out, 0, −1, 0)`); no item → nothing
/// more (the next operate tries again, edge case 9); an item → mode 1,
/// object event 1 at frame + (`FrameCnt1` >> 8) + 1 and event 7 (the gold
/// piles, [`wirt_body`]) at frame + 10.
pub fn wirt_body_operate<W: QuestWorld>(w: &mut W, object: UnitId) {
    if w.object_mode(object) != 0 {
        return;
    }
    if w.quest_drop(object, *b"leg ", 2, None, false).is_none() {
        return;
    }
    w.set_object_mode(object, 1);
    let at = w.frame() + (w.object_anim_length(object) >> 8) + 1;
    w.schedule_object_event(object, 1, at);
    let at = w.frame() + w.tick_rate().ticks(10);
    w.schedule_quest_event(object, at);
}
