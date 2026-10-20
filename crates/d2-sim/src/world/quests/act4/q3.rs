// Spec: specs/world/quests-act4.md §4 (A4Q3 Hell's Forge), §7, §8
//! A4Q3 Hell's Forge (chain 24, slot 27) callback by callback: events 0,
//! 2, 3, 4, 8, 9, 10, 11, 13, 14, the active function, the flag iterate
//! `0x005B5FC0`, the Hellforge object's init and operate functions
//! (object 376, §4.6), its event 7 with the gem and rune drops (§4.7),
//! Hephasto's hammer (§4.8) and the creation hooks (§8).

use super::super::late::{
    add_guid, add_state, clear, completion_flag, flags, guid_listed, in_act, iterate_all, leave,
    party_of, quick_remove, refresh, reset_progress, set, set_state, status_silent, status_to_all,
};
use super::super::{bit, event, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};
use crate::units::UnitId;

const CHAIN: u8 = 24;
const SLOT: u8 = 27;
/// `cain4`.
pub const CAIN4: u16 = 246;
/// `hephasto` (monster base id).
pub const HEPHASTO_BASE: u16 = 409;
/// The Hellforge (`objects.txt` row 376).
pub const HELLFORGE: u16 = 376;
/// Mephisto's Soulstone.
pub const SOULSTONE: [u8; 4] = *b"mss ";
/// Hellforge Hammer.
pub const HAMMER: [u8; 4] = *b"hfh ";
/// The Pandemonium Fortress.
const TOWN: u32 = 103;
/// Sound of a refused operate.
const SOUND_REFUSED: u16 = 19;
/// FX byte of the smash.
const FX_SMASHED: u8 = 14;
/// Live `objects.txt` Hellforge `FrameCnt1` and `FrameCnt3` (§1.4).
const FRAME_CNT1: i32 = 22;
const FRAME_CNT3: i32 = 22;
/// Frames between gem rounds (§4.7).
const ROUND_DELAY: i32 = 20;
/// Item qualities of `0x00559A30`: normal, unique.
const NORMAL: u8 = 2;
const UNIQUE: u8 = 7;

/// `0x0073E56C`: perfect gems and skull (tier 4).
pub const PERFECT_GEMS: [[u8; 4]; 7] = [
    *b"gpv ", *b"gpr ", *b"gpb ", *b"gpy ", *b"gpg ", *b"gpw ", *b"skz ",
];
/// `0x0073E588`: flawless gems and skull (tiers 3, 2).
pub const FLAWLESS_GEMS: [[u8; 4]; 7] = [
    *b"gzv ", *b"glr ", *b"glb ", *b"gly ", *b"glg ", *b"glw ", *b"skl ",
];
/// `0x0073E5A4`: standard gems and skull (tier 1).
pub const STANDARD_GEMS: [[u8; 4]; 7] = [
    *b"gsv ", *b"gsr ", *b"gsb ", *b"gsy ", *b"gsg ", *b"gsw ", *b"sku ",
];

/// The rune of `index` 0–10 by difficulty: `0x0073E114` (`r01`–`r11`),
/// `0x0073E514` (`r12`–`r22`), `0x0073E540` (`r15`–`r25`).
pub fn rune_code(difficulty: u8, index: u32) -> [u8; 4] {
    let first = match difficulty {
        0 => 1,
        1 => 12,
        _ => 15,
    };
    let n = first + index;
    [b'r', b'0' + (n / 10) as u8, b'0' + (n % 10) as u8, b' ']
}

/// Quest extra data (record +0x18, 0x20 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the Hellforge mode to restore (0, 2 soulstone placed, 3
    /// smashed).
    pub forge_mode: u16,
    /// +0x02: Cain started (chat end pending).
    pub started: bool,
    /// +0x03: gem drops pending.
    pub gems_pending: bool,
    /// +0x04: the soulstone was smashed.
    pub smashed: bool,
    /// +0x08: the gem tier (4 → 0).
    pub tier: i32,
    /// +0x0C: hammer hits (whole game).
    pub hits: i32,
    /// +0x10: hammers in the game (written, never read).
    pub hammers: i32,
    /// +0x14: gem sets (players credited at the smash).
    pub sets: i32,
    /// +0x1C: Cain gave a soulstone.
    pub gave_stone: bool,
}

/// Timers this quest makes: none (its delays are object events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init `0x005B6610` beyond `quests.tsv` (§2: state 0, set by
/// `act4::init`; the 0x20 extra bytes zeroed).
pub fn init(r: &mut super::super::QuestRecord) {
    r.extra.a4.q3 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a4.q3
}

/// "lacking 27.0 and 27.1".
fn uncredited(f: &QuestFlags) -> bool {
    !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING)
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
        event::ITEM_PICKED_UP => {
            // `0x005B5A90`.
            let Some(p) = args.player else { return true };
            let f = flags(w, p);
            let r = &mut ctl.records[i];
            if uncredited(&f)
                && !f.get(SLOT, bit::PRIMARY_GOAL_DONE)
                && r.not_intro
                && r.status != 13
                && r.state == 0
            {
                r.state = 1;
            }
        }
        event::MONSTER_KILLED => hephasto_killed(ctl, w, i, args),
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x005B64A0`.
            if args.target.and_then(|t| w.item_code(t)) == Some(HAMMER) {
                x(ctl, i).hammers -= 1;
            }
        }
        event::PLAYER_LEAVES_GAME => {
            // `0x005B6050`.
            match args.player {
                Some(p) => leave(ctl, w, i, p),
                None => ctl.records[i].guids.remove(u32::MAX),
            }
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x005B64C0`.
            if args.player.is_some_and(|p| w.has_item(p, HAMMER)) {
                x(ctl, i).hammers += 1;
            }
        }
        _ => return false,
    }
    true
}

/// Flag iterate `0x005B5FC0` for every player (§4.5).
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let r = &ctl.records[i];
    let (state, gave) = (r.state, r.extra.a4.q3.gave_stone);
    for p in w.players() {
        if !uncredited(&flags(w, p)) {
            continue;
        }
        match state {
            2 if gave => set(w, p, SLOT, bit::CUSTOM1),
            2 => set(w, p, SLOT, bit::STARTED),
            3 => set(w, p, SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// Event 0 `0x005B62D0` (§4.3).
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    // NPC chat always has a player.
    let Some(p) = args.player else { return };
    let npc = args
        .target
        .and_then(|n| w.monster_class(n))
        .unwrap_or(u16::MAX);
    let f = flags(w, p);
    if f.get(SLOT, bit::REWARD_PENDING) {
        return add_state(ctl, i, list, npc, 2);
    }
    if guid_listed(ctl, w, i, p) {
        return add_state(ctl, i, list, npc, 3);
    }
    let r = &ctl.records[i];
    if r.state == 0
        || f.get(SLOT, bit::REWARD_GRANTED)
        || (r.state >= 4 && !f.get(SLOT, bit::PRIMARY_GOAL_DONE))
        || !r.not_intro
    {
        return;
    }
    let holds = w.has_item(p, SOULSTONE);
    let offer = !r.extra.a4.q3.gave_stone && r.status < 3;
    if r.state == 1 && holds {
        add_state(ctl, i, list, npc, 0);
    } else if !holds && offer {
        add_state(ctl, i, list, npc, 1);
    }
}

/// Active `0x005B5EE0` (§4.3).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    let r = &ctl.records[i];
    npc_class == CAIN4
        && uncredited(&flags(w, player))
        && r.not_intro
        && r.state == 1
        && !r.extra.a4.q3.gave_stone
}

/// Event 11 `0x005B6100` (§4.4; NPC 246 only; a = NPC class, b =
/// message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(CAIN4) {
        return;
    }
    let refresh_npc = |ctl: &mut QuestControl, w: &mut W| {
        if let Some(n) = args.target {
            refresh(ctl, w, p, n);
        }
    };
    match args.b {
        678 => {
            x(ctl, i).started = true;
            set_state(ctl, i, 2);
            refresh_npc(ctl, w);
        }
        679 => {
            // `0x005466B0(game, player, 'mss ', 0, 2, 1)`.
            if w.reward_item(p, SOULSTONE, 0, NORMAL, true).is_none() {
                return;
            }
            x(ctl, i).started = true;
            if ctl.records[i].state < 2 {
                set_state(ctl, i, 2);
            }
            x(ctl, i).gave_stone = true;
            refresh_npc(ctl, w);
        }
        680 => {
            let f = flags(w, p);
            if !f.get(SLOT, bit::REWARD_PENDING) {
                return;
            }
            if f.get(SLOT, bit::PRIMARY_GOAL_DONE) && ctl.records[i].state != 5 {
                ctl.records[i].flags = 0;
                status_silent(ctl, i, 13);
                set_state(ctl, i, 5);
                super::sequence(ctl, w, CHAIN);
            }
            set(w, p, SLOT, bit::REWARD_GRANTED);
            clear(w, p, SLOT, bit::REWARD_PENDING);
            add_guid(ctl, w, i, p);
            refresh_npc(ctl, w);
        }
        _ => {}
    }
}

/// Event 2 `0x005B6440` (§4.4, edge case 7): the status goes out before
/// the flags byte is cleared; the callback removes itself.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(CAIN4) || !x(ctl, i).started {
        return;
    }
    let n = if x(ctl, i).gave_stone { 4 } else { 1 };
    ctl.records[i].status = n;
    iterate_all(ctl, w, i);
    ctl.records[i].flags = 0;
    x(ctl, i).started = false;
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    flag_iterate(ctl, w, i);
}

/// Event 3 `0x005B6080` (a = old level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a != TOWN {
        return;
    }
    let Some(p) = args.player else { return };
    quick_remove(ctl, w, i, p);
    if ctl.records[i].state == 2 && uncredited(&flags(w, p)) {
        set_state(ctl, i, 3);
        flag_iterate(ctl, w, i);
    }
}

/// Event 13 `0x005B64E0` (§4.5).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if w.has_item(p, HAMMER) {
        x(ctl, i).hammers += 1;
    }
    let f = flags(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
        return;
    }
    let holds = w.has_item(p, SOULSTONE);
    let r = &mut ctl.records[i];
    if !holds {
        r.state = 1;
    } else if f.get(SLOT, bit::CUSTOM1) {
        r.extra.a4.q3.gave_stone = true;
        (r.status, r.state) = (4, 2);
    } else if f.get(SLOT, bit::LEAVE_TOWN) {
        (r.status, r.state) = (1, 3);
    } else if f.get(SLOT, bit::STARTED) {
        (r.status, r.state) = (1, 2);
    }
}

/// Event 8 `0x005B65D0`: Hephasto's hammer (§4.8, edge case 8).
fn hephasto_killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    let Some(v) = args.target else { return };
    // `0x00559A30`'s `&level` is an output (open question 4): the level
    // comes from the dropping unit (§4.7), so none is passed.
    if w.drop_item_at(v, HAMMER, UNIQUE) {
        x(ctl, i).hammers += 1;
    }
}

/// The status function: chain 24 has none (§2); never reached.
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

// ------------------------------------------------------- §4.6 the forge

/// Init 48 `0x005B5A20` of the Hellforge (§4.6).
pub fn forge_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN).filter(|&i| ctl.records[i].not_intro) else {
        w.set_object_mode(object, 3);
        return;
    };
    let mode = i32::from(x(ctl, i).forge_mode);
    w.set_object_mode(object, mode);
    if !matches!(ctl.records[i].status, 13 | 2 | 3) {
        status_to_all(ctl, w, i, 2);
    }
}

/// Operate 49 `0x005B5C10` of the Hellforge (§4.6; returns 0 in every
/// case).
pub fn forge_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !uncredited(&flags(w, player)) {
        w.attach_sound(player, SOUND_REFUSED);
        return;
    }
    match w.object_mode(object) {
        0 => {
            if w.has_item(player, SOULSTONE) {
                w.set_object_mode(object, 1);
                let at = w.frame() + FRAME_CNT1;
                w.schedule_object_event(object, 1, at);
                x(ctl, i).forge_mode = 2;
                status_to_all(ctl, w, i, 3);
                w.delete_item(player, SOULSTONE);
            } else {
                w.attach_sound(player, SOUND_REFUSED);
                let r = &mut ctl.records[i];
                if r.not_intro && r.state == 0 {
                    r.state = 1;
                }
            }
        }
        2 => {
            if !w.has_item(player, HAMMER) || w.wielded_weapon_code(player) != Some(HAMMER) {
                w.attach_sound(player, SOUND_REFUSED);
                return;
            }
            x(ctl, i).hits += 1;
            if x(ctl, i).hits <= 2 {
                return;
            }
            smash(ctl, w, i, object, player);
        }
        _ => {}
    }
}

/// The third valid hammer hit (§4.6, edge case 5).
fn smash<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    object: UnitId,
    player: UnitId,
) {
    w.set_object_mode(object, 3);
    x(ctl, i).forge_mode = 3;
    w.delete_item(player, HAMMER);
    set(w, player, SLOT, bit::PRIMARY_GOAL_DONE);
    set(w, player, SLOT, bit::REWARD_PENDING);
    reset_progress(w, player, SLOT);
    let e = x(ctl, i);
    e.smashed = true;
    e.gems_pending = true;
    e.tier = 4;
    e.sets = 1;
    // `0x005B5B00` per member.
    for m in party_of(w, player) {
        if in_act(w, m, 3) && uncredited(&flags(w, m)) {
            w.delete_item(m, HAMMER);
            w.delete_item(m, SOULSTONE);
            set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
            set(w, m, SLOT, bit::REWARD_PENDING);
            reset_progress(w, m, SLOT);
            x(ctl, i).sets += 1;
        }
    }
    set_state(ctl, i, 4);
    status_to_all(ctl, w, i, 13);
    let at = w.frame() + FRAME_CNT3;
    w.schedule_quest_event(object, at);
    // `0x005B5BB0`.
    completion_flag(w, CHAIN, SLOT, &[bit::REWARD_GRANTED, bit::REWARD_PENDING]);
    ctl.unique_event(w, FX_SMASHED);
}

/// `0x005B6710`: object event 7 of the Hellforge (class 376, §4.7): one
/// gem round per event, then the rune.
pub fn forge_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if x(ctl, i).smashed {
        w.set_object_mode(object, 4);
    }
    if !x(ctl, i).gems_pending || x(ctl, i).sets <= 0 {
        return;
    }
    // The level slot (50, edge case 16) is passed by address, but
    // `0x00559A30` writes it from the dropping unit before reading it
    // (`0x00559AF8`, open question 4): the 50 has no effect.
    let mut count = 0;
    for _ in 0..x(ctl, i).sets {
        let table = match x(ctl, i).tier {
            4 => &PERFECT_GEMS,
            2 | 3 => &FLAWLESS_GEMS,
            1 => &STANDARD_GEMS,
            _ => return,
        };
        let code = table[(ctl.seed.step() % 7) as usize];
        if w.drop_item_at(object, code, NORMAL) {
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    let e = x(ctl, i);
    e.tier -= 1;
    if e.tier > 0 {
        let at = w.frame() + w.tick_rate().ticks(ROUND_DELAY);
        w.schedule_quest_event(object, at);
        return;
    }
    e.gems_pending = false;
    if w.expansion() {
        let rune = rune_code(w.difficulty(), ctl.seed.roll(11));
        w.drop_item_at(object, rune, NORMAL);
    }
}

// ------------------------------------------------------------ §8 hooks

/// The monster creation switch `0x005B1CF0` for base 409 (§8): link to
/// chain 24 (`0x005436B0`).
pub fn link_hephasto<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) -> bool {
    ctl.add_link(w, unit, CHAIN, None)
}

/// Item creation `0x00555D20` for `hfh ` (§8): link to chain 24.
pub fn link_hammer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, item: UnitId) -> bool {
    ctl.add_link(w, item, CHAIN, None)
}

/// `0x005B6930`, called at Mephisto's death (`quests-act3.md` §8.5): a
/// `ret` stub.
pub fn mephisto_killed() {}

#[cfg(test)]
#[path = "q3_tests.rs"]
mod tests;
