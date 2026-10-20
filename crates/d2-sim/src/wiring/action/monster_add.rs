//! The messages that announce a monster to a client.
//!
//! Part A is the 0xAC assign message, the position stat 328, class 528's 0x98,
//! the skill messages 0x21, the unit states 0xAA and the pending event records.
//! Part B is the mode message, then the inventory messages for state 93 and
//! classes 291, 417 and 418. Sent by the room switch and by the per-unit walk
//! for a monster the client has not been told about yet
//! ([`View::monster_update`]).

use crate::monsters::init::{assign_mode, component_bits, type_flag, BitWriter, MonsterData};
use crate::stats::states::group;
use crate::units::messages;
use crate::units::{UnitId, UnitType};

use super::{Pending, View};

/// Stat 328, the position stat: set to (x + y) & 0xFFFF after 0xAC is sent.
const STAT_POSITION: u16 = 328;
/// Class 528 (evilhut) sends 0x98 after 0xAC.
const CLASS_98: u32 = 528;
/// State 93 and the classes whose part B sends the inventory.
const STATE_INVENTORY: u16 = 93;
const CLASSES_INVENTORY: [u32; 3] = [291, 417, 418];
/// Second unit flag word, bit 0x400: the unit has a source link (owner) to send.
const FLAG_EX_SOURCE: u32 = 0x400;
/// Unit flag 0x200: the type block is sent even with no umods.
const FLAG_TYPE_BLOCK: u32 = 0x200;
/// The most bytes of bit stream a 0xAC message carries.
const STREAM_MAX: usize = 0xF4;

/// The inputs of the 0xAC assign message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssignMonster {
    pub guid: u32,
    pub class: u16,
    pub x: u16,
    pub y: u16,
    /// Life as a fraction of maximum life, scaled to 0..=128.
    pub life: u8,
    /// The unit's mode; `assign_mode` maps it to the mode that is sent.
    pub mode: u32,
    /// The monster data's component choices and the class's choice counts.
    pub components: [u8; 16],
    pub counts: [u8; 16],
    /// The type block, when sent.
    pub type_block: Option<TypeBlock>,
    /// The owner GUID, when the source flag is set and the owner is a player.
    pub source: Option<u32>,
    /// `None` when the unit has no stat list for this; else its first 16 base
    /// entries (stat, param, value) with their itemstatcost send columns
    /// (`None`: the stat is out of range).
    pub stats: Option<Vec<StatEntry>>,
}

/// The type block: champion/unique flags, umods, name seed and hireling owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeBlock {
    /// Type flags 4, 8, 2, 0x10, 0x40.
    pub flags: u16,
    pub hc_idx: u16,
    /// The umod list (bytes ≠ 0 are sent, at most 9).
    pub umods: Vec<u8>,
    pub name_seed: u16,
    /// The hireling owner's GUID (owner a player, pet type 7).
    pub hireling_owner: Option<u32>,
}

/// One base stat entry of the 0xAC message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatEntry {
    pub stat: u16,
    pub param: u16,
    pub value: i32,
    /// (`send other`, `send bits`, `send param bits`); `None` when the
    /// stat has no itemstatcost row.
    pub send: Option<(bool, u8, u8)>,
}

/// Builds the 0xAC message: a 13-byte header followed by a bit stream of the
/// mode, component choices, type block, source link and stat entries.
pub fn assign_monster(a: &AssignMonster) -> Vec<u8> {
    let mut w = BitWriter::new();
    // Mode.
    w.write(u32::from(assign_mode(a.mode)), 4);
    // Component choices.
    let any = a.components.iter().any(|&c| c != 0);
    w.write(u32::from(any), 1);
    if any {
        for (&c, &n) in a.components.iter().zip(&a.counts) {
            w.write(u32::from(c), component_bits(n));
        }
    }
    // Type block.
    w.write(u32::from(a.type_block.is_some()), 1);
    if let Some(t) = &a.type_block {
        for f in [
            type_flag::CHAMPION,
            type_flag::UNIQUE,
            type_flag::SUPERUNIQUE,
            type_flag::MINION,
            type_flag::GHOSTLY,
        ] {
            w.write(u32::from(t.flags & f != 0), 1);
        }
        if t.flags & type_flag::SUPERUNIQUE != 0 {
            w.write(u32::from(t.hc_idx), 16);
        }
        for &u in t.umods.iter().filter(|&&u| u != 0).take(9) {
            w.write(u32::from(u), 8);
        }
        w.write(0, 8);
        w.write(u32::from(t.name_seed), 16);
        match t.hireling_owner {
            Some(g) => {
                w.write(1, 1);
                w.write(g, 32);
            }
            None => w.write(0, 1),
        }
    }
    // Source link.
    match a.source {
        Some(v) => {
            w.write(1, 1);
            w.write(v & 0x8FFF_FFFF, 31);
        }
        None => w.write(0, 1),
    }
    // Stat entries.
    match &a.stats {
        None => w.write(0, 1),
        Some(entries) => {
            let mut sent = false;
            for e in entries.iter().take(16) {
                let Some((other, bits, param_bits)) = e.send else {
                    continue;
                };
                if !other || bits == 0 {
                    continue;
                }
                if !sent {
                    w.write(1, 1);
                    sent = true;
                }
                w.write(u32::from(e.stat), 9);
                if param_bits > 0 {
                    w.write(u32::from(e.param), u32::from(param_bits));
                }
                w.write(e.value as u32, u32::from(bits.min(32)));
            }
            if sent {
                w.write(0x1FF, 9);
            } else {
                w.write(0, 2);
            }
        }
    }
    // The writer keeps at most 0xF4 bytes.
    let mut stream = w.bytes;
    stream.truncate(STREAM_MAX);
    let mut m = Vec::with_capacity(13 + stream.len());
    m.push(0xAC);
    m.extend_from_slice(&a.guid.to_le_bytes());
    m.extend_from_slice(&a.class.to_le_bytes());
    m.extend_from_slice(&a.x.to_le_bytes());
    m.extend_from_slice(&a.y.to_le_bytes());
    m.push(a.life);
    m.push((13 + stream.len()) as u8);
    m.extend_from_slice(&stream);
    m
}

/// The type block is sent when the umod list is non-empty or unit flag 0x200
/// is set.
fn type_block(m: &MonsterData, unit_flags: u32, hireling_owner: Option<u32>) -> Option<TypeBlock> {
    if m.umod_count() == 0 && unit_flags & FLAG_TYPE_BLOCK == 0 {
        return None;
    }
    Some(TypeBlock {
        flags: m.type_flags,
        hc_idx: m.boss_hc_idx,
        umods: m.umod_list().to_vec(),
        name_seed: m.name_seed,
        hireling_owner,
    })
}

impl<X: Pending> View<'_, X> {
    /// The skill messages 0x21 of a monster's add: for each `monstats` slot
    /// i = 0..7 whose bit i of `SendSkills` is set and whose `Skill<i>` is a
    /// skill the unit has: (skill, base level, 0). The unit's skills are its
    /// summon entries (`ActionHooks::monster_skills`) or its init entries
    /// (`ActionHooks::natural_skills`); a monster entry has no bonus
    /// level. An inferno sentry, for example, sends its summoned skill.
    fn monster_sent_skills(&self, unit: UnitId, class: u32) -> Vec<(u16, u8, u8)> {
        let Some(row) = self.h.tables.combat.monstats.get(class as usize) else {
            return Vec::new();
        };
        let slots = [
            row.skill1, row.skill2, row.skill3, row.skill4, row.skill5, row.skill6, row.skill7,
            row.skill8,
        ];
        let levels = self
            .h
            .monster_skills
            .get(&unit)
            .into_iter()
            .chain(self.h.natural_skills.get(&unit));
        let levels: Vec<_> = levels.collect();
        let mut out = Vec::new();
        for (i, &skill) in slots.iter().enumerate() {
            if row.sendskills >> i & 1 == 0 || skill as i16 <= 0 {
                continue;
            }
            let base = levels.iter().find_map(|m| m.get(&i32::from(skill)));
            if let Some(&b) = base {
                out.push((skill, b.clamp(0, 255) as u8, 0));
            }
        }
        out
    }

    /// Sends part A and part B of a monster's add messages to `receiver`'s
    /// client.
    pub fn monster_add(&mut self, game: &crate::game::Game, receiver: UnitId, unit: UnitId) {
        let Some(r) = self.units.get(unit) else {
            return;
        };
        let (class, mode, guid, unit_flags, flags_ex) =
            (r.class, r.mode, r.guid, r.flags, r.flags2);
        let dead = r.is_dead();
        let (x, y) = self.h.path_position(unit);
        // Part A: 0xAC, skipped for a dead unit in a `hide` state.
        if !(dead && self.stats.has_group(unit, group::HIDE)) {
            let m =
                self.assign_monster_input(unit, class, mode, guid, unit_flags, flags_ex, (x, y));
            self.h.x.send(receiver, &assign_monster(&m));
        }
        let pos = (x.wrapping_add(y)) & 0xFFFF;
        self.stats
            .unit_set(&mut *self.h, unit, STAT_POSITION, pos, 0);
        if class == CLASS_98 {
            // The u16 at offset 5 of 0x98 is the AI control's minion spawn
            // class. An evilhut's GenericSpawner init sets it to -1, so a
            // fresh one sends 0xFFFF; a new AI control starts it at zero.
            let spawn = self
                .h
                .ai
                .as_ref()
                .and_then(|s| s.control(unit))
                .map(|c| c.spawn_class)
                .or_else(|| self.h.spawn_class_notes.get(&unit).copied())
                .unwrap_or(0);
            self.h
                .x
                .send(receiver, &messages::unknown98(guid, spawn as u16));
        }
        let mut skills = self.h.x.monster_add_skills(unit);
        if skills.is_empty() {
            skills = self.monster_sent_skills(unit, class);
        }
        for (skill, base, bonus) in skills {
            let m =
                messages::update_oskill(UnitType::Monster as u8, false, guid, skill, base, bonus);
            self.h.x.send(receiver, &m);
        }
        let states = self.unit_states_message(UnitType::Monster as u8, guid, unit);
        self.h.x.send(receiver, &states);
        // The pending event records belong to part A.
        self.send_event_records(game, receiver, unit);
        // Part B. Without the path provider no monster has a path record,
        // so the mode message needs the provider, as [`View::monster_update`]
        // does.
        let client = game
            .lists
            .clients()
            .into_iter()
            .find(|&c| game.lists.client(c).and_then(|e| e.player) == Some(receiver));
        if let Some(client) = client.filter(|_| self.h.paths.is_some()) {
            self.mode_message(game, client, receiver, unit);
        }
        if self.stats.has_state(unit, u32::from(STATE_INVENTORY))
            || CLASSES_INVENTORY.contains(&class)
        {
            self.h.x.inventory_messages(receiver, unit);
            self.monster_inventory_messages(receiver, unit);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_monster_input(
        &mut self,
        unit: UnitId,
        class: u32,
        mode: u32,
        guid: u32,
        unit_flags: u32,
        flags_ex: u32,
        (x, y): (i32, i32),
    ) -> AssignMonster {
        let life = crate::stats::life_fraction(
            self.stats
                .unit_total(unit, crate::stats::stat::HITPOINTS, 0),
            crate::combat::vitals::VitalsUnits::max_life(self, unit),
        ) as u8;
        let hireling_owner = self.h.x.hireling_owner_guid(unit).or_else(|| {
            // A hireling's owner GUID, when its AI control has a player owner.
            self.h
                .hireling_units
                .contains(&unit)
                .then(|| {
                    self.h
                        .ai
                        .as_ref()
                        .and_then(|s| s.control(unit))
                        .and_then(|c| c.minion_owner)
                        .filter(|o| o.ty == UnitType::Player)
                        .map(|o| o.guid)
                })
                .flatten()
        });
        let data = self.h.monster_data(unit);
        let components = data.map_or([0; 16], |d| d.components);
        let type_block = data.and_then(|d| type_block(d, unit_flags, hireling_owner));
        let counts = self
            .h
            .monster_world
            .as_ref()
            .and_then(|w| w.component_counts(class))
            .unwrap_or([0; 16]);
        let source = (flags_ex & FLAG_EX_SOURCE != 0)
            .then(|| {
                // The owner type and GUID the source link writes
                // ([`BodyEffect::SourceFields`]); the host's own record when
                // it keeps one.
                self.h
                    .x
                    .unit_owner(unit)
                    .or_else(|| self.units.get(unit).map(|r| r.source))
            })
            .flatten()
            .and_then(|(ty, g)| (ty == 0).then_some(g));
        let bodies = self.h.bodies.as_deref();
        let stats = self.stats.list_by_state_flags(unit, 0, 0x40).map(|l| {
            self.stats
                .base_entries(l)
                .into_iter()
                .take(16)
                .map(|(k, v)| {
                    let stat = crate::stats::key_stat(k);
                    StatEntry {
                        stat,
                        param: k as u32 as u16,
                        value: v,
                        send: bodies
                            .and_then(|b| b.stat(i32::from(stat)))
                            .map(|r| (r.send_other, r.send_bits, r.send_param_bits)),
                    }
                })
                .collect()
        });
        AssignMonster {
            guid,
            class: class as u16,
            x: x as u16,
            y: y as u16,
            life,
            mode,
            components,
            counts,
            type_block,
            source,
            stats,
        }
    }
}

impl<X: Pending> View<'_, X> {
    /// Sends the equipment of a monster: one S->C 0x9D action 6 per equipped
    /// item, owner the monster, in item GUID order; the stream is the item
    /// writer's ([`crate::wiring::inventory::bits::stream_item_of`]). The
    /// holdings are read from [`ActionHooks::monster_equip`] (the monster has
    /// no inventory model here, `economy/monster_equip.rs`). Needs the drop
    /// state's item tables; without them nothing is sent.
    ///
    /// [`ActionHooks::monster_equip`]: super::ActionHooks
    fn monster_inventory_messages(&mut self, receiver: UnitId, unit: UnitId) {
        use crate::items::bitstream;
        use crate::wiring::inventory::bits::{stream_item_of, ItemPlace};
        let Some(owner) = self.units.get(unit).map(|r| (r.ty as u8, r.guid)) else {
            return;
        };
        let Some(held) = self.h.monster_equip.get(&unit).cloned() else {
            return;
        };
        let Some(tables) = self.h.object_drops.as_ref().map(|d| d.tables.clone()) else {
            return;
        };
        let t = &tables.items;
        let class = self.units.get(unit).map_or(0, |r| r.class);
        // Both hands filled on a dual-wielder (classes 417 / 418): the item
        // that is not the weapon in use is category 6.
        let dual =
            (class == 0x1A1 || class == 0x1A2) && held.contains_key(&4) && held.contains_key(&5);
        // Items are sent in insertion order. The equipment is added in
        // creation order, so the item GUIDs ascend along it: the order is GUID
        // order, not body-slot order.
        let mut order: Vec<(u8, UnitId)> = held.iter().map(|(&l, &i)| (l, i)).collect();
        order.sort_by_key(|&(_, i)| self.units.get(i).map_or(u32::MAX, |r| r.guid));
        let mut out = Vec::new();
        for (loc, item) in order {
            let (Some(rec), Some(r)) = (self.h.items.get(item), self.units.get(item)) else {
                continue;
            };
            let Some(row) = t.item(rec.record) else {
                continue;
            };
            let place = ItemPlace {
                x: i32::from(loc),
                y: 0,
                body_loc: loc,
                filled: 0,
                runeword: 0xFFFF,
                static_pos: None,
            };
            let guid = r.guid;
            let Some(view) = stream_item_of(
                &*self.units,
                &self.h.items,
                &*self.stats,
                t,
                item,
                &place,
                crate::items::moves::iflag::CHANGED,
                rec.inv_page,
            ) else {
                continue;
            };
            let mut w = bitstream::BitWriter::new(bitstream::BUFFER);
            bitstream::write_into(&mut w, &view, &t.isc);
            let Ok(bits) = w.finish() else { continue };
            let category = if dual && loc == 5 { 6 } else { row.component };
            if let Ok(m) =
                crate::items::moves::layouts::item_owned(6, category, guid, owner.0, owner.1, &bits)
            {
                out.push(m);
            }
        }
        for m in out {
            self.h.x.send(receiver, &m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_monster_minimal_stream() {
        let a = AssignMonster {
            guid: 0x1B,
            class: 5,
            x: 0x1295,
            y: 0x1555,
            life: 128,
            mode: 2,
            ..AssignMonster::default()
        };
        // Mode 1 (walk is not sent), then five 0 bits: one byte 0x01.
        assert_eq!(
            assign_monster(&a),
            [0xAC, 0x1B, 0, 0, 0, 5, 0, 0x95, 0x12, 0x55, 0x15, 128, 14, 0x01]
        );
    }

    #[test]
    fn assign_monster_components_type_block_and_stats() {
        let mut components = [0u8; 16];
        components[0] = 1;
        let mut counts = [0u8; 16];
        counts[0] = 2;
        counts[1] = 5;
        let a = AssignMonster {
            mode: 12,
            components,
            counts,
            type_block: Some(TypeBlock {
                flags: type_flag::UNIQUE,
                hc_idx: 0,
                umods: vec![5, 0],
                name_seed: 0x1234,
                hireling_owner: None,
            }),
            stats: Some(vec![StatEntry {
                stat: 1,
                param: 0,
                value: 3,
                send: Some((false, 8, 0)),
            }]),
            ..AssignMonster::default()
        };
        let mut w = BitWriter::new();
        w.write(12, 4);
        w.write(1, 1);
        w.write(1, 1); // component 0, 1 bit
        w.write(0, 3); // component 1, count 5 → 3 bits
        for _ in 2..16 {
            w.write(0, 1);
        }
        w.write(1, 1);
        for bit in [0, 1, 0, 0, 0] {
            w.write(bit, 1);
        }
        w.write(5, 8);
        w.write(0, 8);
        w.write(0x1234, 16);
        w.write(0, 1); // no hireling owner
        w.write(0, 1); // no source link
        w.write(0, 2); // stats: none sent (no `send other`)
        let m = assign_monster(&a);
        assert_eq!(&m[13..], &w.bytes[..]);
        assert_eq!(m[12] as usize, 13 + w.bytes.len());
    }

    #[test]
    fn overhead_and_portal_builders() {
        assert_eq!(
            messages::overhead_chat(7, 2, 0x10, b"hi"),
            [0x26, 5, 7, 2, 0x10, 0, 0, 0, 0, 0, 0, b'h', b'i', 0]
        );
        let m = messages::portal_ownership(1, b"ab", 2, 0xFFFF_FFFF);
        assert_eq!(&m[..8], &[0x82, 1, 0, 0, 0, b'a', b'b', 0]);
        assert_eq!(&m[21..], &[2, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    }
}
