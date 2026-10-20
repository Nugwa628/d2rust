// Spec: specs/world/quests-act5.md §5 (A5Q3 Prison of Ice, chain 33)
//! A5Q3 callback by callback: the flag iterate (§5.2), chat and active
//! function (§5.3), messages and chat end (§5.4), level changes (§5.5),
//! frozen Anya, her dummy and the thaw timer (§5.6), the rewards and the
//! Scroll of Resistance (§5.7), Anya's portals (§5.8), the town NPCs,
//! their dummies and the town cleanup (§5.9), game start, join and leave
//! (§5.10) and the status function (§5.11). Slot 37.

use super::super::late::{
    add_state, clear, completion_flag, flags, guid_listed, in_act, leave, party_of, quick_remove,
    refresh, s5d, send_flags, set, set_state, status_to_all, table_state,
};
use super::super::{
    bit, event, flags_of, EventArgs, GuidList, QuestControl, QuestFlags, QuestRecord, QuestWorld,
    TextList, TimerFn,
};
use crate::units::UnitId;

const CHAIN: u8 = 33;
const SLOT: u8 = 37;
/// Anya in town, Nihlathak, Malah, frozen Anya (monsters).
pub const DREHYA: u16 = 512;
pub const MALAH: u16 = 513;
pub const NIHLATHAK: u16 = 514;
pub const DREHYAICED: u16 = 527;
/// Frozen Anya (object 558 `fana`) and the portal object.
pub const FROZEN_ANYA: u16 = 558;
pub const PORTAL: u16 = 189;
/// Nihlathak Boss (superunique 60).
pub const NIHLATHAK_BOSS: u16 = 60;
/// Malah's thawing potion and the Scroll of Resistance.
pub const THAWING_POTION: [u8; 4] = *b"ice ";
pub const RESIST_SCROLL: [u8; 4] = *b"tr2 ";
/// 37.3 and the reward bits 37.6–37.10 (§5.1).
const LEFT_TOWN: u8 = 3;
const BIT6: u8 = 6;
const SCROLL_USED: u8 = 7;
const SCROLL_GIVEN: u8 = 8;
const ITEM_GIVEN: u8 = 9;
const ITEM_TAKEN: u8 = 10;
/// `0x00734304`: table state by record state.
const MSG_STATE: [i8; 7] = [-1, 0, 1, 2, 3, 4, 5];
/// Anya freed: FX byte (§5.6).
const FX_ANYA: u8 = 16;
/// Object event delays (frames).
const DELAY: i32 = 25;
const TOWN_DUMMY_DELAY: i32 = 12;
/// Stats of the resistance list: fire, lightning, cold, poison resist.
pub const RESIST_STATS: [u16; 4] = [39, 41, 43, 45];

/// Quest extra data (record +0x18, 0x114 bytes, §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: a GUID list (reset at init; event 10 removes from it).
    pub guids: GuidList,
    /// +0x84: Anya 0 frozen, 1 thawed, 2 back in town.
    pub anya: u8,
    /// +0x88: Nihlathak gone from town.
    pub nihlathak_gone: bool,
    /// +0x8C: thaw step.
    pub thaw_step: u8,
    /// +0x90 / +0x98: the thawed monster spawned / its GUID.
    pub thawed_spawned: bool,
    pub thawed_guid: u32,
    /// +0x91 / +0x94: Anya in town / her GUID.
    pub anya_in_town: bool,
    pub anya_guid: u32,
    /// +0x92 / +0xA0: Nihlathak boss spawned / its GUID.
    pub boss_spawned: bool,
    pub boss_guid: u32,
    /// +0x93 / +0x9C: Nihlathak in town / his GUID.
    pub nihlathak_in_town: bool,
    pub nihlathak_guid: u32,
    /// +0xA4 / +0xA8: frozen object spawned / its GUID.
    pub frozen_spawned: bool,
    pub frozen_guid: u32,
    /// +0xAC: Malah started the quest (chat end pending).
    pub started: bool,
    /// +0xAD: the outside portal may close.
    pub outside_may_close: bool,
    /// +0xAE: the town portal may close.
    pub town_may_close: bool,
    /// +0xB1 / +0xC0: outside portal spawned / GUID; +0xC4 / +0xC8 its
    /// position.
    pub outside_portal: bool,
    pub outside_portal_guid: u32,
    pub outside_pos: (i32, i32),
    /// +0xB2 / +0xB4: town portal spawned / GUID.
    pub town_portal: bool,
    pub town_portal_guid: u32,
    /// +0xD8: thawing potions in the game.
    pub potions: i32,
    /// +0xDC: town portal close counter.
    pub town_close_counter: i32,
    /// +0xE1 / +0xE4 / +0xE8, +0xEC: town dummy seen / GUID / position.
    pub town_dummy_seen: bool,
    pub town_dummy_guid: u32,
    pub town_dummy_pos: (i32, i32),
    /// +0xE3: the scroll reward found the frozen Anya monster.
    pub iced_found: bool,
    /// +0xF0 / +0xF4: thaw position.
    pub thaw_pos: (i32, i32),
    /// +0xF8: outside dummy (460) GUID.
    pub outside_dummy_guid: u32,
    /// +0xFC: frozen object GUID (init 74).
    pub frozen_object_guid: u32,
    /// +0x100: the thaw timer exists.
    pub thaw_timer: bool,
    /// +0x101: potion given (chat end pending).
    pub potion_given: bool,
    /// +0x102: scroll given again this game.
    pub scroll_again: bool,
    /// +0x104 / +0x108: Anya / Nihlathak map-AI records (0: none);
    /// +0x10C / +0x10D: applied.
    pub anya_map_ai: u32,
    pub nihlathak_map_ai: u32,
    pub anya_map_ai_applied: bool,
    pub nihlathak_map_ai_applied: bool,
    /// +0x110: Nihlathak's town dummy (461) GUID.
    pub nihlathak_dummy_guid: u32,
}

/// Timers this quest makes (`quests.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x0058AAB0`: the thaw (period 1, §5.6).
    Thaw,
}

/// Init `0x0058A410` beyond `quests.tsv` (§2): extra zeroed, list +0x00
/// reset.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q3 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q3
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
        event::PLAYER_LEAVES_GAME => {
            // `0x00589D20`.
            if let Some(p) = args.player {
                leave(ctl, w, i, p);
                let g = w.guid(p);
                x(ctl, i).guids.remove(g);
                if w.has_item(p, THAWING_POTION) {
                    x(ctl, i).potions -= 1;
                }
            }
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => game_join(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// Flag iterate `0x00588F10` (§5.2) for every player.
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 => f.set(SLOT, LEFT_TOWN),
            _ => {}
        }
    }
}

/// The bits of 37 the completion flags test (`0x00589AA0`).
const COMPLETION: [u8; 3] = [
    bit::REWARD_GRANTED,
    bit::REWARD_PENDING,
    bit::PRIMARY_GOAL_DONE,
];

/// Event 0 `0x00589B00` (§5.3).
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
    let state = ctl.records[i].state;
    // Step 1.
    if class == DREHYAICED && state == 6 {
        w.kill_in_place(n);
        return;
    }
    // Step 2 (the table-5 re-offer is dead, edge case 3).
    if class == MALAH && state == 4 && !w.has_item(p, THAWING_POTION) && x(ctl, i).potions == 0 {
        add_state(ctl, i, list, class, 3);
        return;
    }
    let f = flags(w, p);
    if !f.get(SLOT, bit::REWARD_PENDING) {
        // Step 3.
        let m = if guid_listed(ctl, w, i, p) {
            6
        } else if f.get(SLOT, bit::REWARD_GRANTED)
            || (state > 4 && !f.get(SLOT, bit::PRIMARY_GOAL_DONE))
            || !ctl.records[i].not_intro
        {
            return;
        } else {
            match table_state(&MSG_STATE, state) {
                Some(m) if m <= 9 => m,
                _ => return,
            }
        };
        add_state(ctl, i, list, class, m);
    } else if !((class == DREHYA && f.get(SLOT, ITEM_GIVEN))
        || (class == MALAH && f.get(SLOT, SCROLL_GIVEN)))
    {
        // Step 4.
        add_state(ctl, i, list, class, 5);
    }
}

/// Active `0x00589F10` (§5.3).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    let pf = flags(w, player);
    let r = &ctl.records[i];
    let potions = r.extra.a5.q3.potions;
    match npc_class {
        MALAH => {
            (pf.get(SLOT, bit::REWARD_PENDING) && !pf.get(SLOT, SCROLL_GIVEN))
                || (!pf.get(SLOT, bit::REWARD_GRANTED)
                    && !pf.get(SLOT, bit::REWARD_PENDING)
                    && (r.state == 1 || (r.state == 4 && potions == 0)))
        }
        DREHYAICED => r.not_intro && r.state <= 4 && !w.has_item(player, THAWING_POTION),
        DREHYA => pf.get(SLOT, bit::REWARD_PENDING) && !pf.get(SLOT, ITEM_GIVEN),
        _ => false,
    }
}

/// Event 11 `0x00589580` (§5.4; a = NPC class, b = message).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    match (args.a, args.b) {
        (_, 20131) => {
            if !ctl.records[i].not_intro {
                return;
            }
            if ctl.records[i].state <= 3 {
                set_state(ctl, i, 4);
                town_cleanup(ctl, w, i);
            }
            if ctl.records[i].status < 3 {
                status_to_all(ctl, w, i, 3);
            }
        }
        (a, 20116) if a == u32::from(MALAH) => {
            x(ctl, i).started = true;
            set_state(ctl, i, 2);
            flag_iterate_all(ctl, w, i);
            if let Some(n) = args.target {
                refresh(ctl, w, p, n);
            }
        }
        (a, 20127) if a == u32::from(MALAH) => {
            if w.has_item(p, THAWING_POTION) || x(ctl, i).potions != 0 {
                return;
            }
            if w.reward_item(p, THAWING_POTION, 0, 2, true).is_some() {
                x(ctl, i).potions += 1;
                s5d(w, p, CHAIN, 0x01, 0);
                x(ctl, i).potion_given = true;
            }
        }
        (a, 20132) if a == u32::from(MALAH) => scroll_reward(ctl, w, i, p),
        (a, 20136) if a == u32::from(DREHYA) => item_reward(ctl, w, p),
        _ => {}
    }
}

/// Event 2 `0x00588F80` (§5.4; never cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(MALAH) {
        return;
    }
    if x(ctl, i).started {
        status_to_all(ctl, w, i, 1);
        x(ctl, i).started = false;
    }
    if ctl.records[i].not_intro && x(ctl, i).potion_given && ctl.records[i].status < 4 {
        status_to_all(ctl, w, i, 4);
        x(ctl, i).potion_given = false;
    }
}

/// Event 3 `0x00589D80` (§5.5; a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let not_intro = ctl.records[i].not_intro;
    // Step 1.
    if args.b == 112 {
        if not_intro && ctl.records[i].state == 0 {
            set_state(ctl, i, 1);
            town_cleanup(ctl, w, i);
        }
    } else if matches!(args.b, 113 | 114) && not_intro {
        let b = ctl.records[i].state <= 2;
        if b {
            set_state(ctl, i, 3);
        }
        if ctl.records[i].status == 0 {
            status_to_all(ctl, w, i, 1);
            flag_iterate_all(ctl, w, i);
        } else if b {
            flag_iterate_all(ctl, w, i);
        }
        town_cleanup(ctl, w, i);
    }
    // Step 2.
    if args.a == 109 {
        quick_remove(ctl, w, i, p);
        let f = flags(w, p);
        // Status and iterate sit inside the state 2 test (`0x00589D80`).
        if ctl.records[i].state == 2
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
        {
            set_state(ctl, i, 3);
            if ctl.records[i].status == 0 {
                status_to_all(ctl, w, i, 1);
            }
            flag_iterate_all(ctl, w, i);
        }
    }
    // Step 3 (edge case 6).
    if (121..=124).contains(&args.b) && not_intro && ctl.records[i].state < 5 {
        completion_flag(w, CHAIN, SLOT, &COMPLETION);
        ctl.records[i].state = 6;
        let e = x(ctl, i);
        e.anya = 2;
        e.nihlathak_gone = true;
    }
}

/// The party step of a freed Anya (`0x00589A50` → `0x00589000`) for
/// each player.
fn party_credit<W: QuestWorld>(w: &mut W) {
    for q in w.players() {
        if !flags(w, q).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in party_of(w, q) {
            let f = flags(w, m);
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
                && in_act(w, m, 4)
            {
                set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                set(w, m, SLOT, bit::REWARD_PENDING);
            }
        }
    }
}

/// The scroll, message 20132 from Malah (§5.7).
fn scroll_reward<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let f = flags(w, p);
    if !f.get(SLOT, bit::REWARD_PENDING) || f.get(SLOT, bit::REWARD_GRANTED) {
        if (f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE))
            && !x(ctl, i).scroll_again
            && f.get(SLOT, SCROLL_GIVEN)
            && !f.get(SLOT, SCROLL_USED)
            && !w.has_item(p, RESIST_SCROLL)
            && w.reward_item(p, RESIST_SCROLL, 0, 2, true).is_some()
        {
            x(ctl, i).scroll_again = true;
        }
        return;
    }
    if w.reward_item(p, RESIST_SCROLL, 0, 2, true).is_none() {
        return;
    }
    set(w, p, SLOT, SCROLL_GIVEN);
    s5d(w, p, CHAIN, 0x02, 0);
    if !f.get(SLOT, ITEM_GIVEN) {
        if ctl.records[i].not_intro && ctl.records[i].status < 6 {
            status_to_all(ctl, w, i, 6);
        }
    } else {
        clear(w, p, SLOT, bit::REWARD_PENDING);
        set(w, p, SLOT, bit::REWARD_GRANTED);
    }
    if x(ctl, i).anya == 1 {
        x(ctl, i).iced_found = false;
        // `0x005890B0` for each monster; returning 1 ends the walk.
        for m in w.monsters() {
            if w.monster_class(m) != Some(DREHYAICED) {
                continue;
            }
            match w.npc_chat_clients(m) {
                // `0x00573180(game, 527, 0x00589070, 0)`: for each entry
                // of the NPC's interaction list, in list order, 0x62 (1,
                // the NPC's GUID), the interaction cleared, then
                // `0x00589070`: S5D(33, 0x20, 0) to that player; the
                // entries are freed and the list emptied. +0xE3 stays 0
                // (edge case 9: iced Anya stays, the drop step still
                // runs).
                Some(chatting) => {
                    let g = w.guid(m);
                    for q in chatting {
                        w.send(q, &crate::world::quests::helpers::msg_end_interaction(1, g));
                        w.set_interact_unit(q, None);
                        s5d(w, q, CHAIN, 0x20, 0);
                    }
                    w.clear_npc_chats(m);
                }
                None => {
                    w.kill_in_place(m);
                    x(ctl, i).iced_found = true;
                }
            }
            break;
        }
        if !x(ctl, i).iced_found {
            w.drop_preset_monster(4, DREHYAICED);
        }
        let e = x(ctl, i);
        e.outside_may_close = true;
        e.anya = 2;
        anya_to_town(ctl, w, i);
    }
    send_flags(w, p);
}

/// The item lists of Anya's reward (§5.7): `tier` 0 normal
/// (`0x00735F6C`), 1 exceptional (`0x00735EFC`), 2 elite
/// (`0x00735F34`); classes amazon, sorceress, necromancer, paladin,
/// barbarian, druid, assassin.
pub fn anya_items(tier: usize, class: u8) -> Option<Vec<[u8; 4]>> {
    const PREFIX: [&[u8; 2]; 6] = [b"am", b"ob", b"ne", b"pa", b"ba", b"dr"];
    const SUFFIX: [&[u8; 5]; 3] = [b"12345", b"6789a", b"bcdef"];
    const ASSASSIN: [[&[u8; 3]; 7]; 3] = [
        [b"ktr", b"wrb", b"axf", b"ces", b"clw", b"btl", b"skr"],
        [b"9ar", b"9wb", b"9xf", b"9cs", b"9lw", b"9tw", b"9qr"],
        [b"7ar", b"7wb", b"7xf", b"7cs", b"7lw", b"7tw", b"7qr"],
    ];
    let suffix = SUFFIX.get(tier)?;
    match class {
        0..=5 => {
            let p = PREFIX[usize::from(class)];
            Some(suffix.iter().map(|&s| [p[0], p[1], s, b' ']).collect())
        }
        6 => Some(
            ASSASSIN[tier]
                .iter()
                .map(|c| [c[0], c[1], c[2], b' '])
                .collect(),
        ),
        _ => None,
    }
}

/// The tier of Anya's item (§5.7): 1 on nightmare with game type 3 or
/// level > 45, 2 on hell with game type 3 or level > 65, else 0.
pub fn anya_tier(difficulty: u8, game_type: u8, level: i32) -> usize {
    match difficulty {
        1 if game_type == 3 || level > 45 => 1,
        2 if game_type == 3 || level > 65 => 2,
        _ => 0,
    }
}

/// Anya's item, message 20136 from Anya (§5.7): one quest-seed roll.
fn item_reward<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: UnitId) {
    let f = flags(w, p);
    if !f.get(SLOT, bit::REWARD_PENDING)
        || f.get(SLOT, bit::REWARD_GRANTED)
        || f.get(SLOT, ITEM_TAKEN)
    {
        return;
    }
    let tier = anya_tier(w.difficulty(), w.game_type(), w.stat(p, 12));
    let Some(list) = anya_items(tier, w.player_class(p)) else {
        return;
    };
    let code = list[ctl.seed.roll(list.len() as i32) as usize];
    let level = w.quest_item_level(p);
    let Some(item) = w.reward_item(p, code, level, 6, true) else {
        return;
    };
    let v = w.item_drop_sound(item).max(0) as u16;
    s5d(w, p, CHAIN, 0x10, v);
    set(w, p, SLOT, ITEM_GIVEN);
    if f.get(SLOT, SCROLL_GIVEN) {
        clear(w, p, SLOT, bit::REWARD_PENDING);
        set(w, p, SLOT, bit::REWARD_GRANTED);
    }
    set(w, p, SLOT, ITEM_TAKEN);
}

/// Using the Scroll of Resistance (the quest part of the item use
/// `0x0055E170`, §5.7): needs 37.8 set and 37.7 clear; true = the
/// scroll is consumed.
pub fn use_resist_scroll<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    let f = flags(w, p);
    if !f.get(SLOT, SCROLL_GIVEN) || f.get(SLOT, SCROLL_USED) {
        return false;
    }
    set(w, p, SLOT, SCROLL_USED);
    apply_resist_scroll(w, p);
    // `0x005458E0`.
    w.send(p, &[0x5D, CHAIN, 0x02, 0, 0, 0]);
    true
}

/// `0x0058A0A0` (also at player load `0x00539A1B`, expansion games
/// only): v = 10 × the player's difficulty records with 37.7; v ≠ 0 →
/// the resistance stat list (edge case 8).
pub fn apply_resist_scroll<W: QuestWorld>(w: &mut W, p: UnitId) {
    let n = w.quests(p).map_or(0, |q| {
        q.flags.iter().filter(|f| f.get(SLOT, SCROLL_USED)).count() as i32
    });
    let v = 10 * n;
    if v != 0 {
        w.add_resist_list(p, v);
    }
}

/// Event 13 `0x0058A110` (§5.10).
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if w.has_item(p, THAWING_POTION) {
        x(ctl, i).potions = 1;
    }
    let f = flags(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
        let e = x(ctl, i);
        e.anya = 2;
        e.nihlathak_gone = true;
        return;
    }
    if !ctl.records[i].not_intro {
        return;
    }
    let r = &mut ctl.records[i];
    if f.get(SLOT, LEFT_TOWN) {
        (r.state, r.status) = (3, 1);
    } else if f.get(SLOT, bit::STARTED) {
        (r.state, r.status) = (2, 1);
    }
    if r.extra.a5.q3.potions != 0 {
        (r.state, r.status) = (3, 4);
    }
}

/// Event 14 `0x0058A1E0` (§5.10).
fn game_join<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if w.has_item(p, THAWING_POTION) {
        x(ctl, i).potions += 1;
    }
    let r = &ctl.records[i];
    if r.not_intro && r.state < 5 {
        return;
    }
    let f = flags(w, p);
    if ctl.record(34).is_some_and(|r| r.not_intro)
        && !f.get(SLOT, bit::REWARD_PENDING)
        && !f.get(SLOT, BIT6)
        && !f.get(SLOT, bit::REWARD_GRANTED)
    {
        set(w, p, 38, bit::COMPLETED_NOW);
    }
}

/// Status `0x0058A300` (§5.11; always true). The party potion test
/// never adds (edge case 4).
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = f;
    let r = &ctl.records[i];
    Some(if pf.get(SLOT, bit::REWARD_GRANTED) {
        0
    } else if pf.get(SLOT, bit::REWARD_PENDING) || pf.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        match (pf.get(SLOT, SCROLL_GIVEN), pf.get(SLOT, ITEM_GIVEN)) {
            (true, false) => 6,
            (false, _) => 5,
            (true, true) => 13,
        }
    } else if !r.not_intro {
        0
    } else if w.has_item(player, THAWING_POTION) {
        4
    } else if pf.get(SLOT, bit::COMPLETED_NOW) {
        12
    } else if r.state > 4 {
        0
    } else {
        r.status
    })
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Thaw => ctl.find(chain).is_some_and(|i| thaw(ctl, w, i)),
    }
}

/// A stored map AI applied once (§5.6, §5.9): Anya's (+0x104, +0x10C)
/// or Nihlathak's (+0x108, +0x10D).
fn apply_map_ai<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    unit: UnitId,
    nihlathak: bool,
) {
    let e = x(ctl, i);
    let (ai, applied) = if nihlathak {
        (e.nihlathak_map_ai, e.nihlathak_map_ai_applied)
    } else {
        (e.anya_map_ai, e.anya_map_ai_applied)
    };
    if ai == 0 || applied || !w.apply_map_ai(unit, ai) {
        return;
    }
    let e = x(ctl, i);
    if nihlathak {
        e.nihlathak_map_ai_applied = true;
    } else {
        e.anya_map_ai_applied = true;
    }
}

/// The map-AI stores `0x0058AD80` (Anya, dummy 459) and `0x0058AE10`
/// (Nihlathak, dummy 461) over `0x00545C90` (§5.8): keep the path's
/// handle (+0x104 / +0x108), then, if that NPC is in town and its unit
/// exists, apply it once.
pub fn map_ai_store<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    map_ai: u32,
    nihlathak: bool,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = x(ctl, i);
    let (in_town, guid) = if nihlathak {
        e.nihlathak_map_ai = map_ai;
        (e.nihlathak_in_town, e.nihlathak_guid)
    } else {
        e.anya_map_ai = map_ai;
        (e.anya_in_town, e.anya_guid)
    };
    if !in_town {
        return;
    }
    if let Some((unit, _)) = w.monster_by_guid(guid) {
        apply_map_ai(ctl, w, i, unit, nihlathak);
    }
}

/// The thaw timer `0x0058AAB0` (§5.6; edge case 5).
fn thaw<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if x(ctl, i).thawed_spawned {
        return false;
    }
    let object = w
        .object_by_guid(x(ctl, i).frozen_object_guid)
        .filter(|o| o.1 == FROZEN_ANYA)
        .map(|o| o.0);
    match x(ctl, i).thaw_step {
        0 => {
            if let Some(o) = object {
                w.set_object_mode(o, 2);
                w.free_object_collision(o);
            }
            x(ctl, i).thaw_step = 1;
            false
        }
        1 => {
            x(ctl, i).thaw_step = 2;
            let Some((_, _, room)) = object.and_then(|o| w.unit_position(o)) else {
                return false;
            };
            let (tx, ty) = x(ctl, i).thaw_pos;
            let Some(a) = w.critical_spawn(room, tx, ty, DREHYAICED) else {
                return false;
            };
            if let Some(o) = object {
                w.object_leave_room(o);
            }
            apply_map_ai(ctl, w, i, a, false);
            let g = w.guid(a);
            let e = x(ctl, i);
            e.thawed_guid = g;
            e.thawed_spawned = true;
            super::sequence(ctl, w, CHAIN);
            x(ctl, i).thaw_timer = false;
            true
        }
        _ => false,
    }
}

/// Init 67 `0x0058A5B0` (dummy 460, Anya outside town, §5.6).
pub fn anya_dummy_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    let g = w.guid(object);
    x(ctl, i).outside_dummy_guid = g;
    if !x(ctl, i).frozen_spawned {
        let frame = w.frame() + w.tick_rate().ticks(DELAY);
        w.schedule_quest_event(object, frame);
    }
}

/// `0x0058A500`: object event 7 of dummy 460 (Anya outside town, §5.6).
pub fn anya_dummy_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro || x(ctl, i).frozen_spawned {
        return;
    }
    let made = w
        .unit_position(object)
        .and_then(|(ox, oy, room)| w.place_object(room, ox, oy, FROZEN_ANYA, [1, 0, 0]));
    match made {
        Some(o) => {
            w.or_unit_flags(o, 0x0300_0000);
            let g = w.guid(o);
            let e = x(ctl, i);
            e.frozen_spawned = true;
            e.frozen_guid = g;
        }
        None => {
            let frame = w.frame() + w.tick_rate().ticks(DELAY);
            w.schedule_quest_event(object, frame);
        }
    }
}

/// Init 74 `0x0058AA50` (object 558, frozen Anya, §5.6).
pub fn frozen_anya_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    x(ctl, i).frozen_object_guid = g;
    if ctl.records[i].not_intro && ctl.records[i].status < 2 {
        status_to_all(ctl, w, i, 2);
    }
}

/// Operate 67 `0x0058ABC0` (object 558, frozen Anya, §5.6). Returns 0.
pub fn frozen_anya_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    if !w.has_item(player, THAWING_POTION) {
        w.open_quest_message(player, object, 20131);
        if ctl.records[i].status == 1 {
            status_to_all(ctl, w, i, 3);
        }
        return 0;
    }
    let f = flags(w, player);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        return 0;
    }
    x(ctl, i).potions -= 1;
    w.delete_item(player, THAWING_POTION);
    set_state(ctl, i, 5);
    x(ctl, i).anya = 1;
    set(w, player, SLOT, bit::PRIMARY_GOAL_DONE);
    set(w, player, SLOT, bit::REWARD_PENDING);
    status_to_all(ctl, w, i, 5);
    ctl.unique_event(w, FX_ANYA);
    party_credit(w);
    completion_flag(w, CHAIN, SLOT, &COMPLETION);
    if !x(ctl, i).thaw_timer {
        x(ctl, i).thaw_timer = true;
        let pos = w.unit_position(object);
        if let Some((ox, oy, _)) = pos {
            x(ctl, i).thaw_pos = (ox, oy);
        }
        let t = TimerFn::Act5(super::Timer::Q3(Timer::Thaw));
        if let Err(e) = ctl.add_timer(CHAIN, t, 1) {
            ctl.faults.push(e);
        }
        if let Some((_, _, room)) = pos {
            w.set_room_portal(room, false);
        }
    }
    0
}

/// `0x0058A730`: object event 7 of an Anya portal (class 189 in level
/// 109 or ≥ 113, §5.8).
pub fn portal_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    match w.object_mode(object) {
        1 => w.set_object_mode(object, 2),
        2 => {
            let e = x(ctl, i);
            let close = if w.unit_level(object) == Some(109) {
                e.town_close_counter += 1;
                if e.town_close_counter > 5 {
                    e.town_may_close = true;
                }
                e.town_may_close
            } else {
                e.outside_may_close
            };
            if close {
                w.set_object_mode(object, 3);
            }
        }
        3 => w.set_object_mode(object, 4),
        _ => {}
    }
    let frame = w.frame() + w.tick_rate().ticks(DELAY);
    w.schedule_quest_event(object, frame);
}

/// Init 66 `0x0058EAC0` (dummy 459, Anya in town, §5.9).
pub fn anya_town_dummy_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    if let Some(i) = ctl.find(CHAIN) {
        let g = w.guid(object);
        let pos = w.unit_position(object);
        let e = x(ctl, i);
        e.town_dummy_guid = g;
        e.town_dummy_seen = true;
        if let Some((ox, oy, _)) = pos {
            e.town_dummy_pos = (ox, oy);
        }
        if e.anya == 2 {
            super::sequence(ctl, w, CHAIN);
            if !x(ctl, i).anya_in_town {
                if let Some((ox, oy, room)) = pos {
                    if let Some(a) = w.critical_spawn(room, ox, oy, DREHYA) {
                        let g = w.guid(a);
                        let e = x(ctl, i);
                        e.anya_guid = g;
                        e.anya_in_town = true;
                        apply_map_ai(ctl, w, i, a, false);
                    }
                }
            }
        }
    }
    // Chain 34 extra +0x87 (part 2 §6.7).
    if ctl.record(34).is_some_and(|r| r.extra.a5.q4.portal_wanted) {
        let frame = w.frame() + w.tick_rate().ticks(TOWN_DUMMY_DELAY);
        w.schedule_quest_event(object, frame);
    }
}

/// Anya to town `0x005891D0` (§5.9).
fn anya_to_town<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let e = x(ctl, i);
    if !e.town_dummy_seen {
        return;
    }
    let Some((dummy, _)) = w.object_by_guid(e.town_dummy_guid) else {
        return;
    };
    // The portal and the seq fn run only after a successful spawn in
    // this call (`0x0058926C`, `0x0058928F`): +0x91 already set, or a
    // failed spawn, returns.
    if x(ctl, i).anya_in_town {
        return;
    }
    let Some((dx, dy, room)) = w.unit_position(dummy) else {
        return;
    };
    let Some(a) = w.critical_spawn(room, dx, dy, DREHYA) else {
        return;
    };
    let g = w.guid(a);
    let e = x(ctl, i);
    e.anya_guid = g;
    e.anya_in_town = true;
    apply_map_ai(ctl, w, i, a, false);
    // The portal: the room covering her position (`0x00463740` from her
    // room), object 189 there with flags (1, 1, 0) (`0x00589313`); no
    // room or no object → no portal, the seq fn still runs.
    let at = w
        .unit_position(a)
        .and_then(|(ax, ay, own)| w.room_at(own, ax, ay).map(|r| (ax, ay, r)));
    if let Some((ax, ay, room)) = at {
        if let Some(o) = w.place_object(room, ax, ay, PORTAL, [1, 1, 0]) {
            let g = w.guid(o);
            let e = x(ctl, i);
            e.town_portal = true;
            e.town_portal_guid = g;
        }
    }
    super::sequence(ctl, w, CHAIN);
}

/// Init 68 `0x0058A610` (dummy 461, Nihlathak in town, §5.9).
pub fn nihlathak_town_dummy_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    x(ctl, i).nihlathak_dummy_guid = g;
    if x(ctl, i).nihlathak_gone || x(ctl, i).nihlathak_in_town {
        return;
    }
    let Some((ox, oy, room)) = w.unit_position(object) else {
        return;
    };
    if let Some(n) = w.critical_spawn(room, ox, oy, NIHLATHAK) {
        let g = w.guid(n);
        let e = x(ctl, i);
        e.nihlathak_guid = g;
        e.nihlathak_in_town = true;
        apply_map_ai(ctl, w, i, n, true);
    }
}

/// Init 69 `0x0058A6C0` (dummy 462, Nihlathak in his temple, §5.9).
pub fn nihlathak_temple_dummy_init<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !x(ctl, i).nihlathak_gone || x(ctl, i).boss_spawned {
        return;
    }
    let Some((ox, oy, room)) = w.unit_position(object) else {
        return;
    };
    if let Some(b) = w.preset_superunique_spawn(room, ox, oy, NIHLATHAK_BOSS) {
        let g = w.guid(b);
        let e = x(ctl, i);
        e.boss_guid = g;
        e.boss_spawned = true;
    }
}

/// Town cleanup `0x005893E0` (§5.9).
pub fn town_cleanup<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    if x(ctl, i).anya_in_town && x(ctl, i).anya != 2 {
        match w.monster_by_guid(x(ctl, i).anya_guid) {
            None => w.drop_preset_monster(4, DREHYA),
            Some((a, _)) => w.npc_leave_town(a),
        }
        x(ctl, i).anya_in_town = false;
    }
    if x(ctl, i).nihlathak_in_town {
        match w.monster_by_guid(x(ctl, i).nihlathak_guid) {
            None => w.drop_preset_monster(4, NIHLATHAK),
            Some((n, _)) => {
                w.kill_in_town(n);
                if let Some((d, _)) = w.object_by_guid(x(ctl, i).nihlathak_dummy_guid) {
                    let frame = w.frame() + 1;
                    w.schedule_quest_event(d, frame);
                }
            }
        }
        x(ctl, i).nihlathak_in_town = false;
    }
    x(ctl, i).nihlathak_gone = true;
}

/// `0x00589540`: object event 7 of dummy 461 (Nihlathak in town, §5.9).
/// "Back" = a monster unit with GUID +0x9C still exists
/// (`0x00552F60(game, 1, +0x9C)`); then `0x00589340` runs on it again.
/// No other test (not +0x93, not his mode); +0x9C is never cleared.
pub fn nihlathak_dummy_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let _ = object;
    let Some(i) = ctl.find(CHAIN) else { return };
    if let Some((n, _)) = w.monster_by_guid(x(ctl, i).nihlathak_guid) {
        w.kill_in_town(n);
    }
}

#[cfg(test)]
#[path = "q3_tests.rs"]
mod tests;
