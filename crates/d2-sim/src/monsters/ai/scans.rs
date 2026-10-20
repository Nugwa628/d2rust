//! Room scans: the scan
//! table (13 × 8 bytes: scan mode, callback) and the four
//! scan modes over a [`ScanWorld`].

use crate::units::{RoomId, UnitId};

/// The callback of a scan id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanCallback {
    /// Passed in by the caller.
    Caller,
    /// A fixed function of the table.
    Fixed(u32),
}

/// Table: (mode, callback) per scan id 0…12.
pub const SCAN_TABLE: [(u8, ScanCallback); 13] = {
    use ScanCallback::{Caller, Fixed};
    [
        (0, Caller),
        (0, Caller),
        (1, Caller),
        (0, Fixed(0x005D_C870)),
        (0, Fixed(0x005D_C8E0)),
        (2, Fixed(0x005D_CA70)),
        (0, Fixed(0x005D_CBD0)),
        (0, Fixed(0x005D_CC60)),
        (0, Fixed(0x005D_CD50)),
        (3, Fixed(0x005D_CDA0)),
        (3, Caller),
        (2, Fixed(0x005D_CCC0)),
        (0, Fixed(0x005D_CE10)),
    ]
};

/// What the scan modes read.
pub trait ScanWorld {
    /// The scanner's active room's near-room list, own room included, in
    /// list order.
    fn adjacent_rooms(&self, unit: UnitId) -> Vec<RoomId>;
    /// A room's unit list in list order.
    fn room_units(&self, room: RoomId) -> Vec<UnitId>;
    /// The unit is a client's player.
    fn is_client_player(&self, unit: UnitId) -> bool;
    fn room_in_town(&self, room: RoomId) -> bool;
    /// The room has clients.
    fn room_has_clients(&self, room: RoomId) -> bool;
    /// The scanner is a living player or monster.
    fn scanner_is_living_player_or_monster(&self, unit: UnitId) -> bool;
    /// The minions of the scanner's minion owner.
    fn owner_minions(&self, unit: UnitId) -> Vec<UnitId>;
}

/// The units a scan of mode `mode` visits, in order.
pub fn scan_candidates<W: ScanWorld + ?Sized>(w: &W, unit: UnitId, mode: u8) -> Vec<UnitId> {
    match mode {
        0 => w
            .adjacent_rooms(unit)
            .into_iter()
            .flat_map(|r| w.room_units(r))
            .collect(),
        1 => w
            .adjacent_rooms(unit)
            .into_iter()
            .flat_map(|r| w.room_units(r))
            .filter(|&u| w.is_client_player(u))
            .collect(),
        2 => {
            if !w.scanner_is_living_player_or_monster(unit) {
                return Vec::new();
            }
            w.adjacent_rooms(unit)
                .into_iter()
                .filter(|&r| !w.room_in_town(r) && w.room_has_clients(r))
                .flat_map(|r| w.room_units(r))
                .collect()
        }
        3 => w.owner_minions(unit),
        _ => Vec::new(),
    }
}

/// Runs `callback` over the candidates of scan `id` until it
/// returns a unit; that unit, or none. An id outside the table scans
/// nothing.
pub fn scan<W: ScanWorld + ?Sized>(
    w: &W,
    unit: UnitId,
    id: usize,
    mut callback: impl FnMut(UnitId) -> Option<UnitId>,
) -> Option<UnitId> {
    let &(mode, _) = SCAN_TABLE.get(id)?;
    scan_candidates(w, unit, mode)
        .into_iter()
        .find_map(&mut callback)
}
