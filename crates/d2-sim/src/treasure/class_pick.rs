//! The item class picks of the object and quest drop helpers: the part
//! pickers (armor), (weapons),
//! (misc) with their filter, the random
//! class, the weapon rack's six tries and
//! the class choice of the quest drop. Pure: the seed is
//! the caller's (a room seed or a unit seed), item creation and the floor
//! search are the wiring's (`crate::wiring::economy::drop_helpers`).

use d2_data::tables::{Armor, Misc, Weapons};

use crate::rng::Seed;

/// Item type 40 (`body`): misc rows skipped unless the dropper is a
/// monster.
pub const TYPE_BODY: i16 = 40;
/// The candidate cap of the pickers (fewer than 1,023 held).
pub const MAX_CANDIDATES: usize = 1023;
/// The random class's level bound (L > 65 → fatal 0x180).
pub const MAX_LEVEL: i32 = 65;
/// The weapon rack's tries.
pub const WEAPON_TRIES: u32 = 6;
/// The quality-4 loop's random-class re-picks before it switches to the
/// weapon pick.
pub const MAGIC_RANDOM_REPICKS: u32 = 11;
/// D2RUST guard for the unbounded quality-4 loop (edge case
/// 4): picks before [`PickError::Hang`].
pub const MAGIC_LOOP_GUARD: u32 = 1 << 20;
/// The gold code `gld `.
pub const GOLD_CODE: [u8; 4] = *b"gld ";

/// The columns a pick reads of one items record (combined index order:
/// weapons, armor, misc).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickRow {
    pub code: [u8; 4],
    /// `spawnable` (+0x133).
    pub spawnable: u8,
    /// `quest` (+0x12A).
    pub quest: u8,
    /// `level` (+0xFD).
    pub level: u8,
    /// `rarity` (+0xFC).
    pub rarity: u8,
    /// `type` (+0x11E), read as i16.
    pub type_: i16,
    /// `version` (+0xF6).
    pub version: u16,
    /// `bitfield1` (+0xDC): bit 0 "may be magic", bit 1 the
    /// weapon rack's keep bit.
    pub bitfield1: u32,
}

macro_rules! pick_row {
    ($($t:ty),*) => {$(
        impl From<&$t> for PickRow {
            fn from(r: &$t) -> Self {
                PickRow {
                    code: r.code,
                    spawnable: r.spawnable,
                    quest: r.quest,
                    level: r.level,
                    rarity: r.rarity,
                    type_: r.type_ as i16,
                    version: r.version,
                    bitfield1: r.bitfield1,
                }
            }
        }
    )*};
}
pick_row!(Weapons, Armor, Misc);

impl From<&crate::items::tables::ItemRec> for PickRow {
    fn from(r: &crate::items::tables::ItemRec) -> Self {
        PickRow {
            code: r.code,
            spawnable: r.spawnable,
            quest: r.quest,
            level: r.level,
            rarity: r.rarity,
            type_: r.type_,
            version: r.version,
            bitfield1: r.bitfield1,
        }
    }
}

/// A part of the combined items array (the header's (start, count)
/// pairs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Weapons,
    Armor,
    Misc,
}

/// The combined items array as the pickers see it, with its parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassPicks {
    pub rows: Vec<PickRow>,
    pub weapons: usize,
    pub armor: usize,
    /// The header's (start, count) per part (weapons, armor, misc) when
    /// given (`ItemTables::parts`; `None` in a slot: the part is absent,
    /// start pointer 0); `None`: the parts follow from `weapons` and
    /// `armor`, misc to the end.
    pub parts: Option<[Option<(usize, usize)>; 3]>,
}

/// A fatal assert of the picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PickError {
    /// L > 65 (fatal 0x180).
    #[error("random class level {0} > 65 (fatal 0x180)")]
    Level(i32),
    /// No `gld ` row (fatal 0x17C), or its index is 0 (0x17F).
    #[error("random class without a gold row (fatal 0x17C / 0x17F)")]
    NoGold,
    /// The drop code is not an items code (fatal 0x9EA).
    #[error("drop code {0:#010x} is not an items code (fatal 0x9EA)")]
    Code(u32),
    /// The quality-4 loop found no
    /// magic class in [`MAGIC_LOOP_GUARD`] picks (the base game never leaves it;
    /// D2RUST stops, edge case 4).
    #[error("quality-4 class loop without a magic class")]
    Hang,
}

impl ClassPicks {
    pub fn new(weapons: &[Weapons], armor: &[Armor], misc: &[Misc]) -> Self {
        let mut rows: Vec<PickRow> = weapons.iter().map(PickRow::from).collect();
        rows.extend(armor.iter().map(PickRow::from));
        rows.extend(misc.iter().map(PickRow::from));
        Self {
            rows,
            weapons: weapons.len(),
            armor: armor.len(),
            parts: None,
        }
    }

    /// The pick rows of the combined items array with its header's parts.
    pub fn of_items(
        items: &[crate::items::tables::ItemRec],
        parts: [Option<(usize, usize)>; 3],
    ) -> Self {
        Self {
            rows: items.iter().map(PickRow::from).collect(),
            weapons: parts[0].map_or(0, |p| p.1),
            armor: parts[1].map_or(0, |p| p.1),
            parts: Some(parts),
        }
    }

    /// The part's combined index range.
    pub fn part(&self, p: Part) -> std::ops::Range<usize> {
        if let Some(parts) = self.parts {
            let n = self.rows.len();
            // Misc runs from its start to the array end.
            return match parts[p as usize] {
                None => 0..0,
                Some((s, _)) if p == Part::Misc => s.min(n)..n,
                Some((s, c)) => s.min(n)..(s + c).min(n),
            };
        }
        let a = self.weapons;
        let m = (a + self.armor).min(self.rows.len());
        match p {
            Part::Weapons => 0..a.min(self.rows.len()),
            Part::Armor => a.min(m)..m,
            Part::Misc => m..self.rows.len(),
        }
    }

    /// The combined index of a code (first exact match).
    pub fn find_code(&self, code: [u8; 4]) -> Option<usize> {
        self.rows.iter().position(|r| r.code == code)
    }

    /// `bitfield1` bit `bit` of the record; no record → `None`.
    pub fn bit(&self, idx: i32, bit: u32) -> Option<bool> {
        let r = self.rows.get(usize::try_from(idx).ok()?)?;
        Some(r.bitfield1 >> bit & 1 != 0)
    }
}

/// With (L): the act of **level id** `L` (edge case 5):
/// thresholds 1, 40, 75, 103, 109, 1024.
pub fn pick_act(l: i32) -> i32 {
    match l {
        i32::MIN..=39 => 0,
        40..=74 => 1,
        75..=102 => 2,
        103..=108 => 3,
        109..=1023 => 4,
        _ => 0,
    }
}

/// A part pick: `t` the type filter (−1 any), `skip_act`
/// the act flag `a` / `p7` (nonzero: no rarity roll), `monster` the misc
/// pick's `m`. Draws on `seed`: one `roll(d)` per record that reaches the
/// rarity test with d > 0, in index order, then `roll(k)`. No candidate
/// (or no part) → −1 (D2RUST for the uninitialised slot, edge case 3).
#[allow(clippy::too_many_arguments)]
pub fn part_pick(
    p: &ClassPicks,
    part: Part,
    seed: &mut Seed,
    l: i32,
    t: i32,
    skip_act: bool,
    monster: bool,
    expansion: bool,
) -> i32 {
    let range = p.part(part);
    if range.is_empty() {
        return -1;
    }
    let l = l.max(1);
    let act = pick_act(l);
    let mut cands: Vec<usize> = Vec::new();
    for i in range {
        let r = &p.rows[i];
        if part == Part::Misc && !monster && r.type_ == TYPE_BODY {
            continue;
        }
        if r.spawnable == 0 || r.quest != 0 || i32::from(r.level) > l {
            continue;
        }
        if !skip_act {
            let d = i32::from(r.rarity) - act;
            if d > 0 && seed.roll(d) != 0 {
                continue;
            }
        }
        if t != -1 && i32::from(r.type_) != t {
            continue;
        }
        if (expansion || r.version < 100) && cands.len() < MAX_CANDIDATES {
            cands.push(i);
        }
    }
    if cands.is_empty() {
        return -1;
    }
    let j = seed.roll(cands.len() as i32) as usize;
    cands[j] as i32
}

/// Gold (65 − L) %, armor (⌊L/2⌋ + 5) %, weapons
/// (⌈L/2⌉ + 10) %, misc 20 %, on one `roll(100)` then the part's draws.
pub fn random_class(
    p: &ClassPicks,
    seed: &mut Seed,
    l: i32,
    t: i32,
    skip_act: bool,
    monster: bool,
    expansion: bool,
) -> Result<i32, PickError> {
    if l > MAX_LEVEL {
        return Err(PickError::Level(l));
    }
    let gold = match p.find_code(GOLD_CODE) {
        Some(0) | None => return Err(PickError::NoGold),
        Some(g) => g as i32,
    };
    let g = MAX_LEVEL - l;
    let r = seed.roll(100) as i32;
    let pick = |part, seed: &mut Seed| part_pick(p, part, seed, l, t, skip_act, monster, expansion);
    Ok(if r < g {
        gold
    } else if r < g + l / 2 + 5 {
        pick(Part::Armor, seed)
    } else if r < 80 {
        pick(Part::Weapons, seed)
    } else {
        pick(Part::Misc, seed)
    })
}

///'s pick: up to 6 weapon picks; a pick ≥ 0 whose
/// record has `bitfield1` bit 1 is kept at once (no record → fatal
/// 0x104A, read here as not kept); after 6 failing tries the 6th pick
/// stands.
pub fn weapon_rack_pick(
    p: &ClassPicks,
    seed: &mut Seed,
    l: i32,
    t: i32,
    skip_act: bool,
    expansion: bool,
) -> i32 {
    let mut id = -1;
    for _ in 0..WEAPON_TRIES {
        id = part_pick(p, Part::Weapons, seed, l, t, skip_act, false, expansion);
        if id >= 0 && p.bit(id, 1) == Some(true) {
            return id;
        }
    }
    id
}

/// The class of the quest drop: the drop
/// code's index when `code` ≠ 0 (no draw), else the random class on the
/// unit seed; quality 4 re-picks while the record is missing or lacks
/// `bitfield1` bit 0 (11 random-class re-picks, then weapon picks; the base game
/// has no bound, edge case 4: D2RUST stops after [`MAGIC_LOOP_GUARD`] picks
/// with [`PickError::Hang`]).
#[allow(clippy::too_many_arguments)]
pub fn source_class(
    p: &ClassPicks,
    seed: &mut Seed,
    code: u32,
    l: i32,
    quality: u8,
    t: i32,
    skip_act: bool,
    monster: bool,
    expansion: bool,
) -> Result<i32, PickError> {
    if code != 0 {
        return p
            .find_code(code.to_le_bytes())
            .map(|i| i as i32)
            .ok_or(PickError::Code(code));
    }
    let mut id = random_class(p, seed, l, t, skip_act, monster, expansion)?;
    if quality == 4 {
        let mut n = 0u32;
        while p.bit(id, 0) != Some(true) {
            if n + 1 >= MAGIC_LOOP_GUARD {
                return Err(PickError::Hang);
            }
            id = if n < MAGIC_RANDOM_REPICKS {
                random_class(p, seed, l, t, skip_act, monster, expansion)?
            } else {
                part_pick(p, Part::Weapons, seed, l, t, skip_act, false, expansion)
            };
            n += 1;
        }
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(code: &[u8; 4], level: u8, rarity: u8, bits: u32) -> PickRow {
        PickRow {
            code: *code,
            spawnable: 1,
            quest: 0,
            level,
            rarity,
            type_: 1,
            version: 0,
            bitfield1: bits,
        }
    }

    fn picks() -> ClassPicks {
        ClassPicks {
            rows: vec![
                row(b"axe ", 1, 1, 3),
                row(b"swd ", 1, 1, 1),
                row(b"cap ", 1, 1, 1),
                row(b"gld ", 1, 1, 0),
                row(b"hp1 ", 1, 1, 0),
            ],
            weapons: 2,
            armor: 1,
            parts: None,
        }
    }

    #[test]
    fn pick_act_uses_the_level_as_a_level_id() {
        let got: Vec<i32> = [39, 40, 108, 109, 1024].map(pick_act).to_vec();
        assert_eq!(got, vec![0, 1, 3, 4, 0]);
    }

    #[test]
    fn random_class_bands_on_one_roll() {
        // S = {1, 666}: r = 51. L 10 → gold (51 < 55); L 20 → armor
        // (45 ≤ 51 < 60); L 40 → weapons (50 ≤ 51 < 80).
        let p = picks();
        let first = |l| {
            let mut s = Seed::new(1, 666);
            let r = {
                let mut c = s;
                c.roll(100)
            };
            assert_eq!(r, 51);
            random_class(&p, &mut s, l, -1, true, false, true).unwrap()
        };
        assert_eq!(first(10), 3);
        assert_eq!(first(20), 2);
        assert!((0..2).contains(&first(40)));
    }

    #[test]
    fn random_class_fatals() {
        let p = picks();
        let mut s = Seed::new(1, 666);
        assert_eq!(
            random_class(&p, &mut s, 66, -1, true, false, true),
            Err(PickError::Level(66))
        );
        let mut q = picks();
        q.rows[3].code = *b"xxx ";
        assert_eq!(
            random_class(&q, &mut s, 10, -1, true, false, true),
            Err(PickError::NoGold)
        );
    }

    #[test]
    fn part_pick_filters_and_draws_per_row() {
        let mut p = picks();
        // Rarity 3 at act 0: d = 3 → one roll per weapon row.
        p.rows[0].rarity = 3;
        p.rows[1].rarity = 3;
        let mut s = Seed::new(1, 666);
        let mut model = s;
        let a = model.roll(3) == 0;
        let b = model.roll(3) == 0;
        let k = u32::from(a) + u32::from(b);
        let id = part_pick(&p, Part::Weapons, &mut s, 1, -1, false, false, true);
        if k > 0 {
            model.roll(k as i32);
            assert!(id == 0 || id == 1);
        } else {
            assert_eq!(id, -1);
        }
        assert_eq!(s, model);
        // Level above L, quest, not spawnable, classic version ≥ 100:
        // no candidate, no pick draw.
        let mut q = picks();
        q.rows[2].level = 5;
        let mut s2 = Seed::new(1, 666);
        assert_eq!(
            part_pick(&q, Part::Armor, &mut s2, 4, -1, true, false, true),
            -1
        );
        assert_eq!(s2, Seed::new(1, 666));
        q.rows[2].level = 1;
        q.rows[2].version = 100;
        assert_eq!(
            part_pick(&q, Part::Armor, &mut s2, 4, -1, true, false, false),
            -1
        );
        assert_eq!(
            part_pick(&q, Part::Armor, &mut s2, 4, -1, true, false, true),
            2
        );
    }

    #[test]
    fn misc_skips_body_parts_unless_a_monster_drops() {
        let mut p = picks();
        p.rows[3].type_ = TYPE_BODY;
        p.rows[4].type_ = TYPE_BODY;
        let mut s = Seed::new(1, 666);
        assert_eq!(
            part_pick(&p, Part::Misc, &mut s, 1, -1, true, false, true),
            -1
        );
        let id = part_pick(&p, Part::Misc, &mut s, 1, -1, true, true, true);
        assert!(id == 3 || id == 4);
    }

    #[test]
    fn weapon_rack_keeps_bit_one_or_the_sixth_pick() {
        let p = picks();
        let mut s = Seed::new(1, 666);
        let id = weapon_rack_pick(&p, &mut s, 1, -1, true, true);
        assert_eq!(id, 0);
        let mut q = picks();
        q.rows[0].bitfield1 = 0;
        let mut s = Seed::new(1, 666);
        let mut model = s;
        let id = weapon_rack_pick(&q, &mut s, 1, -1, true, true);
        for _ in 0..WEAPON_TRIES {
            model.roll(2);
        }
        assert_eq!(s, model);
        assert!(id == 0 || id == 1);
    }

    #[test]
    fn source_class_code_and_magic_loop() {
        let p = picks();
        let mut s = Seed::new(1, 666);
        let gld = u32::from_le_bytes(*b"gld ");
        assert_eq!(
            source_class(&p, &mut s, gld, 5, 2, -1, false, false, true),
            Ok(3)
        );
        assert_eq!(s, Seed::new(1, 666));
        assert_eq!(
            source_class(&p, &mut s, 0x2020_2020, 5, 2, -1, false, false, true),
            Err(PickError::Code(0x2020_2020))
        );
        // Quality 4: gold (bit 0 clear) is re-picked until a magic-able row.
        let id = source_class(&p, &mut s, 0, 10, 4, -1, true, false, true).unwrap();
        assert_eq!(p.bit(id, 0), Some(true));
    }
}
