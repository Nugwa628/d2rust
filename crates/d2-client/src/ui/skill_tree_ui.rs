//! The skill tree's game facts for the play app: the class skill rows
//! ([`SkillTreeTables`], from `skills` / `skilldesc`) joined with the local
//! player's stats and skill list into the entries the panel draws and
//! tests ([`entries`]). The panel (`panels::skilltree`) decides nothing
//! itself; a spent point leaves as C→S 0x3B.

use crate::bridge::skills::NATIVE;
use crate::bridge::world::{ClientUnit, ClientWorld};
use crate::ui::panels::skilltree::SkillEntry;

/// The class icon prefixes `CC` of `Spells\<CC>Skillicon` (table), by class
/// 0–6.
pub const ICON_PREFIX: [&str; 7] = ["Am", "So", "Ne", "Pa", "Ba", "Dr", "As"];

/// Stat ids the learnability test reads.
const STAT_STR: u16 = 0;
const STAT_ENERGY: u16 = 1;
const STAT_DEX: u16 = 2;
const STAT_VIT: u16 = 3;
const STAT_FREE_POINTS: u16 = 5;
const STAT_LEVEL: u16 = 12;

/// The `InGame` flag in the flag byte (mask 4).
pub const FLAG_INGAME: u8 = 4;

/// The `skills` / `skilldesc` fields one tree icon reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillTreeRow {
    pub skill: u16,
    /// `charclass`.
    pub class: u8,
    pub page: u8,
    pub row: u8,
    pub column: u8,
    pub icon_cel: u8,
    pub maxlvl: u16,
    pub ingame: bool,
    pub passive: bool,
    pub reqlevel: u16,
    /// `reqskill1`–`reqskill3` (0xFFFF: none).
    pub reqskill: [u16; 3],
    pub reqstr: u16,
    pub reqdex: u16,
    pub reqint: u16,
    pub reqvit: u16,
}

/// The class skill rows in `skills` row order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillTreeTables {
    pub rows: Vec<SkillTreeRow>,
}

/// Registered art names: the seven class icon files.
pub fn icon_files() -> Vec<String> {
    ICON_PREFIX
        .iter()
        .map(|c| format!("spells\\{}skillicon", c.to_ascii_lowercase()))
        .collect()
}

/// The icon file name of `class`.
pub fn icon_file_name(class: u8) -> Option<String> {
    let c = ICON_PREFIX.get(usize::from(class))?;
    Some(format!("spells\\{}skillicon", c.to_ascii_lowercase()))
}

/// Base and shown level of `skill` (base + bonus, at least 0). D2RUST's own,
/// unverified: the bonus is the native entry's `level_bonus` plus the base of
/// every item-owned entry of the skill.
fn levels(unit: &ClientUnit, skill: u16) -> (i32, i32, i32) {
    let Some(list) = unit.skills.as_ref() else {
        return (0, 0, 0);
    };
    let base = list.native(skill).map_or(0, |i| list.entries[i].base);
    let mut bonus = list
        .native(skill)
        .map_or(0, |i| list.entries[i].level_bonus);
    bonus += list
        .entries
        .iter()
        .filter(|e| e.skill == skill && e.owner != NATIVE)
        .map(|e| e.base)
        .sum::<i32>();
    let shown = if base != 0 || bonus != 0 {
        (base + bonus).max(0)
    } else {
        0
    };
    (base, shown, bonus)
}

/// The tree entries of the local player's class.
pub fn entries(t: &SkillTreeTables, w: &ClientWorld, class: u8) -> Vec<SkillEntry> {
    let Some(p) = w.local() else {
        return Vec::new();
    };
    let level = p.stat(STAT_LEVEL);
    let free = p.stat(STAT_FREE_POINTS);
    t.rows
        .iter()
        .filter(|r| r.class == class && (1..=3).contains(&r.page))
        .map(|r| {
            let (base, shown, bonus) = levels(p, r.skill);
            let max = if r.maxlvl == 0 || (r.maxlvl as i16) < 1 {
                20
            } else {
                i32::from(r.maxlvl as i16)
            };
            let reqs_ok = level >= i32::from(r.reqlevel)
                && r.reqskill
                    .iter()
                    .filter(|&&s| s != u16::MAX)
                    .all(|&s| levels(p, s).0 >= 1);
            let below_max = base < max;
            let stats_ok = r.ingame
                && p.stat(STAT_STR) >= i32::from(r.reqstr)
                && p.stat(STAT_DEX) >= i32::from(r.reqdex)
                && p.stat(STAT_ENERGY) >= i32::from(r.reqint)
                && p.stat(STAT_VIT) >= i32::from(r.reqvit);
            // D2RUST's own, unverified: the point cost is 1 (`skpoints` is
            // empty in the shipped table).
            let learnable = reqs_ok && below_max && stats_ok && free >= 1;
            SkillEntry {
                skill: r.skill,
                page: r.page,
                row: r.row,
                column: r.column,
                icon_cel: u32::from(r.icon_cel),
                level: shown,
                hard_points: base,
                bonus,
                flags: if r.ingame { FLAG_INGAME } else { 0 },
                learnable,
                passive: r.passive,
                req_level_ok: reqs_ok,
                below_max,
            }
        })
        .collect()
}
