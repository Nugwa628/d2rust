//! The record-byte fix-ups that need more than a string id: itemstatcost
//! op tables, charstats, set attachment, gems, monstats, monequip,
//! monumod, levels, tile paths and objects. `apply` calls them
//! table by table in load order; they edit the record bytes of the copy
//! of the loaded set that becomes `FixedSet`.

use d2_formats::animdata::AnimData;

use super::text::{copy_wide, fix_path, wide_text};
use super::{err, get_u16, i16_at, i32_at, rec, records_mut, set_u16, FixupError};
use crate::bin::{cstr, u32_at, BinTable};
use crate::compile::CodeLinker;
use crate::strings::StringTables;

/// Largest valid itemstatcost op; a larger op is cleared.
pub const MAX_OP: u8 = 13;
/// Op-base slots and op-stat entries per stat.
const OP_BASE_SLOTS: usize = 64;
const OP_STAT_ENTRIES: usize = 16;
/// Items per set.
pub const SET_ITEMS: i32 = 6;
/// monstats rows below this take their run base from the walk.
pub const RUN_BASE_ROWS: usize = 410;
/// Speed cap.
pub const SPEED_CAP: u32 = 32_767;
/// Chain step limit.
const CHAIN_STEPS: u32 = 255;
/// monequip loc range.
const LOCS: std::ops::RangeInclusive<i8> = 1..=10;
/// An empty `item` cell.
const NO_ITEM: u32 = 0x2020_2020;
/// monumod row limit.
pub const MONUMOD_ROWS: usize = 256;

/// itemstatcost op tables; an op of 0 or above [`MAX_OP`] is cleared to 0.
pub fn stat_ops(t: &mut BinTable) {
    let n = t.count;
    for r in records_mut(t) {
        r[0x5E..0xDE].fill(0xFF);
    }
    for i in 0..n {
        let r = rec(t, i);
        let (op, param) = (r[0x54], r[0x55]);
        if op == 0 || op > MAX_OP {
            r[0x54] = 0;
            continue;
        }
        let base = get_u16(r, 0x56);
        let op_stats = [get_u16(r, 0x58), get_u16(r, 0x5A), get_u16(r, 0x5C)];
        if usize::from(base) < n {
            let b = rec(t, usize::from(base));
            b[0x51] = 1;
            if let Some(slot) = (0..OP_BASE_SLOTS)
                .map(|k| 0x5E + 2 * k)
                .find(|&o| usize::from(get_u16(b, o)) >= n)
            {
                set_u16(b, slot, i as u16);
            }
            if op == 4 || op == 5 {
                rec(t, i)[0x53] = 1;
            }
        }
        for s in op_stats {
            let s = usize::from(s);
            if s >= n {
                break;
            }
            rec(t, i)[0x51] = 1;
            let target = rec(t, s);
            let Some(e) = (0..OP_STAT_ENTRIES)
                .map(|m| 0xDE + 6 * m)
                .find(|&e| target[e + 4] == 0)
            else {
                continue;
            };
            set_u16(target, e, base);
            set_u16(target, e + 2, i as u16);
            target[e + 4] = op;
            target[e + 5] = param;
            target[0x52] = 1;
            let bit = if i == 7 || s == 7 {
                6
            } else if i == 9 || s == 9 {
                7
            } else if i == 11 || s == 11 {
                8
            } else {
                continue;
            };
            let r = rec(t, i);
            let flags = u32_at(r, 0x04) | 1 << bit | 1 << 5;
            r[0x04..0x08].copy_from_slice(&flags.to_le_bytes());
        }
    }
}

/// charstats: the class name, wide, at +0x00 (16 units).
pub fn charstats(t: &mut BinTable, strings: &StringTables) -> Result<(), FixupError> {
    for r in records_mut(t) {
        let text = wide_text(strings, "charstats", cstr(r, 0x20..0x30))?;
        r[0x00..0x10].fill(0);
        copy_wide(r, 0x00, &text, 16);
    }
    Ok(())
}

/// Set attachment. The slot pointer at sets +0x110 + 4c holds
/// the setitems index.
pub fn attach_set_items(setitems: &mut BinTable, sets: &mut BinTable) -> Result<(), FixupError> {
    let size = sets.record_size;
    for (n, r) in records_mut(setitems).enumerate() {
        let s = i16_at(r, 0x2C);
        if s < 0 || s as usize >= sets.count {
            continue;
        }
        let set = &mut sets.records[s as usize * size..(s as usize + 1) * size];
        let c = i32_at(set, 0x0C);
        if c >= SET_ITEMS {
            continue;
        }
        if c < 0 {
            return Err(err("setitems", format!("set {s}: item count {c} at +0x0C")));
        }
        set_u16(r, 0x22, get_u16(set, 0x04));
        set_u16(r, 0x2E, c as u16);
        let slot = 0x110 + 4 * c as usize;
        set[slot..slot + 4].copy_from_slice(&(n as u32).to_le_bytes());
        set[0x0C..0x10].copy_from_slice(&(c + 1).to_le_bytes());
    }
    Ok(())
}

/// The combined item array: record `j` of weapons,
/// armor, misc in that order.
fn item_mut(tables: &mut [BinTable], j: usize) -> Option<&mut [u8]> {
    let mut j = j;
    for name in ["weapons", "armor", "misc"] {
        let t = tables.iter().position(|t| t.name == name)?;
        if j < tables[t].count {
            return Some(rec(&mut tables[t], j));
        }
        j -= tables[t].count;
    }
    None
}

/// gems: the buggy string id at +0x2C, then items `gemoffset`.
pub fn gems(
    gems: &mut BinTable,
    items: &mut [BinTable],
    strings: &StringTables,
) -> Result<(), FixupError> {
    for r in records_mut(gems) {
        let mut key: Vec<u8> = r[0x28..0x2B]
            .iter()
            .map(|&b| if b == 0x20 { 0 } else { b })
            .collect();
        key.truncate(key.iter().position(|&b| b == 0).unwrap_or(3));
        set_u16(r, 0x2C, strings.id(&key) as u16);
    }
    for (k, r) in gems.iter().enumerate() {
        let item = item_mut(items, k)
            .ok_or_else(|| err("gems", format!("gem {k}: no item {k} to reset")))?;
        item[0xF0..0xF4].copy_from_slice(&(-1i32).to_le_bytes());
        let j = i32_at(r, 0x28);
        if j >= 0 {
            let item = item_mut(items, j as usize)
                .ok_or_else(|| err("gems", format!("gem {k}: item index {j} out of range")))?;
            item[0xF0..0xF4].copy_from_slice(&(k as i32).to_le_bytes());
        }
    }
    Ok(())
}

/// monstats pass A: chain length +0x4A and position +0x4B.
pub fn monstats_chains(t: &mut BinTable) -> Result<(), FixupError> {
    let (n, size) = (t.count, t.record_size);
    let read = |t: &BinTable, row: usize, o: usize| i16_at(t.record(row), o);
    let range = |v: i16, what: &str, row: usize| -> Result<usize, FixupError> {
        if v < 0 || v as usize >= n {
            Err(err(
                "monstats",
                format!("row {row}: {what} {v} out of range"),
            ))
        } else {
            Ok(v as usize)
        }
    };
    for r in 0..n {
        let mut cur = range(read(t, r, 0x02), "BaseId", r)?;
        let mut steps: u32 = 0;
        let mut pos = None;
        loop {
            if cur == r {
                pos = Some(steps as u8);
            }
            let next = read(t, cur, 0x04);
            steps += 1;
            if cur as i32 == i32::from(next) || steps > CHAIN_STEPS {
                break;
            }
            if next < 0 {
                break;
            }
            cur = range(next, "NextInClass", cur)?;
        }
        let rec = &mut t.records[r * size..(r + 1) * size];
        rec[0x4A] = steps as u8;
        if let Some(p) = pos {
            rec[0x4B] = p;
        }
    }
    Ok(())
}

/// The first 3 bytes of a 4-byte code with 0x20 → 0, up to the first 0
/// (COF name parts).
fn part(code: &[u8]) -> Vec<u8> {
    code[..3]
        .iter()
        .map(|&b| if b == 0x20 { 0 } else { b })
        .take_while(|&b| b != 0)
        .collect()
}

/// Speed of the AnimData record for (monster class `b`, `mode`).
fn speed(
    monstats: &BinTable,
    b: usize,
    mode: usize,
    monstats2: &BinTable,
    monmode: &BinTable,
    anim: &AnimData,
) -> Result<u32, FixupError> {
    let m = monstats.record(b);
    let mode_rec = (mode < monmode.count)
        .then(|| monmode.record(mode))
        .ok_or_else(|| err("monstats", format!("monmode {mode} missing")))?;
    let ex = i16_at(m, 0x18);
    let wclass = if ex >= 0 && (ex as usize) < monstats2.count {
        part(&monstats2.record(ex as usize)[0x10..0x14])
    } else {
        b"hth".to_vec()
    };
    let mut name = part(&m[0x10..0x14]);
    name.extend(part(&mode_rec[0x20..0x24]));
    name.extend(wclass);
    anim.record(&name)
        .map(|r| r.speed)
        .map_err(|e| err("monstats", format!("row {b}: AnimData: {e}")))
}

/// `(factor × w) / divisor`: 32-bit wrapping product of the sign-extended
/// factor, unsigned division.
fn scale(factor: i16, w: u32, divisor: i16) -> u32 {
    (i32::from(factor) as u32).wrapping_mul(w) / divisor as u32
}

/// monstats pass B: BaseId repair, walk and run speeds.
pub fn monstats_speeds(
    t: &mut BinTable,
    monstats2: &BinTable,
    monmode: &BinTable,
    anim: &AnimData,
) -> Result<(), FixupError> {
    let (n, size) = (t.count, t.record_size);
    for r in 0..n {
        let mut b = i16_at(t.record(r), 0x02);
        if b < 0 || b as usize >= n {
            set_u16(&mut t.records[r * size..(r + 1) * size], 0x02, r as u16);
            b = r as i16;
        }
        let b = b as usize;
        let (vel_r, run_r) = (i16_at(t.record(r), 0x32), i16_at(t.record(r), 0x34));
        let (vel_b, run_b) = (i16_at(t.record(b), 0x32), i16_at(t.record(b), 0x34));
        let mut w = speed(t, b, 2, monstats2, monmode, anim)?;
        if b != r && vel_b > 0 {
            w = scale(vel_r, w, vel_b);
        }
        set_u16(
            &mut t.records[r * size..(r + 1) * size],
            0x36,
            w.min(SPEED_CAP) as u16,
        );
        let mut run = if r < RUN_BASE_ROWS {
            (i32::from(i16_at(t.record(b), 0x36)) / 2) as u32
        } else {
            speed(t, b, 15, monstats2, monmode, anim)?
        };
        if b != r && run_b > 0 {
            run = scale(run_r, run, run_b);
        }
        set_u16(
            &mut t.records[r * size..(r + 1) * size],
            0x38,
            run.min(SPEED_CAP) as u16,
        );
    }
    Ok(())
}

/// monequip: monstats +0x2A := first monequip row of the monster,
/// else −1; locs outside 1–10, or whose non-empty item code is not in the
/// item code map, are cleared. Rows naming no valid monster are left as
/// they are.
pub fn link_monequip(monequip: &mut BinTable, monstats: &mut BinTable, items: &CodeLinker) {
    let size = monstats.record_size;
    for r in records_mut(monstats) {
        set_u16(r, 0x2A, 0xFFFF);
    }
    for (n, r) in records_mut(monequip).enumerate() {
        let m = i16_at(r, 0x00);
        if m < 0 || m as usize >= monstats.count {
            continue;
        }
        let rec = &mut monstats.records[m as usize * size..(m as usize + 1) * size];
        if i16_at(rec, 0x2A) < 0 {
            set_u16(rec, 0x2A, n as u16);
        }
        for k in 0..3 {
            let code = u32_at(r, 0x08 + 4 * k);
            let loc = r[0x14 + k] as i8;
            if !LOCS.contains(&loc) || (code != NO_ITEM && items.find(code).is_none()) {
                r[0x14 + k] = 0;
            }
        }
    }
}

/// monumod: a count above 256 is cut to 256.
pub fn clamp_monumod(t: &mut BinTable) {
    if t.count > MONUMOD_ROWS {
        t.count = MONUMOD_ROWS;
        t.records.truncate(MONUMOD_ROWS * t.record_size);
    }
}

/// levels: wide names and monster list counts.
pub fn levels(t: &mut BinTable, strings: &StringTables) -> Result<(), FixupError> {
    for r in records_mut(t) {
        let name = wide_text(strings, "levels", cstr(r, 0xF5..0x11D))?;
        let warp = wide_text(strings, "levels", cstr(r, 0x11D..0x145))?;
        r[0x16E..0x1BE].fill(0);
        r[0x1BE..0x20E].fill(0);
        copy_wide(r, 0x16E, &name, 40);
        copy_wide(r, 0x1BE, &warp, 40);
        set_u16(r, 0x1BC, 0);
        set_u16(r, 0x20C, 0);
        for (k, list) in [0x36, 0x68, 0x9A].into_iter().enumerate() {
            r[0x33 + k] = (0..25)
                .take_while(|&e| i16_at(r, list + 2 * e) >= 0)
                .count() as u8;
        }
    }
    Ok(())
}

/// Tile paths of lvltypes, lvlprest (gated by `lod` or `Expansion`
/// = 0) and lvlsub.
pub fn tile_paths(t: &mut BinTable, lod: bool) -> Result<(), FixupError> {
    let name = t.name.clone();
    let fields: Vec<usize> = match name.as_str() {
        "lvltypes" => (0..32).map(|k| 0x3C * k).collect(),
        "lvlprest" => (0..6).map(|k| 0x44 + 0x3C * k).collect(),
        "lvlsub" => vec![0x04],
        _ => return Ok(()),
    };
    for r in records_mut(t) {
        if name == "lvlprest" && !lod && u32_at(r, 0x20) != 0 {
            continue;
        }
        for &f in &fields {
            fix_path(r, f, &name)?;
        }
    }
    Ok(())
}

/// objects: wide name at +0x40, frame counts × 256.
pub fn objects(t: &mut BinTable, strings: &StringTables) -> Result<(), FixupError> {
    for r in records_mut(t) {
        let text = wide_text(strings, "objects", cstr(r, 0x00..0x40))?;
        r[0x40..0xC0].fill(0);
        copy_wide(r, 0x40, &text, 64);
        for k in 0..8 {
            let o = 0xD8 + 4 * k;
            let v = u32_at(r, o) << 8;
            r[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    Ok(())
}
