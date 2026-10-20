//! The mouse-over label of a hovered object or warp tile (the object case
//! of the hover label). A warp tile (unit type 5) names the level its
//! lvlwarp record leads to: the destination's `LevelWarp` key, else its
//! `LevelName`.
//!
//! The text depends on `SubClass`: a town portal names its destination
//! level, a secret door (bit 7) has no label, a waypoint (bit 6) shows its
//! name, a space, a line feed and the name of the level its room is in,
//! and any other object shows name(U) (door state, shrine name,
//! `Locked Chest`, else the row's `Name` string). The text is the pop-up
//! of the control panel with (text, x, y, 0, centre 1)
//! at the object's draw point, moved down by `NameOffset`. Lines go up, so
//! a waypoint's level name is drawn above `Waypoint`.
// An `OpenWarp` object in mode 2 (classes 74, 194, 195, 386) names the level
// its room's warp tile leads to. A
// multiplayer portal's owner suffix is not drawn (D2RUST is single player).

use crate::bridge::hover::unit_feet;
use crate::bridge::world::{ClientWorld, KindData, UnitKey, TILE};
use crate::rules::camera::Camera;
use crate::ui::draw::{UiDraw, UiDrawSink};
use crate::ui::original::hud_tips;
use crate::ui::original::FontMeasure;
use crate::ui::{Point, StringLookup};

/// The unit type of an object.
pub const OBJECT: u8 = 2;

/// `SubClass` bits.
const SUB_SHRINE: u8 = 1 << 0;
const SUB_PORTAL: u8 = 1 << 2;
const SUB_WAYPOINT: u8 = 1 << 6;
const SUB_NO_LABEL: u8 = 1 << 7;

/// Strings of the object label.
const STR_SPACE: u16 = 3995;
const STR_NEWLINE: u16 = 3998;
const STR_NO_LEVEL_NAME: u16 = 5389;
const STR_CAVE_LEVEL_1: u16 = 5047;
const STR_DEN_OF_EVIL: u16 = 5048;
const STR_CLOSED_DOOR: u16 = 3225;
const STR_OPEN_DOOR: u16 = 3227;
const STR_BLOCKED_DOOR: u16 = 3228;
const STR_LOCKED_DOOR: u16 = 3229;
const STR_SHRINE_BASE: u16 = 10809;
const STR_LOCKED_CHEST: u16 = 3262;

/// Level 8 (Den of Evil), named by client quest byte 1.
const LEVEL_DEN: u8 = 8;

pub use d2_server::game::rows::ObjectLabelRow;

/// The rows by object class id and the `levels` `LevelName` keys by level
/// id. The default has neither: no label.
#[derive(Clone, Debug, Default)]
pub struct ObjectLabels {
    pub rows: Vec<ObjectLabelRow>,
    pub level_names: Vec<String>,
    /// The `levels` `LevelWarp` keys (+0x1BE) by level id.
    pub level_warps: Vec<String>,
}

impl ObjectLabels {
    pub fn new(
        rows: Vec<ObjectLabelRow>,
        level_names: Vec<String>,
        level_warps: Vec<String>,
    ) -> Self {
        ObjectLabels {
            rows,
            level_names,
            level_warps,
        }
    }

    /// The wide `LevelWarp` text of level `id` (+ 0x1BE).
    fn level_warp(&self, id: usize, strings: &dyn StringLookup) -> Option<Vec<u16>> {
        let key = self.level_warps.get(id).filter(|k| !k.is_empty())?;
        Some(
            strings
                .get(key)
                .map_or_else(|| key.encode_utf16().collect(), <[u16]>::to_vec),
        )
    }

    /// The first tile unit (type 5) in the unit
    /// list of the object's room; none: the same in each room of the
    /// room's near list (own room skipped). The warp tile found in room
    /// R names the level R's warp of that class leads to. `Err()`: no
    /// tile in any room (step 2 applies); `Ok(None)`: a tile without a
    /// level, a null text.
    fn warp_name(
        &self,
        world: &ClientWorld,
        key: UnitKey,
        strings: &dyn StringLookup,
    ) -> Result<Option<Vec<u16>>, ()> {
        let own = world.unit_room(key).ok_or(())?;
        let drlg = world.drlg.as_ref().ok_or(())?;
        let tile_in = |room| {
            world
                .room_units
                .list(room)
                .iter()
                .filter(|k| k.unit_type == TILE)
                .find_map(|k| world.units.get(k).map(|u| u.class))
        };
        let found = tile_in(own.room).map(|c| (own.room, c)).or_else(|| {
            drlg.adjacency(own.room)
                .into_iter()
                .filter(|&r| r != own.room)
                .find_map(|r| tile_in(r).map(|c| (r, c)))
        });
        let (room, class) = found.ok_or(())?;
        Ok(drlg
            .warp_dest_level(room, class)
            .and_then(|l| self.level_warp(l as usize, strings)))
    }

    /// The wide `LevelName` of level `id`; `None` without a `levels`
    /// record.
    fn level_name(&self, id: usize, strings: &dyn StringLookup) -> Option<Vec<u16>> {
        let key = self.level_names.get(id)?;
        Some(
            strings
                .get(key)
                .map_or_else(|| key.encode_utf16().collect(), <[u16]>::to_vec),
        )
    }

    /// name(U): the door state, the shrine name,
    /// `Locked Chest`, else the row's `Name` string (the key itself when
    /// the table has no such key).
    fn name(
        &self,
        row: &ObjectLabelRow,
        mode: u32,
        interact: u8,
        strings: &dyn StringLookup,
    ) -> Option<Vec<u16>> {
        let s = |id: u16| strings.get_id(id).map(<[u16]>::to_vec);
        if row.is_door {
            return s(match mode {
                0 => STR_CLOSED_DOOR,
                4 | 5 => STR_BLOCKED_DOOR,
                6 => STR_LOCKED_DOOR,
                _ => STR_OPEN_DOOR,
            });
        }
        if row.subclass & SUB_SHRINE != 0 {
            return s(STR_SHRINE_BASE + u16::from(interact));
        }
        if row.lockable && interact & 0x80 != 0 {
            return s(STR_LOCKED_CHEST);
        }
        if row.name.is_empty() {
            return None;
        }
        Some(
            strings
                .get(&row.name)
                .map_or_else(|| row.name.encode_utf16().collect(), <[u16]>::to_vec),
        )
    }

    /// The label text of the object `key`; `None`: no label.
    pub fn text(
        &self,
        world: &ClientWorld,
        key: UnitKey,
        strings: &dyn StringLookup,
    ) -> Option<Vec<u16>> {
        let u = world.units.get(&key)?;
        if key.unit_type == TILE {
            let room = world.unit_room(key)?;
            let dest = world.drlg.as_ref()?.warp_dest_level(room.room, u.class)?;
            return self
                .level_warp(dest as usize, strings)
                .or_else(|| self.level_name(dest as usize, strings));
        }
        let row = self.rows.get(u.class as usize)?;
        if row.open_warp && u.mode == 2 {
            if let Ok(t) = self.warp_name(world, key, strings) {
                return t;
            }
        }
        let interact = match &u.kind {
            KindData::Object(o) => o.interact,
            _ => 0,
        };
        let s = |id: u16| strings.get_id(id).map(<[u16]>::to_vec);
        if row.subclass & SUB_PORTAL != 0 {
            // The destination level's name.
            if interact == LEVEL_DEN {
                let open = world.client_quest_byte(1).is_some_and(|b| b != 0);
                return s(if open {
                    STR_DEN_OF_EVIL
                } else {
                    STR_CAVE_LEVEL_1
                });
            }
            return self
                .level_name(usize::from(interact), strings)
                .or_else(|| s(STR_NO_LEVEL_NAME));
        }
        if row.subclass & SUB_NO_LABEL != 0 {
            return None;
        }
        if row.subclass & SUB_WAYPOINT != 0 {
            // Name, " ", line feed, the room's level name.
            let level = world.unit_level(key)?;
            let mut t = self.name(row, u.mode, interact, strings)?;
            t.extend(s(STR_SPACE).unwrap_or_default());
            t.extend(s(STR_NEWLINE).unwrap_or_default());
            t.extend(
                self.level_name(usize::from(level), strings)
                    .unwrap_or_default(),
            );
            return Some(t);
        }
        self.name(row, u.mode, interact, strings)
    }

    /// The pop-up draws of the hovered unit `hover`: none
    /// unless it is an object with a label or a warp tile with a
    /// destination. The anchor is the unit's draw point plus `NameOffset`
    /// in y (a warp tile's `SelectY`).
    pub fn draw(
        &self,
        world: &ClientWorld,
        cam: &Camera,
        hover: Option<UnitKey>,
        strings: &dyn StringLookup,
        screen: (i32, i32),
        fonts: Option<&FontMeasure>,
    ) -> Vec<UiDraw> {
        let mut out: Vec<UiDraw> = Vec::new();
        let Some(key) = hover.filter(|k| matches!(k.unit_type, OBJECT | TILE)) else {
            return out;
        };
        let Some(u) = world.units.get(&key) else {
            return out;
        };
        let Some(cell) = u.position else {
            return out;
        };
        let name_offset = if key.unit_type == TILE {
            let Some((_, y, _, _)) = world.drlg.as_ref().and_then(|d| d.warp_select(u.class))
            else {
                return out;
            };
            y
        } else {
            let Some(row) = self.rows.get(u.class as usize) else {
                return out;
            };
            row.name_offset
        };
        let Some(text) = self.text(world, key, strings).filter(|t| !t.is_empty()) else {
            return out;
        };
        let (x, y) = unit_feet(cam, key.unit_type, cell);
        hud_tips::push_popup(
            text,
            Point::new(x, y + name_offset),
            0,
            true,
            screen,
            fonts,
            &mut out as &mut dyn UiDrawSink,
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::drlg::DrlgRoomId;
    use crate::bridge::world::{ActiveRoom, ClientUnit, ObjectData, PlayerData};

    /// Strings by key and id.
    struct Strings {
        keys: Vec<(&'static str, Vec<u16>)>,
        ids: Vec<(u16, Vec<u16>)>,
    }
    impl StringLookup for Strings {
        fn get(&self, key: &str) -> Option<&[u16]> {
            self.keys
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, t)| t.as_slice())
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            self.ids
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, t)| t.as_slice())
        }
    }

    fn strings() -> Strings {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        Strings {
            keys: ["chest", "Waypoint", "Blood Moor"]
                .into_iter()
                .map(|k| (k, w(if k == "chest" { "Chest" } else { k })))
                .collect(),
            ids: [
                (STR_SPACE, " "),
                (STR_NEWLINE, "\n"),
                (STR_CLOSED_DOOR, "Closed Door"),
                (STR_OPEN_DOOR, "Open Door"),
                (STR_BLOCKED_DOOR, "Blocked Door"),
                (STR_LOCKED_DOOR, "Locked Door"),
                (10810, "Refilling Shrine"),
                (STR_LOCKED_CHEST, "Locked Chest"),
                (STR_CAVE_LEVEL_1, "Cave Level 1"),
                (STR_DEN_OF_EVIL, "Den of Evil"),
                (STR_NO_LEVEL_NAME, "No Level Name"),
            ]
            .into_iter()
            .map(|(i, t)| (i, w(t)))
            .collect(),
        }
    }

    const ME: UnitKey = UnitKey {
        unit_type: 0,
        guid: 1,
    };
    const OBJ: UnitKey = UnitKey {
        unit_type: OBJECT,
        guid: 7,
    };
    const MON: UnitKey = UnitKey {
        unit_type: 1,
        guid: 8,
    };

    /// Class 1: chest (`NameOffset` −50, lockable), 2: door, 3: shrine,
    /// 4: waypoint, 5: town portal, 6: secret door (`SubClass` bit 7).
    fn labels() -> ObjectLabels {
        let row = |name: &str, subclass: u8, name_offset: i32| ObjectLabelRow {
            name: name.into(),
            subclass,
            name_offset,
            ..Default::default()
        };
        let mut rows = vec![
            ObjectLabelRow::default(),
            row("chest", 0, -50),
            row("door", 0, -70),
            row("shrine", SUB_SHRINE, -145),
            row("Waypoint", SUB_WAYPOINT, -70),
            row("portal", SUB_PORTAL, -70),
            row("secret", SUB_NO_LABEL, 0),
            row("chest", 0, -50),
        ];
        rows[7].open_warp = true;
        rows[1].lockable = true;
        rows[2].is_door = true;
        ObjectLabels::new(
            rows,
            vec!["".into(), "Rogue Encampment".into(), "Blood Moor".into()],
            vec!["".into(), "To Rogue".into(), "To The Blood Moor".into()],
        )
    }

    fn world(class: u32, mode: u32, interact: u8) -> ClientWorld {
        let mut w = ClientWorld::default();
        let mut p = ClientUnit::new(ME);
        p.position = Some((100, 100));
        p.kind = KindData::Player(PlayerData::default());
        w.units.insert(ME, p);
        w.local_player = Some(ME);
        let mut o = ClientUnit::new(OBJ);
        o.class = class;
        o.mode = mode;
        o.position = Some((100, 100));
        o.kind = KindData::Object(ObjectData {
            interact,
            ..Default::default()
        });
        w.units.insert(OBJ, o);
        let mut m = ClientUnit::new(MON);
        m.class = 1;
        m.position = Some((104, 100));
        w.units.insert(MON, m);
        w.active_rooms = Some(vec![ActiveRoom {
            x0: 0,
            y0: 0,
            w: 1000,
            h: 1000,
            level: 2,
            room: DrlgRoomId(1),
        }]);
        w.room_units.place(OBJ, Some(DrlgRoomId(1)));
        w
    }

    fn text(class: u32, mode: u32, interact: u8) -> Option<String> {
        labels()
            .text(&world(class, mode, interact), OBJ, &strings())
            .map(|t| String::from_utf16(&t).unwrap())
    }

    #[test]
    fn a_chest_label_is_a_centred_popup_name_offset_below_its_draw_point() {
        let w = world(1, 0, 0);
        let cam = crate::world_view::corpse_click::camera_for(&w, 0).unwrap();
        let (x, y) = unit_feet(&cam, OBJECT, (100, 100));
        let draws = labels().draw(&w, &cam, Some(OBJ), &strings(), (800, 600), None);
        let [UiDraw::Text(t)] = draws.as_slice() else {
            panic!("no label: {draws:?}");
        };
        assert_eq!(String::from_utf16(&t.text).unwrap(), "Chest");
        assert_eq!(t.at, Point::new(x, y - 50));
        assert_eq!(t.style.color, 0);
    }

    #[test]
    fn doors_shrines_and_locked_chests_name_their_state() {
        let doors: Vec<_> = [0, 1, 2, 4, 6]
            .into_iter()
            .map(|m| text(2, m, 0).unwrap())
            .collect();
        assert_eq!(
            doors,
            [
                "Closed Door",
                "Open Door",
                "Open Door",
                "Blocked Door",
                "Locked Door"
            ]
        );
        assert_eq!(text(3, 0, 1).as_deref(), Some("Refilling Shrine"));
        assert_eq!(text(1, 0, 0x80).as_deref(), Some("Locked Chest"));
        assert_eq!(text(1, 0, 0).as_deref(), Some("Chest"));
    }

    #[test]
    fn waypoints_portals_and_secret_doors() {
        assert_eq!(text(4, 0, 0).as_deref(), Some("Waypoint \nBlood Moor"));
        assert_eq!(text(5, 0, 2).as_deref(), Some("Blood Moor"));
        assert_eq!(text(5, 0, 8).as_deref(), Some("Cave Level 1"));
        assert_eq!(text(5, 0, 99).as_deref(), Some("No Level Name"));
        assert_eq!(text(6, 0, 0), None);
        // A waypoint in no room has no label.
        let mut w = world(4, 0, 0);
        w.room_units.place(OBJ, None);
        assert_eq!(labels().text(&w, OBJ, &strings()), None);
    }

    #[test]
    fn an_open_warp_without_a_tile_falls_through_to_its_name() {
        // Mode 2 and no tile in the room or its near rooms: step 2.
        let l = labels();
        let w = world(7, 2, 0);
        let t = l.text(&w, OBJ, &strings());
        assert_eq!(
            t.map(|t| String::from_utf16_lossy(&t)).as_deref(),
            Some("Chest")
        );
    }

    #[test]
    fn only_a_hovered_object_is_labelled() {
        let w = world(1, 0, 0);
        let cam = crate::world_view::corpse_click::camera_for(&w, 0).unwrap();
        let l = labels();
        assert!(l
            .draw(&w, &cam, Some(MON), &strings(), (800, 600), None)
            .is_empty());
        assert!(l
            .draw(&w, &cam, None, &strings(), (800, 600), None)
            .is_empty());
    }
}
