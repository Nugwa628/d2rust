//! The runtime maps the table loaders build outside the records: type
//! equivalence matrices, state bitsets, skill lists, gamble and monster
//! sequence indexes, hireling id tables and automap ranges. `apply`
//! builds them from the fixed-up tables into `FixedSet`, which
//! D2RUST's simulation and server read.

use super::qsort::qsort;
use super::{err, i16_at, i32_at, FixupError};
use crate::bin::{cstr, u32_at, BinTable};
use crate::compile::CodeLinker;

/// Equivalence walk stack: 128 ints, overflow test > 124.
const WALK_LIMIT: usize = 124;
/// Pops allowed per matrix cell, on average over the whole matrix. A link
/// cycle that keeps the stack under [`WALK_LIMIT`] (a row whose `equiv1`
/// leads back to itself) never ends the walk, so D2RUST
/// reports a load error once the matrix has used this budget.
const WALK_STEPS_PER_CELL: usize = 128;
/// Number of state flag bitsets.
pub const STATE_FLAGS: usize = 40;
/// Number of player classes.
pub const CLASSES: usize = 7;
/// Number of gamble level thresholds.
pub const GAMBLE_LEVELS: usize = 100;
/// Number of monpreset acts.
pub const ACTS: usize = 5;
/// Hireling id table size.
pub const HIRELING_IDS: usize = 256;
/// Hireling version at and above which a row goes in the second table.
const HIRELING_EXPANSION: u16 = 100;
/// automap level names (list A); a row's `LevelName` must be one of these.
pub const AUTOMAP_LEVELS: &[&str] = &[
    "None",
    "1 Town",
    "1 Wilderness",
    "1 Cave",
    "1 Crypt",
    "1 Monestary",
    "1 Courtyard",
    "1 Barracks",
    "1 Jail",
    "1 Cathedral",
    "1 Catacombs",
    "1 Tristram",
    "2 Town",
    "2 Sewer",
    "2 Harem",
    "2 Basement",
    "2 Desert",
    "2 Tomb",
    "2 Lair",
    "2 Arcane",
    "3 Town",
    "3 Jungle",
    "3 Kurast",
    "3 Spider",
    "3 Dungeon",
    "3 Sewer",
    "4 Town",
    "4 Mesa",
    "4 Lava",
    "5 Town",
    "5 Siege",
    "5 Barricade",
    "5 Temple",
    "5 Ice",
    "5 Baal",
    "5 Lava",
];
/// automap tile names (list B); a row's `TileName` must be one of these.
pub const AUTOMAP_TILES: &[&str] = &[
    "fl", "wl", "wr", "wtlr", "wtll", "wtr", "wbl", "wbr", "wld", "wrd", "wle", "wre", "co", "sh",
    "tr", "rf", "ld", "rd", "fd", "fi",
];

/// A type-equivalence bit matrix: `n` rows of `words` u32.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EquivMatrix {
    /// Number of types (rows and columns).
    pub n: usize,
    /// u32 words per row.
    pub words: usize,
    /// Row-major bits, row i at `i * words`.
    pub bits: Vec<u32>,
}

impl EquivMatrix {
    /// Bit (i, j): "is row i of type j?".
    pub fn get(&self, i: usize, j: usize) -> bool {
        i < self.n && j < self.n && self.bits[i * self.words + j / 32] & 1 << (j % 32) != 0
    }
}

/// Which equivalence table: the column-0 rule and the links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquivKind {
    /// The itemtypes table.
    ItemTypes,
    /// The montype table.
    MonType,
}

fn equiv(
    t: &BinTable,
    kind: EquivKind,
    i: i32,
    j: i32,
    steps: &mut usize,
) -> Result<bool, FixupError> {
    let n = t.count as i32;
    if j <= 0 {
        return Ok(kind == EquivKind::ItemTypes);
    }
    if i <= 0 || i >= n {
        return Ok(false);
    }
    let (o1, o2, o3) = match kind {
        EquivKind::ItemTypes => (0x04, 0x06, None),
        EquivKind::MonType => (0x02, 0x04, Some(0x06)),
    };
    let mut stack = vec![i];
    while let Some(tt) = stack.pop() {
        *steps = steps.checked_sub(1).ok_or_else(|| {
            err(
                &t.name,
                format!("equivalence walk ({i}, {j}) does not end (link cycle)"),
            )
        })?;
        if tt == j {
            return Ok(true);
        }
        if tt >= n {
            return Ok(false);
        }
        if stack.len() > WALK_LIMIT {
            return Ok(false);
        }
        let r = t.record(tt as usize);
        let e1 = i32::from(i16_at(r, o1));
        if e1 > 0 {
            stack.push(e1);
            let e2 = i32::from(i16_at(r, o2));
            let e3 = o3.map_or(0, |o| i32::from(i16_at(r, o)));
            let mut push = |v: i32| -> Result<(), FixupError> {
                if v < 0 {
                    return Err(err(
                        &t.name,
                        format!("row {tt}: negative equivalence link {v}"),
                    ));
                }
                stack.push(v);
                Ok(())
            };
            if e2 != 0 {
                push(e2)?;
                if e3 != 0 {
                    push(e3)?;
                }
            }
        }
    }
    Ok(false)
}

/// The equivalence matrix of itemtypes or montype.
pub fn equiv_matrix(t: &BinTable, kind: EquivKind) -> Result<EquivMatrix, FixupError> {
    let n = t.count;
    let words = n.div_ceil(32);
    let mut bits = vec![0u32; n * words];
    let mut steps = n.saturating_mul(n).saturating_mul(WALK_STEPS_PER_CELL);
    for i in 0..n {
        for j in 0..n {
            if equiv(t, kind, i as i32, j as i32, &mut steps)? {
                bits[i * words + j / 32] |= 1 << (j % 32);
            }
        }
    }
    Ok(EquivMatrix { n, words, bits })
}

/// itemstatcost description list: stats with `descfunc` ≠ 0, sorted
/// by signed `descpriority` with the CRT sort.
pub fn desc_list(t: &BinTable) -> Vec<u16> {
    let mut v: Vec<(u16, i16)> = t
        .iter()
        .enumerate()
        .filter(|(_, r)| r[0x36] != 0)
        .map(|(i, r)| (i as u16, i16_at(r, 0x34)))
        .collect();
    qsort(&mut v, |a, b| a.1.cmp(&b.1));
    v.into_iter().map(|(i, _)| i).collect()
}

/// states maps: flag bitsets and the lists of states with a given flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateMaps {
    /// 40 bitsets of `words` u32, bitset k at k·words.
    pub words: usize,
    /// The bitsets, flag k's bitset at `k * words`.
    pub bitsets: Vec<u32>,
    /// States with flag bit 4 (`pgsv`).
    pub pgsv: Vec<u16>,
    /// States with flag bit 11 (`curse`).
    pub curse: Vec<u16>,
    /// States with flag bit 16 (`disguise`).
    pub disguise: Vec<u16>,
    /// States with flag bit 5 (`active`).
    pub active: Vec<u16>,
    /// States whose +0x2A field is above 0.
    pub itemtype: Vec<u16>,
}

/// Builds the states bitsets and lists from the states table.
pub fn states(t: &BinTable) -> StateMaps {
    let words = t.count.div_ceil(32);
    let flag = |r: &[u8], k: usize| r[0x10 + k / 8] & 1 << (k % 8) != 0;
    let mut m = StateMaps {
        words,
        bitsets: vec![0; STATE_FLAGS * words],
        ..StateMaps::default()
    };
    for (s, r) in t.iter().enumerate() {
        for k in 0..STATE_FLAGS {
            if flag(r, k) {
                m.bitsets[k * words + s / 32] |= 1 << (s % 32);
            }
        }
        let s16 = s as u16;
        for (bit, list) in [
            (4, &mut m.pgsv),
            (11, &mut m.curse),
            (16, &mut m.disguise),
            (5, &mut m.active),
        ] {
            if flag(r, bit) {
                list.push(s16);
            }
        }
        if i16_at(r, 0x2A) > 0 {
            m.itemtype.push(s16);
        }
    }
    m
}

/// skills class and passive lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillLists {
    /// Skills per class.
    pub counts: [u32; CLASSES],
    /// 7 × max entries, class c's list at c·max.
    pub max: usize,
    /// The class lists, class c's list at `c * max`.
    pub lists: Vec<u16>,
    /// Skills whose +0x94 field is 0 or above (passives).
    pub passives: Vec<u16>,
}

/// Builds the per-class skill lists and the passive list from the skills table.
pub fn skill_lists(t: &BinTable) -> SkillLists {
    let mut by_class: [Vec<u16>; CLASSES] = Default::default();
    let mut passives = Vec::new();
    for (s, r) in t.iter().enumerate() {
        let c = r[0x0C] as i8;
        if (0..CLASSES as i8).contains(&c) {
            by_class[c as usize].push(s as u16);
        }
        if i16_at(r, 0x94) >= 0 {
            passives.push(s as u16);
        }
    }
    let max = by_class.iter().map(Vec::len).max().unwrap_or(0);
    let mut lists = vec![0u16; CLASSES * max];
    for (c, l) in by_class.iter().enumerate() {
        lists[c * max..c * max + l.len()].copy_from_slice(l);
    }
    SkillLists {
        counts: std::array::from_fn(|c| by_class[c].len() as u32),
        max,
        lists,
        passives,
    }
}

/// Version-0 item list: one slot per item, the version-0 indices
/// first, the rest 0.
pub fn version0_items(items: &[&BinTable]) -> Vec<u16> {
    let all: Vec<&[u8]> = items.iter().flat_map(|t| t.iter()).collect();
    let mut out = vec![0u16; all.len()];
    let zero = all
        .iter()
        .enumerate()
        .filter(|(_, r)| u16::from_le_bytes([r[0xF6], r[0xF7]]) == 0);
    for (slot, (j, _)) in zero.enumerate() {
        out[slot] = j as u16;
    }
    out
}

/// gamble: sorted item indices and the 100 level thresholds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gamble {
    /// Absent when the table is empty.
    pub index: Option<Vec<u32>>,
    /// Per level, the position in `index` of the first item above that level.
    pub thresholds: [u32; GAMBLE_LEVELS],
}

/// Builds the gamble maps; also writes +0x04 (level) and +0x08 (item) of
/// each row.
pub fn gamble(
    t: &mut BinTable,
    item_codes: &CodeLinker,
    items: &[&BinTable],
) -> Result<Gamble, FixupError> {
    if t.count == 0 {
        return Ok(Gamble {
            index: None,
            thresholds: [0; GAMBLE_LEVELS],
        });
    }
    let all: Vec<&[u8]> = items.iter().flat_map(|t| t.iter()).collect();
    let size = t.record_size;
    let mut rows: Vec<(u32, u32)> = Vec::with_capacity(t.count);
    for (r, rec) in t.records.chunks_exact_mut(size).enumerate() {
        let code = u32_at(rec, 0x00);
        let j = item_codes
            .find(code)
            .filter(|&j| (j as usize) < all.len())
            .ok_or_else(|| err("gamble", format!("row {r}: code {code:#010x} not an item")))?;
        let level = u32::from(all[j as usize][0xFD]);
        rec[0x04..0x08].copy_from_slice(&level.to_le_bytes());
        rec[0x08..0x0C].copy_from_slice(&j.to_le_bytes());
        rows.push((level, j));
    }
    qsort(&mut rows, |a, b| a.0.cmp(&b.0));
    let count = rows.len() as u32;
    let mut thresholds = [0u32; GAMBLE_LEVELS];
    thresholds[0] = 2;
    for (l, th) in thresholds.iter_mut().enumerate().skip(1) {
        *th = rows
            .iter()
            .position(|&(level, _)| level > l as u32)
            .map_or(count, |p| p as u32);
    }
    Ok(Gamble {
        index: Some(rows.into_iter().map(|(_, j)| j).collect()),
        thresholds,
    })
}

/// One monseq index entry: first record, count, count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeqEntry {
    /// First row of the sequence, if any row has it.
    pub first: Option<u32>,
    /// Number of rows in the sequence.
    pub count: u32,
    /// A second copy of the row count.
    pub count2: u32,
}

/// Builds the monseq index: one entry per sequence number up to the last row's.
pub fn monseq(t: &BinTable) -> Result<Vec<SeqEntry>, FixupError> {
    if t.count == 0 {
        return Ok(Vec::new());
    }
    let last = i32::from(i16_at(t.record(t.count - 1), 0x00)) + 1;
    let e = usize::try_from(last).map_err(|_| err("monseq", "last sequence < 0"))?;
    let mut out = vec![SeqEntry::default(); e];
    for (r, rec) in t.iter().enumerate() {
        let s = i16_at(rec, 0x00);
        let entry = usize::try_from(s)
            .ok()
            .and_then(|s| out.get_mut(s))
            .ok_or_else(|| err("monseq", format!("row {r}: sequence {s} outside 0..{e}")))?;
        entry.first.get_or_insert(r as u32);
        entry.count += 1;
        entry.count2 += 1;
    }
    Ok(out)
}

/// monpreset per-act ranges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActRanges {
    /// First row of each act.
    pub first: [Option<u32>; ACTS],
    /// Number of rows in each act.
    pub count: [u32; ACTS],
}

/// Builds the per-act row ranges of monpreset.
pub fn monpreset(t: &BinTable) -> Result<ActRanges, FixupError> {
    let mut a = ActRanges::default();
    let mut cur = 0usize;
    a.first[0] = Some(0);
    let mut run = 0u32;
    for (r, rec) in t.iter().enumerate() {
        let act = usize::from(rec[0x00]);
        while cur + 1 < act {
            a.count[cur] = run;
            cur += 1;
            if cur >= ACTS {
                return Err(err("monpreset", format!("row {r}: Act {act} > {ACTS}")));
            }
            a.first[cur] = Some(r as u32);
            run = 0;
        }
        run += 1;
    }
    a.count[cur] = run;
    Ok(a)
}

/// hireling id tables: [version < 100, version ≥ 100], first row per
/// Id, −1 when none.
pub fn hireling_first(t: &BinTable) -> Result<[[i32; HIRELING_IDS]; 2], FixupError> {
    let mut out = [[-1i32; HIRELING_IDS]; 2];
    for (r, rec) in t.iter().enumerate() {
        let id = i32_at(rec, 0x04);
        if id < 0 {
            return Err(err("hireling", format!("row {r}: Id {id}")));
        }
        if (id as usize) < HIRELING_IDS {
            let table = usize::from(u16::from_le_bytes([rec[0], rec[1]]) >= HIRELING_EXPANSION);
            let slot = &mut out[table][id as usize];
            if *slot < 0 {
                *slot = r as i32;
            }
        }
    }
    Ok(out)
}

/// leveldefs portal list.
pub fn portals(t: &BinTable) -> Vec<u32> {
    t.iter()
        .enumerate()
        .filter(|(_, r)| u32_at(r, 0x8C) != 0)
        .map(|(i, _)| i as u32)
        .collect()
}

/// lvlsub first row per Type; empty when the largest Type is 0.
pub fn lvlsub_types(t: &BinTable) -> Result<Vec<u32>, FixupError> {
    let mut m = 0;
    for (r, rec) in t.iter().enumerate() {
        let ty = i32_at(rec, 0x00);
        if ty < 0 {
            return Err(err("lvlsub", format!("row {r}: Type {ty}")));
        }
        m = m.max(ty as usize);
    }
    if m == 0 {
        return Ok(Vec::new());
    }
    let mut out = vec![0u32; m + 1];
    let mut prev = 0;
    for (r, rec) in t.iter().enumerate() {
        let ty = i32_at(rec, 0x00) as usize;
        if ty != prev {
            out[ty] = r as u32;
            prev = ty;
        }
    }
    Ok(out)
}

/// automap converted records and level-name ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Automap {
    /// 0x20-byte converted records.
    pub records: Vec<[u8; 0x20]>,
    /// (first, end) per list-A index, (−1, −1) when unused.
    pub ranges: Vec<(i32, i32)>,
}

fn automap_name(r: usize, value: &[u8], list: &[&str], what: &str) -> Result<i32, FixupError> {
    if value.first() == Some(&b'0') {
        return Ok(0);
    }
    list.iter()
        .position(|n| n.as_bytes() == value)
        .map(|i| i as i32)
        .ok_or_else(|| {
            err(
                "automap",
                format!(
                    "row {r}: {what} {:?} not in its list",
                    String::from_utf8_lossy(value)
                ),
            )
        })
}

/// Converts the automap rows to 0x20-byte records and builds the level-name ranges.
pub fn automap(t: &BinTable) -> Result<Automap, FixupError> {
    let mut records = Vec::with_capacity(t.count);
    for (r, rec) in t.iter().enumerate() {
        let level = automap_name(r, cstr(rec, 0x00..0x10), AUTOMAP_LEVELS, "LevelName")?;
        let tile = automap_name(r, cstr(rec, 0x10..0x18), AUTOMAP_TILES, "TileName")?;
        let mut o = [0u8; 0x20];
        o[0x00..0x04].copy_from_slice(&level.to_le_bytes());
        o[0x04..0x08].copy_from_slice(&tile.to_le_bytes());
        o[0x08..0x0B].copy_from_slice(&rec[0x18..0x1B]);
        o[0x0C..0x1C].copy_from_slice(&rec[0x1C..0x2C]);
        let cels = (0..4)
            .take_while(|&k| i32_at(rec, 0x1C + 4 * k) != -1)
            .count() as i32;
        o[0x1C..0x20].copy_from_slice(&cels.to_le_bytes());
        records.push(o);
    }
    let mut ranges = vec![(-1, -1); AUTOMAP_LEVELS.len()];
    let mut f = 0;
    while f < records.len() {
        let key = &records[f][..4];
        let e = f + records[f..].iter().take_while(|o| &o[..4] == key).count();
        let level = i32_at(&records[f], 0) as usize;
        ranges[level] = (f as i32, e as i32);
        f = e;
    }
    Ok(Automap { records, ranges })
}
