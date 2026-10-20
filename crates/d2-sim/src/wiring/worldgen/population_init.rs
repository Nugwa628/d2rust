//! Population → monster init and unit allocation: [`MonsterInit`] on
//! [`WorldHost`]. Population's monster creation is the one
//! creation path: unit allocation (one game-seed
//! step) followed by the monster type init
//! ([`init::type_init`]); the per-class extras are init's normal
//! mods, then its boss mods; the boss calls go to init's umod
//! choice, transfer and init functions. Monster data lives in
//! [`super::WorldState::monsters`].
//!
//! The type init runs right after the allocator returns: the allocator's
//! per-kind hook (`LifecycleHooks::init_kind`) is its last step, so
//! nothing the allocator does follows it.

use crate::monsters::ai::MapNode;
use crate::monsters::init;
use crate::monsters::population::{Alloc, CoordRect, MonsterInit, OwnerKey, PopState, PresetUnit};
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};

use super::{WorldHost, WorldPending, WorldgenError};

/// Monster flag 2 is the "not counted" flag (the monster does not count
/// toward the level's population).
const FLAG_NOT_COUNTED: u32 = 2;
/// Champion pack member modifier.
const UMOD_CHAMPION: u8 = 16;
/// Quest modifier (umod 22).
const UMOD_QUEST: u8 = 22;

impl<X: WorldPending> MonsterInit for WorldHost<'_, X> {
    /// Allocation (type 1) then the type init with the
    /// game's regions in place (the type init reads the level's
    /// region data).
    fn allocate_monster(&mut self, a: Alloc, state: &mut PopState) -> Option<UnitId> {
        let class = u32::try_from(a.class).ok()?;
        let req = AllocRequest {
            ty: UnitType::Monster,
            class,
            room: Some(a.room),
            add: true,
            fixed_guid: a.guid,
            mode: u32::from(a.mode),
            allied: false,
        };
        // The allocator runs the type init inside itself,
        // before the unit is added, whose monster
        // branch schedules the first think.
        let unit = self.v.allocate_unadded(self.game, &req)?;
        self.with_state(state, |h| {
            h.init(|cx, h| init::type_init(cx, h, unit));
        });
        self.v
            .add_allocated(self.game, unit, &req, a.x, a.y)
            .then_some(unit)
    }

    fn set_monster_flag(&mut self, unit: UnitId, flag: u32) {
        if flag == FLAG_NOT_COUNTED {
            self.w.monsters.entry(unit).not_counted = true;
        } else {
            self.v.h.x.set_monster_flag(unit, flag);
        }
    }

    fn set_coord_record(
        &mut self,
        unit: UnitId,
        rect: Option<CoordRect>,
        room: RoomId,
        x: i32,
        y: i32,
    ) {
        // The AI vision record: a given
        // record `rect` is the caller's, else the one at (x, y). The
        // record is kept by identity (act, clipped box, index), which
        // names one record in every case D2RUST uses.
        let act = self.game.lists.room(room).map(|r| r.act);
        let rec =
            rect.or_else(|| crate::monsters::population::PopWorld::coord_at(self, room, x, y));
        if let (Some(act), Some(m)) = (act, self.w.monsters.get_mut(unit)) {
            m.vision = rec.map(|c| crate::monsters::init::VisionRecord {
                act,
                rect: c.rect,
                index: c.index,
            });
        }
        self.v.h.x.set_coord_record(unit, rect, room, x, y);
    }

    /// Sets the unit's alignment: its state-105 list
    /// ([`crate::wiring::action::View::set_alignment`]),
    /// then the host's copy.
    fn set_alignment(&mut self, unit: UnitId, align: u8) {
        self.v.set_alignment(self.game, unit, align);
        self.v.h.x.set_alignment(unit, align);
    }

    /// Unit flags (+0xC4) `|=`.
    fn set_unit_flags(&mut self, unit: UnitId, flags: u32) {
        if let Some(r) = self.v.units.get_mut(unit) {
            r.flags |= flags;
        }
    }

    /// The per-class extras: normal mods, with the regions in
    /// place: a boss umod there counts the boss.
    fn class_extras(&mut self, unit: UnitId, state: &mut PopState) {
        self.with_state(state, |h| h.init(|cx, h| init::normal_mods(cx, h, unit)));
    }

    /// The boss mods, with the regions in
    /// place: Izual's umod 22 counts him in the level's bosses
    /// (the level's boss count).
    fn init_monster(&mut self, unit: UnitId, state: &mut PopState) {
        self.with_state(state, |h| h.init(|cx, h| init::boss_mods(cx, h, unit)));
    }

    fn unit_class(&self, unit: UnitId) -> i32 {
        self.v.units.get(unit).map_or(-1, |r| r.class as i32)
    }

    /// The monster level (monster data).
    fn unit_level(&self, unit: UnitId) -> i32 {
        self.w.monsters.get(unit).map_or(0, |m| m.level_id)
    }

    fn type_flags(&self, unit: UnitId) -> u16 {
        self.w.monsters.get(unit).map_or(0, |m| m.type_flags)
    }

    fn set_type_flags(&mut self, unit: UnitId, flags: u16) {
        self.w.monsters.entry(unit).type_flags |= flags;
    }

    /// Rolls the boss's modifiers.
    fn boss_modifiers(&mut self, boss: UnitId, champion_allowed: bool) {
        self.init(|cx, h| init::choose_umods(cx, h, boss, champion_allowed));
    }

    /// The end of the boss modifier roll: the boss-modifier init
    /// (population spawned and linked the minions first).
    fn boss_modifier_init(&mut self, boss: UnitId) {
        self.init(|cx, h| init::boss_minions_and_init(cx, h, boss, 0, 0, None, false));
    }

    /// Adds modifier `m`: a champion pack member or the closing quest
    /// modifier 22; a superunique runs its last two init steps right before its
    /// closing 22.
    fn add_modifier(&mut self, unit: UnitId, m: u8, state: &mut PopState) {
        self.with_state(state, |h| match m {
            UMOD_CHAMPION => h.init(|cx, h| init::champion_pack_member(cx, h, unit, m)),
            UMOD_QUEST => {
                if let Some((row, aura)) = h.w.superunique_tail.remove(&unit) {
                    h.init(|cx, h| init::superunique_finish_with(cx, h, unit, row, aura, false));
                }
                h.init(|cx, h| init::assign_umod(cx, h, unit, m, true));
            }
            _ => h.w.errors.push(WorldgenError::Modifier(m)),
        });
    }

    /// Moves the boss's modifiers to a minion.
    fn transfer_modifiers(&mut self, boss: UnitId, minion: UnitId) {
        self.init(|cx, h| init::xfer_umods(cx, h, boss, minion));
    }

    /// `(game, unit, GUID, type a, f1 b, f2 c)`: the AI control
    /// record's minion owner (GUID and type), read back by the
    /// pack-leader test (the Fallen flee when their leader dies).
    /// Both keys name the unit's GUID.
    ///
    /// f2 ≠ 0 → control flags |= 0x2, f1 ≠ 0 → |= 0x1 (a zero flag
    /// leaves its bit as it was; no AI restart),
    /// whether or not the owner lookup succeeds.
    fn set_owner_data(&mut self, unit: UnitId, owner: OwnerKey, a: i32, b: i32, c: i32) {
        let (OwnerKey::Guid(o) | OwnerKey::DataOf(o)) = owner;
        if let Some(ctl) = self.v.h.ai.as_mut().and_then(|s| s.control_mut(unit)) {
            ctl.flags |= (if c != 0 { 0x2 } else { 0 }) | (if b != 0 { 0x1 } else { 0 });
        }
        let guid = self.game.lists.unit(o).map(|e| e.guid);
        let ty = u8::try_from(a)
            .ok()
            .and_then(|t| UnitType::ALL.get(usize::from(t)).copied());
        if let (Some(guid), Some(ty)) = (guid, ty) {
            if let Some(ctl) = self.v.h.ai.as_mut().and_then(|s| s.control_mut(unit)) {
                ctl.minion_owner = Some(crate::monsters::ai::UnitRef { ty, guid });
            }
        }
        self.v.h.x.set_owner_data(unit, owner, a, b, c);
    }

    /// Adds a minion: also the leader's AI control minion list
    /// (GUIDs), which the leader's command copy walks.
    fn add_minion(&mut self, leader: UnitId, minion: UnitId) {
        self.w.minions.entry(leader).or_default().push(minion);
        if let Some(guid) = self.game.lists.unit(minion).map(|e| e.guid) {
            if let Some(ctl) = self.v.h.ai.as_mut().and_then(|s| s.control_mut(leader)) {
                ctl.minions.push(guid);
            }
        }
    }

    /// Sets a minion's owner: also the minion's AI control leash owner
    /// (GUID and type), which the AI's precheck A reads.
    fn set_owner(&mut self, minion: UnitId, owner: UnitId) {
        self.w.owners.insert(minion, owner);
        if let Some(e) = self.game.lists.unit(owner) {
            let leash = crate::monsters::ai::UnitRef {
                ty: e.ty,
                guid: e.guid,
            };
            if let Some(ctl) = self.v.h.ai.as_mut().and_then(|s| s.control_mut(minion)) {
                ctl.owner = Some(leash);
            }
        }
    }

    /// The boss quest hook inside the creation: the lent
    /// quest host first, as `init_units`.
    fn boss_quest_hook(&mut self, boss: UnitId) {
        if let Some(mut host) = self.v.h.quest_host.take() {
            let done = host.boss_created(&mut *self.game, &mut self.v, boss);
            self.v.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.v.h.x.boss_quest_hook(boss);
    }

    /// Superunique init steps 1–2; steps 4–5 wait for the closing modifier
    /// 22 (population spawns the minions, step 3, in between).
    // TODO: the order
    // of the extra minion spawns and the aura re-run / quest records is
    // not settled; the spawns run first here.
    fn superunique_init(&mut self, boss: UnitId, su: i32) {
        let Ok(row) = u16::try_from(su) else {
            return;
        };
        if let Some(aura) = self.init(|cx, h| init::superunique_mods(cx, h, boss, row)) {
            self.w.superunique_tail.insert(boss, (row, aura));
        }
    }

    /// Allocation (type 2, class, …) with the object init
    /// on the action wiring's object state
    /// ([`crate::wiring::action::View::create_object`]); a game without
    /// one: [`WorldPending::create_object`].
    ///
    /// Allocated in mode 0 with flag 1 and GUID 0 (the evilhut and
    /// barricade-door objects).
    fn create_object(&mut self, room: RoomId, class: i32, x: i32, y: i32) {
        match u32::try_from(class) {
            Ok(c) if self.v.h.objects.is_some() => {
                self.v.create_object(self.game, room, c, x, y, 0);
            }
            _ => self.v.h.x.create_object(room, class, x, y),
        }
    }

    /// The preset path's points (level
    /// sub-tiles from the preset room) become the map-AI nodes
    /// (action, x, y) of the monster's AI control. A monster without an
    /// AI control gets none; the preset's path is cleared either way.
    fn move_preset_path(&mut self, unit: UnitId, room: RoomId, index: usize) {
        let Some((act, r)) = self.drlg_room(room) else {
            return;
        };
        let path = self
            .w
            .types
            .borrow_mut()
            .act_presets_mut(act)
            .and_then(|p| p.take_unit_path(r, index));
        let (Some(path), Some(ai)) = (path, self.v.h.ai.as_mut()) else {
            return;
        };
        if let Some(c) = ai.control_mut(unit) {
            c.map_ai = Some(
                path.iter()
                    .map(|p| MapNode {
                        action: p.action as i32,
                        x: p.x,
                        y: p.y,
                    })
                    .collect(),
            );
        }
    }

    fn preset_created(&mut self, unit: UnitId, preset: &PresetUnit) {
        self.v.h.x.preset_created(unit, preset);
    }

    /// Class 438: event 7 (MONUMOD) at
    /// frame + 250 + `roll(50)` on the created monster's own seed
    /// (it gets its modifier a few seconds after it spawns).
    fn schedule_monumod(&mut self, unit: UnitId) {
        let r = self
            .v
            .units
            .get_mut(unit)
            .map_or(0, |rec| rec.seed.roll(50)) as i32;
        let at = self.game.after(250_i32.wrapping_add(r));
        // A unit outside the lists schedules nothing.
        let _ = self
            .game
            .schedule_event(unit, init::EVENT_UMOD, at, None, 0, 0);
    }

    fn change_alignment(&mut self, unit: UnitId, a: i32, b: i32) {
        self.v.h.x.change_alignment(unit, a, b);
    }

    fn restore_inactive_units(&mut self, room: RoomId) {
        self.v.h.x.restore_inactive_units(room);
    }

    /// Populates a room's objects on the action wiring's
    /// object state with the room facts of the DRLG; a game without one:
    /// [`WorldPending::populate_objects`].
    fn populate_objects(&mut self, room: RoomId) {
        if self.v.h.objects.is_none() {
            self.v.h.x.populate_objects(room);
            return;
        }
        let info = self.object_room_info(room);
        let mut souls = Souls { w: &mut *self.w };
        self.v.populate_objects_with(self.game, &info, &mut souls);
    }
}

/// The object init's monster creation through population's placement,
/// on the world state that runs the
/// room pass.
struct Souls<'a> {
    w: &'a mut super::WorldState,
}

impl<X: WorldPending> crate::wiring::action::SoulSpawner<X> for Souls<'_> {
    fn spawn_soul(
        &mut self,
        game: &mut crate::game::Game,
        v: &mut crate::wiring::action::View<'_, X>,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
    ) -> Option<UnitId> {
        let mut wh = WorldHost {
            game,
            v: crate::wiring::action::objects::reborrow(v),
            w: &mut *self.w,
        };
        wh.population(|cx| {
            crate::monsters::population::placement::place_at(
                cx,
                room,
                None,
                x,
                y,
                i32::from(class),
                1,
                -1,
                0,
            )
        })
        .unit()
    }
}
