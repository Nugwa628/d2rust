//! Unit snapshots shared by the server's NPC and quest rests.
//! [`super::local_seams::sync_seams`] copies positions and unit facts into
//! [`Snap`], shared through [`SnapRef`]. Distances use integer sub-tiles.

use d2_sim::path::coords::Point;
use d2_sim::path::tables::PathTables;
use d2_sim::path::walk::geom::unit_distance;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use d2_sim::units::{UnitId, UnitType};

/// `NpcRest::axis_check`: the player within 50 sub-tiles
/// of the NPC on both axes.
const AXIS_LIMIT: i32 = 50;

/// One unit as the last sync saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapUnit {
    pub ty: UnitType,
    pub pos: (i32, i32),
    pub act: u8,
    pub guid: u32,
    /// The level id of the unit's room (0 without a DRLG room).
    pub level: u32,
    /// The unit's class (monsters: the `monstats` row).
    pub class: u32,
}

/// The players and monsters of the game at the last sync.
#[derive(Debug, Default)]
pub struct Snap {
    pub units: BTreeMap<UnitId, SnapUnit>,
    /// A sync has seen a unit in Act II: the Act II record
    /// is built then and kept to the game's end (`QuestRest::has_act2`).
    pub act2_built: bool,
}

/// Unit size used by [`Snap::distance`] (D2RUST's own, unverified).
const UNIT_SIZE: i32 = 2;

/// The snapshot shared by the sim's seams (writer) and the host rest
/// (reader).
pub type SnapRef = Arc<Mutex<Snap>>;

impl Snap {
    /// The sync's units; latches [`Snap::act2_built`].
    pub fn set_units(&mut self, units: BTreeMap<UnitId, SnapUnit>) {
        self.act2_built |= units.values().any(|u| u.act == 1);
        self.units = units;
    }

    /// Size-adjusted unit distance in sub-tiles (pathing.md section 9.5).
    pub fn distance(&self, a: UnitId, b: UnitId) -> i32 {
        let (Some(a), Some(b)) = (self.units.get(&a), self.units.get(&b)) else {
            return i32::MAX;
        };
        static TABLES: std::sync::OnceLock<Option<PathTables>> = std::sync::OnceLock::new();
        match TABLES.get_or_init(|| PathTables::spec().ok()) {
            Some(t) => unit_distance(
                t,
                Point::new(a.pos.0, a.pos.1),
                UNIT_SIZE,
                Point::new(b.pos.0, b.pos.1),
                UNIT_SIZE,
            ),
            None => i32::MAX,
        }
    }

    /// 0 when both axes are within [`AXIS_LIMIT`], else 1.
    pub fn axis_check(&self, player: UnitId, npc: UnitId) -> u32 {
        let (Some(a), Some(b)) = (self.units.get(&player), self.units.get(&npc)) else {
            return 1;
        };
        let near =
            (a.pos.0 - b.pos.0).abs() <= AXIS_LIMIT && (a.pos.1 - b.pos.1).abs() <= AXIS_LIMIT;
        u32::from(!near)
    }

    pub fn of_type(&self, ty: UnitType) -> Vec<UnitId> {
        self.units
            .iter()
            .filter(|(_, u)| u.ty == ty)
            .map(|(&id, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snap {
        let mut s = Snap::default();
        let u = |ty, x, y| SnapUnit {
            ty,
            pos: (x, y),
            act: 0,
            guid: 0,
            level: 0,
            class: 0,
        };
        s.units.insert(UnitId(1), u(UnitType::Player, 0, 0));
        s.units.insert(UnitId(2), u(UnitType::Monster, 3, 4));
        s.units.insert(UnitId(3), u(UnitType::Monster, 51, 0));
        s
    }

    #[test]
    fn distance_is_the_unit_distance_and_unknown_is_far() {
        let s = snap();
        // Both axes under 8: the `dist8_unit` table.
        assert_eq!(s.distance(UnitId(1), UnitId(2)), 4);
        // Past 8 on an axis: 2·max + min of Δ minus the sizes.
        assert_eq!(s.distance(UnitId(1), UnitId(3)), 2 * 49);
        assert_eq!(s.distance(UnitId(1), UnitId(9)), i32::MAX);
    }

    #[test]
    fn axis_check_is_fifty_on_both_axes() {
        let s = snap();
        assert_eq!(s.axis_check(UnitId(1), UnitId(2)), 0);
        assert_eq!(s.axis_check(UnitId(1), UnitId(3)), 1);
        assert_eq!(s.axis_check(UnitId(1), UnitId(9)), 1);
    }
}

/// Encode the 34 bytes of S-to-C 0x27: count at byte 0, entry kind at
/// 2 + 4k and string id at 4 + 4k. Entries leave newest-added first.
/// The count is capped at eight to fit the fixed message buffer.
pub fn encode_text_list(list: &[(u16, u32)]) -> [u8; 34] {
    let mut out = [0u8; 34];
    let n = list.len().min(8);
    out[0] = n as u8;
    for (k, &(string, kind)) in list.iter().rev().take(n).enumerate() {
        out[2 + 4 * k] = kind as u8;
        out[4 + 4 * k..6 + 4 * k].copy_from_slice(&string.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod encode_tests {
    use super::*;

    #[test]
    fn the_list_round_trips_through_the_client_layout() {
        let b = encode_text_list(&[(300, 2), (64, 0)]);
        assert_eq!(b[0], 2);
        assert_eq!((b[2], u16::from_le_bytes([b[4], b[5]])), (0, 64));
        assert_eq!((b[6], u16::from_le_bytes([b[8], b[9]])), (2, 300));
    }

    #[test]
    fn eight_entries_fill_the_record_and_more_are_cut() {
        // Entry 7 ends at byte 33; the count remains eight.
        let l: Vec<(u16, u32)> = (0..8).map(|i| (i, 1)).collect();
        let b = encode_text_list(&l);
        assert_eq!(b[0], 8);
        assert_eq!((b[30], u16::from_le_bytes([b[32], b[33]])), (1, 0));
        let l: Vec<(u16, u32)> = (0..10).map(|i| (i, 0)).collect();
        assert_eq!(encode_text_list(&l)[0], 8);
    }
}
