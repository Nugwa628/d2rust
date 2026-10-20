//! `state-dump --client-out FILE`: a dump of the client model's units. One
//! `snap` per server tick
//! that ran, holding the client model's units in format `state-1` with
//! header `side` = `d2rust-client`: set S ([`ClientWorld::units`]) and set C
//! (`objclient.set_c`), each unit tagged `"set":"S"` / `"set":"C"`, sorted
//! by (`set`, `ut`, `g`). Reads the model only; not game logic.
//!
//! Keys the client model does not hold are left out of the line and named
//! in the header `gaps` (`RUN_GAPS`), so a comparison reports them as gaps
//! and not as a match.

use std::fmt::Write as _;

use crate::bridge::items;
use crate::bridge::world::{room_of_point, ClientUnit, ClientWorld, KindData, UnitKey};
use d2_sim::debug::state::{json_string, Header};

/// Keys never written by this dump, with the reason (header `gaps`).
pub const GAPS: [&str; 0] = [];

/// The `fields` of the header.
pub const FIELDS: [&str; 27] = [
    "ut", "g", "cl", "m", "x", "y", "xf", "yf", "tx", "ty", "d", "fr", "fc", "sp", "s", "act",
    "lv", "hp", "hpx", "mp", "mpx", "st", "stx", "str", "ene", "own", "q",
];

const DYNAMIC: [u8; 3] = [0, 1, 3];

/// The header line of the dump.
/// `grid` (`--client-grid`) adds the `fp` key.
pub fn header(
    tool: &str,
    date: &str,
    command: &str,
    grid: bool,
    rate: d2_sim::tick::TickRate,
) -> String {
    let h = Header {
        side: "d2rust-client".into(),
        tool: tool.into(),
        date: date.into(),
        command: command.into(),
        fields: FIELDS
            .iter()
            .chain(
                [
                    "dex", "vit", "lvl", "iq", "if", "fi", "il", "aa", "pf", "sf", "rp", "rs",
                    "ik", "ss", "is",
                ]
                .iter(),
            )
            .chain(grid.then_some(&"fp"))
            .map(|k| (*k).to_owned())
            .collect(),
        gaps: GAPS.iter().map(|g| (*g).to_owned()).collect(),
        save: None,
        seed: None,
        tick_rate: rate,
    };
    h.to_json_line()
}

/// One `snap` line for server frame `frame` (no newline); `units` is
/// [`units_json`] of the model as the tick end left it: the client has not
/// yet read that tick's messages.
pub fn snap_line(units: &str, frame: i32) -> String {
    format!("{{\"k\":\"snap\",\"f\":{frame},\"units\":{units}}}")
}

/// The `units` array of a `snap` line.
pub fn units_json(w: &ClientWorld) -> String {
    units_json_with(w, None)
}

/// `fp` (`--client-grid`): the masks of the client collision grid in the
/// [`FP_RADIUS`] window around each set S object, row by row, from the object's
/// own room (-1 outside it). The grid is the one the client path sees
/// (`bridge::client_path`): the DRLG's, with the footprints of the model's
/// objects and living monsters stamped on it, and the local player's own at its
/// cell (0x80 on the plus, 0x1080 at the centre).
pub struct Footprints<'a> {
    path: crate::bridge::client_path::ClientPath,
    drlg: &'a d2_sim::drlg::Drlg,
}

/// The `fp` window: the sub-tiles within 4 of the object's (9 × 9).
pub const FP_RADIUS: i32 = 4;

impl<'a> Footprints<'a> {
    /// The grid of `w` with its footprints; `None` without a client DRLG.
    pub fn of(w: &'a ClientWorld, inputs: &crate::bridge::world::ModelInputs) -> Option<Self> {
        let drlg = &w.drlg.as_ref()?.drlg;
        let t = crate::bridge::predict::path_tables()?;
        let others = crate::world_view::walk::other_units(w, &inputs.tables.monsters);
        let objects = crate::world_view::walk::other_objects(w, &inputs.objclient.rows);
        let mut path = crate::bridge::client_path::ClientPath::default();
        path.stamp_others(t, drlg, &others, &objects);
        if let Some((x, y)) = w.local_cell() {
            path.stamp_local(t, drlg, x, y);
        }
        Some(Footprints { path, drlg })
    }

    fn window(&self, x: u16, y: u16) -> Vec<i32> {
        self.path
            .window(self.drlg, i32::from(x), i32::from(y), FP_RADIUS)
    }
}

/// [`units_json`] with the `fp` key of each set S object when `fp` is given.
pub fn units_json_with(w: &ClientWorld, fp: Option<&Footprints>) -> String {
    let mut rows: Vec<(char, &ClientUnit)> = w.units.values().map(|u| ('S', u)).collect();
    rows.extend(w.objclient.set_c.values().map(|u| ('C', u)));
    rows.sort_by_key(|(s, u)| (*s, u.key.unit_type, u.key.guid));
    let mut o = String::from("[");
    for (i, (set, u)) in rows.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        unit_json(&mut o, w, *set, u, fp);
    }
    o.push(']');
    o
}

/// `d`: the path direction of a monster or player; the zeroed static path of an
/// object or item (allocated zeroed, no model rule writes +0x1C).
fn direction(w: &ClientWorld, set: char, u: &ClientUnit) -> Option<u8> {
    match (set, u.key.unit_type) {
        ('C', 1) => crate::bridge::critter_path::direction(w, u.key),
        // A path record the model does not write is the zeroed allocation
        // (`path_dir` is set by the creation draw only, `monster_anim`).
        // A server monster's client path (`critter_path::server_setup`)
        // holds the direction the mode machine turns.
        ('S', 1) => crate::bridge::critter_path::direction(
            w,
            crate::bridge::critter_path::Who::server(u.key),
        )
        .or(u.path_dir)
        .or(Some(0)),
        ('S', 0) => Some(u.path_dir.unwrap_or(0)),
        (_, 2 | 4 | 5) => Some(0),
        _ => None,
    }
}

/// `tx`, `ty`: the path target (+0x10, +0x12) of a dynamic unit. A monster's
/// client path holds it (C critters; S monsters through the server copy); a
/// missile's path holds the create's target point; a player's path holds the
/// point of its last 0x15 skill start and is otherwise the zeroed allocation.
fn path_target(
    w: &ClientWorld,
    set: char,
    u: &ClientUnit,
    missile: Option<&crate::bridge::client_missiles::ClientMissile>,
) -> Option<(u16, u16)> {
    use crate::bridge::critter_path::{target, Who};
    match (set, u.key.unit_type) {
        (_, 3) => missile.map(|m| (m.target_point.0 as u16, m.target_point.1 as u16)),
        ('C', 1) => Some(target(w, u.key).unwrap_or((0, 0))),
        ('S', 1) => Some(target(w, Who::server(u.key)).unwrap_or((0, 0))),
        (_, 0) => Some(u.path_target.unwrap_or((0, 0))),
        _ => None,
    }
}

/// `own`: a missile's owner (+0x98 when flag-ex 0x400, set by the
/// create); an item's holder (item data +0x5C inventory → its unit's GUID):
/// grid pages, body and belt link the item into the inventory, the cursor sets
/// +0x5C alone; a ground item is unlinked. A monster's owner is the server AI
/// control, which a client unit lacks. GUID 0 / −1 absent. A store item (0x9D
/// action 11) is linked into the NPC's trade inventory, so its owner is the NPC
/// (the store of Akara reads `own`
/// 12 on the client). A socketed item (mode 6) is linked by the 0x9D socket
/// filler into the inventory of the header's owner, linked
/// only when that owner is an item: its owner is the item it fills
/// ([`item_holder`]).
fn owner(
    w: &ClientWorld,
    u: &ClientUnit,
    missile: Option<&crate::bridge::client_missiles::ClientMissile>,
) -> Option<u32> {
    let g = match u.key.unit_type {
        3 => missile
            .filter(|_| u.flag_ex & 0x400 != 0)
            .and_then(|m| m.owner)
            .map(|k| k.guid),
        4 => items::item(w, u.key)
            .and_then(|v| item_holder(v.mode, v.owner))
            .map(|k| k.guid),
        _ => None,
    }?;
    (g != 0 && g != u32::MAX).then_some(g)
}

/// The unit whose inventory an item of `mode` with record owner `owner` is
/// linked into: grid, body, belt and cursor (modes 0, 1, 2, 4) the owner;
/// socketed (mode 6) the owner only when it is an item (the link
/// returns without linking for any other owner); the ground (3) and anything
/// else none.
fn item_holder(mode: u8, owner: Option<UnitKey>) -> Option<UnitKey> {
    match mode {
        0 | 1 | 2 | 4 => owner,
        6 => owner.filter(|k| k.unit_type == 4),
        _ => None,
    }
}

/// `act` (u8 unit +0x18): the client create (every type but
/// monsters, which the monster create makes) writes the act of the level of the room
/// under a non-zero creation point; otherwise the zeroed allocation. A client
/// unit never leaves its act (an act change frees them all, model), so the room
/// under its point now stands for the one it was made in.
fn act(w: &ClientWorld, u: &ClientUnit, x: u16, y: u16) -> u8 {
    if u.key.unit_type == 1 || (x, y) == (0, 0) {
        return 0;
    }
    w.active_rooms
        .as_deref()
        .and_then(|rooms| room_of_point(rooms, i32::from(x), i32::from(y)))
        .map_or(0, |r| d2_sim::drlg::act_of_level(u32::from(r.level)))
}

fn unit_json(o: &mut String, w: &ClientWorld, set: char, u: &ClientUnit, fp: Option<&Footprints>) {
    let UnitKey { unit_type, guid } = u.key;
    // An item's point and class come from its last record (the stream's
    // position, the code's table row) in every mode.
    let item = (unit_type == 4).then(|| items::item(w, u.key)).flatten();
    let fields = match &u.kind {
        KindData::Item(d) => d.fields.as_ref(),
        _ => None,
    };
    let (x, y) = item
        .as_ref()
        .map(|v| (v.x, v.y))
        .or(u.position)
        .unwrap_or((0, 0));
    let class = fields.map_or(u.class, |f| f.class);
    let _ = write!(
        o,
        "{{\"ut\":{unit_type},\"g\":{guid},\"cl\":{class},\"m\":{},\"x\":{x},\"y\":{y}",
        u.mode
    );
    // A client missile: the path's precise point, direction and the
    // animation record.
    let missile = (unit_type == 3)
        .then(|| w.objclient.missiles.get(&u.key))
        .flatten();
    if DYNAMIC.contains(&unit_type) {
        let (xf, yf) = missile
            .map(|m| (m.pos.0 & 0xFFFF, m.pos.1 & 0xFFFF))
            .or(u.precise.map(|(px, py)| (px & 0xFFFF, py & 0xFFFF)))
            .unwrap_or((0x8000, 0x8000));
        let _ = write!(o, ",\"xf\":{xf},\"yf\":{yf}");
    }
    if let Some((tx, ty)) = path_target(w, set, u, missile) {
        let _ = write!(o, ",\"tx\":{tx},\"ty\":{ty}");
    }
    if let Some(m) = missile {
        let _ = write!(
            o,
            ",\"d\":{},\"fr\":{},\"fc\":{},\"sp\":{}",
            m.direction, m.frame, m.anim_len, m.anim_speed
        );
    } else if let Some(d) = direction(w, set, u) {
        let _ = write!(o, ",\"d\":{d}");
    }
    if missile.is_none() {
        let _ = write!(o, ",\"fr\":{},\"fc\":{}", u.frame, u.frame_count);
        // +0x4C is zero until an animation set-up writes it.
        let _ = write!(o, ",\"sp\":{}", u.speed.unwrap_or(0));
    }
    if let Some((lo, hi)) = u.seed {
        let _ = write!(o, ",\"s\":[{lo},{hi}]");
    }
    if let Some(r) = w
        .active_rooms
        .as_deref()
        .and_then(|rooms| room_of_point(rooms, i32::from(x), i32::from(y)))
    {
        let _ = write!(o, ",\"lv\":{}", r.level);
    }
    if let Some(f) = fields.filter(|_| unit_type == 4) {
        // The item's own base array: stats 0–3, 6–12 read 0 when absent.
        let base = |id: u16| {
            f.base
                .iter()
                .find(|p| p.stat == id && p.layer == 0)
                .map_or(0, |p| p.value)
        };
        let _ = write!(
            o,
            ",\"iq\":{},\"if\":{},\"fi\":{},\"il\":{},\"aa\":{},\"pf\":[{},{},{}],\"sf\":[{},{},{}],\"rp\":{},\"rs\":{},\"ik\":[1,666],\"ss\":0",
            f.quality,
            f.flags,
            f.file_index,
            f.ilvl,
            f.auto,
            f.prefix[0],
            f.prefix[1],
            f.prefix[2],
            f.suffix[0],
            f.suffix[1],
            f.suffix[2],
            f.rare.0,
            f.rare.1
        );
        // hp … stx are the full array (the item's property lists
        // included), str … lvl the base array.
        let props = match &u.kind {
            KindData::Item(d) => d.props.as_slice(),
            _ => &[],
        };
        let full = |id: u16| {
            props
                .iter()
                .filter(|p| p.stat == id && p.layer == 0)
                .fold(base(id), |a, p| a.wrapping_add(p.value))
        };
        for (k, id) in [
            ("hp", 6u16),
            ("hpx", 7),
            ("mp", 8),
            ("mpx", 9),
            ("st", 10),
            ("stx", 11),
        ] {
            let _ = write!(o, ",\"{k}\":{}", full(id));
        }
        for (k, id) in [
            ("str", 0u16),
            ("ene", 1),
            ("dex", 2),
            ("vit", 3),
            ("lvl", 12),
        ] {
            let _ = write!(o, ",\"{k}\":{}", base(id));
        }
        o.push_str(",\"is\":[");
        for (i, p) in f.base.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "[{},{},{}]", p.stat, p.layer, p.value);
        }
        o.push(']');
    }
    if matches!(unit_type, 0 | 1 | 3) {
        for (k, id) in [
            ("hp", 6u16),
            ("hpx", 7),
            ("mp", 8),
            ("mpx", 9),
            ("st", 10),
            ("stx", 11),
        ] {
            // The full array of the unit's list:
            // base plus the attached item and state lists.
            let v = if unit_type == 0 {
                w.total(u.key, id, 0)
            } else {
                u.stats.get(&id).copied().unwrap_or(0)
            };
            let _ = write!(o, ",\"{k}\":{v}");
        }
        for (k, id) in [
            ("str", 0u16),
            ("ene", 1),
            ("dex", 2),
            ("vit", 3),
            ("lvl", 12),
        ] {
            let _ = write!(o, ",\"{k}\":{}", u.stats.get(&id).copied().unwrap_or(0));
        }
    }
    let _ = write!(o, ",\"act\":{}", act(w, u, x, y));
    // `q`: every client player has its player data's quest records (three
    // zeroed buffers). The model keeps no copy of bits a 0x58
    // would set: the zero record. A local player's record stays all zero after
    // a C→S 0x58 quest message.
    if unit_type == 0 {
        o.push_str(",\"q\":[]");
    }
    if let Some(g) = owner(w, u, missile) {
        let _ = write!(o, ",\"own\":{g}");
    }
    if let Some(fp) = fp.filter(|_| set == 'S' && unit_type == 2) {
        let cells: Vec<String> = fp.window(x, y).iter().map(i32::to_string).collect();
        let _ = write!(o, ",\"fp\":[{}]", cells.join(","));
    }
    let _ = write!(o, ",\"set\":{}}}", json_string(&set.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    // A socket filler's holder is the item it fills; a mode-6
    // item whose record owner is not an item is not linked (absent).
    #[test]
    fn a_socketed_item_is_held_by_the_item_it_fills() {
        let host = UnitKey::new(4, 30);
        let player = UnitKey::new(0, 1);
        assert_eq!(item_holder(6, Some(host)), Some(host));
        assert_eq!(item_holder(6, Some(player)), None);
        assert_eq!(item_holder(6, None), None);
        assert_eq!(item_holder(1, Some(player)), Some(player));
        assert_eq!(item_holder(3, Some(player)), None);
    }
}
