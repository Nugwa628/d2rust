//! The tables the play HUD reads ([`HudTables`]): each skill's `charclass` and
//! skilldesc `IconCel`, the `experience` rows, the `stambarblue` states and the
//! monster and difficulty rows of the skill hover lines, from the user's `.bin`
//! set.

use bevy::prelude::*;
use d2_data::bin::TableFiles;
use d2_data::tables::{
    decode_all, Charstats, Difficultylevels, Experience, Monlvl, Monstats, Skilldesc, Skills,
    States,
};

use crate::ui::char_feed::{CharTables, DescRow};
use crate::ui::original::hud::{HudTables, SkillButtonFlags};
use crate::world_view::WorldViewUi;

/// The HUD tables of a live install.
pub fn hud_tables(archives: &dyn TableFiles) -> Result<HudTables, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
    let skills: Vec<Skills> = decode_all(table("skills")?).map_err(|e| e.to_string())?;
    let descs: Vec<Skilldesc> = decode_all(table("skilldesc")?).map_err(|e| e.to_string())?;
    let exp: Vec<Experience> = decode_all(table("experience")?).map_err(|e| e.to_string())?;
    let states: Vec<States> = decode_all(table("states")?).map_err(|e| e.to_string())?;
    let mut tables = HudTables {
        warn: warn_tables(&set)?,
        // The states with flag bit 24 (`stambarblue`).
        stambarblue: states
            .iter()
            .enumerate()
            .filter(|(_, s)| s.stambarblue)
            .filter_map(|(id, _)| u8::try_from(id).ok())
            .collect(),
        ..HudTables::default()
    };
    for (id, s) in skills.iter().enumerate() {
        // D2RUST's own, unverified: the `skilldesc` link is read as the
        // skilldesc row index.
        let (Ok(id), Some(d)) = (u16::try_from(id), descs.get(usize::from(s.skilldesc))) else {
            continue;
        };
        tables.icons.insert(id, (s.charclass, d.iconcel));
        tables.descs.insert(
            id,
            crate::ui::original::skill_hover::SkillText {
                name: d.str_name,
                long: d.str_long,
                short: d.str_short,
            },
        );
    }
    // The hover box's summon and curse lines (line
    // types 13, 31, 34, 39, 41): `monstats`, `monlvl` and
    // `difficultylevels` `AiCurseDivisor`; left empty when a table is
    // missing (those lines then show nothing).
    let rows = |name: &str| set.table(name);
    if let Some(Ok(m)) = rows("monstats").map(decode_all::<Monstats>) {
        tables.hover.monstats = std::sync::Arc::new(m);
    }
    if let Some(Ok(m)) = rows("monlvl").map(decode_all::<Monlvl>) {
        tables.hover.monlvl = std::sync::Arc::new(m);
    }
    if let Some(Ok(d)) = rows("difficultylevels").map(decode_all::<Difficultylevels>) {
        tables.hover.curse_divisors = d.iter().map(|r| r.aicursedivisor as i32).collect();
    }
    // The flags of the button state (the in-game, aura and passive tests and
    // `InTown`).
    // The select panel's flags and its `skilldesc` list cells.
    for (id, s) in skills.iter().enumerate() {
        if let Ok(id) = u16::try_from(id) {
            let d = descs.get(usize::from(s.skilldesc));
            tables.flags.insert(
                id,
                SkillButtonFlags {
                    in_game: s.ingame,
                    aura: s.aura,
                    passive: s.passive,
                    in_town: s.intown,
                    left_skill: s.leftskill,
                    scroll: s.scroll,
                    list_row: d.map_or(-1, |d| d.listrow as i8),
                    list_pool: d.map_or(0, |d| d.listpool),
                },
            );
        }
    }
    tables.experience = exp
        .iter()
        .map(|e| {
            [
                e.amazon,
                e.sorceress,
                e.necromancer,
                e.paladin,
                e.barbarian,
                e.druid,
                e.assassin,
            ]
        })
        .collect();
    Ok(tables)
}

/// Gives the play UI of `app` the tables of `archives`.
pub fn install_hud_tables(
    app: &mut impl crate::app::session::Setup,
    archives: &dyn TableFiles,
) -> Result<(), String> {
    let tables = hud_tables(archives)?;
    set_hud_tables(app, tables);
    Ok(())
}

/// Gives the play UI of `app` (when installed) `tables`.
pub fn set_hud_tables(app: &mut impl crate::app::session::Setup, tables: HudTables) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_hud_tables(tables);
        }
    }
}

/// The character panel's extra tables of a live install: the `charstats`
/// class keys, the `states` colour flags and each skill's skilldesc
/// `str name`, `descdam` and `descatt` (`ui::char_feed`).
pub fn char_tables(archives: &dyn TableFiles) -> Result<CharTables, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
    let chars: Vec<Charstats> = decode_all(table("charstats")?).map_err(|e| e.to_string())?;
    let states: Vec<States> = decode_all(table("states")?).map_err(|e| e.to_string())?;
    let skills: Vec<Skills> = decode_all(table("skills")?).map_err(|e| e.to_string())?;
    let descs: Vec<Skilldesc> = decode_all(table("skilldesc")?).map_err(|e| e.to_string())?;
    let class_keys = chars
        .iter()
        .map(|c| {
            let n = c
                .class
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(c.class.len());
            String::from_utf8_lossy(&c.class[..n]).into_owned()
        })
        .collect();
    let state_flags = states
        .iter()
        .map(|s| {
            [
                s.armblue, s.rfblue, s.rcblue, s.rlblue, s.rpblue, s.armred, s.rfred, s.rcred,
                s.rlred, s.rpred,
            ]
            .iter()
            .enumerate()
            .fold(0u16, |a, (i, &f)| a | (u16::from(f) << i))
        })
        .collect();
    let mut skill_desc = std::collections::BTreeMap::new();
    for (id, s) in skills.iter().enumerate() {
        // D2RUST's own, unverified: the `skilldesc` link is the row index
        // (as `hud_tables` reads it).
        if let (Ok(id), Some(d)) = (u16::try_from(id), descs.get(usize::from(s.skilldesc))) {
            skill_desc.insert(
                id,
                DescRow {
                    name_id: d.str_alt,
                    descdam: d.descdam,
                    descatt: d.descatt,
                    src_dam: s.srcdam,
                },
            );
        }
    }
    let weapons: Vec<d2_data::tables::Weapons> =
        decode_all(table("weapons")?).map_err(|e| e.to_string())?;
    let weapons = weapons
        .iter()
        .map(|w| {
            (
                w.code,
                crate::ui::char_feed::WeaponRow {
                    str_bonus: i32::from(w.strbonus as i16),
                    dex_bonus: i32::from(w.dexbonus as i16),
                },
            )
        })
        .collect();
    let desc = desc_tables(&set, &states)?;
    Ok(CharTables {
        class_keys,
        state_flags,
        skill_desc,
        tohit_factor: chars.iter().map(|c| c.tohitfactor as i32).collect(),
        weapons,
        desc,
    })
}

/// The item tables of the equipment warning icons: `itemtypes` `Shoots` /
/// `equiv` and the type columns of the item rows.
fn warn_tables(set: &d2_data::bin::BinSet) -> Result<crate::ui::original::hud::WarnTables, String> {
    use crate::ui::original::hud::{WarnItem, WarnTables, WarnType};
    use d2_data::tables::{Armor, Itemtypes, Misc, Weapons};
    let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
    let mut items = std::collections::BTreeMap::new();
    let weapons: Vec<Weapons> = decode_all(table("weapons")?).map_err(|e| e.to_string())?;
    for w in &weapons {
        items.insert(
            w.code,
            WarnItem {
                type1: w.type_,
                type2: w.type2,
                weapon: true,
                stackable: w.stackable != 0,
            },
        );
    }
    let armor: Vec<Armor> = decode_all(table("armor")?).map_err(|e| e.to_string())?;
    for a in &armor {
        items.insert(
            a.code,
            WarnItem {
                type1: a.type_,
                type2: a.type2,
                weapon: false,
                stackable: a.stackable != 0,
            },
        );
    }
    let misc: Vec<Misc> = decode_all(table("misc")?).map_err(|e| e.to_string())?;
    for m in &misc {
        items.entry(m.code).or_insert(WarnItem {
            type1: m.type_,
            type2: m.type2,
            weapon: false,
            stackable: m.stackable != 0,
        });
    }
    let itypes: Vec<Itemtypes> = decode_all(table("itemtypes")?).map_err(|e| e.to_string())?;
    let itypes = itypes
        .iter()
        .map(|t| WarnType {
            shoots: t.shoots,
            equiv1: t.equiv1,
            equiv2: t.equiv2,
            throwable: t.throwable != 0,
        })
        .collect();
    Ok(WarnTables { itypes, items })
}

/// The tables of the `descdam` / `descatt` functions (`ui::char_desc`):
/// the skills tables and `skilldesccode`, the item rows of `weapons`,
/// `armor` and `misc`, `itemtypes` nesting and `throwable`, the `states`
/// value-colour flags and forms.
fn desc_tables(
    set: &d2_data::bin::BinSet,
    states: &[States],
) -> Result<crate::ui::char_desc::DescTables, String> {
    use crate::ui::char_desc::{DescTables, ItemRow, ItypeRow, ATTBLUE, ATTRED, DAMBLUE, DAMRED};
    use d2_data::tables::{Armor, Itemtypes, Misc, Weapons};
    let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
    let skills = d2_sim::skills::SkillTables::from_bin(set, d2_sim::skills::LEVEL_CAP_114D)
        .map_err(|e| e.to_string())?;
    let desc_code = set
        .code
        .get(&d2_data::schema::CalcBuffer::SkillDescCode)
        .map(|c| c.bytes.clone())
        .ok_or("skilldesccode not loaded")?;
    let mut items = std::collections::BTreeMap::new();
    let weapons: Vec<Weapons> = decode_all(table("weapons")?).map_err(|e| e.to_string())?;
    for w in &weapons {
        items.insert(
            w.code,
            ItemRow {
                type1: w.type_,
                type2: w.type2,
                weapon: true,
                two_handed: w.f_2handed != 0,
                one_or_two: w.f_1or2handed != 0,
                mindam: w.mindam,
                maxdam: w.maxdam,
                str_bonus: i32::from(w.strbonus as i16),
                dex_bonus: i32::from(w.dexbonus as i16),
                missile_type: w.missiletype as i16,
            },
        );
    }
    let armor: Vec<Armor> = decode_all(table("armor")?).map_err(|e| e.to_string())?;
    for a in &armor {
        items.insert(
            a.code,
            ItemRow {
                type1: a.type_,
                type2: a.type2,
                weapon: false,
                two_handed: a.f_2handed != 0,
                one_or_two: a.f_1or2handed != 0,
                mindam: a.mindam,
                maxdam: a.maxdam,
                str_bonus: i32::from(a.strbonus as i16),
                dex_bonus: i32::from(a.dexbonus as i16),
                missile_type: -1,
            },
        );
    }
    let misc: Vec<Misc> = decode_all(table("misc")?).map_err(|e| e.to_string())?;
    for m in &misc {
        items.entry(m.code).or_insert(ItemRow {
            type1: m.type_,
            type2: m.type2,
            ..ItemRow::default()
        });
    }
    let itypes: Vec<Itemtypes> = decode_all(table("itemtypes")?).map_err(|e| e.to_string())?;
    let itypes = itypes
        .iter()
        .map(|t| ItypeRow {
            equiv1: t.equiv1,
            equiv2: t.equiv2,
            throwable: t.throwable != 0,
        })
        .collect();
    let state_colors = states
        .iter()
        .map(|s| {
            let mut f = 0;
            if s.damblue {
                f |= DAMBLUE;
            }
            if s.damred {
                f |= DAMRED;
            }
            if s.attblue {
                f |= ATTBLUE;
            }
            if s.attred {
                f |= ATTRED;
            }
            f
        })
        .collect();
    let form_states = states
        .iter()
        .enumerate()
        .filter(|(_, s)| s.restrict)
        .filter_map(|(i, _)| u8::try_from(i).ok())
        .collect();
    let pgsv_states = states
        .iter()
        .enumerate()
        .filter(|(_, s)| s.pgsv)
        .filter_map(|(i, _)| u8::try_from(i).ok())
        .collect();
    Ok(DescTables {
        skills: Some(std::sync::Arc::new(skills)),
        desc_code,
        items,
        itypes,
        state_colors,
        form_states,
        pgsv_states,
    })
}

/// Gives the play UI of `app` the character panel's tables.
pub fn install_char_tables(
    app: &mut impl crate::app::session::Setup,
    archives: &dyn TableFiles,
) -> Result<(), String> {
    let tables = char_tables(archives)?;
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_char_tables(tables);
        }
    }
    Ok(())
}

/// The skill tree's class skill rows of a live install
/// ([`crate::ui::skill_tree_ui::SkillTreeTables`]): `skills` joined with
/// `skilldesc` (page, row, column, `IconCel`) by the `skilldesc` link.
pub fn skill_tree_tables(
    archives: &dyn TableFiles,
) -> Result<crate::ui::skill_tree_ui::SkillTreeTables, String> {
    use crate::ui::skill_tree_ui::{SkillTreeRow, SkillTreeTables};
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
    let skills: Vec<Skills> = decode_all(table("skills")?).map_err(|e| e.to_string())?;
    let descs: Vec<Skilldesc> = decode_all(table("skilldesc")?).map_err(|e| e.to_string())?;
    let mut t = SkillTreeTables::default();
    for (id, s) in skills.iter().enumerate() {
        let (Ok(id), Some(d)) = (u16::try_from(id), descs.get(usize::from(s.skilldesc))) else {
            continue;
        };
        t.rows.push(SkillTreeRow {
            skill: id,
            class: s.charclass,
            page: d.skillpage,
            row: d.skillrow,
            column: d.skillcolumn,
            icon_cel: d.iconcel,
            maxlvl: s.maxlvl,
            ingame: s.ingame,
            passive: s.passive,
            reqlevel: s.reqlevel,
            reqskill: [s.reqskill1, s.reqskill2, s.reqskill3],
            reqstr: s.reqstr,
            reqdex: s.reqdex,
            reqint: s.reqint,
            reqvit: s.reqvit,
        });
    }
    Ok(t)
}

/// Gives the play UI of `app` (when installed) the skill tree rows
/// of `archives`.
pub fn install_skill_tree_tables(
    app: &mut impl crate::app::session::Setup,
    archives: &dyn TableFiles,
) -> Result<(), String> {
    let tables = skill_tree_tables(archives)?;
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_skill_tree_tables(tables);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skill button state of the learned, castable skills on the live
    /// tables (playtest B005: Fire Bolt drawn red).
    #[test]
    #[ignore = "real data: needs D2_GAME_DIR"]
    fn live_fire_bolt_button_is_not_red_in_the_field() {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
        let set = d2_formats::mpq::ArchiveSet::open_dir(&dir).unwrap();
        let t = hud_tables(&set).unwrap();
        let f = t.flags.get(&36).copied().unwrap();
        eprintln!("Fire Bolt flags {f:?}");
        let k = |level, town| {
            crate::ui::original::hud::skill_button_state(&t, 36, level, town, (0, 0), (500, 500))
        };
        assert_eq!(k(1, false), 0, "learned, outside town: usable");
        assert_eq!(k(0, false), 1, "not learned");
    }
}
