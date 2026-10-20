//! Forced targets: the
//! monster-data override (kind +0x38, GUID +0x34) that terror, confuse and
//! skill callbacks set. The world is reached through [`ForcedWorld`].

use crate::units::{UnitId, UnitType};

/// What the forced-target check reads and writes.
pub trait ForcedWorld {
    /// Monster data override (kind, GUID); `None` for a non-monster.
    fn override_of(&self, unit: UnitId) -> Option<(u32, u32)>;
    /// Kind, GUID := 0.
    fn clear_override(&mut self, unit: UnitId);
    fn find_by_guid(&self, ty: UnitType, guid: u32) -> Option<UnitId>;
    /// Full-size distance from `from` to `to`.
    fn full_distance(&self, from: UnitId, to: UnitId) -> i32;
    /// Hits.
    fn line_blocked(&self, a: UnitId, b: UnitId) -> bool;
    fn alignment(&self, unit: UnitId) -> u8;
    fn set_alignment(&mut self, unit: UnitId, value: u8);
    /// One raw step of the unit seed, `& 1`.
    fn seed_bit(&mut self, unit: UnitId) -> u32;
    /// Room scan 5 (`s = 0`, context with LOS flag `a`, radius 35) or scan
    /// 6 (`s ≠ 0`) of the forced-target search: the best unit and its distance.
    fn scan(&mut self, unit: UnitId, scan: u8, a: bool) -> Option<(UnitId, i32)>;
    fn unit_type(&self, unit: UnitId) -> Option<UnitType>;
    fn is_dead(&self, unit: UnitId) -> bool;
}

/// The temporary alignment of kind 3 (confuse) for alignment `a` and the
/// drawn bit `r`.
pub fn confused_alignment(a: u8, r: u32) -> u8 {
    match (a, r != 0) {
        (1, true) => 2,
        (1, false) => 0,
        (0, true) => 2,
        (2, true) => 0,
        _ => a,
    }
}

/// The forced target and its distance, or none.
pub fn forced_target<W: ForcedWorld + ?Sized>(
    w: &mut W,
    unit: UnitId,
    a: bool,
    s: bool,
) -> Option<(UnitId, i32)> {
    // 1.
    let (k, g) = w.override_of(unit)?;
    if k == 0 {
        return None;
    }
    let found = match k {
        // 2.
        1 | 2 | 4 => {
            let ty = match k {
                1 => UnitType::Player,
                2 => UnitType::Monster,
                _ => UnitType::Missile,
            };
            let Some(u) = w.find_by_guid(ty, g) else {
                w.clear_override(unit);
                return None;
            };
            let d = w.full_distance(u, unit);
            if s && w.line_blocked(u, unit) {
                w.clear_override(unit);
                return None;
            }
            Some((u, d))
        }
        // 3.
        3 => {
            let original = w.alignment(unit);
            let r = w.seed_bit(unit);
            w.set_alignment(unit, confused_alignment(original, r));
            let best = w.scan(unit, if s { 6 } else { 5 }, a);
            w.set_alignment(unit, original);
            best
        }
        // The base game (checked 2026-10-11): any other kind is the
        // switch default, which clears the
        // override and finds none.
        _ => None,
    };
    // 4.
    let accept = found.filter(|&(u, _)| match w.unit_type(u) {
        Some(UnitType::Player | UnitType::Monster) => !w.is_dead(u),
        Some(UnitType::Missile) => true,
        _ => false,
    });
    if accept.is_none() {
        w.clear_override(unit);
    }
    accept
}
