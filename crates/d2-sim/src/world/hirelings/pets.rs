//! The hireling pet list (pet type 7) and its messages: add, find,
//! remove and unlink, dismiss, mark living, join sync and the encoders
//! of S→C 0x81, 0x7A and 0x9B.
//!
//! Only the hireling list lives here; the other pet types (and the
//! lookup over every type) live in `d2-sim::player::pets`.

use super::{flags, HirelingError, HirelingState, HirelingTables, HirelingWorld, PetNode};
use super::{PET_HIRELING, UNIT_PLAYER};
use crate::units::UnitId;

/// S→C 0x81 AssignMerc.
pub const MSG_ASSIGN_MERC: u8 = 0x81;
/// S→C 0x7A PetAction.
pub const MSG_PET_ACTION: u8 = 0x7A;
/// S→C 0x9B.
pub const MSG_MERC_DEAD: u8 = 0x9B;
/// 0x7A action: remove.
pub const ACTION_REMOVE: u8 = 0;
/// 0x7A action: add.
pub const ACTION_ADD: u8 = 1;

/// S→C 0x81 AssignMerc (20 bytes, zeroed first): u8 pet type @1
/// (7), u16 monstats class @2, u32 owner GUID @4, u32 hireling GUID @8,
/// u32 seed @12, u32 name id @16.
pub fn assign_merc(class: u16, owner: u32, merc: u32, seed: u32, name: u32) -> [u8; 20] {
    let mut b = [0u8; 20];
    b[0] = MSG_ASSIGN_MERC;
    b[1] = PET_HIRELING;
    b[2..4].copy_from_slice(&class.to_le_bytes());
    b[4..8].copy_from_slice(&owner.to_le_bytes());
    b[8..12].copy_from_slice(&merc.to_le_bytes());
    b[12..16].copy_from_slice(&seed.to_le_bytes());
    b[16..20].copy_from_slice(&name.to_le_bytes());
    b
}

/// S→C 0x7A (13 bytes, zeroed first): u8 action @1, u8 pet
/// type @2, u16 class @3, u32 owner GUID @5, u32 pet GUID @9 (layout
/// corrected 2026-10-07; a measured remove:
/// `7a 00 00 0000 00000000 0d000000`).
pub fn pet_action(action: u8, pet_type: u8, class: u16, pet: u32, owner: u32) -> [u8; 13] {
    let mut b = [0u8; 13];
    b[0] = MSG_PET_ACTION;
    b[1] = action;
    b[2] = pet_type;
    b[3..5].copy_from_slice(&class.to_le_bytes());
    b[5..9].copy_from_slice(&owner.to_le_bytes());
    b[9..13].copy_from_slice(&pet.to_le_bytes());
    b
}

/// S→C 0x9B (7 bytes): u16 @1, u32 @3. Death / classic
/// act change: name id and resurrect cost; replace, resurrect: 0xFFFF, 0.
pub fn merc_dead_message(name: u16, cost: u32) -> [u8; 7] {
    let mut b = [0u8; 7];
    b[0] = MSG_MERC_DEAD;
    b[1..3].copy_from_slice(&name.to_le_bytes());
    b[3..7].copy_from_slice(&cost.to_le_bytes());
    b
}

/// The message to every player's client, in broadcast
/// order.
pub fn broadcast<W: HirelingWorld>(w: &mut W, bytes: &[u8]) {
    for p in w.players() {
        w.send(p, bytes);
    }
}

/// The "remove" broadcast: 0x7A action 0 with only the
/// GUID set.
pub fn broadcast_remove<W: HirelingWorld>(w: &mut W, guid: u32) {
    broadcast(w, &pet_action(ACTION_REMOVE, 0, 0, guid, 0));
}

/// The "add" broadcast: record {pet GUID,
/// owner GUID, class (0xFFFF without a unit), type 7, seed, name}; seed
/// or name set → 0x81, else 0x7A action 1.
fn broadcast_add<W: HirelingWorld>(w: &mut W, owner: UnitId, unit: Option<UnitId>, node: &PetNode) {
    let class = unit.map_or(0xFFFF, |u| w.class(u) as u16);
    let owner_guid = w.guid(owner);
    let bytes: Vec<u8> = if node.seed != 0 || node.name != 0 {
        assign_merc(
            class,
            owner_guid,
            node.guid,
            node.seed,
            u32::from(node.name),
        )
        .to_vec()
    } else {
        pet_action(ACTION_ADD, PET_HIRELING, class, node.guid, owner_guid).to_vec()
    };
    broadcast(w, &bytes);
}

/// Dismiss: the monster of `guid`; none →
/// nothing. Flags |= NOXP, the kill or death request (seam), then the
/// "remove" broadcast.
pub fn dismiss<W: HirelingWorld>(w: &mut W, guid: u32) {
    let Some(unit) = w.monster_by_guid(guid) else {
        return;
    };
    let f = w.flags(unit);
    w.set_flags(unit, f | flags::NOXP);
    w.dismiss(unit);
    broadcast_remove(w, guid);
}

/// Append a node
/// {seed, name, `Id`} for `merc` to the player's hireling list and
/// broadcast it. `false`: the maximum is 0 even after the recompute, the
/// unit was dismissed and nothing was added.
#[allow(clippy::too_many_arguments)]
pub fn add<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &mut HirelingState,
    player: UnitId,
    merc: UnitId,
    seed: u32,
    name: u16,
    id: u32,
) -> Result<bool, HirelingError> {
    let guid = w.guid(merc);
    if st.list_mut(player).max == 0 {
        let max = recompute_max(w, t, player);
        st.list_mut(player).max = max;
        if max == 0 {
            dismiss(w, guid);
            return Ok(false);
        }
    }
    let list = st.list_mut(player);
    if list.nodes.len() as i32 >= list.max {
        // The full-list eviction: the unlink of the head GUID with kill,
        // directly (one 0x7A remove from the unlink, then the kill
        // with its own messages); not the pet remove.
        if let Some(head) = list.nodes.first().map(|n| n.guid) {
            unlink(w, st, player, head, true)?;
        }
    }
    let node = PetNode {
        dead: false,
        guid,
        seed,
        name,
        id,
    };
    st.list_mut(player).nodes.push(node);
    broadcast_add(w, player, Some(merc), &node);
    Ok(true)
}

/// The first pet of type 7: the first node with
/// `any` or bit 0 clear, mapped to its unit by GUID (may be `None` when
/// the node's unit no longer exists).
pub fn find<W: HirelingWorld>(
    w: &W,
    st: &HirelingState,
    player: UnitId,
    any: bool,
) -> Option<UnitId> {
    let node = st.first_node(player, any)?;
    w.monster_by_guid(node.guid)
}

/// The max recompute, type 7 part: the largest
/// `petmax` (each below 1 counted as 1) over the player's skills whose
/// `pettype` is 7; none → `pettype` `basemax`. The trim evicts only
/// for types other than 7. (The base game's `skills.txt` has no skill of
/// `pettype` `hireable`, so this is `basemax` 1 there.)
pub fn recompute_max<W: HirelingWorld>(w: &W, t: &HirelingTables, player: UnitId) -> i32 {
    if w.unit_type(player) != UNIT_PLAYER {
        return 0;
    }
    match w.skill_pet_max(player, PET_HIRELING) {
        Some(v) => v.max(1),
        None => t.pet_basemax,
    }
}

/// "Living hireling" `(7, 0)`.
pub fn living<W: HirelingWorld>(w: &W, st: &HirelingState, player: UnitId) -> Option<UnitId> {
    find(w, st, player, false)
}

/// "Any hireling" `(7, 1)`.
pub fn any<W: HirelingWorld>(w: &W, st: &HirelingState, player: UnitId) -> Option<UnitId> {
    find(w, st, player, true)
}

/// Unlink: the first node of `guid` (none →
/// nothing); unlink it, broadcast "remove", then the unit of `guid`:
/// kill → dismiss, else its flags lose 0x80000000. Count
/// −= 1. Returns whether a node was unlinked.
pub fn unlink<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    guid: u32,
    kill: bool,
) -> Result<bool, HirelingError> {
    let Some(list) = st.lists.get_mut(&player) else {
        return Ok(false);
    };
    let Some(i) = list.nodes.iter().position(|n| n.guid == guid) else {
        return Ok(false);
    };
    list.nodes.remove(i);
    broadcast_remove(w, guid);
    if let Some(unit) = w.monster_by_guid(guid) {
        if kill {
            dismiss(w, guid);
        } else {
            let f = w.flags(unit);
            w.set_flags(unit, f & !flags::OWNED);
        }
    }
    // The fatal assertion on a negative count cannot fire
    // here: the count is the node vector's length and a node was just
    // removed from it.
    Ok(true)
}

/// The pet remove (game, player, GUID, kill),
/// hireling list part: unlink `guid` (which broadcasts once), then the
/// remove's own broadcast: two 0x7A removes
/// (edge case 1).
///
/// A GUID in no hireling node: the type lookup (over every
/// pet type) is `d2-sim::player::pets`'s; here it is treated as type 0
/// (kill → dismiss). TODO: route through the generic
/// lookup once that module exists.
pub fn remove<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    guid: u32,
    kill: bool,
) -> Result<(), HirelingError> {
    let in_list = st.node_by_guid(player, guid).is_some();
    if in_list {
        unlink(w, st, player, guid, kill)?;
    } else if kill {
        dismiss(w, guid);
    }
    broadcast_remove(w, guid);
    Ok(())
}

/// The node of `guid` becomes living (bit 0 :=
/// 0) and 0x81 is broadcast for it. `false`: no node.
pub fn mark_living<W: HirelingWorld>(
    w: &mut W,
    st: &mut HirelingState,
    player: UnitId,
    guid: u32,
) -> bool {
    let Some(node) = st
        .lists
        .get_mut(&player)
        .and_then(|l| l.nodes.iter_mut().find(|n| n.guid == guid))
    else {
        return false;
    };
    node.dead = false;
    let node = *node;
    let class = w
        .monster_by_guid(guid)
        .map_or(0xFFFF, |u| w.class(u) as u16);
    let owner = w.guid(player);
    broadcast(
        w,
        &assign_merc(class, owner, node.guid, node.seed, u32::from(node.name)),
    );
    true
}

/// The join sync, type 7 part: 0x81 for every living node of
/// `player`'s list that has a unit, to the joining client `to`.
///
/// TODO: the other pet types (0x7A action 1) are
/// `d2-sim::player::pets`'s; the caller runs both, types in row order.
pub fn join_sync<W: HirelingWorld>(w: &mut W, st: &HirelingState, player: UnitId, to: UnitId) {
    if w.unit_type(player) != UNIT_PLAYER {
        return;
    }
    let Some(list) = st.list(player) else {
        return;
    };
    let owner = w.guid(player);
    for node in list.nodes.iter().filter(|n| !n.dead) {
        let Some(unit) = w.monster_by_guid(node.guid) else {
            continue;
        };
        let class = w.class(unit) as u16;
        let bytes = assign_merc(class, owner, node.guid, node.seed, u32::from(node.name));
        w.send(to, &bytes);
    }
}
