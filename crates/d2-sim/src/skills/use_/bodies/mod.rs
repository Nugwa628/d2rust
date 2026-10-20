//! The per-skill start / do bodies. Two kinds:
//!
//! - [`start`]: bodies that read and change nothing (srvst 18 Attract,
//!   `functions.tsv` notes). The start core runs
//!   them directly.
//! - [`run_start`] / [`run_do`]: the bodies of the `functions.tsv` rows
//!   with status `spec'd-here` ([`START_BODIES`], [`DO_BODIES`]), over
//!   the [`BodyWorld`] seam. They run behind
//!   [`super::SkillFunctions`]: a provider answers `srvst` / `srvdo`
//!   from here when the slot has a body and from its own seam otherwise
//!   (every other filled slot: status `mapped`, answered by the
//!   provider's own seam).
//!
//! Helpers are in [`helpers`], starts in [`starts`], do
//! functions in [`dos`]; the `srvmissile` path is the do core's
//! (`use_::do_core`) with the missile record of [`skill_missile`].
//!
//! Status: implemented, not yet checked against a playthrough.

pub mod b3_lvl01;
pub mod b3_lvl06;
pub mod b3_lvl12;
pub mod b3_lvl18;
pub mod b3_lvl24;
pub mod b3_lvl30;
pub mod b4_helpers;
pub mod b4_mon;
pub mod b4_more;
pub mod dos;
pub mod dos2;
pub mod effects;
#[cfg(test)]
pub(crate) mod fake;
pub mod helpers;
pub mod helpers2;
pub mod helpers3;
pub mod passive;
pub mod starts;
pub mod starts2;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod tests2;
#[cfg(test)]
mod tests3;
#[cfg(test)]
mod tests_use;

#[cfg(test)]
mod tests_b3b;

#[cfg(test)]
mod tests4;
#[cfg(test)]
mod tests5;
#[cfg(test)]
mod tests6;
#[cfg(test)]
mod tests_b1a;
#[cfg(test)]
mod tests_b1b;
#[cfg(test)]
mod tests_b3a;

pub use b4_helpers::{diab_wall_cb, zigzag_cb, zigzag_ring_cb, PathMissile};
pub use effects::{BodyEffect, PathOp};
pub use helpers::*;
pub use helpers2::*;
pub use helpers3::*;

use super::UseWorld;
use crate::combat::{CombatTables, CombatWorld};
use crate::skills::{KickItems, SkillEntry, SkillTables};

/// `srvst` slots whose body reads and changes nothing ([`start`]).
pub const PURE_START: &[u16] = &[18];
/// `srvst` slots with a body over [`BodyWorld`] (`functions.tsv` status
/// `spec'd-here`).
pub const START_BODIES: &[u16] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65,
];
/// `srvdo` slots with a body over [`BodyWorld`] (status `spec'd-here`).
pub const DO_BODIES: &[u16] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75,
    76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
    100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
    119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137,
    139, 140, 141, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152,
];

/// A duration of `n` base ticks as ticks at the game's rate: skill
/// timings (skill calcs, `Param` values, fixed delays) are stated at the
/// base rate.
pub fn ticks<W: UseWorld + ?Sized>(w: &W, n: i32) -> i32 {
    UseWorld::tick_rate(w).ticks(n)
}

/// The frame `n` base ticks from now.
pub fn after<W: UseWorld + ?Sized>(w: &W, n: i32) -> i32 {
    UseWorld::frame(w).wrapping_add(ticks(w, n))
}

/// `srvst[index](game, unit, skill, level)` when the slot's body reads
/// and changes nothing; `None` otherwise.
pub fn start(index: u16) -> Option<i32> {
    match index {
        // SrvSt18 Attract: `mov eax, 1; ret 8`, arguments
        // unread, nothing changed.
        18 => Some(1),
        _ => None,
    }
}

/// The remove callbacks of a stat list (list +0x38 ids, opaque values).
pub mod callback {
    /// Missile just-hit list:
    /// state off and the update queue only.
    pub const JUSTHIT: u32 = 0x005A_DAF0;
    /// Default.
    pub const DEFAULT: u32 = 0x0056_E900;
    /// Cold: state off; a living
    /// unit also has shatter (107) off; then the anim refresh.
    pub const COLD: u32 = 0x0057_AD80;
    /// Freeze end: a dead unit keeping the state on
    /// death is left alone; else state off, mode 1, and a monster's
    /// thinks are rescheduled at frame + `aidel`.
    pub const FREEZE_END: u32 = 0x0057_B170;
    /// Self aura.
    pub const SELF_AURA: u32 = 0x005C_EC50;
    /// Buff.
    pub const BUFF: u32 = 0x005C_9420;
    /// AI curse (Dim Vision, Terror).
    pub const AI_CURSE: u32 = 0x005C_3370;
    /// Charge counter: state off.
    pub const CHARGE: u32 = 0x005D_3310;
    /// Inferno: state off, flags |= 0x40.
    pub const INFERNO: u32 = 0x005C_8BF0;
    /// Blade Fury: as Inferno's.
    pub const BLADE_FURY: u32 = 0x005D_69B0;
    /// Werewolf / Werebear.
    pub const SHAPE: u32 = 0x005C_6C50;
    /// Whirlwind.
    pub const WHIRLWIND: u32 = 0x005D_8EF0;
    /// Holy Freeze self list.
    pub const HOLY_FREEZE: u32 = 0x005D_0770;
    /// Confuse.
    pub const CONFUSE: u32 = 0x005C_3DB0;
    /// Attract.
    pub const ATTRACT: u32 = 0x005C_3B00;
    /// Conversion.
    pub const CONVERSION: u32 = 0x005D_01A0;
    /// Mind Blast.
    pub const MIND_BLAST: u32 = 0x005D_7310;
    /// Pregnant.
    pub const PREGNANT: u32 = 0x005D_21B0;
    /// Every callback id the bodies specify.
    pub const ALL: [u32; 15] = [
        DEFAULT,
        SELF_AURA,
        BUFF,
        AI_CURSE,
        CHARGE,
        INFERNO,
        BLADE_FURY,
        SHAPE,
        WHIRLWIND,
        HOLY_FREEZE,
        CONFUSE,
        ATTRACT,
        CONVERSION,
        MIND_BLAST,
        PREGNANT,
    ];
}

/// The itemstatcost fields the bodies read (`direct` +0x04 bit 4,
/// `updateanimrate` +0x04 bit 9, `maxstat` +0x32, `itemevent1–2` +0x48 /
/// +0x4A, `itemeventfunc1–2` +0x4C / +0x4E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BodyStat {
    pub direct: bool,
    pub updateanimrate: bool,
    /// −1 when none.
    pub maxstat: i32,
    /// `itemevent1`, `itemevent2` (i16; ≤ 0 none).
    pub itemevent: [i32; 2],
    /// `itemeventfunc1`, `itemeventfunc2` (i16).
    pub itemeventfunc: [i32; 2],
    /// `send bits` (+0x08), `send param bits` (+0x09), `signed` (+0x04
    /// bit 1): read by the unit-state message 0xAA
    /// (step 1.3 of that message), not by the bodies.
    pub send_bits: u8,
    pub send_param_bits: u8,
    pub signed: bool,
    /// `send other` (+0x04 bit 0): read by the monster assign 0xAC.
    pub send_other: bool,
}

/// The table data the bodies read beyond [`SkillTables`] and
/// [`CombatTables`]: itemstatcost flags, states `group` and `aura`, the
/// overlay count. A provider without them answers "no record".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyTables {
    /// By itemstatcost row.
    pub stats: Vec<BodyStat>,
    /// States `group` (+0x1E) by state.
    pub state_group: Vec<i32>,
    /// States `aura` (+0x10 bit 1) by state.
    pub state_aura: Vec<bool>,
    /// States `nosend` (+0x10 bit 0) by state (the unit-state message,
    /// step 1.1 of that message).
    pub state_nosend: Vec<bool>,
    /// `overlay.txt` record count.
    pub overlay_count: i32,
    /// `monlvl.txt` rows.
    pub monlvl: Vec<d2_data::tables::Monlvl>,
    /// `pettype.txt` record count.
    pub pettype_count: i32,
    /// `pettype.txt` `group` (+0x08, i16) by row.
    pub pettype_group: Vec<i16>,
    /// `pettype.txt` `basemax` (+0x0A, i16) by row.
    pub pettype_basemax: Vec<i16>,
    /// `pettype.txt` flag byte (+0x04) by row: bit 0 `warp`, bit 1 `range`.
    pub pettype_flags: Vec<u8>,
    /// The item-event layer split (data +0xC6C shift, +0xC70 mask;
    /// runtime maps): `stuff` of itemstatcost record 0
    /// (+0x140) when in 1..8, else 6; mask (1 << stuff) − 1. `None`
    /// without itemstatcost rows.
    pub layer_split: Option<(u32, u32)>,
}

/// The runtime-map layer split from `stuff` of
/// itemstatcost record 0.
pub fn layer_split(stuff: u32) -> (u32, u32) {
    let s = if (1..=8).contains(&stuff) { stuff } else { 6 };
    (s, (1 << s) - 1)
}

impl BodyTables {
    /// From a loaded `.bin` set.
    pub fn from_bin(set: &d2_data::bin::BinSet) -> Result<Self, crate::skills::TablesError> {
        use crate::skills::TablesError;
        use d2_data::tables::decode_all;
        let table = |n: &'static str| set.table(n).ok_or(TablesError::Missing(n));
        let mut b = Self::from_tables(table("itemstatcost")?, table("states")?, table("overlay")?)?;
        b.monlvl = d2_data::tables::decode_all(table("monlvl")?)?;
        b.pettype_count = i32::try_from(table("pettype")?.count).unwrap_or(i32::MAX);
        b.pettype_group = decode_all::<d2_data::tables::Pettype>(table("pettype")?)?
            .iter()
            .map(|r| r.group as i16)
            .collect();
        b.pettype_basemax = decode_all::<d2_data::tables::Pettype>(table("pettype")?)?
            .iter()
            .map(|r| r.basemax as i16)
            .collect();
        b.pettype_flags = decode_all::<d2_data::tables::Pettype>(table("pettype")?)?
            .iter()
            .map(|r| u8::from(r.warp) | u8::from(r.range) << 1)
            .collect();
        Ok(b)
    }

    /// From the itemstatcost, states and overlay tables (no monlvl rows,
    /// no pet types).
    pub fn from_tables(
        itemstatcost: &d2_data::bin::BinTable,
        states: &d2_data::bin::BinTable,
        overlay: &d2_data::bin::BinTable,
    ) -> Result<Self, crate::skills::TablesError> {
        use d2_data::tables::{decode_all, Itemstatcost, States};
        let isc: Vec<Itemstatcost> = decode_all(itemstatcost)?;
        let states: Vec<States> = decode_all(states)?;
        let i16v = |v: u16| i32::from(v as i16);
        Ok(Self {
            stats: isc
                .iter()
                .map(|r| BodyStat {
                    direct: r.direct,
                    updateanimrate: r.updateanimrate,
                    maxstat: i16v(r.maxstat),
                    itemevent: [i16v(r.itemevent1), i16v(r.itemevent2)],
                    itemeventfunc: [i16v(r.itemeventfunc1), i16v(r.itemeventfunc2)],
                    send_bits: r.send_bits,
                    send_param_bits: r.send_param_bits,
                    signed: r.signed,
                    send_other: r.send_other,
                })
                .collect(),
            state_group: states.iter().map(|r| i32::from(r.group as i16)).collect(),
            state_aura: states.iter().map(|r| r.aura).collect(),
            state_nosend: states.iter().map(|r| r.nosend).collect(),
            overlay_count: i32::try_from(overlay.count).unwrap_or(i32::MAX),
            monlvl: Vec::new(),
            pettype_count: 0,
            pettype_group: Vec::new(),
            pettype_basemax: Vec::new(),
            pettype_flags: Vec::new(),
            layer_split: isc.first().map(|r| layer_split(r.stuff)),
        })
    }

    /// The itemstatcost fields of stat `s`, `None` past the table.
    pub fn stat(&self, s: i32) -> Option<BodyStat> {
        usize::try_from(s)
            .ok()
            .and_then(|i| self.stats.get(i))
            .copied()
    }
}

/// A unit event handler record (0x20 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handler {
    /// +0x00 event (`events.txt` index).
    pub event: u8,
    /// +0x04 key type (1 = a state).
    pub key_type: i32,
    /// +0x08 key.
    pub key: i32,
    /// +0x0C skill.
    pub skill: i32,
    /// +0x10 level.
    pub level: i32,
    /// +0x14 function (index into the event table).
    pub func: i32,
}

/// One room of a scan: the town test and the room's unit list in
/// list order (head +0x74, next +0xE8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRoom<U> {
    pub town: bool,
    pub units: Vec<U>,
}

/// The missile parameter record the bodies fill (0x5C bytes,
/// zeroed first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissileRequest<U> {
    /// +0x00.
    pub flags: u32,
    /// +0x04.
    pub owner: U,
    /// +0x08.
    pub origin: Option<U>,
    /// +0x0C target unit.
    pub target: Option<U>,
    /// +0x10.
    pub class: i32,
    /// +0x14, +0x18.
    pub x: i32,
    pub y: i32,
    /// +0x1C, +0x20.
    pub target_x: i32,
    pub target_y: i32,
    /// +0x28.
    pub velocity: i32,
    /// +0x2C, +0x30.
    pub skill: i32,
    pub level: i32,
    /// +0x34.
    pub loops: i32,
    /// +0x44 activate frames.
    pub activate: i32,
    /// +0x48.
    pub attack_bonus: i32,
    /// +0x4C.
    pub range: i32,
    /// +0x54 init callback (its id) and +0x58 its argument.
    pub init: Option<(u32, u32)>,
}

impl<U> MissileRequest<U> {
    /// A zeroed record with the owner and the class.
    pub fn new(owner: U, class: i32) -> Self {
        Self {
            flags: 0,
            owner,
            origin: None,
            target: None,
            class,
            x: 0,
            y: 0,
            target_x: 0,
            target_y: 0,
            velocity: 0,
            skill: 0,
            level: 0,
            loops: 0,
            activate: 0,
            attack_bonus: 0,
            range: 0,
            init: None,
        }
    }
}

/// Missile init callbacks the bodies pass (+0x54).
pub mod init_cb {
    /// Jitter.
    pub const JITTER: u32 = 0x005C_9290;
    /// `damagepercent(25)` += argument.
    pub const DAMAGE_PERCENT: u32 = 0x005D_B6A0;
    /// DiabWall.
    pub const DIAB_WALL: u32 = 0x005C_D110;
    /// Lightning fan.
    pub const ZIGZAG: u32 = 0x005D_4680;
    /// Lightning ring.
    pub const ZIGZAG_RING: u32 = 0x005D_40F0;
}

/// Message 0xA3 queued by the progressive finisher (its step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressiveMsg<U> {
    pub charges: u8,
    pub skill: u16,
    pub level: u16,
    pub unit: U,
    pub target: U,
    pub roll: u32,
}

/// What the bodies need beyond the skill use pipeline's [`UseWorld`] (a
/// seam). Combat runs through [`BodyWorld::combat`] on the same units.
/// Providers by area: stat lists and states, rooms
/// (adjacency, unit order), items (quantity,
/// durability, composits), monsters (AI install),
/// player messages.
pub trait BodyWorld: UseWorld + KickItems {
    /// A stat list handle.
    type List: Copy + Eq + std::fmt::Debug;
    /// A room handle.
    type Room: Copy + Eq + std::fmt::Debug;
    /// Combat's view of the same units.
    type Combat: CombatWorld<Unit = Self::Unit, Item = Self::Item>;
    fn combat(&mut self) -> &mut Self::Combat;

    // ---- data
    /// The itemstatcost record of `s` (`None`: no record).
    fn stat_info(&self, s: i32) -> Option<BodyStat>;
    /// States count (`[0x744304]+0xC4`).
    fn state_count(&self) -> i32;
    /// State `s` has flag group `g` (the runtime-map bitsets).
    fn state_flag(&self, s: i32, g: usize) -> bool;
    /// States `group` (+0x1E) of `s`.
    fn state_group(&self, s: i32) -> i32;
    /// States `aura` (+0x10 bit 1).
    fn state_is_aura(&self, s: i32) -> bool;
    /// Overlay count (bound of `srvoverlay` / `tgtoverlay`).
    fn overlay_count(&self) -> i32;

    // ---- states on a unit
    /// Any state with flag bit g.
    fn has_group(&self, u: Self::Unit, g: usize) -> bool;
    /// State on / off (toggle with update-queue insert).
    fn state_on(&mut self, u: Self::Unit, s: i32, on: bool);
    /// Mark changed.
    fn mark_state_changed(&mut self, u: Self::Unit, s: i32);
    /// Every state of group `g` cleared at
    /// once (changed bits set, bits cleared, unit queued).
    fn clear_group_states(&mut self, u: Self::Unit, g: usize);
    /// Queue the unit for update.
    fn queue_update(&mut self, u: Self::Unit);
    /// The state stays on the unit at death.
    fn stays_on_death(&self, u: Self::Unit, s: i32) -> bool;

    // ---- stat lists
    /// The unit's list of state `s`.
    fn state_list(&self, u: Self::Unit, s: i32) -> Option<Self::List>;
    /// The unit's first list with all of `flags`.
    fn first_list_with_flags(&self, u: Self::Unit, flags: u32) -> Option<Self::List>;
    /// A new stat list (pool, flags, expire, owner type, owner GUID); no
    /// owner → type 6, GUID −1.
    fn alloc_list(
        &mut self,
        flags: u32,
        expire: i32,
        owner: Option<Self::Unit>,
    ) -> Option<Self::List>;
    fn list_state(&self, l: Self::List) -> i32;
    fn set_list_state(&mut self, l: Self::List, s: i32);
    /// List +0x1C skill, +0x20 level.
    fn list_skill(&self, l: Self::List) -> (i32, i32);
    fn set_list_skill(&mut self, l: Self::List, skill: i32, lvl: i32);
    fn list_expire(&self, l: Self::List) -> i32;
    fn set_list_expire(&mut self, l: Self::List, e: i32);
    fn list_get(&self, l: Self::List, s: i32) -> i32;
    /// List set (layer 0).
    fn list_set(&mut self, l: Self::List, s: i32, v: i32);
    /// List set on a layer (the passive
    /// refresh's `passiveitype` layer).
    fn list_set_layer(&mut self, l: Self::List, s: i32, v: i32, layer: u16);
    fn attach(&mut self, u: Self::Unit, l: Self::List);
    fn set_remove_callback(&mut self, l: Self::List, cb: u32);
    /// Detach (which
    /// runs the list's remove callback, [`remove_callback`]) and free.
    fn detach_free(&mut self, u: Self::Unit, l: Self::List);

    // ---- unit event handlers
    /// Allocate the record and prepend it.
    fn add_handler(&mut self, u: Self::Unit, h: Handler);
    fn remove_handlers(&mut self, u: Self::Unit, key_type: i32, key: i32);

    // ---- rooms and relations
    /// The rooms a scan visits: the adjacency array (
    /// itself included) of the source's room (`at` = `None`) or of the
    /// room containing `at`, searched from the source's room
    /// (the room-at-point lookup). `None`: no such room.
    fn scan_rooms(
        &self,
        source: Self::Unit,
        at: Option<(i32, i32)>,
    ) -> Option<Vec<ScanRoom<Self::Unit>>>;
    /// Allies (same unit after the monster owner
    /// resolution, or two players in one party).
    fn allied(&self, a: Self::Unit, b: Self::Unit) -> bool;
    /// A missile's owner.
    fn missile_owner(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// A monster's minion owner.
    fn minion_owner(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// The monster's AI control owner, as leave pack reads it.
    /// Default: [`Self::minion_owner`].
    fn pack_owner(&self, u: Self::Unit) -> Option<Self::Unit> {
        self.minion_owner(u)
    }
    /// The pet is unsummonable: the pet's owner link and the pettype `unsummon` bit (+0x04
    /// bit 3) of the pet's type.
    fn pet_unsummonable(&self, u: Self::Unit, pet: Self::Unit) -> bool;

    // ---- unit fields
    /// The frame bonus.
    fn frame_bonus(&self, u: Self::Unit) -> i32;
    /// Unit +0x44 (animation frame, 8.8).
    fn set_anim_frame(&mut self, u: Self::Unit, v: i32);
    /// Used skill entry param `i` (1: +0x18, 2: the entry's second
    /// param).
    fn set_entry_param(&mut self, u: Self::Unit, i: u8, v: i32);
    /// Maximum life, mana,
    /// stamina (stat 6, 8, 10).
    fn stat_max(&self, u: Self::Unit, s: u16) -> i32;

    // ---- items
    /// The composit weapon class (through the item's own class when
    /// unit +0xC8 bit 3 is set).
    fn composit_weapon_class(&self, u: Self::Unit) -> i32;
    /// A player's hand class.
    fn hand_class(&self, u: Self::Unit) -> i32;
    /// The item's type has a `shoots` value (itemtypes
    /// +0x0C).
    fn item_shoots(&self, item: Self::Item) -> bool;
    /// Items `stackable` (+0x132).
    fn item_stackable(&self, item: Self::Item) -> bool;
    /// Unit getter on an item (stats 70, 72, 125, 157).
    fn item_stat_of(&self, item: Self::Item, s: u16) -> i32;
    /// Unit stat set on an item.
    fn set_item_stat(&mut self, item: Self::Item, s: u16, v: i32);
    /// The maximum stack.
    fn item_max_stack(&self, item: Self::Item) -> i32;
    /// The maximum durability.
    fn item_max_durability(&self, item: Self::Item) -> i32;
    /// The quantity-replenish timer.
    fn quantity_timer(&mut self, item: Self::Item);
    /// Message 0x3E (item stat update) to the player's client.
    fn send_item_stat(&mut self, u: Self::Unit, item: Self::Item, s: u16, v: i32);
    /// Attack-mode cleanup (Open question 4).
    fn attack_cleanup(&mut self, u: Self::Unit);
    /// Weapon cleanup after the attack.
    fn weapon_cleanup(&mut self, u: Self::Unit);

    // ---- other systems
    /// Creates the missile.
    fn spawn_missile(&mut self, req: MissileRequest<Self::Unit>) -> Option<Self::Unit>;
    /// Passive refresh.
    fn passive_refresh(&mut self, u: Self::Unit);
    /// The buff callback's passive refresh.
    fn buff_refresh(&mut self, u: Self::Unit);
    /// A player's skill resync (Open question 5).
    fn skill_resync(&mut self, u: Self::Unit);
    /// Applies the passive's stat changes after its state is switched on.
    fn passive_state_apply(&mut self, u: Self::Unit, e: &SkillEntry);
    /// Sets the monster's AI state `k` (through its AI control when it has
    /// one).
    fn set_ai_state(&mut self, u: Self::Unit, k: i32);
    /// Mana paid with the blood mana state (114).
    fn blood_mana(&mut self, u: Self::Unit, cost: i32);
    /// Queue message 0xA3 on the unit.
    fn queue_progressive(&mut self, u: Self::Unit, msg: ProgressiveMsg<Self::Unit>);

    // ---- batch 2 and 3
    /// A call into another system whose result the bodies do not read.
    fn effect(&mut self, e: BodyEffect<Self::Unit, Self::Item, Self::Room>);
    /// A path operation on the unit's path; returns the compute result
    /// for [`PathOp::Compute`], else 0.
    fn path_op(&mut self, u: Self::Unit, op: PathOp<Self::Unit>) -> i32;

    // ---- data
    /// monlvl rows.
    fn monlvl(&self) -> &[d2_data::tables::Monlvl];
    /// `pettype.txt` record count.
    fn pettype_count(&self) -> i32;
    /// The L-flag: game +0x6A ≠ 0 or game +0x74 ≠ 0 (the monster
    /// init flag).
    fn l_flag(&self) -> bool;

    // ---- units
    /// Unit +0xC8.
    fn unit_c8(&self, u: Self::Unit) -> u32;
    fn set_unit_c8(&mut self, u: Self::Unit, v: u32);
    /// Unit +0x44 (animation frame, 8.8).
    fn anim_frame(&self, u: Self::Unit) -> i32;
    /// Frame event index (unit +0x38 bits 8+).
    fn frame_event_index(&self, u: Self::Unit) -> i32;
    fn set_frame_event_index(&mut self, u: Self::Unit, i: i32);
    /// Unit +0x48 (frame count, 8.8).
    fn frame_count(&self, u: Self::Unit) -> i32;
    fn set_frame_count(&mut self, u: Self::Unit, v: i32);
    /// Unit +0xD0 (target-list slot; 11 = none).
    fn node_slot(&self, u: Self::Unit) -> i32;
    /// Unit +0x5C: the unit has a stat holder.
    fn has_stat_holder(&self, u: Self::Unit) -> bool;
    /// The unit size.
    fn unit_size(&self, u: Self::Unit) -> i32;
    /// The minion spawn class of a monster's AI control (+0x3C,
    /// with no extra lookup); `None` without monster data or control.
    fn minion_spawn_class(&self, u: Self::Unit) -> Option<i32>;
    /// The owner's linked unit (GUID, looked up as a monster
    /// by GUID).
    fn linked_unit(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// The killer the death delay names.
    fn killer_of(&self, u: Self::Unit) -> Option<Self::Unit>;
    /// A monster's minion owner record (AI control +0x2C
    /// GUID, +0x30 type); `None` for GUID −1.
    fn minion_owner_ident(&self, u: Self::Unit) -> Option<(u32, u32)>;
    /// Unit stat += v.
    fn add_stat(&mut self, u: Self::Unit, s: u16, v: i32);
    /// List stat += v.
    fn list_add(&mut self, l: Self::List, s: i32, v: i32);
    /// Every stat of the list removed.
    fn list_clear(&mut self, l: Self::List);
    /// The unit has a handler with this key and skill field.
    fn has_handler(&self, u: Self::Unit, key_type: i32, key: i32, skill: i32) -> bool;

    // ---- skill entries (the skill list's owner)
    /// Param `i` (1…4: +0x18, +0x1C, +0x20, +0x24) of the unit's entry
    /// `e`.
    fn entry_param(&self, u: Self::Unit, e: &SkillEntry, i: u8) -> i32;
    fn set_entry_param_of(&mut self, u: Self::Unit, e: &SkillEntry, i: u8, v: i32);
    /// Entry flags +0x0C.
    fn entry_flags(&self, u: Self::Unit, e: &SkillEntry) -> u32;
    fn set_entry_flags(&mut self, u: Self::Unit, e: &SkillEntry, f: u32);
    /// Entry mode +0x08 := m.
    fn set_entry_mode(&mut self, u: Self::Unit, e: &SkillEntry, m: u32);
    /// A mode through the disguise remap.
    fn disguise_mode(&self, u: Self::Unit, m: u32) -> u32;
    /// The used skill's sequence frame records (6 bytes each,
    /// in the sequence table); `None` without one.
    fn skill_sequence(&self, u: Self::Unit) -> Option<Vec<[u8; 6]>>;

    // ---- animation (unit mode variants)
    /// Rewinds the animation to frame p.
    fn anim_rewind(&mut self, u: Self::Unit, p: i32);
    fn anim_restart(&mut self, u: Self::Unit, v: i32);
    /// The animation from frame f.
    fn anim_from(&mut self, u: Self::Unit, f: i32);

    // ---- rooms
    /// The unit's room.
    fn unit_room(&self, u: Self::Unit) -> Option<Self::Room>;
    /// The room containing (x, y), searched from `from`.
    fn room_at(&self, from: Self::Room, x: i32, y: i32) -> Option<Self::Room>;
    fn room_in_town(&self, r: Self::Room) -> bool;
    /// The act of the room's level.
    fn room_act(&self, r: Self::Room) -> i32;
    /// The room level's `Teleport` (levels +0x04); `None` without a
    /// level record.
    fn room_teleport(&self, r: Self::Room) -> Option<i32>;
    /// The free point
    /// and its room.
    fn free_point(
        &mut self,
        r: Self::Room,
        at: (i32, i32),
        size: i32,
        mask: u32,
        fallback: bool,
    ) -> Option<(Self::Room, (i32, i32))>;
    /// The point collides for u's path pattern and `mask`.
    fn pattern_collides(&self, r: Self::Room, at: (i32, i32), u: Self::Unit, mask: u32) -> bool;
    /// The box (size × size) at the point collides with `mask`.
    fn box_collides(&self, r: Self::Room, at: (i32, i32), size: i32, mask: u32) -> bool;
    /// The line from → to is blocked for `mask` (a unit's line uses the
    /// same test).
    fn line_blocked(&self, r: Self::Room, from: (i32, i32), to: (i32, i32), mask: u32) -> bool;
    /// Places the unit at the point (not exact, no alternative spot): true
    /// when placed.
    fn place_unit(&mut self, u: Self::Unit, r: Option<Self::Room>, at: (i32, i32)) -> bool;

    // ---- paths
    /// The unit has a path (+0x2C).
    fn has_path(&self, u: Self::Unit) -> bool;
    /// The path's point count.
    fn path_point_count(&self, u: Self::Unit) -> i32;
    /// The path's last point.
    fn path_last_point(&self, u: Self::Unit) -> (i32, i32);
    /// The path's target point.
    fn path_target_point(&self, u: Self::Unit) -> (i32, i32);

    // ---- monsters
    fn create_monster(
        &mut self,
        r: Self::Room,
        at: (i32, i32),
        class: i32,
        mode: i32,
        spread: i32,
    ) -> Option<Self::Unit>;

    /// Monster mode request: the request is built, then sent with the
    /// skill flag 1, at `target` when given; the send
    /// call's result.
    fn mode_request(&mut self, m: Self::Unit, mode: i32, target: Option<Self::Unit>) -> i32;

    // ---- items
    /// The item handle of an item unit (type 4); `None` for other units.
    fn as_item(&self, u: Self::Unit) -> Option<Self::Item>;
    /// The player is busy.
    fn inventory_busy(&self, u: Self::Unit) -> bool;
    /// The unit has an inventory (+0x60).
    fn has_inventory(&self, u: Self::Unit) -> bool;
    /// The weapon in use.
    fn weapon_in_use(&self, u: Self::Unit) -> Option<Self::Item>;
    /// The item's body location.
    fn body_loc(&self, i: Self::Item) -> i32;
    /// Usable (not broken, item flag 0x4000 clear).
    fn item_usable(&self, i: Self::Item) -> bool;
    /// Active on its owner.
    fn item_active(&self, i: Self::Item) -> bool;
    /// The item has durability (breakable).
    fn item_breakable(&self, i: Self::Item) -> bool;
    /// The equipped shield.
    fn shield(&self, u: Self::Unit) -> Option<Self::Item>;
    /// armor `mindam` / `maxdam` (+0xFE, +0xFF) of the item's class;
    /// `None` without an item record.
    fn shield_damage(&self, i: Self::Item) -> Option<(i32, i32)>;
    /// weapons `missiletype` (+0xFA) of the item's class.
    fn item_missile_type(&self, i: Self::Item) -> i32;
    /// Iron Golem start: an item unit in mode 3
    /// whose class has `bitfield1` bit 1, with item flag 0x10, not active
    /// on a unit (Open question 9).
    fn golem_item(&self, t: Self::Unit) -> bool;
    /// An item unit's first allowed body location.
    fn item_first_loc(&self, t: Self::Unit) -> i32;
    /// Body locations 6 and 5 (the two hands) both hold an item of item type
    /// 46.
    fn two_melee_weapons(&self, u: Self::Unit) -> bool;
    /// Attack frames (AnimData); `None` = fatal.
    fn attack_frames(&self, u: Self::Unit, w: Self::Item) -> Option<i32>;

    // ---- batch 4
    /// The 64-step direction from the unit's
    /// position to (x, y).
    fn dir64(&self, u: Self::Unit, at: (i32, i32)) -> i32;
    /// Unit +0x4E (action frame).
    fn action_frame(&self, u: Self::Unit) -> i32;
    fn set_action_frame(&mut self, u: Self::Unit, v: i32);
    /// Unit +0x3C := v (sequence speed).
    fn set_seq_speed(&mut self, u: Self::Unit, v: i32);
    /// Unit +0x4C (animation speed; set).
    fn anim_speed(&self, u: Self::Unit) -> i32;
    fn set_anim_speed(&mut self, u: Self::Unit, v: i32);
    /// A missile's lifetime, in base ticks.
    fn missile_frames(&self, m: Self::Unit) -> i32;
    /// Sets a missile's lifetime and time left, in base ticks.
    fn set_missile_frames(&mut self, m: Self::Unit, total: i32, left: i32);
    /// The unit's running sequence: its frame count
    /// (8.8 fixed point); `None` without a sequence.
    fn sequence_frames(&self, u: Self::Unit) -> Option<i32>;
    /// The sequence event at frame `f`
    /// (8.8).
    fn sequence_event(&self, u: Self::Unit, f: i32) -> i32;
    /// The unit's AnimData record (+0x50): frame count (+0x08) and event
    /// bytes (+0x10 …); `None` without one.
    fn anim_data(&self, u: Self::Unit) -> Option<(u32, Vec<u8>)>;
    /// An action event (1 or 2) lies in the frames (a, b].
    fn action_event_between(&self, u: Self::Unit, a: i32, b: i32) -> bool;
    /// The monstats chain position (+0x4B,
    /// fixups); 0 for an invalid class.
    fn chain_position(&self, class: i32) -> i32;
    /// The class of `class`'s chain for the
    /// room's level.
    fn class_for_level(&self, room: Option<Self::Room>, class: i32) -> i32;
    /// The books row of an item's spell index (item data +0x3E,
    /// its spell row): (`scrollskill`, `bookskill`).
    fn book_skills(&self, i: Self::Item) -> Option<(i32, i32)>;
    /// The unit's inventory nodes in list order (first, next
    /// link): each node's item and kind.
    fn inventory_nodes(&self, u: Self::Unit) -> Vec<(Self::Item, i32)>;
    /// The unit find (default filter)
    /// around `at` with radius `r` and flags
    /// `f` over the neighbourhood of `room`: the found units in order.
    fn unit_find(&self, room: Self::Room, at: (i32, i32), r: i32, f: u32) -> Vec<Self::Unit>;
    /// The point collides with `mask`.
    fn point_collides(&self, room: Self::Room, at: (i32, i32), mask: u32) -> bool;
    /// A monster creation entry point.
    fn spawn_monster(&mut self, q: MonsterSpawn<Self::Unit, Self::Room>) -> Option<Self::Unit>;
    /// Places the unit with its sixth placement argument
    /// `a`: true when placed.
    fn place_unit_flag(
        &mut self,
        u: Self::Unit,
        r: Option<Self::Room>,
        at: (i32, i32),
        a: i32,
    ) -> bool;
    /// Monster data component byte k (+0x04 + k);
    /// 0 without monster data.
    fn component(&self, u: Self::Unit, k: usize) -> i32;
}

/// The monster creation entry points the batch 4 bodies call
/// (the creation entry table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterSpawn<U, R> {
    At {
        room: R,
        x: i32,
        y: i32,
        class: i32,
        mode: i32,
        spread: i32,
        flags: u32,
    },
    /// Near the unit.
    Near {
        unit: U,
        class: i32,
        mode: i32,
        spread: i32,
        flags: u32,
    },
    /// The class
    /// remapped for the unit's level, then near the unit.
    NearLevel {
        unit: U,
        class: i32,
        mode: i32,
        spread: i32,
        flags: u32,
    },
    /// One
    /// spawn with owner data (a pattern leader).
    Leader {
        room: R,
        x: i32,
        y: i32,
        class: i32,
        mode: i32,
        spread: i32,
    },
    /// The
    /// leader's minion.
    Minion {
        leader: U,
        x: i32,
        y: i32,
        class: i32,
        mode: i32,
        spread: i32,
    },
}

/// `srvst[index](game, unit, skill, level)` for a slot of
/// [`START_BODIES`]; `None` for any other slot.
pub fn run_start<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    index: u16,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> Option<i32> {
    use b3_lvl01 as l1;
    use b3_lvl06 as l6;
    use b3_lvl12 as l12;
    use b3_lvl18 as l18;
    use b3_lvl24 as l24;
    use b3_lvl30 as l30;
    use b4_mon as m4;
    use b4_more as x4;
    use starts::*;
    use starts2 as s2;
    Some(match index {
        1 => attack(w, t, ct, u, lvl),
        2 => kick(w, t, ct, u, skill),
        3 => unsummon(w, u),
        4 | 65 => ammo(w, u),
        5 => jab(w, t, u, skill),
        6 => s2::power_strike(w, t, ct, u, skill, lvl),
        7 => l12::impale(w, t, ct, u, skill, lvl),
        8 => l24::strafe_start(w, t, ct, u, skill, lvl),
        9 => l24::fend_start(w, t, ct, u, skill, lvl),
        10 => l30::lightning_strike_start(w, t, ct, u, skill, lvl),
        11 => s2::inferno(w, t, ct, u, skill, lvl),
        12 => s2::telekinesis(w, t, u, skill, lvl),
        13 => l24::thunder_storm_start(w, t, u, skill),
        14 => l30::hydra_start(w, u),
        15 => raise(w, ct, u),
        16 => l6::poison_dagger_start(w, t, ct, u, skill, lvl),
        17 => s2::corpse_explosion(w, ct, u),
        // The pure body of [`start`].
        18 => 1,
        19 => l24::bone_prison_start(w, u),
        20 => l24::iron_golem_start(w, u),
        21 => l30::revive_start(w, ct, u),
        22 => l1::psychic_hammer_start(w, u),
        23 => s2::charge_strike(w, u),
        24 => l1::dragon_talon_start(w, t, ct, u, skill, lvl),
        25 => l6::dragon_claw_start(w, u),
        26 => l18::blade_fury_start(w, t, ct, u, skill, lvl),
        27 => l18::dragon_tail_start(w, t, ct, u, skill, lvl),
        28 => l30::blade_shield_start(w, t, ct, u, skill, lvl),
        29 => sacrifice(w, t, ct, u, skill, lvl),
        31 => l12::charge_start(w, t, ct, u, skill),
        32 => bash(w, t, ct, u, skill, lvl),
        33 => find_potion(w, ct, u),
        34 => l12::find_item_start(w, ct, u),
        35 => l18::vengeance(w, t, ct, u, skill, lvl),
        36 => l24::holy_shield_start(w, u),
        37 => s2::zeal(w, t, ct, u, skill, lvl),
        38 => l30::whirlwind_start(w, t, ct, u, skill, lvl),
        39 => l30::berserk(w, t, ct, u, skill, lvl),
        40 => l6::leap_start(w, t, ct, u, skill, lvl),
        41 => l18::leap_attack_start(w, u, skill),
        42 => m4::fire_hit_start(w, t, ct, u, skill, lvl),
        43 => m4::maggot_egg_start(w, u),
        44 => m4::maggot_up(w, u),
        45 => m4::maggot_down_start(w, u),
        46 => andrial_spray(w, u),
        47 => m4::jump_start(w, t, ct, u, skill, lvl),
        48 => m4::swarm_move_start(w, u),
        49 => m4::nest_start(w, t, ct, u, skill),
        50 => m4::quick_strike_start(w, t, ct, u),
        51 => m4::submerge_start(w, u),
        52 => m4::emerge_start(w, u),
        53 => m4::mon_inferno_start(w, t, u, skill, lvl, false),
        54 => m4::diab_run_start(w, u),
        55 => x4::mosquito_start(w, t, u, skill, lvl),
        56 => s2::feral_rage(w, t, ct, u, skill, lvl),
        57 => l18::rabies_start(w, t, ct, u, skill, lvl),
        58 => l18::fire_claws(w, t, ct, u, skill, lvl),
        59 => x4::imp_inferno_start(w, t, u, skill, lvl),
        60 => x4::suck_blood_start(w, t, ct, u, skill, lvl),
        61 => x4::self_resurrect(w, u),
        62 => x4::minion_spawner_start(w, t, ct, u, skill),
        63 => m4::corpse_cycler(w, t, u, skill, lvl),
        64 => m4::mon_frenzy_start(w, u),
        _ => return None,
    })
}

/// `srvdo[index](game, unit, skill, level)` for a slot of
/// [`DO_BODIES`]; `None` for any other slot. The do core's `charge`,
/// `item` and `aim` are not arguments of the bodies.
pub fn run_do<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    index: u16,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> Option<i32> {
    use b3_lvl01 as l1;
    use b3_lvl06 as l6;
    use b3_lvl12 as l12;
    use b3_lvl18 as l18;
    use b3_lvl24 as l24;
    use b3_lvl30 as l30;
    use b4_helpers::throw;
    use b4_mon as m4;
    use b4_more as x4;
    use dos::*;
    use dos2 as d2;
    Some(match index {
        1 => attack(w, t, ct, u, skill, lvl),
        2 => melee_state(w, t, ct, u, skill, lvl),
        3 => throw(w, t, u, skill, lvl, false),
        4 => m4::unsummon_do(w, u),
        5 => throw(w, t, u, skill, lvl, true),
        6 => d2::inner_sight(w, t, ct, u, skill, lvl),
        7 => l1::jab(w, t, ct, u, skill, lvl),
        8 => d2::multiple_shot(w, t, u, skill, lvl),
        9 => l24::frenzy(w, t, ct, u, skill, lvl),
        10 => d2::guided_arrow(w, t, u, skill, lvl),
        11 => l18::charged_strike(w, t, ct, u, skill, lvl),
        12 => l24::strafe(w, t, ct, u, skill, lvl),
        13 => d2::fend(w, t, ct, u, skill, lvl),
        14 => l30::lightning_strike(w, t, ct, u, skill, lvl),
        15 => l24::dopplezon(w, t, ct, u, skill, lvl),
        16 => l30::valkyrie(w, t, ct, u, skill, lvl),
        17 => l1::charged_bolt(w, t, u, skill, lvl),
        18 => buff(w, t, ct, u, skill, lvl),
        19 => d2::inferno_cast(w, t, ct, u, skill, lvl),
        20 => l6::static_field(w, t, ct, u, skill, lvl),
        21 => l6::telekinesis(w, t, ct, u, skill, lvl),
        22 => d2::nova(w, t, u, skill, lvl),
        23 => d2::blaze(w, t, ct, u, skill, lvl),
        24 => l18::fire_wall(w, t, u, skill, lvl),
        25 => l18::enchant(w, t, ct, u, skill, lvl),
        26 => l18::chain_lightning(w, t, u, skill, lvl),
        27 => l18::teleport(w, u),
        28 => d2::meteor(w, t, u, skill, lvl),
        29 => l24::thunder_storm(w, t, ct, u, skill, lvl),
        30 => curse(w, t, ct, u, skill, lvl),
        31 => d2::raise_skeleton(w, t, ct, u, skill, lvl),
        32 => l6::poison_dagger(w, ct, u),
        33 => l1::psychic_hammer(w, t, ct, u, skill, lvl),
        34 => d2::charge_hit(w, t, ct, u, skill, lvl),
        35 => d2::claws(w, t, ct, u, skill, lvl),
        36 => x4::thunder_ring(w, t, u, skill, lvl),
        37 => x4::lightning(w, t, u, skill, lvl, true),
        38 => x4::prog_burst(w, t, ct, u, skill, lvl),
        39 => x4::prog_scatter(w, t, u, skill, lvl),
        40 => x4::royal_meteor(w, t, u, skill, lvl),
        41 => x4::royal_shards(w, t, u, skill, lvl),
        42 => l1::dragon_talon(w, t, ct, u, skill, lvl),
        43 => l6::shock_field(w, t, u, skill, lvl),
        44 => l6::blade_sentinel(w, t, ct, u, skill, lvl),
        45 => d2::sentry_do(w, t, ct, u, skill, lvl),
        46 => l6::dragon_claw(w, t, ct, u, skill, lvl),
        47 => l12::cloak(w, t, ct, u, skill, lvl),
        48 => l18::blade_fury(w, t, ct, u, skill, lvl),
        49 => d2::shadow(w, t, ct, u, skill, lvl),
        50 => l18::dragon_tail(w, t, ct, u, skill, lvl),
        51 => l24::mind_blast(w, t, ct, u, skill, lvl),
        52 => l24::dragon_flight(w, t, ct, u, skill, lvl),
        54 => l30::blade_shield(w, t, ct, u, skill, lvl),
        55 => l6::corpse_explosion(w, t, ct, u, skill, lvl),
        56 => d2::golem(w, t, ct, u, skill, lvl),
        57 => l24::iron_golem(w, t, ct, u, skill, lvl),
        58 => l30::revive(w, t, ct, u, skill, lvl),
        59 => l24::attract(w, t, ct, u, skill, lvl),
        60 => l12::bone_wall(w, t, ct, u, skill, lvl),
        61 => l18::confuse(w, t, ct, u, skill, lvl),
        62 => l24::bone_prison(w, t, ct, u, skill, lvl),
        63 => l18::poison_explosion(w, t, ct, u, skill, lvl),
        64 => l1::sacrifice(w, t, ct, u, skill, lvl),
        65 => aura(w, t, ct, u, skill, lvl),
        66 => d2::damage_aura(w, t, ct, u, skill, lvl, false),
        67 => l12::charge(w, t, ct, u, skill, lvl),
        68 => d2::shout(w, t, u, skill, lvl),
        69 => l1::find_potion(w, t, ct, u, skill, lvl),
        70 => l6::double_swing(w, t, ct, u, skill, lvl),
        71 => l6::taunt(w, t, ct, u, skill, lvl),
        72 => l12::find_item(w, t, ct, u, skill, lvl),
        73 => l18::blessed_hammer(w, t, u, skill, lvl),
        74 => l12::double_throw(w, t, u, skill, lvl),
        75 => l24::grim_ward(w, t, ct, u, skill, lvl),
        76 => l30::whirlwind(w, t, ct, u, skill, lvl),
        77 => l6::leap(w, t, ct, u, skill, lvl),
        78 => l18::leap_attack(w, t, ct, u, skill, lvl),
        79 => l24::conversion_do(w, t, ct, u, skill, lvl),
        80 => l30::fist_of_heavens(w, t, u, skill, lvl),
        81 => d2::damage_aura(w, t, ct, u, skill, lvl, true),
        82 => l30::redemption(w, t, ct, u, skill, lvl),
        83 => m4::fire_hit(w, t, ct, u, skill, lvl),
        84 => m4::maggot_egg(w, t, ct, u, skill, lvl),
        85 => m4::family_bolt(w, t, u, skill, lvl),
        86 => m4::maggot_down(w, t, u, skill, lvl),
        87 => m4::maggot_lay(w, t, ct, u, skill),
        88 => m4::andrial_spray(w, t, u, skill, lvl),
        89 => m4::jump(w, ct, u),
        90 => m4::swarm_move(w, t, u, skill, lvl),
        91 => m4::nest(w, t, u, skill),
        92 => m4::quick_strike(w, t, ct, u, skill, lvl),
        93 => m4::gargoyle_trap(w, t, u, skill, lvl),
        94 => m4::submerge(w, u),
        95 | 152 => m4::mon_inferno(w, t, ct, u, skill, lvl),
        96 => m4::zakarum_heal(w, t, u, skill, lvl),
        97 => m4::resurrect(w, t, ct, u, skill),
        98 => m4::mon_teleport(w, u),
        99 => m4::prime_poison_nova(w, t, u, skill, lvl),
        100 => m4::diab_cold(w, t, ct, u, skill, lvl),
        101 => m4::finger_mage_spider(w, t, u, skill, lvl),
        102 => m4::diab_wall(w, t, u, skill, lvl),
        103 => m4::diab_run(w, t, ct, u, skill, lvl),
        104 => m4::diab_prison(w, t, ct, u, skill),
        105 => m4::desert_turret(w, t, u, skill, lvl),
        106 => m4::arcane_tower(w, t, u, skill, lvl),
        107 => x4::mosquito(w, t, ct, u, skill, lvl),
        108 => x4::regurgitator_eat(w, t, u, skill, lvl),
        109 => m4::mon_frenzy(w, t, ct, u, skill, lvl),
        110 => m4::hireable_missile(w, t, u, skill, lvl),
        111 => m4::fetish_aura(w, t, ct, u, skill, lvl),
        112 => x4::mon_curse_cast(w, t, ct, u, lvl),
        113 => m4::scroll_book(w, u, skill),
        114 => l1::raven(w, t, ct, u, skill, lvl),
        115 => d2::vines(w, t, ct, u, skill, lvl),
        116 => d2::shape_shift(w, t, u, skill, lvl),
        117 => l1::firestorm(w, t, u, skill, lvl),
        118 => d2::twister(w, t, u, skill, lvl),
        119 => d2::druid_summon(w, t, ct, u, skill, lvl),
        120 => d2::feral_rage(w, t, ct, u, skill, lvl),
        121 => l18::rabies(w, t, ct, u, skill, lvl),
        122 => l24::hunger(w, t, ct, u, skill, lvl),
        123 => l24::volcano(w, t, u, skill, lvl),
        124 => d2::armageddon(w, t, u, skill, lvl),
        125 => x4::wake_of_destruction(w, t, u, skill, lvl),
        126 => x4::imp_inferno(w, t, ct, u, skill, lvl),
        127 => x4::suck_blood(w, t, ct, u, skill, lvl),
        128 => x4::cry_help(w, t, u, skill, lvl),
        129 => x4::imp_teleport(w, t, ct, u, skill, lvl),
        130 => x4::vine_attack(w, t, u, skill, lvl),
        131 => x4::overseer_whip(w, t, ct, u, skill, lvl),
        132 => x4::imp_fire_missile(w, t, u, skill, lvl),
        133 => x4::impregnate(w, t, ct, u, skill, lvl),
        134 => x4::siege_stomp(w, t, ct, u, skill, lvl),
        135 => x4::minion_spawner(w, t, u, skill),
        136 => x4::death_maul(w, t, u, skill, lvl),
        137 => x4::fenris_rage(w, t, ct, u, skill, lvl),
        139 => x4::baal_cold_missiles(w, t, u, skill, lvl),
        140 => x4::baal_tentacle(w, ct, u),
        141 => x4::baal_corpse_explode(w, t, ct, u, skill, lvl),
        143 => x4::lightning(w, t, u, skill, lvl, false),
        144 => l30::hydra(w, t, ct, u, skill, lvl),
        145 => x4::hurricane_state(w, t, ct, u, skill, lvl),
        146 => x4::armageddon_state(w, t, u, skill, lvl),
        147 => x4::attached_state(w, u),
        148 => x4::component_missile(w, t, u, skill, lvl, 10),
        149 => x4::component_missile(w, t, u, skill, lvl, 11),
        150 => l1::smite(w, t, ct, u, skill, lvl),
        151 => x4::chain_lightning_item(w, t, u, skill, lvl),
        _ => return None,
    })
}

/// A do slot called from a body (the progressive finisher, its step
/// 4): a body here, else the provider's seam (with the core's
/// arguments off).
pub(crate) fn do_slot<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    index: u16,
    u: W::Unit,
    skill: i32,
    lvl: i32,
) -> i32 {
    match run_do(w, t, ct, index, u, skill, lvl) {
        Some(v) => v,
        None => w.srvdo(index, u, skill, lvl, false, false, false),
    }
}

/// The remove callback `cb` of a detached list `l` (ECX unit, EDX state,
/// stack list; the batch 1 to 3 callbacks). Returns
/// false for an id the bodies do not specify (nothing runs).
pub fn remove_callback<W: BodyWorld>(
    w: &mut W,
    t: &SkillTables,
    ct: &CombatTables,
    u: W::Unit,
    state: i32,
    cb: u32,
    l: W::List,
) -> bool {
    match cb {
        callback::DEFAULT => remove_default(w, u, state),
        callback::SELF_AURA => remove_self_aura(w, t, u, state),
        callback::BUFF => remove_buff(w, u, state),
        callback::AI_CURSE => remove_ai_curse(w, u, state),
        callback::CHARGE => w.state_on(u, state, false),
        callback::INFERNO | callback::BLADE_FURY => {
            w.state_on(u, state, false);
            flags_or(w, u, FLAG_40);
        }
        callback::SHAPE => helpers3::remove_shape(w, u, state, l),
        callback::WHIRLWIND => remove_buff(w, u, state),
        callback::HOLY_FREEZE => helpers3::remove_holy_freeze(w, u, state),
        callback::CONFUSE | callback::ATTRACT => helpers3::remove_alignment(w, u, state),
        callback::CONVERSION => helpers3::remove_conversion(w, u, state, false),
        callback::MIND_BLAST => helpers3::remove_conversion(w, u, state, true),
        callback::PREGNANT => b4_more::remove_pregnant(w, t, ct, u, state, l),
        _ => return false,
    }
    true
}
