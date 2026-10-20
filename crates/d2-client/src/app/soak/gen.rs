//! The seeded random input of a soak run. The generator's RNG is the
//! tool's own (splitmix64), never the game's: it only picks inputs, and
//! every pick is written to the log as a concrete action ([`Act`]).

use crate::bridge::items as citems;
use crate::bridge::world::{ClientWorld, ITEM, MONSTER, OBJECT};
use crate::controls::Action as Key;

use super::log::Act;

/// splitmix64 (the tool's RNG; D2RUST's own).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in 0..n (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n.max(1))) as u32
    }

    pub fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.below(v.len() as u32) as usize])
        }
    }
}

/// The key actions the soak presses: potions, skill hotkeys, the panels,
/// Esc, Alt (show items) and the rest of the world keys.
pub const KEYS: &[Key] = &[
    Key::BeltSlot1,
    Key::BeltSlot2,
    Key::BeltSlot3,
    Key::BeltSlot4,
    Key::SkillSlot1,
    Key::SkillSlot2,
    Key::SkillSlot3,
    Key::SkillSlot4,
    Key::SkillSlot5,
    Key::SkillSlot6,
    Key::SkillSlot7,
    Key::SkillSlot8,
    Key::ToggleInventory,
    Key::ToggleCharacter,
    Key::ToggleSkillTree,
    Key::ToggleQuests,
    Key::ToggleAutomap,
    Key::ToggleMinimap,
    Key::ToggleSkillMenuLeft,
    Key::ToggleSkillMenuRight,
    Key::ToggleBelt,
    Key::ToggleHireling,
    Key::ToggleMessageLog,
    Key::ToggleRun,
    Key::SwapWeapons,
    Key::ShowItems,
    Key::GameMenu,
    Key::PanelClose,
    Key::SkillUp,
    Key::SkillDown,
    Key::ClearScreen,
    Key::ToggleHelp,
];

/// Waypoint levels (`levels` rows with a waypoint) the `wp` action asks
/// for most of the time; the rest of the time any level 0..=136 (the
/// server must refuse the ones the player cannot take).
pub const WP_LEVELS: &[u32] = &[
    1, 3, 4, 5, 6, 27, 29, 32, 35, 40, 42, 43, 44, 46, 48, 52, 57, 74, 75, 76, 77, 78, 79, 80, 81,
    83, 101, 103, 106, 107, 109, 111, 112, 113, 115, 117, 118, 123, 129,
];

/// Frame point of a sub-tile, with the local player at the frame's
/// centre (an isometric sub-tile is 32 × 16 pixels): close enough for a
/// random click to land on or near a unit.
pub fn frame_point(world: &ClientWorld, at: (u16, u16)) -> Option<(i32, i32)> {
    let me = local_cell(world)?;
    let dx = i32::from(at.0) - i32::from(me.0);
    let dy = i32::from(at.1) - i32::from(me.1);
    let (x, y) = (400 + (dx - dy) * 16, 290 + (dx + dy) * 8);
    ((0..800).contains(&x) && (0..550).contains(&y)).then_some((x, y))
}

/// The local player's cell: its walk prediction's while the client moves
/// it, else the model's.
pub fn local_cell(w: &ClientWorld) -> Option<(u16, u16)> {
    let u = w.local_player.and_then(|k| w.units.get(&k))?;
    w.predicted(u).map(|p| p.cell()).or(u.position)
}

/// The client's interact distance for an item (d ≤ 4 sends the pick-up at
/// once).
const PICK_REACH: i32 = 4;
/// The unit sizes of the distance.
const ITEM_SIZE: i32 = 1;
const PLAYER_SIZE: i32 = 2;

/// The interact sender (the bridge's `interact`) runs where the client would
/// send at once (unit distance The client's and the server's test), without the
/// walk to the unit: an item at d ≤ 4, an NPC at its reach 2 and an object the
/// player stands at (read as d ≤ 2, the object's size unknown here). From
/// farther the server walks the player to the unit and the client, which sent
/// no walk, stands: a false desync. Farther units are reached by the clicks on
/// a unit, through the client's own decision.
fn interact_reach(ty: u8, at: Option<(u16, u16)>, me: Option<(u16, u16)>) -> bool {
    let (Some(at), Some(me)) = (at, me) else {
        return false;
    };
    let (size, reach) = if ty == ITEM {
        (ITEM_SIZE, PICK_REACH)
    } else {
        (0, 2)
    };
    crate::bridge::objects::unit_distance_at(at, size, me, PLAYER_SIZE) <= reach
}

/// A decorative object whose current mode is not selectable never gets a
/// C→S 0x13 from the click path: do not invent one. (Operating it could move
/// it to a mode without client graphics, which the client treats as fatal.)
fn object_interactable(u: &crate::bridge::world::ClientUnit) -> bool {
    u.flag_4
}

/// The generator: one action or none per step.
pub struct Gen {
    pub rng: Rng,
    /// Percent of steps that get an action.
    pub rate: u32,
    /// Object classes that are waypoints (operate function 23).
    pub waypoint_classes: Vec<u32>,
}

impl Gen {
    pub fn new(seed: u64, waypoint_classes: Vec<u32>) -> Self {
        Self {
            rng: Rng::new(seed),
            rate: 30,
            waypoint_classes,
        }
    }

    /// The action of this step, if any.
    pub fn next(&mut self, w: &ClientWorld) -> Option<Act> {
        if !self.rng.chance(self.rate) {
            return None;
        }
        let r = &mut self.rng;
        let roll = r.below(100);
        let my_pos = local_cell(w);
        let items = citems::local_items(w);
        let cursor = citems::cursor_item(w);
        // A ground pick-up (C→S 0x16) is sent by the client only within its
        // interact distance: d ≤ 4 (item). A farther item is walked to first
        // (code 2 / 4 and a pending interaction), which the clicks on a unit
        // below drive through the client; a 0x16 from farther makes the server
        // walk the player while the client stands (a false
        // desync).
        let ground: Vec<_> = citems::ground_items(w)
            .into_iter()
            .filter(|i| interact_reach(ITEM, Some((i.x, i.y)), my_pos))
            .collect();
        let act = match roll {
            // Left clicks: anywhere (UI panels, ground), or near the
            // centre (short walks).
            0..=21 => {
                let (x, y) = if r.chance(50) {
                    (r.below(800) as i32, r.below(600) as i32)
                } else {
                    (250 + r.below(300) as i32, 180 + r.below(220) as i32)
                };
                Act::Click { right: false, x, y }
            }
            22..=29 => Act::Click {
                right: true,
                x: r.below(800) as i32,
                y: r.below(600) as i32,
            },
            30..=31 => Act::Move {
                x: r.below(800) as i32,
                y: r.below(600) as i32,
            },
            // A click on a unit on screen (monster, NPC, object, item).
            32..=41 => {
                let units: Vec<(u16, u16)> = w
                    .units
                    .iter()
                    .filter(|(k, u)| {
                        matches!(k.unit_type, MONSTER | OBJECT | ITEM)
                            && Some(**k) != w.local_player
                            && u.position.is_some()
                    })
                    .filter_map(|(_, u)| u.position)
                    .collect();
                let p = r.pick(&units).and_then(|&p| frame_point(w, p));
                match p {
                    Some((x, y)) => Act::Click {
                        right: r.chance(25),
                        x: x + r.below(9) as i32 - 4,
                        y: y + r.below(9) as i32 - 12,
                    },
                    None => Act::Key(*r.pick(KEYS).unwrap_or(&Key::ShowItems)),
                }
            }
            42..=59 => Act::Key(*r.pick(KEYS).unwrap_or(&Key::ShowItems)),
            // Interaction with a nearby unit (NPC talk, object operate,
            // item pick-up through the click path).
            60..=65 => {
                let units: Vec<_> = w
                    .units
                    .iter()
                    .filter(|(k, u)| {
                        matches!(k.unit_type, MONSTER | OBJECT | ITEM)
                            && (k.unit_type != OBJECT || object_interactable(u))
                            && interact_reach(k.unit_type, u.position, my_pos)
                    })
                    .map(|(k, _)| *k)
                    .collect();
                match r.pick(&units) {
                    Some(k) => Act::Interact {
                        ut: k.unit_type,
                        guid: k.guid,
                    },
                    None => Act::Wait(1),
                }
            }
            // Item moves.
            66..=86 => {
                let any_item = |r: &mut Rng| r.pick(&items).map(|i| i.key.guid);
                match (cursor.as_ref(), r.below(10)) {
                    (Some(c), 0..=1) => Act::Drop { guid: c.key.guid },
                    (Some(c), 2..=4) => Act::Insert {
                        guid: c.key.guid,
                        x: r.below(10),
                        y: r.below(8),
                        // Inventory, cube, stash (and the odd page).
                        page: *r.pick(&[0, 0, 0, 3, 4, 1]).unwrap_or(&0),
                    },
                    (Some(c), 5..=7) => Act::Equip {
                        guid: c.key.guid,
                        body: 1 + r.below(12) as u8,
                    },
                    (Some(c), _) => Act::Belt {
                        guid: c.key.guid,
                        slot: r.below(16),
                    },
                    (None, 0..=3) => match r.pick(&ground) {
                        Some(g) => Act::Pick {
                            guid: g.key.guid,
                            cursor: r.chance(50),
                        },
                        None => Act::Wait(1),
                    },
                    (None, 4..=6) => match any_item(r) {
                        Some(guid) => Act::Remove { guid },
                        None => Act::Wait(1),
                    },
                    (None, 7..=8) => {
                        let belt: Vec<u32> = items
                            .iter()
                            .filter(|i| i.mode == 2)
                            .map(|i| i.key.guid)
                            .collect();
                        match r.pick(&belt) {
                            Some(&guid) => Act::UseBelt { guid },
                            None => Act::Wait(1),
                        }
                    }
                    (None, _) => match any_item(r) {
                        Some(guid) => Act::UseGrid { guid },
                        None => Act::Wait(1),
                    },
                }
            }
            // Town portal: a portal scroll or book used from the grid.
            87..=90 => {
                let tp: Vec<u32> = items
                    .iter()
                    .filter(|i| matches!(i.code, Some(c) if &c[..3] == b"tsc" || &c[..3] == b"tbk"))
                    .map(|i| i.key.guid)
                    .collect();
                match r.pick(&tp) {
                    Some(&guid) => Act::UseGrid { guid },
                    None => Act::Key(Key::ToggleInventory),
                }
            }
            // Waypoint travel.
            91..=95 => {
                let wps: Vec<u32> = w
                    .units
                    .iter()
                    .filter(|(k, u)| {
                        k.unit_type == OBJECT && self.waypoint_classes.contains(&u.class)
                    })
                    .map(|(k, _)| k.guid)
                    .collect();
                match r.pick(&wps) {
                    Some(&guid) if r.chance(50) => Act::Interact { ut: OBJECT, guid },
                    Some(&guid) => Act::Waypoint {
                        guid,
                        level: if r.chance(80) {
                            *r.pick(WP_LEVELS).unwrap_or(&1)
                        } else {
                            r.below(137)
                        },
                    },
                    None => Act::Wait(1),
                }
            }
            _ => Act::Wait(1 + r.below(20)),
        };
        Some(act)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_picks_the_same_inputs() {
        let w = ClientWorld::default();
        let mut a = Gen::new(9, vec![]);
        let mut b = Gen::new(9, vec![]);
        for _ in 0..500 {
            assert_eq!(a.next(&w), b.next(&w));
        }
    }

    #[test]
    fn an_object_interact_needs_the_model_selectable_flag() {
        use crate::bridge::world::ClientUnit;
        let key = crate::bridge::world::UnitKey::new(OBJECT, 6);
        let mut u = ClientUnit::new(key);
        u.flag_4 = false;
        assert!(!object_interactable(&u));
        u.flag_4 = true;
        assert!(object_interactable(&u));
    }

    #[test]
    fn every_generated_action_reads_back_from_its_log_line() {
        let w = ClientWorld::default();
        let mut g = Gen::new(3, vec![]);
        for _ in 0..2000 {
            if let Some(a) = g.next(&w) {
                assert_eq!(Act::parse(&a.to_string()).unwrap(), a);
            }
        }
    }
}
