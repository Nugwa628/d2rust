//! Maze level generation: the generation sequence, the layout builders (ring, grow
//! tree, hub, spiral, lava cross), the special cells each level gets, placement
//! against a neighbouring preset level, the theme-cell pass, and the final build
//! that turns every cell into a preset map with a DS1 file choice.
//!
//! The cell primitives these are made of live in `cells`; the special-room tables
//! they stamp from are in `specials`.

use super::cells::{Extreme, Gen};
use super::{LinkTarget, MazeError, MazeLink, MazePresets, Rotation};
use crate::drlg::data::DrlgData;
use crate::drlg::{DrlgRoomId, TileRect};

/// Generates the level's cells and builds them: place a centred first cell, run the
/// level type's builder, fit the cells to the level rect (or to a neighbouring
/// preset level), add theme cells, then build.
pub fn generate(
    gen: &mut Gen<'_>,
    data: &DrlgData,
    presets: &mut dyn MazePresets,
    rotation: &mut Vec<Rotation>,
) -> Result<(), MazeError> {
    // The first cell, centred (signed division truncates).
    let f = gen.alloc();
    let lr = gen.drlg.level(gen.level).rect;
    {
        let r = &mut gen.drlg.room_mut(f).rect;
        r.x = lr.x + (lr.w - gen.row.size_x) / 2;
        r.y = lr.y + (lr.h - gen.row.size_y) / 2;
    }
    gen.add(f);
    // The level type's layout and special cells.
    builder(gen, f)?;
    // Fit the cells to the level rect; two levels instead move against a preset level.
    match gen.level_id {
        28 => barracks(gen, data, presets)?,
        107 => river_of_flame(gen, data, presets)?,
        _ => normalize(gen)?,
    }
    // Theme cells, then the build.
    theme(gen)?;
    build(gen, data, presets, rotation)
}

/// Runs the builder of the level's type: a layout (ring, grow tree, hub, spiral, lava
/// cross) followed by the level's special cells. `f` is the first cell.
pub(super) fn builder(gen: &mut Gen<'_>, f: DrlgRoomId) -> Result<(), MazeError> {
    match gen.level_type {
        3 => {
            grow_tree(gen)?;
            cave(gen)
        }
        4 => {
            grow_tree(gen)?;
            crypt(gen)
        }
        7 => {
            ring(gen, f, 2)?;
            grow_tree(gen)
        }
        8 => {
            ring(gen, f, 2)?;
            grow_tree(gen)?;
            jail(gen)
        }
        10 => {
            catacomb_start(gen, f)?;
            grow_tree(gen)?;
            catacombs(gen)
        }
        13 => {
            ring(gen, f, 2)?;
            grow_tree(gen)?;
            act2_sewers(gen)
        }
        14 | 15 | 23 => ring(gen, f, 2),
        17 if gen.level_id == 61 => {
            gen.set(f, 480, -1);
            Ok(())
        }
        17 => {
            hub(gen, f, &TOMB_HUB)?;
            grow_tree(gen)?;
            tombs(gen)
        }
        18 => {
            ring(gen, f, 2)?;
            grow_tree(gen)?;
            lair(gen)
        }
        19 => {
            spiral(gen, f)?;
            arcane(gen)
        }
        22 => {
            ring(gen, f, 2)?;
            grow_tree(gen)?;
            durance(gen)
        }
        24 => {
            ring(gen, f, 2)?;
            grow_tree(gen)?;
            dungeon(gen)
        }
        25 => {
            if gen.level_id == 92 {
                ring(gen, f, 5)?;
                sewer_corner_swaps(gen)?;
            } else {
                ring(gen, f, 2)?;
            }
            grow_tree(gen)?;
            act3_sewers(gen)
        }
        28 => grow_tree(gen),
        32 => {
            ring(gen, f, 2)?;
            temple(gen)
        }
        33 => match gen.level_id {
            114 => {
                let def = if gen.drlg.level_mut(gen.level).seed.roll(2) != 0 {
                    1038
                } else {
                    1039
                };
                gen.set(f, def, -1);
                Ok(())
            }
            116 => {
                gen.set(f, 1040, -1);
                Ok(())
            }
            119 => {
                gen.set(f, 1041, -1);
                Ok(())
            }
            _ => {
                ring(gen, f, 2)?;
                grow_tree(gen)?;
                ice(gen)
            }
        },
        34 => {
            hub(gen, f, &BAAL_HUB)?;
            grow_tree(gen)?;
            baal(gen)
        }
        35 => lava_cross(gen, f),
        t => Err(MazeError::BadLevelType(t)),
    }
}

// ---- layout builders ---------------------------------------------------------

/// Ring(n): the border of an n × n square with F at its south-east corner. A
/// rejected grow is an error (`NullCell`): the next step would have no parent.
pub fn ring(gen: &mut Gen<'_>, f: DrlgRoomId, n: u32) -> Result<(), MazeError> {
    let mut p = f;
    for (count, dir) in [(n - 1, 1), (n - 1, 0), (n - 1, 3), (n - 2, 2)] {
        for _ in 0..count {
            p = gen.grow(p, dir)?.ok_or(MazeError::NullCell("ring"))?;
        }
    }
    gen.link(p, f, 2);
    gen.pick(p)?;
    gen.pick(f)
}

/// Grow tree: grows random unlocked cells in random directions until the level has
/// its target room count (`grow_target`).
pub fn grow_tree(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let d = gen.drlg.difficulty;
    let base = *gen
        .row
        .rooms
        .get(d as usize)
        .ok_or(MazeError::BadDifficulty(d))?;
    let target = grow_target(base, gen.level_id, gen.drlg.staff_tomb, gen.drlg.boss_tomb);
    while gen.count() < target {
        let r = gen
            .random_cell()
            .ok_or(MazeError::NullCell("grow tree random cell"))?;
        let dir = (gen.room_step(r) & 3) as u8;
        if !gen.cell(r).lock {
            gen.grow(r, dir)?;
        }
    }
    Ok(())
}

/// Grow-tree target room count: `Rooms[d]`, ×3 for the staff tomb level, ×2
/// for the boss tomb level.
pub fn grow_target(rooms: u32, level_id: u32, staff_tomb: u32, boss_tomb: u32) -> u32 {
    if level_id == staff_tomb {
        rooms.wrapping_mul(3)
    } else if level_id == boss_tomb {
        rooms.wrapping_mul(2)
    } else {
        rooms
    }
}

/// Catacomb start: grows the first cell into a fixed opening shape (four arms on
/// level 34, otherwise two opposite arms picked by a level-seed step) and sets its def.
pub(super) fn catacomb_start(gen: &mut Gen<'_>, f: DrlgRoomId) -> Result<(), MazeError> {
    let def = if gen.level_id == 34 {
        for d in [1, 2, 3, 0] {
            gen.grow(f, d)?;
        }
        290
    } else if gen.level_step() & 1 == 0 {
        gen.grow(f, 1)?;
        gen.grow(f, 3)?;
        289
    } else {
        gen.grow(f, 0)?;
        gen.grow(f, 2)?;
        288
    };
    gen.set(f, def, -1);
    Ok(())
}

/// Hub defs by open side q: the tomb hubs and the Worldstone Keep hubs.
const TOMB_HUB: [u32; 4] = [447, 444, 446, 445];
const BAAL_HUB: [u32; 4] = [1075, 1077, 1076, 1074];

/// Hub: grows the first cell in three consecutive directions from a random start,
/// leaving one side open, and gives it the hub def for that side.
pub(super) fn hub(gen: &mut Gen<'_>, f: DrlgRoomId, table: &[u32; 4]) -> Result<(), MazeError> {
    let r = gen.level_step() & 3;
    for i in 0..3 {
        gen.grow(f, ((r + i) % 4) as u8)?;
    }
    gen.set(f, table[((r + 3) % 4) as usize], -1);
    Ok(())
}

/// Spiral direction offsets o_k: the turn of the k-th cell of a branch.
const SPIRAL_OFFSETS: [u32; 15] = [0, 0, 3, 0, 0, 0, 0, 1, 0, 1, 2, 2, 3, 2, 2];

/// Arcane spiral: four branches of 15 grown cells around the first cell; a cell's file
/// is its branch number offset by a random start, mod 4.
pub(super) fn spiral(gen: &mut Gen<'_>, f: DrlgRoomId) -> Result<(), MazeError> {
    let r = gen.level_step() & 3;
    // The 60-slot record: slot 15·b + k; rejected cells and the dead
    // ends k = 8, 12 leave it empty.
    let mut slots: [Option<DrlgRoomId>; 60] = [None; 60];
    for b in 0..4u32 {
        let mut cells: [Option<DrlgRoomId>; 15] = [None; 15];
        for k in 0..15 {
            let parent = match k {
                0 => Some(f),
                9 => cells[7],
                13 => cells[11],
                _ => cells[k - 1],
            };
            let parent = parent.ok_or(MazeError::NullCell("arcane spiral"))?;
            cells[k] = gen.grow(parent, ((b + SPIRAL_OFFSETS[k]) % 4) as u8)?;
            if k != 8 && k != 12 {
                slots[15 * b as usize + k] = cells[k];
            }
        }
    }
    // After all four branches: file = (r + slot / 15) mod 4, so a file
    // reset to −1 by a later branch's merge is overwritten.
    for (slot, c) in slots.iter().enumerate() {
        if let Some(c) = c {
            gen.cell_mut(*c).file = ((r + slot as u32 / 15) % 4) as i32;
        }
    }
    gen.cell_mut(f).file = 4;
    Ok(())
}

/// Lava cross table: (def, dir, file), two entries per random choice.
const LAVA_CROSS: [(u32, u8, i32); 8] = [
    (1054, 1, 0),
    (1053, 3, 1),
    (1054, 1, 1),
    (1053, 3, 0),
    (1055, 0, 1),
    (1056, 2, 0),
    (1055, 0, 0),
    (1056, 2, 1),
];

/// Act 5 lava cross: two fixed arms chosen by a room-seed step, then blanks around
/// everything.
pub(super) fn lava_cross(gen: &mut Gen<'_>, f: DrlgRoomId) -> Result<(), MazeError> {
    let s = (gen.room_step(f) & 3) as usize;
    for (def, dir, file) in [LAVA_CROSS[2 * s], LAVA_CROSS[2 * s + 1]] {
        gen.fixed(f, dir, def, file, true)?;
    }
    fill_blanks(gen, 836, None);
    Ok(())
}

/// Fill blanks: eight "blank" attaches around every
/// cell of the list as it was, except `excluded`.
pub fn fill_blanks(gen: &mut Gen<'_>, def: u32, excluded: Option<DrlgRoomId>) {
    for c in gen.list() {
        if Some(c) == excluded {
            continue;
        }
        for j in 0..8 {
            gen.blank(c, j, def);
        }
    }
}

/// Act 3 Sewers 1 corner swaps after its ring(5): four corner defs are replaced.
pub(super) fn sewer_corner_swaps(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    for (old, new) in [(709, 735), (710, 736), (713, 737), (714, 738)] {
        let c = first_unlocked(gen, |d| d == old).ok_or(MazeError::MissingSwap(old))?;
        gen.set(c, new, -1);
    }
    Ok(())
}

/// The first unlocked cell in list order whose def passes `f`.
pub(super) fn first_unlocked(gen: &Gen<'_>, f: impl Fn(u32) -> bool) -> Option<DrlgRoomId> {
    gen.list().into_iter().find(|&c| {
        let cell = gen.cell(c);
        !cell.lock && f(cell.def)
    })
}

// ---- special cells -----------------------------------------------------------

/// The common special builder: r = level-seed step & 3, then the
/// stamps in order, the counter advancing after each.
pub(super) fn stamps(gen: &mut Gen<'_>, kinds: &[&'static str]) -> Result<(), MazeError> {
    let mut r = gen.level_step() & 3;
    for &k in kinds {
        gen.stamp_next(k, &mut r)?;
    }
    Ok(())
}

/// Special cells of cave levels.
pub(super) fn cave(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    // The builder tests level ids: cave_prev always; cave_doe if 8
    // else cave_down; cave_coldcrow if 9; cave_next if 10 (so 13–16 get
    // cave_prev, cave_down).
    let id = gen.level_id;
    let mut kinds = vec!["cave_prev", if id == 8 { "cave_doe" } else { "cave_down" }];
    if id == 9 {
        kinds.push("cave_coldcrow");
    }
    if id == 10 {
        kinds.push("cave_next");
    }
    stamps(gen, &kinds)
}

/// Special cells of crypt levels.
pub(super) fn crypt(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    // crypt_prev always (so 25 gets crypt_prev only).
    let kinds: &[&'static str] = match gen.level_id {
        18 => &["crypt_prev", "crypt_bonebreak"],
        19 | 133 => &["crypt_prev", "crypt_chest"],
        21..=24 => &["crypt_prev", "crypt_next"],
        _ => &["crypt_prev"],
    };
    stamps(gen, kinds)
}

/// Special cells of jail levels.
pub(super) fn jail(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        29 => &["jail_prev", "jail_waypoint", "jail_next"],
        30 => &["jail_prev", "jail_pitspawn", "jail_next"],
        31 => &["jail_prev", "jail_cath"],
        _ => &[],
    };
    stamps(gen, kinds)
}

/// Special cells of catacomb levels.
pub(super) fn catacombs(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        // catacombs_next always (so 37 gets it).
        35 => &["catacombs_next", "catacombs_waypoint"],
        _ => &["catacombs_next"],
    };
    stamps(gen, kinds)
}

/// Special cells of the Arcane Sanctuary.
pub(super) fn arcane(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        74 => &["arcane_summoner"],
        _ => &[],
    };
    stamps(gen, kinds)
}

/// Special cells of the Flayer / Swampy dungeons.
pub(super) fn dungeon(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    // Both stamps unconditionally (86–91).
    stamps(gen, &["dungeon_prev", "dungeon_next"])
}

/// Special cells of the Act 3 sewers.
pub(super) fn act3_sewers(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    // Both stamps unconditionally (92, 93).
    stamps(gen, &["a3sewer_drain", "a3sewer_chest"])
}

/// Special cells of the Durance of Hate.
pub(super) fn durance(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        100 => &["meph_prev", "meph_next"],
        101 => &["meph_prev", "meph_waypoint", "meph_next"],
        _ => &[],
    };
    stamps(gen, kinds)
}

/// Special cells of the Worldstone Keep.
pub(super) fn baal(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        // baal_next always (so 132 gets it).
        129 => &["baal_next", "baal_waypoint"],
        _ => &["baal_next"],
    };
    stamps(gen, kinds)
}

/// Special cells of the Act 5 ice caves.
pub(super) fn ice(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let kinds: &[&'static str] = match gen.level_id {
        113 | 118 => &["ice_prev", "ice_next", "ice_down", "ice_waypoint"],
        115 => &[
            "ice_prev",
            "ice_next",
            "ice_down",
            "ice_theme",
            "ice_waypoint",
        ],
        _ => &[],
    };
    stamps(gen, kinds)
}

/// Special cells of the lair levels (62, 63, 64).
pub(super) fn lair(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let mut r = gen.level_step() & 3;
    match gen.level_id {
        64 => {
            gen.stamp("lair_tightspot", 0)?;
            r = 3;
            gen.stamp_next("lair_treasure", &mut r)?;
            gen.stamp_next("lair_prev", &mut r)
        }
        62 | 63 => {
            gen.stamp_next("lair_next", &mut r)?;
            gen.stamp_next("lair_prev", &mut r)
        }
        _ => Ok(()),
    }
}

/// Special cells of the Act 2 sewers; Sewers 1 (47) also gets extra cells at its
/// north and east extremes.
pub(super) fn act2_sewers(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let a = gen.level_step();
    let mut r = gen.level_step() & 3;
    let e = ((a & 1) * 2 + 1) as u8;
    let null = |what| move || MazeError::NullCell(what);
    match gen.level_id {
        47 => {
            let a = gen
                .extreme(Extreme::MinY)?
                .ok_or_else(null("sewers 1 north"))?;
            let b = gen.grow(a, 1)?.ok_or_else(null("sewers 1 north"))?;
            let c = gen.grow(b, 1)?.ok_or_else(null("sewers 1 north"))?;
            gen.fixed(c, 0, 333, 0, true)?;
            let d = gen
                .extreme(Extreme::MaxX)?
                .ok_or_else(null("sewers 1 east"))?;
            let e1 = gen.grow(d, 2)?.ok_or_else(null("sewers 1 east"))?;
            let e2 = gen.grow(e1, 2)?.ok_or_else(null("sewers 1 east"))?;
            let g = gen
                .fixed(e2, e, 336, 0, true)?
                .ok_or_else(null("sewers 1 G"))?;
            if let Some(h) = gen.place(g, e) {
                gen.link(g, h, e);
                gen.add(h);
                gen.pick(h)?;
            }
            gen.stamp_next("a2sewer_next", &mut r)
        }
        48 => {
            for k in ["a2sewer_prev", "a2sewer_waypoint", "a2sewer_next"] {
                gen.stamp_next(k, &mut r)?;
            }
            Ok(())
        }
        49 => {
            gen.stamp_next("a2sewer_prev", &mut r)?;
            gen.stamp_next("a2sewer_radament", &mut r)
        }
        65 => {
            gen.stamp_next("a2sewer_prev", &mut r)?;
            gen.stamp_next("a2sewer_chest", &mut r)
        }
        _ => Ok(()),
    }
}

/// Special cells of the tomb levels. Draws no seed step: the stamp counter starts from
/// the hub cell's def.
pub(super) fn tombs(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let hub = gen
        .list()
        .into_iter()
        .map(|c| gen.cell(c).def)
        .find(|&d| d > 428);
    let mut r = match hub {
        Some(446) => 0,
        Some(445) => 1,
        Some(447) => 2,
        Some(444) => 3,
        other => return Err(MazeError::TombHub(other)),
    };
    let id = gen.level_id;
    let (staff, boss) = (gen.drlg.staff_tomb, gen.drlg.boss_tomb);
    let lines: [(&'static str, bool); 9] = [
        ("tomb_next", (55..=58).contains(&id)),
        ("tomb_waypoint", id == 57),
        ("tomb_chest", id == 59 || id == 61),
        ("tomb_chest", (66..=72).contains(&id) && id != staff),
        ("tomb_leatherarm", id == 59),
        ("tomb_cube", id == 60),
        ("tomb_treasure", id == 59 || id == 61),
        ("tomb_talrasha", id == staff),
        ("tomb_kaa", id == boss),
    ];
    for (k, on) in lines {
        if on {
            gen.stamp_next(k, &mut r)?;
        }
    }
    Ok(())
}

/// Special cells of the Act 5 temple levels; the first row comes from a level-seed step mod 3.
pub(super) fn temple(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let mut r = gen.level_step() % 3;
    if gen.level_id != 124 {
        gen.stamp_next("temple_down", &mut r)?;
    }
    if gen.level_id == 123 {
        gen.stamp_next("temple_waypoint", &mut r)?;
    }
    Ok(())
}

// ---- neighbouring preset levels ------------------------------------------------

/// Shifts the cells by (dx, dy) and sets the level rect to their bounding box.
pub(super) fn shift_and_fit(gen: &mut Gen<'_>, dx: i32, dy: i32) {
    gen.shift(dx, dy);
    let bb = gen.bounding_box();
    gen.drlg.level_mut(gen.level).rect = bb;
}

/// Barracks: attaches a court cell at the side facing the Outer Cloister preset
/// level, links it across levels, and shifts the cells to sit against that level.
pub(super) fn barracks(
    gen: &mut Gen<'_>,
    data: &DrlgData,
    presets: &mut dyn MazePresets,
) -> Result<(), MazeError> {
    let l = presets.level(gen.drlg, data, 27)?;
    let q = presets.preset_direction(gen.drlg, l)?;
    let (finder, dir) = match q {
        0 => (Extreme::MaxX, 2),
        1 => (Extreme::MaxY, 3),
        2 => (Extreme::MinX, 0),
        q => return Err(MazeError::BarracksDirection(q)),
    };
    let null = || MazeError::NullCell("barracks court cell");
    let p = gen.extreme(finder)?.ok_or_else(null)?;
    let c = gen.fixed(p, dir, 167, q as i32, true)?.ok_or_else(null)?;
    gen.link_level(c, l, dir);
    let (lr, cr) = (gen.drlg.level(l).rect, gen.rect(c));
    let (sx, sy) = (gen.row.size_x, gen.row.size_y);
    let (dx, dy) = match q {
        0 => (lr.x - (cr.x + sx), lr.y + lr.h / 2 - cr.y),
        1 => (lr.x + lr.w / 2 - cr.x - 6, lr.y - (cr.y + sy)),
        _ => (lr.x + lr.w - cr.x, lr.y + lr.h / 2 - cr.y + 1),
    };
    let q = q as usize;
    if gen.level_step() & 1 != 0 {
        gen.stamp("barracks_next", q)?;
        gen.stamp("barracks_forge", q + 1)?;
    } else {
        gen.stamp("barracks_forge", q)?;
        gen.stamp("barracks_next", q + 1)?;
    }
    shift_and_fit(gen, dx, dy);
    Ok(())
}

/// River of Flame: attaches a warp cell and a three-cell bridge to the Chaos Sanctum
/// preset level, fills blanks and shifts the cells against it. Act 4 does nothing.
pub(super) fn river_of_flame(
    gen: &mut Gen<'_>,
    data: &DrlgData,
    presets: &mut dyn MazePresets,
) -> Result<(), MazeError> {
    if gen.drlg.act == 4 {
        return Ok(());
    }
    let s = presets.level(gen.drlg, data, 108)?;
    let null = |what| move || MazeError::NullCell(what);
    let p = gen.extreme(Extreme::MaxY)?.ok_or_else(null("river warp"))?;
    gen.fixed(p, 3, 852, -1, true)?;
    let p = gen
        .extreme(Extreme::MinY)?
        .ok_or_else(null("river bridge"))?;
    let b1 = gen
        .fixed(p, 1, 855, -1, true)?
        .ok_or_else(null("river bridge 1"))?;
    let b2 = gen.blank(b1, 1, 856).ok_or_else(null("river bridge 2"))?;
    let b3 = gen.blank(b2, 1, 856).ok_or_else(null("river bridge 3"))?;
    gen.link_level(b3, s, 1);
    let (sr, br) = (gen.drlg.level(s).rect, gen.rect(b3));
    let (dx, dy) = (sr.x + 2 * br.w - br.x, sr.y + sr.h - br.y);
    let row = (gen.level_step() & 1) as usize;
    gen.stamp("a4lava_forge", row)?;
    fill_blanks(gen, 836, Some(b3));
    shift_and_fit(gen, dx, dy);
    Ok(())
}

/// Moves the cells so their bounding box starts at the level rect's origin; fails if
/// they do not fit the rect.
pub(super) fn normalize(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let bb = gen.bounding_box();
    let lr = gen.drlg.level(gen.level).rect;
    if bb.w > lr.w || bb.h > lr.h {
        return Err(MazeError::DoesNotFit);
    }
    gen.shift(lr.x - bb.x, lr.y - bb.y);
    Ok(())
}

// ---- theme cells ---------------------------------------------------------------

/// Theme base def by level type; `None` for types without theme cells.
pub fn theme_base(level_type: u32) -> Option<u32> {
    Some(match level_type {
        3 => 52,
        4 => 108,
        7 => 167,
        8 => 205,
        10 => 257,
        13 => 301,
        17 => 413,
        22 => 753,
        24 => 664,
        25 => 704,
        _ => return None,
    })
}

/// Theme need (cells to convert) and tries for a room count.
pub fn theme_budget(count: u32) -> (u32, u32) {
    ((count / 5 + 1).max(2), count.wrapping_mul(2))
}

/// Theme pass: converts some unlocked cells whose def is a theme base variant (picked
/// in a shuffled order) to their themed def (+15). Level 8 and types without a theme
/// base are skipped.
pub(super) fn theme(gen: &mut Gen<'_>) -> Result<(), MazeError> {
    let Some(base) = theme_base(gen.level_type) else {
        return Ok(());
    };
    if gen.level_id == 8 {
        return Ok(());
    }
    let mut start = (gen.level_step() % 15) as usize;
    let mut a: [u32; 15] = std::array::from_fn(|i| i as u32);
    for _ in 0..15 {
        let i = (gen.level_step() % 15) as usize;
        let j = (gen.level_step() % 15) as usize;
        a.swap(i, j);
    }
    let (mut need, mut tries) = theme_budget(gen.count());
    while need > 0 && tries > 0 {
        let target = base + a[start];
        if let Some(c) = first_unlocked(gen, |d| d == target) {
            gen.set(c, target + 15, -1);
            need -= 1;
        }
        tries -= 1;
        start = (start + 1) % 15;
    }
    Ok(())
}

// ---- build ---------------------------------------------------------------------

/// File-rotation base B' by level type: cells with a def in B'+1..B'+15 and file −1
/// rotate through the def's files; `None` for types without rotation.
pub fn rotation_base(level_type: u32) -> Option<u32> {
    Some(match level_type {
        3 => 52,
        4 => 108,
        7 => 167,
        8 => 205,
        10 => 257,
        13 => 301,
        17 => 413,
        18 => 481,
        22 => 753,
        24 => 664,
        25 => 704,
        28 => 836,
        33 => 1002,
        34 => 1058,
        35 => 1052,
        _ => return None,
    })
}

/// Builds every cell in list order: allocate its preset map, choose the DS1 file,
/// build the map into rooms, link the built room to the cell's neighbours, then free
/// the cell.
pub(super) fn build(
    gen: &mut Gen<'_>,
    data: &DrlgData,
    presets: &mut dyn MazePresets,
    rotation: &mut Vec<Rotation>,
) -> Result<(), MazeError> {
    for c in gen.list() {
        let (def, file) = (gen.cell(c).def, gen.cell(c).file);
        let rect: TileRect = gen.rect(c);
        let map = presets.alloc_map(gen.drlg, data, gen.level, def, rect)?;
        if file != -1 {
            presets.set_map_file(map, file);
        } else if rotation_base(gen.level_type).is_some_and(|b| b < def && def < b + 16) {
            let pos = match rotation.iter().position(|r| r.def == def) {
                Some(p) => p,
                None => {
                    let n = gen.data.files(def)?;
                    let v = gen.drlg.level_mut(gen.level).seed.roll(n as i32) as i32;
                    rotation.insert(0, Rotation { def, n, v });
                    0
                }
            };
            let rec = &mut rotation[pos];
            if rec.n == 0 {
                return Err(MazeError::ZeroFiles(def));
            }
            rec.v = (rec.v + 1) % rec.n as i32;
            presets.set_map_file(map, rec.v);
        }
        let small = rect.w <= 12 && rect.h <= 12;
        let links: Vec<MazeLink> = gen
            .cell(c)
            .links
            .iter()
            .copied()
            .filter(|l| l.init)
            .collect();
        // Build the map (room flags are the provider's), link the room it returns to
        // each init link in list order, then free the cell, which removes the
        // neighbours' links back to it.
        let built = presets.build_map(gen.drlg, data, gen.level, map, small, &links)?;
        if let Some(b) = built {
            gen.cells.entry(b).or_default();
            for l in &links {
                if let LinkTarget::Cell(n) = l.target {
                    gen.link(b, n, l.dir);
                }
            }
        } else if !links.is_empty() {
            // Building returns a room for every cell size lvlmaze has, so none here means
            // the provider failed; there is nothing to link.
            return Err(MazeError::NullCell("built room"));
        }
        gen.free_listed(c);
    }
    Ok(())
}
