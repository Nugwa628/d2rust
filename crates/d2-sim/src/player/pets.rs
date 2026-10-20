//! Player pet lists: one list per `pettype.txt` row, add with group
//! eviction and trimming, remove / unlink, dismiss, the
//! 0x7A / 0x81 broadcast and the GUID lookup.
//!
//! Everything outside the lists (units, monstats, kill, mode requests,
//! messages, the resync) is reached through [`PetWorld`]. The
//! lists themselves are owned by the world (player data +0x44) and borrowed
//! through [`PetWorld::pet_lists`] again after every world call, the way the
//! base game re-reads its entry pointer's fields.
//!
//! Fatal assertions of the base game are returned as [`PetError`] instead
//! of aborting. The state may already be changed when one is returned.

use std::fmt::Debug;

#[cfg(test)]
#[path = "pets_tests.rs"]
mod tests;

/// Pet type 1 `single`: its maximum is only ever 1.
pub const PETTYPE_SINGLE: i32 = 1;
/// Pet type 7 `hireable`: never trimmed by [`set_max`].
pub const PETTYPE_HIREABLE: i32 = 7;
/// Unit flag (+0xC4) cleared when a pet is unlinked without a kill.
pub const FLAG_PET: u32 = 0x8000_0000;
/// Unit flag (+0xC4) set on a dismissed pet: no experience.
pub const FLAG_NO_EXPERIENCE: u32 = 0x0400_0000;
/// The player state that skips the broadcast.
pub const STATE_NO_BROADCAST: u16 = 7;
/// Message id of S→C PetAction; the one encoder is
/// `world::hirelings::pets::pet_action`.
pub const MSG_PET_ACTION: u8 = crate::world::hirelings::pets::MSG_PET_ACTION;
/// Message id of S→C AssignMerc.
pub const MSG_ASSIGN_MERC: u8 = 0x81;
/// Node flags bit 0: skipped by [`first_pet`] with `any` = false.
pub const NODE_SKIP: u32 = 1;

/// Fatal assertions of the base game, returned instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PetError {
    /// The player has no player data.
    #[error("player data missing")]
    NoPlayerData,
    /// Any other fatal assertion; the text names the step.
    #[error("pet list assertion: {0}")]
    Fatal(&'static str),
}

/// One pet node (0x18 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetNode {
    /// +0x00; bit 0 ([`NODE_SKIP`]) is skipped by [`first_pet`] with `any` = false.
    pub flags: u32,
    /// +0x04 pet GUID (−1 for none).
    pub guid: i32,
    /// +0x08, +0x0C, +0x10 caller values (0 from [`add`]; the hireling
    /// caller's values are not settled).
    pub extra: [i32; 3],
}

/// One 12-byte list entry per pet type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PetEntry {
    /// The list: index 0 is the head (oldest), the last is the tail (newest).
    pub nodes: Vec<PetNode>,
    /// +0x04 count. Kept as its own field like the base game, not `nodes.len()`.
    pub count: i32,
    /// +0x08 max.
    pub max: i32,
}

impl PetEntry {
    /// The head's GUID, −1 without a head.
    pub fn head_guid(&self) -> i32 {
        head_guid(self)
    }
}

/// The pet lists P (player data +0x44): one entry per `pettype`
/// row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PetLists {
    pub entries: Vec<PetEntry>,
}

impl PetLists {
    /// `pettype_count` empty entries with count 0 and max 0.
    ///
    /// Who sets the initial max (from `basemax`) is not settled.
    pub fn new(pettype_count: usize) -> Self {
        Self {
            entries: vec![PetEntry::default(); pettype_count],
        }
    }

    /// One empty entry per value of `max`, with that max. The caller (player
    /// init) decides the values.
    pub fn with_max(max: &[i32]) -> Self {
        Self {
            entries: max
                .iter()
                .map(|&max| PetEntry {
                    nodes: Vec::new(),
                    count: 0,
                    max,
                })
                .collect(),
        }
    }
}

/// The add record handed to the broadcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddRecord {
    /// Pet GUID (−1 none).
    pub pet_guid: i32,
    /// The player's GUID.
    pub owner_guid: i32,
    /// Pet class (0xFFFF none).
    pub class: u16,
    /// Pet type t.
    pub pet_type: i32,
    /// Record +0x10: both it and `x14` 0 → 0x7A, else 0x81.
    pub x10: i32,
    /// Record +0x14.
    pub x14: i32,
}

/// A message the broadcast sends to one player's client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMsg {
    /// S→C 0x7A PetAction (13 bytes).
    PetAction {
        /// 1 add, 0 remove.
        action: u8,
        pet_type: u8,
        class: u16,
        owner: u32,
        pet: u32,
    },
    /// S→C 0x81 AssignMerc; its layout belongs to the
    /// hireling module, so the add record is carried as is.
    AssignMerc { record: AddRecord },
}

impl PetMsg {
    /// The 0x7A wire bytes; `None` for 0x81, whose layout the hireling
    /// module owns.
    pub fn bytes(&self) -> Option<[u8; 13]> {
        match *self {
            PetMsg::PetAction {
                action,
                pet_type,
                class,
                owner,
                pet,
            } => Some(crate::world::hirelings::pets::pet_action(
                action, pet_type, class, pet, owner,
            )),
            PetMsg::AssignMerc { .. } => None,
        }
    }
}

/// The world outside the pet lists.
pub trait PetWorld {
    type Unit: Copy + Eq + Debug;

    /// Unit type 0 (player).
    fn is_player(&self, u: Self::Unit) -> bool;
    /// The player has player data (false → fatal assertion).
    fn has_player_data(&self, player: Self::Unit) -> bool;
    /// Player data +0x44: the pet lists, `None` when P is null. Only called
    /// when [`has_player_data`](Self::has_player_data) is true.
    fn pet_lists(&mut self, player: Self::Unit) -> Option<&mut PetLists>;
    /// `pettype.txt` row count (data tables +0xBF0).
    fn pettype_count(&self) -> i32;
    /// `group` (+0x08, read as i16) of `pettype` row `t`, 0 ≤ t < count.
    fn pettype_group(&self, t: i32) -> i16;
    /// The monster of a GUID.
    fn monster_by_guid(&self, guid: i32) -> Option<Self::Unit>;
    /// Unit GUID.
    fn unit_guid(&self, u: Self::Unit) -> i32;
    /// Unit class id (monstats row for a monster).
    fn unit_class(&self, u: Self::Unit) -> u16;
    /// Unit +0xC8 bit 8 (0x100) set.
    fn flag_c8_bit8(&self, u: Self::Unit) -> bool;
    /// Unit flags (+0xC4).
    fn unit_flags(&self, u: Self::Unit) -> u32;
    /// Writes the unit flags (+0xC4).
    fn set_unit_flags(&mut self, u: Self::Unit, flags: u32);
    /// The unit's monstats has `killable` (+0x0D bit 7).
    fn killable(&self, u: Self::Unit) -> bool;
    /// Kill the unit.
    fn kill(&mut self, u: Self::Unit);
    /// The unit's owner.
    fn owner(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// Mode request 0 (death) for `u` with `target`.
    fn request_death_mode(&mut self, u: Self::Unit, target: Option<Self::Unit>);
    /// The maximum resync of the player (when it runs is not settled); it
    /// may change entry maxima through [`pet_lists`](Self::pet_lists).
    fn resync(&mut self, player: Self::Unit);
    /// Every player of the game in player-list order.
    fn players(&self) -> Vec<Self::Unit>;
    /// The unit has state `state`.
    fn has_state(&self, u: Self::Unit, state: u16) -> bool;
    /// Sends `msg` to the client of `client_player`.
    fn send(&mut self, client_player: Self::Unit, msg: PetMsg);
}

/// Entry `t` of the player's lists: `None` for P null or t out of
/// 0…pettype count − 1. Player data must be present.
fn entry<W: PetWorld>(w: &mut W, player: W::Unit, t: i32) -> Option<&mut PetEntry> {
    if t < 0 || t >= w.pettype_count() {
        return None;
    }
    w.pet_lists(player)?.entries.get_mut(t as usize)
}

fn require_data<W: PetWorld>(w: &W, player: W::Unit) -> Result<(), PetError> {
    if w.has_player_data(player) {
        Ok(())
    } else {
        Err(PetError::NoPlayerData)
    }
}

/// The head's GUID, −1 without a head.
pub fn head_guid(entry: &PetEntry) -> i32 {
    entry.nodes.first().map_or(-1, |n| n.guid)
}

/// The first pet of type `t`: the
/// first node from the head (with `any` false: the first node whose flags
/// bit 0 is clear) and its monster. `None` when the player
/// data, P, the entry, the node or its unit is missing.
pub fn first_pet<W: PetWorld>(w: &mut W, player: W::Unit, t: i32, any: bool) -> Option<W::Unit> {
    if !w.has_player_data(player) {
        return None;
    }
    let guid = entry(w, player, t)?
        .nodes
        .iter()
        .find(|n| any || n.flags & NODE_SKIP == 0)?
        .guid;
    w.monster_by_guid(guid)
}

/// Add `pet` to entry `t` with maximum `max`.
pub fn add<W: PetWorld>(
    w: &mut W,
    player: Option<W::Unit>,
    pet: Option<W::Unit>,
    t: i32,
    max: i32,
) -> Result<(), PetError> {
    // Step 1.
    let Some(player) = player else { return Ok(()) };
    if !w.is_player(player) || t < 1 || t >= w.pettype_count() {
        return Ok(());
    }
    // Steps 2, 3.
    group_evict(w, player, t)?;
    set_max(w, player, t, max)?;
    // Step 4.
    if !w.has_player_data(player) || entry(w, player, t).is_none() {
        return Ok(());
    }
    // Step 5.
    if !append(w, player, t, pet, [0; 3])? {
        return Ok(());
    }
    // Step 6. The record's +0x10 / +0x14 are not set by the add; narrowest
    // reading: 0, so the add path always sends 0x7A.
    // TODO: confirm with the hireling caller.
    let record = AddRecord {
        pet_guid: pet.map_or(-1, |p| w.unit_guid(p)),
        owner_guid: w.unit_guid(player),
        class: pet.map_or(0xFFFF, |p| w.unit_class(p)),
        pet_type: t,
        x10: 0,
        x14: 0,
    };
    broadcast_add(w, &record);
    Ok(())
}

/// Group eviction of type `t`: removes, with kill, every pet of
/// the other types of `t`'s `group`, in row order, each list from its head
/// until a head's unit is missing (edge case 3).
pub fn group_evict<W: PetWorld>(w: &mut W, player: W::Unit, t: i32) -> Result<(), PetError> {
    let g = w.pettype_group(t);
    if g <= 0 {
        return Ok(());
    }
    for u in 0..w.pettype_count() {
        if u == t || w.pettype_group(u) != g {
            continue;
        }
        let mut k = first_pet(w, player, u, true);
        while let Some(unit) = k {
            let guid = w.unit_guid(unit);
            remove(w, player, guid, true)?;
            k = first_pet(w, player, u, true);
        }
    }
    Ok(())
}

/// Set the maximum of entry `t` to `max`: trims the list
/// from its head while it holds more than `max` (never the hirelings).
pub fn set_max<W: PetWorld>(w: &mut W, player: W::Unit, t: i32, max: i32) -> Result<(), PetError> {
    require_data(w, player)?;
    if t == PETTYPE_SINGLE && max != 1 {
        return Ok(());
    }
    let Some(e) = entry(w, player, t) else {
        return Ok(());
    };
    e.max = max;
    loop {
        let Some(e) = entry(w, player, t) else {
            return Ok(());
        };
        if !(max < e.count && e.count > 0 && t != PETTYPE_HIREABLE) {
            return Ok(());
        }
        let guid = head_guid(e);
        remove(w, player, guid, true)?;
    }
}

/// Append (pet, extra) to entry `t`. Returns whether the
/// pet was linked (the base game's 1 / 0).
pub fn append<W: PetWorld>(
    w: &mut W,
    player: W::Unit,
    t: i32,
    pet: Option<W::Unit>,
    extra: [i32; 3],
) -> Result<bool, PetError> {
    let pet_guid = pet.map_or(-1, |p| w.unit_guid(p));
    // The base game receives the entry pointer itself (the add checks
    // it first); a missing entry is a caller error here, not a rule.
    let e = entry(w, player, t).ok_or(PetError::Fatal("append: no entry"))?;
    // Step 1.
    if e.max == 0 {
        w.resync(player);
        // The resync may have set this entry's maximum; read it
        // again; an entry that vanished reads as 0.
        let max = entry(w, player, t).map_or(0, |e| e.max);
        if max == 0 {
            dismiss(w, pet_guid)?;
            return Ok(false);
        }
    }
    // Step 2.
    let e = entry(w, player, t).ok_or(PetError::Fatal("append: no entry"))?;
    if e.count == e.max {
        if e.nodes.is_empty() {
            return Err(PetError::Fatal("append: full list without a head"));
        }
        let head = head_guid(e);
        unlink(w, player, t, head, true)?;
        let e = entry(w, player, t).ok_or(PetError::Fatal("append: no entry"))?;
        if e.count == e.max {
            return Err(PetError::Fatal("append: still full after unlink"));
        }
    }
    // Step 3.
    let e = entry(w, player, t).ok_or(PetError::Fatal("append: no entry"))?;
    e.nodes.push(PetNode {
        flags: 0,
        guid: pet_guid,
        extra,
    });
    e.count += 1;
    if e.count > e.max {
        return Err(PetError::Fatal("append: count above max"));
    }
    Ok(true)
}

/// Remove the pet of `guid` (kill: kill its unit). Broadcasts
/// "remove" even for a GUID in no list (edge case 2), and a second time
/// after the unlink's own (edge case 1).
pub fn remove<W: PetWorld>(
    w: &mut W,
    player: W::Unit,
    guid: i32,
    kill: bool,
) -> Result<(), PetError> {
    // Step 1.
    require_data(w, player)?;
    if w.pet_lists(player).is_none() {
        return Ok(());
    }
    // Step 2.
    let t = lookup(w, player, guid)?;
    if t == 0 && kill {
        dismiss(w, guid)?;
    }
    // Step 3.
    if t >= 0 && t < w.pettype_count() {
        unlink(w, player, t, guid, kill)?;
    }
    // Step 4.
    broadcast_remove(w, guid);
    Ok(())
}

/// Unlink: drops the first node of `guid` from entry `t`,
/// broadcasts "remove", then clears the pet flag (no kill) or dismisses
/// (kill) the unit if it exists.
pub fn unlink<W: PetWorld>(
    w: &mut W,
    player: W::Unit,
    t: i32,
    guid: i32,
    kill: bool,
) -> Result<(), PetError> {
    let Some(e) = entry(w, player, t) else {
        return Ok(());
    };
    let Some(i) = e.nodes.iter().position(|n| n.guid == guid) else {
        return Ok(());
    };
    e.nodes.remove(i);
    broadcast_remove(w, guid);
    if let Some(unit) = w.monster_by_guid(guid) {
        if kill {
            dismiss(w, guid)?;
        } else {
            let flags = w.unit_flags(unit) & !FLAG_PET;
            w.set_unit_flags(unit, flags);
        }
    }
    let e = entry(w, player, t).ok_or(PetError::Fatal("unlink: entry vanished"))?;
    e.count -= 1;
    if e.count < 0 {
        return Err(PetError::Fatal("unlink: count below 0"));
    }
    Ok(())
}

/// Dismiss: no-experience flag, then kill (`killable`) or
/// a death mode request aimed at the owner; broadcasts "remove".
pub fn dismiss<W: PetWorld>(w: &mut W, guid: i32) -> Result<(), PetError> {
    let Some(unit) = w.monster_by_guid(guid) else {
        return Ok(());
    };
    if w.flag_c8_bit8(unit) {
        return Err(PetError::Fatal("dismiss: unit +0xC8 bit 8 set"));
    }
    let flags = w.unit_flags(unit) | FLAG_NO_EXPERIENCE;
    w.set_unit_flags(unit, flags);
    if w.killable(unit) {
        w.kill(unit);
    } else {
        let owner = w.owner(unit);
        w.request_death_mode(unit, owner);
    }
    broadcast_remove(w, guid);
    Ok(())
}

/// Sends `msg` to every player without state 7, in player-list order.
fn broadcast<W: PetWorld>(w: &mut W, msg: PetMsg) {
    for p in w.players() {
        if !w.has_state(p, STATE_NO_BROADCAST) {
            w.send(p, msg);
        }
    }
}

/// Broadcast "add": 0x7A action 1 when record +0x10 and
/// +0x14 are both 0, else 0x81.
pub fn broadcast_add<W: PetWorld>(w: &mut W, record: &AddRecord) {
    let msg = if record.x10 == 0 && record.x14 == 0 {
        PetMsg::PetAction {
            action: 1,
            pet_type: record.pet_type as u8,
            class: record.class,
            owner: record.owner_guid as u32,
            pet: record.pet_guid as u32,
        }
    } else {
        PetMsg::AssignMerc { record: *record }
    };
    broadcast(w, msg);
}

/// Broadcast "remove": 0x7A action 0, type, class and
/// owner 0.
pub fn broadcast_remove<W: PetWorld>(w: &mut W, guid: i32) {
    broadcast(
        w,
        PetMsg::PetAction {
            action: 0,
            pet_type: 0,
            class: 0,
            owner: 0,
            pet: guid as u32,
        },
    );
}

/// Lookup of a GUID: the first type t in 1…pettype
/// count − 1 whose list holds `guid`, else 0 (also for P null).
pub fn lookup<W: PetWorld>(w: &mut W, player: W::Unit, guid: i32) -> Result<i32, PetError> {
    require_data(w, player)?;
    let count = w.pettype_count();
    let Some(lists) = w.pet_lists(player) else {
        return Ok(0);
    };
    for t in 1..count {
        let Some(e) = lists.entries.get(t as usize) else {
            break;
        };
        if e.nodes.iter().any(|n| n.guid == guid) {
            return Ok(t);
        }
    }
    Ok(0)
}

// ---- creation, free, death and maximum resync ----------------------

/// One skill of the player for the resync: its `pettype` and the
/// `petmax` calc already evaluated at the skill's level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResyncSkill {
    /// Skills row `pettype` (+0xBE, i8).
    pub pettype: i8,
    /// `eval(petmax)`.
    pub petmax: i32,
}

/// The world the creation, free and resync need beyond [`PetWorld`].
pub trait PetLifecycleWorld: PetWorld {
    /// Stores the pet lists P (player data +0x44; `None` clears them).
    fn set_pet_lists(&mut self, player: Self::Unit, lists: Option<PetLists>);
    /// `basemax` (+0x0A, i16) of `pettype` row `t`.
    fn pettype_basemax(&self, t: i32) -> i32;
    /// The player's skills in list order; `None` without a
    /// skill list.
    fn skills(&self, player: Self::Unit) -> Option<Vec<ResyncSkill>>;
    /// Hireling removal: the unit's room gets a removal notice
    /// (type 1 with the GUID) and the unit is freed.
    fn free_hireling_unit(&mut self, unit: Self::Unit);
}

/// Create: a new list set with every maximum
/// set to its row's `basemax`. Lists already present are freed first.
pub fn create<W: PetLifecycleWorld>(w: &mut W, player: W::Unit) -> Result<(), PetError> {
    require_data(w, player)?;
    if w.pet_lists(player).is_some() {
        free_all(w, player)?;
    }
    let count = w.pettype_count();
    w.set_pet_lists(player, Some(PetLists::new(count.max(0) as usize)));
    for t in 0..count {
        let max = w.pettype_basemax(t);
        set_max(w, player, t, max)?;
    }
    Ok(())
}

/// Walks entry `t` from the head: a non-hireling node is
/// dismissed, a hireling node whose unit exists is announced removed and
/// freed; count and max drop by one per node.
fn drain_entry<W: PetLifecycleWorld>(
    w: &mut W,
    player: W::Unit,
    t: i32,
    hireling: bool,
) -> Result<(), PetError> {
    loop {
        let Some(e) = entry(w, player, t) else {
            return Ok(());
        };
        if e.nodes.is_empty() {
            return Ok(());
        }
        let guid = e.nodes[0].guid;
        if !hireling {
            dismiss(w, guid)?;
        } else if let Some(unit) = w.monster_by_guid(guid) {
            broadcast_remove(w, guid);
            w.free_hireling_unit(unit);
        }
        if let Some(e) = entry(w, player, t) {
            e.nodes.remove(0);
            e.count -= 1;
            e.max -= 1;
        }
    }
}

/// Free: every list drained for t = 1 … count − 1, then the
/// lists are dropped. P null → nothing.
pub fn free_all<W: PetLifecycleWorld>(w: &mut W, player: W::Unit) -> Result<(), PetError> {
    require_data(w, player)?;
    if w.pet_lists(player).is_none() {
        return Ok(());
    }
    for t in 1..w.pettype_count() {
        drain_entry(w, player, t, t == PETTYPE_HIREABLE)?;
    }
    w.set_pet_lists(player, None);
    Ok(())
}

/// Player death: every pet of a type other than 7 is
/// dismissed and its node freed (count and max decremented), then the
/// maxima are resynced.
pub fn player_death<W: PetLifecycleWorld>(w: &mut W, player: W::Unit) -> Result<(), PetError> {
    require_data(w, player)?;
    if w.pet_lists(player).is_none() {
        return Ok(());
    }
    for t in 1..w.pettype_count() {
        if t != PETTYPE_HIREABLE {
            drain_entry(w, player, t, false)?;
        }
    }
    resync_max(w, player)
}

/// Resync: each skill with `0 < pettype < count` raises that
/// type's maximum to `max(petmax, 1)` when above the highest met so far
/// (setting it at once, which may trim); then every type no skill raised
/// returns to `basemax`.
pub fn resync_max<W: PetLifecycleWorld>(w: &mut W, player: W::Unit) -> Result<(), PetError> {
    if !w.is_player(player) {
        return Ok(());
    }
    require_data(w, player)?;
    if w.pet_lists(player).is_none() {
        return Ok(());
    }
    let Some(skills) = w.skills(player) else {
        return Ok(());
    };
    let count = w.pettype_count();
    let mut m = [0i32; 256];
    for s in skills {
        let t = i32::from(s.pettype);
        if t > 0 && t < count {
            let v = s.petmax.max(1);
            if v > m[t as usize] {
                m[t as usize] = v;
                set_max(w, player, t, v)?;
            }
        }
    }
    for t in 0..count.min(256) {
        if m[t as usize] <= 0 {
            let max = w.pettype_basemax(t);
            set_max(w, player, t, max)?;
        }
    }
    Ok(())
}
