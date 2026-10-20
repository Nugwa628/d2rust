// Spec: specs/world/quests-act3.md; specs/world/quests-act3-2.md §11
//! Act III quest records (chains 14–20, 28, 39), callback by callback:
//! Hratli's gossip and the intro ([`q0`], [`intro`]), Lam Esen's Tome
//! ([`q1`]), Khalim's Will ([`q2`]), the Blade of the Old Religion
//! ([`q3`]), the Golden Bird ([`q4`]), the Blackened Temple ([`q5`]), the
//! Guardian ([`q6`]) and the Dark Wanderer ([`q7`]). The shared
//! shorthands of §1.1 (status to all, silent status, completion flag,
//! quick remove, the iterate rule of §2) and the sequence chain of §1.3
//! are here; quest objects and the hooks of §10 are public functions of
//! the quest modules, re-exported below.
//!
//! Functions the spec names but leaves to other specs (item creation,
//! monster spawning, object warps, collision) are reached through the
//! Act III seams of [`QuestWorld`]; their default bodies report the
//! function through `QuestWorld::unhandled`.

pub mod intro;
pub mod q0;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod q4;
pub mod q5;
pub mod q6;
pub mod q7;

pub use q0::{hratli_end_init, hratli_start_init};
pub use q1::{tome_init, tome_operate};
pub use q2::{
    chest_operate, lever_event, lever_init, lever_operate, stairs_init, stairs_operate, will_cubed,
    KhalimChest,
};
pub use q3::{
    activate_altar, altar_init, altar_position, decoy_init, decoy_operate, gidbinn_kill_test,
};
pub use q4::{
    alkor_bird_brought, alkor_bird_clear, bird_boss_removed, choose_bird_boss, potion_of_life,
};
pub use q5::{
    council_preset, durance_open, orb_init, orb_operate, stairs_r_init, weapon_in_use, HandItem,
};
pub use q6::{bridge_event, bridge_init, durance_warp, hellgate_init, natalya_init};
pub use q7::{wanderer_init, wanderer_minions, wanderer_target};

use super::{
    bit, event, flags_of, EventArgs, QuestControl, QuestFlags, QuestRecord, QuestWorld, TextList,
};
use crate::units::{RoomId, UnitId};

/// The act index of Act III (`0x006427F0`).
pub const ACT: u8 = 2;
/// Kurast Docks (the town).
pub const DOCKS: u32 = 75;

/// NPC and monster class ids (§Constants).
pub mod npc {
    pub const MEPHISTO: u16 = 242;
    pub const CAIN3: u16 = 245;
    pub const ASHEARA: u16 = 252;
    pub const HRATLI: u16 = 253;
    pub const ALKOR: u16 = 254;
    pub const ORMUS: u16 = 255;
    pub const MESHIF2: u16 = 264;
    pub const NATALYA: u16 = 297;
    pub const VILECHILD1: u16 = 301;
    pub const COMPELLING_ORB: u16 = 366;
    pub const DARK_WANDERER: u16 = 368;
    pub const FETISH11: u16 = 407;
}

/// Sound ids (§Constants).
pub mod sound {
    pub const REFUSED: u16 = 19;
    pub const COUNCIL: u16 = 64;
    pub const GIDBINN: u16 = 65;
    pub const MEPHISTO: u16 = 66;
    pub const TOME: u16 = 67;
    pub const FIGURINE: u16 = 72;
}

/// The Act III timers (§5.6, §8.5, §9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x005B9A30`: the Gidbinn boss spawn attempt (period 7).
    GidbinnBoss,
    /// `0x005BC720`: the Guardian's status 4 after Mephisto (period 12).
    MephistoStatus,
    /// `0x005BD390`: the Dark Wanderer's minions (period 2).
    WandererMinions,
}

/// The point of an object's init record (`world/objects.md` §3 step 6:
/// room +0x08, x +0x14, y +0x18), the object's creation point.
/// `quests-act3-2.md` §11.7: "its position" in §5.6, §5.7, §9.1 and §9.2
/// is this point, and the room the decoy's boss and Hratli spawn in is
/// this room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitPoint {
    pub room: RoomId,
    pub x: i32,
    pub y: i32,
}

/// The source unit of `0x00559A30` (`quests-act3-2.md` §11.3), with the
/// value its item level is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropSource {
    /// No source unit.
    None,
    /// Unit type 1: its total stat 12 (`0x00625480`).
    Monster { level: i32 },
    /// Unit type 0: its base stat 12 (`0x006253B0`).
    Player { level: i32 },
    /// Any other type (objects): the area level of the unit's room's
    /// level (`0x0061DCA0`: `MonLvlEx` in an expansion game, else
    /// `MonLvl`, by difficulty).
    Other { area_level: i32 },
}

/// The `&level` output of `0x00559A30` (`quests-act3-2.md` §11.3), written
/// at `0x00559AF8` before any read and then the drop's item level: the
/// caller's incoming value is never used (so the Act III callers pass
/// `None` to [`QuestWorld::quest_drop`]). A result ≤ 1, or no source
/// unit, gives 1.
pub fn drop_item_level(source: DropSource) -> i32 {
    let level = match source {
        DropSource::None => return 1,
        DropSource::Monster { level } | DropSource::Player { level } => level,
        DropSource::Other { area_level } => area_level,
    };
    level.max(1)
}

/// Per-quest extra data of the Act III records (record +0x18). One
/// struct per record; each record uses its own field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra3 {
    pub q0: q0::Extra,
    pub q1: q1::Extra,
    pub q2: q2::Extra,
    pub q3: q3::Extra,
    pub q4: q4::Extra,
    pub q5: q5::Extra,
    pub q6: q6::Extra,
    pub q7: q7::Extra,
}

/// The init functions' stores beyond `quests.tsv` (§2). All Act III
/// records are active.
pub fn init(r: &mut QuestRecord) {
    match r.chain {
        14 | 17 | 19 | 20 => r.active = true,
        15 => {
            r.active = true;
            r.extra.act3.q1.reward_open = true;
        }
        16 => {
            r.active = true;
            r.status = 1;
        }
        18 => {
            r.active = true;
            r.extra.act3.q4.may_choose = true;
            r.extra.act3.q4.to_drop = true;
        }
        28 => {
            r.active = true;
            r.status = 0;
            r.extra.act3.q7.to_spawn = true;
        }
        _ => {}
    }
}

// ------------------------------------------------------------ helpers

/// The player's current-difficulty record (zero without one).
pub(super) fn pf<W: QuestWorld>(w: &mut W, p: UnitId) -> QuestFlags {
    flags_of(w, p).copied().unwrap_or_default()
}

/// Set bits of `slot` on the player's record.
pub(super) fn set<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, bits: &[u8]) {
    if let Some(f) = flags_of(w, p) {
        for &b in bits {
            f.set(slot, b);
        }
    }
}

/// Clear bits of `slot` on the player's record.
pub(super) fn clear<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, bits: &[u8]) {
    if let Some(f) = flags_of(w, p) {
        for &b in bits {
            f.clear(slot, b);
        }
    }
}

/// "in Act III": the player's room's level has act 2.
pub(super) fn in_act3<W: QuestWorld>(w: &W, p: UnitId) -> bool {
    w.unit_act(p) == Some(ACT)
}

/// The NPC class of the event's target.
pub(super) fn npc_of<W: QuestWorld>(w: &W, args: &EventArgs) -> Option<u16> {
    args.target.and_then(|n| w.monster_class(n))
}

/// The status iterate F of §2 for one player: chains 15, 17–20 send when
/// (s.0 and s.15 clear) or s.13 or s.14; chain 16 only when 18.0 and
/// 18.15 are clear.
pub(super) fn iterate_status<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    p: UnitId,
) {
    let (chain, slot) = (ctl.records[i].chain, ctl.records[i].filter);
    let f = pf(w, p);
    let open = !f.get(slot, bit::REWARD_GRANTED) && !f.get(slot, bit::COMPLETED_BEFORE);
    let send = if chain == 16 {
        open
    } else {
        open || f.get(slot, bit::PRIMARY_GOAL_DONE) || f.get(slot, bit::COMPLETED_NOW)
    };
    if send {
        if let Err(e) = ctl.send_status(w, p, chain) {
            ctl.faults.push(e);
        }
    }
}

/// "status n to all" (§1.1): flags := 0, status := n, F for every
/// player.
pub(super) fn status_all<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, n: u8) {
    ctl.records[i].flags = 0;
    ctl.records[i].status = n;
    for p in w.players() {
        iterate_status(ctl, w, i, p);
    }
}

/// "status n (silent)" (§1.1): the same store without the iterate.
pub(super) fn status_silent(ctl: &mut QuestControl, i: usize, n: u8) {
    ctl.records[i].flags = 0;
    ctl.records[i].status = n;
}

/// "add GUID" (§1.1): the record's player list.
pub(super) fn add_guid<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    let g = w.guid(p);
    ctl.records[i].guids.add(g);
}

/// "GUID listed" (`0x005452C0`).
pub(super) fn guid_listed<W: QuestWorld>(ctl: &QuestControl, w: &W, i: usize, p: UnitId) -> bool {
    ctl.records[i].guids.contains(w.guid(p))
}

/// "quick remove" `0x00545310`: the record list is not empty → remove
/// the player's GUID.
pub(super) fn quick_remove<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    if !ctl.records[i].guids.0.is_empty() {
        let g = w.guid(p);
        ctl.records[i].guids.remove(g);
    }
}

/// Event 10's list remove `0x00545530`.
pub(super) fn list_remove<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &W,
    i: usize,
    args: &EventArgs,
) {
    let g = args.player.map_or(u32::MAX, |p| w.guid(p));
    ctl.records[i].guids.remove(g);
}

/// "completion flag" (§1.1): every player lacking the bits of `slot`
/// gets s.14 and `5D <chain> 00 0C 0000` (`0x00545920`, act 0).
pub(super) fn completion_flag<W: QuestWorld>(
    w: &mut W,
    p: UnitId,
    slot: u8,
    chain: u8,
    lacking: &[u8],
) {
    let f = pf(w, p);
    if lacking.iter().any(|&b| f.get(slot, b)) {
        return;
    }
    set(w, p, slot, &[bit::COMPLETED_NOW]);
    let send = match w.unit_level(p) {
        None => true,
        Some(0) => false,
        Some(_) => true,
    };
    if send {
        w.send(p, &[0x5D, chain, 0, 12, 0, 0]);
    }
}

/// `0x005458E0`: `5D <chain> 02 00 0000` to the player.
pub(super) fn send_reward_ack<W: QuestWorld>(w: &mut W, p: UnitId, chain: u8) {
    w.send(p, &[0x5D, chain, 2, 0, 0, 0]);
}

/// Install callback `ev` (store a function pointer; the dispatch maps
/// (chain, event) to its function).
pub(super) fn install(ctl: &mut QuestControl, i: usize, ev: u8) {
    ctl.records[i].callbacks |= 1 << ev;
}

/// "refresh" (`quests.md` §7.2) for the event's NPC.
pub(super) fn refresh<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    p: UnitId,
    args: &EventArgs,
) {
    if let Some(n) = args.target {
        ctl.refresh_text(w, p, n);
    }
}

/// Add table state `k` of the record's message table for the NPC.
pub(super) fn add_state<W: QuestWorld>(
    ctl: &QuestControl,
    w: &W,
    i: usize,
    list: Option<&mut TextList>,
    target: Option<UnitId>,
    k: u8,
) {
    let class = target.and_then(|n| w.monster_class(n));
    if let (Some(t), Some(list), Some(c)) = (ctl.records[i].msgs, list, class) {
        ctl.add_messages(t, list, c, k);
    }
}

/// An index table's table state for `state`: `None` for −1, a value
/// above `max`, or a state past the table.
pub(super) fn table_state(table: &[i8], state: u8, max: i8) -> Option<u8> {
    match table.get(usize::from(state)) {
        Some(&m) if m >= 0 && m <= max => Some(m as u8),
        _ => None,
    }
}

/// "Lam Esen done" = game 17.13.
pub(super) fn lam_esen_done(ctl: &QuestControl) -> bool {
    ctl.game.get(17, bit::PRIMARY_GOAL_DONE)
}

/// "S(a, b)" (§7.1): `a` when Lam Esen is done, else `b`.
pub(super) fn s_ab(ctl: &QuestControl, a: u8, b: u8) -> u8 {
    if lam_esen_done(ctl) {
        a
    } else {
        b
    }
}

/// `0x005BBF80`: chain 19 absent or the Compelling Orb not smashed.
pub fn orb_intact(ctl: &QuestControl) -> bool {
    ctl.record(19).is_none_or(|r| !r.extra.act3.q5.orb_smashed)
}

// ------------------------------------------------------------ §1.3

/// "Call seq(c)" (§1.3): look up chain `c` and call its sequence
/// function. 0 (false) for an absent chain.
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return false;
    };
    let (state, not_intro) = (ctl.records[i].state, ctl.records[i].not_intro);
    match chain {
        18 => {
            if state != 5 && not_intro {
                return true;
            }
            let r1 = sequence(ctl, w, 16);
            let r2 = sequence(ctl, w, 17);
            r1 || r2
        }
        16 | 17 => {
            if state != 5 && not_intro {
                if state == 0 {
                    ctl.records[i].state = 1;
                }
                return true;
            }
            sequence(ctl, w, 15)
        }
        15 => {
            if state != 5 && not_intro {
                return true;
            }
            sequence(ctl, w, 19)
        }
        19 => {
            if state != 7 && not_intro {
                return true;
            }
            sequence(ctl, w, 20)
        }
        20 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
                install(ctl, i, event::NPC_DEACTIVATE);
            }
            true
        }
        _ => {
            let f = ctl.records[i].seq_fn.unwrap_or(0);
            w.unhandled(chain, f);
            false
        }
    }
}

// ------------------------------------------------------------ dispatch

/// Dispatches one callback of an Act III record; false = no body.
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match ctl.records[i].chain {
        14 => q0::callback(ctl, w, i, args, list),
        15 => q1::callback(ctl, w, i, args, list),
        16 => q2::callback(ctl, w, i, args, list),
        17 => q3::callback(ctl, w, i, args, list),
        18 => q4::callback(ctl, w, i, args, list),
        19 => q5::callback(ctl, w, i, args, list),
        20 => q6::callback(ctl, w, i, args, list),
        28 => q7::callback(ctl, w, i, args, list),
        39 => intro::callback(ctl, w, i, args, list),
        _ => false,
    }
}

/// Active functions (§6.4 of `quests.md`; "wants to talk").
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    match ctl.records[i].chain {
        14 => q0::active(w, player, npc_class),
        15 => q1::active(ctl, w, i, player, npc_class),
        16 => q2::active(ctl, w, i, player, npc_class),
        17 => q3::active(ctl, w, i, player, npc_class),
        18 => q4::active(ctl, w, i, player, npc_class),
        19 => q5::active(ctl, w, i, player, npc_class),
        20 => q6::active(ctl, w, i, player, npc_class),
        39 => intro::active(w, player, npc_class),
        // `0x005BD0C0`: false.
        _ => false,
    }
}

/// Status functions (§2): chains 15–18 always report; 14, 28, 39
/// return false.
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
) -> Option<u8> {
    match ctl.records[i].chain {
        15 => Some(q1::status(ctl, w, i, player, pf)),
        16 => Some(q2::status(ctl, w, i, player, pf)),
        17 => Some(q3::status(ctl, w, i, player, pf)),
        18 => Some(q4::status(ctl, w, i, player, pf)),
        _ => None,
    }
}

/// Runs an Act III timer; true = remove it. Each returns 1.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return true;
    };
    match t {
        Timer::GidbinnBoss => q3::boss_timer(ctl, w, i),
        Timer::MephistoStatus => q6::status_timer(ctl, w, i),
        Timer::WandererMinions => q7::minion_timer(ctl, w, i),
    }
}

/// `0x005436B0` links from monster creation (§10): the chain a new
/// monster of `base` class / `class` id links to, if any. Base 366 also
/// gets unit flags 0x20000 (done by the caller's switch).
pub fn monster_link(base: u16, class: u16) -> Option<u8> {
    match base {
        npc::FETISH11 => Some(17),
        npc::COMPELLING_ORB => Some(19),
        npc::MEPHISTO if class != 704 => Some(20),
        _ => None,
    }
}

/// Superunique creation links (§10): 26, 27, 29 → chain 19.
pub fn superunique_link(superunique: u32) -> Option<u8> {
    matches!(superunique, 26 | 27 | 29).then_some(19)
}
