//! The TC runtime form built at load step 46 and the TC lookup by
//! id and level. Built from `d2-data` typed records; no draws.

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{
    text, Armor, Itemtypes, Misc, Setitems, Treasureclassex, Uniqueitems, Weapons,
};

use super::TreasureError;

/// Entry flags.
pub const FLAG_UNIQUE: u8 = 0x01;
pub const FLAG_SET: u8 = 0x02;
pub const FLAG_TC: u8 = 0x04;
pub const FLAG_NOT_CLASSIC: u8 = 0x10;

/// itemtypes record 38 `tpot`.
pub const TYPE_TPOT: usize = 38;

/// Largest TC count the loader accepts.
pub const MAX_TCS: usize = 65_534;

/// The item fields treasure reads (items columns), one per
/// item index (weapons, armor, misc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemData {
    pub code: [u8; 4],
    pub ubercode: [u8; 4],
    pub ultracode: [u8; 4],
    pub version: u16,
    pub level: u8,
    pub type_: u16,
    pub type2: u16,
    pub unique: u8,
    pub quest: u8,
    pub spawnable: u8,
}

macro_rules! item_data_from {
    ($t:ty) => {
        impl From<&$t> for ItemData {
            fn from(r: &$t) -> Self {
                ItemData {
                    code: r.code,
                    ubercode: r.ubercode,
                    ultracode: r.ultracode,
                    version: r.version,
                    level: r.level,
                    type_: r.type_,
                    type2: r.type2,
                    unique: r.unique,
                    quest: r.quest,
                    spawnable: r.spawnable,
                }
            }
        }
    };
}
item_data_from!(Weapons);
item_data_from!(Armor);
item_data_from!(Misc);

/// The item list in item-index order: weapons, armor, misc.
pub fn item_list(weapons: &[Weapons], armor: &[Armor], misc: &[Misc]) -> Vec<ItemData> {
    let w = weapons.iter().map(ItemData::from);
    let a = armor.iter().map(ItemData::from);
    let m = misc.iter().map(ItemData::from);
    w.chain(a).chain(m).collect()
}

/// `items.code` find: the first exact 4-byte
/// match, case-sensitive.
pub fn find_item_code(items: &[ItemData], code: [u8; 4]) -> Option<usize> {
    items.iter().position(|i| i.code == code)
}

/// The item test: the item's `type`,
/// or its nonzero `type2`, is equivalent to `t`.
pub fn item_is_type(equiv: &EquivMatrix, item: &ItemData, t: usize) -> bool {
    equiv.get(usize::from(item.type_), t)
        || (item.type2 != 0 && equiv.get(usize::from(item.type2), t))
}

/// One TC entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcEntry {
    pub start_classic: i32,
    pub start_expansion: i32,
    /// Item index, or TC index when [`FLAG_TC`]; 0xFFFF for a unique or
    /// set whose code is not an item.
    pub id: u16,
    /// uniqueitems / setitems record (flags 1 / 2), else gold multiplier.
    pub row: u16,
    pub flags: u8,
    /// magic, rare, set, unique, slot 5, slot 6 (classic games only).
    pub mods: [u16; 6],
}

/// One TC record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreasureClass {
    /// The name as stored (not normalized).
    pub name: Vec<u8>,
    pub group: u16,
    pub level: u16,
    pub total_classic: i32,
    pub total_expansion: i32,
    /// Never 0, except TC 0 (all zero).
    pub picks: i32,
    pub nodrop: i32,
    /// magic, rare, set, unique, slot 5, slot 6.
    pub mods: [u16; 6],
    pub entries: Vec<TcEntry>,
}

impl TreasureClass {
    fn new(name: Vec<u8>, group: u16, level: u16, picks: i32, nodrop: i32, mods: [u16; 6]) -> Self {
        TreasureClass {
            name,
            group,
            level,
            total_classic: 0,
            total_expansion: 0,
            picks,
            nodrop,
            mods,
            entries: Vec::new(),
        }
    }

    /// The total for the game's mode.
    pub fn total(&self, expansion: bool) -> i32 {
        if expansion {
            self.total_expansion
        } else {
            self.total_classic
        }
    }

    /// Appends an entry with prob `p`.
    fn push(&mut self, p: i32, expansion_only: bool, id: u16, row: u16, flags: u8) {
        self.entries.push(TcEntry {
            start_classic: self.total_classic,
            start_expansion: self.total_expansion,
            id,
            row,
            flags,
            mods: [0; 6],
        });
        self.total_expansion = self.total_expansion.wrapping_add(p);
        if !expansion_only {
            self.total_classic = self.total_classic.wrapping_add(p);
        }
    }
}

/// The tables reads.
pub struct TcSources<'a> {
    pub treasureclassex: &'a [Treasureclassex],
    pub itemtypes: &'a [Itemtypes],
    /// [`item_list`] order.
    pub items: &'a [ItemData],
    /// itemtypes equivalence.
    pub equiv: &'a EquivMatrix,
    pub uniqueitems: &'a [Uniqueitems],
    pub setitems: &'a [Setitems],
}

/// The runtime TC array and the chest TC table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreasureClasses {
    pub tcs: Vec<TreasureClass>,
    /// Group offset `A`.
    pub group_offset: u16,
    /// Index (difficulty × 5 + act) × 3 + tier; `None` for a missing name.
    pub chest: [Option<u16>; 45],
    /// Load notes (Edge case 7: a dropped forward TC reference).
    pub notes: Vec<String>,
}

/// Normalized name key: first 31 bytes, ASCII
/// upper case folded. Bytes ≥ 0x80 are rejected (D2RUST, E11).
fn name_key(s: &[u8]) -> Result<Vec<u8>, TreasureError> {
    let s = &s[..s.len().min(31)];
    if let Some(&b) = s.iter().find(|&&b| b >= 0x80) {
        return Err(TreasureError::NonAsciiName(b));
    }
    Ok(s.to_ascii_lowercase())
}

/// Find in an add-always name linker: the first
/// index stored under the key.
fn find_key(keys: &[Vec<u8>], key: &[u8]) -> Option<usize> {
    keys.iter().position(|k| k == key)
}

/// The keys of an add-always name linker over every record, so index =
/// record number.
fn name_linker<'a>(names: impl Iterator<Item = &'a [u8]>) -> Result<Vec<Vec<u8>>, TreasureError> {
    names.map(|n| name_key(text(n))).collect()
}

/// `atol` cut to u16.
///
/// TODO(treasure, `atol`): the CRT `atol` result for
/// values outside i32 is not known; this saturates like `strtol`.
/// The base game values are small.
fn atol_u16(s: &[u8]) -> u16 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C) {
        i += 1;
    }
    let neg = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    while let Some(d) = s.get(i).filter(|c| c.is_ascii_digit()) {
        v = (v * 10 + i64::from(d - b'0')).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    let v = if neg { -v } else { v };
    let v = v.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    v as u16
}

impl TreasureClasses {
    /// Builds the runtime form.
    pub fn build(src: &TcSources) -> Result<Self, TreasureError> {
        let uniques = name_linker(src.uniqueitems.iter().map(|u| &u.index[..]))?;
        let sets = name_linker(src.setitems.iter().map(|s| &s.index[..]))?;
        let mut tcs = vec![TreasureClass::new(Vec::new(), 0, 0, 0, 0, [0; 6])];
        let mut keys: Vec<Vec<u8>> = vec![Vec::new()];
        let mut notes = Vec::new();
        let mut a: u16 = 0;

        // automatic TCs.
        for (t, ty) in src.itemtypes.iter().enumerate() {
            if ty.treasureclass == 0 {
                continue;
            }
            a = a.wrapping_add(1);
            let code: Vec<u8> = ty.code.iter().copied().filter(|&b| b != b' ').collect();
            for lv in (3..=96).step_by(3) {
                let mut name = code.clone();
                name.extend_from_slice(lv.to_string().as_bytes());
                let mut tc = TreasureClass::new(name, 0, (lv - 3) as u16, 1, 0, [0; 6]);
                for (i, item) in src.items.iter().enumerate() {
                    let lvl = i32::from(item.level);
                    let ok = item.quest == 0
                        && item.spawnable != 0
                        && item_is_type(src.equiv, item, t)
                        && (t == TYPE_TPOT || !item_is_type(src.equiv, item, TYPE_TPOT))
                        && lv - 3 < lvl
                        && lvl <= lv;
                    if !ok {
                        continue;
                    }
                    let rarity = src
                        .itemtypes
                        .get(usize::from(item.type_))
                        .map_or(0, |r| i32::from(r.rarity));
                    let exp = item.version >= 100;
                    let flags = if exp { FLAG_NOT_CLASSIC } else { 0 };
                    tc.push(rarity.max(1), exp, i as u16, 0, flags);
                }
                keys.push(name_key(&tc.name)?);
                tcs.push(tc);
            }
        }

        // treasureclassex rows.
        for row in src.treasureclassex {
            let name = text(&row.treasure_class);
            if name.is_empty() {
                break;
            }
            let group = if row.group == 0 {
                0
            } else {
                row.group.wrapping_add(a)
            };
            let picks = match row.picks as i32 {
                0 => 1,
                p => p,
            };
            // Slots 5 and 6 come from record +0x30/+0x32, which no column
            // fills; the typed record has no field for them.
            let mods = [row.magic, row.rare, row.set, row.unique, 0, 0];
            let mut tc = TreasureClass::new(
                name.to_vec(),
                group,
                row.level,
                picks,
                row.nodrop as i32,
                mods,
            );
            let cells = [
                (&row.item1, row.prob1),
                (&row.item2, row.prob2),
                (&row.item3, row.prob3),
                (&row.item4, row.prob4),
                (&row.item5, row.prob5),
                (&row.item6, row.prob6),
                (&row.item7, row.prob7),
                (&row.item8, row.prob8),
                (&row.item9, row.prob9),
                (&row.item10, row.prob10),
            ];
            for (cell, prob) in cells {
                let s = text(cell);
                if s.is_empty() {
                    break;
                }
                let resolved =
                    item_string(src, &tcs, &keys, &uniques, &sets, &mut tc, s, prob as i32)?;
                if !resolved {
                    notes.push(format!(
                        "TC {:?}: item string {:?} matches nothing (dropped)",
                        String::from_utf8_lossy(name),
                        String::from_utf8_lossy(s)
                    ));
                }
            }
            keys.push(name_key(name)?);
            tcs.push(tc);
        }
        if tcs.len() > MAX_TCS {
            return Err(TreasureError::TooManyTcs(tcs.len()));
        }

        // chest table.
        let mut chest = [None; 45];
        for (d, dn) in ["", " (N)", " (H)"].iter().enumerate() {
            for act in 0..5 {
                for (tier, tn) in ["A", "B", "C"].iter().enumerate() {
                    let name = format!("Act {}{dn} Chest {tn}", act + 1);
                    let key = name_key(name.as_bytes())?;
                    chest[(d * 5 + act) * 3 + tier] = find_key(&keys, &key).map(|i| i as u16);
                }
            }
        }
        Ok(TreasureClasses {
            tcs,
            group_offset: a,
            chest,
            notes,
        })
    }

    /// TC count.
    pub fn len(&self) -> usize {
        self.tcs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tcs.is_empty()
    }

    /// `get(id, lvl)`.
    pub fn get(&self, id: u16, lvl: i32) -> Option<u16> {
        let id = usize::from(id);
        if id == 0 || id >= self.tcs.len() {
            return None;
        }
        let mut cur = id;
        if lvl > 0 && self.tcs[id].group != 0 {
            while let Some(next) = self.tcs.get(cur + 1) {
                if next.group != self.tcs[id].group || i32::from(next.level as i16) > lvl {
                    break;
                }
                cur += 1;
            }
        }
        Some(cur as u16)
    }

    /// The chest table entry, each index clamped to its range.
    pub fn chest_tc(&self, difficulty: i32, act: i32, tier: i32) -> Option<u16> {
        let d = difficulty.clamp(0, 2) as usize;
        let a = act.clamp(0, 4) as usize;
        let t = tier.clamp(0, 2) as usize;
        self.chest[(d * 5 + a) * 3 + t]
    }
}

/// One item string. Returns whether the name resolved.
#[allow(clippy::too_many_arguments)]
fn item_string(
    src: &TcSources,
    tcs: &[TreasureClass],
    keys: &[Vec<u8>],
    uniques: &[Vec<u8>],
    sets: &[Vec<u8>],
    tc: &mut TreasureClass,
    s: &[u8],
    p: i32,
) -> Result<bool, TreasureError> {
    if p < 1 {
        return Ok(true);
    }
    let s = s.strip_prefix(b"\"").unwrap_or(s);
    let s = &s[..s.iter().position(|&b| b == b'"').unwrap_or(s.len())];
    let (name, params) = match s.iter().position(|&b| b == b',') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };

    let mut found = None;
    if name.len() <= 4 {
        let mut code = [b' '; 4];
        code[..name.len()].copy_from_slice(name);
        if let Some(i) = find_item_code(src.items, code) {
            let exp = src.items[i].version >= 100;
            let flags = if exp { FLAG_NOT_CLASSIC } else { 0 };
            found = Some((exp, i as u16, 0, flags));
        }
    }
    if found.is_none() {
        let key = name_key(name)?;
        if let Some(i) = find_key(keys, &key).filter(|&i| i >= 1) {
            let exp = tcs[i].total_classic == 0;
            let flags = FLAG_TC | if exp { FLAG_NOT_CLASSIC } else { 0 };
            found = Some((exp, i as u16, 0, flags));
        } else if let Some(r) = find_key(uniques, &key).filter(|&r| r >= 1) {
            let id = item_id(src.items, src.uniqueitems[r].code);
            found = Some((true, id, r as u16, FLAG_NOT_CLASSIC | FLAG_UNIQUE));
        } else if let Some(r) = find_key(sets, &key) {
            let id = item_id(src.items, src.setitems[r].item);
            found = Some((true, id, r as u16, FLAG_NOT_CLASSIC | FLAG_SET));
        }
    }
    let Some((exp, id, row, flags)) = found else {
        return Ok(false);
    };
    tc.push(p, exp, id, row, flags);
    let entry = tc.entries.last_mut().expect("just pushed");
    if let Some(params) = params {
        for part in params.split(|&b| b == b',') {
            let Some(eq) = part.iter().position(|&b| b == b'=') else {
                break;
            };
            let v = atol_u16(&part[eq + 1..]);
            match &part[..eq] {
                b"mul" | b"ma" | b"mg" => entry.row = v,
                b"cm" => entry.mods[0] = v,
                b"cr" => entry.mods[1] = v,
                b"cs" => entry.mods[2] = v,
                b"cu" => entry.mods[3] = v,
                b"ce" => entry.mods[4] = v,
                b"cg" => entry.mods[5] = v,
                _ => break,
            }
        }
    }
    Ok(true)
}

/// Item index of a code, or 0xFFFF (−1 cut to u16) on a miss.
fn item_id(items: &[ItemData], code: [u8; 4]) -> u16 {
    find_item_code(items, code).map_or(0xFFFF, |i| i as u16)
}
