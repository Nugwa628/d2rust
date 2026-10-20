// Spec: specs/world/npc.md §6, §8
//! NPC services: imbue, socket, personalize (§8.1), Akara's respec
//! (§8.2), act travel (§8.3); the inventory view of Cain identify (§6).

use super::class::*;
use super::{service_result, NpcVendors, NpcWorld};
use crate::units::UnitId;

/// Where an inventory item sits (§6 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// In a grid with that inventory page (0 backpack, 3 cube, 4 stash).
    Grid(u8),
    /// Equipped (node page 3).
    Equipped,
    Belt,
    Other,
}

/// One inventory entry in inventory order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvEntry {
    pub item: UnitId,
    pub place: Place,
    /// Item flags (0x10 = identified).
    pub flags: u32,
}

/// The item facts the §8.1 predicates read (items spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemFacts {
    pub gold: bool,
    /// Item flags (0x100 broken, 0x800 socketed, 0x1000,
    /// 0x1000000 personalized).
    pub flags: u32,
    /// Item type record `bitfield1`.
    pub bitfield1: u32,
    pub throwable: bool,
    /// Unit flags (bit 25 lifts the throwable test).
    pub unit_flags: u32,
    pub quest: bool,
    pub code: [u8; 4],
    /// The item holds socketed items.
    pub has_socketed: bool,
    pub quality: u8,
    pub item_type: u16,
    /// `0x0062BC20`.
    pub max_sockets: u32,
    /// Stat 194.
    pub stat194: i32,
    /// `Nameable`.
    pub nameable: bool,
    pub ethereal: bool,
}

/// What the imbue sets in the drop request filled from the input
/// (§8.1 Imbue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImbueMods {
    /// OR-ed into the request flags: 0x20 | (ethereal ? 4 : 2).
    pub flags: u32,
    /// Game +0x78.
    pub format: u16,
    pub quality: u8,
    pub level: u32,
}

const FLAG_BROKEN: u32 = 0x100;
const FLAG_SOCKETED: u32 = 0x800;
const FLAG_1000: u32 = 0x1000;
const FLAG_PERSONALIZED: u32 = 0x0100_0000;
const LEG: [u8; 4] = *b"leg ";

/// `0x0062C590` (Charsi imbue).
pub fn can_imbue(f: &ItemFacts) -> bool {
    !f.gold
        && f.flags & FLAG_1000 == 0
        && f.bitfield1 & 1 != 0
        && (!f.throwable || f.unit_flags & (1 << 25) != 0)
        && (!f.quest || f.code == LEG)
        && !f.has_socketed
        && f.flags & FLAG_SOCKETED == 0
        && (1..=3).contains(&f.quality)
}

/// `0x0062C770` (Larzuk socket).
pub fn can_socket(f: &ItemFacts) -> bool {
    !f.gold
        && f.flags & FLAG_1000 == 0
        && (!f.quest || f.code == LEG)
        && f.flags & FLAG_BROKEN == 0
        && !f.has_socketed
        && f.flags & FLAG_SOCKETED == 0
        && f.max_sockets > 0
        && f.stat194 == 0
}

/// `0x0062C6A0` (Drehya personalize).
pub fn can_personalize(f: &ItemFacts) -> bool {
    !f.gold
        && f.flags & FLAG_1000 == 0
        && !matches!(f.item_type, 5..=7)
        && f.flags & FLAG_BROKEN == 0
        && f.flags & FLAG_PERSONALIZED == 0
        && !f.has_socketed
        && f.nameable
}

/// §8.1 imbue item level: base level (at least 1, `0x00558200`) + 4 if
/// above 5.
pub fn imbue_level(base_level: u32) -> u32 {
    let l = base_level.max(1);
    if l > 5 {
        l + 4
    } else {
        l
    }
}

/// §8.1 socket count of the duplicate: quality 4 rolls on the item seed;
/// 5–9 give 1; others keep `max`.
pub fn socket_count(seed: &mut crate::rng::Seed, quality: u8, max: u32) -> u32 {
    match quality {
        4 => seed.roll(max.min(2) as i32) + 1,
        5..=9 if max > 0 => 1,
        _ => max,
    }
}

const DONE: u8 = 6;
const REFUSED: u8 = 7;

fn refuse<W: NpcWorld>(w: &mut W, player: UnitId, npc_guid: u32, item: UnitId) {
    w.send(player, &service_result(npc_guid, REFUSED));
    w.put_back(player, item);
}

/// §8.1: imbue (charsi), socket (larzuk), personalize (drehya).
pub(super) fn item_service<W: NpcWorld + NpcVendors>(
    w: &mut W,
    player: UnitId,
    npc: UnitId,
    class: u16,
    item_guid: u32,
) {
    // `0x00578610`: the item must be the cursor item.
    let Some(item) = w.cursor_item(player).filter(|&i| w.guid(i) == item_guid) else {
        return;
    };
    let g = w.guid(npc);
    let f = w.quest_flags(player);
    let facts = w.item_facts(item);
    let (gate, ok) = match class {
        CHARSI => (f.get(3, 1), can_imbue(&facts)),
        LARZUK => (f.get(35, 1), can_socket(&facts)),
        _ => (f.get(38, 1), can_personalize(&facts)),
    };
    if !gate || !ok {
        refuse(w, player, g, item);
        return;
    }
    match class {
        CHARSI => imbue(w, player, g, item, &facts),
        LARZUK => socket(w, player, g, item),
        _ => personalize(w, player, g, item),
    }
}

fn imbue<W: NpcWorld + NpcVendors>(
    w: &mut W,
    player: UnitId,
    g: u32,
    input: UnitId,
    facts: &ItemFacts,
) {
    let mods = ImbueMods {
        flags: 0x20 | if facts.ethereal { 4 } else { 2 },
        format: w.item_format(),
        quality: 6,
        level: imbue_level(w.base_stat(player, super::stat::LEVEL)),
    };
    let name = w.personal_name(input);
    if !w.remove_cursor_item(player, input) {
        refuse(w, player, g, input);
        return;
    }
    let Some(new) = w.create_imbued(player, input, &mods) else {
        // The input is lost.
        w.send(player, &service_result(g, REFUSED));
        return;
    };
    w.repair(new);
    w.item_refresh(new);
    w.set_item_page(new, 0);
    w.set_personal_name(new, &name);
    w.place_or_drop(player, new);
    w.imbue_granted(player);
    w.send(player, &service_result(g, DONE));
}

fn socket<W: NpcWorld + NpcVendors>(w: &mut W, player: UnitId, g: u32, input: UnitId) {
    // TODO(npc §8.1 Socket): "either fails → refuse" read in order: a
    // failed duplicate refuses before the removal; a failed removal
    // refuses and leaves the duplicate where `0x0055A2A0` put it.
    let Some(dup) = w.duplicate(player, input) else {
        refuse(w, player, g, input);
        return;
    };
    if !w.remove_cursor_item(player, input) {
        refuse(w, player, g, input);
        return;
    }
    w.set_item_flag(dup, FLAG_SOCKETED);
    let max = w.max_sockets(dup);
    let quality = w.item_facts(dup).quality;
    let n = socket_count(w.item_seed(dup), quality, max);
    w.add_sockets(dup, n);
    w.repair(dup);
    w.item_refresh(dup);
    w.set_item_page(dup, 0);
    w.place_or_drop(player, dup);
    w.socket_granted(player);
    w.send(player, &service_result(g, DONE));
}

fn personalize<W: NpcWorld + NpcVendors>(w: &mut W, player: UnitId, g: u32, input: UnitId) {
    let Some(dup) = w.duplicate(player, input) else {
        // Edge case 6: the original continues with a null duplicate;
        // D2RUST stops after the refusal (Open question 4).
        refuse(w, player, g, input);
        return;
    };
    if !w.remove_cursor_item(player, input) {
        refuse(w, player, g, input);
        return;
    }
    w.repair(dup);
    w.set_item_page(dup, 0);
    w.place_or_drop(player, dup);
    w.set_item_flag(dup, FLAG_PERSONALIZED);
    let name = w.player_name(player);
    w.set_personal_name(dup, &name);
    w.personalize_granted(player);
    w.send(player, &service_result(g, DONE));
}

/// §8.2: Akara's respec (any action ∉ {1, 2, 3}).
pub(super) fn respec<W: NpcWorld>(w: &mut W, player: UnitId, difficulty: u8) {
    if difficulty == 2 {
        let f = w.quest_flags(player);
        if f.get(1, 0) && !f.get(41, 1) && !f.get(41, 0) {
            w.respec_offer(player);
        }
    }
    if w.quest_flags(player).get(41, 1) {
        // Skills (`0x00570360`) then stats (`0x00570C80`), in that order.
        w.reset_skills(player);
        w.reset_stats(player);
        w.respec_sound(player);
        w.respec_done(player);
    }
}

/// §8.3: act travel.
pub(super) fn act_travel<W: NpcWorld>(
    w: &mut W,
    player: UnitId,
    npc: UnitId,
    class: u16,
    action: u32,
    expansion: bool,
) {
    let f = w.quest_flags(player);
    // (slot gate, level, act completion "from", waypoint) or a plain
    // act change (level, arg).
    let quest = |w: &mut W, level: u32, from: u32| {
        // §8.3: the act change (arg 0), the act completion, then the
        // waypoint.
        w.act_change(player, level, 0);
        w.act_completion(player, npc, level, from);
        w.activate_waypoint(player, level);
    };
    match (class, action) {
        (WARRIV1, _) if f.get(6, 0) => quest(w, 40, 1),
        (WARRIV2, _) => w.act_change(player, 1, 5),
        (MESHIF1, 0) if f.get(14, 0) => quest(w, 75, 40),
        (MESHIF2, 0) => w.act_change(player, 40, 5),
        (TYRAEL2, 0) if expansion && f.get(26, 0) => quest(w, 109, 103),
        (CAIN6, 0) => w.act_change(player, 103, 5),
        _ => {}
    }
}
