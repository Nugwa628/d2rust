//! The umod callback bodies of the callback table
//! (`umods.tsv` columns `cb_mode0`…`cb_mode5`) and the
//! helpers they share that no other module owns: area damage, the
//! cross burst, the elemental fill and the think restart. The unit find
//! is [`super::find`]. Every call into
//! another system goes through [`InitHost`]; randomness is the unit
//! seed in a fixed order per callback.

use crate::combat::DamageRecord;
use crate::rng::Seed;
use crate::skills::use_::bodies::{init_cb, ring_offset, MissileRequest};
use crate::stats::lists::ListId;
use crate::units::{UnitId, UnitType};

use super::calc::{pct, stats_by_level};
use super::find::FindQuery;
use super::{mode, s16, type_flag, Ctx, InitHost, Unhandled, EVENT_UMOD};

/// The callback addresses (`umods.tsv`), each with its section.
pub mod addr {
    /// Umod 7, mode 3.
    pub const CURSE: u32 = 0x005A_2530;
    /// Umod 9, mode 1.
    pub const FIRE_MODE: u32 = 0x005A_25F0;
    /// Umod 9, mode 2.
    pub const FIRE_EXPLODE: u32 = 0x005A_2620;
    /// Umod 31, mode 2.
    pub const GOBOOM: u32 = 0x005A_2840;
    /// Umod 42, mode 2.
    pub const LIGHTNING_DEATH: u32 = 0x005A_2910;
    /// Umod 17, mode 2.
    pub const LIGHTNING_BURST: u32 = 0x005A_29A0;
    /// Umod 17, mode 4.
    pub const LIGHTNING_HIT: u32 = 0x005A_2BA0;
    /// Umod 18, mode 2.
    pub const COLD_NOVA: u32 = 0x005A_2BD0;
    /// Umod 10, mode 2.
    pub const POISON_DEAD: u32 = 0x005A_2C20;
    /// Umod 15, mode 1.
    pub const PARTY_DEAD: u32 = 0x005A_2D10;
    /// Umod 19, mode 0.
    pub const HIREABLE_MODE: u32 = 0x005A_2D80;
    /// Umod 19, mode 5.
    pub const HIREABLE_MISSILE: u32 = 0x005A_2E00;
    /// Umod 20, mode 1.
    pub const SCARAB_MODE: u32 = 0x005A_2EB0;
    /// Umod 20, mode 2.
    pub const SCARAB_BURST: u32 = 0x005A_2EE0;
    /// Umod 27, mode 0.
    pub const SPECTRAL_MODE: u32 = 0x005A_3040;
    /// Umod 27, mode 5.
    pub const SPECTRAL_MISSILE: u32 = 0x005A_30B0;
    /// Umod 24, mode 3.
    pub const THIEF: u32 = 0x005A_30E0;
    /// Umod 22, mode 1.
    pub const QUEST_COMPLETE: u32 = 0x005A_3250;
    /// Umod 23, mode 0.
    pub const POISON_HIT: u32 = 0x005A_3490;
    /// Umod 29, mode 5.
    pub const MULTISHOT: u32 = 0x005A_3610;
    /// Umod 17, mode 1.
    pub const LIGHTNING_MODE: u32 = 0x005A_37D0;
    /// Umods 10, 18, 31, 32, 42, mode 1.
    pub const DEATH_MODE: u32 = 0x005A_3800;
    /// Umod 34, mode 1.
    pub const AI_AFTER_DEATH: u32 = 0x005A_3840;
    /// Umod 34, mode 2.
    pub const REVIVE: u32 = 0x005A_3910;
    /// Umod 35, mode 1.
    pub const SHATTER: u32 = 0x005A_3A80;
    /// Umod 21, mode 2.
    pub const KILL_SELF: u32 = 0x005A_3AA0;
    /// Umod 14, mode 0.
    pub const SPC_DAMAGE: u32 = 0x005A_3B50;
    /// Umod 32, mode 2.
    pub const FIRESPIKE: u32 = 0x005A_3D20;
    /// Umod 33, mode 1.
    pub const SUICIDE_MODE: u32 = 0x005A_3E70;
    /// Umod 33, mode 2.
    pub const SUICIDE_EXPLODE: u32 = 0x005A_3EF0;
    /// Umod 40, mode 1.
    pub const WORMS: u32 = 0x005A_4200;
    /// Umod 41, mode 2.
    pub const ALWAYS_RUN_AI: u32 = 0x005A_4230;

    /// Every address with a body here.
    pub const ALL: [u32; 32] = [
        CURSE,
        FIRE_MODE,
        FIRE_EXPLODE,
        GOBOOM,
        LIGHTNING_DEATH,
        LIGHTNING_BURST,
        LIGHTNING_HIT,
        COLD_NOVA,
        POISON_DEAD,
        PARTY_DEAD,
        HIREABLE_MODE,
        HIREABLE_MISSILE,
        SCARAB_MODE,
        SCARAB_BURST,
        SPECTRAL_MODE,
        SPECTRAL_MISSILE,
        THIEF,
        QUEST_COMPLETE,
        POISON_HIT,
        MULTISHOT,
        LIGHTNING_MODE,
        DEATH_MODE,
        AI_AFTER_DEATH,
        REVIVE,
        SHATTER,
        KILL_SELF,
        SPC_DAMAGE,
        FIRESPIKE,
        SUICIDE_MODE,
        SUICIDE_EXPLODE,
        WORMS,
        ALWAYS_RUN_AI,
    ];
}

/// Stat ids the callbacks read or write.
pub mod stat {
    pub const LEVEL: u16 = 12;
    pub const TOHIT: u16 = 19;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const SECONDARY_MINDAMAGE: u16 = 23;
    pub const SECONDARY_MAXDAMAGE: u16 = 24;
    pub const FIREMINDAM: u16 = 48;
    pub const FIREMAXDAM: u16 = 49;
    pub const LIGHTMINDAM: u16 = 50;
    pub const LIGHTMAXDAM: u16 = 51;
    pub const MAGICMINDAM: u16 = 52;
    pub const MAGICMAXDAM: u16 = 53;
    pub const COLDMINDAM: u16 = 54;
    pub const COLDMAXDAM: u16 = 55;
    pub const COLDLENGTH: u16 = 56;
    pub const POISONMINDAM: u16 = 57;
    pub const POISONMAXDAM: u16 = 58;
    pub const POISONLENGTH: u16 = 59;
}

/// Missile classes (`missiles.txt` `Id`).
pub mod missile {
    pub const NOVA: i32 = 90;
    pub const MONSTER_CORPSE_EXPLODE: i32 = 117;
    pub const FROST_NOVA: i32 = 119;
    pub const CORPSE_POISON_CLOUD: i32 = 155;
    pub const COLD_UNIQUE: i32 = 194;
    pub const LIGHT_UNIQUE: i32 = 195;
    pub const BUG_LIGHTNING: i32 = 225;
    pub const QUEEN_POISON_CLOUD: i32 = 321;
    pub const SUICIDE_CORPSE_EXPLODE: i32 = 426;
    pub const SUICIDE_FIRE_EXPLODE: i32 = 427;
    pub const SUICIDE_ICE_EXPLODE: i32 = 428;
    pub const MEPHISTO_DEATH_CONTROL: i32 = 299;
    pub const RADAMENT_DEATH: i32 = 347;
    pub const RADAMENT_HAND_OF_GOD: i32 = 348;
    /// `mummy1`…`mummy4`.
    pub const MUMMIES: std::ops::RangeInclusive<i32> = 63..=66;
}

/// missiles row flags (+0x04).
pub mod missile_flag {
    /// `Explosion`, bit 1.
    pub const EXPLOSION: u32 = 1 << 1;
    /// `NoMultiShot`, bit 12 (mask).
    pub const NO_MULTI_SHOT: u32 = 1 << 12;
    /// `NoUniqueMod`, bit 13 (mask).
    pub const NO_UNIQUE_MOD: u32 = 1 << 13;
}

/// Monster data +0x16 bits the callbacks use.
pub mod data_flag {
    /// Multishot copy in progress.
    pub const MULTISHOT: u16 = 0x80;
    /// Lightning burst fired.
    pub const BURST: u16 = 0x100;
}

/// Finder flags of the callers.
pub mod finder {
    /// Players and monsters.
    pub const PLAYERS_MONSTERS: u32 = 3;
    /// Players only: live, unit flags 4 and 8, not in town.
    pub const PLAYERS_LIVE: u32 = 0x581;
    /// [`PLAYERS_LIVE`] plus monsters.
    pub const PLAYERS_MONSTERS_LIVE: u32 = 0x583;
}

/// Skill ids.
pub mod skill {
    pub const AMPLIFY_DAMAGE: u16 = 66;
    pub const SHADOW_WARRIOR: u16 = 268;
    pub const SELF_RESURRECT: u16 = 293;
}

/// States.
pub mod state {
    pub const UNINTERRUPTABLE: u16 = 54;
    pub const CORPSE_NODRAW: u16 = 104;
    pub const SHATTER: u16 = 107;
    /// Flag group 33 `udead`.
    pub const GROUP_UDEAD: u8 = 33;
}

/// Monster classes.
pub mod class {
    pub const ROGUEHIRE: u32 = 271;
    pub const TRAP_FIREBOLT: u32 = 326;
    pub const TRAP_HORZMISSILE: u32 = 327;
    pub const TRAP_VERTMISSILE: u32 = 328;
    pub const TRAP_POISONCLOUD: u32 = 329;
    pub const TRAP_LIGHTNING: u32 = 330;
    pub const TRAP_NOVA: u32 = 369;
    pub const PAINWORM1: u32 = 551;
    /// Base classes whose think reschedules.
    pub const THINK_BASES: [i32; 4] = [110, 118, 136, 247];
}

/// The quest death calls of the quest monsters by class (two jump
/// tables in the game; class 267 tested
/// directly).
pub fn quest_death_call(class: u32) -> Option<u32> {
    Some(match class {
        156 => 0x005D_FE00,
        229 => 0x005D_FE20,
        242 => 0x005D_FDB0,
        256 => 0x005E_0020,
        267 => 0x005D_FD90,
        479 => 0x005E_0040,
        540..=542 => 0x005E_0060,
        704 | 705 | 709 => 0x005E_0070,
        _ => return None,
    })
}

/// The quest death bodies (ECX game, EDX unit).
pub fn quest_death<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId, call: u32) {
    match call {
        0x005D_FD90 => purge(cx, h, u, 35, 25, 125, true),
        0x005D_FE00 => purge(cx, h, u, 35, 1, 51, false),
        0x005D_FE20 => {
            // `m` = max(missile 348 `Range` − 100, 100).
            // Fewer than 349 missile rows: there is no missile record to read
            // (never with the game's
            // data, which has the row); nothing is done here.
            let Some(range) = h.missile_range(missile::RADAMENT_HAND_OF_GOD) else {
                return;
            };
            let m = range.wrapping_sub(100).max(100);
            purge(cx, h, u, 35, 40, m, false);
            // The missile call with 347 at Radament's own
            // position (distance 0).
            let (x, y) = h.position(u);
            let at = if (x, y) == (0, 0) {
                h.target_position(u)
            } else {
                Some((x, y))
            };
            if let Some((x, y)) = at.filter(|&p| p != (0, 0)) {
                let _ = h.create_missile(MissileRequest {
                    flags: 1,
                    x,
                    y,
                    skill: 0,
                    level: 1,
                    ..MissileRequest::new(u, missile::RADAMENT_DEATH)
                });
            }
        }
        0x005E_0020 => purge(cx, h, u, 105, 1, 2, false),
        0x005E_0040 => purge(cx, h, u, 35, 25, 125, false),
        0x005E_0060 => {}
        0x005D_FDB0 => {
            let _ = h.create_missile(MissileRequest {
                flags: 0x8000,
                origin: Some(u),
                level: 1,
                range: 100,
                ..MissileRequest::new(u, missile::MEPHISTO_DEATH_CONTROL)
            });
        }
        0x005E_0070 => {
            let slot = match class_of(h, u) {
                704 => 0,
                705 => 1,
                709 => 2,
                c => panic!("uber death of class {c} (fatal 0xE6)"),
            };
            if h.set_uber_death(slot) {
                h.quest_drop(u, *b"cm2 ", 7, true);
                for _ in 0..h.game_8c() {
                    h.quest_drop(u, *b"std ", 2, false);
                }
            }
        }
        _ => {}
    }
}

/// The minion purge (radius r, min, max, undead) around the
/// boss `b`.
#[allow(clippy::too_many_arguments)]
pub fn purge<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    b: UnitId,
    r: i32,
    min: i32,
    max: i32,
    undead: bool,
) {
    assert!(max != 0, "minion purge: max 0 (fatal 0x20)");
    assert!(max > min, "minion purge: max ≤ min (fatal 0x21)");
    let (x, y) = h.position(b);
    let found = h.find_units(
        b,
        FindQuery {
            flags: finder::PLAYERS_MONSTERS_LIVE,
            exclude: Some(b),
            x,
            y,
            r,
            ..FindQuery::default()
        },
    );
    for m in found {
        if type_of(h, m) != Some(UnitType::Monster)
            || h.alignment(m) != 0
            || (undead && !is_undead_class(cx, class_of(h, m)))
            || class_of(h, m) == 333
        {
            continue;
        }
        let roll = seed(h, b).map_or(0, |s| s.roll(max.wrapping_sub(min)));
        event7(h, m, min.wrapping_add(roll as i32));
        super::create::assign_umod(cx, h, m, 21, false);
    }
}

/// On a class: monstats `lUndead` or `hUndead`.
fn is_undead_class(cx: &Ctx<'_>, class: u32) -> bool {
    cx.monstats(class).is_some_and(|m| m.lundead || m.hundead)
}

/// The skill-calc fields of skill 66 the curse evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillCalc {
    /// `aurarangecalc` (+0x64).
    AuraRange,
    /// `auralencalc` (+0x60).
    AuraLen,
    /// `aurastatcalc1`…`aurastatcalc6` (+0x68…), 1-based.
    AuraStat(u8),
}

/// The aura columns of a skills row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuraFields {
    /// `aurastat1`…`aurastat6` (signed 16-bit).
    pub stats: [i32; 6],
    /// `auratargetstate` (+0x82, signed 16-bit).
    pub target_state: i32,
}

/// `apply_state` arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateApply {
    pub source: UnitId,
    pub target: UnitId,
    pub skill: i32,
    pub level: i32,
    pub duration: i32,
    pub stat: i32,
    pub value: i32,
    pub state: i32,
    /// 0: the default remove callback.
    pub callback: u32,
}

/// The cross-burst offsets (two tables).
pub const CROSS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The spectral-hit stat rows (table): (min, max, length).
pub const SPECTRAL: [(u16, u16, i32); 5] = [
    (stat::FIREMINDAM, stat::FIREMAXDAM, -1),
    (stat::LIGHTMINDAM, stat::LIGHTMAXDAM, -1),
    (stat::MAGICMINDAM, stat::MAGICMAXDAM, -1),
    (stat::COLDMINDAM, stat::COLDMAXDAM, stat::COLDLENGTH as i32),
    (
        stat::POISONMINDAM,
        stat::POISONMAXDAM,
        stat::POISONLENGTH as i32,
    ),
];

// ---- small helpers --------------------------------------------------------

fn frame<H: InitHost + ?Sized>(h: &mut H) -> i32 {
    h.game().frame
}

fn mode_of<H: InitHost + ?Sized>(h: &mut H, u: UnitId) -> u32 {
    h.units().get(u).map_or(u32::MAX, |r| r.mode)
}

fn type_of<H: InitHost + ?Sized>(h: &mut H, u: UnitId) -> Option<UnitType> {
    h.units().get(u).map(|r| r.ty)
}

fn class_of<H: InitHost + ?Sized>(h: &mut H, u: UnitId) -> u32 {
    h.units().get(u).map_or(0, |r| r.class)
}

fn level<H: InitHost + ?Sized>(h: &H, u: UnitId) -> i32 {
    h.stat_total(u, stat::LEVEL)
}

/// The unit's own seed (`None` for a unit without a record: no draw).
fn seed<H: InitHost + ?Sized>(h: &mut H, u: UnitId) -> Option<&mut Seed> {
    h.units().get_mut(u).map(|r| &mut r.seed)
}

/// A user-modifier event `n` base ticks from now.
pub fn event7<H: InitHost + ?Sized>(h: &mut H, u: UnitId, n: i32) {
    let g = h.game();
    let at = g.after(n);
    // A unit outside the lists schedules nothing.
    let _ = g.schedule_event(u, EVENT_UMOD, at, None, 0, 0);
}

/// Cancels the unit's events of type `ty`.
fn cancel<H: InitHost + ?Sized>(h: &mut H, u: UnitId, ty: u8) {
    h.game().timers.cancel_unit_events(u, ty, None);
}

/// A mode event `n` base ticks from now.
fn event2<H: InitHost + ?Sized>(h: &mut H, u: UnitId, n: i32) {
    let g = h.game();
    let at = g.after(n);
    let _ = g.schedule_event(u, 2, at, None, 0, 0);
}

fn diff_col(a: u16, n: u16, hh: u16, d: usize) -> i32 {
    s16([a, n, hh][d.min(2)])
}

fn sign(v: i32) -> i32 {
    v.signum()
}

// ---- the shared helpers ----------------------------------------------------

/// `skill_missile(game, m, u, skill, L, 0, 0, tx, ty, quant)`
/// (the skill missile body) for a monster owner: the target point (the
/// owner's target position when tx or ty is 0), record flags 0x21 from
/// the owner's position, and the tohit bonus (flag 0x1000) of a monster
/// owner. `quant` only reaches players; the callbacks'
/// owners are monsters.
pub fn skill_missile<H: InitHost + ?Sized>(
    h: &mut H,
    m: i32,
    owner: UnitId,
    skill: i32,
    lvl: i32,
    (tx, ty): (i32, i32),
) -> Option<UnitId> {
    let (mut tx, mut ty) = (tx, ty);
    if tx == 0 || ty == 0 {
        match h.target_position(owner) {
            Some((a, b)) if a != 0 && b != 0 => (tx, ty) = (a, b),
            _ => return None,
        }
    }
    let (x, y) = h.position(owner);
    let mut req = MissileRequest {
        flags: 0x21,
        x,
        y,
        target_x: tx,
        target_y: ty,
        skill,
        level: lvl,
        ..MissileRequest::new(owner, m)
    };
    if type_of(h, owner) == Some(UnitType::Monster) {
        let b = h.stat_total(owner, stat::TOHIT);
        if b != 0 {
            req.flags |= 0x1000;
            req.attack_bonus = b;
        }
    }
    h.create_missile(req)
}

/// Area damage; true when it hit something.
#[allow(clippy::too_many_arguments)]
pub fn area_damage<H: InitHost + ?Sized>(
    h: &mut H,
    src: UnitId,
    (x, y): (i32, i32),
    r: i32,
    rec: &DamageRecord,
    hit_owner: bool,
    hit_owner2: bool,
    flags: u32,
) -> bool {
    let flags = if flags == 0 {
        finder::PLAYERS_MONSTERS_LIVE
    } else {
        flags
    };
    let found = h.find_units(
        src,
        FindQuery {
            flags,
            x,
            y,
            r,
            ..FindQuery::default()
        },
    );
    let Some(o1) = h.owner(src) else {
        return false;
    };
    let o2 = h.owner(o1);
    // The hit callback applies nothing unless the source is a missile.
    let missile = type_of(h, src) == Some(UnitType::Missile);
    let mut hit = false;
    for v in found {
        if Some(v) == o2 && !hit_owner2 {
            continue;
        }
        if v == o1 && !hit_owner {
            continue;
        }
        if !h.hostile(o1, v) || !h.line_clear((x, y), v) {
            continue;
        }
        let copy = *rec;
        if missile {
            h.missile_hit(src, v, &copy);
        }
        hit = true;
    }
    hit
}

/// The cross burst: 8 missiles of class `m` at level `lvl`.
pub fn cross_burst<H: InitHost + ?Sized>(h: &mut H, u: UnitId, m: i32, lvl: i32) {
    let (x, y) = h.position(u);
    for (dx, dy) in CROSS {
        for j in 0..2 {
            let req = MissileRequest {
                flags: 0x21,
                x,
                y,
                target_x: x.wrapping_add(dx),
                target_y: y.wrapping_add(dy),
                level: lvl,
                skill: 0,
                init: Some((init_cb::JITTER, j)),
                ..MissileRequest::new(u, m)
            };
            let _ = h.create_missile(req);
        }
    }
}

/// The missile ring
/// (the ring body): 64 missiles from `at`'s position.
pub fn ring<H: InitHost + ?Sized>(
    h: &mut H,
    owner: UnitId,
    at: UnitId,
    m: i32,
    skill: i32,
    lvl: i32,
    v: i32,
) {
    let (x, y) = h.position(at);
    let mut req = MissileRequest {
        flags: 3,
        x,
        y,
        skill,
        level: lvl,
        ..MissileRequest::new(owner, m)
    };
    if v != 0 {
        req.flags |= 4;
        req.velocity = v;
    }
    for i in 0..64 {
        let (ox, oy) = ring_offset(i);
        req.target_x = ox;
        req.target_y = oy;
        let _ = h.create_missile(req);
    }
}

/// The values of the elemental fill: (min, max) for a
/// unit of `level`, `None` when monlvl is empty.
pub fn elem_values(cx: &Ctx<'_>, l_flag: bool, d: usize, level: i32) -> Option<(i32, i32)> {
    let rows = cx.tables.monlvl.len() as i32;
    if rows - 1 < 0 {
        return None;
    }
    let row = &cx.tables.monlvl[level.max(1).min(rows - 1) as usize];
    let d = d.min(2);
    let v = if l_flag {
        [row.l_dm, row.l_dm_n, row.l_dm_h][d]
    } else {
        [row.dm, row.dm_n, row.dm_h][d]
    } as i32;
    let n = cx.tables.monumod.len();
    let a = if n >= 29 { cx.k(28) } else { 0 };
    let b = if n >= 32 { cx.k(31) } else { 0 };
    Some((a.wrapping_mul(v) / 100, b.wrapping_mul(v) / 100))
}

/// Elemental damage fill.
pub fn elem_fill<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    u: UnitId,
    (s_min, s_max, s_len): (u16, u16, i32),
) {
    let ty = type_of(h, u);
    if ty == Some(UnitType::Missile) {
        let class = class_of(h, u) as i32;
        match h.missile_row_flags(class) {
            Some(f) if f & missile_flag::NO_UNIQUE_MOD == 0 => {}
            _ => return,
        }
    }
    let info = h.info();
    let Some((min, max)) =
        elem_values(cx, info.l_flag(), usize::from(info.difficulty), level(h, u))
    else {
        return;
    };
    let monster = ty == Some(UnitType::Monster);
    let set = |h: &mut H, s: u16, v: i32| {
        if monster {
            h.set_base_list_stat(u, s, v);
        } else {
            h.set_stat(u, s, v);
        }
    };
    set(h, s_min, min);
    set(h, s_max, max);
    if s_len != -1 {
        let s = s_len as u16;
        let t = h.stat_total(u, s).wrapping_add(40);
        set(h, s, t);
    }
}

/// Base class: monstats `BaseId`, −1 for a class outside
/// the table.
fn base_class(cx: &Ctx<'_>, class: u32) -> i32 {
    cx.monstats(class).map_or(-1, |m| i32::from(m.baseid))
}

/// Think restart.
pub fn think_restart<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) {
    cancel(h, u, 2);
    match mode_of(h, u) {
        mode::NEUTRAL => {
            cancel(h, u, 2);
            event2(h, u, 2);
        }
        mode::DEATH | mode::DEAD => return,
        _ => {}
    }
    let class = class_of(h, u);
    if class::THINK_BASES.contains(&base_class(cx, class)) {
        cancel(h, u, 2);
        event2(h, u, 2);
    }
}

// ---- the bodies --------------------------------------------------------------

/// Runs the callback at `addr`. `unit` is the dispatcher's
/// argument (mode 5: the missile); `unique` is type flag 8 of the
/// monster whose list is walked. False for an address without a body.
pub fn run<H: InitHost + ?Sized>(
    cx: &Ctx<'_>,
    h: &mut H,
    unit: UnitId,
    umod: u8,
    unique: bool,
    addr: u32,
) -> bool {
    let u = unit;
    match addr {
        addr::DEATH_MODE => {
            if (umod != 18 || unique) && mode_of(h, u) == mode::DEATH {
                event7(h, u, 4);
            }
        }
        addr::CURSE => curse(cx, h, u, unique),
        addr::FIRE_MODE => {
            if unique && mode_of(h, u) == mode::DEATH {
                event7(h, u, 4);
            }
        }
        addr::FIRE_EXPLODE => fire_explode(cx, h, u),
        addr::POISON_DEAD => {
            let l = level(h, u);
            let l = if l < 2 { 1 } else { l };
            let at = h.position(u);
            let _ = skill_missile(h, missile::CORPSE_POISON_CLOUD, u, 0, l, at);
        }
        addr::SPC_DAMAGE => spc_damage(h, u),
        addr::PARTY_DEAD => party_dead(h, u),
        addr::LIGHTNING_MODE => {
            if unique && mode_of(h, u) == mode::GETHIT {
                event7(h, u, 2);
            }
        }
        addr::LIGHTNING_BURST => lightning_burst(h, u, unique),
        addr::LIGHTNING_HIT => {
            if unique && mode_of(h, u) != mode::GETHIT {
                lightning_burst(h, u, unique);
            }
        }
        addr::COLD_NOVA => {
            if unique {
                let l = level(h, u) / 2;
                let l = if l <= 1 { 1 } else { l };
                let v = h.missile_velocity(missile::FROST_NOVA, 0);
                ring(h, u, u, missile::COLD_UNIQUE, 0, l, v);
            }
        }
        addr::HIREABLE_MODE => hireable_mode(h, u),
        addr::HIREABLE_MISSILE => hireable_missile(h, u),
        addr::SCARAB_MODE => {
            if matches!(mode_of(h, u), mode::GETHIT | mode::DEATH) {
                event7(h, u, 2);
            }
        }
        addr::SCARAB_BURST => {
            let l = level(h, u);
            cross_burst(h, u, missile::BUG_LIGHTNING, l);
        }
        addr::KILL_SELF => kill_self(h, u),
        addr::QUEST_COMPLETE => {
            if mode_of(h, u) == mode::DEATH {
                if let Some(call) = quest_death_call(class_of(h, u)) {
                    quest_death(cx, h, u, call);
                }
            }
        }
        addr::POISON_HIT => {
            let l = level(h, u);
            cross_burst(h, u, missile::QUEEN_POISON_CLOUD, l);
        }
        addr::THIEF => thief(h, u, unique),
        addr::SPECTRAL_MODE => spectral(cx, h, u, unique),
        addr::SPECTRAL_MISSILE => {
            if unique && type_of(h, u) == Some(UnitType::Missile) {
                spectral(cx, h, u, unique);
            }
        }
        addr::MULTISHOT => multishot(h, u, unique),
        addr::GOBOOM => {
            let at = h.position(u);
            if let Some(m) = skill_missile(h, missile::MONSTER_CORPSE_EXPLODE, u, 0, 1, at) {
                let rec = DamageRecord {
                    physical: 0x6400,
                    fire: 0x6400,
                    ..DamageRecord::default()
                };
                area_damage(
                    h,
                    m,
                    at,
                    6,
                    &rec,
                    false,
                    false,
                    finder::PLAYERS_MONSTERS_LIVE,
                );
            }
        }
        addr::FIRESPIKE => firespike(cx, h, u),
        addr::SUICIDE_MODE => match mode_of(h, u) {
            mode::GETHIT => {
                h.set_mode(u, mode::DEATH);
                event7(h, u, 4);
            }
            mode::DEATH | mode::DEAD => event7(h, u, 4),
            _ => {}
        },
        addr::SUICIDE_EXPLODE => suicide_explode(cx, h, u),
        addr::AI_AFTER_DEATH => {
            if mode_of(h, u) == mode::DEATH && h.alignment(u) == 0 {
                cancel(h, u, EVENT_UMOD as u8);
                let (aip8, aip1) = aip8_aip1(cx, h, u);
                let lo = seed(h, u).map_or(0, |s| s.step());
                if ((lo % 100) as i32) < aip8 {
                    event7(h, u, 10i32.wrapping_mul(aip1).wrapping_add(1));
                }
            }
        }
        addr::REVIVE => revive(cx, h, u),
        addr::SHATTER => {
            if mode_of(h, u) == mode::DEATH {
                h.set_state(u, state::SHATTER);
            }
        }
        addr::WORMS => {
            if mode_of(h, u) == mode::DEATH {
                h.spawn_near(u, class::PAINWORM1, mode::NEUTRAL, 1, 0);
                h.set_state(u, state::CORPSE_NODRAW);
            }
        }
        addr::ALWAYS_RUN_AI => {
            if !h.units().is_dead(u) {
                think_restart(cx, h, u);
                event7(h, u, 75);
            }
        }
        addr::LIGHTNING_DEATH => lightning_death(h, u, unique),
        _ => return false,
    }
    true
}

fn aip8_aip1<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) -> (i32, i32) {
    let d = h.info().d();
    let class = class_of(h, u);
    cx.monstats(class).map_or((0, 0), |m| {
        (
            diff_col(m.aip8, m.aip8_n, m.aip8_h, d),
            diff_col(m.aip1, m.aip1_n, m.aip1_h, d),
        )
    })
}

/// Curse level: level / 5 + 1, at least 1.
pub fn curse_level(level: i32) -> i32 {
    (level / 5 + 1).max(1)
}

/// Umod 7, mode 3.
fn curse<H: InitHost + ?Sized>(_cx: &Ctx<'_>, h: &mut H, u: UnitId, unique: bool) {
    if !unique {
        return;
    }
    let lo = seed(h, u).map_or(0, |s| s.step());
    if lo & 3 == 0 {
        return;
    }
    let l = curse_level(level(h, u));
    if h.aura_fields(skill::AMPLIFY_DAMAGE).is_none() {
        return;
    }
    let r = h
        .skill_calc(u, skill::AMPLIFY_DAMAGE, SkillCalc::AuraRange, l)
        .clamp(1, 40);
    let Some((x, y)) = h.target_position(u) else {
        return;
    };
    let found = h.find_units(
        u,
        FindQuery {
            flags: finder::PLAYERS_MONSTERS,
            exclude: Some(u),
            x,
            y,
            r,
            ..FindQuery::default()
        },
    );
    for v in found {
        curse_target(h, u, v, l);
    }
}

/// Per target.
fn curse_target<H: InitHost + ?Sized>(h: &mut H, u: UnitId, v: UnitId, l: i32) {
    if !h.hostile(u, v) {
        return;
    }
    let Some(a) = h.aura_fields(skill::AMPLIFY_DAMAGE) else {
        return;
    };
    let (stats, states) = h.stat_and_state_counts();
    if a.stats[0] < -1 || a.stats[0] >= stats || a.target_state < 0 || a.target_state >= states {
        return;
    }
    let sk = skill::AMPLIFY_DAMAGE;
    let duration = h.skill_calc(u, sk, SkillCalc::AuraLen, l);
    let value = h.skill_calc(u, sk, SkillCalc::AuraStat(1), l);
    let Some(list) = h.apply_state(StateApply {
        source: u,
        target: v,
        skill: i32::from(sk),
        level: l,
        duration,
        stat: a.stats[0],
        value,
        state: a.target_state,
        callback: 0,
    }) else {
        return;
    };
    for i in 2..=6u8 {
        let s = a.stats[usize::from(i - 1)];
        if !(0..stats).contains(&s) {
            continue;
        }
        let val = h.skill_calc(u, sk, SkillCalc::AuraStat(i), l);
        if val != 0 {
            set_list(h, list, s as u16, val);
        }
    }
}

fn set_list<H: InitHost + ?Sized>(h: &mut H, list: ListId, s: u16, v: i32) {
    h.set_list_stat(list, s, v);
}

/// (a, b) of the fire explosion from H and
/// `MonsterCEDamagePercent` on difficulty `d`.
pub fn fire_range(hp: i32, ce: i32, d: u8) -> (i32, i32) {
    let mut a = pct(hp, ce, 100);
    match d {
        0 => a -= a / 4,
        1 => a -= a / 3,
        2 => a /= 8,
        _ => {}
    }
    (a, pct(a, 60, 100))
}

/// Umod 9, mode 2 (death explosion).
fn fire_explode<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) {
    let at = h.position(u);
    let Some(m) = skill_missile(h, missile::MONSTER_CORPSE_EXPLODE, u, 0, 1, at) else {
        return;
    };
    let info = h.info();
    let class = class_of(h, u);
    let lv = level(h, u);
    let hp = cx.monstats(class).map_or(0, |ms| {
        stats_by_level(
            ms,
            cx.tables.monlvl,
            info.l_flag(),
            usize::from(info.difficulty),
            lv,
        )
        .max_hp
    });
    let d = info.difficulty;
    let Some(dl) = cx.tables.difficultylevels.get(usize::from(d)) else {
        return;
    };
    let (a, b) = fire_range(hp, dl.monstercedamagepercent as i32, d);
    let roll = seed(h, u).map_or(0, |s| s.roll(a.wrapping_sub(b)));
    let dmg = b.wrapping_add(roll as i32);
    let rec = DamageRecord {
        physical: dmg << 6,
        fire: dmg << 6,
        ..DamageRecord::default()
    };
    area_damage(
        h,
        m,
        at,
        i32::from(d) + 4,
        &rec,
        false,
        false,
        finder::PLAYERS_LIVE,
    );
}

/// The base-list values sets for a trap class at area level n.
pub fn trap_stats(class: u32, n: i32) -> Vec<(u16, i32)> {
    let h = n >> 1;
    let three_half = n.wrapping_mul(3) >> 1;
    match class {
        class::TRAP_FIREBOLT => vec![
            (stat::MINDAMAGE, 0),
            (stat::MAXDAMAGE, 0),
            (stat::FIREMINDAM, h),
            (stat::FIREMAXDAM, three_half),
        ],
        class::TRAP_HORZMISSILE | class::TRAP_VERTMISSILE => {
            vec![(stat::MINDAMAGE, (h + 1).min(n)), (stat::MAXDAMAGE, n)]
        }
        class::TRAP_POISONCLOUD => vec![
            (stat::MINDAMAGE, 0),
            (stat::MAXDAMAGE, 0),
            (stat::POISONMINDAM, n),
            (stat::POISONMAXDAM, n.wrapping_mul(2)),
            (stat::POISONLENGTH, n.wrapping_mul(2)),
        ],
        class::TRAP_LIGHTNING | class::TRAP_NOVA => vec![
            (stat::MINDAMAGE, 0),
            (stat::MAXDAMAGE, 0),
            (stat::LIGHTMINDAM, h),
            (stat::LIGHTMAXDAM, three_half),
        ],
        _ => Vec::new(),
    }
}

/// Umod 14, mode 0 (traps).
fn spc_damage<H: InitHost + ?Sized>(h: &mut H, u: UnitId) {
    let mut n = 2;
    if let Some(al) = h.room_area_level(u) {
        n = al;
    }
    if n < 2 {
        n = 1;
    }
    h.set_stat(u, stat::LEVEL, n);
    h.set_stat(u, stat::TOHIT, (n + 50).min(90));
    if type_of(h, u) != Some(UnitType::Monster) {
        h.monsters().unhandled.push(Unhandled::Assert {
            addr: addr::SPC_DAMAGE,
            unit: u,
        });
        return;
    }
    let class = class_of(h, u);
    for (s, v) in trap_stats(class, n) {
        h.set_base_list_stat(u, s, v);
    }
}

/// Umod 15, mode 1.
fn party_dead<H: InitHost + ?Sized>(h: &mut H, u: UnitId) {
    if mode_of(h, u) != mode::DEATH {
        return;
    }
    let Some(o) = h.minion_owner(u) else {
        return;
    };
    for m in h.minions(u) {
        h.free_minions(m);
        h.clear_owner_data(m);
        if mode_of(h, m) != mode::DEATH {
            h.set_mode(m, mode::DEATH);
        }
    }
    h.clear_owner_data(o);
    if o != u && type_of(h, o) == Some(UnitType::Monster) {
        h.set_mode(o, mode::DEATH);
    }
}

/// Burst level: level / 2, at least 1.
pub fn burst_level(level: i32) -> i32 {
    (level / 2).max(1)
}

/// Umod 17, mode 2 (burst).
fn lightning_burst<H: InitHost + ?Sized>(h: &mut H, u: UnitId, unique: bool) {
    if !unique {
        return;
    }
    let f = frame(h);
    let burst_gap = h.game().ticks(10);
    let t = h.monsters().get(u).map_or(0, |m| m.last_burst);
    if f.wrapping_sub(t).wrapping_abs() < burst_gap {
        if let Some(m) = h.monsters().get_mut(u) {
            m.type_flags &= !data_flag::BURST;
        }
        return;
    }
    if let Some(m) = h.monsters().get_mut(u) {
        m.last_burst = f;
        m.type_flags |= data_flag::BURST;
    }
    let l = burst_level(level(h, u));
    cross_burst(h, u, missile::LIGHT_UNIQUE, l);
}

/// Values: (min, max) after the secondary damage.
pub fn hireable_values(min: i32, max: i32, smin: i32, smax: i32) -> (i32, i32) {
    (
        min.wrapping_sub(1).wrapping_add(smin).max(0),
        max.wrapping_sub(1).wrapping_add(smax).max(0),
    )
}

/// Values on a missile.
pub fn hireable_missile_values(min: i32, max: i32, smin: i32, smax: i32) -> (i32, i32) {
    (
        min.wrapping_sub(256)
            .wrapping_add(smin.wrapping_mul(256))
            .max(0),
        max.wrapping_sub(256)
            .wrapping_add(smax.wrapping_mul(256))
            .max(0),
    )
}

/// Umod 19, mode 0.
fn hireable_mode<H: InitHost + ?Sized>(h: &mut H, u: UnitId) {
    let a = h.stat_total(u, stat::MINDAMAGE);
    let b = h.stat_total(u, stat::MAXDAMAGE);
    if b <= 0 || class_of(h, u) == class::ROGUEHIRE {
        return;
    }
    let (a, b) = hireable_values(
        a,
        b,
        h.stat_total(u, stat::SECONDARY_MINDAMAGE),
        h.stat_total(u, stat::SECONDARY_MAXDAMAGE),
    );
    h.set_base_list_stat(u, stat::MINDAMAGE, a);
    h.set_base_list_stat(u, stat::MAXDAMAGE, b);
}

/// Umod 19, mode 5 (on the missile).
fn hireable_missile<H: InitHost + ?Sized>(h: &mut H, m: UnitId) {
    if type_of(h, m) != Some(UnitType::Missile) {
        return;
    }
    let a = h.stat_total(m, stat::MINDAMAGE);
    let b = h.stat_total(m, stat::MAXDAMAGE);
    if b <= 0 {
        return;
    }
    let Some(o) = h.owner(m) else {
        return;
    };
    if class_of(h, o) != class::ROGUEHIRE {
        return;
    }
    let (a, b) = hireable_missile_values(
        a,
        b,
        h.stat_total(o, stat::SECONDARY_MINDAMAGE),
        h.stat_total(o, stat::SECONDARY_MAXDAMAGE),
    );
    h.set_stat(m, stat::MINDAMAGE, a);
    h.set_stat(m, stat::MAXDAMAGE, b);
}

/// Umod 21, mode 2.
fn kill_self<H: InitHost + ?Sized>(h: &mut H, u: UnitId) {
    if h.units().is_dead(u) {
        return;
    }
    if h.has_state(u, state::UNINTERRUPTABLE) {
        event7(h, u, 3);
        return;
    }
    match h.minion_owner(u) {
        Some(o) if type_of(h, o) == Some(UnitType::Player) => h.remove_pet(o, u),
        _ => h.set_mode(u, mode::DEATH),
    }
}

/// Umod 24, mode 3 (no unit reaches it with the game's data).
fn thief<H: InitHost + ?Sized>(h: &mut H, u: UnitId, unique: bool) {
    if !unique {
        return;
    }
    let lo = seed(h, u).map_or(0, |s| s.step());
    if lo % 100 < 30 {
        return;
    }
    let Some(t) = h.target(u) else {
        return;
    };
    if type_of(h, t) != Some(UnitType::Player) {
        return;
    }
    h.steal_belt_item(u, t);
}

/// On `u` (the monster, or the missile of mode 5).
fn spectral<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId, unique: bool) {
    if !unique {
        return;
    }
    let lo = seed(h, u).map_or(0, |s| s.step());
    let row = SPECTRAL[(lo % 5) as usize];
    elem_fill(cx, h, u, row);
}

/// Copy targets: the two (x, y) of the copies for owner position
/// `o`, target `t` and missile class `m`.
pub fn multishot_targets(o: (i32, i32), t: (i32, i32), m: i32) -> [(i32, i32); 2] {
    let (mut sx, mut sy) = (sign(o.0.wrapping_sub(t.0)), sign(o.1.wrapping_sub(t.1)));
    if missile::MUMMIES.contains(&m) {
        (sx, sy) = (0, 0);
    }
    [
        (t.0.wrapping_sub(sy), t.1.wrapping_add(sx)),
        (t.0.wrapping_add(sy), t.1.wrapping_sub(sx)),
    ]
}

/// Umod 29, mode 5 (on the missile).
fn multishot<H: InitHost + ?Sized>(h: &mut H, mi: UnitId, unique: bool) {
    if !unique || type_of(h, mi) != Some(UnitType::Missile) {
        return;
    }
    let m = class_of(h, mi) as i32;
    let Some(o) = h.owner(mi) else {
        return;
    };
    match h.missile_row_flags(m) {
        Some(f) if f & missile_flag::NO_MULTI_SHOT == 0 => {}
        _ => return,
    }
    if h.monsters()
        .get(o)
        .is_some_and(|d| d.type_flags & data_flag::MULTISHOT != 0)
    {
        return;
    }
    let t = match h.target(o) {
        Some(t) => h.position(t),
        None => h.path_target_point(mi),
    };
    // Step 4: a local seed {'SEIS', O's GUID} is built and never drawn.
    let op = h.position(o);
    let targets = multishot_targets(op, t, m);
    if let Some(d) = h.monsters().get_mut(o) {
        d.type_flags |= data_flag::MULTISHOT;
    }
    let (k, l) = h.missile_skill_level(mi);
    for at in targets {
        let _ = skill_missile(h, m, o, k, l, at);
    }
    if let Some(d) = h.monsters().get_mut(o) {
        d.type_flags &= !data_flag::MULTISHOT;
    }
}

/// Umod 32, mode 2 (no damage applied: the source is the unit).
fn firespike<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) {
    let class = class_of(h, u);
    let Some(ms) = cx.monstats(class) else {
        return;
    };
    let d = h.info().d();
    let (min, max) = (s16(ms.el1mind), s16(ms.el1maxd));
    let aip3 = diff_col(ms.aip3, ms.aip3_n, ms.aip3_h, d);
    let roll = seed(h, u).map_or(0, |s| s.roll(max.wrapping_sub(min)));
    let v = min.wrapping_add(roll as i32).wrapping_mul(256);
    let r = level(h, u).wrapping_mul(aip3);
    let rec = DamageRecord {
        fire: v,
        ..DamageRecord::default()
    };
    let at = h.position(u);
    area_damage(h, u, at, r, &rec, false, false, 0);
}

/// Record and missile for `aip1` = a and damage v.
pub fn suicide_record(a: i32, v: i32) -> (DamageRecord, i32) {
    let mut rec = DamageRecord {
        result: 8,
        physical: v,
        ..DamageRecord::default()
    };
    let m = if a == 1 {
        rec.fire = v;
        missile::SUICIDE_FIRE_EXPLODE
    } else if a >= 2 {
        rec.cold = v;
        rec.cold_len = a;
        missile::SUICIDE_ICE_EXPLODE
    } else {
        missile::SUICIDE_CORPSE_EXPLODE
    };
    (rec, m)
}

/// Umod 33, mode 2.
fn suicide_explode<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) {
    let class = class_of(h, u);
    let Some(ms) = cx.monstats(class) else {
        return;
    };
    let info = h.info();
    let d = info.d();
    let a = diff_col(ms.aip1, ms.aip1_n, ms.aip1_h, d);
    let (p, q) = super::calc::a1_damage(
        ms,
        cx.tables.monlvl,
        info.l_flag(),
        usize::from(info.difficulty),
        level(h, u),
    );
    let roll = seed(h, u).map_or(0, |s| s.roll(q.wrapping_sub(p)));
    let v = p.wrapping_add(roll as i32).wrapping_mul(256);
    let (rec, m) = suicide_record(a, v);
    let r = diff_col(ms.aip4, ms.aip4_n, ms.aip4_h, d);
    let at = h.position(u);
    if let Some(mi) = skill_missile(h, m, u, 0, 1, at) {
        area_damage(h, mi, at, r, &rec, false, false, finder::PLAYERS_LIVE);
    }
}

/// Umod 34, mode 2 (revive).
fn revive<H: InitHost + ?Sized>(cx: &Ctx<'_>, h: &mut H, u: UnitId) {
    cancel(h, u, 2);
    if !h.units().is_dead(u)
        || h.has_state_in_group(u, state::GROUP_UDEAD)
        || h.monsters()
            .get(u)
            .is_some_and(|m| m.has_flag(type_flag::UNIQUE))
        || h.alignment(u) == 2
    {
        return;
    }
    if h.footprint_occupied(u) {
        let (_, aip1) = aip8_aip1(cx, h, u);
        event7(h, u, 10i32.wrapping_mul(aip1).wrapping_add(1));
        return;
    }
    let n = h.ai_param0(u);
    if n >= 2 {
        return;
    }
    if !h.can_raise(u) || h.alignment(u) != 0 {
        return;
    }
    h.ai_use_skill(u, mode::SKILL1, skill::SELF_RESURRECT);
    h.clear_unit_flags(u, 0x0402_0000);
    h.set_ai_param0(u, n + 1);
    cancel(h, u, 2);
    event2(h, u, 51);
}

/// Level from the owner's Shadow Warrior level (`None`: no entry)
/// and the life percent.
pub fn lightning_death_level(sw_level: Option<i32>, life_pct: i32) -> i32 {
    let mut l = 1;
    if let Some(s) = sw_level {
        let k = s / 2 + 1;
        if k > 1 {
            l = k.min(15);
        }
    }
    if life_pct > 10 {
        l = 1;
    }
    l
}

/// Umod 42, mode 2.
fn lightning_death<H: InitHost + ?Sized>(h: &mut H, u: UnitId, unique: bool) {
    if !unique {
        return;
    }
    let sw = match h.minion_owner(u) {
        Some(o) => h.skill_level(o, skill::SHADOW_WARRIOR),
        None => None,
    };
    let l = lightning_death_level(sw, h.life_percent(u));
    let v = h.missile_velocity(missile::NOVA, 0);
    ring(h, u, u, missile::NOVA, 0, l, v);
}
