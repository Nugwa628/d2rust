//! The room-unit facts the draw order reads (`UnitFacts`) from the client
//! model: unit flags (+0xC4) and flag-ex (+0xC8) as the message rules
//! leave them, the states 7 `playerbody`, 143 `attached` and 146 `invis`,
//! the per-class table columns (monstats2 `unflatDead`, objects
//! `DrawUnder`) and the level gate of the sight test (`LOSDraw`).
//!
//! PROVISIONAL: a client missile's bit 0x10000 is the bridge's
//! `flat` (functions 2 and 11 at the animation end);
//! a server missile's reads 0. The sight test
//! runs over the client DRLG's collision grids ([`ClientRooms`]);
//! a unit it cannot place (no client DRLG, no local player or room, no
//! grid cell, a class without a size row) leaves `sight_hidden` `None`,
//! which the draw order refuses in a `LOSDraw` level. D2RUST's own,
//! unverified: the positions are the model's cells (no walk
//! prediction), an unplaced unit is at (0, 0).

use d2_data::tables::{decode_all, Leveldefs, Missiles, Monstats, Monstats2, Objects, Record};
use d2_sim::drlg::{CollisionGrid, Drlg, TileRect};
use d2_sim::path::CollisionRooms;
use d2_sim::units::RoomId;

use crate::bridge::world::{
    ClientUnit, ClientWorld, LevelRow, UnitKey, ITEM, MISSILE, MONSTER, OBJECT, PLAYER,
    VIEW_CLIENT_BIT,
};
use crate::rules::draw_order::sight::{sight_hidden, SightUnit};
use crate::rules::draw_order::UnitFacts;

use super::ViewError;

/// Unit flag +0xC4 bits the model holds.
pub const FLAG_2: u32 = 0x2;
pub const FLAG_4: u32 = 0x4;
pub const FLAG_200: u32 = 0x200;
pub const FLAG_ROOM_FREED: u32 = 0x80_0000;

/// States the draw order reads.
pub const STATE_PLAYERBODY: u8 = 7;
pub const STATE_ATTACHED: u8 = 143;
pub const STATE_INVIS: u8 = 146;

/// The table columns the facts read, by class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitFactTables {
    /// `monstats2` `unflatDead` by `monstats` row (through `MonStatsEx`);
    /// `None`: no `monstats2` row.
    pub unflat_dead: Vec<Option<bool>>,
    /// `objects` `DrawUnder` by class.
    pub draw_under: Vec<u8>,
    /// leveldefs `LOSDraw` by level id.
    pub los_draw: Vec<bool>,
    /// monstats2 `SizeX` (signed) by `monstats` row.
    pub monster_size: Vec<Option<i8>>,
    /// objects `SizeX` by class.
    pub object_size: Vec<i32>,
    /// missiles `Size` by missile id.
    pub missile_size: Vec<u8>,
}

/// The client act's active rooms as [`CollisionRooms`] (read only:
/// `grid_mut` answers none; the sight test never writes).
pub struct ClientRooms<'a>(pub &'a Drlg);

impl CollisionRooms for ClientRooms<'_> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        let r = self.0.drlg_room_of(room)?;
        self.0.active_room(r).map(|a| a.subtiles)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.0
            .drlg_room_of(room)
            .and_then(|r| self.0.active_room(r))
            .map_or(0, |a| a.adjacency.len())
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        let r = self.0.drlg_room_of(room)?;
        let n = *self.0.active_room(r)?.adjacency.get(i)?;
        self.0.active_room(n).map(|a| a.id)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        let r = self.0.drlg_room_of(room)?;
        self.0.active_room(r).map(|a| &a.collision)
    }
    fn grid_mut(&mut self, _room: RoomId) -> Option<&mut CollisionGrid> {
        None
    }
}

impl UnitFactTables {
    /// From the typed rows.
    pub fn from_rows(
        monstats: &[Monstats],
        monstats2: &[Monstats2],
        objects: &[Objects],
        defs: &[Leveldefs],
        missiles: &[Missiles],
    ) -> Self {
        Self {
            unflat_dead: monstats
                .iter()
                .map(|m| {
                    monstats2
                        .get(usize::from(m.monstatsex))
                        .map(|r| r.unflatdead)
                })
                .collect(),
            draw_under: objects.iter().map(|o| o.drawunder).collect(),
            los_draw: defs.iter().map(|d| d.losdraw != 0).collect(),
            monster_size: monstats
                .iter()
                .map(|m| {
                    monstats2
                        .get(usize::from(m.monstatsex))
                        .map(|r| r.sizex as i8)
                })
                .collect(),
            object_size: objects.iter().map(|o| o.sizex as i32).collect(),
            missile_size: missiles.iter().map(|m| m.size).collect(),
        }
    }
}

/// The columns from the user's tables.
pub fn load(archives: &dyn d2_data::bin::TableFiles) -> Result<UnitFactTables, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    fn all<R: Record>(set: &d2_data::bin::BinSet, name: &str) -> Result<Vec<R>, String> {
        let t = set
            .table(name)
            .ok_or_else(|| format!("{name} not loaded"))?;
        decode_all(t).map_err(|e| e.to_string())
    }
    Ok(UnitFactTables::from_rows(
        &all::<Monstats>(&set, "monstats")?,
        &all::<Monstats2>(&set, "monstats2")?,
        &all::<Objects>(&set, "objects")?,
        &all::<Leveldefs>(&set, "leveldefs")?,
        &all::<Missiles>(&set, "missiles")?,
    ))
}

fn unresolved(what: &'static str, message: String) -> ViewError {
    ViewError::Unresolved {
        what,
        spec: "render/draw-order.md",
        message,
    }
}

/// The unit flag word +0xC4 as far as the model writes it.
pub fn flags_of(unit: &ClientUnit) -> u32 {
    let mut f = 0;
    if unit.flag_2 == Some(true) && !unit.quest_untargetable {
        f |= FLAG_2;
    }
    if unit.flag_4 {
        f |= FLAG_4;
    }
    if unit.flag_200 {
        f |= FLAG_200;
    }
    if unit.room_freed {
        f |= FLAG_ROOM_FREED;
    }
    if unit.flag_flat {
        f |= crate::rules::draw_order::UNIT_FLAT;
    }
    f
}

/// Writes the model's facts of `unit` into `f` (flags, flag-ex, the three
/// states, `unflatDead`, `DrawUnder`); `f.flag_ex` keeps the bits it
/// already has (the draw's own, 0x80).
pub fn fill_model(
    f: &mut UnitFacts,
    world: &ClientWorld,
    unit: &ClientUnit,
    t: &UnitFactTables,
) -> Result<(), ViewError> {
    f.flags |= flags_of(unit);
    // A client missile reaches the view under its view key
    // (`ClientWorld::view_unit`); the bridge keys it without the bit.
    let model_key = UnitKey::new(unit.key.unit_type, unit.key.guid & !VIEW_CLIENT_BIT);
    if unit.key.unit_type == MISSILE
        && world
            .objclient
            .missiles
            .get(&model_key)
            .is_some_and(|m| m.flat)
    {
        f.flags |= crate::rules::draw_order::UNIT_MISSILE_FLAT;
    }
    f.flag_ex |= unit.flag_ex;
    f.playerbody = unit.states.contains(&STATE_PLAYERBODY);
    f.attached = unit.states.contains(&STATE_ATTACHED);
    f.invis = unit.states.contains(&STATE_INVIS);
    let class = unit.class as usize;
    match unit.key.unit_type {
        MONSTER => {
            f.unflat_dead = t.unflat_dead.get(class).copied().flatten().ok_or_else(|| {
                unresolved(
                    "monstats2 unflatDead",
                    format!("monster class {class} has no monstats2 row"),
                )
            })?;
        }
        OBJECT => {
            f.draw_under = t.draw_under.get(class).copied().ok_or_else(|| {
                unresolved(
                    "objects DrawUnder",
                    format!("object class {class} past the objects rows"),
                )
            })?;
        }
        _ => {}
    }
    Ok(())
}

/// The size of `unit` for the sight line.
fn sight_size(unit: &ClientUnit, t: &UnitFactTables) -> Option<i32> {
    let class = unit.class as usize;
    match unit.key.unit_type {
        PLAYER => Some(2),
        MONSTER => t.monster_size.get(class).copied().flatten().map(i32::from),
        OBJECT => t.object_size.get(class).copied(),
        MISSILE => t.missile_size.get(class).map(|&s| i32::from(s)),
        ITEM => Some(1),
        _ => Some(0),
    }
}

/// over any collision rooms: whether `unit` is hidden from `local`
/// (`local_room` its room); `None` when the test cannot be answered.
pub fn sight_with<R: CollisionRooms + ?Sized>(
    rooms: &R,
    los_draw: bool,
    local_room: Option<RoomId>,
    local: &ClientUnit,
    unit: &ClientUnit,
    t: &UnitFactTables,
) -> Option<bool> {
    let end = |u: &ClientUnit, room| {
        let (x, y) = u.cell();
        Some(SightUnit {
            room,
            x: i32::from(x),
            y: i32::from(y),
            size: sight_size(u, t)?,
        })
    };
    let a = end(local, local_room)?;
    let b = end(unit, None)?;
    sight_hidden(los_draw, &a, &b, rooms).ok()
}

/// The sight test's answer for `unit` in the
/// local player's level: `Some(false)` when its `LOSDraw` is 0, else the
/// line test over the client DRLG ([`sight_with`]); `None` when it cannot
/// be run (see the module doc).
pub fn sight_gate(world: &ClientWorld, unit: &ClientUnit, t: &UnitFactTables) -> Option<bool> {
    let level = world.player_level()?;
    if !t.los_draw.get(usize::from(level)).copied().unwrap_or(false) {
        return Some(false);
    }
    let drlg = &world.drlg.as_ref()?.drlg;
    let local = world.units.get(&world.local_player?)?;
    let room = drlg.active_room(world.local_room()?.room)?.id;
    sight_with(&ClientRooms(drlg), true, Some(room), local, unit, t)
}

/// The facts of a room unit from the model alone.
pub fn model_facts(
    world: &ClientWorld,
    unit: &ClientUnit,
    t: &UnitFactTables,
    _levels: Option<&[LevelRow]>,
) -> Result<UnitFacts, ViewError> {
    let mut f = UnitFacts {
        unit_type: unit.key.unit_type,
        mode: unit.mode,
        local: world.local_player == Some(unit.key),
        sight_hidden: sight_gate(world, unit, t),
        ..UnitFacts::default()
    };
    fill_model(&mut f, world, unit, t)?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{view_key, PLAYER};
    use crate::rules::draw_order::{is_flat, UNIT_EX_VISIBLE};
    use d2_sim::drlg::collision::bits;

    fn tables() -> UnitFactTables {
        UnitFactTables {
            unflat_dead: vec![Some(false), Some(true), None],
            draw_under: vec![0, 2, 1],
            los_draw: vec![false, true],
            monster_size: vec![Some(1), Some(1), None],
            object_size: vec![1, 1, 1],
            missile_size: Vec::new(),
        }
    }

    fn unit(ty: u8, class: u32, mode: u32) -> ClientUnit {
        let mut u = ClientUnit::new(UnitKey::new(ty, 1));
        u.class = class;
        u.mode = mode;
        u
    }

    #[test]
    fn dead_unflat_dead_monster_is_not_flat() {
        let w = ClientWorld::default();
        let t = tables();
        let flat = model_facts(&w, &unit(MONSTER, 0, 12), &t, None).unwrap();
        assert!(is_flat(&flat));
        let upright = model_facts(&w, &unit(MONSTER, 1, 12), &t, None).unwrap();
        assert!(upright.unflat_dead && !is_flat(&upright));
        assert!(model_facts(&w, &unit(MONSTER, 2, 12), &t, None).is_err());
    }

    #[test]
    fn draw_under_object_is_flat() {
        let w = ClientWorld::default();
        let t = tables();
        let under = model_facts(&w, &unit(OBJECT, 1, 0), &t, None).unwrap();
        assert_eq!(under.draw_under, 2);
        assert!(is_flat(&under));
        // DrawUnder bit 1 only in mode 2.
        assert!(!is_flat(
            &model_facts(&w, &unit(OBJECT, 2, 0), &t, None).unwrap()
        ));
        assert!(is_flat(
            &model_facts(&w, &unit(OBJECT, 2, 2), &t, None).unwrap()
        ));
        assert!(!is_flat(
            &model_facts(&w, &unit(OBJECT, 0, 2), &t, None).unwrap()
        ));
        assert!(model_facts(&w, &unit(OBJECT, 9, 0), &t, None).is_err());
    }

    #[test]
    fn states_flags_and_flag_ex_come_from_the_model() {
        let w = ClientWorld::default();
        let t = tables();
        let mut u = unit(PLAYER, 0, 17);
        u.states.extend([STATE_PLAYERBODY, STATE_INVIS]);
        u.flag_ex = 0x2000_0000;
        u.flag_200 = true;
        u.flag_2 = Some(true);
        let f = model_facts(&w, &u, &t, None).unwrap();
        assert!(f.playerbody && f.invis && !f.attached);
        assert_eq!(f.flags, FLAG_200 | FLAG_2);
        assert_eq!(f.flag_ex, 0x2000_0000);
        assert_eq!(f.flag_ex & UNIT_EX_VISIBLE, 0);
        u.quest_untargetable = true;
        assert_eq!(flags_of(&u), FLAG_200);
    }

    /// The 0x51 init's `OrderFlag[mode]` (and the mode 1 → 2 turn's
    /// `OrderFlag2`) makes an object flat.
    #[test]
    fn object_order_flag_makes_it_flat() {
        let w = ClientWorld::default();
        let t = tables();
        let mut u = unit(OBJECT, 0, 0);
        assert!(!is_flat(&model_facts(&w, &u, &t, None).unwrap()));
        u.flag_flat = true;
        assert_eq!(flags_of(&u), crate::rules::draw_order::UNIT_FLAT);
        assert!(is_flat(&model_facts(&w, &u, &t, None).unwrap()));
    }

    /// A client missile whose function 2 / 11 ran to its animation end
    /// (the bridge's `flat`) has unit flag 0x10000 and files flat
    /// (in the base game the guts of a Bone Spear kill draw in the
    /// shadow pass).
    #[test]
    fn a_flat_client_missile_files_flat() {
        let mut w = ClientWorld::default();
        let t = tables();
        // The view hands the missile over under its view key
        // (`ClientWorld::view_unit`); the bridge keys it without the bit.
        let real = unit(MISSILE, 117, 0).key;
        let mut u = unit(MISSILE, 117, 0);
        u.key = view_key(real);
        w.objclient.missiles.insert(real, Default::default());
        assert!(!is_flat(&model_facts(&w, &u, &t, None).unwrap()));
        w.objclient.missiles.get_mut(&real).unwrap().flat = true;
        let f = model_facts(&w, &u, &t, None).unwrap();
        assert_eq!(f.flags, crate::rules::draw_order::UNIT_MISSILE_FLAT);
        assert!(is_flat(&f));
    }

    /// One room over (0, 0)–(19, 19).
    struct One(TileRect, CollisionGrid);

    impl One {
        fn new() -> Self {
            let r = TileRect {
                x: 0,
                y: 0,
                w: 20,
                h: 20,
            };
            Self(r, CollisionGrid::new(r))
        }
    }

    impl CollisionRooms for One {
        fn subtile_rect(&self, _: RoomId) -> Option<TileRect> {
            Some(self.0)
        }
        fn adjacent_count(&self, _: RoomId) -> usize {
            0
        }
        fn adjacent(&self, _: RoomId, _: usize) -> Option<RoomId> {
            None
        }
        fn grid(&self, _: RoomId) -> Option<&CollisionGrid> {
            Some(&self.1)
        }
        fn grid_mut(&mut self, _: RoomId) -> Option<&mut CollisionGrid> {
            Some(&mut self.1)
        }
    }

    fn placed(ty: u8, guid: u32, at: (u16, u16)) -> ClientUnit {
        let mut u = ClientUnit::new(UnitKey::new(ty, guid));
        u.position = Some(at);
        u
    }

    #[test]
    fn a_wall_between_player_and_unit_hides_it_in_a_los_draw_level() {
        let t = tables();
        let mut rooms = One::new();
        let player = placed(PLAYER, 1, (2, 5));
        let monster = placed(MONSTER, 2, (12, 5));
        let r = Some(RoomId(0));
        let run = |rooms: &One| sight_with(rooms, true, r, &player, &monster, &t);
        assert_eq!(run(&rooms), Some(false));
        // Collision bit 0x2 on the line hides it; bit 0x1 (wall) does not.
        *rooms.1.get_mut(7, 5).unwrap() = 1;
        assert_eq!(run(&rooms), Some(false));
        *rooms.1.get_mut(7, 5).unwrap() = bits::VISIBLE;
        assert_eq!(run(&rooms), Some(true));
        // LOSDraw 0: every unit passes.
        assert_eq!(
            sight_with(&rooms, false, r, &player, &monster, &t),
            Some(false)
        );
        // A class without a size row cannot be answered.
        let unknown = placed(MONSTER, 3, (12, 5));
        let mut unknown = unknown;
        unknown.class = 2;
        assert_eq!(sight_with(&rooms, true, r, &player, &unknown, &t), None);
    }
}
