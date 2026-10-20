//! The compress of the tick (monsters, items and objects leave the active
//! lists into the inactive store) and the restore of a room's first
//! population on the action wiring: the rules of [`crate::units::inactive`]
//! with the unit records, the stat lists, the timer queue, the room lists
//! and the DRLG; the facts and re-creations without a d2-sim provider go to
//! [`Pending`]'s inactive-store seams.
//!
//! The store is [`ActionHooks::inactive`]: `None` (the default) keeps the
//! old behaviour (no compress, no restore) for every unit but warp tiles,
//! which go to [`ActionHooks::fallback_tiles`] (a D2RUST-only path for hosts
//! that leave the store off; the game's rule is the store, and the play
//! host turns the store on at `single_player.rs`, so the path is not
//! reached in play);
//! [`ActionHooks::enable_inactive_store`] turns it on.
//!
//! The facts come from the game's tables where it has them (leveldefs
//! `SaveMonsters` and the `objects` rows of the object state, monstats2
//! `restore` of the action tables, the lent monster world's monster data),
//! else from the [`Pending`] seams. The restore re-creates the units
//! itself (a unit allocation, so a monster gets its type init from the lent
//! world and an object its init from the object state).
//!
//! The node key is the DRLG room's sub-tile origin (the rules order nodes
//! by the room origin's x; sub-tiles are tiles × 5, so the order is the
//! same).

use crate::game::Game;
use crate::units::hooks::Sim;
use crate::units::inactive::{
    self as ia, Compress, CompressFacts, MonsterFacts, MonsterRecord, MonsterRestore, OtherRecord,
    SpawnKind,
};
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};

use super::{ActionHooks, ActionSim, Pending, View};

/// State 7 (`playerbody`).
const STATE_PLAYERBODY: u16 = 7;
/// Event types cancelled by the store.
const EVENT_TYPE_1: u8 = 1;
const EVENT_TYPE_2: u8 = 2;
/// Stats of the monster record.
const STAT_13: u16 = 13;
const STAT_MAXLIFE: u16 = 7;
const STAT_LIFE: u16 = 6;
/// Region accounting carried across unit unloading, above the shared record bits.
const REGION_DEATH_COUNTED: u32 = 0x1000;
const REGION_NOT_COUNTED: u32 = 0x2000;
/// Object event 5 (shrine regrow) and 2 (well refill).
const OBJECT_EVENT_5: u8 = 5;
const OBJECT_EVENT_2: u32 = 2;
/// Object classes whose GUID the record keeps.
const GUID_CLASSES: [u32; 3] = [59, 60, 100];
/// Unit flags set on every unit the allocation creates.
const PRESET_UNIT_FLAGS: u32 = 0x300_0000;
/// Unit flag 0x10 a kept unit gets back.
const UNIT_FLAG_10: u32 = 0x10;
/// The mode a kept player body must be in (otherwise fatal, error 0x156).
const PLAYER_BODY_MODE: u32 = 0x11;
/// `objects` `SubClass` bits: shrine, well.
const SUBCLASS_SHRINE: u8 = 0x1;
const SUBCLASS_WELL: u8 = 0x20;

impl<X> ActionHooks<X> {
    /// Turns the inactive store on (empty).
    pub fn enable_inactive_store(&mut self) {
        self.inactive = Some(ia::InactiveStore::default());
    }
}

impl<X: Pending> View<'_, X> {
    /// The node key of a room: its act and sub-tile origin.
    fn node_key(&self, game: &Game, room: RoomId) -> Option<(u8, (i32, i32))> {
        use crate::path::CollisionRooms;
        let act = game.lists.room(room)?.act;
        let r = self.h.drlg.subtile_rect(room)?;
        Some((act, (r.x, r.y)))
    }

    /// The level part of the save flag: the room's level has `SaveMonsters`
    /// ≠ 0 in leveldefs, from the object state's leveldefs; without them the
    /// host's answer.
    fn save_monsters(&self, game: &Game, room: RoomId) -> bool {
        let row = self.h.objects.as_ref().and_then(|o| {
            let level = self.h.drlg.level_id(game, room)?;
            o.tables.leveldefs.get(usize::try_from(level).ok()?)
        });
        match row {
            Some(d) => d.savemonsters != 0,
            None => self.h.x.save_monsters(game, room),
        }
    }

    /// monstats2 `restore` of the monster's class through monstats
    /// `MonStatsEx`; `None`: no row. Without monstats2 rows at all the host
    /// answers.
    fn monster_restore(&self, u: UnitId) -> Option<u8> {
        let t = &self.h.tables.combat;
        if t.monstats2.is_empty() {
            return self.h.x.monster_restore(u);
        }
        let class = self.units.get(u)?.class;
        let ex = t.monstats.get(usize::try_from(class).ok()?)?.monstatsex;
        t.monstats2.get(usize::from(ex)).map(|r| r.restore)
    }

    /// The objects row of an object unit (object state tables).
    fn object_row(&self, u: UnitId) -> Option<&d2_data::tables::Objects> {
        let class = u16::try_from(self.units.get(u)?.class).ok()?;
        self.h.objects.as_ref()?.tables.object(class).ok()
    }

    /// Objects: (`Restore` = 0, the object's spark byte has 0x2,
    /// `RestoreVirgins` ≠ 0), from the object state; without it the host's
    /// answer.
    fn object_restore_facts(&self, u: UnitId) -> (bool, bool, bool) {
        let Some(row) = self.object_row(u) else {
            return self.h.x.object_restore_facts(u);
        };
        let spark = self
            .h
            .objects
            .as_ref()
            .and_then(|o| o.control.data.get(&u))
            .map_or(0, |d| d.spark);
        (row.restore == 0, spark & 0x2 != 0, row.restorevirgins != 0)
    }

    /// Monster type flags `& mask`.
    fn type_flag(&self, u: UnitId, mask: u16) -> bool {
        self.h.monster_flag(u, u32::from(mask))
    }

    fn monster_record(&self, game: &Game, u: UnitId) -> Option<MonsterRecord> {
        let r = self.units.get(u)?;
        let (x, y) = self.h.path_position(u);
        let level = game
            .lists
            .unit(u)
            .and_then(|e| e.room())
            .and_then(|rm| self.h.drlg.level_id(game, rm))
            .unwrap_or(0);
        use crate::monsters::init::type_flag as tf;
        let mut rec = MonsterRecord {
            x,
            y,
            class: r.class,
            guid: r.guid,
            unit_flags: r.flags,
            flags_ex: r.flags2,
            bits: ia::monster_bits(
                self.type_flag(u, tf::BOSS),
                self.type_flag(u, tf::CHAMPION),
                r.mode,
                0,
                self.type_flag(u, tf::MINION),
                false,
                i32::from(self.h.x.alignment(u)),
                r.node_index,
                self.type_flag(u, tf::SUPERUNIQUE),
            ),
            owner_guid: -1,
            owner_value: 0,
            ai_state: 0,
            level,
            name_seed: 0x1506,
            umods: [0; 9],
            superunique: 0,
            stat13: self.stats.unit_total(u, STAT_13, 0),
            max_life: self.stats.unit_total(u, STAT_MAXLIFE, 0),
            life: self.stats.unit_base(u, STAT_LIFE, 0),
            frame: game.frame,
            x4c: 0,
            x50: 0,
            path: None,
        };
        // The fields of the monster data (level, name seed, umods,
        // superunique index) when the world is lent.
        if let Some(m) = self.h.monster_data(u) {
            rec.bits |= if m.death_counted {
                REGION_DEATH_COUNTED
            } else {
                0
            };
            rec.bits |= if m.not_counted { REGION_NOT_COUNTED } else { 0 };
            rec.level = u32::try_from(m.level_id).unwrap_or(0);
            rec.name_seed = m.name_seed;
            rec.umods = m.umods;
            rec.superunique = m.boss_hc_idx;
        }
        self.store_ai_fields(u, &mut rec);
        self.h.x.monster_record_extra(u, &mut rec);
        Some(rec)
    }

    /// The AI part of the store: the AI special state and the `x4c` / `x50`
    /// fields: the unit-flag markers (flags 0x4000000: x50 |= 1; 0x20000:
    /// x50 |= 2; x4c = marker), then for npc / interact / inTown classes the
    /// home command's point when its x is nonzero, and for classes 267, 243
    /// and superunique index 6 the home command's point (created empty when
    /// absent). Imp (0x44, 0x21F) and classes 0x137 / 0x138 write other
    /// values into `x4c`: not stored (not yet modelled).
    fn store_ai_fields(&self, u: UnitId, rec: &mut MonsterRecord) {
        const HOME: i32 = 10;
        let class = rec.class as usize;
        let flags = rec.unit_flags;
        if flags & 0x400_0000 != 0 {
            rec.x50 |= 1;
            rec.x4c = ia::MARKER;
        }
        if flags & 0x2_0000 != 0 {
            rec.x4c = ia::MARKER;
            rec.x50 |= 2;
        }
        let gate = self
            .h
            .tables
            .combat
            .monstats
            .get(class)
            .is_some_and(|m| m.npc || m.interact || m.intown);
        let Some(ctl) = self.h.ai.as_ref().and_then(|a| a.control(u)) else {
            return;
        };
        rec.ai_state = ctl.special_state as i32;
        // The home command, "created empty" when absent (the unit is
        // freed right after, so only its params matter).
        let home = ctl
            .commands
            .iter()
            .find(|c| c.params[0] == HOME)
            .map_or([HOME, 0, 0, 0, 0], |c| c.params);
        if gate && home[1] != 0 {
            rec.x4c = home[1] as u32;
            rec.x50 = home[2] as u32;
        }
        // After the home point: a map-AI path the unit holds moves into the
        // record.
        if gate {
            rec.path = ctl
                .map_ai
                .as_ref()
                .map(|n| n.iter().map(|m| (m.action, m.x, m.y)).collect());
        }
        if class == 0x10B
            || class == 0xF3
            || (rec.bits & ia::mrec::SUPERUNIQUE != 0 && rec.superunique == 6)
        {
            rec.x4c = home[1] as u32;
            rec.x50 = home[2] as u32;
        }
    }

    /// The compress of one unit of `room`.
    pub fn compress_unit(&mut self, game: &mut Game, room: RoomId, u: UnitId) {
        let Some(r) = self.units.get(u) else {
            return;
        };
        let (ty, class, mode, flags, node) = (r.ty, r.class, r.mode, r.flags, r.node_index);
        let dead = r.is_dead();
        let save = ia::save_flag(self.save_monsters(game, room), flags);
        let c: Compress = if ty == UnitType::Monster {
            let f = MonsterFacts {
                save,
                mode,
                dead,
                udead_state: self.h.x.udead_state(u),
                alignment: i32::from(self.h.x.alignment(u)),
                node_index: node,
                class,
                type_flags: [8u32, 0x10]
                    .iter()
                    .filter(|&&m| self.h.monster_flag(u, m))
                    .fold(0, |a, m| a | m),
                unit_flags: flags,
                player_pet: self.h.x.player_pet(u),
                restore: self.monster_restore(u),
            };
            ia::compress_monster(&f, || self.h.x.room_seed_step(game, room))
        } else {
            let (nr, b78, virgins) = if ty == UnitType::Object {
                self.object_restore_facts(u)
            } else {
                (false, false, false)
            };
            ia::compress_other(&CompressFacts {
                ty,
                class,
                mode,
                save,
                player_body: ty == UnitType::Player && self.h.x.unit_has_state(u, STATE_PLAYERBODY),
                object_no_restore: nr,
                object_byte78_2: b78,
                object_restore_virgins: virgins,
            })
        };
        let key = self.node_key(game, room);
        if c.cancel_events {
            game.timers.cancel_unit_events(u, EVENT_TYPE_1, None);
        }
        if c.neutral_mode {
            if let Some(r) = self.units.get_mut(u) {
                r.mode = ia::MODE_NEUTRAL;
            }
        }
        if c.mark_kept {
            if let Some(r) = self.units.get_mut(u) {
                r.flags2 |= ia::FLAGS2_KEPT;
            }
        }
        let mut stored = false;
        if c.store {
            if let Some((act, origin)) = key {
                stored = self.store_record(game, u, ty, act, origin);
            }
        }
        if ty == UnitType::Item {
            // The store frees the item. An item without a record (no item
            // writer in this host) is not freed here; the freed room
            // unlinks it. Every ground item gets its record and is freed;
            // only a host without item tables has none.
            if stored {
                self.remove(game, u);
            }
            return;
        }
        if c.detach {
            self.h.x.detach_path(u, room);
            let r = game.lists.room_remove(u);
            if let Err(e) = r {
                self.unit_error(crate::game::GameError::from(e).into());
            }
        } else if c.free {
            self.remove(game, u);
        }
    }

    /// Pushes the unit's record; `false`: no record.
    fn store_record(
        &mut self,
        game: &mut Game,
        u: UnitId,
        ty: UnitType,
        act: u8,
        at: (i32, i32),
    ) -> bool {
        match ty {
            UnitType::Monster => {
                game.timers.cancel_unit_events(u, EVENT_TYPE_2, None);
                let Some(rec) = self.monster_record(game, u) else {
                    return false;
                };
                let Some(s) = self.h.inactive.as_mut() else {
                    return false;
                };
                s.push_monster(act, at, rec);
                true
            }
            UnitType::Item => {
                let Some(rec) = self.item_record_of(u).or_else(|| self.h.x.item_record(u)) else {
                    return false;
                };
                let Some(s) = self.h.inactive.as_mut() else {
                    return false;
                };
                s.push_item(act, at, rec);
                true
            }
            _ => {
                let Some(r) = self.units.get(u) else {
                    return false;
                };
                let guid = r.guid;
                let (x, y) = self.h.path_position(u);
                let mut rec = OtherRecord {
                    x,
                    y,
                    ty: ty as u8,
                    class: r.class,
                    mode: r.mode,
                    frame: game.frame,
                    unit_flags: r.flags,
                    flags_ex: r.flags2,
                    v20: if ty == UnitType::Player { guid } else { 0 },
                    ..OtherRecord::default()
                };
                if ty == UnitType::Object
                    && !self.object_record(game, u, &mut rec)
                    && ia::PORTAL_CLASSES.contains(&rec.class)
                {
                    rec.v20 = guid;
                }
                self.h.x.other_record_extra(u, &mut rec);
                let Some(s) = self.h.inactive.as_mut() else {
                    return false;
                };
                s.push_other(act, at, rec);
                true
            }
        }
    }

    /// The object fields of an "other" record from the object state; `false`: no object state
    /// or no data for `u`.
    fn object_record(&mut self, game: &mut Game, u: UnitId, rec: &mut OtherRecord) -> bool {
        let Some(row) = self.object_row(u).cloned() else {
            return false;
        };
        let Some(data) = self
            .h
            .objects
            .as_ref()
            .and_then(|o| o.control.data.get(&u))
            .copied()
        else {
            return false;
        };
        // A mode-1 object whose mode 1 does not cycle (`CycleAnim1`) and
        // that has a mode 2 (`Mode2`) is set to mode 2.
        if rec.mode == 1 && row.cycleanim1 == 0 && row.mode2 != 0 {
            if !self.object_set_mode(game, u, 2) {
                if let Some(r) = self.units.get_mut(u) {
                    r.mode = 2;
                }
            }
            rec.mode = 2;
        }
        if row.subclass & SUBCLASS_SHRINE != 0 {
            // The pending event 5 time: high 16 bits in `v20`, low 16 in
            // `v24`.
            let t = &game.timers;
            let at = t
                .unit_timers(u)
                .into_iter()
                .filter(|&id| t.event(id).is_some_and(|e| e.0 == OBJECT_EVENT_5))
                .filter_map(|id| t.expire(id))
                .filter(|&e| e > 0)
                .min()
                .unwrap_or(0) as u32;
            rec.v20 = at >> 16;
            rec.v24 = at & 0xFFFF;
        } else if GUID_CLASSES.contains(&rec.class) {
            rec.v20 = data.guid;
        } else {
            rec.v24 = u32::from(data.spark);
        }
        rec.byte4 = data.interact;
        rec.b8 = data.drop_code;
        true
    }

    /// The restore of `room`'s first population.
    pub fn restore_inactive(&mut self, game: &mut Game, room: RoomId) {
        let Some((act, (x, y))) = self.node_key(game, room) else {
            return;
        };
        let Some(node) = self.h.inactive.as_mut().and_then(|s| s.take(act, x, y)) else {
            return;
        };
        let order = ia::restore_order(node);
        let gate = self.h.x.sanctuary_gate(game, room);
        for rec in &order.monsters {
            let how = ia::restore_monster(rec, gate);
            self.restore_monster(game, room, rec, how);
        }
        for rec in &order.items {
            if let Some(expiry) =
                ia::restore_item_expiry_for(rec.expiry, game.frame, game.ticks(ia::RESTORE_EXPIRY))
            {
                self.restore_item_record(game, room, rec, expiry);
            }
        }
        for rec in &order.others {
            self.restore_other(game, room, rec);
        }
    }

    /// The existing unit of `ty` and `guid` (a kept unit, detached by its
    /// compress) placed again in `room` at (x, y).
    fn replace_kept(
        &mut self,
        game: &mut Game,
        room: RoomId,
        ty: UnitType,
        guid: u32,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        let u = game.lists.find_unit(ty, guid)?;
        if let Err(e) = game.lists.room_insert(u, room) {
            self.unit_error(crate::game::GameError::from(e).into());
            return None;
        }
        self.path_place(game, u, x, y);
        Some(u)
    }

    /// The restore of one monster record.
    fn restore_monster(
        &mut self,
        game: &mut Game,
        room: RoomId,
        rec: &MonsterRecord,
        how: MonsterRestore,
    ) {
        use ia::mrec;
        let (kind, mode) = match how {
            MonsterRestore::Skip => return,
            MonsterRestore::Replace => {
                // The unit of the record's GUID is placed again when it
                // still exists; the record's stored owner chain is only
                // freed. Nothing is re-linked: the kept pet was never freed, so it keeps
                // its owner and its owner's lists.
                self.replace_kept(game, room, UnitType::Monster, rec.guid, rec.x, rec.y);
                return;
            }
            MonsterRestore::Spawn { kind, mode } => (kind, mode),
        };
        // Spawned again with the stored GUID; the allocation runs the
        // monster type init on the lent world (created anew). The record's
        // alignment (1 with bit 0x80, else 2 with bit 0x100, else 0) is set
        // on the new unit after the spawn (below); the allocation's allied
        // mark already follows it.
        let align: u8 = if rec.bits & mrec::ALIGN_1 != 0 {
            1
        } else if rec.bits & mrec::ALIGN_2 != 0 {
            2
        } else {
            0
        };
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: rec.class,
            room: Some(room),
            add: true,
            fixed_guid: Some(rec.guid),
            mode,
            allied: align == 2,
        };
        let spawned = if kind == SpawnKind::Plain {
            self.restore_plain(game, room, rec, mode)
        } else {
            None
        };
        let Some(u) = spawned.or_else(|| self.allocate(game, &req, rec.x, rec.y)) else {
            return;
        };
        // Restore suppresses a new spawn count; the saved accounting eligibility
        // and first-death marker must survive that suppression.
        if let Some(m) = self.h.monster_data_mut(u) {
            m.not_counted = rec.bits & REGION_NOT_COUNTED != 0;
            m.death_counted = rec.bits & REGION_DEATH_COUNTED != 0;
        }
        // PROVISIONAL: the restore should run the whole
        // boss-mod case again through the creation
        // (chains, umods, unit flags,
        // states, the Ancients' items); D2RUST re-runs only the `Chain`
        // states, the Ancients' items); D2RUST re-runs only the `Chain`
        // steps here (the barbarian cages' prison door 434 -> chain 32),
        // the rest is `restore_boss_fields`; the remaining steps are not yet
        // re-run on a restore.
        self.relink_boss_chains(u);
        // The boss / minion fields and their umod init belong to the
        // creation, before the record's stats below.
        if kind != SpawnKind::Plain {
            self.restore_boss_fields(game, u, rec, kind);
        }
        // After the spawn: stat 13 comes back from the record; max life (stat 7) and life (stat
        // 6) too when the record is younger than 7,500 frames (0x1D4C, unsigned), else the unit
        // comes back at full life (stat 6 := the stored max life). The spawn's own fresh HP
        // roll is overwritten (replay bloodmoor-bar-10k frame 1461).
        self.stats.unit_set(&mut *self.h, u, 13, rec.stat13, 0);
        self.stats.unit_set(&mut *self.h, u, 7, rec.max_life, 0);
        let age = game.frame.wrapping_sub(rec.frame) as u32;
        let life = if age < game.ticks(0x1D4C) as u32 {
            rec.life
        } else {
            rec.max_life
        };
        self.stats.unit_set(&mut *self.h, u, 6, life, 0);
        // The AI is installed again in its stored special state. The NPC
        // follow-up is `restore_home` below, except for classes 311 / 312
        // (two monster-data bytes := the low byte of `x4c`) and 68 / 543
        // (a further call with `x4c`). PROVISIONAL: those four classes are
        // not modelled.
        self.restore_ai_state(game, u, rec.ai_state);
        self.restore_home(u, rec);
        // The alignment is set after the umods.
        self.set_alignment(game, u, align);
    }

    /// The boss / minion fields of a restored record: the saved umods
    /// copied in with the boss's champion flag or the minion's type flag
    /// 0x10, then the umod init (the fixed umods 1-4, then the unit's list),
    /// then the boss's name seed and superunique index.
    fn restore_boss_fields(
        &mut self,
        game: &mut Game,
        u: UnitId,
        rec: &MonsterRecord,
        kind: SpawnKind,
    ) {
        use crate::monsters::init::type_flag as tf;
        use ia::mrec;
        let Some(m) = self.h.monster_world.as_mut().and_then(|w| w.monster_mut(u)) else {
            return;
        };
        m.umods = rec.umods;
        match kind {
            SpawnKind::Unique => {
                m.type_flags |= tf::BOSS;
                if rec.bits & mrec::CHAMPION != 0 {
                    m.type_flags |= tf::CHAMPION;
                }
            }
            SpawnKind::Minion => m.type_flags |= tf::MINION,
            SpawnKind::Plain => return,
        }
        let mut sim = Sim {
            game: &mut *game,
            units: &mut *self.units,
            stats: &mut *self.stats,
            data: self.data,
        };
        self.h
            .restore_umod_init(&mut sim, u, kind == SpawnKind::Unique);
        if kind != SpawnKind::Unique {
            return;
        }
        let Some(m) = self.h.monster_world.as_mut().and_then(|w| w.monster_mut(u)) else {
            return;
        };
        m.name_seed = rec.name_seed;
        if rec.bits & mrec::SUPERUNIQUE != 0 {
            m.type_flags |= tf::SUPERUNIQUE;
            m.boss_hc_idx = rec.superunique;
        }
    }

    /// A plain record's re-spawn: the whole monster creation with the
    /// record's GUID at the record point, spread −1 (the point itself), then
    /// spread 5, then at the spawn point, last at the free point
    /// ([`spawn::restore_plain`]); flags 0x62 (0x6A with record bit 0x40) |
    /// 0x80 for a `flying` class. `None`: no lent
    /// monster world, or no point at all (the caller allocates at the
    /// record point).
    ///
    /// [`spawn::restore_plain`]: crate::monsters::population::spawn::restore_plain
    fn restore_plain(
        &mut self,
        game: &mut Game,
        room: RoomId,
        rec: &MonsterRecord,
        mode: u32,
    ) -> Option<UnitId> {
        use ia::mrec;
        let mut flags: u16 = if rec.bits & mrec::UNIT_573540 != 0 {
            0x6A
        } else {
            0x62
        };
        let flying = usize::try_from(rec.class)
            .ok()
            .and_then(|c| self.h.tables.combat.monstats.get(c))
            .is_some_and(|m| m.flying);
        if flying {
            flags |= 0x80;
        }
        let mut sim = Sim {
            game: &mut *game,
            units: &mut *self.units,
            stats: &mut *self.stats,
            data: self.data,
        };
        self.h
            .with_monster_world(|w, h| {
                w.restore_spawn(
                    &mut sim,
                    h,
                    room,
                    rec.x,
                    rec.y,
                    rec.guid,
                    rec.class as i32,
                    mode as u8,
                    flags,
                )
            })
            .flatten()
            .flatten()
    }

    /// The restore tail for `x4c` / `x50`: for npc / interact / inTown
    /// classes the unit's home command (type 10, found or created) gets the
    /// record's point and param 3 := 0, whatever the values (an NPC stored
    /// before its first think has none, so its home think does not run
    /// again); for classes 267, 243 and superunique 6 the same when
    /// nonzero; the unit flags 0x4000000 / 0x20000 come back from the
    /// marker. The imp and class 0x137 / 0x138 cases are not modelled.
    fn restore_home(&mut self, u: UnitId, rec: &MonsterRecord) {
        use crate::monsters::ai::AiCommand;
        const HOME: i32 = 10;
        let class = rec.class as usize;
        let gate = self
            .h
            .tables
            .combat
            .monstats
            .get(class)
            .is_some_and(|m| m.npc || m.interact || m.intown);
        let su6 = rec.bits & ia::mrec::SUPERUNIQUE != 0 && rec.superunique == 6;
        if let Some(ctl) = self.h.ai.as_mut().and_then(|a| a.control_mut(u)) {
            let mut set = |x: u32, y: u32, third: bool| {
                let i = match ctl.commands.iter().position(|c| c.params[0] == HOME) {
                    Some(i) => i,
                    None => {
                        ctl.commands.push(AiCommand {
                            params: [HOME, 0, 0, 0, 0],
                        });
                        ctl.commands.len() - 1
                    }
                };
                ctl.commands[i].params[1] = x as i32;
                ctl.commands[i].params[2] = y as i32;
                if third {
                    ctl.commands[i].params[3] = 0;
                }
            };
            if gate {
                set(rec.x4c, rec.x50, true);
                // The stored path becomes the unit's map AI again.
                if let Some(path) = &rec.path {
                    ctl.map_ai = Some(
                        path.iter()
                            .map(|&(action, x, y)| crate::monsters::ai::MapNode { action, x, y })
                            .collect(),
                    );
                }
            }
            if (rec.class == 0x10B || rec.class == 0xF3 || su6) && (rec.x4c != 0 || rec.x50 != 0) {
                set(rec.x4c, rec.x50, false);
            }
        }
        if rec.x4c == ia::MARKER {
            if let Some(r) = self.units.get_mut(u) {
                if rec.x50 & 1 != 0 {
                    r.flags |= 0x400_0000;
                }
                if rec.x50 & 2 != 0 {
                    r.flags |= 0x2_0000;
                }
            }
        }
    }

    /// Installs the AI on a restored monster in its stored special state.
    fn restore_ai_state(&mut self, game: &mut Game, u: UnitId, state: i32) {
        use crate::monsters::ai;
        let Some(mut store) = self.h.ai.take() else {
            return;
        };
        let t = self.h.tables.clone();
        let info = self.h.ai_info;
        {
            let mut cx = ai::Ctx {
                tables: ai::AiTables {
                    monstats: &t.combat.monstats,
                    monstats2: &t.combat.monstats2,
                    levels: &t.levels,
                    skill_modes: &t.skill_modes,
                    skills: &t.skills.skills,
                    missiles: &t.skills.missiles,
                },
                info,
                store: &mut store,
                world: &mut *self,
            };
            ai::install(game, &mut cx, u, state as u32);
        }
        self.h.ai = Some(store);
    }

    /// The `Chain` steps of the unit's boss mods.
    fn relink_boss_chains(&mut self, u: UnitId) {
        if let Some(class) = self.units.get(u).map(|r| r.class) {
            self.link_boss_chains(u, class);
        }
    }

    /// The restore of one "other" record.
    fn restore_other(&mut self, game: &mut Game, room: RoomId, rec: &OtherRecord) {
        let Some(&ty) = UnitType::ALL.get(usize::from(rec.ty)) else {
            return;
        };
        if rec.flags_ex & ia::FLAGS2_KEPT != 0 {
            let Some(u) = self.replace_kept(game, room, ty, rec.v20, rec.x, rec.y) else {
                return;
            };
            if let Some(r) = self.units.get_mut(u) {
                r.flags |= UNIT_FLAG_10;
                if ty == UnitType::Player {
                    // Fatal (error 0x156) outside mode 0x11.
                    r.mode = PLAYER_BODY_MODE;
                }
            }
            if ty == UnitType::Object {
                if let Some(d) = self.object_data_mut(u) {
                    d.interact = rec.byte4;
                }
            }
            return;
        }
        // A new unit (new GUID), unit flags 0x3000000. A tile keeps its
        // stored flags under them ([`View::create_tile`]).
        if ty == UnitType::Tile {
            self.create_tile(
                game,
                room,
                rec.class,
                rec.x,
                rec.y,
                rec.mode,
                rec.unit_flags,
            );
            return;
        }
        let u = if ty == UnitType::Object {
            let mode = u8::try_from(rec.mode).unwrap_or(u8::MAX);
            match self.create_object(game, room, rec.class, rec.x, rec.y, mode) {
                Some(u) => Some(u),
                None if self.h.objects.is_none() => self.allocate_other(game, room, ty, rec),
                None => None,
            }
        } else {
            self.allocate_other(game, room, ty, rec)
        };
        let Some(u) = u else {
            return;
        };
        if let Some(r) = self.units.get_mut(u) {
            r.flags |= PRESET_UNIT_FLAGS;
        }
        if ty == UnitType::Object {
            self.restore_object(game, u, rec);
        }
    }

    fn allocate_other(
        &mut self,
        game: &mut Game,
        room: RoomId,
        ty: UnitType,
        rec: &OtherRecord,
    ) -> Option<UnitId> {
        let req = AllocRequest {
            ty,
            class: rec.class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: rec.mode,
            allied: false,
        };
        self.allocate(game, &req, rec.x, rec.y)
    }

    /// The object part of the restore of a new object `u`.
    fn restore_object(&mut self, game: &mut Game, u: UnitId, rec: &OtherRecord) {
        let Some(row) = self.object_row(u).cloned() else {
            return;
        };
        let at = (rec.v20 << 16 | rec.v24 & 0xFFFF) as i32;
        if row.subclass & SUBCLASS_SHRINE != 0 && at > 0 {
            // Event 5 at max(t, frame + 1), the byte and its shrine record.
            let when = at.max(game.frame.wrapping_add(1));
            if let Err(e) = game.schedule_event(u, u32::from(OBJECT_EVENT_5), when, None, 0, 0) {
                self.unit_error(e.into());
            }
            let shrines = self
                .h
                .objects
                .as_ref()
                .map_or(0, |o| o.tables.shrines.len());
            if let Some(d) = self.object_data_mut(u) {
                d.interact = rec.byte4;
                d.shrine = (usize::from(rec.byte4) < shrines).then_some(u16::from(rec.byte4));
            }
        } else if row.subclass & SUBCLASS_WELL != 0 {
            // The charges regrow. M = 2 · `Parm2` as a byte; a
            // stored byte at or above M leaves the new object as created.
            // Else, with P = `Parm0`: when the store frame is
            // before frame + P, n = (frame − store) / P capped at 1, the
            // byte b + n (u8) capped at M, mode := 2 −
            // byte / `Parm2`; a mode past 2 (negative) becomes 0 with the
            // byte at M and nothing scheduled. Then the byte is written,
            // and below M one event 2 per missing charge k = 1..M − byte at
            // frame + k · P.
            let cap = row.parm2.wrapping_mul(2) as u8;
            if rec.byte4 >= cap {
                return;
            }
            let period = game.ticks(row.parm0 as i32);
            if period == 0 {
                // A fatal error in the game (0x18B): nothing here.
                return;
            }
            let mut byte = rec.byte4;
            if rec.frame < game.frame.wrapping_add(period) {
                let n = game
                    .frame
                    .wrapping_sub(rec.frame)
                    .wrapping_div(period)
                    .min(1);
                byte = byte.wrapping_add(n as u8).min(cap);
                let mode = 2i32.wrapping_sub(i32::from(byte) / row.parm2 as i32) as u32;
                if let Some(r) = self.units.get_mut(u) {
                    r.mode = if mode > 2 { 0 } else { mode };
                }
                if mode > 2 {
                    if let Some(d) = self.object_data_mut(u) {
                        d.interact = cap;
                    }
                    return;
                }
            }
            if let Some(d) = self.object_data_mut(u) {
                d.interact = byte;
            }
            for k in 1..=i32::from(cap.saturating_sub(byte)) {
                let when = game.frame.wrapping_add(k.wrapping_mul(period));
                if let Err(e) = game.schedule_event(u, OBJECT_EVENT_2, when, None, 0, 0) {
                    self.unit_error(e.into());
                }
            }
        } else if let Some(d) = self.object_data_mut(u) {
            d.interact = rec.byte4;
            d.spark = rec.v24 as u8;
            if rec.b8 != 0 {
                d.drop_code = rec.b8;
            }
        }
    }

    /// Tick step 9 for a tile while the inactive store is off
    /// ([`ActionHooks::fallback_tiles`]): the tile's record
    /// stored and the tile freed, as the store does.
    /// A D2RUST-only path: the game's rule is the store.
    fn compress_fallback_tile(&mut self, game: &mut Game, room: RoomId, u: UnitId) {
        let Some((act, at)) = self.node_key(game, room) else {
            return;
        };
        let Some(r) = self.units.get(u) else {
            return;
        };
        let (x, y) = self.h.path_position(u);
        let rec = OtherRecord {
            x,
            y,
            ty: UnitType::Tile as u8,
            class: r.class,
            mode: r.mode,
            frame: game.frame,
            unit_flags: r.flags,
            flags_ex: r.flags2,
            ..OtherRecord::default()
        };
        self.h.fallback_tiles.push_other(act, at, rec);
        self.remove(game, u);
    }

    /// The restore of [`ActionHooks::fallback_tiles`] for `room`: its
    /// tiles re-created from the list head (reverse store order), new
    /// GUIDs, as the other records. The caller runs it
    /// after the host's restore (monsters and items first).
    /// A fallback for hosts without the store: the game's rule is the store.
    pub fn restore_fallback_tiles(&mut self, game: &mut Game, room: RoomId) {
        let Some((act, (x, y))) = self.node_key(game, room) else {
            return;
        };
        let Some(node) = self.h.fallback_tiles.take(act, x, y) else {
            return;
        };
        for rec in &ia::restore_order(node).others {
            self.restore_other(game, room, rec);
        }
    }
}

impl<X: Pending> ActionSim<X> {
    /// The tick's compress when the store is on.
    /// Off: only tiles, into [`ActionHooks::fallback_tiles`].
    pub fn compress(&mut self, game: &mut Game, unit: UnitId) {
        let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
            return;
        };
        if self.sys.hooks.inactive.is_none() {
            if game
                .lists
                .unit(unit)
                .is_some_and(|e| e.ty == UnitType::Tile)
            {
                self.with(game, |g, v| v.compress_fallback_tile(g, room, unit));
            }
            return;
        }
        self.with(game, |g, v| v.compress_unit(g, room, unit));
    }

    /// The restore when the store is on; `false`: off.
    pub fn restore(&mut self, game: &mut Game, room: RoomId) -> bool {
        if self.sys.hooks.inactive.is_none() {
            return false;
        }
        self.with(game, |g, v| v.restore_inactive(g, room));
        true
    }
}
