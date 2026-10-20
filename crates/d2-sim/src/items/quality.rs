//! The itemratio quality roll, the quality dispatch with its
//! overrides, downgrade chain and finishing steps, and the
//! low-quality, superior, unique and set routines, with their
//! format-0 branches (legacy items of version-0x47 saves).

use super::affixes;
use super::create::{
    class_skill_mods, ethereal_roll, has_durability, normal_by_format, socket_roll,
};
use super::props::{apply_property, apply_quality_row, apply_set_item, apply_unique, PropCtx};
use super::provider::GenHost;
use super::tables::{ItemTables, PropRec};
use super::{flag, q, req, stat, ty, Fatal, Item, ItemGame, ItemRequest, ItemStats, UniqueBits};
use crate::rng::Seed;

/// The itemratio order.
pub const RATIO_ORDER: [u8; 6] = [q::UNIQUE, q::RARE, q::SET, q::MAGIC, q::SUPERIOR, q::NORMAL];

/// The hellbovine set.
pub const HELLBOVINE_SET: i16 = 29;

/// The itemratio row with
/// the version limit `limit`.
pub fn ratio_row(t: &ItemTables, item: usize, limit: u16) -> Option<usize> {
    let r = t.item(item)?;
    let class_specific = t.itype_of(item).is_some_and(|it| it.class < 7);
    let uber = (t.is_type(item, ty::WEAP as i16) || t.is_type(item, ty::ARMO as i16))
        && (r.code == r.ubercode || r.code == r.ultracode)
        && r.type_ != 38
        && r.quest == 0;
    let mut best: Option<(usize, u16)> = None;
    for (i, row) in t.itemratio.iter().enumerate() {
        if (row.class_specific != 0) == class_specific
            && (row.uber != 0) == uber
            && row.version <= limit
            && best.is_none_or(|(_, v)| row.version >= v)
        {
            best = Some((i, row.version));
        }
    }
    best.map(|(i, _)| i)
}

/// The quality roll on the itemratio rows (format 0: the legacy roll).
pub fn roll_quality<S>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> Result<u8, Fatal> {
    if item.format >= 1 && rq.quality != q::NONE {
        return Ok(rq.quality);
    }
    let limit = if item.format == 0 { 0 } else { 100 };
    let row = ratio_row(t, item.record, limit).ok_or(Fatal::NoRatioRow)?;
    let rec = t.item(item.record).ok_or(Fatal::NoItemRecord)?;
    if rec.quest != 0 {
        return Ok(q::NORMAL);
    }
    if item.format == 0 {
        return roll_quality_legacy(t, item, rq, row);
    }
    let mut l = rq.ilvl;
    if t.is_type(item.record, ty::MISC as i16) {
        l = 1;
    } else {
        l = (l - i32::from(rec.level)).max(1);
    }
    let rr = &t.itemratio[row];
    let cols = [
        (rr.unique, rr.uniquedivisor),
        (rr.rare, rr.raredivisor),
        (rr.set, rr.setdivisor),
        (rr.magic, rr.magicdivisor),
        (rr.hiquality, rr.hiqualitydivisor),
        (rr.normal, rr.normaldivisor),
    ];
    for (&quality, (base, div)) in RATIO_ORDER.iter().zip(cols) {
        let d = l.checked_div(div as i32).ok_or(Fatal::DivideByZero)?;
        let c = (base as i32).wrapping_sub(d);
        if c < 1 || item.item_seed.roll(c) == 0 {
            return Ok(quality);
        }
    }
    Ok(if rq.flags2 & req::SUPERIOR != 0 {
        q::SUPERIOR
    } else {
        q::LOW
    })
}

/// The format-0 columns, after the ratio row and the `quest` test.
/// L is the request ilvl unadjusted; unique, rare and set have no
/// divisor; c < 1 becomes 1, so every quality is still drawn.
fn roll_quality_legacy<S>(
    t: &ItemTables,
    item: &mut Item<S>,
    rq: &ItemRequest,
    row: usize,
) -> Result<u8, Fatal> {
    let l = rq.ilvl;
    let rr = &t.itemratio[row];
    // (base, divisor); divisor None: no division. Divided in order, so a
    // divisor 0 is fatal only when its quality is reached.
    let cols = [
        (rr.unique, None),
        (rr.rare, None),
        (rr.set, None),
        (rr.magic, Some(rr.magicdivisor)),
        (rr.hiquality, Some(rr.hiqualitydivisor)),
        (rr.normal, Some(rr.normaldivisor)),
    ];
    for (&quality, (base, div)) in RATIO_ORDER.iter().zip(cols) {
        let d = match div {
            None => l,
            Some(d) => l.checked_div(d as i32).ok_or(Fatal::DivideByZero)?,
        };
        let c = (base as i32).wrapping_sub(d);
        if item.item_seed.roll(c.max(1)) == 0 {
            return Ok(quality);
        }
    }
    Ok(if rq.flags2 & req::SUPERIOR != 0 {
        q::SUPERIOR
    } else {
        q::LOW
    })
}

/// "Save": the item seed's low word.
fn save<S>(item: &Item<S>) -> u32 {
    item.item_seed.lo
}

/// `D2`: durability × `f` when the item has durability (format ≥ 1).
fn dur_factor<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, f: i32) {
    if item.format >= 1 && has_durability(t, item) {
        let d = item
            .stats
            .stat(stat::DURABILITY, 0)
            .wrapping_mul(f)
            .min(255);
        let m = item
            .stats
            .base(stat::MAXDURABILITY, 0)
            .wrapping_mul(f)
            .min(255);
        item.stats.set_base(stat::DURABILITY, 0, d);
        item.stats.set_base(stat::MAXDURABILITY, 0, m);
    }
}

/// Runs the routine of quality `x` (2, 3, 4 or 6 in a downgrade).
fn routine<S: ItemStats>(
    t: &ItemTables,
    game: &dyn ItemGame,
    item: &mut Item<S>,
    rq: &ItemRequest,
    x: u8,
) -> Result<bool, Fatal> {
    Ok(match x {
        q::SUPERIOR => superior(t, item, rq),
        q::MAGIC => affixes::magic(t, item, rq),
        q::RARE => affixes::rare(t, item, rq),
        _ => {
            normal_by_format(t, game, item, rq)?;
            true
        }
    })
}

/// Downgrade chain: D(x) for each x in `chain` while the previous
/// routine failed; `s` is the saved low word.
fn downgrade<S: ItemStats>(
    t: &ItemTables,
    game: &dyn ItemGame,
    item: &mut Item<S>,
    rq: &mut ItemRequest,
    s: u32,
    chain: &[u8],
) -> Result<(), Fatal> {
    let mut s = s;
    for &x in chain {
        item.clear();
        item.start_seed = s;
        item.item_seed = Seed::init_low(s);
        roll_quality(t, item, rq)?;
        item.quality = x;
        rq.quality = x;
        s = save(item);
        if routine(t, game, item, rq, x)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Quality dispatch. Returns the result (false: the
/// pipeline removes the item).
pub fn dispatch<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    item: &mut Item<S>,
    rq: &mut ItemRequest,
) -> Result<bool, Fatal> {
    let Some(rec) = t.item(item.record).cloned() else {
        return Ok(false);
    };
    item.auto_affix = 0;
    let fi = item.file_index;
    item.clear();
    item.file_index = fi;
    let mut quality = roll_quality(t, item, rq)?;
    if rq.quality != q::NONE {
        quality = rq.quality;
    }
    let it = t.itype_of(item.record).cloned();
    if let Some(it) = &it {
        if it.magic != 0 {
            if rec.quest != 0 {
                quality = q::UNIQUE;
            } else if !(q::MAGIC..=q::TEMPERED).contains(&quality) {
                quality = q::MAGIC;
            }
        }
        if it.rare == 0 && quality == q::RARE {
            quality = q::MAGIC;
        }
    }
    if rec.unique != 0 {
        quality = q::UNIQUE;
    }
    if it.as_ref().is_some_and(|it| it.normal != 0) {
        quality = q::NORMAL;
    }
    item.quality = quality;
    // A mod game's routine for the quality (MS7d) runs first.
    let provider = t.rules.items();
    let by_mod = provider
        .filter(|p| p.has_quality_fn(quality))
        .and_then(|p| p.quality_fn(&mut GenView { t, item }, quality, rq));
    match by_mod {
        Some(true) => {}
        Some(false) => return Ok(false),
        None => {
            if !routine_by_quality(t, game, item, rq, quality)? {
                return Ok(false);
            }
        }
    }
    // A legacy property function's fatal ends the creation.
    if let Some(f) = item.fatal {
        return Err(f);
    }
    // Finishing: D2RUST's qualities and a mod game's.
    let known = (q::LOW..=q::TEMPERED).contains(&item.quality)
        || provider.is_some_and(|p| p.has_quality_fn(item.quality));
    if !known {
        return Ok(false);
    }
    if item.format >= 100 {
        ethereal_roll(t, item, rq);
    }
    if matches!(item.quality, q::LOW | q::NORMAL | q::SUPERIOR) {
        socket_roll(t, game, item, rq);
    }
    if item.format >= 100
        && item.quality != q::SET
        && item.quality != q::UNIQUE
        && t.item(item.record).is_some_and(|r| r.auto_prefix != 0)
    {
        affixes::automagic(t, item);
    }
    Ok(true)
}

/// D2RUST's routine of `quality`, with its downgrades on a failure;
/// false for a quality D2RUST has none for.
fn routine_by_quality<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    item: &mut Item<S>,
    rq: &mut ItemRequest,
    quality: u8,
) -> Result<bool, Fatal> {
    match quality {
        q::LOW => {
            let s = save(item);
            if !low_quality(t, item, rq) {
                downgrade(t, game, item, rq, s, &[q::NORMAL])?;
            }
        }
        q::NORMAL => normal_by_format(t, game, item, rq)?,
        q::SUPERIOR => {
            item.file_index = -1;
            let s = save(item);
            if !superior(t, item, rq) {
                downgrade(t, game, item, rq, s, &[q::NORMAL])?;
            }
        }
        q::MAGIC => {
            let s = save(item);
            if !affixes::magic(t, item, rq) {
                downgrade(t, game, item, rq, s, &[q::SUPERIOR, q::NORMAL])?;
            }
        }
        q::SET => {
            item.file_index = -1;
            let s = save(item);
            if !set_item(t, item, rq) {
                dur_factor(t, item, 2);
                downgrade(t, game, item, rq, s, &[q::MAGIC, q::SUPERIOR, q::NORMAL])?;
            }
        }
        q::RARE => {
            item.rare_prefix = 0;
            item.rare_suffix = 0;
            let s = save(item);
            if !affixes::rare(t, item, rq) {
                downgrade(t, game, item, rq, s, &[q::MAGIC, q::SUPERIOR, q::NORMAL])?;
            }
        }
        q::UNIQUE => {
            item.file_index = -1;
            let s = save(item);
            if !unique(t, game, item, rq) {
                dur_factor(t, item, 3);
                downgrade(
                    t,
                    game,
                    item,
                    rq,
                    s,
                    &[q::RARE, q::MAGIC, q::SUPERIOR, q::NORMAL],
                )?;
            }
        }
        q::CRAFTED | q::TEMPERED => {
            item.rare_prefix = 0;
            item.rare_suffix = 0;
            let s = save(item);
            let ok = if quality == q::CRAFTED {
                affixes::crafted(t, item, rq)?
            } else {
                affixes::tempered(t, item)
            };
            if !ok {
                downgrade(t, game, item, rq, s, &[q::NORMAL])?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// A new item, as a mod game's quality routine sees it ([`GenHost`]).
struct GenView<'a, S> {
    t: &'a ItemTables,
    item: &'a mut Item<S>,
}

impl<S: ItemStats> GenHost for GenView<'_, S> {
    fn tables(&self) -> &ItemTables {
        self.t
    }
    fn record(&self) -> usize {
        self.item.record
    }
    fn item_level(&mut self) -> i32 {
        self.item.item_level()
    }
    fn roll(&mut self, n: i32) -> u32 {
        self.item.item_seed.roll(n)
    }
    fn set_file_index(&mut self, index: i32) {
        self.item.file_index = index;
    }
    fn apply_property(&mut self, mode: u8, rec: &PropRec) {
        apply_property(self.t, self.item, &mut PropCtx::item(mode), rec);
    }
    fn item_stats(&mut self) -> &mut dyn ItemStats {
        &mut self.item.stats
    }
}

/// Low quality (format 0: the legacy branch).
pub fn low_quality<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    if item.format < 1 {
        return low_quality_legacy(t, item);
    }
    item.file_index = item.item_seed.roll(t.n_lowquality as i32) as i32;
    if has_durability(t, item) {
        let d = t.item(item.record).map_or(0, |r| u32::from(r.durability));
        let m = (d * 33 / 100).max(1) as i32;
        let mut v = item.unit_seed.roll(m >> 1) as i32 + (m >> 1);
        if v == 0 {
            v = 1;
        }
        item.stats.set_base(stat::DURABILITY, 0, v);
        item.stats.set_base(stat::MAXDURABILITY, 0, m);
    }
    let scale = |item: &mut Item<S>, s: u16, floor: i32| {
        let v = (item.stats.stat(s, 0).wrapping_mul(75) / 100).max(floor);
        item.stats.set_base(s, 0, v);
    };
    let result = if t.is_type(item.record, ty::WEAP as i16) {
        scale(item, stat::MAXDAMAGE, 2);
        scale(item, stat::MINDAMAGE, 1);
        scale(item, stat::SECONDARY_MAXDAMAGE, 2);
        scale(item, stat::SECONDARY_MINDAMAGE, 1);
        if t.itype_of(item.record).is_some_and(|it| it.throwable != 0) {
            // Swapped bounds, reproduced (edge case 2).
            scale(item, stat::THROW_MINDAMAGE, 2);
            scale(item, stat::THROW_MAXDAMAGE, 1);
        }
        true
    } else if t.is_type(item.record, ty::ARMO as i16) {
        scale(item, stat::ARMORCLASS, 1);
        true
    } else {
        false
    };
    class_skill_mods(t, item, rq);
    result
}

/// Low quality, format 0: durability / 3, base
/// damage and defense (the throw stats from their totals), no class
/// skill mods.
fn low_quality_legacy<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) -> bool {
    // "No lowqualityitems table": D2RUST holds the table as a row count; an
    // empty one stands for none.
    let Some(d) = t.item(item.record).map(|r| u32::from(r.durability)) else {
        return false;
    };
    if t.n_lowquality == 0 {
        return false;
    }
    item.file_index = item.item_seed.roll(t.n_lowquality as i32) as i32;
    if has_durability(t, item) {
        let m = (d / 3).max(1) as i32;
        let mut v = item.unit_seed.roll(m >> 1) as i32 + (m >> 1);
        if v == 0 {
            v = 1;
        }
        item.stats.set_base(stat::DURABILITY, 0, v);
        item.stats.set_base(stat::MAXDURABILITY, 0, m);
    }
    let scale = |item: &mut Item<S>, s: u16, floor: i32, total: bool| {
        let cur = if total {
            item.stats.stat(s, 0)
        } else {
            item.stats.base(s, 0)
        };
        let v = (cur.wrapping_mul(75) / 100).max(floor);
        item.stats.set_base(s, 0, v);
    };
    if t.is_type(item.record, ty::WEAP as i16) {
        scale(item, stat::MAXDAMAGE, 2, false);
        scale(item, stat::MINDAMAGE, 1, false);
        scale(item, stat::SECONDARY_MAXDAMAGE, 2, false);
        scale(item, stat::SECONDARY_MINDAMAGE, 1, false);
        if t.itype_of(item.record).is_some_and(|it| it.throwable != 0) {
            // The swapped bounds of the throw damage, from the totals.
            scale(item, stat::THROW_MINDAMAGE, 2, true);
            scale(item, stat::THROW_MAXDAMAGE, 1, true);
        }
        true
    } else if t.is_type(item.record, ty::ARMO as i16) {
        scale(item, stat::ARMORCLASS, 1, false);
        true
    } else {
        false
    }
}

/// Superior row fits.
pub fn superior_fits<S>(t: &ItemTables, item: &Item<S>, row: usize) -> bool {
    let Some(r) = t.qualityitems.get(row) else {
        return false;
    };
    let tp = t.item(item.record).map_or(-1, |r| r.type_);
    let tp = |x: u16| tp == x as i16;
    (r.weapon != 0
        && t.is_type(item.record, ty::WEAP as i16)
        && ![ty::STAF, ty::BOW, ty::XBOW, ty::SCEP, ty::WAND]
            .into_iter()
            .any(tp))
        || (r.armor != 0
            && t.is_type(item.record, ty::ARMO as i16)
            && ![ty::SHIE, ty::BOOT, ty::GLOV, ty::BELT].into_iter().any(tp))
        || (tp(ty::SHIE) && r.shield != 0)
        || (tp(ty::SCEP) && r.scepter != 0)
        || (tp(ty::WAND) && r.wand != 0)
        || (tp(ty::STAF) && r.staff != 0)
        || ((tp(ty::BOW) || tp(ty::XBOW)) && r.bow != 0)
        || (tp(ty::BOOT) && r.boots != 0)
        || (tp(ty::GLOV) && r.gloves != 0)
        || (tp(ty::BELT) && r.belt != 0)
}

/// Superior.
pub fn superior<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    let mut n = t.qualityitems.len();
    let throwable = t.itype_of(item.record).is_some_and(|it| it.throwable != 0);
    if throwable || t.item(item.record).is_some_and(|r| r.nodurability != 0) {
        n = 4;
    }
    // The tried array has one entry per qualityitems row (at least one).
    let mut tried = vec![false; n.max(1)];
    loop {
        let r = item.item_seed.roll(n as i32) as usize;
        if tried[r] {
            continue;
        }
        if r >= t.qualityitems.len() {
            return false;
        }
        if superior_fits(t, item, r) {
            item.file_index = r as i32;
            apply_quality_row(t, item, r);
            class_skill_mods(t, item, rq);
            return true;
        }
        tried[r] = true;
        if tried.iter().take(n).all(|&b| b) {
            return false;
        }
    }
}

/// Unique-dropped marking.
fn mark_unique(bits: &mut UniqueBits, idx: u32, nolimit: bool, quest: bool) -> bool {
    if nolimit {
        return true;
    }
    if !quest && bits.get(idx) {
        return false;
    }
    if idx > UniqueBits::MAX {
        return false;
    }
    bits.set(idx);
    true
}

/// Unique (format 0: the legacy branch).
pub fn unique<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> bool {
    if item.format < 1 && has_durability(t, item) {
        // Durability × 5, before the pick.
        let d = item
            .stats
            .stat(stat::DURABILITY, 0)
            .wrapping_mul(5)
            .min(255);
        let m = item
            .stats
            .base(stat::MAXDURABILITY, 0)
            .wrapping_mul(5)
            .min(255);
        item.stats.set_base(stat::DURABILITY, 0, d);
        item.stats.set_base(stat::MAXDURABILITY, 0, m);
    }
    let Some(rec) = t.item(item.record).cloned() else {
        return false;
    };
    if rq.force {
        let Some(u) = usize::try_from(rq.index)
            .ok()
            .and_then(|i| t.uniques.get(i))
        else {
            return false;
        };
        item.file_index = rq.index;
        if u.code != rec.code {
            return true;
        }
        item.flags &= !flag::IDENTIFIED;
        apply_unique(t, item);
        return true;
    }
    let (l6a, l74) = game.ladder_flags();
    if item.format < 1 {
        // (format 0 comes only with a forced request): the first
        // fitting row; no `lvl` test, no
        // weights, no draw, no marking.
        let quest = rec.quest != 0;
        let pick = t.uniques.iter().enumerate().position(|(i, u)| {
            u.version < 100
                && u.enabled
                && (l6a || l74 || !u.ladder)
                && u.code == rec.code
                && (quest || !game.uniques().get(i as u32))
        });
        let Some(i) = pick else {
            return false;
        };
        item.file_index = i as i32;
        item.flags &= !flag::IDENTIFIED;
        apply_unique(t, item);
        return true;
    }
    let ilvl = item.item_level();
    let mut cands: Vec<(usize, u32)> = Vec::new();
    let mut total = 0u32;
    let mut preferred = None;
    for (i, u) in t.uniques.iter().enumerate() {
        if (u.version < 100 || item.format >= 100)
            && u.enabled
            && u.code == rec.code
            && (l6a || l74 || !u.ladder)
            && i32::from(u.lvl) <= ilvl
        {
            cands.push((i, total));
            total = total.wrapping_add(u.rarity.max(1));
            if rq.index != 0 && rq.index - 1 == i as i32 {
                preferred = Some(i);
            }
        }
    }
    if cands.is_empty() {
        if rec.unique != 0 {
            return true;
        }
        item.file_index = -1;
        return false;
    }
    let idx = preferred.unwrap_or_else(|| {
        let r = item.item_seed.roll_range(0, total as i32) as u32;
        let mut pick = cands[0].0;
        for &(i, start) in &cands[1..] {
            if r >= start {
                pick = i;
            } else {
                break;
            }
        }
        pick
    });
    let u = &t.uniques[idx];
    let quest = rec.quest != 0;
    if quest || (idx as u32 <= UniqueBits::MAX && !game.uniques().get(idx as u32)) {
        if u.code != rec.code {
            return false;
        }
        item.file_index = idx as i32;
        if mark_unique(game.uniques(), idx as u32, u.nolimit, quest) {
            item.flags &= !flag::IDENTIFIED;
            apply_unique(t, item);
            return true;
        }
    }
    item.file_index = -1;
    false
}

/// Set item (format 0: the legacy branch).
pub fn set_item<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    if item.format < 1 {
        return set_item_legacy(t, item);
    }
    let Some(code) = t.item(item.record).map(|r| r.code) else {
        return false;
    };
    let ilvl = item.item_level();
    let mut cands: Vec<(usize, u32)> = Vec::new();
    let mut total = 0u32;
    let mut preferred = None;
    for (i, s) in t.setitems.iter().enumerate() {
        if (s.version < 100 || item.format >= 100)
            && i32::from(s.lvl) <= ilvl
            && s.item == code
            && (s.set != HELLBOVINE_SET || rq.flags2 & req::HELLBOVINE != 0)
        {
            let w = s.rarity.max(1);
            cands.push((i, w));
            total = total.wrapping_add(w);
            if rq.index != 0 && rq.index - 1 == i as i32 {
                preferred = Some(i);
            }
        }
    }
    let row = match preferred {
        Some(i) => i,
        None => {
            if total == 0 {
                return false;
            }
            let mut r = item.item_seed.roll(total as i32);
            let mut pick = cands[cands.len() - 1].0;
            for &(i, w) in &cands {
                if r < w {
                    pick = i;
                    break;
                }
                r -= w;
            }
            pick
        }
    };
    item.file_index = row as i32;
    item.flags &= !flag::IDENTIFIED;
    apply_set_item(t, item);
    true
}

/// The setitems rows attached to sets row `set`: the rows of that set in
/// record order, at most 6, cut at the set's count.
fn set_members(t: &ItemTables, set: usize) -> impl Iterator<Item = usize> + '_ {
    let count = t.sets.get(set).map_or(0, |s| s.count.clamp(0, 6) as usize);
    t.setitems
        .iter()
        .enumerate()
        .filter(move |(_, si)| usize::try_from(si.set).ok() == Some(set))
        .map(|(i, _)| i)
        .take(6)
        .take(count)
}

/// Set item, format 0.
fn set_item_legacy<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) -> bool {
    let Some(code) = t.item(item.record).map(|r| r.code) else {
        return false;
    };
    let s0 = save(item);
    // Step 2: durability × 2, no durability test.
    let d = item
        .stats
        .stat(stat::DURABILITY, 0)
        .wrapping_mul(2)
        .min(255);
    let m = item
        .stats
        .base(stat::MAXDURABILITY, 0)
        .wrapping_mul(2)
        .min(255);
    item.stats.set_base(stat::DURABILITY, 0, d);
    item.stats.set_base(stat::MAXDURABILITY, 0, m);
    let n = t.sets.iter().take_while(|s| s.version < 100).count();
    let k = item.item_seed.roll(n as i32) as usize;
    let mut found = None;
    for j in 0..n {
        let set = (k + j) % n;
        if let Some(i) = set_members(t, set).find(|&i| t.setitems[i].item == code) {
            found = Some(i);
        }
    }
    let Some(row) = found else {
        item.item_seed = Seed::init_low(s0);
        return false;
    };
    item.file_index = row as i32;
    item.flags &= !flag::IDENTIFIED;
    apply_set_item(t, item);
    true
}
