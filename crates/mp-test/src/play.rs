//! What a bot knows about play in its game beyond the players: the
//! monsters, the items it holds and the ones on the ground, the objects
//! (waypoints), and the act and area it is in. All of it comes from the
//! server's game messages; the bot never runs the game's rules.
//!
//! Several of these messages have no full layout in `d2_proto::s2c` yet
//! (S→C 0xAC, 0x9C, 0x9D come back as `Unbuilt`); their fixed fields and
//! the head of the item bit stream (flags, version, location, code) are
//! read here directly. The rest of an item's record needs the game's
//! tables and isn't read.

use std::collections::{BTreeMap, BTreeSet};

use d2_proto::item_bits::{hflag, BitReader};
use d2_proto::s2c::Message;

/// Unit types in S→C 0x0A RemoveUnit (and C→S unit targets).
pub mod unit {
    /// A player.
    pub const PLAYER: u8 = 0;
    /// A monster (or NPC).
    pub const MONSTER: u8 = 1;
    /// An object (waypoints, chests, doors, ...).
    pub const OBJECT: u8 = 2;
    /// An item.
    pub const ITEM: u8 = 4;
}

/// S→C 0x69 MonsterState codes of a death: dying, then dead.
const STATE_DEATH: u8 = 8;
const STATE_DEAD: u8 = 9;

/// Item mode 3: on the ground.
const MODE_ON_GROUND: u8 = 3;
/// Item mode 4: on the cursor.
pub const MODE_CURSOR: u8 = 4;
/// Item mode 5: being dropped (on the ground too).
const MODE_DROPPING: u8 = 5;

/// Sub-tiles per tile.
const SUBTILES_PER_TILE: u16 = 5;

/// The `objects` rows whose operate function is the waypoint's (23).
const WAYPOINT_CLASSES: &[u16] = &[
    119, 145, 156, 157, 237, 238, 288, 323, 324, 398, 402, 429, 494, 496, 511, 539,
];

/// Areas scenarios name, with their level ids.
const AREAS: &[(&str, u8)] = &[
    ("rogue-encampment", 1),
    ("blood-moor", 2),
    ("cold-plains", 3),
    ("stony-field", 4),
    ("dark-wood", 5),
    ("black-marsh", 6),
    ("lut-gholein", 40),
    ("kurast-docks", 75),
    ("pandemonium-fortress", 103),
    ("harrogath", 109),
];

/// Each act's town, by act number (1..=5).
const TOWNS: [u8; 5] = [1, 40, 75, 103, 109];

/// The level id an area word names: a name from the list, or a number.
pub fn area_level(word: &str) -> Option<u8> {
    AREAS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(word))
        .map(|&(_, l)| l)
        .or_else(|| word.parse().ok().filter(|l| *l > 0))
}

/// A level id as scenarios say it: its name when the list has one.
pub fn area_name(level: u8) -> String {
    AREAS
        .iter()
        .find(|(_, l)| *l == level)
        .map_or_else(|| level.to_string(), |(n, _)| (*n).to_string())
}

/// The town of act `act` (1..=5).
pub fn town_of(act: u8) -> Option<u8> {
    TOWNS.get(usize::from(act).checked_sub(1)?).copied()
}

/// A monster the bot was told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monster {
    /// Its `monstats` row.
    pub class: u16,
    /// Where it was first seen.
    pub first: (u16, u16),
    /// Where it is (or walks to), as last told.
    pub at: (u16, u16),
    /// Life as last told (0..=128).
    pub life: u8,
    /// The bot saw it die.
    pub dead: bool,
}

/// An item: its code and, on the ground, where it lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The item code (`hp1`, `ssd`, ...), trailing spaces cut.
    pub code: String,
    /// Its ground position (sub-tiles), when on the ground.
    pub at: (u16, u16),
    /// Item mode: 0 stored, 1 equipped, 2 belt, 3 ground, 4 cursor, 5
    /// being dropped.
    pub mode: u8,
}

/// What the bot knows about play in its game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Play {
    /// Monsters by unit id; a dead one stays known after it is removed.
    pub monsters: BTreeMap<u32, Monster>,
    /// Items its own player holds (inventory, belt, cursor, equipped).
    pub held: BTreeMap<u32, Item>,
    /// Items it sees on the ground.
    pub ground: BTreeMap<u32, Item>,
    /// Every item code it has seen, held or on the ground.
    pub codes: BTreeSet<String>,
    /// Objects by unit id: class and position.
    pub objects: BTreeMap<u32, (u16, (u16, u16))>,
    /// The act it is in (1..=5), from the last 0x03.
    pub act: Option<u8>,
    /// The rooms of this act revealed to it (0x07): tile origin -> level.
    pub rooms: BTreeMap<(u16, u16), u8>,
}

impl Play {
    /// Takes in one delivered message (`parsed` is its parse, when it
    /// parsed); `own` is the bot's own player.
    pub fn apply(&mut self, msg: &[u8], parsed: Option<&Message>, own: Option<u32>) {
        match msg.first() {
            Some(0xAC) => self.assign_monster(msg),
            Some(0x9C) => self.item(msg, 8, own, None),
            Some(0x9D) if msg.len() >= 13 => {
                let owner = (msg[8], u32_at(msg, 9));
                self.item(msg, 13, own, Some(owner));
            }
            _ => {}
        }
        let Some(m) = parsed else {
            return;
        };
        match m {
            Message::LoadAct(a) => {
                // A new act: what the bot saw in the old one is gone.
                self.act = Some(a.act + 1);
                self.monsters.clear();
                self.ground.clear();
                self.objects.clear();
                self.rooms.clear();
            }
            Message::MapReveal(r) => {
                self.rooms.insert((r.x, r.y), r.level);
            }
            Message::RemoveUnit(r) => match r.type_ {
                unit::MONSTER => {
                    if self.monsters.get(&r.guid).is_some_and(|m| !m.dead) {
                        self.monsters.remove(&r.guid);
                    }
                }
                unit::OBJECT => {
                    self.objects.remove(&r.guid);
                }
                unit::ITEM => {
                    self.ground.remove(&r.guid);
                    self.held.remove(&r.guid);
                }
                _ => {}
            },
            Message::MonsterState(s) => {
                if let Some(m) = self.monsters.get_mut(&s.guid) {
                    if s.code == STATE_DEATH || s.code == STATE_DEAD {
                        m.dead = true;
                        m.life = 0;
                    }
                }
            }
            Message::MonsterMove(mv) => {
                if let Some(m) = self.monsters.get_mut(&mv.guid) {
                    m.at = (mv.x, mv.y);
                }
            }
            Message::MonsterStop(s) => {
                if let Some(m) = self.monsters.get_mut(&s.guid) {
                    m.at = (s.x, s.y);
                    m.life = s.life;
                }
            }
            Message::AssignObject(o) => {
                self.objects.insert(o.guid, (o.class, (o.x, o.y)));
            }
            _ => {}
        }
    }

    /// S→C 0xAC AssignMonster: guid @1, class u16 @5, x @7, y @9, life @11.
    fn assign_monster(&mut self, msg: &[u8]) {
        if msg.len() < 13 {
            return;
        }
        let guid = u32_at(msg, 1);
        let class = u16_at(msg, 5);
        let at = (u16_at(msg, 7), u16_at(msg, 9));
        let life = msg[11];
        let m = self.monsters.entry(guid).or_insert(Monster {
            class,
            first: at,
            at,
            life,
            dead: false,
        });
        m.at = at;
        m.life = life;
    }

    /// S→C 0x9C / 0x9D: the item's id @4 and its bit stream from `data`.
    /// On the ground it is in sight; otherwise the owner holds it.
    fn item(&mut self, msg: &[u8], data: usize, own: Option<u32>, owner: Option<(u8, u32)>) {
        if msg.len() < data {
            return;
        }
        let guid = u32_at(msg, 4);
        let Some(head) = ItemHead::read(&msg[data..]) else {
            return;
        };
        let Some(code) = head.code else {
            return;
        };
        self.codes.insert(code.clone());
        let item = Item {
            code,
            at: head.ground.unwrap_or_default(),
            mode: head.mode,
        };
        if head.ground.is_some() {
            self.held.remove(&guid);
            self.ground.insert(guid, item);
            return;
        }
        self.ground.remove(&guid);
        // 0x9C goes to the item's owner only; 0x9D names the owner.
        let mine = match owner {
            None => true,
            Some((kind, id)) => kind == unit::PLAYER && own == Some(id),
        };
        if mine {
            self.held.insert(guid, item);
        } else {
            self.held.remove(&guid);
        }
    }

    /// The level at sub-tile point `at`: the level of the revealed room
    /// whose tile origin is nearest above and left of it (a room holds the
    /// tiles from its origin on).
    pub fn level_at(&self, at: (u16, u16)) -> Option<u8> {
        let (tx, ty) = (at.0 / SUBTILES_PER_TILE, at.1 / SUBTILES_PER_TILE);
        self.rooms
            .iter()
            .filter(|((x, y), _)| *x <= tx && *y <= ty)
            .min_by_key(|((x, y), _)| (tx - x) + (ty - y))
            .map(|(_, &l)| l)
    }

    /// The monsters of `class`, nearest to `near` first.
    pub fn monsters_of(&self, class: u16, near: (u16, u16)) -> Vec<(u32, &Monster)> {
        let mut v: Vec<(u32, &Monster)> = self
            .monsters
            .iter()
            .filter(|(_, m)| m.class == class)
            .map(|(&g, m)| (g, m))
            .collect();
        v.sort_by_key(|(g, m)| (distance(m.first, near), *g));
        v
    }

    /// Items held with this code.
    pub fn held_with(&self, code: &str) -> Option<u32> {
        self.held
            .iter()
            .find(|(_, i)| i.code.eq_ignore_ascii_case(code))
            .map(|(&g, _)| g)
    }

    /// The ground item with this code nearest to `near`.
    pub fn ground_with(&self, code: &str, near: (u16, u16)) -> Option<(u32, (u16, u16))> {
        self.ground
            .iter()
            .filter(|(_, i)| i.code.eq_ignore_ascii_case(code))
            .min_by_key(|(g, i)| (distance(i.at, near), **g))
            .map(|(&g, i)| (g, i.at))
    }

    /// The waypoint object nearest to `near`.
    pub fn nearest_waypoint(&self, near: (u16, u16)) -> Option<(u32, (u16, u16))> {
        self.objects
            .iter()
            .filter(|(_, (class, _))| WAYPOINT_CLASSES.contains(class))
            .min_by_key(|(g, (_, at))| (distance(*at, near), **g))
            .map(|(&g, &(_, at))| (g, at))
    }
}

/// The largest axis distance between two sub-tile points.
pub fn distance(a: (u16, u16), b: (u16, u16)) -> u16 {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))
}

/// The head of an item's network bit stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemHead {
    /// Header flags.
    pub flags: u32,
    /// Item mode.
    pub mode: u8,
    /// Its ground position, for modes 3 and 5.
    pub ground: Option<(u16, u16)>,
    /// The item code, trailing spaces cut (`None` for an ear).
    pub code: Option<String>,
}

impl ItemHead {
    /// Reads flags (32 bits), version (10), mode (3), the location (ground
    /// x and y, 16 bits each; or body, x, y, page, 4/4/4/3 bits) and the
    /// code (32 bits, first character in the low byte).
    pub fn read(stream: &[u8]) -> Option<Self> {
        let mut r = BitReader::new(stream);
        let flags = r.read(32).ok()?;
        r.read(10).ok()?;
        let mode = r.read(3).ok()? as u8;
        let ground = if mode == MODE_ON_GROUND || mode == MODE_DROPPING {
            Some((r.read(16).ok()? as u16, r.read(16).ok()? as u16))
        } else {
            r.read(15).ok()?;
            None
        };
        let code = if flags & hflag::EAR != 0 {
            None
        } else {
            let b = r.read(32).ok()?.to_le_bytes();
            Some(String::from_utf8_lossy(&b).trim_end().to_string())
        };
        Some(Self {
            flags,
            mode,
            ground,
            code,
        })
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes bits LSB-first, as the server's item writer does.
    struct Bits(Vec<u8>, usize);

    impl Bits {
        fn put(&mut self, v: u32, n: usize) {
            for i in 0..n {
                if self.1 / 8 == self.0.len() {
                    self.0.push(0);
                }
                if v >> i & 1 == 1 {
                    self.0[self.1 / 8] |= 1 << (self.1 % 8);
                }
                self.1 += 1;
            }
        }
    }

    fn stream(mode: u8, ground: Option<(u16, u16)>, code: &[u8; 4]) -> Vec<u8> {
        let mut b = Bits(Vec::new(), 0);
        b.put(hflag::IDENTIFIED | hflag::COMPACT, 32);
        b.put(101, 10);
        b.put(u32::from(mode), 3);
        match ground {
            Some((x, y)) => {
                b.put(u32::from(x), 16);
                b.put(u32::from(y), 16);
            }
            None => b.put(0, 15),
        }
        b.put(u32::from_le_bytes(*code), 32);
        b.0
    }

    fn msg_9c(guid: u32, data: &[u8]) -> Vec<u8> {
        let mut m = vec![0x9C, 0, 0, 0];
        m.extend_from_slice(&guid.to_le_bytes());
        m.extend_from_slice(data);
        m[2] = m.len() as u8;
        m
    }

    fn msg_9d(guid: u32, owner: u32, data: &[u8]) -> Vec<u8> {
        let mut m = vec![0x9D, 0, 0, 0];
        m.extend_from_slice(&guid.to_le_bytes());
        m.push(unit::PLAYER);
        m.extend_from_slice(&owner.to_le_bytes());
        m.extend_from_slice(data);
        m[2] = m.len() as u8;
        m
    }

    #[test]
    fn an_item_head_gives_its_code_and_place() {
        let h = ItemHead::read(&stream(3, Some((5000, 4200)), b"hp1 ")).unwrap();
        assert_eq!(h.code.as_deref(), Some("hp1"));
        assert_eq!(h.ground, Some((5000, 4200)));
        let h = ItemHead::read(&stream(0, None, b"ssd ")).unwrap();
        assert_eq!((h.code.as_deref(), h.ground), (Some("ssd"), None));
        assert_eq!(ItemHead::read(&[1, 2]), None);
    }

    #[test]
    fn items_move_between_hand_and_ground() {
        let mut p = Play::default();
        p.apply(&msg_9c(7, &stream(0, None, b"hp1 ")), None, Some(1));
        assert_eq!(p.held_with("hp1"), Some(7));
        p.apply(
            &msg_9c(7, &stream(3, Some((10, 20)), b"hp1 ")),
            None,
            Some(1),
        );
        assert_eq!(p.held_with("hp1"), None);
        assert_eq!(p.ground_with("hp1", (0, 0)), Some((7, (10, 20))));
        // Another player takes it: off the ground, not ours.
        p.apply(&msg_9d(7, 2, &stream(0, None, b"hp1 ")), None, Some(1));
        assert_eq!(
            (p.ground_with("hp1", (0, 0)), p.held_with("hp1")),
            (None, None)
        );
        // Ours by 0x9D.
        p.apply(&msg_9d(8, 1, &stream(0, None, b"ssd ")), None, Some(1));
        assert_eq!(p.held_with("ssd"), Some(8));
    }

    #[test]
    fn a_monster_is_seen_and_dies() {
        let mut p = Play::default();
        let mut m = vec![0xAC, 9, 0, 0, 0, 19, 0, 100, 0, 200, 0, 128, 13];
        p.apply(&m, None, None);
        m[1] = 10;
        m[7] = 140;
        p.apply(&m, None, None);
        let near = p.monsters_of(19, (101, 200));
        assert_eq!(near.len(), 2);
        assert_eq!(near[0].0, 9);
        let state = d2_proto::s2c::parse(&[0x69, 9, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0]).unwrap();
        p.apply(&[], Some(&state), None);
        assert!(p.monsters[&9].dead);
        // A dead monster stays known when its corpse goes.
        let gone = d2_proto::s2c::parse(&[0x0A, 1, 9, 0, 0, 0]).unwrap();
        p.apply(&[], Some(&gone), None);
        assert!(p.monsters[&9].dead);
        let gone = d2_proto::s2c::parse(&[0x0A, 1, 10, 0, 0, 0]).unwrap();
        p.apply(&[], Some(&gone), None);
        assert!(!p.monsters.contains_key(&10));
    }

    #[test]
    fn the_level_is_the_room_a_point_is_in() {
        let mut p = Play::default();
        // Rooms at tiles (100, 200) level 3, (108, 200) level 2, (100, 208) level 4.
        for (x, y, level) in [(100u16, 200u16, 3u8), (108, 200, 2), (100, 208, 4)] {
            let [x0, x1] = x.to_le_bytes();
            let [y0, y1] = y.to_le_bytes();
            let r = d2_proto::s2c::parse(&[0x07, x0, x1, y0, y1, level]).unwrap();
            p.apply(&[], Some(&r), Some(5));
        }
        assert_eq!(p.level_at((503, 1003)), Some(3));
        assert_eq!(p.level_at((545, 1010)), Some(2));
        assert_eq!(p.level_at((503, 1045)), Some(4));
        assert_eq!(p.level_at((10, 10)), None);
        let act = d2_proto::s2c::parse(&[0x03, 1, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0]).unwrap();
        p.apply(&[], Some(&act), Some(5));
        assert_eq!((p.act, p.level_at((503, 1003))), (Some(2), None));
        assert_eq!(area_name(3), "cold-plains");
        assert_eq!(area_level("cold-plains"), Some(3));
        assert_eq!(area_level("40"), Some(40));
        assert_eq!(town_of(2), Some(40));
        assert_eq!(town_of(6), None);
    }
}
