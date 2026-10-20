//! Unit allocation apart from `SUNIT_Add`. The base game allocates a unit
//! (GUID), runs the type's init, and only then adds
//! it to the lists (`SUNIT_Add`). The init may
//! schedule timers for the still unlisted unit: missile setup
//! schedules its every-tick event there,
//! seen in `traces/sim/tick/sim-0006`, `sim-0008` (timer_set before the
//! missile's room_add). [`UnitLists::add_unit`] is both steps at once.

use super::{ListError, RoomId, UnitEntry, UnitId, UnitLists, UnitType};

impl UnitLists {
    /// A unit record in no list yet (allocation). The GUID comes
    /// from [`super::GuidCounters::alloc`] or, for restored units, the
    /// caller. [`crate::game::Game::schedule_event`] accepts it.
    pub fn alloc_unit(&mut self, ty: UnitType, guid: u32, allied: bool) -> UnitId {
        UnitId(self.units.insert(UnitEntry {
            ty,
            guid,
            allied,
            room: None,
            hash_next: None,
            room_next: None,
            update_next: None,
            queued: false,
        }))
    }

    /// `SUNIT_Add` of an allocated unit: place it in
    /// `room`, insert it in its hash list, queue it for
    /// update. Same order and checks as [`UnitLists::add_unit`].
    pub fn add_allocated(&mut self, id: UnitId, room: Option<RoomId>) -> Result<(), ListError> {
        let (ty, guid) = {
            let e = self.unit_ok(id)?;
            (e.ty, e.guid)
        };
        if let Some(r) = room {
            self.room(r).ok_or(ListError::UnknownRoom(r))?;
        }
        // Checked first so a fatal duplicate leaves no partial insert.
        if self.hash_bucket_of(ty, guid).any(|(_, e)| e.guid == guid) {
            return Err(ListError::DuplicateGuid { ty, guid });
        }
        if let Some(r) = room {
            self.room_insert(id, r)?;
        }
        self.hash_insert(id);
        self.queue_update(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_unit_is_in_no_list_until_added() {
        let mut l = UnitLists::new();
        l.ensure_act(0).unwrap();
        let r = l.create_room(0).unwrap();
        l.activate_room(r).unwrap();
        let m = l.alloc_unit(UnitType::Missile, 1, false);
        assert!(l.units_of_type(UnitType::Missile).is_empty());
        assert!(l.room_units(r).is_empty());
        l.add_allocated(m, Some(r)).unwrap();
        assert_eq!(l.units_of_type(UnitType::Missile), [m]);
        assert_eq!(l.room_units(r), [m]);
        assert_eq!(l.update_queue(r), [m]);
        // A second add is the fatal duplicate, with no partial insert.
        assert_eq!(
            l.add_allocated(m, Some(r)),
            Err(ListError::DuplicateGuid {
                ty: UnitType::Missile,
                guid: 1
            })
        );
        assert_eq!(l.room_units(r), [m]);
    }
}
