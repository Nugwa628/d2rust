//! Every table view a game is built from, loaded from the user's archives.
//!
//! [`GameTables`] holds the loaded set (`bin::load`), `AnimData.d2` and the
//! fixed-up set (`fixup::apply`), and builds the views the sim and the wired
//! host read: skills, combat, vitals, monsters, objects, items, vendors,
//! hirelings. It is the production counterpart of the game-file tests'
//! builders (`test-fixtures` `game::GameData`): the same calls, nothing
//! added. A missing or malformed table is an error naming it.

use d2_data::bin::{self, BinSet, BinTable};
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{
    decode_all, Difficultylevels, Levels, Missiles, Monequip, Monlvl, Monprop, Monstats, Monstats2,
    Monumod, Objects, Record, Shrines, Superuniques,
};
use d2_data::tables::{Armor, Misc, Weapons};
use d2_formats::animdata::AnimData;
use d2_formats::mpq::ArchiveSet;
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::items::ItemTables;
use d2_sim::monsters::ai::skill_modes;
use d2_sim::monsters::init::{component_counts, monstats_extra, NamedIds};
use d2_sim::monsters::population::PopTables;
use d2_sim::skills::{SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{StatData, StateTable};
use d2_sim::treasure::class_pick::ClassPicks;
use d2_sim::units::hooks::UnitData;
use d2_sim::wiring::action::ActionTables;
use d2_sim::wiring::worldgen::WorldTables;
use d2_sim::world::npc::HireRow;
use d2_sim::world::objects::ObjectTables;
use d2_sim::world::vendors::VendorTables;

use super::WorldDataError;

fn err(table: &str, detail: impl ToString) -> WorldDataError {
    WorldDataError::Table {
        table: table.to_owned(),
        detail: detail.to_string(),
    }
}

/// The loaded set, `AnimData.d2` and the fixed-up set of one install.
#[derive(Clone, Debug)]
pub struct GameTables {
    /// The loaded set: the skill, combat and vitals views read it.
    pub bins: BinSet,
    /// The parsed `AnimData.d2`.
    pub anim: AnimData,
    /// The fixed-up set: every other view.
    pub fixed: FixedSet,
}

impl GameTables {
    /// Loads the set (`bin::DEFAULT_LANGUAGE`), `AnimData.d2` and the
    /// fix-ups from `archives`.
    pub fn load(archives: &ArchiveSet) -> Result<Self, WorldDataError> {
        let bins =
            bin::load(archives, bin::DEFAULT_LANGUAGE).map_err(|e| err("bin", e.to_string()))?;
        let anim = fixup::read_animdata(archives).map_err(|e| err("AnimData.d2", e))?;
        Self::from_loaded(bins, anim)
    }

    /// From a loaded set and its `AnimData.d2`: the fix-ups applied.
    pub fn from_loaded(bins: BinSet, anim: AnimData) -> Result<Self, WorldDataError> {
        let fixed = fixup::apply(&bins, &anim).map_err(|e| err("fixup", e))?;
        Ok(Self { bins, anim, fixed })
    }

    /// A fixed-up table by name.
    pub fn table(&self, name: &str) -> Result<&BinTable, WorldDataError> {
        self.fixed
            .table(name)
            .ok_or_else(|| err(name, "not loaded"))
    }

    /// A fixed-up table's typed rows.
    pub fn rows<T: Record>(&self) -> Result<Vec<T>, WorldDataError> {
        decode_all(self.table(T::TABLE)?).map_err(|e| err(T::TABLE, e))
    }

    /// Missiles, skills (at the level cap `LEVEL_CAP_114D`), combat, levels and the monster
    /// skill modes.
    pub fn action_tables(&self) -> Result<ActionTables, WorldDataError> {
        Ok(ActionTables {
            missiles: self.rows::<Missiles>()?,
            skills: SkillTables::from_bin(&self.bins, LEVEL_CAP_114D)
                .map_err(|e| err("skills", e))?,
            combat: CombatTables::from_bin(&self.bins).map_err(|e| err("combat", e))?,
            levels: self.rows::<Levels>()?,
            skill_modes: skill_modes(self.table("monstats")?),
            overlay_count: i32::try_from(self.table("overlay")?.count).unwrap_or(i32::MAX),
            monequip: self.rows::<Monequip>()?,
            arena: self.rows::<d2_data::tables::Arena>()?,
        })
    }

    /// The object code's tables (`objects`, `shrines`, `levels`,
    /// `objgroup`, `leveldefs`).
    pub fn object_tables(&self) -> Result<ObjectTables, WorldDataError> {
        Ok(ObjectTables {
            objects: self.rows::<Objects>()?,
            shrines: self.rows::<Shrines>()?,
            levels: self.rows::<Levels>()?,
            objgroup: self.rows::<d2_data::tables::Objgroup>()?,
            leveldefs: self.rows::<d2_data::tables::Leveldefs>()?,
        })
    }

    /// The pick columns of the combined items array (weapons, armor,
    /// misc) the object and quest drop helpers read; a host sets them on
    /// its drop state (`DeathDrops::with_picks`).
    pub fn class_picks(&self) -> Result<ClassPicks, WorldDataError> {
        Ok(ClassPicks::new(
            &self.rows::<Weapons>()?,
            &self.rows::<Armor>()?,
            &self.rows::<Misc>()?,
        ))
    }

    /// Population and monster-init tables.
    pub fn world_tables(&self) -> Result<WorldTables, WorldDataError> {
        let levels = self.rows::<Levels>()?;
        let monstats = self.rows::<Monstats>()?;
        let monstats2 = self.rows::<Monstats2>()?;
        let superuniques = self.rows::<Superuniques>()?;
        Ok(WorldTables {
            pop: PopTables::from_records(&levels, &monstats, &monstats2, &superuniques)
                .with_bins(self.table("monstats")?, self.table("monstats2")?),
            monstats,
            monstats2,
            monlvl: self.rows::<Monlvl>()?,
            levels,
            monprop: self.rows::<Monprop>()?,
            monequip: self.rows::<Monequip>()?,
            monumod: self.rows::<Monumod>()?,
            superuniques,
            difficultylevels: self.rows::<Difficultylevels>()?,
            monstats_extra: monstats_extra(self.table("monstats")?),
            components: component_counts(self.table("monstats2")?),
            ids: NamedIds::default(),
            montype_equiv: self.fixed.montype_equiv.clone(),
        })
    }

    /// The monsters' skill sequences: monstats slot sequences and `monseq`
    /// rows.
    pub fn monster_sequences(
        &self,
    ) -> Result<d2_sim::skills::sequences::MonsterSequences, WorldDataError> {
        Ok(d2_sim::skills::sequences::MonsterSequences::from_tables(
            self.table("monstats")?,
            &self.rows::<d2_data::tables::Monseq>()?,
        ))
    }

    /// The life / mana / stamina tables, from the loaded set.
    pub fn vitals(&self) -> Result<VitalsTables, WorldDataError> {
        VitalsTables::from_bin(&self.bins).map_err(|e| err("vitals", e))
    }

    /// The skill bodies' table data (itemstatcost flags, state groups,
    /// overlays, monlvl, pettype).
    pub fn body_tables(&self) -> Result<d2_sim::skills::use_::bodies::BodyTables, WorldDataError> {
        d2_sim::skills::use_::bodies::BodyTables::from_bin(&self.bins).map_err(|e| err("bodies", e))
    }

    /// The stat definitions (`itemstatcost`, `charstats`, `states`,
    /// `monstats`, `skills`).
    pub fn stat_data(&self) -> Result<StatData, WorldDataError> {
        let states = StateTable::new(self.table("states")?, &self.fixed.states)
            .map_err(|e| err("states", e))?;
        StatData::new(
            self.table("itemstatcost")?,
            self.table("charstats")?,
            states,
            self.table("monstats")?,
            self.table("skills")?,
        )
        .map_err(|e| err("stat data", e))
    }

    /// Unit data of a game of `expansion` (`UnitData::expansion`; the
    /// session's creation fields write it again).
    pub fn unit_data(&self, expansion: bool) -> Result<UnitData, WorldDataError> {
        let u = UnitData::new(self.table("monstats")?, self.table("monstats2")?)
            .map_err(|e| err("unit data", e))?;
        Ok(UnitData { expansion, ..u })
    }

    /// The `hireling` rows of the NPC control.
    pub fn hire_rows(&self) -> Result<Vec<HireRow>, WorldDataError> {
        HireRow::from_table(self.table("hireling")?).map_err(|e| err("hireling", e))
    }

    /// The item tables, from the fixed-up set.
    pub fn item_tables(&self) -> Result<ItemTables, WorldDataError> {
        ItemTables::from_fixed(&self.fixed).map_err(|e| err("items", e))
    }

    /// The vendor tables, from the fixed-up set.
    pub fn vendor_tables(&self) -> Result<VendorTables, WorldDataError> {
        VendorTables::from_fixed(&self.fixed).map_err(|e| err("vendors", e))
    }
}
