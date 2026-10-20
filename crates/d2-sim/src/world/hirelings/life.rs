//! The hireling's life: init, following the player (type 7), the
//! classic act change and the owner's death (type 7), death, revive and
//! restoring from a save.
//!
//! The C→S 0x62 part of a resurrect (cost, flag, mode, life,
//! the 0x9B / 0x2A replies) is the NPC module's
//! ([`crate::world::npc::hire`]), which then calls [`revive`].

use super::pets::{self, broadcast_remove, merc_dead_message};
use super::{
    flags, level, resurrect_cost, stat, state, HirelingError, HirelingState, HirelingTables,
    HirelingWorld, Offer, Slot, NEW_HIRE, UNIT_PLAYER,
};
use crate::units::UnitId;

/// Unit mode: neutral (alive).
pub const MODE_NEUTRAL: u8 = 1;
/// Unit mode: dead (corpse).
pub const MODE_DEAD: u8 = 12;
/// `owner` type argument of the owner clear (old, −1, 1).
const CLEAR_OWNER_TYPE: u8 = 1;

/// An old hireling unit leaves: owner cleared
/// (GUID −1, type 1), pet node removed (no kill),
/// room removal queued and the unit freed with its items.
fn discard<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    old: UnitId,
) -> Result<(), HirelingError> {
    let guid = w.guid(old);
    w.set_owner(old, u32::MAX, CLEAR_OWNER_TYPE);
    pets::remove(w, st, player, guid, false)?;
    w.free_unit(old);
    Ok(())
}

/// The hireling init (game, player, merc, saved_id, slot, restore).
/// `saved_id`: the saved `Id`, or [`NEW_HIRE`]. `restore` (param_6) only
/// gates step 3: the save loader passes the saved dead flag
/// (`flags & 0x10000`), the other callers
/// 0, so a living merc restored from a save joins the team. Returns the offer of
/// step 5, `None` when nothing happened (player not type 0) or there was
/// no offer (step 5: the unit stays without owner or pet node).
#[allow(clippy::too_many_arguments)]
pub fn init<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
    saved_id: u16,
    slot: Slot,
    dead: bool,
) -> Result<Option<Offer>, HirelingError> {
    if w.unit_type(player) != UNIT_PLAYER {
        return Ok(None);
    }
    // Step 1.
    let f = w.flags(merc);
    w.set_flags(merc, f | flags::INIT);
    // Step 2: stat 172 := 2 in the stat list of state 105.
    w.set_state_stat(merc, state::ALIGNMENT, stat::ALIGNMENT, 2);
    // Step 3.
    if !dead {
        w.join_team(merc, player);
    }
    // Step 4: replace the first hireling node (living or dead) whose
    // unit exists.
    if let Some(old) = pets::any(w, st, player) {
        w.send(player, &merc_dead_message(0xFFFF, 0));
        discard(w, st, player, old)?;
    }
    // Step 5.
    let act0 = t.rows.act_of_name(w.expansion(), slot.name);
    let player_level = w.stat(player, stat::LEVEL);
    let Some(offer) = t.rows.offer(
        w.expansion(),
        player_level,
        slot.seed,
        act0,
        u32::from(w.difficulty()),
    ) else {
        return Ok(None);
    };
    // Step 6. The add is void in the base game (no test after the
    // call): a `false` (no node added) does not stop the init.
    pets::add(w, t, st, player, merc, slot.seed, slot.name, offer.id)?;
    // Step 7.
    let f = w.flags(merc);
    w.set_flags(merc, f | flags::OWNED);
    // Step 8.
    let pg = w.guid(player);
    w.set_owner(merc, pg, UNIT_PLAYER);
    // Step 9.
    if saved_id == NEW_HIRE {
        if t.rows
            .row_at(w.expansion(), offer.id, offer.level)
            .is_none()
        {
            return Ok(Some(offer));
        }
        w.set_base_stat(merc, stat::EXPERIENCE, offer.experience);
        level::apply_level(w, t, st, player, Some(merc), offer.level);
        level::send_stats(w, st, player)?;
    }
    // Rules 10, 11.
    w.hireling_ai(merc);
    Ok(Some(offer))
}

/// The hireling part of the kill with (game, unit,
/// killer, flag): the kill's own guards (monster mode not 0 / 12, the
/// monstats `killable` test) are the caller's; then with
/// `flag` ≠ 0 (1 at every caller except the expired-pet kill) and a
/// player owner (type 0), the death ([`death`]). Returns whether a node
/// was marked dead.
pub fn on_kill<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    merc: UnitId,
    flag: bool,
) -> bool {
    if !flag {
        return false;
    }
    let Some((guid, UNIT_PLAYER)) = w.owner(merc) else {
        return false;
    };
    let Some(owner) = w.player_by_guid(guid) else {
        return false;
    };
    death(w, st, owner, merc)
}

/// The hireling's death (game, owner, merc), hireling list part: the
/// node of the merc's GUID gets bit 0 (dead), the owner's client gets
/// 0x9B (name id, resurrect cost at the current level) and every client
/// 0x7A remove with only the GUID. `false`: the GUID is in no hireling
/// node.
///
/// The pet type lookup and the type ≠ 7 node removal belong to
/// `d2-sim::player::pets`; the trigger is [`on_kill`].
pub fn death<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    owner: UnitId,
    merc: UnitId,
) -> bool {
    let guid = w.guid(merc);
    let Some(node) = st
        .lists
        .get_mut(&owner)
        .and_then(|l| l.nodes.iter_mut().find(|n| n.guid == guid))
    else {
        return false;
    };
    node.dead = true;
    let name = node.name;
    let cost = resurrect_cost(w.stat(merc, stat::LEVEL));
    w.send(owner, &merc_dead_message(name, cost));
    broadcast_remove(w, guid);
    true
}

/// The revive (game, player, merc).
pub fn revive<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
) -> Result<(), HirelingError> {
    // Step 3. Edge case 5: when the living hireling is `merc` itself the
    // base game frees it here; the NPC resurrect handler refuses that
    // case (code 9).
    if let Some(old) = pets::living(w, st, player) {
        discard(w, st, player, old)?;
    }
    // Step 4.
    w.set_mode(merc, MODE_NEUTRAL);
    let max = w.max_life(merc);
    w.set_base_stat(merc, stat::HITPOINTS, max);
    let guid = w.guid(merc);
    pets::mark_living(w, st, player, guid);
    w.join_team(merc, player);
    // Step 5.
    for s in [state::FREEZE, state::SHATTER, state::RECYCLED] {
        w.remove_state(merc, s);
    }
    // Step 6.
    let f = w.flags(merc);
    w.set_flags(merc, f | flags::REVIVE);
    let f2 = w.flags2(merc);
    w.set_flags2(merc, f2 & !flags::REVIVE_CLEAR2);
    // Flags 2 bit 0x100 is kept; no base-game reader reaches a
    // type-7 node.
    // Step 7.
    w.reapply_item_stats(merc);
    // Step 8.
    follow(w, t, st, player);
    // Step 9.
    level::send_stats(w, st, player)
}

/// The pet follow (game, player, x, y), pet type 7: `warp` →
/// every living node's unit moves to the player (flags 2 |=
/// 0x10000); else `range` → every living node whose unit exists and is
/// farther than squared distance 1600 from the player unit is removed
/// with kill; else every node is freed
/// with its unit.
///
/// TODO: pet types 1 … count − 1 other than 7 are
/// `d2-sim::player::pets`'s; the caller runs them in row order.
pub fn follow<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
) {
    let living: Vec<u32> = st
        .list(player)
        .map(|l| l.nodes.iter().filter(|n| !n.dead).map(|n| n.guid).collect())
        .unwrap_or_default();
    if t.pet_flags & HirelingTables::WARP != 0 {
        for guid in living {
            if let Some(unit) = w.monster_by_guid(guid) {
                w.warp_to(unit, player);
                let f2 = w.flags2(unit);
                w.set_flags2(unit, f2 | flags::WARP2);
            }
        }
    } else if t.pet_flags & HirelingTables::RANGE != 0 {
        // The range check (limit 1600) uses the player unit's position,
        // not the follow's (x, y).
        let (px, py) = w.position(player);
        for guid in living {
            let Some(unit) = w.monster_by_guid(guid) else {
                continue;
            };
            if range_distance(w.position(unit), (px, py)) > RANGE_LIMIT {
                // The pet remove with kill. The
                // node is in the list (taken from it above), so the
                // remove cannot fail on a missing node.
                let _ = pets::remove(w, st, player, guid, true);
            }
        }
    } else {
        // Every node of the type freed with its unit.
        // TODO: the pet list's own unit free is not written;
        // [`HirelingWorld::free_unit`] stands for it. Unused by the
        // base game's row 7 (warp 1).
        let guids: Vec<u32> = st
            .lists
            .get_mut(&player)
            .map(|l| l.nodes.drain(..).map(|n| n.guid).collect())
            .unwrap_or_default();
        for guid in guids {
            if let Some(unit) = w.monster_by_guid(guid) {
                w.free_unit(unit);
            }
        }
    }
}

/// The range pets' limit (0x640).
pub const RANGE_LIMIT: i32 = 1600;

/// Dx² + dy² (32-bit signed, wrapping), compared signed
/// against [`RANGE_LIMIT`].
pub fn range_distance(pet: (i32, i32), player: (i32, i32)) -> i32 {
    let dx = pet.0.wrapping_sub(player.0);
    let dy = pet.1.wrapping_sub(player.1);
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

/// The join follow (game, player), run from the join placement: the pet
/// follow to the player's position when the player has a living
/// hireling (`(7, 0)`); else nothing. A dead restored hireling stays
/// roomless.
///
/// Caller: the join placement
/// (`wiring::path::place::game_entry`) queues the call
/// (`wiring::action::HirelingCall::JoinFollow`) for the host holding the
/// hireling lists (`d2-server` `WiredWorld::hireling_calls`).
pub fn join_follow<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
) {
    if pets::living(w, st, player).is_some() {
        follow(w, t, st, player);
    }
}

/// The owner-death pet pass (game, player), t = 7 (with keep =
/// keep_dead = 1):
/// for each node whose unit exists, in list order: broadcast 0x7A remove
/// with only the GUID, post the room removal notice {1, GUID} (the unit
/// is not unlinked from its room nor freed); a living node: bit 0 := 1,
/// list head := this node (earlier nodes leave the list, not freed), the
/// death mode request when the unit is in a room, and 0x9B (name id,
/// resurrect cost) to the player's client. A node whose unit does not
/// exist is left as it is. Count and max are unchanged.
///
/// TODO: t ≠ 7 (every pet unit killed, its node freed) and
/// the max recompute after the walk are
/// `d2-sim::player::pets`'s.
pub fn kill_with_owner<W: HirelingWorld>(w: &mut W, st: &mut HirelingState, player: UnitId) {
    let mut i = 0;
    while i < st.list(player).map_or(0, |l| l.nodes.len()) {
        let node = st.lists[&player].nodes[i];
        let Some(unit) = w.monster_by_guid(node.guid) else {
            i += 1;
            continue;
        };
        broadcast_remove(w, node.guid);
        w.queue_room_removal(unit);
        if !node.dead {
            let list = st.list_mut(player);
            list.nodes[i].dead = true;
            // "list head := node". With type 7 the list holds
            // one node, so nothing is dropped with live data;
            // in the base game the dropped nodes stay counted, which a list held
            // as its nodes cannot show.
            list.nodes.drain(..i);
            i = 0;
            if w.in_room(unit) {
                w.death_event(unit);
            }
            let cost = resurrect_cost(w.stat(unit, stat::LEVEL));
            w.send(player, &merc_dead_message(node.name, cost));
        }
        i += 1;
    }
}

/// The classic act change (classic game
/// only, before the player is placed in the new act) →
/// [`kill_with_owner`]. The caller then runs the pet follow ([`follow`])
/// to the new position (in both game types).
///
/// Caller: the path wiring's `level_warp` meets
/// the act change (another act: `None`, the warp stays on its
/// `Pending::warp` route, owned by the act / level change) and queues
/// it (`wiring::action::HirelingCall::ActChange`); the host holding the
/// hireling lists runs this, then [`follow`] (`d2-server`
/// `WiredWorld::hireling_calls`).
pub fn classic_act_change<W: HirelingWorld>(w: &mut W, st: &mut HirelingState, player: UnitId) {
    kill_with_owner(w, st, player);
}

/// The player mode-17 start
/// runs the owner-death pet pass, after the corpse
/// creation, **in every game type**: the hireling dies with its owner
/// ([`kill_with_owner`]). Its messages follow the corpse's (step 5).
pub fn player_death<W: HirelingWorld>(w: &mut W, st: &mut HirelingState, player: UnitId) {
    kill_with_owner(w, st, player);
}

/// The saved hireling fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SavedHireling {
    /// Dead bit 0x10000 of the saved flags.
    pub dead: bool,
    pub seed: u32,
    /// Name index (added to the row's `NameFirst`); the name
    /// word of a version-0x47 save ([`Loader::OldV47`]).
    pub name_index: u16,
    /// `hireling` `Id`.
    pub id: u16,
    pub experience: u32,
}

/// The base-game loader that restores the hireling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Loader {
    /// Save version ≥ 0x5C.
    #[default]
    Current,
    /// Older saves, main path.
    Old,
    /// The older loader's version-0x47 path (header word `JM`
    /// 0x4D4A): `Id` 0xFFFF, class 0, dead 0.
    OldV47,
}

impl Loader {
    /// The old loader (both paths).
    pub fn is_old(self) -> bool {
        matches!(self, Loader::Old | Loader::OldV47)
    }
}

/// Why a save restores no hireling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreSkip {
    /// Step 1: seed, name and experience are all 0 (version-0x47 path:
    /// seed or name word 0).
    NoHireling,
    /// Step 2, ≥ 0x5C loader: no row (`Id`, level 1); the hireling is
    /// skipped and the load goes on (no error).
    NoRow,
    /// Step 2, old loader: no row (`Id`, level 1): the load fails with
    /// "Unable to load merc." (error 0xE).
    LoadFailed,
    /// Step 3: classic game and the name's act is not the player's act.
    OtherAct,
    /// A fatal error of the base game by string id: 0x744 (`Id` 0xFFFF and no
    /// candidate row).
    Fatal(u16),
}

/// The old loader's load error "Unable to load merc.".
pub const LOAD_ERROR_MERC: u32 = 0xE;
/// Fatal string of an `Id` 0xFFFF restore without a row.
pub const FATAL_NO_ROW: u16 = 0x744;
/// Fatal string of a null row in the old loader's level loop.
pub use super::level::FATAL_LEVEL_ROW;

/// What [`restore_plan`] decides for the restore (game, player, name,
/// seed, Id, class, dead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestorePlan {
    /// The name id (clamped).
    pub name: u16,
    /// Mode of the unit: 12 dead, else 1.
    pub mode: u8,
    /// The class argument the unit is allocated with: the
    /// `Class` of row (`Id`, 1), or 0 on the version-0x47 path.
    pub class: u32,
    /// The `Id` argument: the saved `Id`, or 0xFFFF on the version-0x47
    /// path.
    pub id: u16,
    /// `Seller` (+0x14) of the row looks up: the NPC record
    /// whose hire list gets the slot marked.
    pub seller: u32,
    pub loader: Loader,
}

/// The restore from a save up to the unit creation. `player_act`: the player's act (client act when the
/// player has a client, else unit +0x18), read only in a classic game;
/// `difficulty`: game +0x6D (read only for `Id` 0xFFFF).
///
/// The caller then marks the hire slot of `name` hired in the hire list
/// of the NPC record of `seller` **only when it is not currently
/// offered** (step 3, edge case 8; the hire list is the NPC module's),
/// allocates the unit of class `class` with `mode` **with no room** at
/// point (0, 0) (unit type 1, room 0, flag 1) and calls [`restore`].
pub fn restore_plan(
    t: &HirelingTables,
    expansion: bool,
    difficulty: u8,
    saved: &SavedHireling,
    player_act: u32,
    loader: Loader,
) -> Result<RestorePlan, RestoreSkip> {
    let (name, id, class, dead, row) = if loader == Loader::OldV47 {
        // Only when seed ≠ 0 and the name word ≠ 0; Id
        // 0xFFFF, class 0, dead 0. The name word goes in as it is.
        if saved.seed == 0 || saved.name_index == 0 {
            return Err(RestoreSkip::NoHireling);
        }
        // Row := the first candidate of (expansion, act of
        // the name, difficulty) (after 0); none → fatal.
        let act0 = t.rows.act_of_name(expansion, saved.name_index);
        let row = *t
            .rows
            .candidates(expansion, act0, u32::from(difficulty))
            .first()
            .ok_or(RestoreSkip::Fatal(FATAL_NO_ROW))?;
        (saved.name_index, NEW_HIRE, 0, false, row)
    } else {
        // Step 1.
        if saved.seed == 0 && saved.name_index == 0 && saved.experience == 0 {
            return Err(RestoreSkip::NoHireling);
        }
        // Step 2: the first bracket; none → skipped (≥ 0x5C) or the load
        // fails (old). Name id := NameFirst + index, above
        // NameLast → NameFirst.
        let row = t
            .rows
            .row_at(expansion, u32::from(saved.id), 1)
            .ok_or(if loader.is_old() {
                RestoreSkip::LoadFailed
            } else {
                RestoreSkip::NoRow
            })?;
        let r = &t.rows.rows[row];
        let name = u32::from(r.name_first) + u32::from(saved.name_index);
        let name = if name > u32::from(r.name_last) {
            r.name_first
        } else {
            name as u16
        };
        (name, saved.id, r.class, saved.dead, row)
    };
    // Step 3: classic → only in the name's act. On the main paths the
    // row of step 3 is the same (Id, 1) row, so its name clamp leaves
    // `name` unchanged; on the 0x47 path the row is the first candidate
    // of the name's act, whose range holds the name when a row matched
    // it (one range per act).
    if !expansion && t.rows.act_of_name(expansion, name) != player_act {
        return Err(RestoreSkip::OtherAct);
    }
    // TODO: the form of the restore's name
    // clamp for a name outside every row's range (0x47 path only) is not
    // written; the name is kept.
    let mode = if dead { MODE_DEAD } else { MODE_NEUTRAL };
    Ok(RestorePlan {
        name,
        mode,
        class,
        id,
        seller: t.rows.rows[row].seller,
        loader,
    })
}

/// The restore's init step: the init (with `saved_id` = `Id`,
/// restore = the saved dead flag), then the pet node's seed / name / `Id` := the saved
/// values (right after the unit exists and before the
/// experience step). Returns whether the
/// node exists (no offer → no node).
///
/// `Id` 0xFFFF (the version-0x47 path): the init takes the
/// new-hire branch (the node gets the offer's row `Id`, the unit the
/// offer's experience and level, from the saved seed and name) and the
/// path writes no node values of its own (a base-game bug, kept on
/// purpose).
///
/// The caller then runs the restore's experience and level
/// ([`super::level::restore_experience`], not on the 0x47 path) and
/// [`restore_tail`].
///
/// Caller: the save load
/// (`d2-server` `adapters::character`) queues the restore
/// (`wiring::action::HirelingCall::Restore`); the wired host
/// (`WiredWorld::restore_hireling`) allocates the roomless unit and runs
/// this with [`restore_plan`], the experience step and [`restore_tail`].
#[allow(clippy::too_many_arguments)]
pub fn restore<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
    plan: &RestorePlan,
    saved: &SavedHireling,
) -> Result<bool, HirelingError> {
    let slot = Slot {
        name: plan.name,
        seed: saved.seed,
    };
    init(w, t, st, player, merc, plan.id, slot, saved.dead)?;
    let guid = w.guid(merc);
    let Some(node) = st
        .lists
        .get_mut(&player)
        .and_then(|l| l.nodes.iter_mut().find(|n| n.guid == guid))
    else {
        return Ok(false);
    };
    if plan.loader != Loader::OldV47 {
        node.seed = saved.seed;
        node.name = plan.name;
        node.id = u32::from(saved.id);
    }
    Ok(true)
}

/// With (game, merc): cancel the
/// merc's pending timer events of type 2 (AI think), then type 3 (stat
/// regeneration), any argument.
pub fn cancel_ai_and_regen<W: HirelingWorld>(w: &mut W, merc: UnitId) {
    w.cancel_timers(merc, TIMER_AI, 0);
    w.cancel_timers(merc, TIMER_REGEN, 0);
}

/// Timer event type 2: AI think.
pub const TIMER_AI: u8 = 2;
/// Timer event type 3: stat regeneration.
pub const TIMER_REGEN: u8 = 3;

/// The restore tail's dead part: the death (node dead, 0x9B to the
/// owner, 0x7A remove), flags |= 0x10000, mode 12, then the AI and
/// regeneration timers cancelled ([`cancel_ai_and_regen`]).
pub fn restore_dead<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
) {
    death(w, st, player, merc);
    let f = w.flags(merc);
    w.set_flags(merc, f | flags::DEAD);
    w.set_mode(merc, MODE_DEAD);
    cancel_ai_and_regen(w, merc);
}

/// The restore tail in each loader's order: ≥
/// 0x5C loader: the dead steps ([`restore_dead`], when `dead`), then "no
/// inventory → create one"; old loader: the inventory step first, then
/// the dead steps.
pub fn restore_tail<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
    dead: bool,
    loader: Loader,
) {
    if loader.is_old() {
        w.ensure_inventory(merc);
    }
    if dead {
        restore_dead(w, st, player, merc);
    }
    if !loader.is_old() {
        w.ensure_inventory(merc);
    }
}

/// After the
/// hireling's items, life := max (also when dead), then the stats are sent
/// ([`super::level::send_stats`]; a dead hireling has no living node, so nothing is
/// queued).
pub fn restore_stats<W: HirelingWorld>(
    w: &mut W,
    st: &HirelingState,
    player: UnitId,
    merc: UnitId,
) {
    let max = w.max_life(merc);
    w.set_base_stat(merc, stat::HITPOINTS, max);
    let _ = level::send_stats(w, st, player);
}
