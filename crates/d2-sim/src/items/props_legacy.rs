//! Format-0 property functions (the legacy table): the wrapper sends a
//! property record of a format-0 item (version-0x47 saves only) here
//! instead of the property dispatcher, for every mode except 6
//! (runewords) and for the format-0 quality and affix paths.
//! One function per property code, no slots, no prev value. All draws on
//! the item seed.

use super::create::max_sockets;
use super::props::{base_reset, charges, set_raw, skill_or_zero, PropCtx};
use super::tables::{ItemTables, PropRec};
use super::{flag, stat, ty, Fatal, Item, ItemStats};

/// The legacy functions, named by their the base game address (table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyFn {
    /// Not filled (a test checks that every code 0–243 is filled).
    Unset,
    ///`(s, kind 0).
    D1C0,
    ///`(s, kind 1).
    D2B0,
    ///`(127, kind 0).
    DB90,
    /// A(s, R).
    E450,
    /// V := `param`; A(s, v).
    D110,
    /// By time, with fatal range checks.
    DD80,
    /// Stat 83 with a layer by s.
    E2D0,
    /// Stat 126, layer 1.
    E230,
    /// Stat 107, layer skill.
    E170,
    /// S, s + 1, s + 2 := `min`, `max`, `param`.
    E070,
    /// S, s + 1 := `min`, `max` (s = 21: the damage pairs).
    DE40,
    /// Min damage.
    D650,
    /// Max damage.
    D7D0,
    /// Enhanced damage (17, 18).
    D950,
    /// Resistances 39, 41, 43, 45.
    D310,
    /// Max resistances 40, 42, 44, 46.
    D4B0,
    /// Sockets.
    D220,
    /// Base durability 73, 72 := 0.
    D270,
    /// Charges (function 19's formulas).
    DBC0,
    /// Nothing.
    E440,
}

/// Entries of the legacy table (codes 0–243).
pub const LEGACY_COUNT: usize = 244;

/// (first code, last code, function, stat of the first code, stat step
/// per code): the legacy table as runs.
#[rustfmt::skip]
const RUNS: &[(usize, usize, LegacyFn, u16, u16)] = {
    use LegacyFn::*;
    &[
        (0, 0, D1C0, 31, 0), (1, 1, D1C0, 32, 0), (2, 2, D1C0, 33, 0), (3, 3, D1C0, 34, 0),
        (4, 4, D1C0, 36, 0), (6, 6, D1C0, 35, 0), (7, 7, D1C0, 0, 0), (8, 8, D1C0, 2, 0),
        (9, 9, D1C0, 3, 0), (10, 10, D1C0, 1, 0), (11, 11, D1C0, 9, 0), (13, 13, D1C0, 7, 0),
        (15, 16, D1C0, 19, 1), (17, 19, D1C0, 54, 1), (20, 23, D1C0, 48, 1),
        (24, 26, D1C0, 57, 1), (31, 36, D1C0, 39, 1), (37, 38, D1C0, 37, 1),
        (39, 40, D1C0, 45, 1), (43, 50, D1C0, 142, 1), (51, 51, D1C0, 73, 0),
        (53, 53, D1C0, 74, 0), (54, 54, D1C0, 78, 0), (58, 60, D1C0, 79, 1),
        (63, 63, D1C0, 11, 0), (64, 64, D1C0, 82, 0), (65, 65, D1C0, 62, 0),
        (66, 66, D1C0, 60, 0), (72, 75, D1C0, 88, 1), (88, 91, D1C0, 110, 1),
        (92, 95, D1C0, 115, 1), (97, 97, D1C0, 120, 0), (100, 102, D1C0, 123, 1),
        (105, 105, D1C0, 128, 0), (106, 113, D1C0, 134, 1), (114, 114, D1C0, 150, 0),
        (115, 120, D1C0, 153, 1), (181, 181, D1C0, 254, 0),
        (5, 5, D2B0, 16, 0), (12, 12, D2B0, 77, 0), (14, 14, D2B0, 76, 0),
        (30, 30, D2B0, 114, 0), (52, 52, D2B0, 75, 0), (61, 61, D2B0, 28, 0),
        (62, 62, D2B0, 27, 0), (96, 96, D2B0, 119, 0), (98, 98, D2B0, 121, 0),
        (99, 99, D2B0, 122, 0),
        (104, 104, DB90, 127, 0),
        (55, 57, E450, 93, 0), (76, 78, E450, 96, 0), (79, 81, E450, 99, 0),
        (82, 84, E450, 102, 0), (85, 87, E450, 105, 0),
        (141, 177, D110, 214, 1), (179, 179, D110, 252, 0), (180, 180, D110, 253, 0),
        (195, 230, DD80, 268, 1),
        (67, 71, E2D0, 83, 1), (121, 121, E2D0, 179, 0), (122, 122, E2D0, 180, 0),
        (103, 103, E230, 126, 0),
        (123, 123, E170, 107, 0),
        (137, 137, E070, 54, 0), (138, 138, E070, 57, 0),
        (134, 134, DE40, 48, 0), (135, 135, DE40, 50, 0), (136, 136, DE40, 52, 0),
        (139, 139, DE40, 159, 0), (140, 140, DE40, 21, 0),
        (27, 27, D650, 21, 0),
        (28, 28, D7D0, 22, 0),
        (29, 29, D950, 0, 0),
        (41, 41, D310, 0, 0),
        (42, 42, D4B0, 0, 0),
        (133, 133, D220, 194, 0),
        (242, 242, D270, 0, 0),
        (243, 243, DBC0, 204, 0),
        (124, 132, E440, 0, 0), (178, 178, E440, 0, 0), (182, 194, E440, 0, 0),
        (231, 241, E440, 0, 0),
    ]
};

const fn build() -> [(LegacyFn, u16); LEGACY_COUNT] {
    let mut t = [(LegacyFn::Unset, 0u16); LEGACY_COUNT];
    let mut k = 0;
    while k < RUNS.len() {
        let (a, b, f, s, step) = RUNS[k];
        let mut c = a;
        while c <= b {
            t[c] = (f, s + step * (c - a) as u16);
            c += 1;
        }
        k += 1;
    }
    t
}

/// The legacy table: (function, stat) per code 0–243.
pub const LEGACY: [(LegacyFn, u16); LEGACY_COUNT] = build();

/// R(min, max) (helpers): no draw when min = max; else
/// lo + roll(hi − lo, + 1 when format ≥ 1); so format 0 never rolls max.
fn r<S>(item: &mut Item<S>, min: i32, max: i32) -> i32 {
    if min == max {
        return min;
    }
    let (lo, hi) = (min.min(max), min.max(max));
    let mut n = hi.wrapping_sub(lo);
    if item.format >= 1 {
        n = n.wrapping_add(1);
    }
    lo.wrapping_add(item.item_seed.roll(n) as i32)
}

/// The target list: the owner's when given, else the item's.
macro_rules! target {
    ($item:expr, $ctx:expr) => {{
        let t: &mut dyn ItemStats = match $ctx.owner.as_deref_mut() {
            Some(o) => o,
            None => &mut $item.stats,
        };
        t
    }};
}

/// The call's fixed arguments.
struct Call<'r> {
    rec: &'r PropRec,
    /// The wrapper's sixth argument ("n").
    n: i32,
}

/// A(stat, v): base reset when mode = 1 or `force`; v = 0
/// → 0; v × 256 for stats 6–11, 216, 217; add to the list (layer 0), or
/// with n ≠ 0 the negative of the list's current value. Returns 1.
///
/// Reset "then A": read as one reset, A's,
/// forced by the caller's condition ("or when the caller forces it").
fn a<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    c: &Call,
    id: u16,
    v: i32,
    force: bool,
) -> i32 {
    if force || ctx.mode == 1 {
        base_reset(t, item, id);
    }
    if v == 0 {
        return 0;
    }
    let v = if matches!(id, 6..=11 | 216 | 217) {
        v.wrapping_mul(256)
    } else {
        v
    };
    let key = ctx.list;
    let target = target!(item, ctx);
    let add = if c.n != 0 {
        target.list_get(key, id, 0).wrapping_neg()
    } else {
        v
    };
    target.list_add(key, id, 0, add);
    1
}

/// Adds 1 to stat 326 (`poison_count`) in the list.
fn poison_count<S: ItemStats>(item: &mut Item<S>, ctx: &mut PropCtx) {
    let key = ctx.list;
    target!(item, ctx).list_add(key, stat::POISON_COUNT, 0, 1);
}

///`(stat, kind): v := R; base reset when kind ≠ 0 or mode =
/// 1; A(stat, v); stat 58 → stat 326 + 1. Returns 1.
fn cf40<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    c: &Call,
    id: u16,
    kind: bool,
) -> i32 {
    let v = r(item, c.rec.min, c.rec.max);
    a(t, item, ctx, c, id, v, kind);
    if id == stat::POISONMAXDAM {
        poison_count(item, ctx);
    }
    1
}

/// A layered add.
fn add_layer<S: ItemStats>(item: &mut Item<S>, ctx: &mut PropCtx, id: u16, layer: u16, v: i32) {
    let key = ctx.list;
    target!(item, ctx).list_add(key, id, layer, v);
}

/// One format-0 property record. `n` is the wrapper's sixth
/// argument. `Err` is a fatal error of the base game (crash codes, by-time
/// range checks).
pub fn apply<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    rec: &PropRec,
    n: i32,
) -> Result<(), Fatal> {
    let code = rec.code;
    let Some(ci) = usize::try_from(code)
        .ok()
        .filter(|&c| c < t.properties.len())
    else {
        return Ok(());
    };
    let Some(&(f, s)) = LEGACY.get(ci) else {
        // Codes 244–267 read the dispatcher table's words as (function, stat):
        // 244 and 257–261 meet a null function; every other one calls a
        // non-legacy address and crashes.
        // Codes ≥ 268 exist only in a table with more rows than the base game's
        // 268 (synthetic tables): D2RUST's own, read as the same crash.
        return if ci == 244 || (257..=261).contains(&ci) {
            Ok(())
        } else {
            Err(Fatal::LegacyPropertyCrash(code))
        };
    };
    let c = Call { rec, n };
    run(t, item, ctx, &c, f, s)
}

fn run<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    c: &Call,
    f: LegacyFn,
    s: u16,
) -> Result<(), Fatal> {
    use LegacyFn::*;
    let rec = c.rec;
    match f {
        Unset | E440 => {}
        D1C0 => {
            cf40(t, item, ctx, c, s, false);
        }
        D2B0 => {
            cf40(t, item, ctx, c, s, true);
        }
        DB90 => {
            cf40(t, item, ctx, c, 127, false);
        }
        E450 => {
            let v = r(item, rec.min, rec.max);
            a(t, item, ctx, c, s, v, false);
        }
        D110 => {
            if rec.param != 0 {
                let force = (16..=18).contains(&s);
                a(t, item, ctx, c, s, rec.param, force);
            }
        }
        DD80 => {
            // Function 18 with fatals instead of clamps, in
            // the base game's check order.
            if rec.min > rec.max {
                return Err(Fatal::LegacyByTime(0x444));
            }
            // Unsigned compare (`cmp 3 / ja`): a negative param is fatal.
            if (rec.param as u32) > 3 {
                return Err(Fatal::LegacyByTime(0x44C));
            }
            let a_ = rec.min.wrapping_add(256);
            let b = rec.max.wrapping_add(256);
            if a_ as u32 > 0x3FF {
                return Err(Fatal::LegacyByTime(0x44D));
            }
            if b as u32 > 0x3FF {
                return Err(Fatal::LegacyByTime(0x44E));
            }
            let v = rec
                .param
                .wrapping_add(b.wrapping_mul(1024).wrapping_add(a_).wrapping_mul(4));
            set_raw(item, ctx, s, 0, v);
        }
        E2D0 => {
            let v = r(item, rec.min, rec.max);
            if v != 0 {
                let layer = match s {
                    83..=87 => s - 83,
                    179 => 5,
                    _ => 6,
                };
                add_layer(item, ctx, 83, layer, v);
            }
        }
        E230 => {
            let v = r(item, rec.min, rec.max);
            if v != 0 {
                add_layer(item, ctx, 126, 1, v);
            }
        }
        E170 => {
            let v = r(item, rec.min, rec.max);
            let skill = skill_or_zero(t, rec.param);
            add_layer(item, ctx, stat::ITEM_SINGLESKILL, skill as u16, v);
        }
        E070 => {
            a(t, item, ctx, c, s, rec.min, false);
            a(t, item, ctx, c, s + 1, rec.max, false);
            a(t, item, ctx, c, s + 2, rec.param, false);
            if s == 57 {
                poison_count(item, ctx);
            }
        }
        DE40 if s == stat::MINDAMAGE => {
            // The owner is never an item here: the item's own row.
            let Some(it) = t.item(item.record).cloned() else {
                return Ok(());
            };
            let weapon = t.is_type(item.record, ty::WEAP as i16);
            let throwable = t.itype_of(item.record).is_some_and(|x| x.throwable != 0);
            if !(weapon && it.maxdam == 0 && it.maxdam2 != 0) {
                a(t, item, ctx, c, stat::MINDAMAGE, rec.min, false);
                a(t, item, ctx, c, stat::MAXDAMAGE, rec.max, false);
            }
            if !(weapon && it.maxdam2 == 0 && it.mindam != 0) {
                a(t, item, ctx, c, stat::SECONDARY_MINDAMAGE, rec.min, false);
                a(t, item, ctx, c, stat::SECONDARY_MAXDAMAGE, rec.max, false);
            }
            if throwable {
                a(t, item, ctx, c, stat::THROW_MINDAMAGE, rec.min, false);
                a(t, item, ctx, c, stat::THROW_MAXDAMAGE, rec.max, false);
            }
        }
        DE40 => {
            a(t, item, ctx, c, s, rec.min, false);
            a(t, item, ctx, c, s + 1, rec.max, false);
        }
        D650 | D7D0 => {
            let v = r(item, rec.min, rec.max);
            let Some(it) = t.item(item.record).cloned() else {
                return Ok(());
            };
            let weapon = t.is_type(item.record, ty::WEAP as i16);
            let throwable = t.itype_of(item.record).is_some_and(|x| x.throwable != 0);
            let (one, two, mis, c1, c2) = if f == D650 {
                (
                    stat::MINDAMAGE,
                    stat::SECONDARY_MINDAMAGE,
                    stat::THROW_MINDAMAGE,
                    it.mindam == 0 && it.mindam2 != 0,
                    it.mindam2 == 0 && it.mindam != 0,
                )
            } else {
                (
                    stat::MAXDAMAGE,
                    stat::SECONDARY_MAXDAMAGE,
                    stat::THROW_MAXDAMAGE,
                    it.maxdam == 0 && it.maxdam2 != 0,
                    it.maxdam2 == 0 && it.mindam != 0,
                )
            };
            if !(weapon && c1) {
                a(t, item, ctx, c, one, v, false);
            }
            if !(weapon && c2) {
                a(t, item, ctx, c, two, v, false);
            }
            if !weapon || throwable {
                a(t, item, ctx, c, mis, v, false);
            }
        }
        D950 => {
            cf40(t, item, ctx, c, stat::MAXDAMAGE_PERCENT, true);
            cf40(t, item, ctx, c, stat::MINDAMAGE_PERCENT, true);
        }
        D310 => {
            for id in [39, 41, 43, 45] {
                cf40(t, item, ctx, c, id, false);
            }
        }
        D4B0 => {
            for id in [40, 42, 44, 46] {
                cf40(t, item, ctx, c, id, false);
            }
        }
        D220 => {
            item.flags |= flag::SOCKETED;
            let Some(it) = t.item(item.record) else {
                return Ok(());
            };
            let wh = i32::from(it.invwidth) * i32::from(it.invheight);
            if wh != 0 {
                let cap = wh.min(6).min(max_sockets(t, item));
                let count = if cap < 1 {
                    0
                } else {
                    rec.param.max(1).min(cap)
                };
                item.stats.set_base(stat::NUMSOCKETS, 0, count);
            }
        }
        D270 => {
            // The extra unit if it is an item (a rune's socketed item,
            // mode 5), else the item.
            match ctx.extra.as_deref_mut() {
                Some(x) => {
                    x.set_base(stat::MAXDURABILITY, 0, 0);
                    x.set_base(stat::DURABILITY, 0, 0);
                }
                None => {
                    item.stats.set_base(stat::MAXDURABILITY, 0, 0);
                    item.stats.set_base(stat::DURABILITY, 0, 0);
                }
            }
        }
        DBC0 => {
            charges(t, item, ctx, rec, s);
        }
    }
    Ok(())
}
