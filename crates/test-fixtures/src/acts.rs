//! All five acts in one synthetic set ([`all_acts`]), so a game that
//! creates every act at start (the play app's live build) runs on made-up
//! data. Each act variant ([`crate::act1`] … [`crate::act5`]) replaces the
//! level-type tables for its own act only; this module merges them:
//!
//! - `levels`: each id from the act that owns it (0..=39 Act I, 40..=74
//!   Act II, 75..=102 Act III, 103..=108 Act IV, 109.. Act V).
//! - `lvlprest` (`Def` = row): each `Def` from the act
//!   whose generator stamps it ([`DEF_OWNERS`]); the presets found by
//!   `LevelId` whose own `Def` is taken by another act (the Act II and
//!   Act IV towns, the Act III preset dungeons) are appended after the
//!   last owned `Def`. Act III's maze filler rows (659..=1100) are not
//!   kept: maze levels are generated when entered, not at act creation,
//!   so Act III's mazes are out of this set.
//! - `lvltypes`: each row from the act whose levels use it; Act IV's
//!   made-up type 16 (Act II's desert in the Act II set) is moved to
//!   [`ACT4_STEPPES`].
//! - `lvlsub`, `lvlmaze`, the DRLG files and the strings: the union
//!   (the first act's row wins for a key two acts share).
//!
//! Every other table is the base set's, as in each variant. Made up: the
//! moved `Def`s and the moved level type's id.

use crate::synth::{Synthetic, TxtFile};
use crate::{act1, act2, act3, act4, act5};

/// The `Def` ranges each act's generator stamps, by act index.
pub const DEF_OWNERS: [(u32, u32, usize); 5] = [
    (0, 165, 0),
    (166, 528, 1),
    (529, 658, 2),
    (659, 862, 3),
    (863, 990, 4),
];

/// Act IV's level type in the merged set (16 in [`act4::act4`]).
pub const ACT4_STEPPES: u32 = 32;

/// The act of level `id`.
pub fn act_of(id: u32) -> usize {
    match id {
        ..=39 => 0,
        40..=74 => 1,
        75..=102 => 2,
        103..=108 => 3,
        _ => 4,
    }
}

fn col(f: &TxtFile, name: &str) -> usize {
    f.columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("{}: no column {name}", f.name))
}

fn num(s: &str) -> u32 {
    s.trim().parse().unwrap_or(0)
}

/// The five act sets merged (module docs).
pub fn all_acts() -> Synthetic {
    let acts = [
        act1::act1(),
        act2::act2(),
        act3::act3(),
        act4::act4(),
        act5::act5(),
    ];
    let mut out = acts[0].clone();
    // Lookups borrow the act tables and clone only the rows they keep: a
    // whole-table copy per lookup made this merge take half a minute.
    let tab = |a: usize, txt: &str| acts[a].tables.file(txt);
    let get = |a: usize, txt: &str| tab(a, txt).clone();

    // levels: the owner's row of each id; Act IV's type moved.
    let mut levels = get(0, "levels.txt");
    levels.rows.clear();
    let (id_c, type_c) = (col(&levels, "Id"), col(&levels, "LevelType"));
    let act_c = col(&levels, "Act");
    let last = acts
        .iter()
        .flat_map(|a| a.tables.file("levels.txt").rows.iter())
        .map(|r| num(&r[id_c]))
        .max()
        .unwrap_or(0);
    for id in 0..=last {
        let a = act_of(id);
        let find = |b: usize| {
            tab(b, "levels.txt")
                .rows
                .iter()
                .find(|r| num(&r[id_c]) == id)
                .cloned()
        };
        // Past the owner's rows: a later act's placeholder (DrlgType 0),
        // in the owner's act.
        let mut row = find(a).unwrap_or_else(|| {
            let mut r = (a + 1..5)
                .find_map(find)
                .unwrap_or_else(|| panic!("no levels row {id}"));
            r[act_c] = a.to_string();
            r
        });
        if a == 3 && num(&row[type_c]) == act4::STEPPES {
            row[type_c] = ACT4_STEPPES.to_string();
        }
        levels.rows.push(row);
    }

    // lvltypes: the row of the act whose levels use the type.
    let mut types = get(0, "lvltypes.txt");
    let mut owner = vec![None; ACT4_STEPPES as usize + 1];
    for r in &levels.rows {
        let t = num(&r[type_c]) as usize;
        if t > 1 && owner[t].is_none() {
            owner[t] = Some(act_of(num(&r[id_c])));
        }
    }
    types.rows.clear();
    for (t, o) in owner.iter().enumerate() {
        let (a, src) = match (t as u32, o) {
            (ACT4_STEPPES, _) => (3, act4::STEPPES as usize),
            (_, Some(a)) => (*a, t),
            // Types 0 and 1 (none, towns and presets) and unused rows:
            // Act I's, else the first act that has the row.
            _ => (
                (0..5)
                    .find(|&a| tab(a, "lvltypes.txt").rows.len() > t)
                    .unwrap(),
                t,
            ),
        };
        types.rows.push(tab(a, "lvltypes.txt").rows[src].clone());
    }

    // lvlprest: Def = row.
    let mut prest = get(0, "lvlprest.txt");
    let (def_c, lid_c) = (col(&prest, "Def"), col(&prest, "LevelId"));
    prest.rows.clear();
    let mut moved = Vec::new();
    for &(lo, hi, a) in &DEF_OWNERS {
        let src = tab(a, "lvlprest.txt");
        for def in lo..=hi {
            // Past the owner's rows: a filler row (no level) of another act.
            let row = src.rows.get(def as usize).cloned().unwrap_or_else(|| {
                (0..5)
                    .filter_map(|b| tab(b, "lvlprest.txt").rows.get(def as usize).cloned())
                    .find(|r| num(&r[lid_c]) == 0)
                    .unwrap_or_else(|| panic!("no lvlprest row {def}"))
            });
            assert_eq!(num(&row[def_c]), def, "act {a}: Def = row");
            prest.rows.push(row);
        }
        // This act's presets by level id outside its own range.
        for r in &src.rows {
            let d = num(&r[def_c]);
            let level = num(&r[lid_c]);
            if level != 0 && !(lo..=hi).contains(&d) && act_of(level) == a {
                moved.push(r.clone());
            }
        }
    }
    for mut r in moved {
        r[def_c] = prest.rows.len().to_string();
        prest.rows.push(r);
    }

    // lvlsub, lvlmaze: the union by key.
    let union = |txt: &str, key: &str| {
        let mut f = get(0, txt);
        let k = col(&f, key);
        for a in 1..5 {
            for r in &tab(a, txt).rows {
                if !f.rows.iter().any(|x| x[k] == r[k]) {
                    f.rows.push(r.clone());
                }
            }
        }
        f
    };
    let subs = union("lvlsub.txt", "Type");
    let mazes = union("lvlmaze.txt", "Level");

    for f in [levels, types, prest, subs, mazes] {
        out.tables.files.insert(f.name.to_ascii_lowercase(), f);
    }
    for a in &acts[1..] {
        for (name, bytes) in &a.files {
            if !out.files.iter().any(|(n, _)| n == name) {
                out.files.push((name.clone(), bytes.clone()));
            }
        }
        for s in &a.strings.base {
            if !out.strings.base.iter().any(|(k, _)| *k == s.0) {
                out.strings.base.push(s.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_and_def_is_there_once() {
        let s = all_acts();
        let levels = s.tables.file("levels.txt");
        let id = col(levels, "Id");
        for (i, r) in levels.rows.iter().enumerate() {
            assert_eq!(num(&r[id]), i as u32);
        }
        let prest = s.tables.file("lvlprest.txt");
        let (def, lid) = (col(prest, "Def"), col(prest, "LevelId"));
        for (i, r) in prest.rows.iter().enumerate() {
            assert_eq!(num(&r[def]), i as u32, "Def = row");
        }
        for town in [act1::TOWN, act2::TOWN, act3::TOWN, act4::TOWN, act5::TOWN] {
            let n = prest.rows.iter().filter(|r| num(&r[lid]) == town).count();
            assert_eq!(n, 1, "town {town} found once by LevelId");
        }
    }
}
