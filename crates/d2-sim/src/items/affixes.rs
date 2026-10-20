//! Magic affix ids and slots, the affix level, the magic affix
//! roller and its fit tests, rare names and the magic, rare,
//! crafted, tempered, charm and automagic routines, with the
//! format-0 roller, rare names, rare and crafted routines. All
//! draws use the item seed.

use super::create::{class_skill_mods, max_sockets};
use super::props::apply_affix;
use super::tables::{AffixRec, ItemTables};
use super::{flag, q, ty, Fatal, Item, ItemRequest, ItemStats};

/// Rare affix count table.
pub const RARE_COUNTS: [u32; 8] = [3, 4, 4, 5, 5, 5, 6, 6];
/// Candidate list cap.
pub const MAX_CANDIDATES: usize = 511;
/// Crafted tries per affix.
pub const CRAFTED_TRIES: u32 = 252;

/// Which part of the magic affix array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Suffix,
    Prefix,
    Auto,
}

/// First combined index and row count of a part.
fn part(t: &ItemTables, p: Part) -> (usize, usize) {
    match p {
        Part::Suffix => (0, t.n_suffix),
        Part::Prefix => (t.n_suffix, t.n_prefix),
        Part::Auto => (t.first_auto(), t.magic.len() - t.first_auto()),
    }
}

/// The affix row of a magic id (combined index + 1).
pub fn affix(t: &ItemTables, id: u16) -> Option<&AffixRec> {
    usize::from(id).checked_sub(1).and_then(|i| t.magic.get(i))
}

/// Affix level from item level, qlvl and `magic lvl`.
pub fn alvl(ilvl: i32, qlvl: i32, magic_lvl: i32) -> i32 {
    let i = ilvl.max(qlvl);
    let a = if magic_lvl != 0 {
        i + magic_lvl
    } else {
        let h = qlvl / 2;
        if i < 99 - h {
            i - h
        } else {
            2 * i - 99
        }
    };
    a.clamp(1, 99)
}

/// "Item is any of `types`" over a list that stops at the first < 1.
fn any_type(t: &ItemTables, item: usize, types: &[i16]) -> bool {
    types
        .iter()
        .take_while(|&&x| x >= 1)
        .any(|&x| t.is_type(item, x))
}

/// Format < 100 stackable or throwable items fit no affix.
fn classic_excluded<S>(t: &ItemTables, item: &Item<S>) -> bool {
    item.format < 100
        && (t.item(item.record).is_some_and(|r| r.stackable != 0)
            || t.itype_of(item.record).is_some_and(|it| it.throwable != 0))
}

/// Magic affix fits the item.
pub fn magic_fits<S: ItemStats>(t: &ItemTables, item: &Item<S>, row: &AffixRec) -> bool {
    if classic_excluded(t, item) {
        return false;
    }
    let socketable = t.item(item.record).is_some_and(|r| r.hasinv != 0);
    if !(socketable && max_sockets(t, item) != 0) {
        let code = row.mods[0].code;
        if code >= 0 {
            match t.properties.get(code as usize) {
                None => return false,
                Some(p) if p.slots[0].stat == super::stat::NUMSOCKETS => return false,
                _ => {}
            }
        }
    }
    if any_type(t, item.record, &row.etype) {
        return false;
    }
    any_type(t, item.record, &row.itype)
}

/// Group taken.
fn group_taken<S>(t: &ItemTables, item: &Item<S>, group: i32) -> bool {
    let scan = |slots: &[u16]| {
        slots
            .iter()
            .take_while(|&&id| id != 0)
            .any(|&id| affix(t, id).is_some_and(|a| a.group == group))
    };
    // On `scro` and `book` suffix slot 0 holds a books row index, not an
    // affix id: it is skipped.
    let books = t.is_type(item.record, super::ty::SCRO as i16)
        || t.is_type(item.record, super::ty::BOOK as i16);
    scan(&item.prefix)
        || scan(if books {
            &item.suffix[1..]
        } else {
            &item.suffix
        })
}

/// The item's class for `classspecific` (itemtype `class`, ≥ 7 as 7).
fn item_class<S>(t: &ItemTables, item: &Item<S>) -> u8 {
    t.itype_of(item.record).map_or(7, |it| it.class.min(7))
}

/// Magic affix roller (the wrappers send a format-0 item to the legacy
/// roller). `preferred` is
/// 1-based within the part (≤ 0 none); `group` ≠ 0 selects the automagic
/// part. Returns the affix id or 0.
#[allow(clippy::too_many_arguments)]
pub fn roll_affix<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    spawnable: bool,
    force: bool,
    assign: bool,
    prefix: bool,
    preferred: i32,
    group: i32,
) -> u16 {
    if item.format < 1 {
        // The wrappers drop the group, so a format-0 call with a
        // group rolls a plain prefix.
        return roll_affix_legacy(
            t,
            item,
            spawnable,
            force,
            assign,
            prefix || group != 0,
            preferred,
        );
    }
    let which = if group != 0 {
        Part::Auto
    } else if prefix {
        Part::Prefix
    } else {
        Part::Suffix
    };
    let (first, len) = part(t, which);
    if item.item_seed.step() & 1 == 0 && !force {
        return 0;
    }
    let Some(rec) = t.item(item.record) else {
        return 0;
    };
    let a = alvl(
        item.ilvl.max(1),
        i32::from(rec.level),
        i32::from(rec.magic_lvl),
    );
    let weighted = rec.magic_lvl != 0;
    let class = item_class(t, item);
    let mut cands: Vec<(u16, i32)> = Vec::new();
    let mut total: i32 = 0;
    for i in 0..len {
        let row = &t.magic[first + i];
        let id = (first + i + 1) as u16;
        let pref = preferred > 0 && preferred - 1 == i as i32;
        let ok = (!spawnable || row.spawnable != 0)
            && (row.version < 100 || item.format >= 100)
            && (pref || (row.level <= a && (row.maxlevel == 0 || row.maxlevel >= a)))
            && (row.rare != 0 || !matches!(item.quality, q::RARE | q::CRAFTED | q::TEMPERED))
            && magic_fits(t, item, row)
            && (group == 0 || row.group == group)
            && row.frequency != 0
            && (row.classspecific == 0xFF || class == 7 || class == row.classspecific)
            && !group_taken(t, item, row.group);
        if !ok {
            continue;
        }
        if pref {
            if assign {
                apply_affix(t, item, id);
            }
            return id;
        }
        let w = if weighted {
            i32::from(row.frequency).wrapping_mul(row.level)
        } else {
            i32::from(row.frequency)
        };
        if cands.len() < MAX_CANDIDATES {
            cands.push((id, w));
        }
        total = total.wrapping_add(w);
    }
    let Some(&(last, _)) = cands.last() else {
        return 0;
    };
    let mut r = item.item_seed.roll_range(0, total.wrapping_add(1));
    let mut pick = last;
    for &(id, w) in &cands {
        r -= w;
        if r < 0 {
            pick = id;
            break;
        }
    }
    if assign {
        apply_affix(t, item, pick);
    }
    pick
}

/// Magic affix roller, format 0: alvl = the item
/// level plus 2, filter by spawnable, version, `level` and fit only, an
/// unweighted pick, and a preferred row that replaces the pick only when
/// listed.
fn roll_affix_legacy<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    spawnable: bool,
    force: bool,
    assign: bool,
    prefix: bool,
    preferred: i32,
) -> u16 {
    let (first, len) = part(t, if prefix { Part::Prefix } else { Part::Suffix });
    if item.item_seed.step() & 1 == 0 && !force {
        return 0;
    }
    let a = item.item_level().wrapping_add(2).clamp(1, 99);
    // (combined index, row index in the magic array)
    let mut cands: Vec<usize> = Vec::new();
    for i in first..first + len {
        if cands.len() >= MAX_CANDIDATES {
            break;
        }
        let row = &t.magic[i];
        if (!spawnable || row.spawnable != 0)
            && (row.version < 100 || item.format >= 100)
            && row.level <= a
            && magic_fits(t, item, row)
        {
            cands.push(i);
        }
    }
    if cands.is_empty() {
        return 0;
    }
    let r = item.item_seed.roll_range(0, cands.len() as i32) as usize;
    let row = cands[r];
    // Step 6: the preferred row replaces the pick when listed; else the id
    // becomes −1 (returned as 0) while candidate r's row stays.
    let id: i64 = if preferred > 0 {
        let want = first as i64 + i64::from(preferred) - 1;
        if cands.iter().any(|&c| c as i64 == want) {
            want
        } else {
            -1
        }
    } else {
        row as i64
    };
    if assign {
        let assigned = if id >= 0 { id as usize } else { row };
        apply_affix(t, item, (assigned + 1) as u16);
    }
    (id + 1) as u16
}

/// Rare affix fits.
fn rare_fits<S>(t: &ItemTables, item: &Item<S>, i: usize) -> bool {
    let row = &t.rare[i];
    !classic_excluded(t, item)
        && !(row.version >= 100 && item.format < 100)
        && !any_type(t, item.record, &row.etype)
        && any_type(t, item.record, &row.itype)
}

/// Rare name pick: a rare id or 0. Format 0 takes
/// Whose logic is the same: this function serves
/// both.
pub fn rare_name<S>(t: &ItemTables, item: &mut Item<S>, prefix: bool) -> u16 {
    let (first, len) = if prefix {
        (t.n_rare_suffix, t.rare.len() - t.n_rare_suffix)
    } else {
        (0, t.n_rare_suffix)
    };
    let cands: Vec<usize> = (first..first + len)
        .filter(|&i| rare_fits(t, item, i))
        .take(MAX_CANDIDATES)
        .collect();
    if cands.is_empty() {
        return 0;
    }
    let r = item.item_seed.roll_range(0, cands.len() as i32) as usize;
    (cands[r] + 1) as u16
}

/// Magic item.
pub fn magic<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    let (p, s) = (rq.prefix[0], rq.suffix[0]);
    let mut forced = false;
    let mut skip_suffix = false;
    if p < 0 {
        forced = true;
    } else {
        let a = roll_affix(t, item, true, p > 0, true, true, p, 0);
        item.prefix[0] = a;
        if a == 0 {
            forced = true;
        } else if s < 0 {
            skip_suffix = true;
        }
    }
    if !skip_suffix {
        if s > 0 {
            forced = true;
        }
        item.suffix[0] = roll_affix(t, item, true, forced, true, false, s, 0);
    }
    if item.prefix[0] == 0 && item.suffix[0] == 0 {
        return false;
    }
    item.flags &= !flag::IDENTIFIED;
    class_skill_mods(t, item, rq);
    true
}

/// Mode-0 properties of the six slots in the order P0 S0 P1 S1 P2 S2.
fn interleaved_props<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) {
    for k in 0..3 {
        for id in [item.prefix[k], item.suffix[k]] {
            if id != 0 {
                apply_affix(t, item, id);
            }
        }
    }
}

/// Rare item (a format-0 item goes to `rare_legacy`).
pub fn rare<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    if t.itype_of(item.record).is_none_or(|it| it.rare == 0) {
        return false;
    }
    if item.format < 1 {
        return rare_legacy(t, item, rq);
    }
    let rp = rare_name(t, item, true);
    let rs = rare_name(t, item, false);
    if rp == 0 || rs == 0 {
        return false;
    }
    item.rare_prefix = rp;
    item.rare_suffix = rs;
    let jewel = t
        .item(item.record)
        .is_some_and(|r| r.type_ == ty::JEWL as i16);
    let n = if jewel {
        item.item_seed.roll_range(3, 2) as u32
    } else {
        RARE_COUNTS[(item.item_seed.step() & 7) as usize]
    };
    let (mut np, mut ns) = (0usize, 0usize);
    let (mut pdone, mut sdone) = (false, false);
    let mut k = 0;
    while k < n {
        if pdone && sdone {
            break;
        }
        let suffix = if pdone {
            true
        } else if sdone {
            false
        } else {
            item.item_seed.step() & 1 == 1
        };
        let pref = if suffix { rq.suffix[ns] } else { rq.prefix[np] };
        let a = roll_affix(t, item, true, true, false, !suffix, pref, 0);
        if a == 0 {
            if suffix {
                sdone = true;
            } else {
                pdone = true;
            }
            continue;
        }
        if suffix {
            item.suffix[ns] = a;
            ns += 1;
            sdone |= ns == 3;
        } else {
            item.prefix[np] = a;
            np += 1;
            pdone |= np == 3;
        }
        k += 1;
    }
    if np == 0 && ns == 0 {
        return false;
    }
    item.flags &= !flag::IDENTIFIED;
    interleaved_props(t, item);
    class_skill_mods(t, item, rq);
    true
}

/// The rare roll's taken test: a filled slot of the kind (empty ones
/// skipped) holds `a` or an affix of `a`'s group. `a`'s group is read
/// without a record test (edge case 3): `a` = 0 with a filled slot is
/// [`Fatal::NullAffixGroup`].
fn taken(t: &ItemTables, slots: [u16; 3], a: u16) -> Result<bool, Fatal> {
    let filled: Vec<u16> = slots.iter().copied().filter(|&x| x != 0).collect();
    if filled.is_empty() {
        return Ok(false);
    }
    // Edge case 3: the group of id 0 is read from address 0x5C.
    let g = affix(t, a).ok_or(Fatal::NullAffixGroup)?.group;
    Ok(filled
        .iter()
        .any(|&x| x == a || affix(t, x).is_some_and(|r| r.group == g)))
}

/// Rare item, format 0: count 4–6 (jewels 3–4),
/// the kind step on every pass, no preferences, up to 252 tries per pass
/// with the rare roll's taken test.
fn rare_legacy<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) -> bool {
    let rp = rare_name(t, item, true);
    let rs = rare_name(t, item, false);
    if rp == 0 || rs == 0 {
        return false;
    }
    item.rare_prefix = rp;
    item.rare_suffix = rs;
    let jewel = t
        .item(item.record)
        .is_some_and(|r| r.type_ == ty::JEWL as i16);
    let n = if jewel {
        item.item_seed.roll_range(3, 2)
    } else {
        item.item_seed.roll_range(4, 3)
    };
    let (mut np, mut ns) = (0usize, 0usize);
    for _ in 0..n {
        let lo = item.item_seed.step();
        let suffix = np == 3 || (ns != 3 && lo & 1 == 1);
        for _ in 0..CRAFTED_TRIES {
            let a = roll_affix(t, item, true, true, false, !suffix, 0, 0);
            if a == 0 {
                break;
            }
            let slots = if suffix { item.suffix } else { item.prefix };
            // `a` ≠ 0 has a record, so the group read cannot fail.
            if !taken(t, slots, a).unwrap_or(true) {
                if suffix {
                    item.suffix[ns] = a;
                    ns += 1;
                } else {
                    item.prefix[np] = a;
                    np += 1;
                }
                break;
            }
            // All 252 taken: slot P (S) := 0, not advanced (it is
            // still empty here).
        }
    }
    if np == 0 && ns == 0 {
        return false;
    }
    item.flags &= !flag::IDENTIFIED;
    interleaved_props(t, item);
    class_skill_mods(t, item, rq);
    true
}

/// Crafted item (format 0: the same routine
/// with the legacy names and the legacy roller, both reached through the
/// format dispatch of [`rare_name`] and [`roll_affix`]).
pub fn crafted<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> Result<bool, Fatal> {
    let rp = rare_name(t, item, true);
    let rs = rare_name(t, item, false);
    if rp == 0 || rs == 0 {
        return Ok(false);
    }
    item.rare_prefix = rp;
    item.rare_suffix = rs;
    let m = match rq.ilvl {
        l if l > 70 => 4,
        l if l > 50 => 3,
        l if l > 30 => 2,
        _ => 1,
    };
    let n = (item.item_seed.step() % 5).max(m);
    let (mut np, mut ns) = (0usize, 0usize);
    for _ in 0..n {
        let lo = item.item_seed.step();
        let suffix = np == 3 || (ns != 3 && lo & 1 == 1);
        let at = if suffix { ns } else { np };
        let pref = if suffix { rq.suffix[at] } else { rq.prefix[at] };
        let mut stored = None;
        for _ in 0..CRAFTED_TRIES {
            let a = roll_affix(t, item, true, true, false, !suffix, pref, 0);
            let slots = if suffix { item.suffix } else { item.prefix };
            if !taken(t, slots, a)? {
                stored = Some(a);
                break;
            }
        }
        let slots = if suffix {
            &mut item.suffix
        } else {
            &mut item.prefix
        };
        if let Some(a) = stored {
            slots[at] = a;
            if suffix {
                ns += 1;
            } else {
                np += 1;
            }
        } else {
            slots[at] = 0;
        }
    }
    item.flags &= !flag::IDENTIFIED;
    interleaved_props(t, item);
    class_skill_mods(t, item, rq);
    Ok(true)
}

/// Tempered item (dispatch case 9): rare names only.
pub fn tempered<S>(t: &ItemTables, item: &mut Item<S>) -> bool {
    let rp = rare_name(t, item, true);
    let rs = rare_name(t, item, false);
    if rp == 0 || rs == 0 {
        return false;
    }
    item.rare_prefix = rp;
    item.rare_suffix = rs;
    true
}

/// Charm.
pub fn charm<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> Result<(), Fatal> {
    let (p, s) = (rq.prefix[0], rq.suffix[0]);
    let mut forced = false;
    if p > 0 {
        item.prefix[0] = roll_affix(t, item, true, true, true, true, p, 0);
    } else if s <= 0 {
        let a = roll_affix(t, item, true, false, true, true, s, 0);
        if a == 0 {
            forced = true;
        }
        item.prefix[0] = a;
    }
    if s > 0 {
        item.suffix[0] = roll_affix(t, item, true, true, true, false, s, 0);
    } else if p <= 0 {
        item.suffix[0] = roll_affix(t, item, true, forced, true, false, p, 0);
    }
    if item.prefix[0] == 0 && item.suffix[0] == 0 {
        return Err(Fatal::Charm);
    }
    item.flags &= !flag::IDENTIFIED;
    Ok(())
}

/// Automagic (finishing step).
pub fn automagic<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) {
    let g = t.item(item.record).map_or(0, |r| i32::from(r.auto_prefix));
    let a = roll_affix(t, item, false, true, false, false, 0, g);
    if a != 0 {
        item.auto_affix = a;
        apply_affix(t, item, a);
    }
}
