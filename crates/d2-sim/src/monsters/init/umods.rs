//! Unique-monster mods (umods): boss spawns after the spawn, the umod
//! choice for champions and uniques, boss minions and umod init, the umod
//! init functions, superuniques, the restore paths, and the umod callback
//! dispatcher with its timer event (type 7). [`UMODS`] is copied from
//! `tables/monsters/umods.tsv` (columns `id`, `uniquemod`, `init_fn`,
//! `unique_gate`, `cb_mode0`…`cb_mode5`) and checked against it by
//! `tests::umods_match_tsv`. A mod game's umods
//! (`docs/plans/mods/seams.md` MS5) run before D2RUST's: its init in
//! [`run_umod_init`], its callbacks in [`dispatch`].

use crate::rng::Seed;
use crate::units::{UnitId, UnitType};

use super::calc::{monlvl_dm, pct};
use super::create::seed;
use super::{
    s16, stat, type_flag, CreateRequest, Ctx, InitHost, MonsterData, Unhandled, EVENT_UMOD,
    MAX_UMODS,
};

/// How an init function treats its `unique` argument (`unique_gate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// No init function (`-`).
    None,
    /// Does nothing when unique = 0 (`yes`).
    Unique,
    /// Ignores it (`no`).
    Any,
    /// Different constants (`branch`).
    Branch,
}

/// One umod: init function and callbacks.
/// Function ids; 0 = none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UmodRow {
    pub id: u8,
    pub name: &'static str,
    pub init_fn: u32,
    pub gate: Gate,
    /// Modes 0..5.
    pub callbacks: [u32; 6],
}

const fn row(id: u8, name: &'static str, init_fn: u32, gate: Gate, callbacks: [u32; 6]) -> UmodRow {
    UmodRow {
        id,
        name,
        init_fn,
        gate,
        callbacks,
    }
}

/// The catalogue text, for the consistency test.
pub const UMODS_TSV: &str = include_str!("../../../../../tables/monsters/umods.tsv");

/// The umod table, by id.
pub const UMODS: [UmodRow; 43] = [
    row(0, "none", 0, Gate::None, [0, 0, 0, 0, 0, 0]),
    row(1, "rndname", 0x005A_0CE0, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(
        2,
        "hpmultiply",
        0x005A_0DC0,
        Gate::Branch,
        [0, 0, 0, 0, 0, 0],
    ),
    row(3, "light", 0, Gate::None, [0, 0, 0, 0, 0, 0]),
    row(4, "leveladd", 0x005A_0E40, Gate::Any, [0, 0, 0, 0, 0, 0]),
    row(5, "strong", 0x005A_17E0, Gate::Branch, [0, 0, 0, 0, 0, 0]),
    row(6, "fast", 0x005A_1910, Gate::Any, [0, 0, 0, 0, 0, 0]),
    row(7, "curse", 0, Gate::None, [0, 0, 0, 0x005A_2530, 0, 0]),
    row(8, "resist", 0x005A_1370, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(
        9,
        "fire",
        0x005A_1990,
        Gate::Branch,
        [0, 0x005A_25F0, 0x005A_2620, 0, 0, 0],
    ),
    row(
        10,
        "poisondead",
        0,
        Gate::None,
        [0, 0x005A_3800, 0x005A_2C20, 0, 0, 0],
    ),
    row(11, "durieldead", 0, Gate::None, [0, 0, 0, 0, 0, 0]),
    row(12, "bloodraven", 0, Gate::None, [0, 0, 0, 0, 0, 0]),
    row(13, "rage", 0, Gate::None, [0, 0, 0, 0, 0, 0]),
    row(14, "spcdamage", 0, Gate::None, [0x005A_3B50, 0, 0, 0, 0, 0]),
    row(15, "partydead", 0, Gate::None, [0, 0x005A_2D10, 0, 0, 0, 0]),
    row(
        16,
        "champion",
        0x005A_0E80,
        Gate::Unique,
        [0, 0, 0, 0, 0, 0],
    ),
    row(
        17,
        "lightning",
        0x005A_1B00,
        Gate::Branch,
        [0, 0x005A_37D0, 0x005A_29A0, 0, 0x005A_2BA0, 0],
    ),
    row(
        18,
        "cold",
        0x005A_1C70,
        Gate::Branch,
        [0, 0x005A_3800, 0x005A_2BD0, 0, 0, 0],
    ),
    row(
        19,
        "hireable",
        0,
        Gate::None,
        [0x005A_2D80, 0, 0, 0, 0, 0x005A_2E00],
    ),
    row(
        20,
        "scarab",
        0,
        Gate::None,
        [0, 0x005A_2EB0, 0x005A_2EE0, 0, 0, 0],
    ),
    row(21, "killself", 0, Gate::None, [0, 0, 0x005A_3AA0, 0, 0, 0]),
    row(
        22,
        "questcomplete",
        0,
        Gate::None,
        [0, 0x005A_3250, 0, 0, 0, 0],
    ),
    row(
        23,
        "poisonhit",
        0x005A_1E00,
        Gate::Branch,
        [0x005A_3490, 0, 0, 0, 0, 0],
    ),
    row(24, "thief", 0, Gate::None, [0, 0, 0, 0x005A_30E0, 0, 0]),
    row(25, "manahit", 0x005A_1F90, Gate::Branch, [0, 0, 0, 0, 0, 0]),
    row(
        26,
        "teleport",
        0x005A_1600,
        Gate::Unique,
        [0, 0, 0, 0, 0, 0],
    ),
    row(
        27,
        "spectralhit",
        0x005A_1370,
        Gate::Unique,
        [0x005A_3040, 0, 0, 0, 0, 0x005A_30B0],
    ),
    row(
        28,
        "stoneskin",
        0x005A_1370,
        Gate::Unique,
        [0, 0, 0, 0, 0, 0],
    ),
    row(29, "multishot", 0, Gate::None, [0, 0, 0, 0, 0, 0x005A_3610]),
    row(30, "aura", 0x005A_1650, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(
        31,
        "goboom",
        0,
        Gate::None,
        [0, 0x005A_3800, 0x005A_2840, 0, 0, 0],
    ),
    row(
        32,
        "firespike_explode",
        0,
        Gate::None,
        [0, 0x005A_3800, 0x005A_3D20, 0, 0, 0],
    ),
    row(
        33,
        "suicideminion_explode",
        0,
        Gate::None,
        [0, 0x005A_3E70, 0x005A_3EF0, 0, 0, 0],
    ),
    row(
        34,
        "ai_after_death",
        0,
        Gate::None,
        [0, 0x005A_3840, 0x005A_3910, 0, 0, 0],
    ),
    row(
        35,
        "shatter_on_death",
        0,
        Gate::None,
        [0, 0x005A_3A80, 0, 0, 0, 0],
    ),
    row(36, "ghostly", 0x005A_1080, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(37, "fanatic", 0x005A_11F0, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(
        38,
        "possessed",
        0x005A_1230,
        Gate::Unique,
        [0, 0, 0, 0, 0, 0],
    ),
    row(39, "berserk", 0x005A_1280, Gate::Unique, [0, 0, 0, 0, 0, 0]),
    row(
        40,
        "worms_on_death",
        0,
        Gate::None,
        [0, 0x005A_4200, 0, 0, 0, 0],
    ),
    row(
        41,
        "always_run_ai",
        0x005A_1330,
        Gate::Any,
        [0, 0, 0x005A_4230, 0, 0, 0],
    ),
    row(
        42,
        "lightningdeath",
        0,
        Gate::None,
        [0, 0x005A_3800, 0x005A_2910, 0, 0, 0],
    ),
];

/// Callback addresses; every one has a body in
/// [`super::callbacks`].
pub use super::callbacks::addr as callback;

/// The aura table: (min level, level offset,
/// multiplier, divisor, skill).
pub const AURAS: [(i32, i32, i32, i32, u16); 8] = [
    (0, 0, 1, 6, 98),
    (0, 0, 1, 6, 102),
    (0, 0, 1, 5, 108),
    (0, 0, 1, 7, 114),
    (0, 0, 1, 8, 123),
    (0, 0, 1, 8, 122),
    (20, 0, 1, 8, 118),
    (999, 0, 0, 1, 103),
];

/// Conviction (class 704's fixed aura).
const CONVICTION: u16 = 123;
/// Willowisp1 `BaseId` (its bonus is halved).
const WILLOWISP: u16 = 118;
/// The umods run before the unit's own.
const FIXED_UMODS: [u8; 4] = [1, 2, 3, 4];

fn data<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) -> MonsterData {
    h.monsters().get(unit).cloned().unwrap_or_default()
}

fn flags_or<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, f: u16) {
    h.monsters().entry(unit).type_flags |= f;
}

fn add<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, s: u16, d: i32) {
    // An add of 0 changes nothing and creates
    // no entry.
    if d == 0 {
        return;
    }
    let v = h.stat(unit, s);
    h.set_stat(unit, s, v.wrapping_add(d));
}

fn schedule<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, n: i32) {
    let g = h.game();
    let at = g.after(n);
    // A monster in the lists always has a timer owner; a unit outside
    // them schedules nothing.
    let _ = g.schedule_event(unit, EVENT_UMOD, at, None, 0, 0);
}

fn class_of<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) -> u32 {
    h.units().get(unit).map_or(0, |r| r.class)
}

/// The unit exists and is a
/// monster (unit type 1).
fn is_monster<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) -> bool {
    h.units()
        .get(unit)
        .is_some_and(|r| r.ty == UnitType::Monster)
}

fn base_id<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) -> Option<u16> {
    let class = class_of(h, unit);
    cx.monstats(class).map(|m| m.baseid)
}

// ---------

/// Unless type flag 8 is set, count the boss in its region;
/// then set type flag 8.
pub fn mark_unique<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) {
    if !data(h, unit).has_flag(type_flag::UNIQUE) {
        h.count_region_boss(unit);
    }
    flags_or(h, unit, type_flag::UNIQUE);
}

/// Step 5: unique mark, type flag 1,
/// quest hook, owner data.
pub fn mark_boss<H: InitHost + ?Sized>(h: &mut H, unit: UnitId) {
    mark_unique(h, unit);
    flags_or(h, unit, type_flag::BOSS);
    h.boss_quest_hook(unit);
    h.boss_owner_data(unit);
}

/// Random boss. `req` carries room, coord list,
/// class and position for the boss spawn.
pub fn random_boss<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    req: &CreateRequest,
    champion_allowed: bool,
    warp_check: bool,
) -> Option<UnitId> {
    let unit = h.boss_spawn(req, None, warp_check)?;
    mark_boss(h, unit);
    choose_umods(cx, h, unit, champion_allowed);
    boss_minions_and_init(cx, h, unit, 3, 6, req.coord_list, true);
    Some(unit)
}

/// Champion pack member.
pub fn champion_pack_member<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, umod: u8) {
    if data(h, unit).has_flag(type_flag::CHAMPION) {
        return;
    }
    mark_unique(h, unit);
    flags_or(h, unit, type_flag::BOSS | type_flag::CHAMPION);
    h.monsters().entry(unit).push_umod(umod);
    boss_minions_and_init(cx, h, unit, 0, 0, None, true);
}

// ---------

/// Eligibility of umod `id` for `class`.
pub fn eligible<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, class: u32, id: usize) -> bool {
    let Some(r) = cx.tables.monumod.get(id) else {
        return false;
    };
    if r.enabled == 0 || (!h.info().expansion && r.version >= 100) {
        return false;
    }
    let Some(m) = cx.monstats(class) else {
        return false;
    };
    for ex in [r.exclude1, r.exclude2] {
        // Matrix row = the exclude type, column =
        // the class's MonType, so set when `ex` is MonType or nested in it.
        if s16(ex) > 0 && h.montype_is(ex, m.montype) {
            return false;
        }
    }
    let m2 = cx.monstats2(class);
    match r.fpick {
        1 => m2.is_some_and(|m2| m2.ma1),
        2 => !(m.ismelee || m.nomultishot),
        3 => m2.is_some_and(|m2| m2.mwl),
        _ => true,
    }
}

fn weighted<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, cands: &[(u8, i32)]) -> u8 {
    let total: i32 = cands.iter().map(|c| c.1).sum();
    let mut r = seed(h, unit).roll(total) as i32;
    for &(id, w) in cands {
        if w > r {
            return id;
        }
        r -= w;
    }
    0
}

/// Champion pick; 0 = none.
pub fn pick_champion<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, d: usize) -> u8 {
    let class = class_of(h, unit);
    let mut cands = Vec::new();
    for (i, r) in cx.tables.monumod.iter().enumerate() {
        let w = i32::from([r.cpick, r.cpick_n, r.cpick_h][d.min(2)]);
        if w > 0 && r.champion != 0 && eligible(cx, h, class, i) {
            cands.push((i as u8, w));
        }
    }
    weighted(h, unit, &cands)
}

/// Unique pick; 0 = none.
pub fn pick_unique<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    d: usize,
    used: &[u8],
) -> u8 {
    let class = class_of(h, unit);
    let mut cands = Vec::new();
    for (i, r) in cx.tables.monumod.iter().enumerate() {
        let w = i32::from([r.upick, r.upick_n, r.upick_h][d.min(2)]);
        if w > 0 && r.champion == 0 && !used.contains(&(i as u8)) && eligible(cx, h, class, i) {
            cands.push((i as u8, w));
        }
    }
    weighted(h, unit, &cands)
}

/// Choosing umods.
pub fn choose_umods<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    champion_allowed: bool,
) {
    let diff = h.info().difficulty;
    if diff >= 3 {
        return;
    }
    let Some(existing) = h.monsters().get(unit).map(|m| m.umod_count()) else {
        return;
    };
    if existing >= 8 {
        return;
    }
    let d = usize::from(diff);
    // Step 1.
    if champion_allowed {
        let chance = cx.tables.monumod.first().map_or(0, |r| r.constants as i32);
        if (seed(h, unit).roll(100) as i32) < chance {
            flags_or(h, unit, type_flag::CHAMPION);
            let c = pick_champion(cx, h, unit, d);
            if c != 0 {
                h.monsters().entry(unit).push_umod(c);
            }
            return;
        }
    }
    // Step 2.
    let mut count = seed(h, unit).roll(1) as usize + 1 + d;
    if count + existing >= MAX_UMODS {
        count = MAX_UMODS - existing;
    }
    // Step 3.
    let mut used = data(h, unit).umod_list().to_vec();
    for _ in 0..count {
        let u = pick_unique(cx, h, unit, d, &used);
        if u == 0 {
            break;
        }
        h.monsters().entry(unit).push_umod(u);
        used.push(u);
    }
}

// ---------

/// Umod transfer: the boss's `xfer` umods are
/// appended to the minion's; every boss umod visited counts against the
/// minion's free slots (edge case 7).
pub fn xfer_umods<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, boss: UnitId, minion: UnitId) {
    let b = data(h, boss);
    let own = data(h, minion).umod_count();
    for &u in b.umods.iter().take(MAX_UMODS - own) {
        if u == 0 {
            break;
        }
        if cx
            .tables
            .monumod
            .get(usize::from(u))
            .is_some_and(|r| r.xfer == 1)
        {
            h.monsters().entry(minion).push_umod(u);
        }
    }
}

/// Boss minions and umod init.
pub fn boss_minions_and_init<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    min: i32,
    max: i32,
    coord_list: Option<u32>,
    spawn_minions: bool,
) {
    // Step 1 (spawn rules; the count draw
    // is on the boss's unit seed, held here).
    if spawn_minions && !data(h, unit).has_flag(type_flag::CHAMPION) {
        let own = class_of(h, unit);
        let class = cx
            .monstats(own)
            .map(|m| s16(m.minion1))
            .and_then(|c| u32::try_from(c).ok())
            .filter(|&c| (c as usize) < cx.tables.monstats.len())
            .unwrap_or(own);
        // count = min + roll(max − min + 1) (the plain roll helper).
        let count =
            min.wrapping_add(seed(h, unit).roll(max.wrapping_sub(min).wrapping_add(1)) as i32);
        for _ in 0..count.max(0) {
            if let Some(m) = h.spawn_boss_minion(unit, class, coord_list) {
                xfer_umods(cx, h, unit, m);
                h.link_minion(unit, m);
                flags_or(h, m, type_flag::MINION);
            }
        }
    }
    // Step 2.
    let mut list = FIXED_UMODS.to_vec();
    list.extend_from_slice(data(h, unit).umod_list());
    let minions = h.minions(unit);
    for u in list {
        run_umod_init(cx, h, unit, u, true);
        for &m in &minions {
            run_umod_init(cx, h, m, u, false);
        }
    }
}

// ---------

/// Runs umod `umod`'s init function on `unit` ([`UMODS`] `init_fn`, gated
/// on `unique` by its `unique_gate`). A mod game's init for the umod runs
/// instead when it has one.
pub fn run_umod_init<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    umod: u8,
    unique: bool,
) {
    if !h.rules().is_original() {
        let rules = h.rules().clone();
        if let Some(p) = rules.monsters() {
            if p.umod_init(cx, h.as_dyn_init(), unit, umod, unique) {
                return;
            }
        }
    }
    let Some(r) = UMODS.get(usize::from(umod)) else {
        return;
    };
    match r.gate {
        Gate::None => return,
        Gate::Unique if !unique => return,
        _ => {}
    }
    let d = h.info().d();
    match umod {
        1 => {
            // Low 16 bits of lo'.
            let v = seed(h, unit).step() as u16;
            h.monsters().entry(unit).name_seed = v;
        }
        2 => hp_multiply(cx, h, unit, unique, d),
        4 => {
            add(h, unit, stat::LEVEL, 3);
            let e = h.stat(unit, stat::EXPERIENCE);
            h.set_stat(unit, stat::EXPERIENCE, e.wrapping_mul(5));
        }
        5 => strong(cx, h, unit, unique, d),
        6 => fast(cx, h, unit),
        8 | 27 | 28 => resist(h, unit, umod),
        9 | 17 | 18 | 23 | 25 => elemental(cx, h, unit, umod, unique, d),
        16 => champion_fn(cx, h, unit, 16, d),
        26 => teleport(cx, h, unit, unique),
        30 => aura(cx, h, unit),
        36 => ghostly(cx, h, unit, d),
        37 => {
            h.set_stat(unit, stat::ITEM_ARMOR_PERCENT, -70);
            champion_fn(cx, h, unit, 37, d);
        }
        38 => {
            flags_or(h, unit, type_flag::POSSESSED);
            raise_hp(h, unit, 100);
            champion_fn(cx, h, unit, 38, d);
        }
        39 => {
            raise_hp(h, unit, -75);
            let b = cx.champion_bonus(d);
            damage_bonus(cx, h, unit, 300 * b / 100);
            add(h, unit, stat::ITEM_TOHIT_PERCENT, 300 * b / 100);
        }
        41 => schedule(h, unit, 75),
        _ => {}
    }
}

/// maxhp and hitpoints += pct(maxhp, p, 100) (umods 38, 39). Both are
/// read before either is written: the maxhp write rescales hitpoints
/// through the server callback, and the
/// hitpoints write then sets the value read before it plus the delta.
/// Measured: a Cave Level 1 berserker (maxhp 12288) ends at maxhp =
/// hitpoints = 3072; a re-read after the maxhp write would give −6144.
fn raise_hp<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, p: i32) {
    let max = h.stat(unit, stat::MAXHP);
    let hp = h.stat(unit, stat::HITPOINTS);
    let delta = pct(max, p, 100);
    if delta == 0 {
        return;
    }
    h.set_stat(unit, stat::MAXHP, max.wrapping_add(delta));
    h.set_stat(unit, stat::HITPOINTS, hp.wrapping_add(delta));
}

/// damagepercent += v, halved (signed / 2) for `BaseId` 118.
fn damage_bonus<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, v: i32) {
    let v = if base_id(cx, h, unit) == Some(WILLOWISP) {
        v / 2
    } else {
        v
    };
    add(h, unit, stat::DAMAGEPERCENT, v);
}

/// Umod 2.
fn hp_multiply<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    unique: bool,
    d: usize,
) {
    let k = if !unique {
        cx.k(d + 1)
    } else if data(h, unit).has_flag(type_flag::CHAMPION) {
        cx.k(d + 4)
    } else {
        cx.k(d + 7)
    };
    // The body reads the full maximum life, the
    // equipment's life included (Ancient Barbarian 2's circlet, gen-su-45)
    let hp = h.max_life(unit);
    let v = hp.wrapping_add(pct(hp, k, 100));
    h.set_stat(unit, stat::MAXHP, v);
    h.set_stat(unit, stat::HITPOINTS, v);
    if unique {
        h.set_stat(unit, stat::HPREGEN, 0);
    }
}

/// The champion function (umod 16; also 36, 37, 38).
fn champion_fn<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, umod: u8, d: usize) {
    add(h, unit, stat::LEVEL, -1);
    let e = h.stat(unit, stat::EXPERIENCE);
    h.set_stat(unit, stat::EXPERIENCE, e - 2 * (e / 5));
    let b = cx.champion_bonus(d);
    damage_bonus(cx, h, unit, cx.k(11) * b / 100);
    add(h, unit, stat::ITEM_TOHIT_PERCENT, cx.k(10) * b / 100);
    let class = class_of(h, unit);
    let v = cx.monstats(class).map_or(0, |m| s16(m.velocity));
    if v > 0 && umod != 36 {
        if umod == 37 {
            add(
                h,
                unit,
                stat::VELOCITYPERCENT,
                (2048 / v - 128).clamp(10, 100),
            );
        } else {
            add(h, unit, stat::VELOCITYPERCENT, 20);
        }
    }
}

/// Umod 5 strong.
fn strong<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, unique: bool, d: usize) {
    let b = cx.champion_bonus(d);
    let (kd, kt) = if unique { (15, 13) } else { (14, 12) };
    damage_bonus(cx, h, unit, cx.k(kd) * b / 100);
    add(h, unit, stat::ITEM_TOHIT_PERCENT, cx.k(kt) * b / 100);
}

/// Umod 6 fast.
fn fast<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let class = class_of(h, unit);
    let v = cx.monstats(class).map_or(0, |m| s16(m.velocity));
    if v > 0 {
        add(
            h,
            unit,
            stat::VELOCITYPERCENT,
            (2048 / v - 128).clamp(10, 100),
        );
    }
}

/// The resistance function; unique only.
fn resist<H: InitHost + ?Sized>(h: &mut H, unit: UnitId, umod: u8) {
    if umod == 28 {
        let ac = h.stat(unit, stat::ARMORCLASS);
        h.set_stat(unit, stat::ARMORCLASS, ac.wrapping_mul(2));
    }
    let mut immune = [
        stat::FIRERESIST,
        stat::LIGHTRESIST,
        stat::COLDRESIST,
        stat::POISONRESIST,
        stat::DAMAGERESIST,
        stat::MAGICRESIST,
    ]
    .iter()
    .filter(|&&s| h.stat(unit, s) >= 100)
    .count();
    if immune >= 2 {
        return;
    }
    match umod {
        8 | 27 => {
            let (plus, below) = if umod == 8 { (40, 100) } else { (20, 75) };
            for s in [stat::COLDRESIST, stat::FIRERESIST, stat::LIGHTRESIST] {
                if immune >= 2 {
                    break;
                }
                let v = h.stat(unit, s);
                if v < below {
                    h.set_stat(unit, s, v + plus);
                    if v + plus >= 100 {
                        immune += 1;
                    }
                }
            }
        }
        9 => add(h, unit, stat::FIRERESIST, 75),
        17 => add(h, unit, stat::LIGHTRESIST, 75),
        18 => add(h, unit, stat::COLDRESIST, 75),
        23 => add(h, unit, stat::POISONRESIST, 75),
        25 => add(h, unit, stat::MAGICRESIST, 20),
        28 => add(h, unit, stat::DAMAGERESIST, 50),
        _ => {}
    }
}

/// Umods 9, 17, 18, 23, 25: the shared elemental body.
fn elemental<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    umod: u8,
    unique: bool,
    d: usize,
) {
    // Step 1: a null unit or a unit that is not a monster returns before
    // anything (no resistance tail).
    if !is_monster(h, unit) {
        return;
    }
    // Step 3: d' = min(d, 2); o = the L-flag.
    let dp = d.min(2);
    let o = h.info().l_flag();
    // Step 4: the monlvl row, clamped to 1..rows − 1 (never rejected).
    let level = h.stat(unit, stat::LEVEL);
    let rows = cx.tables.monlvl.len() as i32;
    let r = if level.max(1) >= rows - 1 {
        rows - 1
    } else if level <= 1 {
        1
    } else {
        level
    };
    // Steps 4–5: an empty table returns before the stats and the tail.
    let Some(row) = usize::try_from(r)
        .ok()
        .and_then(|i| cx.tables.monlvl.get(i))
    else {
        return;
    };
    // Step 6: `DM` / `L-DM` of difficulty d'.
    let v = if o {
        [row.l_dm, row.l_dm_n, row.l_dm_h]
    } else {
        [row.dm, row.dm_n, row.dm_h]
    }[dp] as i32;
    // Step 7.
    let (kmin, kmax) = if unique {
        (cx.k(dp + 28), cx.k(dp + 31))
    } else {
        (cx.k(dp + 16), cx.k(dp + 19))
    };
    let (smin, smax, scale) = match umod {
        9 => (stat::FIREMINDAM, stat::FIREMAXDAM, 1),
        17 => (stat::LIGHTMINDAM, stat::LIGHTMAXDAM, 1),
        18 => (stat::COLDMINDAM, stat::COLDMAXDAM, 1),
        23 => (stat::POISONMINDAM, stat::POISONMAXDAM, 1),
        _ => (stat::MANADRAINMINDAM, stat::MANADRAINMAXDAM, 256),
    };
    // Steps 8–9: divide, then scale (shift left 8 after the division).
    add(
        h,
        unit,
        smin,
        (kmin.wrapping_mul(v) / 100).wrapping_mul(scale),
    );
    add(
        h,
        unit,
        smax,
        (kmax.wrapping_mul(v) / 100).wrapping_mul(scale),
    );
    // Step 10: the length stat of the clamped row r, not the level.
    match umod {
        18 => add(h, unit, stat::COLDLENGTH, 5 * r + 100),
        23 => add(h, unit, stat::POISONLENGTH, 2 * (5 * r + 150)),
        _ => {}
    }
    // Step 11: the resistance tail does nothing for a minion.
    if unique {
        resist(h, unit, umod);
    }
}

/// Umod 36 ghostly.
fn ghostly<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, d: usize) {
    flags_or(h, unit, type_flag::GHOSTLY);
    h.set_stat(unit, stat::DAMAGERESIST, 80);
    champion_fn(cx, h, unit, 36, d);
    let level = h.stat(unit, stat::LEVEL);
    let dm = monlvl_dm(cx.tables.monlvl, h.info().l_flag(), d, level);
    add(
        h,
        unit,
        stat::COLDMINDAM,
        dm.wrapping_mul(cx.k(d + 22)) / 100,
    );
    add(
        h,
        unit,
        stat::COLDMAXDAM,
        dm.wrapping_mul(cx.k(d + 25)) / 100,
    );
    add(h, unit, stat::COLDLENGTH, 150);
}

/// Umod 26 teleport:
/// bosses only; skill 184 at level 1, its mode 4, AI flag 0x20.
fn teleport<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId, unique: bool) {
    if !unique || !is_monster(h, unit) {
        return;
    }
    if let Some(sk) = cx.tables.ids.monteleport {
        h.give_skill(unit, sk, 1, Some(4));
    }
    h.set_ai_flag(unit, crate::monsters::ai::flag::MAY_TELEPORT);
}

/// The aura index and skill level of umod 30 for a unit with
/// `level`, name seed `name_seed`, class and superunique row.
pub fn aura_choice(level: i32, name_seed: u16, class: u32, superunique: Option<u16>) -> (u16, i32) {
    if class == 704 {
        return (CONVICTION, 20);
    }
    let lvl = level.max(1);
    let n = AURAS.iter().filter(|a| a.0 <= lvl).count().max(1);
    // A temporary seed; the unit seed is not touched.
    let mut tmp = Seed::init_low(u32::from(name_seed));
    let mut i = tmp.roll(n as i32) as usize;
    if superunique == Some(37) {
        i = 5;
    }
    let (_, off, mul, div, skill) = AURAS[i];
    (skill, ((lvl + off) * mul / div).clamp(1, 99))
}

/// Umod 30 aura enchanted.
fn aura<H: InitHost + ?Sized>(_cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let m = data(h, unit);
    let su = m.has_flag(type_flag::SUPERUNIQUE).then_some(m.boss_hc_idx);
    let class = class_of(h, unit);
    let level = h.stat(unit, stat::LEVEL);
    let (skill, lv) = aura_choice(level, m.name_seed, class, su);
    h.give_aura(unit, skill, lv);
}

// --------------

/// Superunique steps owned here, after
/// population spawned the unit and set type flag 2. `min` /
/// `max` are the minion group (`MinGrp`/`MaxGrp` + difficulty).
pub fn superunique_init<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    row: u16,
    min: i32,
    max: i32,
) {
    let Some(aura) = superunique_mods(cx, h, unit, row) else {
        return;
    };
    // Step 3.
    boss_minions_and_init(cx, h, unit, min, max, None, true);
    superunique_finish(cx, h, unit, row, aura);
    super::create::assign_umod(cx, h, unit, 22, true);
}

/// The superunique row and its umods. Returns whether 30
/// (aura) was one of `Mod1`..`Mod3`; `None` for an unknown row (nothing
/// done). Population's runs the minions itself.
pub fn superunique_mods<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    row: u16,
) -> Option<bool> {
    let su = cx.tables.superuniques.get(usize::from(row))?;
    // Step 1.
    h.monsters().entry(unit).boss_hc_idx = row;
    // Step 2. The difficulty picks are read as part of the "fewer than
    // 5" branch.
    let mut aura = false;
    if data(h, unit).umod_count() < 5 {
        for m in [su.mod1, su.mod2, su.mod3] {
            if m == 0 {
                break;
            }
            if m == 24 {
                continue;
            }
            aura |= m == 30;
            h.monsters().entry(unit).push_umod(m as u8);
        }
        let d = h.info().difficulty;
        let mut used = data(h, unit).umod_list().to_vec();
        for _ in 0..d {
            let u = pick_unique(cx, h, unit, usize::from(d), &used);
            if u == 0 {
                break;
            }
            h.monsters().entry(unit).push_umod(u);
            used.push(u);
        }
    }
    Some(aura)
}

/// Without the closing umod 22: the aura re-run and the
/// quest records by `hcIdx`.
pub fn superunique_finish<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    row: u16,
    aura: bool,
) {
    superunique_finish_with(cx, h, unit, row, aura, true);
}

/// [`superunique_finish`] where the caller says whether the per-`hcIdx`
/// extra spawns (and Radament's `roll(5)`) are still due. Population's
/// `preset::superunique` runs them itself,
/// so its wiring passes `false`: they run once.
pub fn superunique_finish_with<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    row: u16,
    aura: bool,
    extra_spawns: bool,
) {
    let Some(su) = cx.tables.superuniques.get(usize::from(row)) else {
        return;
    };
    // Step 4.
    if aura {
        run_umod_init(cx, h, unit, 30, true);
    }
    // Step 5.
    superunique_quest(h, unit, su.hcidx, extra_spawns);
}

/// The per-`hcIdx` cases of the superunique finish, before the closing
/// umod 22.
fn superunique_quest<H: InitHost + ?Sized>(
    h: &mut H,
    unit: UnitId,
    hc_idx: u32,
    extra_spawns: bool,
) {
    match hc_idx {
        6 => {
            h.set_state(unit, 118);
            h.quest_chain(unit, 5);
            h.ai_install(unit, 13);
        }
        10 if !extra_spawns => {}
        10 => {
            // U `roll(5)` + 2 class-4 spawns, then one of each class.
            let n = seed(h, unit).roll(5) as i32 + 2;
            for _ in 0..n {
                h.spawn_near_unit(unit, 4, 1, 4, 0x40);
            }
            for c in [276, 382, 385, 389] {
                h.spawn_near_unit(unit, c, 1, 4, 0x40);
            }
        }
        26 | 27 | 29 => {
            h.quest_chain(unit, 19);
            h.quest_preset_boss(unit);
        }
        36..=38 => h.quest_chain(unit, 23),
        39 => h.quest_chain(unit, 4),
        42 => {
            if extra_spawns {
                h.spawn_group(unit, 453, 20, 20, 0);
            }
            h.quest_chain(unit, 31);
            h.quest_preset_boss(unit);
            h.set_state(unit, 118);
        }
        43..=45 => {
            h.quest_chain(unit, 35);
            h.quest_preset_boss(unit);
        }
        60 => {
            if extra_spawns {
                h.owner_data_self(unit);
                let c = h.class_for_level(unit, 453);
                h.spawn_group(unit, c, 10, 20, 0x40);
            }
            h.quest_chain(unit, 34);
        }
        62 if extra_spawns => h.spawn_group(unit, 381, 20, 10, 0x40),
        _ => {}
    }
}

// ---------

/// Make unique, the warping shrine's
/// effect on the monster it picked: boss flag, unique mark, umods with
/// the champion test, umod init without minions, then unit flag 0x800
/// and monster data +0x5C bit 0.
pub fn make_unique<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    let monster = h
        .units()
        .get(unit)
        .is_some_and(|r| r.ty == crate::units::UnitType::Monster);
    if !monster || h.monsters().get(unit).is_none() {
        return;
    }
    flags_or(h, unit, type_flag::BOSS);
    mark_unique(h, unit);
    choose_umods(cx, h, unit, true);
    boss_minions_and_init(cx, h, unit, 0, 0, None, false);
    if let Some(r) = h.units().get_mut(unit) {
        r.flags |= 0x800;
    }
    h.monsters().entry(unit).data_flag1 = true;
}

/// What the eligibility test reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarpCandidate {
    /// M is P itself.
    pub is_operator: bool,
    /// M is a monster (type 1).
    pub monster: bool,
    pub relation: i32,
    pub alignment: i32,
    /// ≠ 0.
    pub test_63ea40: bool,
    /// M's mode.
    pub mode: u32,
    /// The class has mode 2.
    pub has_walk: bool,
    /// monstats2 byte +0x0B (`None` without a record).
    pub m2_byte_0b: Option<u8>,
    /// Monster data dword 0 ≠ 0.
    pub has_data: bool,
    /// Boss.
    pub boss: bool,
    /// Prime evil.
    pub prime_evil: bool,
    /// Type flags (monster data +0x16).
    pub type_flags: u16,
}

/// Eligibility: all ten tests in order.
pub fn warp_eligible(c: &WarpCandidate) -> bool {
    !c.is_operator
        && c.monster
        && c.relation != 1
        && c.alignment == 0
        && !c.test_63ea40
        && matches!(c.mode, 1 | 2)
        && c.has_walk
        && c.m2_byte_0b.is_some_and(|b| b != 0)
        && c.has_data
        && !c.boss
        && !c.prime_evil
        && c.type_flags & 0x1F == 0
}

/// The nearest eligible monster
/// over `(unit, distance, eligible)` in room-list then unit-list order:
/// limit 0 means 0x10000; the best starts at 0xFFFF; the first of equal
/// distances wins.
pub fn nearest_eligible<U: Copy>(
    candidates: impl IntoIterator<Item = (U, i32, bool)>,
    limit: i32,
) -> Option<U> {
    let limit = if limit == 0 { 0x10000 } else { limit };
    let mut best = 0xFFFF;
    let mut pick = None;
    for (u, d, ok) in candidates {
        if d < limit && d < best && ok {
            pick = Some(u);
            best = d;
        }
    }
    pick
}

/// A saved boss or minion (for the restore).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub umods: [u8; MAX_UMODS],
    pub name_seed: u16,
    pub champion: bool,
    /// Superunique row, when the boss was one.
    pub superunique: Option<u16>,
}

/// Boss restore. `req` holds class and position;
/// `guid` the saved GUID.
pub fn restore_boss<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    req: &CreateRequest,
    guid: u32,
    saved: &Saved,
) -> Option<UnitId> {
    let unit = h.boss_spawn(req, Some(guid), false)?;
    mark_boss(h, unit);
    if saved.champion {
        flags_or(h, unit, type_flag::CHAMPION);
    }
    h.monsters().entry(unit).umods = saved.umods;
    boss_minions_and_init(cx, h, unit, 0, 0, None, false);
    h.monsters().entry(unit).name_seed = saved.name_seed;
    if data(h, unit).has_umod(30) {
        run_umod_init(cx, h, unit, 30, true);
    }
    if let Some(row) = saved.superunique {
        flags_or(h, unit, type_flag::SUPERUNIQUE);
        h.monsters().entry(unit).boss_hc_idx = row;
        if let Some(su) = cx.tables.superuniques.get(usize::from(row)) {
            superunique_quest(h, unit, su.hcidx, true);
        }
    }
    Some(unit)
}

/// Minion restore. `req` carries the saved GUID with
/// flags 0x62.
pub fn restore_minion<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    req: &CreateRequest,
    saved: &Saved,
) -> Option<UnitId> {
    let unit = h.spawn_with_guid(req)?;
    h.monsters().entry(unit).umods = saved.umods;
    flags_or(h, unit, type_flag::MINION);
    let mut list = FIXED_UMODS.to_vec();
    list.extend_from_slice(data(h, unit).umod_list());
    for u in list {
        run_umod_init(cx, h, unit, u, false);
    }
    Some(unit)
}

// ---------

/// The umod dispatcher: runs mode `mode`'s callback of each umod in the
/// unit's list, in list order. Mode 5 passes `arg` (the missile) to the
/// callbacks instead of the unit. A mod game's callback for a umod runs
/// instead of D2RUST's when it has one.
pub fn dispatch<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    arg: Option<UnitId>,
    mode: u8,
) {
    let m = data(h, unit);
    if m.umods[0] == 0 {
        return;
    }
    let unique = m.has_flag(type_flag::UNIQUE);
    let target = if mode == 5 { arg.unwrap_or(unit) } else { unit };
    let rules = (!h.rules().is_original()).then(|| h.rules().clone());
    let provider = rules.as_ref().and_then(|r| r.monsters());
    // All 9 slots, zero slots included (row 0 has no callbacks).
    for u in m.umods {
        if u != 0
            && provider
                .is_some_and(|p| p.umod_callback(cx, h.as_dyn_init(), target, u, unique, mode))
        {
            continue;
        }
        let Some(addr) = UMODS
            .get(usize::from(u))
            .and_then(|r| r.callbacks.get(usize::from(mode)))
            .copied()
            .filter(|&a| a != 0)
        else {
            continue;
        };
        run_callback(cx, h, target, u, unique, mode, addr);
    }
}

fn run_callback<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    umod: u8,
    unique: bool,
    mode: u8,
    addr: u32,
) {
    if !super::callbacks::run(cx, h, unit, umod, unique, addr) {
        h.monsters().unhandled.push(Unhandled::Callback {
            addr,
            unit,
            umod,
            mode,
        });
    }
}

/// Monster timer event type 7 (the dispatcher's mode 2).
pub fn handle_event7<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, unit: UnitId) {
    dispatch(cx, h, unit, None, 2);
}
