//! Properties to stats: the property modes (affix, quality row, gem,
//! rune, unique, set, runeword, craft), the dispatcher, the property
//! functions ([`FUNCS`], checked against
//! `tables/items/property-functions.tsv` by a test), socket fillers,
//! runewords, set bonuses and craft property lists. All value draws use
//! the item seed. A format-0 item's records go to the legacy table instead
//! ([`super::props_legacy`]).
//!
//! A mod game's property functions (`docs/plans/mods/seams.md` MS7a,
//! [`super::provider`]) are asked first for the slots whose function they
//! answer; see [`apply_property`].

use super::create::{apply_ethereal, has_durability, max_sockets};
use super::provider::{PropCall, PropHost};
use super::tables::{ItemTables, PropRec};
use super::{flag, q, stat, ty, Item, ItemStats, ListKey, LIST_FLAGS};
use crate::rng::Seed;

/// One row of `property-functions.tsv`, as the code reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuncRow {
    pub func: u8,
    pub address: &'static str,
    pub value: &'static str,
    pub draws: &'static str,
    pub reset_base: &'static str,
    pub writes: &'static str,
    pub layer: &'static str,
    pub returns: &'static str,
}

macro_rules! rows {
    ($(($f:expr, $a:expr, $v:expr, $d:expr, $r:expr, $w:expr, $l:expr, $ret:expr)),* $(,)?) => {
        &[$(FuncRow { func: $f, address: $a, value: $v, draws: $d, reset_base: $r, writes: $w, layer: $l, returns: $ret }),*]
    };
}

/// The function table, one row per non-empty entry, in the
/// TSV's order. The generic functions read `value`, `reset_base` and
/// `layer` from here; the test `funcs_match_tsv` keeps it equal to
/// `tables/items/property-functions.tsv`.
#[rustfmt::skip]
pub const FUNCS: &[FuncRow] = rows![
    (1, "0x0065EB30", "roll", "roll(min..max)", "mode1", "stat", "0", "added"),
    (2, "0x0065EBA0", "roll", "roll(min..max)", "always", "stat", "0", "added"),
    (3, "0x0065EC10", "prev_or_roll", "roll(min..max) if prev=0", "mode1", "stat", "0", "added"),
    (4, "0x0065EC80", "prev_or_roll", "roll(min..max) if prev=0", "always", "stat", "0", "added"),
    (5, "0x0065ECF0", "prev_or_roll", "roll(min..max) if prev=0", "no", "21,23,159", "0", "value"),
    (6, "0x0065EE80", "prev_or_roll", "roll(min..max) if prev=0", "no", "22,24,160", "0", "value"),
    (7, "0x0065F010", "prev_or_roll", "roll(min..max) if prev=0", "damage", "18,17 or func6(1)", "0", "value"),
    (8, "0x0065F2B0", "prev_or_roll", "roll(min..max) if prev=0", "no", "stat", "0", "added"),
    (9, "0x0065F310", "prev_or_roll", "roll(min..max) if prev=0", "no", "stat", "param", "added"),
    (10, "0x0065F3F0", "prev_or_roll", "roll(min..max) if prev=0", "no", "stat", "param%3+param/3*8", "added"),
    (11, "0x0065F470", "min", "none", "no", "stat", "param*64+level", "added"),
    (12, "0x0065FC40", "param", "roll(min..max) for layer", "no", "stat", "rolled", "added"),
    (13, "0x0065FC90", "roll", "roll(min..max)", "mode1", "stat,72", "0", "added"),
    (14, "0x0065F590", "prev_or_roll_or_param", "roll(min..max) if prev<1", "no", "194 (set)", "0", "count"),
    (15, "0x0065F960", "min", "none", "no", "stat or func5", "0", "min"),
    (16, "0x0065F9D0", "max", "none", "no", "stat or func6", "0", "max"),
    (17, "0x0065FA40", "param_or_roll", "roll(min..max) if param=0", "no", "stat or func6", "0", "value"),
    (18, "0x0065F870", "packed", "none", "no", "stat (set)", "0", "max+256"),
    (19, "0x0065F6A0", "packed", "roll(charges-charges/8)", "no", "stat (set)", "skill<<shift+level", "charges"),
    (20, "0x0065FAE0", "1", "none", "no", "152 (add)", "0", "1"),
    (21, "0x0065FB50", "roll", "roll(min..max)", "no", "stat", "val", "added"),
    (22, "0x0065FBF0", "roll", "roll(min..max)", "no", "stat", "param (u16)", "added"),
    (23, "0x0065FD20", "-", "none", "no", "ethereal", "-", "1 or 0"),
    (24, "0x0065F390", "prev_or_roll", "roll(min..max) if prev=0", "no", "stat", "param", "added"),
    (36, "0x0065FBA0", "val", "roll(min..max) for layer", "no", "stat", "rolled", "added"),
];

fn func_row(f: u8) -> Option<&'static FuncRow> {
    FUNCS.iter().find(|r| r.func == f)
}

/// Mode numbers.
pub mod mode {
    pub const AFFIX: u8 = 0;
    pub const QUALITY: u8 = 1;
    pub const GEM: u8 = 2;
    pub const UNIQUE: u8 = 3;
    pub const SET: u8 = 4;
    pub const RUNE: u8 = 5;
    pub const RUNEWORD: u8 = 6;
    pub const CRAFT: u8 = 7;
}

/// Runeword list state.
pub const STATE_RUNEWORD: u16 = 171;
/// First set partial state (`itemset1`).
pub const STATE_ITEMSET1: u16 = 165;
/// Flags of the set partial lists.
pub const FLAGS_ITEMSET: u32 = 0x2040;

/// The dispatcher's context: mode, target list and an optional foreign
/// owner (a unit other than the item, e.g. the player for set bonuses).
/// With no owner the item's own lists receive the stats; an owner
/// that is the item itself (runewords) is the same as none.
pub struct PropCtx<'o> {
    pub mode: u8,
    pub list: ListKey,
    pub owner: Option<&'o mut dyn ItemStats>,
    /// The dispatcher's extra unit when it is an item: the socketed item
    /// of a rune's mode 5; read by the legacy function.
    pub extra: Option<&'o mut dyn ItemStats>,
}

impl PropCtx<'_> {
    /// Mode `mode`, the item's list (state 0, flags 0x40), no owner.
    pub fn item(mode: u8) -> PropCtx<'static> {
        PropCtx {
            mode,
            list: ListKey::ITEM,
            owner: None,
            extra: None,
        }
    }
}

/// `min` if `max = min`, else (swapped if `max < min`) `min +
/// roll(max − min + 1)` on `seed`.
pub fn roll_value(seed: &mut Seed, min: i32, max: i32) -> i32 {
    if max == min {
        return min;
    }
    let (lo, hi) = if max < min { (max, min) } else { (min, max) };
    (seed.roll(hi.wrapping_sub(lo).wrapping_add(1)) as i32).wrapping_add(lo)
}

/// Runs one property record: for a format-0 item in every mode except
/// runewords the legacy table, otherwise each slot of the record's
/// `properties` row in order. A slot runs a mod game's property function
/// when the game's rules have one for its id (asked first, also for ids
/// past D2RUST's), else D2RUST's; the first slot with a function neither
/// knows ends the row. Slot 0's result is the `prev` the later slots see.
/// A legacy fatal is kept in [`Item::fatal`] and stops every later
/// record.
pub fn apply_property<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    rec: &PropRec,
) {
    if item.fatal.is_some() {
        return;
    }
    if item.format < 1 && ctx.mode != mode::RUNEWORD {
        // Craft lists (mode 7) take this path too. The legacy table's
        // last argument is 0 for every caller.
        if let Err(e) = super::props_legacy::apply(t, item, ctx, rec, 0) {
            item.fatal = Some(e);
        }
        return;
    }
    let Some(row) = usize::try_from(rec.code)
        .ok()
        .and_then(|c| t.properties.get(c))
    else {
        return;
    };
    let slots = row.slots;
    let provider = t.rules.items();
    let mut prev = 0;
    for (k, s) in slots.iter().enumerate() {
        let modded = provider.filter(|p| p.has_property_fn(s.func));
        if modded.is_none() && func_row(s.func).is_none() {
            break;
        }
        let set = s.set != 0;
        let by_mod = modded.and_then(|p| {
            let call = PropCall {
                func: s.func,
                mode: ctx.mode,
                rec: *rec,
                set,
                stat: s.stat,
                val: s.val,
                prev,
            };
            p.property_fn(&mut PropView { t, item, ctx }, &call)
        });
        let r = match by_mod {
            Some(r) => r,
            None => call(t, item, ctx, rec, s.func, set, s.stat, s.val, prev),
        };
        if k == 0 {
            prev = r;
        }
    }
}

/// An item under a property record, as a mod game's property function
/// sees it ([`PropHost`]).
struct PropView<'a, 'o, S> {
    t: &'a ItemTables,
    item: &'a mut Item<S>,
    ctx: &'a mut PropCtx<'o>,
}

impl<S: ItemStats> PropHost for PropView<'_, '_, S> {
    fn tables(&self) -> &ItemTables {
        self.t
    }
    fn record(&self) -> usize {
        self.item.record
    }
    fn item_level(&mut self) -> i32 {
        self.item.item_level()
    }
    fn quality(&self) -> u8 {
        self.item.quality
    }
    fn roll(&mut self, min: i32, max: i32) -> i32 {
        roll_value(&mut self.item.item_seed, min, max)
    }
    fn add_stat(&mut self, set: bool, stat: u16, layer: u16, value: i32) -> i32 {
        add_stat(self.t, self.item, self.ctx, set, stat, layer, value)
    }
    fn item_stats(&mut self) -> &mut dyn ItemStats {
        &mut self.item.stats
    }
}

/// Add (or set) `value << valshift` in the target list; returns
/// `value`, or 0 when nothing was written.
#[allow(clippy::too_many_arguments)]
fn add_stat<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    set: bool,
    id: u16,
    layer: u16,
    value: i32,
) -> i32 {
    if value == 0 || !t.stat_valid(id) {
        return 0;
    }
    let v = value.wrapping_shl(u32::from(t.valshift[usize::from(id)]));
    let key = ctx.list;
    let target: &mut dyn ItemStats = match ctx.owner.as_deref_mut() {
        Some(o) => o,
        None => &mut item.stats,
    };
    if set {
        target.list_set(key, id, layer, v);
        if id == stat::POISONMAXDAM && target.list_get(key, stat::POISON_COUNT, 0) == 0 {
            target.list_set(key, stat::POISON_COUNT, 0, 1);
        }
    } else {
        target.list_add(key, id, layer, v);
        if id == stat::POISONMAXDAM {
            target.list_add(key, stat::POISON_COUNT, 0, 1);
        }
    }
    value
}

/// Functions 18 and 19: the owner-or-item target list,
/// written with the plain list set: no `valshift`, no itemstatcost range
/// test, no stat-58 rule, and a value 0 is set like any other.
pub(super) fn set_raw<S: ItemStats>(
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    id: u16,
    layer: u16,
    v: i32,
) {
    let key = ctx.list;
    let target: &mut dyn ItemStats = match ctx.owner.as_deref_mut() {
        Some(o) => o,
        None => &mut item.stats,
    };
    target.list_set(key, id, layer, v);
}

/// Base reset for a slot's stat.
pub(super) fn base_reset<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, id: u16) {
    let Some(r) = t.item(item.record).cloned() else {
        return;
    };
    let throwable = t.itype_of(item.record).is_some_and(|it| it.throwable != 0);
    let weapon = t.is_type(item.record, ty::WEAP as i16);
    let st = &mut item.stats;
    match id {
        stat::ARMOR_PERCENT | stat::ARMORCLASS => {
            if t.is_type(item.record, ty::ARMO as i16) && r.maxac != 0 {
                let b = st.base(stat::ARMORCLASS, 0).wrapping_add(1);
                st.set_base(stat::ARMORCLASS, 0, b.max((r.maxac as i32).wrapping_add(1)));
            }
        }
        stat::MAXDAMAGE_PERCENT | stat::MAXDAMAGE if weapon => {
            for (s, v, go) in [
                (stat::MAXDAMAGE, r.maxdam, true),
                (stat::SECONDARY_MAXDAMAGE, r.maxdam2, true),
                (stat::THROW_MAXDAMAGE, r.maxmisdam, throwable),
            ] {
                if go && v != 0 {
                    st.set_base(s, 0, i32::from(v));
                }
            }
        }
        stat::MINDAMAGE_PERCENT | stat::MINDAMAGE if weapon => {
            for (s, v, go) in [
                (stat::MINDAMAGE, r.mindam, true),
                (stat::SECONDARY_MINDAMAGE, r.mindam2, true),
                (stat::THROW_MINDAMAGE, r.minmisdam, throwable),
            ] {
                if go && v != 0 {
                    st.set_base(s, 0, i32::from(v));
                }
            }
        }
        _ => {}
    }
}

/// Skill level from a record's `max`.
fn skill_level(ilvl: i32, reqlevel: i32, maxlvl: i32, max: i32) -> i32 {
    if max > 0 {
        max
    } else if max == 0 {
        let cap = if maxlvl < 1 { 20 } else { maxlvl };
        ((ilvl - reqlevel) / 4 + 1).max(1).min(cap)
    } else {
        let s = (99 - reqlevel).max(1);
        let d = (-(s / max)).max(1);
        let l = (ilvl - reqlevel) / d;
        if l <= 0 {
            1
        } else {
            l
        }
    }
}

/// Function 5 (`up` = false) or 6 (`up` = true): min or max damage.
fn damage<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    rec: &PropRec,
    set: bool,
    prev: i32,
    up: bool,
) -> i32 {
    let v = if prev != 0 {
        prev
    } else {
        roll_value(&mut item.item_seed, rec.min, rec.max)
    };
    // The owner is never an item here (an item owner is the item itself).
    let Some(r) = t.item(item.record).cloned() else {
        return v;
    };
    let weapon = t.is_type(item.record, ty::WEAP as i16);
    let throwable = t.itype_of(item.record).is_some_and(|it| it.throwable != 0);
    let (one, two, mis) = if up {
        (r.maxdam, r.maxdam2, r.maxmisdam)
    } else {
        (r.mindam, r.mindam2, r.minmisdam)
    };
    let (s1, s2, s3) = if up {
        (
            stat::MAXDAMAGE,
            stat::SECONDARY_MAXDAMAGE,
            stat::THROW_MAXDAMAGE,
        )
    } else {
        (
            stat::MINDAMAGE,
            stat::SECONDARY_MINDAMAGE,
            stat::THROW_MINDAMAGE,
        )
    };
    let parts = [
        (s1, one, !(weapon && one == 0 && two != 0)),
        (s2, two, !(weapon && two == 0 && one != 0)),
        (s3, mis, !(weapon && !throwable)),
    ];
    for (s, col, go) in parts {
        if !go {
            continue;
        }
        let col = i32::from(col);
        let mut amount = v;
        if col != 0 && col + v < 1 {
            amount = if up { -col } else { 1 - col };
        }
        if amount != 0 {
            add_stat(t, item, ctx, set, s, 0, amount);
        }
    }
    v
}

/// The skill of functions 11 and 19:
/// `param`, or 0 when it is outside the skills table.
pub(super) fn skill_or_zero(t: &ItemTables, param: i32) -> usize {
    usize::try_from(param)
        .ok()
        .filter(|&s| s < t.skills.len())
        .unwrap_or(0)
}

/// One property function. Returns the function's value.
#[allow(clippy::too_many_arguments)]
fn call<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    rec: &PropRec,
    f: u8,
    set: bool,
    id: u16,
    val: u16,
    prev: i32,
) -> i32 {
    let Some(row) = func_row(f) else { return 0 };
    let ilvl = item.item_level();
    match f {
        5 | 6 => damage(t, item, ctx, rec, set, prev, f == 6),
        7 => {
            let v = if prev != 0 {
                prev
            } else {
                roll_value(&mut item.item_seed, rec.min, rec.max)
            };
            // The function's target is always the item here, also with a
            // foreign owner (the player, for set bonuses), so its
            // non-item branch (add 18 / 17, return 1) is never taken.
            base_reset(t, item, stat::MAXDAMAGE_PERCENT);
            base_reset(t, item, stat::MINDAMAGE_PERCENT);
            let b = t
                .item(item.record)
                .map_or(0, |r| i64::from(r.maxdam.max(r.maxdam2)));
            if t.is_type(item.record, ty::WEAP as i16) && b * i64::from(v) / 100 <= 0 {
                return damage(t, item, ctx, rec, set, 1, true);
            }
            add_stat(t, item, ctx, set, stat::MINDAMAGE_PERCENT, 0, v);
            add_stat(t, item, ctx, set, stat::MAXDAMAGE_PERCENT, 0, v);
            v
        }
        11 => {
            // `param` outside the skills table → skill 0.
            let skill = skill_or_zero(t, rec.param);
            let Some(sk) = t.skills.get(skill).copied() else {
                return 0;
            };
            let chance = if rec.min < 1 { 5 } else { rec.min };
            let level = skill_level(ilvl, sk.reqlevel, sk.maxlvl, rec.max);
            let layer = (skill as u16)
                .wrapping_mul(64)
                .wrapping_add((level & 63) as u16);
            add_stat(t, item, ctx, set, id, layer, chance)
        }
        12 | 36 => {
            let layer = roll_value(&mut item.item_seed, rec.min, rec.max) as u16;
            let value = if f == 12 { rec.param } else { i32::from(val) };
            add_stat(t, item, ctx, set, id, layer, value)
        }
        13 => {
            let v = roll_value(&mut item.item_seed, rec.min, rec.max);
            if ctx.mode == 1 {
                base_reset(t, item, id);
            }
            let added = add_stat(t, item, ctx, set, id, 0, v);
            if added != 0 {
                let m = item.stats.stat(stat::MAXDURABILITY, 0);
                if m > 0 {
                    item.stats.set_base(stat::DURABILITY, 0, m);
                }
            }
            added
        }
        14 => sockets(t, item, rec, prev),
        15..=17 => {
            let v = match f {
                15 => rec.min,
                16 => rec.max,
                _ => {
                    let v = if rec.param != 0 {
                        rec.param
                    } else {
                        roll_value(&mut item.item_seed, rec.min, rec.max)
                    };
                    if v == 0 {
                        return 0;
                    }
                    v
                }
            };
            if f == 15 && id == stat::MINDAMAGE {
                damage(t, item, ctx, rec, set, v, false);
            } else if f != 15 && id == stat::MAXDAMAGE {
                damage(t, item, ctx, rec, set, v, true);
            } else {
                add_stat(t, item, ctx, set, id, 0, v);
            }
            v
        }
        18 => {
            let p = rec.param.clamp(0, 3);
            let a = rec.min.wrapping_add(256).clamp(0, 1023);
            let b = rec.max.wrapping_add(256).clamp(0, 1023);
            // The plain list set, not the stat writer (no valshift, no
            // range test, no stat-58 rule; a 0 is written).
            set_raw(item, ctx, id, 0, p + (b * 1024 + a) * 4);
            b
        }
        19 => charges(t, item, ctx, rec, id),
        20 => {
            if t.valshift.len() > usize::from(stat::INDESTRUCTIBLE) {
                add_stat(t, item, ctx, false, stat::INDESTRUCTIBLE, 0, 1);
            }
            1
        }
        23 => {
            if item.flags & flag::ETHEREAL == 0 && has_durability(t, item) {
                apply_ethereal(t, item);
                1
            } else {
                0
            }
        }
        _ => {
            // Generic stat functions 1–4, 8–10, 21, 22, 24: value, reset
            // and layer per the table.
            let v = match row.value {
                "prev_or_roll" if prev != 0 => prev,
                _ => roll_value(&mut item.item_seed, rec.min, rec.max),
            };
            if row.reset_base == "always" || (row.reset_base == "mode1" && ctx.mode == 1) {
                base_reset(t, item, id);
            }
            let p = rec.param;
            let layer = match row.layer {
                "param" | "param (u16)" => p as u16,
                "param%3+param/3*8" => (p % 3 + p / 3 * 8) as u16,
                "val" => val,
                _ => 0,
            };
            add_stat(t, item, ctx, set, id, layer, v)
        }
    }
}

/// Function 19 (also the legacy table's): charges
/// of skill `param` into stat `id`. Returns c (0: no skills row).
pub(super) fn charges<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    rec: &PropRec,
    id: u16,
) -> i32 {
    let ilvl = item.item_level();
    // An invalid `param` → skill 0.
    let skill = skill_or_zero(t, rec.param);
    let Some(sk) = t.skills.get(skill).copied() else {
        return 0;
    };
    let level = skill_level(ilvl, sk.reqlevel, sk.maxlvl, rec.max);
    let mut c = rec.min;
    if c == 0 {
        c = 5;
    } else if c < 0 {
        c = -c + (-c * level) / 8;
    }
    if c <= 1 {
        c = 1;
    } else if c > 254 {
        c = 255;
    }
    let r = item.item_seed.roll(c - c / 8) as i32;
    let layer = ((skill as u32) << t.stat_shift).wrapping_add(level as u32 & t.stat_mask);
    // Written like function 18: the plain list set.
    set_raw(
        item,
        ctx,
        id,
        layer as u16,
        c * 256 + ((r + c / 8 + 1) & 0xFF),
    );
    c
}

/// Function 14: sockets, set directly on the item.
fn sockets<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rec: &PropRec, prev: i32) -> i32 {
    let Some(r) = t.item(item.record) else {
        return 0;
    };
    let mut cap = (i32::from(r.invwidth) * i32::from(r.invheight)).min(6);
    if cap == 0 {
        return 0;
    }
    cap = cap.min(max_sockets(t, item));
    let mut n = if prev >= 1 {
        prev
    } else {
        roll_value(&mut item.item_seed, rec.min, rec.max)
    };
    if n < 1 {
        n = rec.param;
    }
    if cap < 1 {
        // A cap below 1 leaves the socketed flag and the stat untouched.
        return 0;
    }
    let result = n.max(1).min(cap);
    item.flags |= flag::SOCKETED;
    item.stats.set_base(stat::NUMSOCKETS, 0, result);
    result
}

/// Mode 0: a magic affix row (`id` = combined index + 1).
pub fn apply_affix<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, id: u16) {
    let Some(row) = usize::from(id).checked_sub(1).and_then(|i| t.magic.get(i)) else {
        return;
    };
    let mods = row.mods;
    run_until_none(t, item, &mut PropCtx::item(mode::AFFIX), &mods);
}

/// Mode 1: a qualityitems row.
pub fn apply_quality_row<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, row: usize) {
    let Some(r) = t.qualityitems.get(row) else {
        return;
    };
    let mods = r.mods;
    run_until_none(t, item, &mut PropCtx::item(mode::QUALITY), &mods);
}

fn run_until_none<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    ctx: &mut PropCtx,
    recs: &[PropRec],
) {
    for rec in recs {
        if rec.code < 0 {
            break;
        }
        apply_property(t, item, ctx, rec);
    }
}

/// Mode 3: the uniqueitems row = file index.
pub fn apply_unique<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) {
    let Some(r) = usize::try_from(item.file_index)
        .ok()
        .and_then(|i| t.uniques.get(i))
    else {
        return;
    };
    let props = r.props;
    let mut ctx = PropCtx::item(mode::UNIQUE);
    for rec in props.iter().filter(|r| r.code >= 0) {
        apply_property(t, item, &mut ctx, rec);
    }
}

/// Mode 4: the setitems row = file index. A format-0 item runs
/// only `prop1`–`prop2` and no partial records.
pub fn apply_set_item<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) {
    let Some(r) = usize::try_from(item.file_index)
        .ok()
        .and_then(|i| t.setitems.get(i))
    else {
        return;
    };
    let (props, aprops, add_func) = (r.props, r.aprops, r.add_func);
    let mut ctx = PropCtx::item(mode::SET);
    let n_props = if item.format < 1 { 2 } else { props.len() };
    for rec in props.iter().take(n_props).filter(|r| r.code >= 0) {
        apply_property(t, item, &mut ctx, rec);
    }
    if item.format < 1 {
        return;
    }
    for (k, rec) in aprops.iter().enumerate() {
        if rec.code < 0 {
            continue;
        }
        ctx.list = if add_func != 0 {
            ListKey {
                state: STATE_ITEMSET1 + (k / 2) as u16,
                flags: FLAGS_ITEMSET,
            }
        } else {
            ListKey::ITEM
        };
        apply_property(t, item, &mut ctx, rec);
    }
}

/// Modes 2 / 5 on a socket filler: its gems row and property set
/// (the socketed item's `gemapplytype`). Properties land in the filler's
/// own list. A quality-5 filler of another type adds nothing.
pub fn apply_socket_filler<S: ItemStats>(t: &ItemTables, filler: &mut Item<S>, apply_type: u8) {
    apply_socket_filler_into(t, filler, apply_type, None);
}

/// [`apply_socket_filler`] with the socketed item's stats: a rune's
/// mode 5 passes it as the extra unit.
pub fn apply_socket_filler_into<S: ItemStats>(
    t: &ItemTables,
    filler: &mut Item<S>,
    apply_type: u8,
    socketed: Option<&mut dyn ItemStats>,
) {
    let m = if t.is_type(filler.record, ty::GEM as i16) {
        mode::GEM
    } else if t.is_type(filler.record, ty::RUNE as i16) {
        mode::RUNE
    } else {
        // A quality-5 filler of another type adds nothing.
        return;
    };
    let Some(r) = t.item(filler.record) else {
        return;
    };
    let Some(gem) = usize::try_from(r.gemoffset)
        .ok()
        .and_then(|g| t.gems.get(g))
    else {
        return;
    };
    let Some(block) = gem.mods.get(usize::from(apply_type)).copied() else {
        return;
    };
    let mut ctx = PropCtx {
        mode: m,
        list: ListKey::ITEM,
        owner: None,
        extra: if m == mode::RUNE { socketed } else { None },
    };
    run_until_none(t, filler, &mut ctx, &block);
}

/// The first runes row the item and its fillers (items combined
/// indices, insertion order) match. An item without an inventory (items
/// `hasinv` = 0: none is ever created for it) has no runeword.
pub fn runeword_match<S: ItemStats>(
    t: &ItemTables,
    item: &Item<S>,
    fillers: &[usize],
) -> Option<usize> {
    if t.item(item.record)?.hasinv == 0 {
        return None;
    }
    let sockets = item.stats.stat(stat::NUMSOCKETS, 0) as u8;
    runeword_row(t, item.record, item.quality, sockets, fillers)
}

/// Exact form on the item's parts: items class
/// `record`, quality, the socket count (u8) and the class ids of the items in the item's own inventory, in
/// list order (empty: no inventory or an empty one, step 1).
pub fn runeword_row(
    t: &ItemTables,
    record: usize,
    quality: u8,
    sockets: u8,
    fillers: &[usize],
) -> Option<usize> {
    let r = t.item(record)?;
    if (q::MAGIC..=q::TEMPERED).contains(&quality) || r.quest != 0 || fillers.is_empty() {
        return None;
    }
    // Step 3: c ≠ the socket count (more than 6 fillers never matches a
    // u8 count of a 6-socket item either; the rune slots stop at 6).
    if fillers.len() > 6 || usize::from(sockets) != fillers.len() {
        return None;
    }
    let n = fillers.len();
    t.runes.iter().position(|w| {
        if w.complete == 0 {
            return false;
        }
        // Step 4: rune i + 1 is compared with class-id slot i; i ≥ c
        // reads a slot step 2 never wrote, read as "no class" (no
        // match).
        let mut count = 0;
        for &rune in w.runes.iter().take_while(|&&x| x >= 1) {
            if count >= n || fillers[count] as i32 != rune {
                return false;
            }
            count += 1;
        }
        if count < n {
            return false;
        }
        let is = |x: i16| t.is_type(record, x);
        if w.etype.iter().take_while(|&&e| e >= 1).any(|&e| is(e)) {
            return false;
        }
        w.itype.iter().take_while(|&&e| e >= 1).any(|&e| is(e))
    })
}

/// Activates runes row `row` with the filler just inserted as I
/// (`filler`: its item seed draws every roll, its items row every
/// Reset) and the socketed item's stats as O (`socketed`: its
/// state-171 list, flags 0x40, receives the stats). `ladder` is game
/// +0x74. Returns whether the runeword properties ran; the caller then
/// sets item flag 0x4000000 ([`flag::RUNEWORD`]) on the socketed item
/// and re-runs its replenish timers ([`super::replenish_timer`]).
pub fn activate_runeword<S: ItemStats>(
    t: &ItemTables,
    filler: &mut Item<S>,
    socketed: &mut dyn ItemStats,
    row: usize,
    ladder: bool,
) -> bool {
    let Some(w) = t.runes.get(row) else {
        return false;
    };
    if w.server != 0 && !ladder {
        return false;
    }
    let key = ListKey {
        state: STATE_RUNEWORD,
        flags: LIST_FLAGS,
    };
    if socketed.has_list(key) {
        return false;
    }
    let props = w.props;
    let mut ctx = PropCtx {
        mode: mode::RUNEWORD,
        list: key,
        owner: Some(socketed),
        extra: None,
    };
    run_until_none(t, filler, &mut ctx, &props);
    true
}

/// Popcount of a set mask (masks ≥ 64 → 0).
pub fn set_mask_count(mask: u32) -> u32 {
    if mask >= 64 {
        0
    } else {
        mask.count_ones()
    }
}

/// Set bonuses of the equipped set item `item` (quality 5) with
/// the owner's set mask (computed by the caller's equip logic) into the
/// owner's list `list` (state s). The caller picks the bonus list's
/// flags in `list`.
pub fn set_bonuses<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    mask: u32,
    owner: &mut dyn ItemStats,
    list: ListKey,
) {
    let Some(si) = usize::try_from(item.file_index)
        .ok()
        .and_then(|i| t.setitems.get(i))
    else {
        return;
    };
    let Some(set) = usize::try_from(si.set).ok().and_then(|s| t.sets.get(s)) else {
        return;
    };
    let (count, partial, full) = (set.count, set.partial, set.full);
    let c = set_mask_count(mask) as i32;
    let n = c.min(count - 1);
    let take = (2 * n - 2).max(0) as usize;
    let mut ctx = PropCtx {
        mode: mode::SET,
        list,
        owner: Some(owner),
        extra: None,
    };
    for rec in partial.iter().take(take).filter(|r| r.code >= 0) {
        apply_property(t, item, &mut ctx, rec);
    }
    if c >= count {
        run_until_none(t, item, &mut ctx, &full);
    }
}

/// A craft property list (mode 7), then ethereal re-applied when the
/// item is ethereal.
pub fn apply_craft_list<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, recs: &[PropRec]) {
    let mut ctx = PropCtx::item(mode::CRAFT);
    for rec in recs {
        apply_property(t, item, &mut ctx, rec);
    }
    if item.flags & flag::ETHEREAL != 0 {
        apply_ethereal(t, item);
    }
}
