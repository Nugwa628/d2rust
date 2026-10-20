//! Monster skill check: whether a
//! monster may use a skill now. No draws. The world (rooms, patterns,
//! lines) is reached through [`SkillCheckWorld`].

use d2_data::tables::Skills;

use crate::units::{RoomId, UnitId};

/// Pattern / collision mask of the placement tests.
pub const PATTERN_MASK: u16 = 0x3C01;
/// Line mask of the point tests (rules 4, 6, 7).
pub const LINE_MASK: u16 = 0x0C01;
/// Line mask of skill 203 (step 8).
pub const LINE_MASK_203: u16 = 0x0805;

/// What the check reads. Positions are the units' path positions.
pub trait SkillCheckWorld {
    fn position(&self, unit: UnitId) -> (i32, i32);
    fn room(&self, unit: UnitId) -> Option<RoomId>;
    /// Unit flags (+0xC4) bit `bit`.
    fn has_unit_flag(&self, unit: UnitId, bit: u32) -> bool;
    /// Mode 0 (DT) or 12 (DD).
    fn dead_or_dying(&self, unit: UnitId) -> bool;
    fn in_town(&self, room: RoomId) -> bool;
    /// The pattern test `(room, x, y, pattern of `unit`, mask)` = 0.
    fn pattern_free(&self, room: Option<RoomId>, unit: UnitId, x: i32, y: i32, mask: u16) -> bool;
    /// The free-point search `(room, &p, size of `unit`, mask, 0)`: the room of the
    /// free point found for P, none when there is none.
    fn free_point_room(
        &self,
        room: Option<RoomId>,
        unit: UnitId,
        p: (i32, i32),
        mask: u16,
    ) -> Option<RoomId>;
    /// ≠ 0.
    fn line_clear(&self, from: (i32, i32), to: (i32, i32), room: RoomId, mask: u16) -> bool;
    /// The room holding (x, y), searched from the unit's room.
    fn room_at(&self, unit: UnitId, x: i32, y: i32) -> Option<RoomId>;
    /// The DiabPrison
    /// placement test.
    fn diab_prison_ok(&self, room: Option<RoomId>, target: Option<UnitId>) -> bool;
}

/// Rules 1–9, in order.
pub fn skill_check<W: SkillCheckWorld + ?Sized>(
    w: &W,
    skills: &[Skills],
    unit: UnitId,
    skill: i32,
    target: Option<UnitId>,
    x: i32,
    y: i32,
) -> bool {
    // 1.
    let Some(row) = usize::try_from(skill).ok().and_then(|i| skills.get(i)) else {
        return false;
    };
    if skill == 167 {
        return false;
    }
    let (ux, uy) = w.position(unit);
    // 2.
    if row.tgtplacecheck {
        return match target {
            Some(t) if w.has_unit_flag(t, 0x2) => {
                let (tx, ty) = w.position(t);
                w.pattern_free(w.room(t), t, tx, ty, PATTERN_MASK)
            }
            _ => false,
        };
    }
    // 3.
    if skill == 164 {
        return target.is_some();
    }
    // 4.
    if matches!(row.srvdofunc, 77 | 78) {
        let Some(t) = target else {
            return false;
        };
        if w.dead_or_dying(unit) {
            return false;
        }
        let (tx, ty) = w.position(t);
        let p = (
            tx.wrapping_mul(2).wrapping_sub(ux),
            ty.wrapping_mul(2).wrapping_sub(uy),
        );
        let Some(r) = w.free_point_room(w.room(unit), unit, p, PATTERN_MASK) else {
            return false;
        };
        return !w.in_town(r)
            && w.pattern_free(Some(r), unit, p.0, p.1, PATTERN_MASK)
            && w.line_clear((ux, uy), p, r, LINE_MASK);
    }
    // 5.
    if skill == 199 {
        return w.diab_prison_ok(target.and_then(|t| w.room(t)), target);
    }
    // 6.
    if skill == 184 {
        let Some(r) = w.room_at(unit, x, y) else {
            return false;
        };
        return !w.in_town(r)
            && w.pattern_free(Some(r), unit, x, y, PATTERN_MASK)
            && w.line_clear((ux, uy), (x, y), r, LINE_MASK);
    }
    // 7, 8.
    let mask = if row.srvdofunc == 67 {
        LINE_MASK
    } else if skill == 203 {
        LINE_MASK_203
    } else {
        // 9.
        return true;
    };
    let Some(r) = w.room_at(unit, x, y) else {
        return false;
    };
    !w.in_town(r) && w.line_clear((ux, uy), (x, y), r, mask)
}
