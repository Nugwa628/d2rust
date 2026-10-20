//! The start (`srvst`) and do (`srvdo`)
//! function tables as data: which slots are filled, with the base game's
//! function key and the D2MOO name of each. Bodies of status `spec'd-here`
//! are in [`super::bodies`]; the rest (`mapped`)
//! run behind [`super::SkillFunctions`]. [`check_tsv`] compares [`FUNCS`] with
//! `functions.tsv` row by row.

/// Which table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `srvstfunc` table.
    Start,
    /// `srvdofunc` table.
    Do,
}

impl Kind {
    /// The `kind` column value.
    pub fn code(self) -> &'static str {
        match self {
            Kind::Start => "srvst",
            Kind::Do => "srvdo",
        }
    }

    /// Slot count (bound check `< 0x5B` / `< 0xBF`).
    pub fn slots(self) -> u16 {
        match self {
            Kind::Start => START_SLOTS,
            Kind::Do => DO_SLOTS,
        }
    }
}

/// The `status` column of a filled slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Body written out (run by [`super::bodies`]).
    SpecdHere,
    /// Base-game table entry identified; body not written out.
    Mapped,
    /// Filled, but no base-game data uses it.
    Unreferenced,
}

impl Status {
    fn code(self) -> &'static str {
        match self {
            Status::SpecdHere => "spec'd-here",
            Status::Mapped => "mapped",
            Status::Unreferenced => "unreferenced",
        }
    }
}

/// One filled slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Func {
    pub kind: Kind,
    pub index: u16,
    /// The base game address of the body.
    pub address: u32,
    /// D2MOO name.
    pub name: &'static str,
    pub status: Status,
}

/// `srvst` slots (bound `< 0x5B`).
pub const START_SLOTS: u16 = 91;
/// `srvdo` slots (bound `< 0xBF`).
pub const DO_SLOTS: u16 = 191;

/// The filled slot `index` of table `kind`; `None` for an empty slot or
/// an index past the table.
pub fn lookup(kind: Kind, index: u16) -> Option<&'static Func> {
    FUNCS.iter().find(|f| f.kind == kind && f.index == index)
}

/// Every filled slot of both tables, in `functions.tsv` order.
pub const FUNCS: &[Func] = &[
    Func { kind: Kind::Start, index: 1, address: 0x0056CA40, name: "SrvSt01_Attack_LeftHandSwing", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 2, address: 0x0056CAF0, name: "SrvSt02_Kick", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 3, address: 0x0056CBA0, name: "SrvSt03_Unsummon", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 4, address: 0x005DA8B0, name: "SrvSt04_Arrow_Bolt", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 5, address: 0x005DA8F0, name: "SrvSt05_Jab", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 6, address: 0x005DA940, name: "SrvSt06_PowerStrike_ChargedStrike", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 7, address: 0x005DAB40, name: "SrvSt07_Impale", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 8, address: 0x005DACD0, name: "SrvSt08_Strafe", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 9, address: 0x005DAE30, name: "SrvSt09_Fend", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 10, address: 0x005DB020, name: "SrvSt10_LightningStrike", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 11, address: 0x005C8FA0, name: "SrvSt11_Inferno_ArcticBlast", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 12, address: 0x005C9030, name: "SrvSt12_Telekinesis_DragonFlight", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 13, address: 0x005C91C0, name: "SrvSt13_ThunderStorm", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 14, address: 0x005C9220, name: "SrvSt14_Hydra", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 15, address: 0x005C3070, name: "SrvSt15_RaiseSkeleton_Mage", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 16, address: 0x005C30A0, name: "SrvSt16_PoisonDagger", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 17, address: 0x005C31C0, name: "SrvSt17_Poison_CorpseExplosion", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 18, address: 0x005C3260, name: "SrvSt18_Attract", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 19, address: 0x005C3270, name: "SrvSt19_BonePrison", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 20, address: 0x005C32A0, name: "SrvSt20_IronGolem", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 21, address: 0x005C3350, name: "SrvSt21_Revive", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 22, address: 0x005D30F0, name: "SrvSt22_PsychicHammer", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 23, address: 0x005D32F0, name: "SrvSt23_AssasinChargeStrikes", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 24, address: 0x005D5970, name: "SrvSt24_DragonTalon", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 25, address: 0x005D6330, name: "SrvSt25_64_DragonClaw_MonFrenzy", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 26, address: 0x005D69D0, name: "SrvSt26_BladeFury", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 27, address: 0x005D7090, name: "SrvSt27_DragonTail", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 28, address: 0x005D7A00, name: "SrvSt28_BladeShield", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 29, address: 0x005CE790, name: "SrvSt29_Sacrifice", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 31, address: 0x005CF6B0, name: "SrvSt31_Charge", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 32, address: 0x005D7EA0, name: "SrvSt32_Conversion_Bash_Stun_Concentrate_BearSmite", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 33, address: 0x005D80C0, name: "SrvSt33_FindPotion_GrimWard", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 34, address: 0x005D8760, name: "SrvSt34_FindItem", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 35, address: 0x005CFE10, name: "SrvSt35_Vengeance", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 36, address: 0x005D0180, name: "SrvSt36_HolyShield", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 37, address: 0x005DAF40, name: "SrvSt37_Zeal_Fury_BloodLordFrenzy", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 38, address: 0x005D8F50, name: "SrvSt38_Whirlwind", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 39, address: 0x005D97F0, name: "SrvSt39_Berserk", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 40, address: 0x005D9D50, name: "SrvSt40_Leap", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 41, address: 0x005DA540, name: "SrvSt41_LeapAttack", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 42, address: 0x005CAE40, name: "SrvSt42_FireHit", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 43, address: 0x005CAF80, name: "SrvSt43_MaggotEgg", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 44, address: 0x005CB170, name: "SrvSt44_MaggotUp", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 45, address: 0x005CB270, name: "SrvSt45_MaggotDown", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 46, address: 0x005CB4D0, name: "SrvSt46_AndrialSpray", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 47, address: 0x005CB730, name: "SrvSt47_Jump", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 48, address: 0x005CBBF0, name: "SrvSt48_SwarmMove", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 49, address: 0x005CBD10, name: "SrvSt49_Nest_EvilHutSpawner", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 50, address: 0x005CBF30, name: "SrvSt50_QuickStrike", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 51, address: 0x005CC1D0, name: "SrvSt51_Submerge", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 52, address: 0x005CC220, name: "SrvSt52_Emerge", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 53, address: 0x005CC240, name: "SrvSt53_MonInferno", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 54, address: 0x005CD2C0, name: "SrvSt54_DiabRun", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 55, address: 0x005CD910, name: "SrvSt55_Mosquito", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 56, address: 0x005C7690, name: "SrvSt56_FeralRage_Maul", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 57, address: 0x005C79E0, name: "SrvSt57_Rabies", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 58, address: 0x005C7E00, name: "SrvSt58_FireClaws", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 59, address: 0x005D1280, name: "SrvSt59_ImpInferno", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 60, address: 0x005D1570, name: "SrvSt60_SuckBlood", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 61, address: 0x005D1BF0, name: "SrvSt61_SelfResurrect", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 62, address: 0x005D2420, name: "SrvSt62_MinionSpawner", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 63, address: 0x005D2A10, name: "SrvSt63_Corpse_VineCycler", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 64, address: 0x005CDF00, name: "SrvSt25_64_DragonClaw_MonFrenzy", status: Status::SpecdHere },
    Func { kind: Kind::Start, index: 65, address: 0x0056CAB0, name: "SrvSt65_Throw_LeftHandThrow", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 1, address: 0x0056F070, name: "SrvDo001_Attack_LeftHandSwing", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 2, address: 0x0056F1F0, name: "SrvDo002_Kick_PowerStrike_MonIceSpear_Impale_Bash_Stun_Concentrate_BearSmite_Vengeance_Berserk_FireClaws", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 3, address: 0x0056F460, name: "SrvDo003_Throw", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 4, address: 0x0056CC20, name: "SrvDo004_Unsummon", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 5, address: 0x0056F550, name: "SrvDo005_LeftHandThrow", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 6, address: 0x005DB1C0, name: "SrvDo006_InnerSight_SlowMissiles", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 7, address: 0x005DB2D0, name: "SrvDo007_Jab", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 8, address: 0x005DB410, name: "SrvDo008_MultipleShot_Teeth_ShockWave", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 9, address: 0x005D8E00, name: "SrvDo009_Frenzy", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 10, address: 0x005DB6D0, name: "SrvDo010_GuidedArrow_BoneSpirit", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 11, address: 0x005DB850, name: "SrvDo011_ChargedStrike", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 12, address: 0x005DBA40, name: "SrvDo012_Strafe", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 13, address: 0x005DBC60, name: "SrvDo013_Fend_Zeal_Fury", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 14, address: 0x005DBE50, name: "SrvDo014_LightningStrike", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 15, address: 0x005DC000, name: "SrvDo015_Dopplezon", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 16, address: 0x005DC1E0, name: "SrvDo016_Valkyrie", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 17, address: 0x005C9300, name: "SrvDo017_ChargedBolt_BoltSentry", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 18, address: 0x005C9480, name: "SrvDo018_DefensiveBuff", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 19, address: 0x005C9640, name: "SrvDo019_Inferno_ArcticBlast", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 20, address: 0x005C9800, name: "SrvDo020_StaticField", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 21, address: 0x005C98F0, name: "SrvDo021_Telekinesis", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 22, address: 0x005C9B50, name: "SrvDo022_NovaAttack", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 23, address: 0x005C9C10, name: "SrvDo023_Blaze_EnergyShield_SpiderLay", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 24, address: 0x005C9EA0, name: "SrvDo024_FireWall", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 25, address: 0x005CA030, name: "SrvDo025_Enchant", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 26, address: 0x005CA1B0, name: "SrvDo026_ChainLightning", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 27, address: 0x005CA360, name: "SrvDo027_Teleport", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 28, address: 0x005CA3E0, name: "SrvDo028_Meteor_Blizzard_Eruption_BaalTaunt_Catapult", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 29, address: 0x005CA4D0, name: "SrvDo029_ThunderStorm", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 30, address: 0x005C37C0, name: "SrvDo030_Curse", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 31, address: 0x005C4B00, name: "SrvDo031_RaiseSkeleton_Mage", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 32, address: 0x005C4CD0, name: "SrvDo032_PoisonDagger", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 33, address: 0x005D3140, name: "SrvDo033_PsychicHammer", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 34, address: 0x005D3490, name: "SrvDo034_TigerStrike_CobraStrike_RoyalStrike", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 35, address: 0x005D35D0, name: "SrvDo035_FistsOfFire_ClawsOfThunder_BladesOfIce", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 36, address: 0x005D4DB0, name: "SrvDo036_ClawsOfThunder_ProgressiveFn2", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 37, address: 0x005D4E70, name: "SrvDo037_ClawsOfThunder_ProgressiveFn3", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 38, address: 0x005D3E80, name: "SrvDo038_FistsOfFire_BladesOfIce_ProgressiveFn2", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 39, address: 0x005D3F90, name: "SrvDo039_FistsOfFire_BladesOfIce_ProgressiveFn3", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 40, address: 0x005D5010, name: "SrvDo040_RoyalStrike_ProgressiveFn1", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 41, address: 0x005D5080, name: "SrvDo041_RoyalStrike_ProgressiveFn3", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 42, address: 0x005D5A30, name: "SrvDo042_DragonTalon", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 43, address: 0x005D5D70, name: "SrvDo043_ShockField", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 44, address: 0x005D6020, name: "SrvDo044_BladeSentinel", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 45, address: 0x005D6170, name: "SrvDo045_Sentry", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 46, address: 0x005D6340, name: "SrvDo046_DragonClaw", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 47, address: 0x005D6630, name: "SrvDo047_CloakOfShadows", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 48, address: 0x005D68A0, name: "SrvDo048_BladeFury", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 49, address: 0x005D6E70, name: "SrvDo049_ShadowWarrior_Master", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 50, address: 0x005D7180, name: "SrvDo050_DragonTail", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 51, address: 0x005D76E0, name: "SrvDo051_MindBlast", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 52, address: 0x005D7850, name: "SrvDo052_DragonFlight", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 53, address: 0x005D7B60, name: "SrvDo053_Unused", status: Status::Unreferenced },
    Func { kind: Kind::Do, index: 54, address: 0x005D7E10, name: "SrvDo054_BladeShield", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 55, address: 0x005C4DF0, name: "SrvDo055_CorpseExplosion", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 56, address: 0x005C5100, name: "SrvDo056_Golem", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 57, address: 0x005C5250, name: "SrvDo057_IronGolem", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 58, address: 0x005C56C0, name: "SrvDo058_Revive", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 59, address: 0x005C3B90, name: "SrvDo059_Attract", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 60, address: 0x005C58B0, name: "SrvDo060_BoneWall", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 61, address: 0x005C3F20, name: "SrvDo061_Confuse", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 62, address: 0x005C5D00, name: "SrvDo062_BonePrison", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 63, address: 0x005C5E60, name: "SrvDo063_PoisonExplosion", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 64, address: 0x005CE8E0, name: "SrvDo064_Sacrifice", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 65, address: 0x005CF010, name: "SrvDo065_BasicAura", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 66, address: 0x005CF3A0, name: "SrvDo066_HolyFire_HolyShock_Sanctuary_Conviction", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 67, address: 0x005CF900, name: "SrvDo067_Charge", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 68, address: 0x005D83E0, name: "SrvDo068_BasicShout", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 69, address: 0x005D81C0, name: "SrvDo069_FindPotion", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 70, address: 0x005D8470, name: "SrvDo070_DoubleSwing", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 71, address: 0x005D8570, name: "SrvDo071_Taunt", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 72, address: 0x005D8780, name: "SrvDo072_FindItem", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 73, address: 0x005D0040, name: "SrvDo073_BlessedHammer", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 74, address: 0x005D88B0, name: "SrvDo074_DoubleThrow", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 75, address: 0x005D89B0, name: "SrvDo075_GrimWard", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 76, address: 0x005D9580, name: "SrvDo076_Whirlwind", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 77, address: 0x005DA370, name: "SrvDo077_Leap", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 78, address: 0x005DA7E0, name: "SrvDo078_LeapAttack", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 79, address: 0x005D0350, name: "SrvDo079_Conversion", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 80, address: 0x005D0670, name: "SrvDo080_FistOfTheHeavens", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 81, address: 0x005D0920, name: "SrvDo081_HolyFreeze", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 82, address: 0x005D0E90, name: "SrvDo082_Redemption", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 83, address: 0x005CAF30, name: "SrvDo083_FireHit", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 84, address: 0x005CAFA0, name: "SrvDo084_MaggotEgg", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 85, address: 0x005CB0C0, name: "SrvDo085_UnholyBolt_ShamanFire", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 86, address: 0x005CB300, name: "SrvDo086_MaggotDown", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 87, address: 0x005CB3C0, name: "SrvDo087_MaggotLay", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 88, address: 0x005CB580, name: "SrvDo088_AndrialSpray", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 89, address: 0x005CB940, name: "SrvDo089_Jump", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 90, address: 0x005CBC80, name: "SrvDo090_SwarmMove", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 91, address: 0x005CBE00, name: "SrvDo091_Nest_EvilHutSpawner", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 92, address: 0x005CBF90, name: "SrvDo092_QuickStrike", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 93, address: 0x005CC050, name: "SrvDo093_GargoyleTrap", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 94, address: 0x005CC1F0, name: "SrvDo094_Submerge", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 95, address: 0x005CC4E0, name: "SrvDo095_MonInferno", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 96, address: 0x005CC840, name: "SrvDo096_ZakarumHeal_Bestow", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 97, address: 0x005CCB10, name: "SrvDo097_Resurrect", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 98, address: 0x005CCC80, name: "SrvDo098_MonTeleport", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 99, address: 0x005CCD10, name: "SrvDo099_PrimePoisonNova", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 100, address: 0x005CCE80, name: "SrvDo100_DiabCold", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 101, address: 0x005CCFA0, name: "SrvDo101_FingerMageSpider", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 102, address: 0x005CD1C0, name: "SrvDo102_DiabWall", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 103, address: 0x005CD380, name: "SrvDo103_DiabRun", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 104, address: 0x005CD5D0, name: "SrvDo104_DiabPrison", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 105, address: 0x005CD6A0, name: "SrvDo105_DesertTurret", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 106, address: 0x005CD870, name: "SrvDo106_ArcaneTower", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 107, address: 0x005CDA00, name: "SrvDo107_Mosquito", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 108, address: 0x005CDC10, name: "SrvDo108_RegurgitatorEat", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 109, address: 0x005CDF10, name: "SrvDo109_MonFrenzy", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 110, address: 0x005CE1B0, name: "SrvDo110_Hireable_RogueMissile", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 111, address: 0x005CE670, name: "SrvDo111_FetishAura", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 112, address: 0x005CE2B0, name: "SrvDo112_MonCurseCast", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 113, address: 0x005BF3D0, name: "SrvDo113_Scroll_Book", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 114, address: 0x005C6910, name: "SrvDo114_Raven", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 115, address: 0x005C6A80, name: "SrvDo115_Vines", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 116, address: 0x005C6EC0, name: "SrvDo116_Wearwolf_Wearbear", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 117, address: 0x005C7160, name: "SrvDo117_Firestorm", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 118, address: 0x005C72F0, name: "SrvDo118_Twister_Tornado", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 119, address: 0x005C7390, name: "SrvDo119_DruidSummon", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 120, address: 0x005C77C0, name: "SrvDo120_FeralRage_Maul", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 121, address: 0x005C8AD0, name: "SrvDo121_Rabies", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 122, address: 0x005C7F10, name: "SrvDo122_Hunger", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 123, address: 0x005C8080, name: "SrvDo123_Volcano", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 124, address: 0x005C8190, name: "SrvDo124_Armageddon_Hurricane", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 125, address: 0x005D1170, name: "SrvDo125_WakeOfDestruction", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 126, address: 0x005D1350, name: "SrvDo126_ImpInferno", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 127, address: 0x005D16A0, name: "SrvDo127_SuckBlood", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 128, address: 0x005D1800, name: "SrvDo128_CryHelp", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 129, address: 0x005D1AB0, name: "SrvDo129_ImpTeleport", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 130, address: 0x005D1CD0, name: "SrvDo130_VineAttack", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 131, address: 0x005D1F70, name: "SrvDo131_OverseerWhip", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 132, address: 0x005D2090, name: "SrvDo132_ImpFireMissile", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 133, address: 0x005D2250, name: "SrvDo133_Impregnate", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 134, address: 0x005D2320, name: "SrvDo134_SiegeBeastStomp", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 135, address: 0x005D2490, name: "SrvDo135_MinionSpawner", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 136, address: 0x005D25B0, name: "SrvDo136_DeathMaul", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 137, address: 0x005D26F0, name: "SrvDo137_FenrisRage", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 138, address: 0x005D27B0, name: "SrvDo138_Unused", status: Status::Unreferenced },
    Func { kind: Kind::Do, index: 139, address: 0x005D2940, name: "SrvDo139_BaalColdMissiles", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 140, address: 0x005D2C20, name: "SrvDo140_BaalTentacle", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 141, address: 0x005D2E80, name: "SrvDo141_BaalCorpseExplode", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 142, address: 0x005D7CE0, name: "SrvDo142_Unused", status: Status::Unreferenced },
    Func { kind: Kind::Do, index: 143, address: 0x005D4F40, name: "SrvDo143_FistsOfFire_RoyalStrike_ProgressiveFn", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 144, address: 0x005CA910, name: "SrvDo144_Hydra", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 145, address: 0x005C8380, name: "SrvDo145_Unused", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 146, address: 0x005C8520, name: "SrvDo146_Unused", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 147, address: 0x005D2B20, name: "SrvDo147_Unused", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 148, address: 0x005CDFB0, name: "SrvDo148_DoomKnightMissile", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 149, address: 0x005CE0B0, name: "SrvDo149_NecromageMissile", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 150, address: 0x005CE9F0, name: "SrvDo150_Smite", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 151, address: 0x005CA260, name: "SrvDo151_Unused", status: Status::SpecdHere },
    Func { kind: Kind::Do, index: 152, address: 0x005CC690, name: "SrvDo152_DiabLight", status: Status::SpecdHere },
];

/// One disagreement between [`FUNCS`] and `functions.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// TSV line (1-based, header = 1), 0 for a slot missing from the TSV.
    pub line: usize,
    pub what: String,
}

/// Compares `functions.tsv` (text) with [`FUNCS`]: every slot of both
/// tables appears exactly once (ranges `a-b` are empty slots), each
/// filled slot has the same address, name and status, each empty slot is
/// `null`. Returns every mismatch.
pub fn check_tsv(tsv: &str) -> Vec<Mismatch> {
    let mut out = Vec::new();
    let mut seen: Vec<(Kind, u16)> = Vec::new();
    let mut lines = tsv.lines().enumerate();
    if lines.next().map(|(_, h)| h)
        != Some("kind\tindex\taddress\td2moo_name\tstatus\tskills_using\tnotes")
    {
        out.push(Mismatch {
            line: 1,
            what: "header".into(),
        });
    }
    for (i, l) in lines {
        let line = i + 1;
        if l.is_empty() {
            continue;
        }
        let c: Vec<&str> = l.split('\t').collect();
        if c.len() != 7 {
            out.push(Mismatch {
                line,
                what: format!("{} columns", c.len()),
            });
            continue;
        }
        let kind = match c[0] {
            "srvst" => Kind::Start,
            "srvdo" => Kind::Do,
            k => {
                out.push(Mismatch {
                    line,
                    what: format!("kind {k}"),
                });
                continue;
            }
        };
        let range = match c[1].split_once('-') {
            Some((a, b)) => a.parse::<u16>().ok().zip(b.parse::<u16>().ok()),
            None => c[1].parse::<u16>().ok().map(|a| (a, a)),
        };
        let Some((a, b)) = range.filter(|(a, b)| a <= b && *b < kind.slots()) else {
            out.push(Mismatch {
                line,
                what: format!("index {}", c[1]),
            });
            continue;
        };
        for index in a..=b {
            if seen.contains(&(kind, index)) {
                out.push(Mismatch {
                    line,
                    what: format!("{} {index} repeated", kind.code()),
                });
            }
            seen.push((kind, index));
            let row = (c[2], c[3], c[4]);
            match lookup(kind, index) {
                Some(f) => {
                    let want = (format!("0x{:08X}", f.address), f.name, f.status.code());
                    if a != b || (row.0, row.1, row.2) != (want.0.as_str(), want.1, want.2) {
                        out.push(Mismatch {
                            line,
                            what: format!("{} {index}: tsv {row:?}, code {want:?}", kind.code()),
                        });
                    }
                }
                None => {
                    if row != ("null", "-", "null") {
                        out.push(Mismatch {
                            line,
                            what: format!("{} {index}: tsv {row:?}, code null", kind.code()),
                        });
                    }
                }
            }
        }
    }
    for kind in [Kind::Start, Kind::Do] {
        for index in 0..kind.slots() {
            if !seen.contains(&(kind, index)) {
                out.push(Mismatch {
                    line: 0,
                    what: format!("{} {index} missing from the tsv", kind.code()),
                });
            }
        }
    }
    out
}
