//! Preset monsters: the room's monster pass, class ranges,
//! regular presets, superuniques,
//! special preset ids ([`SPECIAL_PRESETS`], copied from the TSV and
//! test-checked) and class for level.

use super::placement::{flags, place_at, Placed};
use super::room::pick;
use super::seams::{OwnerKey, PopHost};
use super::spawn::{
    boss_minions_and_init, boss_spawn, champion_minions, members, random_boss, type_flag,
};
use super::Ctx;
use crate::units::{RoomId, UnitId};

/// The preset-monsters table.
pub const PRESET_MONSTERS_TSV: &str =
    include_str!("../../../../../tables/monsters/preset-monsters.tsv");

/// Level 136 (Pandemonium Fortress run): no monster pass.
pub const LEVEL_NO_MONSTER_PASS: i32 = 136;
/// Classes not retried with r = 4 (table).
pub const NO_RETRY: [i32; 8] = [229, 284, 285, 286, 287, 288, 392, 393];

/// When a special preset spawns (TSV `condition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Always,
    /// The room pick returned a record.
    PickRecord,
    Normal,
    Never,
}

/// The class of a special preset (TSV `class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassSrc {
    /// Pick(chance 0, umon 1).
    Pick,
    Fixed(i32),
    /// Row 261's chain, n by level.
    Chain,
    /// Class for level of a fixed class.
    Level(i32),
    /// Class for level of 529 in level 110, else 492.
    LevelDeathmaulerOrImp,
    None,
}

/// The mode of a special preset (TSV `mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeSrc {
    Preset,
    Fixed(u8),
    None,
}

/// The placement of a special preset (TSV `radius`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radius {
    /// Point searched (through).
    Search,
    /// With r = −1, no retry.
    Once,
    /// With r = −1, then r = 4.
    Retry,
    /// A pack at the preset point.
    Pack,
    None,
}

/// The creation flags of a special preset (TSV `flags`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagSrc {
    Fixed(u16),
    /// 8 if `neverCount`, else 0.
    NeverCount,
    None,
}

/// One row of `preset-monsters.tsv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialPreset {
    pub id: i32,
    pub cond: Cond,
    pub class: ClassSrc,
    pub mode: ModeSrc,
    pub radius: Radius,
    pub flags: FlagSrc,
}

const fn sp(
    id: i32,
    cond: Cond,
    class: ClassSrc,
    mode: ModeSrc,
    radius: Radius,
    flags: FlagSrc,
) -> SpecialPreset {
    SpecialPreset {
        id,
        cond,
        class,
        mode,
        radius,
        flags,
    }
}

use ClassSrc as C;
use Cond as K;
use FlagSrc as F;
use ModeSrc as M;
use Radius as R;

/// The rows of `preset-monsters.tsv` in file order.
pub const SPECIAL_PRESETS: [SpecialPreset; 20] = [
    sp(
        2,
        K::PickRecord,
        C::Pick,
        M::Fixed(1),
        R::Search,
        F::Fixed(0x40),
    ),
    sp(3, K::PickRecord, C::Pick, M::Preset, R::Once, F::Fixed(0)),
    sp(4, K::Always, C::Fixed(266), M::Preset, R::Once, F::Fixed(0)),
    sp(5, K::Always, C::Fixed(267), M::Preset, R::Once, F::Fixed(0)),
    sp(8, K::Always, C::Fixed(284), M::Preset, R::Once, F::Fixed(8)),
    sp(10, K::Always, C::Chain, M::Preset, R::Once, F::Fixed(4)),
    sp(11, K::Always, C::Chain, M::Preset, R::Once, F::Fixed(0)),
    sp(
        17,
        K::Always,
        C::Level(19),
        M::Preset,
        R::Retry,
        F::NeverCount,
    ),
    sp(
        18,
        K::Always,
        C::Level(58),
        M::Preset,
        R::Retry,
        F::NeverCount,
    ),
    sp(
        22,
        K::Always,
        C::Level(141),
        M::Preset,
        R::Retry,
        F::NeverCount,
    ),
    sp(
        23,
        K::Always,
        C::Level(278),
        M::Preset,
        R::Retry,
        F::NeverCount,
    ),
    sp(
        24,
        K::Normal,
        C::LevelDeathmaulerOrImp,
        M::Fixed(1),
        R::Pack,
        F::Fixed(0),
    ),
    sp(25, K::Never, C::None, M::None, R::None, F::None),
    sp(
        26,
        K::Normal,
        C::Level(453),
        M::Fixed(1),
        R::Pack,
        F::Fixed(0),
    ),
    sp(27, K::Never, C::None, M::None, R::None, F::None),
    sp(28, K::Never, C::None, M::None, R::None, F::None),
    sp(
        29,
        K::Always,
        C::Level(453),
        M::Fixed(12),
        R::Retry,
        F::NeverCount,
    ),
    sp(
        30,
        K::Always,
        C::LevelDeathmaulerOrImp,
        M::Fixed(12),
        R::Retry,
        F::NeverCount,
    ),
    sp(
        31,
        K::Always,
        C::Level(522),
        M::Fixed(12),
        R::Retry,
        F::NeverCount,
    ),
    sp(
        32,
        K::Always,
        C::Level(438),
        M::Fixed(12),
        R::Retry,
        F::NeverCount,
    ),
];

/// The special preset row of `id`.
// An id the TSV does not list (and no rule names) creates nothing and draws
// nothing.
pub fn special(id: i32) -> Option<&'static SpecialPreset> {
    SPECIAL_PRESETS.iter().find(|r| r.id == id)
}

/// The TSV text of the first six columns of a row, for the check.
fn render(r: &SpecialPreset) -> [String; 6] {
    let name = |c: i32| match c {
        19 => "fallen1",
        58 => "fallenshaman1",
        141 => "fetish1",
        266 => "navi",
        267 => "bloodraven",
        278 => "fetishshaman1",
        284 => "maggotqueen1",
        438 => "reanimatedhorde3",
        453 => "minion1",
        522 => "act5barb1",
        _ => "?",
    };
    let cond = match r.cond {
        K::Always => "always",
        K::PickRecord => "pick record exists",
        K::Normal => "difficulty 0",
        K::Never => "never",
    };
    let class = match r.class {
        C::Pick => "pick(chance 0, umon 1)".to_string(),
        C::Fixed(c) => format!("{c} {}", name(c)),
        C::Chain => "chain(261, n by level)".to_string(),
        C::Level(c) => format!("level({c} {})", name(c)),
        C::LevelDeathmaulerOrImp => "level(level 110 ? 529 deathmauler1 : 492 imp1)".to_string(),
        C::None => "-".to_string(),
    };
    let mode = match r.mode {
        M::Preset => "preset".to_string(),
        M::Fixed(m) => m.to_string(),
        M::None => "-".to_string(),
    };
    let radius = match r.radius {
        R::Search => "search",
        R::Once => "-1",
        R::Retry => "-1 then 4",
        R::Pack => "-1 leader, 3 members",
        R::None => "-",
    };
    let fl = match r.flags {
        F::Fixed(0) => "0".to_string(),
        F::Fixed(f) if f >= 0x10 => format!("{f:#x}"),
        F::Fixed(f) => f.to_string(),
        F::NeverCount => "neverCount?8:0".to_string(),
        F::None => "-".to_string(),
    };
    [
        r.id.to_string(),
        cond.to_string(),
        class,
        mode,
        radius.to_string(),
        fl,
    ]
}

/// Checks [`SPECIAL_PRESETS`] against the TSV (columns `id` … `flags`;
/// `then` and `source` are prose). Errors name the first differing row.
pub fn check_special_table(tsv: &str, table: &[SpecialPreset]) -> Result<(), String> {
    let mut lines = tsv.lines();
    let head = lines.next().ok_or("empty TSV")?;
    if head != "id\tcondition\tclass\tmode\tradius\tflags\tthen\tsource" {
        return Err(format!("header: {head}"));
    }
    let rows: Vec<&SpecialPreset> = table.iter().collect();
    let lines: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if lines.len() != rows.len() {
        return Err(format!("{} TSV rows, {} in code", lines.len(), rows.len()));
    }
    for (line, row) in lines.iter().zip(rows) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 8 {
            return Err(format!("row {line:?}: {} columns", cols.len()));
        }
        let ours = render(row);
        for (i, (a, b)) in cols.iter().zip(ours.iter()).enumerate() {
            if a != b {
                return Err(format!("id {}: column {i}: TSV {a:?}, code {b:?}", row.id));
            }
        }
    }
    Ok(())
}

/// The monster pass over the room's presets.
pub fn place_presets<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, room: RoomId) {
    if cx.host.room_level(room) == LEVEL_NO_MONSTER_PASS {
        return;
    }
    let b = cx.host.room_box(room);
    for (index, p) in cx.host.preset_units(room).into_iter().enumerate() {
        if p.unit_type != 1 || p.done {
            continue;
        }
        let (x, y) = (p.x + b.x, p.y + b.y);
        if let Some(u) = preset_spawn(cx, room, p.class, x, y, p.mode) {
            if p.has_data {
                cx.host.move_preset_path(u, room, index);
            }
            cx.host.preset_created(u, &p);
            cx.host.set_unit_flags(u, 0x300_0000);
        }
    }
}

/// One preset spawn. Returns the monster
/// handed back to the preset caller.
pub fn preset_spawn<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    class: i32,
    x: i32,
    y: i32,
    mode: u8,
) -> Option<UnitId> {
    let m = cx.tables.monstats.len() as i32;
    let s = cx.tables.superuniques.len() as i32;
    if class < 0 {
        None
    } else if class < m {
        regular(cx, room, class, x, y, mode)
    } else if class < m + s {
        superunique(cx, room, x, y, class - m)
    } else {
        special_spawn(cx, room, class - m - s, x, y, mode)
    }
}

/// A regular preset monster.
fn regular<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    mut class: i32,
    mut x: i32,
    y: i32,
    mut mode: u8,
) -> Option<UnitId> {
    let level = cx.host.room_level(room);
    if level == 110 {
        if cx.host.quest_flag(0x1F) {
            class = match class {
                498 => 499,
                517 => 518,
                c => c,
            };
        }
        if cx.info.difficulty != 0 && matches!(class, 453 | 529) {
            return None;
        }
    }
    if class == 434 {
        x -= 1;
        if !cx.host.quest_flag(0x20) {
            mode = 12;
        }
    }
    if cx.tables.mon2(class).is_some_and(|m| m.critter) {
        return None;
    }
    let f = never_count_flags(cx, class);
    let mut placed = place_at(cx, room, None, x, y, class, mode, -1, f);
    if placed == Placed::Failed && !NO_RETRY.contains(&class) {
        placed = place_at(cx, room, None, x, y, class, mode, 4, f);
    }
    let unit = placed.unit()?;
    if matches!(class, 432 | 433) && cx.tables.mon2(class).is_some_and(|m| m.obj_col) {
        barricade_object(cx, unit, class);
    }
    Some(unit)
}

/// The barricade doors: barricadedoor1 (432) creates object
/// 571, barricadedoor2 (433) object 572 (object creation of class; both
/// objects.txt rows are `Dummy` "door blocker") at the monster's position.
fn barricade_object<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, unit: UnitId, class: i32) {
    let object = if class == 432 { 571 } else { 572 };
    let Some(room) = cx.host.unit_room(unit) else {
        return;
    };
    let (x, y) = cx.host.unit_position(unit);
    cx.host.create_object(room, object, x, y);
}

/// A boss group: `count` times a spawn with (game, boss, class, mode,
/// r, flags) (near the boss, no coordinate record); each created unit gets owner
/// data with the boss GUID and
/// joins the boss's minion list.
#[allow(clippy::too_many_arguments)]
fn group_spawn<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    boss: UnitId,
    class: i32,
    mode: u8,
    r: i32,
    count: i32,
    flags: u16,
) {
    for _ in 0..count.max(0) {
        if let Some(m) = super::placement::place_near(cx, None, boss, class, mode, r, flags).unit()
        {
            cx.host.set_owner_data(m, OwnerKey::Guid(boss), 1, 0, 0);
            cx.host.add_minion(boss, m);
        }
    }
}

fn never_count_flags<H: ?Sized>(cx: &Ctx<'_, H>, class: i32) -> u16 {
    if cx.tables.mon(class).is_some_and(|m| m.never_count) {
        flags::NO_COUNT
    } else {
        0
    }
}

pub fn superunique<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    mut x: i32,
    mut y: i32,
    su: i32,
) -> Option<UnitId> {
    if cx.info.difficulty >= 3 {
        return None;
    }
    let rec = cx
        .tables
        .superuniques
        .get(usize::try_from(su).ok()?)?
        .clone();
    if rec.class < 0 {
        return None;
    }
    if rec.stacks == 0 && cx.state.superunique_placed(su) {
        return None;
    }
    if rec.auto_pos != 0 {
        x = 0;
        y = 0;
    }
    let boss = boss_spawn(cx, room, None, x, y, None, rec.class, false)?;
    cx.state.set_superunique_placed(su);
    cx.host.set_type_flags(boss, type_flag::SUPERUNIQUE);
    cx.host.superunique_init(boss, su);
    let (mut min, mut max) = (rec.min_grp as i32, rec.max_grp as i32);
    if min != 0 && max != 0 {
        min += i32::from(cx.info.difficulty);
        max += i32::from(cx.info.difficulty);
    }
    boss_minions_and_init(cx, boss, min, max, None);
    match rec.hc_idx {
        10 => {
            // row hcIdx 10 (read from the base game): the `roll(5)` is
            // on the boss's unit seed and each spawn is in mode 1.
            let n = cx.host.unit_seed(boss).roll(5) as i32 + 2;
            for _ in 0..n {
                let _ = super::placement::place_near(cx, None, boss, 4, 1, 4, flags::NO_PARTY);
            }
            for c in [276, 382, 385, 389] {
                let _ = super::placement::place_near(cx, None, boss, c, 1, 4, flags::NO_PARTY);
            }
        }
        // Open question 4: (mode, r, count, flags).
        42 => group_spawn(cx, boss, 453, 1, 20, 20, 0),
        60 => {
            // HcIdx 60 /: the boss's owner
            // data.
            cx.host.set_owner_data(boss, OwnerKey::Guid(boss), 1, 1, 0);
            let broom = cx.host.unit_room(boss).unwrap_or(room);
            let c = class_for_level(cx, broom, 453);
            group_spawn(cx, boss, c, 1, 10, 20, flags::NO_PARTY);
        }
        62 => group_spawn(cx, boss, 381, 1, 20, 10, flags::NO_PARTY),
        _ => {}
    }
    cx.host.add_modifier(boss, 22, cx.state);
    Some(boss)
}

/// A special preset id.
fn special_spawn<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    id: i32,
    x: i32,
    y: i32,
    preset_mode: u8,
) -> Option<UnitId> {
    let row = *special(id)?;
    let normal = cx.info.difficulty == 0;
    let level = cx.host.room_level(room);
    let mode = match row.mode {
        M::Preset => preset_mode,
        M::Fixed(m) => m,
        M::None => return None,
    };
    match row.cond {
        K::Never => return None,
        K::Normal if !normal => return None,
        _ => {}
    }
    let class = match row.class {
        C::None => return None,
        C::Pick => {
            let lvl = cx.host.populated_level(room);
            let p = pick(cx, lvl, room, 0, true);
            if !p.record {
                return None;
            }
            p.class
        }
        C::Fixed(c) => c,
        C::Chain => chain_class(cx, level),
        C::Level(c) => class_for_level(cx, room, c),
        C::LevelDeathmaulerOrImp => {
            let c = if level == 110 { 529 } else { 492 };
            class_for_level(cx, room, c)
        }
    };
    match row.radius {
        R::Search => {
            // Step 1: never a champion, point searched; nothing returned.
            let _ = random_boss(cx, room, None, class, false, 0, 0, true);
            None
        }
        R::Once if row.class == C::Pick => {
            // Step 2: a champion at the preset point.
            let u = place_at(cx, room, None, x, y, class, mode, -1, 0).unit()?;
            cx.host.add_modifier(u, 16, cx.state);
            champion_minions(cx, None, u, class);
            Some(u)
        }
        R::Once => {
            let f = match row.flags {
                F::Fixed(f) => f,
                F::NeverCount => never_count_flags(cx, class),
                F::None => 0,
            };
            place_at(cx, room, None, x, y, class, mode, -1, f).unit()
        }
        R::Retry => {
            // Step 4.
            let class = fallen_swap(cx, level, class);
            let f = never_count_flags(cx, class);
            let mut placed = place_at(cx, room, None, x, y, class, mode, -1, f);
            if placed == Placed::Failed {
                placed = place_at(cx, room, None, x, y, class, mode, 4, f);
            }
            let u = placed.unit()?;
            if (29..=32).contains(&row.id) {
                cx.host.set_unit_flags(u, 0x200_0000);
            }
            if class == 438 {
                cx.host.schedule_monumod(u);
            }
            Some(u)
        }
        R::Pack => preset_pack(cx, room, class, x, y, mode),
        R::None => None,
    }
}

/// Row 261's `BaseId` followed n steps of `NextInClass`.
fn chain_class<H: ?Sized>(cx: &Ctx<'_, H>, level: i32) -> i32 {
    let n = match level {
        76..=78 => 1,
        92 | 93 => 2,
        _ => 0,
    };
    let t = cx.tables;
    let mut class = t.base_id(261).unwrap_or(261);
    for _ in 0..n {
        match t.mon(class).map(|m| i32::from(m.next_in_class)) {
            Some(next) if t.mon(next).is_some() => class = next,
            _ => break,
        }
    }
    class
}

/// The fallen / shaman swap by level.
fn fallen_swap<H: ?Sized>(cx: &Ctx<'_, H>, level: i32, class: i32) -> i32 {
    match (cx.tables.base_id(class), level) {
        (Some(19), 6) => 20,
        (Some(19), 7 | 12 | 16) => 21,
        (Some(58), 6 | 7) => 59,
        (Some(58), 12 | 16) => 60,
        _ => class,
    }
}

/// A pack at the preset point.
fn preset_pack<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    class: i32,
    x: i32,
    y: i32,
    mode: u8,
) -> Option<UnitId> {
    let mon = cx.tables.mon(class)?;
    let (min, max) = (i32::from(mon.min_grp), i32::from(mon.max_grp));
    if min < 1 || max < min {
        return None;
    }
    let cl = cx.host.coord_at(room, x, y);
    let leader = place_at(cx, room, cl, x, y, class, mode, -1, 0).unit()?;
    members(cx, cl, leader, class, min, max);
    Some(leader)
}

/// (D2MOO `D2Common_11063`).
pub fn class_for_level<H: PopHost + ?Sized>(cx: &Ctx<'_, H>, room: RoomId, class: i32) -> i32 {
    let t = cx.tables;
    let Some(l) = t.level(cx.host.room_level(room)) else {
        return class;
    };
    if l.mon.is_empty() {
        return class;
    }
    let Some(b) = t.base_id(class) else {
        return class;
    };
    if let Some(&e) = l.mon.iter().find(|&&e| t.base_id(i32::from(e)) == Some(b)) {
        return i32::from(e);
    }
    let lim = i32::from(l.mon_lvl_ex[0]) + 1;
    let len = t.mon(class).map_or(0, |m| m.chain_len);
    let mut cur = t.mon(b).map_or(-1, |m| i32::from(m.next_in_class));
    let mut accepted = None;
    for _ in 0..len {
        match t.mon(cur) {
            Some(m) if i32::from(m.level) <= lim => {
                accepted = Some(cur);
                cur = i32::from(m.next_in_class);
            }
            _ => break,
        }
    }
    accepted.unwrap_or(class)
}

/// (`MONSTERS_GetSpawnMode_XY`): `spawn` and its point
/// and mode for a unit of `class` at (x, y). Read by the skill summon
/// resolver, not by population.
pub fn spawn_mode_xy<H: ?Sized>(
    cx: &Ctx<'_, H>,
    class: i32,
    x: i32,
    y: i32,
) -> Option<(i32, i32, i32, u8)> {
    let m = cx.tables.mon(class)?;
    let mode = if m.spawn_mode <= 15 { m.spawn_mode } else { 1 };
    Some((
        i32::from(m.spawn),
        x + i32::from(m.spawn_x),
        y + i32::from(m.spawn_y),
        mode,
    ))
}
