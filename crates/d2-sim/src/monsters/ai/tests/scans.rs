//! The scan table and the four scan modes.

use std::collections::{BTreeMap, BTreeSet};

use super::super::scans::{scan, scan_candidates, ScanCallback, ScanWorld, SCAN_TABLE};
use crate::units::{RoomId, UnitId};

const ME: UnitId = UnitId(1);

#[derive(Default)]
struct W {
    adj: Vec<RoomId>,
    units: BTreeMap<u32, Vec<UnitId>>,
    clients: BTreeSet<UnitId>,
    town: BTreeSet<u32>,
    with_clients: BTreeSet<u32>,
    living: bool,
    minions: Vec<UnitId>,
}

impl ScanWorld for W {
    fn adjacent_rooms(&self, _: UnitId) -> Vec<RoomId> {
        self.adj.clone()
    }
    fn room_units(&self, r: RoomId) -> Vec<UnitId> {
        self.units.get(&r.0).cloned().unwrap_or_default()
    }
    fn is_client_player(&self, u: UnitId) -> bool {
        self.clients.contains(&u)
    }
    fn room_in_town(&self, r: RoomId) -> bool {
        self.town.contains(&r.0)
    }
    fn room_has_clients(&self, r: RoomId) -> bool {
        self.with_clients.contains(&r.0)
    }
    fn scanner_is_living_player_or_monster(&self, _: UnitId) -> bool {
        self.living
    }
    fn owner_minions(&self, _: UnitId) -> Vec<UnitId> {
        self.minions.clone()
    }
}

fn u(n: u32) -> UnitId {
    UnitId(n)
}

fn world() -> W {
    let mut w = W {
        adj: vec![RoomId(1), RoomId(2), RoomId(3)],
        living: true,
        minions: vec![u(40), u(41)],
        ..W::default()
    };
    w.units.insert(1, vec![u(10), u(11)]);
    w.units.insert(2, vec![u(20)]);
    w.units.insert(3, vec![u(30), u(31)]);
    w.clients.extend([u(11), u(30)]);
    w.with_clients.extend([1, 3]);
    w
}

#[test]
fn table_and_modes() {
    use ScanCallback::{Caller, Fixed};
    // The scan table: ids 0–12.
    let want = [
        (0, Caller),
        (0, Caller),
        (1, Caller),
        (0, Fixed(0x5DC870)),
        (0, Fixed(0x5DC8E0)),
        (2, Fixed(0x5DCA70)),
        (0, Fixed(0x5DCBD0)),
        (0, Fixed(0x5DCC60)),
        (0, Fixed(0x5DCD50)),
        (3, Fixed(0x5DCDA0)),
        (3, Caller),
        (2, Fixed(0x5DCCC0)),
        (0, Fixed(0x5DCE10)),
    ];
    assert_eq!(SCAN_TABLE, want);

    let mut w = world();
    // Mode 0: every unit of every adjacent room, room then list order.
    assert_eq!(
        scan_candidates(&w, ME, 0),
        [u(10), u(11), u(20), u(30), u(31)]
    );
    // Mode 1: only the clients' players.
    assert_eq!(scan_candidates(&w, ME, 1), [u(11), u(30)]);
    // Mode 2: rooms not in town that have clients, for a living player or
    // monster scanner.
    assert_eq!(scan_candidates(&w, ME, 2), [u(10), u(11), u(30), u(31)]);
    w.town.insert(1);
    assert_eq!(scan_candidates(&w, ME, 2), [u(30), u(31)]);
    w.living = false;
    assert!(scan_candidates(&w, ME, 2).is_empty());
    // Mode 3: the minions of the scanner's minion owner.
    assert_eq!(scan_candidates(&w, ME, 3), [u(40), u(41)]);
    // A scan stops at the first callback that returns a unit; the others
    // are visited in order.
    let w = world();
    let mut seen = Vec::new();
    let got = scan(&w, ME, 0, |c| {
        seen.push(c);
        (c == u(20)).then_some(c)
    });
    assert_eq!(got, Some(u(20)));
    assert_eq!(seen, [u(10), u(11), u(20)]);
    // Scan id 9 (dead minions, mode 3) visits the minion list.
    let mut seen = Vec::new();
    assert_eq!(
        scan(&w, ME, 9, |c| {
            seen.push(c);
            None
        }),
        None
    );
    assert_eq!(seen, [u(40), u(41)]);
    // An id outside the table scans nothing.
    assert_eq!(scan(&w, ME, 13, Some), None);
}
