//! The skill message (S→C 0x4C / 0x4D: a unit uses a skill on a unit or
//! on a point) with its two builders, the player's
//! skill-mode message (the skill rows of the player mode-update table)
//! and the monster hit message 0x0C.
//!
//! The client creates a unit's missiles from these skill messages: no
//! separate missile message is sent.

use crate::game::Game;
use crate::path::CollisionRooms;
use crate::units::{UnitId, UnitType};

use super::super::{Pending, View};

/// Stat 352 `last_sent_hp_pct`.
const STAT_LAST_SENT_HP: u16 = 352;

/// S→C 0x4C, the 16-byte form (flag 0):
/// type @1, GUID @2, skill u16 @6, b @8, target type @9, target GUID
/// @10, w u16 @14.
pub fn skill_on_unit(
    ty: u8,
    guid: u32,
    skill: u16,
    b: u8,
    target_ty: u8,
    target: u32,
    w: u16,
) -> [u8; 16] {
    let mut m = [0u8; 16];
    m[0] = 0x4C;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6..8].copy_from_slice(&skill.to_le_bytes());
    m[8] = b;
    m[9] = target_ty;
    m[10..14].copy_from_slice(&target.to_le_bytes());
    m[14..16].copy_from_slice(&w.to_le_bytes());
    m
}

/// S→C 0x4D, the 17-byte form (also the out-of-rooms form of the 0x4C
/// builder, flag 0): type @1, GUID @2, skill u32
/// @6, b @10, x u16 @11, y u16 @13, w u16 @15.
pub fn skill_on_point(ty: u8, guid: u32, skill: u32, b: u8, x: u16, y: u16, w: u16) -> [u8; 17] {
    let mut m = [0u8; 17];
    m[0] = 0x4D;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6..10].copy_from_slice(&skill.to_le_bytes());
    m[10] = b;
    m[11..13].copy_from_slice(&x.to_le_bytes());
    m[13..15].copy_from_slice(&y.to_le_bytes());
    m[15..17].copy_from_slice(&w.to_le_bytes());
    m
}

/// S→C 0x0C MonsterHit (9 bytes):
/// type 1 @1, GUID @2, 0x13 @6, hit class @7, h @8 where h := p − 1
/// when the life fraction p > 1, else p, | 0x80 when the monster has
/// unit flag 0x100 (`flag_100`).
pub fn monster_hit(guid: u32, b0: u8, life: u8, flag_100: bool) -> [u8; 9] {
    let h = if life > 1 { life - 1 } else { life } | if flag_100 { 0x80 } else { 0 };
    let mut m = [0u8; 9];
    m[0] = 0x0C;
    m[1] = UnitType::Monster as u8;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = 0x13;
    m[7] = b0;
    m[8] = h;
    m
}

/// Player modes whose update row sends the skill message: A1 7, A2 8,
/// SC 10, TH 11, KK 12, S1–S4 13–16, SQ 18.
// These modes' rows send the skill message 0x4C / 0x4D (target unit /
// target point; codes 0x15 / 0x16), flag 0, w 0.
pub fn player_skill_mode(mode: u32) -> bool {
    matches!(mode, 7 | 8 | 10..=16 | 18)
}

impl<X: Pending> View<'_, X> {
    /// The skill message of `unit` (type `ty`,
    /// `guid`) to `receiver`: none used skill → nothing (false). With
    /// the target `target` (type, GUID): the on-unit form; else
    /// the on-point form at `point` (the path target). Flag 0, w 0.
    pub(crate) fn skill_message(
        &mut self,
        game: &Game,
        receiver: UnitId,
        unit: UnitId,
        (ty, guid): (u8, u32),
        target: Option<(u8, u32)>,
        point: (u16, u16),
    ) -> bool {
        let Some(e) = self.h.used_skill_of(unit) else {
            return false;
        };
        // The low byte of the used skill's level (`skill_level(unit, E, 1)`).
        let b = self.used_level(unit, &e) as u8;
        let m = match target {
            Some(t) => {
                self.skill_on_unit_message(game, receiver, (ty, guid), e.skill, b, t, 0, false)
            }
            None => skill_on_point(ty, guid, e.skill as u32, b, point.0, point.1, 0).to_vec(),
        };
        self.h.x.send(receiver, &m);
        true
    }

    /// `skill_level(unit, entry, 1)` on the unit's record, stat list and
    /// states.
    fn used_level(&self, unit: UnitId, e: &crate::skills::SkillEntry) -> i32 {
        let player = self
            .units
            .get(unit)
            .filter(|r| r.ty == UnitType::Player)
            .map(|r| r.class as i32);
        crate::skills::skill_level_of(
            &self.h.tables.skills,
            e,
            true,
            player,
            self.stats.has_state(unit, 134),
            |stat, layer| self.stats.unit_total(unit, stat, layer),
        )
    }

    /// The on-unit skill message (flag 0, w 0): the target found and the
    /// receiver without a room, or the target's room not in the
    /// receiver's room's adjacency list → the 17-byte
    /// form at the target's path target; else the 16-byte 0x4C.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::wiring::action) fn skill_on_unit_message(
        &self,
        game: &Game,
        receiver: UnitId,
        (ty, guid): (u8, u32),
        skill: i32,
        b: u8,
        (tt, tg): (u8, u32),
        w: u16,
        flag: bool,
    ) -> Vec<u8> {
        // Flag ≠ 0 (the pending event records): the id is the
        // flagless id + 0x4D, 0x4C → 0x99 and 0x4D → 0x9A.
        let id = |mut m: Vec<u8>| {
            if flag {
                m[0] = m[0].wrapping_add(0x4D);
            }
            m
        };
        let found = UnitType::ALL
            .get(tt as usize)
            .and_then(|&t| game.lists.find_unit(t, tg));
        if let Some(t) = found {
            let rooms = &self.h.drlg;
            let near = game
                .lists
                .unit(receiver)
                .and_then(|r| r.room())
                .zip(game.lists.unit(t).and_then(|r| r.room()))
                .is_some_and(|(pr, tr)| {
                    (0..rooms.adjacent_count(pr)).any(|i| rooms.adjacent(pr, i) == Some(tr))
                });
            if !near {
                let (x, y) = self
                    .h
                    .paths
                    .as_ref()
                    .and_then(|p| p.dynamic(t))
                    .map_or((0, 0), |d| (d.target_x, d.target_y));
                return id(skill_on_point(ty, guid, skill as u16 as u32, b, x, y, w).to_vec());
            }
        }
        id(skill_on_unit(ty, guid, skill as u16, b, tt, tg, w).to_vec())
    }

    /// The monster hit message: p := the life fraction, stored
    /// as stat 352; S→C 0x0C to `receiver`.
    pub(in crate::wiring::action) fn hit_message(
        &mut self,
        receiver: UnitId,
        unit: UnitId,
        guid: u32,
    ) {
        let p = crate::stats::life_fraction(
            self.stats.unit_total(unit, 6, 0),
            crate::combat::vitals::VitalsUnits::max_life(self, unit),
        );
        self.stats
            .unit_set(&mut *self.h, unit, STAT_LAST_SENT_HP, p, 0);
        let m = monster_hit(
            guid,
            self.unit_b0(unit),
            p as u8,
            self.h.x.monster_flag_100(unit),
        );
        self.h.x.send(receiver, &m);
    }
}
