// Spec: specs/world/quests-act5-2.md §8 (A5Q6 Eve of Destruction), §10, §11
//! A5Q6 Eve of Destruction (chain 36, slot 40): the post-Ancients and
//! post-Baal lines, Baal's death (credits, character progression, gold
//! piles), the Throne / Chamber portals and the Worldstone Chamber's
//! opening, the last portal (game finished, 40.10), the zoo monster id
//! (S→C 0x50) and the sequence function (§8.9). Chain 36 uses the
//! default status rule (part 1 §2).

use super::super::late::{self, flags, set};
use super::super::{
    bit, event, raise_progression, EventArgs, GuidList, QuestControl, QuestFlags, QuestRecord,
    QuestWorld, TextList,
};
use crate::units::{RoomId, UnitId};

const CHAIN: u8 = 36;
const SLOT: u8 = 40;
/// NPCs (`monstats.txt`).
pub const LARZUK: u16 = 511;
pub const DREHYA: u16 = 512;
pub const MALAH: u16 = 513;
pub const QUAL_KEHK: u16 = 515;
pub const CAIN6: u16 = 520;
pub const TYRAEL3: u16 = 521;
/// Levels: Harrogath, Throne of Destruction, Worldstone Chamber.
const HARROGATH: u32 = 109;
pub const THRONE: u32 = 131;
pub const CHAMBER: u32 = 132;
/// `0x00735EE4`: table state by record state.
const MSG_STATE: [i8; 4] = [-1, -1, -1, 0];
/// `0x0058D9D0`: message − 20175 → bit.
const MSG_FIRST: u32 = 20175;
const MSG_BITS: [u8; 6] = [7, 9, 5, 4, 6, 8];
const SOUND_REFUSED: u16 = 19;
const SOUND_BAAL: u16 = 83;
const FX_BAAL: u8 = 19;
/// Missile 625 at Baal's death.
const MISSILE: u16 = 625;
/// The last portal (object 565, §8.4, §8.8).
const LAST_PORTAL: u16 = 565;
/// The act argument of the character progression call.
const PROGRESSION_ACT: u8 = 5;
/// The zoo's draws.
const ZOO_TRIES: u32 = 10;

/// Quest extra data (record +0x18, 0xA8 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the extra GUID list (event 10 removes from it).
    pub guids: GuidList,
    /// +0x86: the Worldstone Chamber is open.
    pub chamber_open: bool,
    /// +0x88: players credited in the Chamber.
    pub credited: u32,
    /// +0x8C: the kill room.
    pub kill_room: Option<RoomId>,
    /// +0x90: the quest was started by the sequence function.
    pub by_sequence: bool,
    /// +0x94: the Throne / Chamber portal mode (1 at init).
    pub portal_mode: i32,
    /// +0x98: the last portal was made.
    pub last_portal_made: bool,
    /// +0x9C: the last portal's mode (1 at init).
    pub last_portal_mode: i32,
    /// +0xA0: the zoo was chosen; +0xA4: its monster id.
    pub zoo_chosen: bool,
    pub zoo_id: u32,
}

/// Timers this quest makes (`quests.md` §5): none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init `0x0058E500` beyond `quests.tsv` (part 1 §2): list +0x00 reset;
/// +0x94 := 1, +0x9C := 1.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q6 = Extra {
        portal_mode: 1,
        last_portal_mode: 1,
        ..Extra::default()
    };
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q6
}

fn xr(ctl: &QuestControl, i: usize) -> &Extra {
    &ctl.records[i].extra.a5.q6
}

/// One callback of record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => chat(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => killed(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x0058DCF0`: both lists.
            if let Some(p) = args.player {
                late::leave(ctl, w, i, p);
                let g = w.guid(p);
                x(ctl, i).guids.remove(g);
            }
        }
        event::SCROLL_MESSAGE => {
            // `0x0058D940` (any NPC).
            let k = args.b.wrapping_sub(MSG_FIRST) as usize;
            if let (Some(&b), Some(p)) = (MSG_BITS.get(k), args.player) {
                set(w, p, SLOT, b);
            }
        }
        event::PLAYER_STARTED_GAME => {
            // `0x0058E430`: plain status writes.
            let Some(p) = args.player else { return true };
            let f = flags(w, p);
            if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
                set(w, p, SLOT, 10);
                let d = w.difficulty();
                raise_progression(w, p, PROGRESSION_ACT, d);
            } else if f.get(SLOT, bit::REWARD_PENDING) {
            } else if f.get(SLOT, bit::LEAVE_TOWN) {
                late::status_silent(ctl, i, 1);
                late::set_state(ctl, i, 3);
            } else if f.get(SLOT, bit::STARTED) {
                x(ctl, i).by_sequence = true;
                late::status_silent(ctl, i, 1);
                late::set_state(ctl, i, 3);
            }
        }
        event::PLAYER_JOINED_GAME => {
            // `0x0058E3F0`.
            if let Some(p) = args.player {
                if flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
                    set(w, p, SLOT, 10);
                }
            }
        }
        _ => return false,
    }
    true
}

/// §8.2 flag iterate `0x0058D8B0`, for every player.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let f = flags(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED)
            || f.get(SLOT, bit::REWARD_PENDING)
            || !late::in_act(w, p, 4)
        {
            continue;
        }
        match state {
            2 => set(w, p, SLOT, bit::STARTED),
            3 => set(w, p, SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// §8.3 step 1's test: chain 35's fight started, no Ancient alive, and
/// (state 1 with +0x90 = 0, or state 2 with +0x90 set). `first`: only
/// the first case (the active function's).
fn statue_case(ctl: &QuestControl, i: usize, first: bool) -> bool {
    let Some(a) = ctl.record(35) else {
        return false;
    };
    let q5 = &a.extra.a5.q5;
    let (state, seq) = (ctl.records[i].state, xr(ctl, i).by_sequence);
    q5.fight_started && q5.alive == 0 && ((state == 1 && !seq) || (!first && state == 2 && seq))
}

/// The post-Baal bit of an NPC (§8.3 step 3, the active function).
fn post_bit(class: u16) -> Option<u8> {
    match class {
        LARZUK => Some(4),
        CAIN6 => Some(5),
        MALAH => Some(6),
        TYRAEL3 => Some(7),
        QUAL_KEHK => Some(8),
        DREHYA => Some(9),
        _ => None,
    }
}

/// §8.3 event 0 `0x0058D9F0`.
fn chat<W: QuestWorld>(
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
    if super::q5::STATUE_NPCS.contains(&class) && statue_case(ctl, i, false) {
        // Edge case 7: table state 3 has no statue entries.
        late::add_state(ctl, i, list, class, 3);
        return;
    }
    let f = flags(w, p);
    let state = ctl.records[i].state;
    if !f.get(SLOT, bit::REWARD_GRANTED) {
        if state <= 3 {
            if let Some(k) = late::table_state(&MSG_STATE, state) {
                late::add_state(ctl, i, list, class, k);
            }
        }
        return;
    }
    let done = f.get(SLOT, bit::PRIMARY_GOAL_DONE);
    let k = match class {
        LARZUK | DREHYA | MALAH | QUAL_KEHK => {
            let b = post_bit(class).expect("listed");
            if !f.get(SLOT, b) {
                Some(2)
            } else if done {
                Some(3)
            } else {
                None
            }
        }
        CAIN6 => {
            if f.get(SLOT, 10) {
                None
            } else if !f.get(SLOT, 5) {
                Some(4)
            } else if done {
                Some(5)
            } else {
                None
            }
        }
        TYRAEL3 if done => Some(2),
        _ => None,
    };
    if let Some(k) = k {
        late::add_state(ctl, i, list, class, k);
    }
}

/// §8.3 active function `0x0058E2B0`.
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    if super::q5::STATUE_NPCS.contains(&npc_class) {
        return statue_case(ctl, i, true);
    }
    let pf = flags(w, player);
    if !pf.get(SLOT, bit::REWARD_GRANTED) {
        return false;
    }
    match npc_class {
        CAIN6 => !pf.get(SLOT, 10) && !pf.get(SLOT, 5),
        c => post_bit(c).is_some_and(|b| !pf.get(SLOT, b)),
    }
}

/// §8.4 event 2 `0x0058D870`: Tyrael's last portal.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let tyrael = args.target.and_then(|n| w.monster_class(n)) == Some(TYRAEL3);
    if !tyrael || xr(ctl, i).last_portal_made {
        return;
    }
    // `0x0058D7D0` per player: the first one in level 132 (its room's
    // level) gets object 565 (type 2, flags 1, 1, 0) at a free spot from
    // its position + (5, 0) in its room (`0x00545340` size 5, mask 0x400,
    // radius 18 unused, limit 100, `0x0058D80A`); it returns 1 for that
    // player whether or not a spot or object was made, so the walk stops
    // and nothing retries.
    for p in w.players() {
        if w.unit_level(p) != Some(CHAMBER) {
            continue;
        }
        if let Some((px, py, room)) = w.unit_position(p) {
            if let Some((sx, sy, r)) = w.free_spot_at(room, px + 5, py, 5, 0x400, 18, 100) {
                w.place_object(r, sx, sy, LAST_PORTAL, [1, 1, 0]);
            }
        }
        break;
    }
    x(ctl, i).last_portal_made = true;
}

/// §8.5 event 8 `0x0058DF20`: Baal's death.
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(victim) = args.target else { return };
    ctl.unique_event(w, FX_BAAL);
    // No victim room ends the whole callback, step 2 included
    // (`0x0058DF5C` → `0x0058E110`).
    if ctl.records[i].not_intro && !baal_credits(ctl, w, i, victim, args.player) {
        return;
    }
    // Step 2 (always).
    if let Some(m) = w.create_missile_at(victim, MISSILE) {
        late::unit_room_portal(w, m, false);
    }
}

/// §8.5 step 1 (not-intro); false: the victim has no room.
fn baal_credits<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    victim: UnitId,
    killer: Option<UnitId>,
) -> bool {
    let Some((_, _, room)) = w.unit_position(victim) else {
        return false;
    };
    x(ctl, i).kill_room = Some(room);
    // Status 4 to all, the flags byte kept (no flags write, `0x0058DF72`).
    ctl.records[i].status = 4;
    late::iterate_all(ctl, w, i);
    if let Some(k) = killer {
        let b = !flags(w, k).get(SLOT, bit::REWARD_GRANTED);
        x(ctl, i).credited = 0;
        let d = w.difficulty();
        // `0x0058DEA0` → credit `0x0058DD30`.
        for p in w.players() {
            let f = flags(w, p);
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
                && w.unit_level(p) == Some(CHAMBER)
            {
                set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
                set(w, p, SLOT, bit::REWARD_GRANTED);
                raise_progression(w, p, PROGRESSION_ACT, d);
                x(ctl, i).credited += 1;
            }
        }
        // `0x0058DD90` → party members `0x0058DC60` (not counted).
        for p in w.players() {
            if !flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in late::party_of(w, p) {
                let f = flags(w, m);
                if !f.get(SLOT, bit::REWARD_GRANTED)
                    && !f.get(SLOT, bit::REWARD_PENDING)
                    && late::in_act(w, m, 4)
                {
                    set(w, m, SLOT, bit::REWARD_GRANTED);
                    set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                    raise_progression(w, m, PROGRESSION_ACT, d);
                }
            }
        }
        // `0x0058DDE0`.
        late::completion_flag(w, CHAIN, SLOT, &[bit::REWARD_GRANTED]);
        // `0x0058DE30`.
        for p in w.players() {
            if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                late::s5d(w, p, CHAIN, 0x02, 0);
                w.attach_sound(p, SOUND_BAAL);
            }
        }
        if b {
            let (min, max) = gold_range(d);
            for _ in 0..xr(ctl, i).credited {
                // `0x00545990` is a `ret 4` stub.
                let amount = min + ctl.seed.roll((max - min) as i32);
                w.drop_gold_amount(victim, amount);
            }
        }
    }
    // "Then": after the killer block, inside not-intro; a host request
    // (`quests-helpers.md` §6).
    ctl.save_pass();
    late::set_state(ctl, i, 5);
    true
}

/// Baal's gold range [min, max) for difficulty `d` (§8.5): min = 6000·d
/// + 1500, max = 6000·d + 3000 capped at 0xFFFF.
pub fn gold_range(d: u8) -> (u32, u32) {
    let d = u32::from(d);
    (6000 * d + 1500, (6000 * d + 3000).min(0xFFFF))
}

/// §8.6 event 3 `0x0058E190`.
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.b == HARROGATH {
        send_zoo(ctl, w, i, p);
    }
    if args.a == HARROGATH {
        late::quick_remove(ctl, w, i, p);
        let f = flags(w, p);
        if ctl.records[i].state == 2
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
        {
            late::set_state(ctl, i, 3);
            if ctl.records[i].status != 1 {
                late::status_to_all(ctl, w, i, 1);
            }
            flag_iterate(ctl, w, i);
        }
    }
    // Edge case 9: every level ≥ 131.
    if args.b >= THRONE {
        let r = &ctl.records[i];
        if r.not_intro && r.status < 2 {
            late::status_to_all(ctl, w, i, 2);
        }
        flag_iterate(ctl, w, i);
    }
}

/// `0x0058E120` / `0x0058E180`: S→C 0x50 with u16 36 and the zoo id
/// (bytes 5–14 are not written in the original; 0 here, as `quests.md`
/// §9.4).
fn send_zoo<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize, p: UnitId) {
    let mut m = [0u8; 15];
    m[0] = 0x50;
    m[1..3].copy_from_slice(&u16::from(CHAIN).to_le_bytes());
    m[3..5].copy_from_slice(&(xr(ctl, i).zoo_id as u16).to_le_bytes());
    w.send(p, &m);
}

/// The status function: chain 36 has none (the default rule, part 1
/// §2), so this is never reached; reported if it is.
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = (player, pf);
    w.unhandled(ctl.records[i].chain, f);
    None
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let _ = (ctl, w, chain);
    match t {}
}

/// `0x0058E390` (§8.9), called with chain 36's record `i`: not-intro and
/// state 0 → +0x90 := 1, state := 2, status 1 to all, flag iterate.
/// Returns 1.
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].not_intro && ctl.records[i].state == 0 {
        x(ctl, i).by_sequence = true;
        ctl.records[i].state = 2;
        late::status_to_all(ctl, w, i, 1);
        flag_iterate(ctl, w, i);
    }
    true
}

/// Init 75 `0x0058E670` (objects 563, 569): mode := +0x94; +0x94 := 2.
pub fn portal_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    w.set_object_mode(object, xr(ctl, i).portal_mode);
    x(ctl, i).portal_mode = 2;
}

/// Operate 70 `0x0058E6A0`: from the Throne to the Chamber (entry 11)
/// only once it is open; elsewhere to the Throne (entry 0). Returns 0.
pub fn portal_operate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, player: UnitId) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    if w.unit_level(player) == Some(THRONE) {
        if xr(ctl, i).chamber_open {
            w.warp_to_level(player, CHAMBER, 11);
        }
    } else {
        w.warp_to_level(player, THRONE, 0);
    }
    0
}

/// Init 77 `0x0058E710` (object 565): mode := +0x9C; +0x9C := 2.
pub fn last_portal_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    w.set_object_mode(object, xr(ctl, i).last_portal_mode);
    x(ctl, i).last_portal_mode = 2;
}

/// Operate 72 `0x0058E740`: the last portal. Returns 0.
pub fn last_portal_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    player: UnitId,
) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    if xr(ctl, i).last_portal_mode != 2 {
        return 0;
    }
    if !flags(w, player).get(SLOT, bit::PRIMARY_GOAL_DONE) {
        w.attach_sound(player, SOUND_REFUSED);
        return 0;
    }
    w.warp_to_level(player, HARROGATH, 0);
    ctl.save_pass();
    if w.client_idle(player) {
        w.set_interact_unit(player, None);
        w.set_player_byte_4c(player, 1);
        // `0x0053D940`.
        w.send(player, &[0x61, 0x07]);
        set(w, player, SLOT, 10);
    }
    0
}

/// Init 79 `0x0058E830` (object 567, the zoo): once per game, up to 10
/// draws of 1 + roll(N − 1) on the quest seed; a zoo-flagged id ends the
/// draws, none → 0. Then 0x50 to every player.
pub fn zoo_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if xr(ctl, i).zoo_chosen {
        return;
    }
    let n = w.monstats_rows();
    let mut id = 0;
    for _ in 0..ZOO_TRIES {
        let c = 1 + ctl.seed.roll(n.wrapping_sub(1) as i32);
        if w.zoo_eligible(c) {
            id = c;
            break;
        }
    }
    let e = x(ctl, i);
    e.zoo_id = id;
    e.zoo_chosen = true;
    for p in w.players() {
        send_zoo(ctl, w, i, p);
    }
}

/// `0x0058E600` (from the Baal throne AI at `0x005EF67B`): the
/// Worldstone Chamber opens; not-intro and status < 3 → status 3 to all.
pub fn chamber_open<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    x(ctl, i).chamber_open = true;
    let r = &ctl.records[i];
    if r.not_intro && r.status < 3 {
        late::status_to_all(ctl, w, i, 3);
    }
}

/// `0x0058E640` (`quests.md` §8.2): the warp to level 132 is open only
/// with +0x86 = 1.
pub fn chamber_warp_open(ctl: &QuestControl) -> bool {
    ctl.find(CHAIN).is_some_and(|i| xr(ctl, i).chamber_open)
}

/// `0x0058E920(game, room, unit)` (from `0x005AD952`, `0x005AD9AB`,
/// `0x005B0A7F`, §8.8): spot := the unit's position − (5, 5), searched
/// from `room` (`0x00545340` size 5, mask 0x400, radius 19 unused, limit
/// 100, `0x0058E940`); found → tyrael3 there (`0x005B2F20`: the found
/// room, mode 1, spread 4, flags 0x42 = skip normal mods + skip party
/// minions). No spot → nothing.
pub fn spawn_tyrael<W: QuestWorld>(w: &mut W, room: RoomId, unit: UnitId) -> Option<UnitId> {
    let (ux, uy) = w.unit_xy(unit)?;
    spawn_tyrael_at(w, room, ux, uy)
}

/// [`spawn_tyrael`] with the unit's position (ux, uy) read by the caller.
pub fn spawn_tyrael_at<W: QuestWorld>(w: &mut W, room: RoomId, ux: i32, uy: i32) -> Option<UnitId> {
    let (sx, sy, r) = w.free_spot_at(room, ux - 5, uy - 5, 5, 0x400, 19, 100)?;
    w.spawn_monster_flags(r, sx, sy, TYRAEL3, 1, 4, 0x42)
}

#[cfg(test)]
#[path = "q6_tests.rs"]
mod tests;
