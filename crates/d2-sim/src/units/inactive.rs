//! Inactive-unit storage: what happens to each unit of a room being
//! deactivated (store a record, detach or free) and
//! the per-act area-node lists of records that the room's next
//! population restores.
//!
//! The rules are pure: the caller gathers the unit's facts
//! ([`CompressFacts`], [`MonsterFacts`]), carries out the returned
//! [`Compress`] (record, detach, free) and, at restore, re-creates the
//! units of the records [`InactiveStore::take`] hands back, in
//! [`RestoreOrder`] order, with the restore filters
//! ([`restore_monster`], [`restore_item_expiry`]).

use crate::units::UnitType;

/// Monster mode 12 (dead) and 1 (neutral).
pub const MODE_DEAD: u32 = 12;
pub const MODE_NEUTRAL: u32 = 1;
/// Object classes kept like a player body (`Portal`, see [`compress_other`]).
pub const PORTAL_CLASSES: [u32; 2] = [59, 60];
/// Monster classes never stored dead.
pub const NO_STORE_DEAD_CLASSES: [u32; 2] = [0x16B, 0x16C];
/// Unit flags (+0xC4) read by the deactivation.
pub const UNIT_FLAG_200: u32 = 0x200;
pub const UNIT_FLAG_2000000: u32 = 0x200_0000;
pub const UNIT_FLAG_BIT31: u32 = 0x8000_0000;
/// Flags 2 (+0xC8) bit set on a kept unit.
pub const FLAGS2_KEPT: u32 = 0x100;
/// Ground expiry added at restore.
pub const RESTORE_EXPIRY: i32 = 15_000;
/// Level 108 (Chaos Sanctuary) and its only restored class.
pub const LEVEL_SANCTUARY: u32 = 108;
pub const SANCTUARY_CLASS: u32 = 243;

/// What the store does with one unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compress {
    /// Cancel the unit's type-1 events first.
    pub cancel_events: bool,
    /// flags 2 |= 0x100 (a kept unit).
    pub mark_kept: bool,
    /// Store a record.
    pub store: bool,
    /// Detach (kept: footprint stays, not freed).
    pub detach: bool,
    /// Free. An item store frees the item itself.
    pub free: bool,
    /// Monster rule 3 for a living monster in a player target list (a
    /// merc or summon): mode set to 1 and its monster data freed.
    pub neutral_mode: bool,
}

/// The deactivation facts for a unit of type other than monster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressFacts {
    pub ty: UnitType,
    pub class: u32,
    pub mode: u32,
    /// `S`: the room's level `SaveMonsters` ≠ 0 or unit flag 0x2000000.
    pub save: bool,
    /// Player with state 7 (`playerbody`).
    pub player_body: bool,
    /// Objects: `Restore` (+0x173) = 0.
    pub object_no_restore: bool,
    /// Objects: unit byte +0x78 has 0x2.
    pub object_byte78_2: bool,
    /// Objects: `RestoreVirgins` (+0x174) ≠ 0.
    pub object_restore_virgins: bool,
}

/// `S` of the deactivation: `SaveMonsters` or unit flag 0x2000000.
pub fn save_flag(save_monsters: bool, unit_flags: u32) -> bool {
    save_monsters || unit_flags & UNIT_FLAG_2000000 != 0
}

/// The deactivation table for players, objects, missiles, items and tiles.
/// Monsters: [`compress_monster`].
pub fn compress_other(f: &CompressFacts) -> Compress {
    let kept = Compress {
        cancel_events: true,
        mark_kept: true,
        store: true,
        detach: true,
        ..Compress::default()
    };
    match f.ty {
        UnitType::Player if f.player_body => kept,
        UnitType::Player => Compress {
            store: f.save,
            free: true,
            ..Compress::default()
        },
        UnitType::Object if PORTAL_CLASSES.contains(&f.class) => kept,
        UnitType::Object => {
            let cleared = f.object_no_restore
                || f.object_byte78_2
                || (f.object_restore_virgins && f.mode != 0);
            Compress {
                cancel_events: true,
                store: f.save && !cleared,
                free: true,
                ..Compress::default()
            }
        }
        UnitType::Missile => Compress {
            free: true,
            ..Compress::default()
        },
        UnitType::Item => Compress {
            store: true,
            ..Compress::default()
        },
        UnitType::Tile => Compress {
            store: true,
            free: true,
            ..Compress::default()
        },
        UnitType::Monster => Compress::default(),
    }
}

/// The facts of the monster rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonsterFacts {
    /// `S`.
    pub save: bool,
    pub mode: u32,
    /// Dead.
    pub dead: bool,
    /// The unit has a `udead` state.
    pub udead_state: bool,
    /// Alignment.
    pub alignment: i32,
    /// Node index (+0xD0).
    pub node_index: u32,
    pub class: u32,
    /// Monster type flags (monster data +0x16).
    pub type_flags: u32,
    /// Unit flags (+0xC4).
    pub unit_flags: u32,
    /// Rule 6: the owner is a player and its pet test holds.
    pub player_pet: bool,
    /// monstats2 `restore` (+0x130); `None`: no row.
    pub restore: Option<u8>,
}

/// The monster rule. Rule 1 calls
/// `room_step` once for a monster in mode 12: one step of the room's
/// seed (room +0x6C), its new low word (`None`: no seed, K := 0).
pub fn compress_monster(f: &MonsterFacts, room_step: impl FnOnce() -> Option<u32>) -> Compress {
    let mut k = f.save;
    let mut out = Compress::default();
    // Rule 1.
    if f.mode == MODE_DEAD {
        k = k && room_step().is_some_and(|lo| lo % 3 == 0);
    }
    // Rule 2.
    if f.dead && (f.udead_state || f.alignment == 2) {
        k = false;
    }
    // Rule 3.
    let mut skip4 = false;
    if crate::game::node::player_list(f.node_index).is_some() {
        if f.dead {
            k = false;
        } else {
            k = true;
            out.neutral_mode = true;
            skip4 = true;
        }
    }
    // Rule 4.
    if !skip4 && f.dead && NO_STORE_DEAD_CLASSES.contains(&f.class) {
        k = false;
    }
    // Rule 5.
    if f.type_flags & 0x18 != 0 && f.dead {
        k = false;
    }
    // Rule 6.
    let p = f.unit_flags & UNIT_FLAG_BIT31 != 0 && f.player_pet;
    if f.unit_flags & UNIT_FLAG_BIT31 != 0 {
        k = false;
    }
    // Rule 7.
    if f.unit_flags & UNIT_FLAG_200 != 0 {
        k = false;
    }
    // Rule 8.
    match f.restore {
        None | Some(0) => k = false,
        Some(2) => k = true,
        _ => {}
    }
    // Rule 9.
    out.store = k;
    if p {
        out.mark_kept = true;
        out.detach = true;
    } else {
        out.free = true;
    }
    out
}

/// A stored monster (0x5C bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterRecord {
    pub x: i32,
    pub y: i32,
    pub class: u32,
    pub guid: u32,
    pub unit_flags: u32,
    pub flags_ex: u32,
    /// +0x18 bits ([`mrec`]).
    pub bits: u32,
    /// +0x1C owner GUID (−1 none) and +0x20 its value.
    pub owner_guid: i32,
    pub owner_value: i32,
    /// +0x28 AI special state.
    pub ai_state: i32,
    /// +0x2C level id.
    pub level: u32,
    /// +0x30 name seed (0x1506 without monster data).
    pub name_seed: u16,
    /// +0x32..+0x3A umods.
    pub umods: [u8; 9],
    /// +0x3C superunique index.
    pub superunique: u16,
    /// +0x40 stat 13, +0x44 max life, +0x48 life.
    pub stat13: i32,
    pub max_life: i32,
    pub life: i32,
    /// +0x54 game frame.
    pub frame: i32,
    /// +0x4C and +0x50: the NPC home point (AI command 10 params 1, 2),
    /// or the flag markers of the store (unit flags 0x4000000 /
    /// 0x20000: `x4c` = [`MARKER`], `x50` bits 1 / 2).
    pub x4c: u32,
    pub x50: u32,
    /// +0x24: the map-AI path (action, x, y) moved out of the unit's AI
    /// control by the store.
    pub path: Option<Vec<(i32, i32, i32)>>,
}

/// `DAT_007314C0`: the +0x4C marker of the unit-flag records
/// (the base game's value); `DAT_007314C4` / `DAT_007314C8`
/// are the +0x50 bits 1 and 2.
pub const MARKER: u32 = 0xDEAD_DEAD;

/// Bits of [`MonsterRecord::bits`] (+0x18).
pub mod mrec {
    pub const TYPE_FLAG_1: u32 = 0x1;
    pub const CHAMPION: u32 = 0x2;
    pub const DEAD: u32 = 0x4;
    pub const OWNER_8: u32 = 0x8;
    pub const OWNER_10: u32 = 0x10;
    pub const MINION: u32 = 0x20;
    pub const UNIT_573540: u32 = 0x40;
    pub const ALIGN_1: u32 = 0x80;
    pub const ALIGN_2: u32 = 0x100;
    pub const ALIGN_0: u32 = 0x200;
    pub const NODE_NOT_11: u32 = 0x400;
    pub const SUPERUNIQUE: u32 = 0x800;
}

/// The +0x18 bits from their facts.
#[allow(clippy::too_many_arguments)]
pub fn monster_bits(
    type_flag_1: bool,
    champion: bool,
    mode: u32,
    owner_bits: u32,
    minion: bool,
    unit_573540: bool,
    alignment: i32,
    node_index: u32,
    superunique: bool,
) -> u32 {
    let mut b = owner_bits & (mrec::OWNER_8 | mrec::OWNER_10);
    let set = |b: &mut u32, on: bool, bit: u32| {
        if on {
            *b |= bit;
        }
    };
    set(&mut b, type_flag_1, mrec::TYPE_FLAG_1);
    set(&mut b, champion, mrec::CHAMPION);
    set(&mut b, mode == MODE_DEAD || mode == 0, mrec::DEAD);
    set(&mut b, minion, mrec::MINION);
    set(&mut b, unit_573540, mrec::UNIT_573540);
    set(&mut b, alignment == 1, mrec::ALIGN_1);
    set(&mut b, alignment == 2, mrec::ALIGN_2);
    set(&mut b, alignment == 0, mrec::ALIGN_0);
    set(&mut b, node_index != 11, mrec::NODE_NOT_11);
    set(&mut b, superunique, mrec::SUPERUNIQUE);
    b
}

/// A stored item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemRecord {
    /// Ground expiry (absolute frame, 0 = never).
    pub expiry: i32,
    pub value_629f20: u32,
    /// The item bit stream in save form, then each
    /// socketed child's record.
    pub bytes: Vec<u8>,
}

/// Any other stored unit (0x34 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OtherRecord {
    pub x: i32,
    pub y: i32,
    pub ty: u8,
    pub class: u32,
    pub mode: u32,
    pub frame: i32,
    pub unit_flags: u32,
    pub flags_ex: u32,
    /// +0x20 and +0x24 (GUID, event time halves, or byte +0x78).
    pub v20: u32,
    pub v24: u32,
    /// +0x28 object-data byte +4, +0x2C unit +0xB8.
    pub byte4: u8,
    pub b8: u32,
}

/// One area node (0x18 bytes): the DRLG room's origin and its three
/// record lists, each newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AreaNode {
    pub x: i32,
    pub y: i32,
    pub items: Vec<ItemRecord>,
    pub monsters: Vec<MonsterRecord>,
    pub others: Vec<OtherRecord>,
}

/// The per-act node lists (game +0xD8 + 4·act), descending x.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InactiveStore {
    pub acts: [Vec<AreaNode>; 5],
}

impl InactiveStore {
    /// The node of the room at (`x`, `y`) in `act`, created when missing:
    /// a new node goes before the first node of smaller x.
    pub fn node_mut(&mut self, act: u8, x: i32, y: i32) -> &mut AreaNode {
        let list = &mut self.acts[usize::from(act.min(4))];
        if let Some(i) = list.iter().position(|n| n.x == x && n.y == y) {
            return &mut list[i];
        }
        let at = list.iter().position(|n| n.x < x).unwrap_or(list.len());
        list.insert(
            at,
            AreaNode {
                x,
                y,
                ..AreaNode::default()
            },
        );
        &mut list[at]
    }

    /// Every store pushes at the head of its list.
    pub fn push_monster(&mut self, act: u8, room: (i32, i32), r: MonsterRecord) {
        self.node_mut(act, room.0, room.1).monsters.insert(0, r);
    }
    pub fn push_item(&mut self, act: u8, room: (i32, i32), r: ItemRecord) {
        self.node_mut(act, room.0, room.1).items.insert(0, r);
    }
    pub fn push_other(&mut self, act: u8, room: (i32, i32), r: OtherRecord) {
        self.node_mut(act, room.0, room.1).others.insert(0, r);
    }

    /// Restore's unlink of the room's node; `None`: no
    /// records for that room.
    pub fn take(&mut self, act: u8, x: i32, y: i32) -> Option<AreaNode> {
        let list = &mut self.acts[usize::from(act.min(4))];
        let i = list.iter().position(|n| n.x == x && n.y == y)?;
        Some(list.remove(i))
    }
}

/// How a monster record comes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonsterRestore {
    /// Skipped (flag 0x400 with alignment 1 or 2; level 108 non-243).
    Skip,
    /// flag-ex 0x100: re-place the existing unit of the GUID and re-link
    /// its owner.
    Replace,
    /// Spawn again with the stored GUID, through the minion spawn
    /// The unique spawn or the plain one.
    Spawn { kind: SpawnKind, mode: u32 },
}

/// The spawn routine of a re-spawned monster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    Minion,
    Unique,
    Plain,
}

/// For one record. `sanctuary_gate`: the level is 108 and
/// Holds.
pub fn restore_monster(r: &MonsterRecord, sanctuary_gate: bool) -> MonsterRestore {
    if sanctuary_gate && r.class != SANCTUARY_CLASS {
        return MonsterRestore::Skip;
    }
    if r.flags_ex & FLAGS2_KEPT != 0 {
        return MonsterRestore::Replace;
    }
    if r.bits & mrec::NODE_NOT_11 != 0 && r.bits & (mrec::ALIGN_1 | mrec::ALIGN_2) != 0 {
        return MonsterRestore::Skip;
    }
    let kind = if r.bits & mrec::MINION != 0 {
        SpawnKind::Minion
    } else if r.bits & mrec::TYPE_FLAG_1 != 0 {
        SpawnKind::Unique
    } else {
        SpawnKind::Plain
    };
    let mode = if r.bits & mrec::DEAD != 0 {
        MODE_DEAD
    } else {
        MODE_NEUTRAL
    };
    MonsterRestore::Spawn { kind, mode }
}

/// The restored item's ground expiry, or `None` when the
/// record is dropped (stored expiry ≠ 0 and < the current frame).
pub fn restore_item_expiry(stored: i32, frame: i32) -> Option<i32> {
    restore_item_expiry_for(stored, frame, RESTORE_EXPIRY)
}

/// [`restore_item_expiry`] with the renewal span given in ticks at the
/// game's rate.
pub fn restore_item_expiry_for(stored: i32, frame: i32, span: i32) -> Option<i32> {
    if stored != 0 && stored < frame {
        return None;
    }
    let renewed = frame.wrapping_add(span);
    Some(if stored == 0 || stored >= renewed {
        stored
    } else {
        renewed
    })
}

/// The restore order: monsters, then items, then other
/// records, each list from its head (the reverse of store order).
pub fn restore_order(node: AreaNode) -> RestoreOrder {
    RestoreOrder {
        monsters: node.monsters,
        items: node.items,
        others: node.others,
    }
}

/// A node's records in restore order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreOrder {
    pub monsters: Vec<MonsterRecord>,
    pub items: Vec<ItemRecord>,
    pub others: Vec<OtherRecord>,
}

#[cfg(test)]
mod tests;
