//! Tests for act travel and portal pairs on the action wiring: the act change
//! (`wiring::path::act_change`), the portal pair creation, the Town Portal
//! cast (`wiring::action::town_portal`), entering the pair through the
//! object operate, and the room delete lists that remove the portals from
//! the clients.

use d2_data::tables::{Leveldefs, Levels, Objects};

use super::*;
use crate::drlg::room_flags;
use crate::units::hooks::Sim;
use crate::units::lists::client_state;
use crate::units::messages::remove_unit;
use crate::wiring::path::act_change::load_act;
use crate::wiring::path::PathCtx;
use crate::world::objects::ObjectTables;

/// Act 0's town, Act II's town, Duriel's Lair, the cow level.
const TOWN: u32 = 1;
const LUT: u32 = 40;
const LAIR: u32 = 73;
const COWS: u32 = 39;
const PORTAL: u32 = 59;
const PERMANENT: u32 = 60;
/// `velocitypercent` at its creation value 100.
const STAT_VELOCITY: u16 = 67;

/// `objects.txt` rows 0–60 with the town portal (59: init 11, operate 15)
/// and the permanent portal (60: init 12); `levels` with Act II's `Act`.
fn object_tables() -> Arc<ObjectTables> {
    let mut objects = vec![blank::<Objects>(); 61];
    objects[PORTAL as usize].initfn = 11;
    objects[PORTAL as usize].operatefn = 15;
    objects[PORTAL as usize].framecnt1 = 10 << 8;
    objects[PERMANENT as usize].initfn = 12;
    objects[PERMANENT as usize].operatefn = 15;
    let mut levels = vec![blank::<Levels>(); 150];
    for l in [LUT, LAIR] {
        levels[l as usize].act = 1;
    }
    Arc::new(ObjectTables {
        objects,
        shrines: Vec::new(),
        levels,
        objgroup: Vec::new(),
        leveldefs: vec![blank::<Leveldefs>(); 150],
    })
}

fn fx_with(rooms: &[(u32, TileRect)]) -> Fx {
    let mut fx = Fx::with_rooms(rooms);
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    Arc::make_mut(&mut h.tables).combat.charstats[0].walkvelocity = 6;
    fx.sim.create_objects(object_tables());
    fx.sim.hooks().x.quest_record = true;
    fx
}

/// Rooms A, B of level 2 and the town room at tiles (0, 16).
fn field_and_town() -> Fx {
    fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (LEVEL, TileRect::new(8, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
    ])
}

fn player_in(fx: &mut Fx, room: RoomId, x: i32, y: i32) -> UnitId {
    let p = fx.spawn(UnitType::Player, 0, room, x, y);
    fx.stats(p, &[(STAT_VELOCITY, 100)]);
    p
}

fn guid(fx: &Fx, u: UnitId) -> u32 {
    fx.game.lists.unit(u).unwrap().guid
}

fn room_of(fx: &Fx, u: UnitId) -> Option<RoomId> {
    fx.game.lists.unit(u).and_then(|e| e.room())
}

fn level_of(fx: &mut Fx, u: UnitId) -> Option<u32> {
    let r = room_of(fx, u)?;
    fx.sim.hooks().drlg.level_id(&fx.game, r)
}

fn pos(fx: &mut Fx, u: UnitId) -> (i32, i32) {
    fx.sim.hooks().path_position(u)
}

/// The class-`class` objects of the game.
fn objects_of(fx: &Fx, class: u32) -> Vec<UnitId> {
    fx.game
        .lists
        .units_of_type(UnitType::Object)
        .into_iter()
        .filter(|&u| fx.sim.sys.units.get(u).is_some_and(|r| r.class == class))
        .collect()
}

fn data(fx: &mut Fx, u: UnitId) -> crate::world::objects::ObjectData {
    fx.sim.hooks().objects.as_ref().unwrap().control.data[&u]
}

fn cast(fx: &mut Fx, p: UnitId) -> (u32, bool) {
    fx.sim.with(&mut fx.game, |g, v| v.town_portal_cast(g, p))
}

fn partner(fx: &mut Fx, o: UnitId) -> Option<UnitId> {
    fx.sim.with(&mut fx.game, |g, v| v.portal_partner(g, o))
}

/// The player's portal (kept in the player data) and its partner.
fn pair(fx: &mut Fx, p: UnitId) -> (UnitId, UnitId) {
    let g = fx.sim.hooks().portals.player_portal(p).expect("+0x48");
    let o1 = fx.game.lists.find_unit(UnitType::Object, g).expect("O1");
    let o2 = partner(fx, o1).expect("O2");
    (o1, o2)
}

fn has_portal_flag(fx: &mut Fx, room: RoomId) -> bool {
    let game = &fx.game;
    let (d, r) = fx.sim.sys.hooks.drlg.drlg_room(game, room).unwrap();
    d.room(r).flags & room_flags::PORTAL != 0
}

fn sent_to(fx: &mut Fx, p: UnitId) -> Vec<Vec<u8>> {
    let all = std::mem::take(&mut fx.sim.hooks().x.sent);
    let (mine, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|(u, _)| *u == p);
    fx.sim.hooks().x.sent = rest;
    mine.into_iter().map(|(_, m)| m).collect()
}

// ---- the Town Portal cast ---------------------------------------------------

#[test]
fn a_cast_in_the_field_makes_a_linked_pair() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    let gp = guid(&fx, p);
    assert_eq!(cast(&mut fx, p), (1, true));
    assert_eq!(objects_of(&fx, PORTAL).len(), 2);
    let (o1, o2) = pair(&mut fx, p);
    assert_eq!(partner(&mut fx, o2), Some(o1));
    // O1 next to P in the field, mode 1, to the town (init 11); O2 in the
    // town, mode 2, to P's level, portal flags 3.
    assert_eq!(level_of(&mut fx, o1), Some(LEVEL));
    assert_eq!(level_of(&mut fx, o2), Some(TOWN));
    assert_eq!(fx.sim.sys.units.get(o1).unwrap().mode, 1);
    assert_eq!(fx.sim.sys.units.get(o2).unwrap().mode, 2);
    let (d1, d2) = (data(&mut fx, o1), data(&mut fx, o2));
    assert_eq!((d1.interact, d2.interact), (TOWN as u8, LEVEL as u8));
    assert_eq!(d2.portal_flags & 3, 3);
    // Both owned by P, linked (flag-ex 0x400).
    assert_eq!((d1.owner, d2.owner), (Some(gp as i32), Some(gp as i32)));
    for o in [o1, o2] {
        assert_ne!(fx.sim.sys.units.get(o).unwrap().flags2 & 0x400, 0);
    }
    let (x1, y1) = pos(&mut fx, o1);
    assert!((x1 - 20).abs() <= 3 && (y1 - 20).abs() <= 3, "({x1}, {y1})");
    // The "has portal" flag on both DRLG rooms; P flag 0x40.
    let (r1, r2) = (room_of(&fx, o1).unwrap(), room_of(&fx, o2).unwrap());
    assert!(has_portal_flag(&mut fx, r1) && has_portal_flag(&mut fx, r2));
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags & 0x40, 0);
    fx.assert_clean();
}

#[test]
fn a_cast_in_town_is_refused_and_the_pair_stays() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    assert_eq!(cast(&mut fx, p), (1, true));
    let (o1, o2) = pair(&mut fx, p);
    let town = room_of(&fx, o2).unwrap();
    let (x, y) = pos(&mut fx, o2);
    fx.sim.with(&mut fx.game, |g, v| {
        PathCtx::of(v, g).teleport_clear(p, Some(town), x + 4, y)
    });
    assert_eq!(level_of(&mut fx, p), Some(TOWN));
    assert_eq!(cast(&mut fx, p), (0, false));
    assert_eq!(objects_of(&fx, PORTAL), {
        let mut v = vec![o1, o2];
        v.sort();
        v
    });
    fx.assert_clean();
}

#[test]
fn a_second_cast_closes_the_old_pair_through_the_delete_lists() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    fx.game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.tick();
    assert_eq!(cast(&mut fx, p), (1, true));
    let (o1, o2) = pair(&mut fx, p);
    let (g1, g2) = (guid(&fx, o1), guid(&fx, o2));
    let r1 = room_of(&fx, o1).unwrap();
    fx.tick();
    sent_to(&mut fx, p);
    assert_eq!(cast(&mut fx, p), (1, true));
    assert!(fx.game.lists.find_unit(UnitType::Object, g1).is_none());
    assert!(fx.game.lists.find_unit(UnitType::Object, g2).is_none());
    assert_eq!(objects_of(&fx, PORTAL).len(), 2);
    assert_ne!(fx.sim.hooks().portals.player_portal(p), Some(g1));
    assert_eq!(fx.sim.hooks().room_deletes[&r1][0], (2, g1));
    // The client's room adjacency holds O1's room, not the town room.
    fx.tick();
    let got = sent_to(&mut fx, p);
    assert!(got.contains(&remove_unit(2, g1).to_vec()), "{got:?}");
    assert!(!got.contains(&remove_unit(2, g2).to_vec()));
    // The delete lists' records are freed.
    assert!(fx.sim.hooks().room_deletes.is_empty());
    fx.assert_clean();
}

#[test]
fn a_cast_in_no_room_closes_the_old_pair_then_is_fatal_0xe42() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    assert_eq!(cast(&mut fx, p), (1, true));
    let (o1, o2) = pair(&mut fx, p);
    let (g1, g2) = (guid(&fx, o1), guid(&fx, o2));
    fx.game.lists.room_remove(p).unwrap();
    assert_eq!(room_of(&fx, p), None);
    assert_eq!(cast(&mut fx, p), (0, false));
    assert!(fx.game.lists.find_unit(UnitType::Object, g1).is_none());
    assert!(fx.game.lists.find_unit(UnitType::Object, g2).is_none());
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().errors),
        vec![WiringError::Portal(0xE42)]
    );
}

/// A monster world that only records the off-tick populates: the room
/// and the portal objects existing at the call; marks the room populated.
#[derive(Default)]
struct Populates(Vec<(RoomId, usize)>);

impl<X> crate::wiring::action::monsters::MonsterWorld<X> for Populates {
    fn type_init(&mut self, _: &mut Sim<'_>, _: &mut ActionHooks<X>, _: UnitId) {}
    fn umods(
        &mut self,
        _: &mut Sim<'_>,
        _: &mut ActionHooks<X>,
        _: UnitId,
        _: Option<UnitId>,
        _: u8,
    ) {
    }
    fn assign_umod(&mut self, _: &mut Sim<'_>, _: &mut ActionHooks<X>, _: UnitId, _: u8) {}
    fn forget(&mut self, _: UnitId) {}
    fn monster(&self, _: UnitId) -> Option<&crate::monsters::init::MonsterData> {
        None
    }
    fn monster_mut(&mut self, _: UnitId) -> Option<&mut crate::monsters::init::MonsterData> {
        None
    }
    fn populate_off_tick(
        &mut self,
        sim: &mut Sim<'_>,
        _: &mut ActionHooks<X>,
        room: RoomId,
    ) -> bool {
        let portals = sim.game.lists.units_of_type(UnitType::Object).len();
        self.0.push((room, portals));
        if let Some(r) = sim.game.lists.room_mut(room) {
            r.populated = true;
        }
        true
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[test]
fn the_far_end_room_is_populated_inside_the_cast() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    fx.sim.hooks().monster_world = Some(Box::new(Populates::default()));
    assert_eq!(cast(&mut fx, p), (1, true));
    let (_, o2) = pair(&mut fx, p);
    let s = room_of(&fx, o2).unwrap();
    // The room is populated at the call, before object 2 exists (only
    // object 1 stands): not left to the next tick's room pass.
    assert!(fx.game.lists.room(s).unwrap().populated);
    let w = fx.sim.hooks().monster_world.take().unwrap();
    let w = w.into_any().downcast::<Populates>().unwrap();
    assert_eq!(w.0, [(s, 1)]);
}

#[test]
fn entering_the_pair_goes_to_town_and_back_and_the_owner_closes_it() {
    let mut fx = field_and_town();
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    assert_eq!(cast(&mut fx, p), (1, true));
    let (o1, o2) = pair(&mut fx, p);
    let (g1, g2) = (guid(&fx, o1), guid(&fx, o2));
    fx.sim.hooks().objects.as_mut().unwrap().host_tick = 10_000;
    fx.sim
        .with(&mut fx.game, |g, v| v.operate_object(g, Some(p), g1));
    assert_eq!(level_of(&mut fx, p), Some(TOWN), "to the town");
    assert_eq!(objects_of(&fx, PORTAL).len(), 2, "the pair stays");
    fx.sim
        .with(&mut fx.game, |g, v| v.operate_object(g, Some(p), g2));
    assert_eq!(level_of(&mut fx, p), Some(LEVEL), "back in the field");
    assert!(objects_of(&fx, PORTAL).is_empty(), "the owner closed it");
    let deletes: Vec<(u8, u32)> = fx
        .sim
        .hooks()
        .room_deletes
        .values()
        .flatten()
        .copied()
        .collect();
    assert!(deletes.contains(&(2, g1)) && deletes.contains(&(2, g2)));
    fx.assert_clean();
}

// ---- the portal pair --------------------------------------------------------

#[test]
fn the_pair_rules_town_refusal_exception_exact_and_one_act() {
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(0, 0, 8, 8)),
        (TOWN, TileRect::new(0, 16, 8, 8)),
        (COWS, TileRect::new(16, 16, 8, 8)),
    ]);
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    let pair_at = |fx: &mut Fx, room, at, level, class, exact| {
        fx.sim.with(&mut fx.game, |g, v| {
            v.create_portal_pair(g, Some(p), Some(room), at, level, class, exact, false)
        })
    };
    // Exact: object 1 at the point itself.
    let (made, o1) = pair_at(&mut fx, a, (21, 22), TOWN, PORTAL, true);
    assert_eq!(made, 1);
    assert_eq!(pos(&mut fx, o1.unwrap()), (21, 22));
    // Owner GUID left −1.
    assert_eq!(data(&mut fx, o1.unwrap()).owner, Some(-1));
    // From the town: refused, except class 60 to the cow level.
    let o2 = partner(&mut fx, o1.unwrap()).unwrap();
    let town = room_of(&fx, o2).unwrap();
    let (tx, ty) = pos(&mut fx, o2);
    assert_eq!(
        pair_at(&mut fx, town, (tx + 4, ty), LEVEL, PORTAL, false),
        (0, None)
    );
    let (made, _) = pair_at(&mut fx, town, (tx + 4, ty), COWS, PERMANENT, false);
    assert_eq!(made, 1);
    fx.assert_clean();
    // Another act: fatal 0xE5C, nothing made.
    assert_eq!(pair_at(&mut fx, a, (20, 20), LUT, PORTAL, false), (0, None));
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().errors),
        vec![WiringError::Portal(0xE5C)]
    );
}

#[test]
fn tyrael_s_pair_leads_to_lut_gholein_without_an_owner() {
    let mut fx = fx_with(&[
        (LAIR, TileRect::new(0, 0, 8, 8)),
        (LUT, TileRect::new(0, 16, 8, 8)),
    ]);
    let a = fx.a;
    let p = player_in(&mut fx, a, 20, 20);
    let (made, o1) = fx.sim.with(&mut fx.game, |g, v| {
        v.create_portal_pair(g, Some(p), Some(a), (20, 20), LUT, PORTAL, false, true)
    });
    assert_eq!(made, 1);
    let o1 = o1.unwrap();
    let o2 = partner(&mut fx, o1).unwrap();
    assert_eq!(level_of(&mut fx, o2), Some(LUT));
    let (d1, d2) = (data(&mut fx, o1), data(&mut fx, o2));
    assert_eq!((d1.interact, d2.interact), (LUT as u8, LAIR as u8));
    assert_eq!((d1.owner, d2.owner), (Some(-1), Some(-1)));
    fx.assert_clean();
}

// ---- the act change ---------------------------------------------------------

#[test]
fn the_act_change_moves_the_player_and_its_client_to_the_new_act() {
    // Both acts' rooms at tiles (8, 8): the leave teleport to (0, 0)
    // finds no room there, as in every act.
    let mut fx = fx_with(&[
        (LEVEL, TileRect::new(8, 8, 8, 8)),
        (LUT, TileRect::new(8, 8, 8, 8)),
    ]);
    let (a, b) = (fx.a, fx.b);
    let p = player_in(&mut fx, a, 60, 60);
    let q = player_in(&mut fx, a, 70, 60);
    let cp = fx
        .game
        .lists
        .add_client(Some(p), None, client_state::IN_GAME);
    fx.game
        .lists
        .add_client(Some(q), None, client_state::IN_GAME);
    fx.tick();
    fx.sim.hooks().x.sent.clear();
    let (gp, gq) = (guid(&fx, p), guid(&fx, q));
    let moved = fx.sim.with(&mut fx.game, |g, v| {
        crate::wiring::path::act_change::run(PathCtx::of(v, g), p, LUT, 0)
    });
    assert!(moved);
    assert_eq!(room_of(&fx, p), Some(b));
    assert_eq!(fx.sim.sys.units.get(p).unwrap().act, 1);
    let c = fx.game.lists.client(cp).unwrap();
    assert_eq!((c.state, c.room), (client_state::CHANGING_ACT, Some(b)));
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags2 & 0x10000, 0);
    // The room-change messages left O with P: the previous room is
    // O when they run. (Q's removal itself is the walk seam
    // `send_unit_removal`, which the path wiring does not provide yet.)
    sent_to(&mut fx, q);
    assert!(!fx.game.lists.room_units(a).contains(&p));
    // P's client: Q's removal and the room hide, never P's own removal;
    // then 0x05, 0x03, 0x53, the new act's 0x07.
    let got = sent_to(&mut fx, p);
    assert!(!got.contains(&remove_unit(0, gp).to_vec()), "{got:?}");
    let at = |id: u8| got.iter().position(|m| m[0] == id).expect("sent");
    let removal_q = got
        .iter()
        .position(|m| *m == remove_unit(0, gq).to_vec())
        .expect("Q removed");
    assert!(removal_q < at(0x05) && at(0x08) < at(0x05));
    let obj_seed = fx.sim.hooks().objects.as_ref().unwrap().obj_seed;
    assert_eq!(
        got[at(0x05) + 1],
        load_act(1, INIT, LUT as u16, obj_seed).to_vec()
    );
    assert_eq!((got[at(0x05) + 2][0], got[at(0x05) + 2].len()), (0x53, 10));
    let reveal = crate::wiring::path::place::map_reveal(8, 8, LUT as u8).to_vec();
    assert_eq!(got[at(0x05) + 3], reveal);
    // The next ticks: S→C 0x15 (flag 1) at P's point, then 0x04 once the
    // room is ready; the client is in game again.
    for _ in 0..5 {
        fx.tick();
        if fx.game.lists.client(cp).unwrap().state == client_state::IN_GAME {
            break;
        }
    }
    assert_eq!(
        fx.game.lists.client(cp).unwrap().state,
        client_state::IN_GAME
    );
    let got = sent_to(&mut fx, p);
    let (x, y) = pos(&mut fx, p);
    let mut reassign = vec![0x15, 0];
    reassign.extend_from_slice(&gp.to_le_bytes());
    reassign.extend_from_slice(&(x as u16).to_le_bytes());
    reassign.extend_from_slice(&(y as u16).to_le_bytes());
    reassign.push(1);
    let r = got.iter().position(|m| *m == reassign).expect("0x15");
    let l = got.iter().position(|m| m[..] == [0x04]).expect("0x04");
    assert!(r < l);
    fx.assert_clean();
    // A level of the client's own act is fatal 0x19F.
    let again = fx.sim.with(&mut fx.game, |g, v| {
        crate::wiring::path::act_change::run(PathCtx::of(v, g), p, LUT, 0)
    });
    assert!(!again);
    assert_eq!(
        std::mem::take(&mut fx.sim.hooks().errors),
        vec![WiringError::ActChange(0x19F)]
    );
}
