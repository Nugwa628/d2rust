//! The level requirement of an item, as a function of
//! the values it reads. Gathering those values (affix rows, set and
//! unique rows, the item's extended stat list, skill rows) is the
//! caller's: [`LevelReqItem`] holds them already read.

/// Highest character level minus one (caps crafted items at
/// max level(0) − 1).
pub const CRAFTED_CAP: i32 = 98;

/// Quality codes read here (item data +0).
mod quality {
    pub const MAGIC: u8 = 4;
    pub const SET: u8 = 5;
    pub const RARE: u8 = 6;
    pub const UNIQUE: u8 = 7;
    pub const CRAFTED: u8 = 8;
}

/// The columns of one magic affix row (`levelreq`, `class`,
/// `classlevelreq`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AffixReq {
    pub levelreq: i32,
    /// Class restriction; 0xFF = none.
    pub class: u8,
    pub classlevelreq: i32,
}

/// The unit the requirement is computed for (only class-specific values
/// depend on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelReqUnit {
    /// Unit class (unit +4).
    pub class: u32,
    /// A player (= 0).
    pub player: bool,
    /// Unit flag 0x2000000 (expansion).
    pub expansion: bool,
}

/// Everything reads of one item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelReqItem {
    /// Quality (item data +0); 2 when the item has no item data.
    pub quality: u8,
    /// Affix rows of item data +0x36 (automagic), +0x38 + 2i (prefix i),
    /// +0x3E + 2i (suffix i); none = id 0 or an unknown id.
    pub automagic: Option<AffixReq>,
    pub prefixes: [Option<AffixReq>; 3],
    pub suffixes: [Option<AffixReq>; 3],
    /// setitems `lvl req` of the file index.
    pub set_lvlreq: i16,
    /// uniqueitems `lvl req` of the file index; none =
    /// file index < 0 or no row.
    pub unique_lvlreq: Option<i16>,
    /// Item version (item data +0x30).
    pub version: u16,
    /// items `levelreq` of the item class.
    pub class_levelreq: i32,
    /// The items of the item's own inventory (socket fillers), in list
    /// order.
    pub fillers: Vec<LevelReqItem>,
    /// skills `reqlevel` of each stat-107 (`item_singleskill`) entry.
    pub single_skills: Vec<i32>,
    /// (skills `reqlevel`, skills `charclass`) of each stat-97
    /// (`item_nonclassskill`) entry.
    pub nonclass_skills: Vec<(i32, i32)>,
    /// Item stat 92 (`item_levelreq`).
    pub stat_levelreq: i32,
}

/// "Affix value" of a magic affix row.
pub fn affix_value(a: &AffixReq, unit: Option<&LevelReqUnit>) -> i32 {
    match unit {
        Some(u) if a.class != 0xFF && u.class == u32::from(a.class) => a.classlevelreq,
        _ => a.levelreq,
    }
}

fn max_affix<'a>(
    it: impl Iterator<Item = &'a Option<AffixReq>>,
    unit: Option<&LevelReqUnit>,
) -> i32 {
    it.flatten().map(|a| affix_value(a, unit)).fold(0, i32::max)
}

/// Level requirement of `item` for `unit` (none: no
/// unit given). Never negative.
pub fn level_requirement(item: &LevelReqItem, unit: Option<&LevelReqUnit>) -> i32 {
    let six = || item.prefixes.iter().chain(item.suffixes.iter());
    let mut r = match item.quality {
        quality::MAGIC => max_affix(
            [&item.prefixes[0], &item.suffixes[0], &item.automagic].into_iter(),
            unit,
        ),
        quality::SET => i32::from(item.set_lvlreq).max(0),
        quality::RARE => max_affix(six().chain(std::iter::once(&item.automagic)), unit),
        quality::UNIQUE => {
            let classic_on_old = unit.is_some_and(|u| !u.expansion) && item.version == 0;
            match item.unique_lvlreq {
                Some(v) if !classic_on_old => i32::from(v).max(0),
                _ => 0,
            }
        }
        quality::CRAFTED => {
            let present = six().flatten().count() as i32;
            (max_affix(six(), unit) + 10 + 3 * present).min(CRAFTED_CAP)
        }
        _ => 0,
    };
    r = r.max(item.class_levelreq);
    for f in &item.fillers {
        r = r.max(level_requirement(f, unit));
    }
    for &req in &item.single_skills {
        r = r.max(req);
    }
    for &(req, charclass) in &item.nonclass_skills {
        let own = unit.is_some_and(|u| {
            u.player && (0..=6).contains(&charclass) && u.class == charclass as u32
        });
        r = r.max(if own { req } else { req.wrapping_add(6) });
    }
    let sum = r.wrapping_add(item.stat_levelreq);
    if sum < 1 {
        0
    } else {
        sum
    }
}
