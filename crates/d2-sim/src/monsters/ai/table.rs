//! The AI table (148 records) and the special-state table
//! (18 records). `AI_TABLE` is copied from `ai-functions.tsv`
//! columns `index`, `think_1_14d`, `init_1_14d`, `alt_1_14d`,
//! `target_mode` and checked against it by `tests::ai_table_matches_tsv`;
//! [`SPECD_HERE`] mirrors the `status` column, checked row
//! by row by `tests::specd_here_matches_tsv`; the special-state records
//! are the special-state table of the AI install.

/// One 16-byte AI table record (D2MOO `D2AiTableStrc`). Functions are
/// their the base game addresses; 0 = none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiRecord {
    /// +0x00 target mode.
    pub target_mode: u32,
    /// +0x04 init function.
    pub init: u32,
    /// +0x08 think function.
    pub think: u32,
    /// +0x0C alternate function.
    pub alt: u32,
}

const fn rec(target_mode: u32, init: u32, think: u32, alt: u32) -> AiRecord {
    AiRecord {
        target_mode,
        init,
        think,
        alt,
    }
}

/// The Idle think handler, used when a record has no think.
pub const IDLE_FN: u32 = 0x005B_0CD0;

/// The catalogue text, for the consistency test.
pub const AI_FUNCTIONS_TSV: &str = include_str!("../../../../../tables/monsters/ai-functions.tsv");

/// The AI indices whose catalogue `status` is `spec'd-here` (full rules
/// read from the base game, a body in D2RUST), ascending.
pub const SPECD_HERE: [u8; 148] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
    98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
    117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135,
    136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147,
];

/// AI table, by monstats `AI` index.
pub const AI_TABLE: [AiRecord; 148] = [
    rec(0, 0, 0x005B_0CC0, 0),                     // 0 None
    rec(0, 0, 0x005B_0CD0, 0),                     // 1 Idle
    rec(1, 0, 0x005E_FCF0, 0),                     // 2 Skeleton
    rec(1, 0, 0x005E_FE20, 0),                     // 3 Zombie
    rec(1, 0, 0x005E_FF50, 0),                     // 4 Bighead
    rec(1, 0, 0x005F_00E0, 0),                     // 5 BloodHawk
    rec(1, 0, 0x005F_02C0, 0),                     // 6 Fallen
    rec(1, 0, 0x005E_FB80, 0),                     // 7 Brute
    rec(1, 0, 0x005F_0700, 0),                     // 8 SandRaider
    rec(1, 0, 0x005F_0A20, 0),                     // 9 Wraith
    rec(1, 0, 0x005F_0B00, 0),                     // 10 CorruptRogue
    rec(1, 0, 0x005F_0CD0, 0),                     // 11 Baboon
    rec(1, 0, 0x005F_12A0, 0),                     // 12 Goatman
    rec(1, 0, 0x005F_1440, 0),                     // 13 FallenShaman
    rec(1, 0, 0x005F_1140, 0),                     // 14 QuillRat
    rec(4, 0, 0x005F_1800, 0x005F_1750),           // 15 SandMaggot
    rec(1, 0, 0x005F_1B60, 0),                     // 16 ClawViper
    rec(1, 0, 0x005F_20A0, 0),                     // 17 SandLeaper
    rec(1, 0, 0x005F_22B0, 0),                     // 18 PantherWoman
    rec(1, 0, 0x005F_2460, 0),                     // 19 Swarm
    rec(1, 0, 0x005F_2540, 0),                     // 20 Scarab
    rec(1, 0, 0x005F_2850, 0),                     // 21 Mummy
    rec(1, 0, 0x005F_2B10, 0),                     // 22 GreaterMummy
    rec(1, 0, 0x005F_3170, 0),                     // 23 Vulture
    rec(1, 0, 0x005F_3730, 0),                     // 24 Mosquito
    rec(1, 0, 0x005F_39B0, 0),                     // 25 WillOWisp
    rec(1, 0, 0x005F_4510, 0),                     // 26 Arach
    rec(1, 0, 0x005F_4850, 0),                     // 27 ThornHulk
    rec(1, 0, 0x005F_4A70, 0),                     // 28 Vampire
    rec(1, 0, 0x005F_5040, 0x005F_4FD0),           // 29 BatDemon
    rec(1, 0, 0x005F_53E0, 0),                     // 30 Fetish
    rec(0, 0, 0x005E_7880, 0),                     // 31 NpcOutOfTown
    rec(0, 0, 0x005E_7130, 0),                     // 32 Npc
    rec(1, 0, 0x005F_56D0, 0),                     // 33 HellMeteor
    rec(1, 0, 0x005F_5830, 0),                     // 34 Andariel
    rec(1, 0, 0x005F_5A20, 0),                     // 35 CorruptArcher
    rec(1, 0, 0x005F_5D50, 0),                     // 36 CorruptLancer
    rec(1, 0, 0x005F_6070, 0),                     // 37 SkeletonBow
    rec(1, 0, 0x005F_6220, 0),                     // 38 MaggotLarva
    rec(1, 0, 0x005F_6340, 0),                     // 39 PinHead
    rec(1, 0, 0x005F_6530, 0),                     // 40 MaggotEgg
    rec(0, 0, 0x005E_7540, 0),                     // 41 Towner
    rec(0, 0, 0x005E_9E00, 0),                     // 42 Vendor
    rec(1, 0x005F_6630, 0x005F_6650, 0),           // 43 FoulCrowNest
    rec(1, 0, 0x005F_67B0, 0),                     // 44 Duriel
    rec(1, 0x005F_6630, 0x005F_6A10, 0),           // 45 Sarcophagus
    rec(1, 0, 0x005F_6B70, 0),                     // 46 ElementalBeast
    rec(1, 0, 0x005F_6CA0, 0),                     // 47 FlyingScimitar
    rec(1, 0, 0x005F_6E60, 0),                     // 48 ZakarumZealot
    rec(1, 0, 0x005F_72D0, 0),                     // 49 ZakarumPriest
    rec(1, 0, 0x005F_78B0, 0),                     // 50 Mephisto
    rec(1, 0, 0x005E_9170, 0x005E_8480),           // 51 Diablo
    rec(5, 0, 0x005F_8260, 0x005F_81D0),           // 52 FrogDemon
    rec(1, 0, 0x005F_85C0, 0),                     // 53 Summoner
    rec(0, 0, 0x005E_73A0, 0),                     // 54 NpcStationary
    rec(1, 0, 0x005F_89B0, 0),                     // 55 Izual
    rec(2, 0, 0x005F_8F80, 0),                     // 56 Tentacle
    rec(2, 0, 0x005F_9270, 0),                     // 57 TentacleHead
    rec(0, 0, 0x005E_7E20, 0),                     // 58 Navi
    rec(1, 0x005E_6300, 0x005E_6320, 0),           // 59 BloodRaven
    rec(0, 0, 0x005E_7AC0, 0),                     // 60 GoodNpcRanged
    rec(0, 0, 0x005E_52D0, 0x005E_5280),           // 61 Hireable
    rec(1, 0, 0x005E_7D60, 0),                     // 62 TownRogue
    rec(1, 0, 0x005F_9490, 0),                     // 63 GargoyleTrap
    rec(1, 0, 0x005F_96C0, 0),                     // 64 SkeletonMage
    rec(1, 0, 0x005F_9A80, 0x005F_9950),           // 65 FetishShaman
    rec(0, 0, 0x005F_9CF0, 0),                     // 66 SandMaggotQueen
    rec(0, 0, 0x005E_4CF0, 0),                     // 67 NecroPet
    rec(1, 0, 0x005F_A010, 0),                     // 68 VileMother
    rec(1, 0, 0x005F_A280, 0),                     // 69 VileDog
    rec(1, 0, 0x005F_A380, 0),                     // 70 FingerMage
    rec(1, 0, 0x005F_A710, 0),                     // 71 Regurgitator
    rec(1, 0, 0x005F_AA90, 0),                     // 72 DoomKnight
    rec(1, 0, 0x005F_AB80, 0),                     // 73 AbyssKnight
    rec(1, 0, 0x005F_AF00, 0),                     // 74 OblivionKnight
    rec(1, 0, 0x005F_B2A0, 0),                     // 75 QuillMother
    rec(1, 0, 0x005F_B410, 0),                     // 76 EvilHole
    rec(2, 0, 0x005F_B5B0, 0),                     // 77 Trap-Missile
    rec(1, 0, 0x005F_B6C0, 0),                     // 78 Trap-RightArrow
    rec(1, 0, 0x005F_B7E0, 0),                     // 79 Trap-LeftArrow
    rec(2, 0, 0x005F_B900, 0),                     // 80 Trap-Poison
    rec(0, 0, 0x005E_7590, 0),                     // 81 JarJar
    rec(1, 0, 0x005E_0160, 0),                     // 82 InvisoSpawner
    rec(0, 0, 0x005E_0260, 0),                     // 83 MosquitoNest
    rec(0, 0x005E_0390, 0x005E_0400, 0),           // 84 BoneWall
    rec(1, 0, 0x005E_0490, 0),                     // 85 HighPriest
    rec(2, 0, 0x005E_9E60, 0),                     // 86 Hydra
    rec(1, 0, 0x005F_BA60, 0),                     // 87 Trap-Melee
    rec(1, 0, 0x005E_A080, 0),                     // 88 7TIllusion
    rec(1, 0, 0x005E_0C80, 0),                     // 89 Megademon
    rec(1, 0, 0x005E_5AC0, 0),                     // 90 Griswold
    rec(1, 0, 0x005E_A130, 0),                     // 91 DarkWanderer
    rec(1, 0, 0x005F_B9B0, 0),                     // 92 Trap-Nova
    rec(1, 0, 0x005E_0F60, 0),                     // 93 ArcaneTower
    rec(1, 0, 0x005E_0980, 0),                     // 94 DesertTurret
    rec(1, 0, 0x005E_1080, 0),                     // 95 PantherJavelin
    rec(1, 0, 0x005E_1250, 0),                     // 96 FetishBlowgun
    rec(1, 0, 0x005E_3840, 0),                     // 97 Spirit
    rec(1, 0, 0x005E_3890, 0),                     // 98 Smith
    rec(1, 0, 0x005E_9F10, 0),                     // 99 TrappedSoul
    rec(0, 0, 0x005E_7F50, 0),                     // 100 Buffy
    rec(0, 0x005E_A290, 0x005E_A3D0, 0),           // 101 AssassinSentry
    rec(0, 0x005E_A510, 0x005E_A540, 0),           // 102 BladeCreeper
    rec(0, 0, 0x005E_A7A0, 0),                     // 103 InvisoPet
    rec(0, 0x005E_A290, 0x005E_A980, 0),           // 104 DeathSentry
    rec(2, 0x005E_AF50, 0x005E_AFA0, 0),           // 105 ShadowWarrior
    rec(2, 0x005E_B490, 0x005E_B970, 0),           // 106 ShadowMaster
    rec(2, 0x005E_CB70, 0x005E_CC10, 0),           // 107 Raven
    rec(0, 0, 0x005E_D710, 0),                     // 108 DruidWolf
    rec(0, 0, 0x005E_D9E0, 0),                     // 109 Totem
    rec(2, 0x005E_C6A0, 0x005E_C6C0, 0),           // 110 Vines
    rec(2, 0x005E_C6A0, 0x005E_C8C0, 0),           // 111 CycleOfLife
    rec(0, 0, 0x005E_D730, 0),                     // 112 DruidBear
    rec(1, 0, 0x005E_1860, 0),                     // 113 SiegeTower
    rec(1, 0, 0x005E_1540, 0),                     // 114 ReanimatedHorde
    rec(1, 0, 0x005E_1900, 0),                     // 115 SiegeBeast
    rec(1, 0, 0x005E_1B60, 0),                     // 116 Minion
    rec(1, 0, 0x005E_1D30, 0),                     // 117 SuicideMinion
    rec(1, 0, 0x005E_1E00, 0),                     // 118 Succubus
    rec(1, 0, 0x005E_2120, 0),                     // 119 SuccubusWitch
    rec(1, 0, 0x005E_27A0, 0),                     // 120 Overseer
    rec(1, 0x005F_6630, 0x005E_2BD0, 0),           // 121 MinionSpawner
    rec(1, 0x005E_2FD0, 0x005E_2FF0, 0),           // 122 Imp
    rec(1, 0, 0x005E_34C0, 0),                     // 123 Catapult
    rec(1, 0, 0x005E_3530, 0),                     // 124 FrozenHorror
    rec(1, 0, 0x005E_36F0, 0),                     // 125 BloodLord
    rec(1, 0, 0x005E_E040, 0),                     // 126 CatapultSpotter
    rec(2, 0x005E_DC40, 0x005E_DC50, 0),           // 127 NpcBarb
    rec(2, 0x005E_E5C0, 0x005E_E5D0, 0x005E_5280), // 128 Nihlathak
    rec(1, 0x005E_6190, 0x005E_61B0, 0),           // 129 GenericSpawner
    rec(1, 0, 0x005E_E260, 0),                     // 130 DeathMauler
    rec(2, 0, 0x005E_E3C0, 0),                     // 131 Wussie
    rec(0, 0, 0x005E_EAA0, 0),                     // 132 AncientStatue
    rec(2, 0, 0x005E_F1A0, 0),                     // 133 Ancient
    rec(2, 0x005E_F310, 0x005E_F320, 0),           // 134 BaalThrone
    rec(0, 0, 0x005F_CFE0, 0x005F_CF30),           // 135 BaalCrab
    rec(1, 0, 0x005E_F710, 0),                     // 136 BaalTaunt
    rec(1, 0, 0x005E_FA90, 0),                     // 137 PutridDefiler
    rec(1, 0, 0x005E_F620, 0),                     // 138 BaalToStairs
    rec(1, 0, 0x005E_F820, 0),                     // 139 BaalTentacle
    rec(0, 0, 0x005F_D210, 0x005F_CF30),           // 140 BaalCrabClone
    rec(1, 0, 0x005E_F910, 0),                     // 141 BaalMinion
    rec(1, 0, 0x005F_1DE0, 0),                     // 142 ClawViperEx
    rec(2, 0x005E_B5C0, 0x005E_B970, 0),           // 143 ShadowMasterNoInit
    rec(1, 0, 0x005F_8C80, 0),                     // 144 UberIzual
    rec(1, 0, 0x005F_D200, 0x005F_CF30),           // 145 UberBaal
    rec(1, 0, 0x005F_81C0, 0),                     // 146 UberMephisto
    rec(1, 0, 0x005E_9DF0, 0x005E_8480),           // 147 UberDiablo
];

/// Special-state table, by AI special state.
pub const SPECIAL_TABLE: [AiRecord; 18] = [
    rec(0, 0, 0, 0),                     // 0 none
    rec(0, 0, 0x005B_0CD0, 0),           // 1 Idle
    rec(1, 0, 0x005B_14E0, 0),           // 2
    rec(1, 0x005E_5730, 0x005E_5870, 0), // 3
    rec(0, 0, 0x005E_52D0, 0x005E_5280), // 4 Hireable
    rec(0, 0, 0x005E_7AC0, 0),           // 5 GoodNpcRanged
    rec(0, 0, 0x005E_7C10, 0),           // 6
    rec(0, 0, 0x005E_4CF0, 0),           // 7 NecroPet
    rec(1, 0, 0x005E_7DC0, 0),           // 8 TownRogue
    rec(1, 0, 0x005E_7F80, 0),           // 9
    rec(1, 0, 0x005E_8020, 0),           // 10 dim vision
    rec(1, 0x005E_80E0, 0x005E_8140, 0), // 11 terror
    rec(0, 0, 0x005E_8340, 0),           // 12 taunt
    rec(1, 0, 0x005E_5C50, 0),           // 13
    rec(1, 0, 0x005E_2610, 0),           // 14
    rec(1, 0, 0x005E_1D30, 0),           // 15 SuicideMinion
    rec(1, 0x005E_2CD0, 0x005E_2D80, 0), // 16
    rec(2, 0, 0x005E_8020, 0),           // 17
];
