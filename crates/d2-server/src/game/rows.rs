//! Loaded table projections shared by the headless game and client bridge.
use d2_sim::skills::sequences::MonsterSequences;
use d2_sim::world::hirelings::HirelingRows;

/// Monster-class facts used by unit creation and animation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterClass {
    /// `monstats2` choice count of each of the 16 components.
    pub components: [u8; 16],
    /// Light radius of each component choice, resolved through its item code.
    pub component_light: [[u8; 12]; 16],
    /// `monstats` flag bit 8 `npc`.
    pub npc: bool,
    /// `monstats` flag bit 9 `interact`.
    pub interact: bool,
    /// `monstats2` `Light` and `light-r`, `light-g`, `light-b`: the
    /// monster light.
    pub light: u8,
    pub light_rgb: (u8, u8, u8),
    /// Ally and nonselectable flags used by the hostility test.
    pub al_sel: bool,
    pub no_sel: bool,
    /// Whether the corpse can be selected.
    pub corpse_sel: bool,
    /// Piece count used to choose the hover test.
    pub total_pieces: u8,
    /// Unit size used for collision and placement.
    pub size_x: i8,
    /// Height used to choose the shatter missile.
    pub height: u8,
    /// Exclude the monster from aura searches.
    pub no_aura: bool,
    /// Suppress overlay creation on this monster.
    pub no_ovly: bool,
    /// Whether the monster belongs to either undead category.
    pub undead: bool,
    /// `monstats` `demon`: the demon test (hit 9).
    pub demon: bool,
    /// `monstats` flag 10 `inTown` (with `npc`: the monster can be in
    /// town, the footprint pattern).
    pub in_town: bool,
    /// Movement capabilities used to select the path mask.
    pub flying: bool,
    pub open_doors: bool,
    /// Loaded `monstats`, `boss`, `switchai`, `WL`.
    pub boss: bool,
    pub switchai: bool,
    /// `monstats2` `critter` (flags bit 13, monstats2 flag 13).
    pub critter: bool,
    /// The columns of the monster set-up;
    /// `None`: the tables do not give them (the set-up's table
    /// parts are not run).
    pub setup: Option<MonsterSetup>,
    /// The AnimData record (frames, speed) of the class's composite name
    /// in each monster mode 0–15 (the default
    /// record when the name has none); `None`: no name (the mode set
    /// leaves the frame count and speed at 0).
    pub anims: [Option<(u32, u32)>; 16],
    pub anim_events: [Option<[u8; d2_formats::animdata::EVENTS]>; 16],
    /// Mode-specific missiles; absent for unused modes or an unset table entry.
    pub mode_missiles: [Option<u16>; 16],
    /// Monster-class facts used by unit creation and animation.
    pub walk_speed: u16,
    pub run_speed: u16,
    /// Minimum and maximum critter group sizes.
    pub min_grp: u8,
    pub max_grp: u8,
    /// Loaded `monstats2`, `mDT`.
    pub modes: u16,
    /// Loaded `monstats2`, `d<mode>`, `DDT`, `DRN`.
    pub directions: [u8; 16],
    /// The direction count `D` (header byte 2) of the class's DT and DD
    /// COFs, at index 0 and 12; 0
    /// when the COF is not loaded (the other modes are not read).
    pub cof_dirs: [u8; 16],
    /// Loaded `monstats2`, `A1mv`.
    pub move_modes: u16,
    /// `monstats` `BaseId`: the class itself when the
    /// tables do not give it.
    pub base_id: u16,
    /// Loaded `monstats`, `Code`, `PB  `.
    pub code: [u8; 4],
    /// Loaded `monstats`, `SplEndDeath`.
    pub spl_end_death: bool,
    /// Loaded `monstats`, `SplClientEnd`.
    pub spl_client_end: bool,
    /// Loaded `monstats2`.
    pub path_byte: u8,
    /// `monstats2` `noUniqueShift` (flag bit 15): no unique reseed after
    /// the set-up.
    pub no_unique_shift: bool,
    /// Loaded `monstats2`, `InfernoAnim`, `InfernoRollback`.
    pub inferno_anim: u8,
    pub inferno_rollback: u8,
    /// `monstats2` `soft` (flag bit 12): the corpse test of `cltdofunc`
    /// 40.
    pub soft: bool,
    /// Loaded `monstats2`, `small`, `large`, `cltdofunc`.
    pub small: bool,
    pub large: bool,
    /// Loaded `monstats2`, `Bleed`.
    pub bleed: u8,
}

/// The `monstats` / `monstats2` columns of the monster set-up,
/// by difficulty where the table has three.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterSetup {
    /// Loaded `Level`, `Level(N)`, `Level(H)`.
    pub level: [u16; 3],
    /// `ResDm`, `ResMa`, `ResFi`, `ResLi`, `ResCo`, `ResPo` by difficulty
    /// (stats 36, 37, 39, 41, 43, 45).
    pub res: [[u16; 3]; 6],
    /// Loaded `Velocity`.
    pub velocity: u16,
    /// `Align` (classic scaling).
    pub align: u8,
    /// Loaded `Skill1`, `Skill8`, `Sk1lvl`, `Sk1mode`.
    pub skills: [(i16, u8, u8); 8],
    /// Loaded `monstats2`, `isSel`, `shadow`, `isAtt`.
    pub is_sel: bool,
    pub shadow: bool,
    pub is_att: bool,
}

/// One `objects.txt` row used by shrine requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectRow {
    /// Loaded `SubClass`.
    pub subclass: u8,
    /// Loaded `ShrineFunction`.
    pub shrine_function: u8,
    /// Loaded `EnvEffect`.
    pub env_effect: bool,
    /// Loaded `Lit0`, `Lit7`.
    pub lit: [u8; 8],
    /// `Selectable0`…`Selectable7`.
    pub selectable: [bool; 8],
    /// `Red`, `Green`, `Blue`: the light color.
    pub rgb: (u8, u8, u8),
}

/// One `states` row as the state messages read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateRow {
    pub dead_bit_only: bool,
    /// The flag: state on keeps an existing list.
    pub keep_list: bool,
    /// Loaded `colorpri`, `colorshift`, `light-r`, `light-g`, `light-b`.
    pub colorpri: u8,
    pub colorshift: u8,
    pub light_rgb: (u8, u8, u8),
    /// Loaded `meleeonly`, `states.txt`, `data/fields.tsv`, `range(P, skill)`.
    pub meleeonly: bool,
    /// `restrict` (`states.txt` flag bit 17, `data/fields.tsv`): the
    /// state-mask group tests, the shapeshift forms
    /// (skill use test 7).
    pub restrict: bool,
    /// Loaded `castoverlay`, `overlay1`, `overlay2`, `removerlay`.
    pub links: StateOverlayLinks,
    /// `aura` (flag bit 1): the state overlays are kinds 8 / 9.
    pub aura: bool,
    /// Loaded `nooverlays`.
    pub nooverlays: bool,
    /// Loaded `transform`, `states.txt`.
    pub transform: bool,
    /// `disguise` (flag group 16): the state bit change
    /// sets or clears flag-ex bit 3.
    pub disguise: bool,
    /// Loaded `setfunc`.
    pub setfunc: u16,
    /// Loaded `remfunc`.
    pub remfunc: u16,
    /// Loaded `gfxtype`, `gfxclass`.
    pub gfxtype: u8,
    pub gfxclass: u16,
    /// Loaded `udead`, `states.txt`.
    pub udead: bool,
    /// Loaded `hide`, `states.txt`, `shatter`.
    pub hide: bool,
    pub shatter: bool,
    /// Loaded `pgsv`, `pgsvoverlay`, `stat`.
    pub pgsv: bool,
    pub pgsv_overlay: i16,
    pub stat: i16,
}

/// The `overlay` links of a `states` row, each the signed short of the
/// record (a negative one is none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateOverlayLinks {
    pub cast: i16,
    pub overlay1: i16,
    pub overlay2: i16,
    pub remove: i16,
}

impl Default for StateOverlayLinks {
    fn default() -> Self {
        StateOverlayLinks {
            cast: -1,
            overlay1: -1,
            overlay2: -1,
            remove: -1,
        }
    }
}

/// Loaded `skilldesc`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillDescRow {
    /// Loaded `skillpage`.
    pub page: i8,
}

/// Loaded `itemstatcost`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatSend {
    /// Loaded `send bits`.
    pub bits: u8,
    /// Loaded `send param bits`.
    pub param_bits: u8,
    pub signed: bool,
    /// Loaded `updateanimrate`.
    pub update_anim_rate: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitRows {
    pub monsters: Vec<Option<MonsterClass>>,
    /// `difficultylevels` `MonsterSkillBonus` by difficulty.
    pub monster_skill_bonus: [i32; 3],
    pub stats: Vec<StatSend>,
    pub objects: Vec<ObjectRow>,
    pub shrines: Vec<u8>,
    /// One entry per `states` row, by state id.
    pub states: Vec<StateRow>,
    /// One entry per `missiles` row.
    pub missiles: Vec<ClientMissileRow>,
    /// One entry per `overlay` row.
    pub overlays: Vec<OverlayLightRow>,
    /// The monsters' skill sequences: `monstats` slots and `monseq` lists.
    pub monster_sequences: std::sync::Arc<MonsterSequences>,
    /// The `hireling` rows of the client hireling set-up.
    pub hirelings: std::sync::Arc<HirelingRows>,
}

impl MonsterClass {
    /// Loaded `monstats`, `monstats2`, `HDv`, `S8v`, `None`.
    pub fn from_record(monstats2: &[u8], npc: bool, interact: bool) -> Option<Self> {
        let mut components = [0u8; 16];
        components.copy_from_slice(monstats2.get(0x15..0x25)?);
        let mut directions = [0u8; 16];
        directions.copy_from_slice(monstats2.get(0xF4..0x104)?);
        Some(MonsterClass {
            components,
            npc,
            interact,
            al_sel: *monstats2.get(0x04)? & 0x10 != 0,
            no_sel: *monstats2.get(0x04)? & 0x20 != 0,
            large: *monstats2.get(0x05)? & 0x08 != 0,
            no_ovly: *monstats2.get(0x04)? & 0x04 != 0,
            corpse_sel: *monstats2.get(0x04)? & 0x80 != 0,
            size_x: *monstats2.get(0x08)? as i8,
            height: *monstats2.get(0x0B)?,
            total_pieces: *monstats2.get(0xEC)?,
            modes: u16::from_le_bytes([*monstats2.get(0xF0)?, *monstats2.get(0xF1)?]),
            path_byte: *monstats2.get(0x0E)?,
            directions,
            move_modes: u16::from_le_bytes([*monstats2.get(0x104)?, *monstats2.get(0x105)?]),
            bleed: *monstats2.get(0x11D)?,
            inferno_anim: *monstats2.get(0x109)?,
            inferno_rollback: *monstats2.get(0x10A)?,
            setup: None,
            ..MonsterClass::default()
        })
    }
}

/// The `skills` fields the client skill list reads (Inputs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillRow {
    /// Loaded `anim`.
    pub anim: u8,
    /// Loaded `monanim`.
    pub monanim: u8,
    /// Loaded `seqnum`.
    pub seqnum: u8,
    /// Loaded `passivestate`.
    pub passivestate: u16,
    /// `maxlvl` (u16 at 300, read signed).
    pub maxlvl: u16,
    /// Loaded `charclass`.
    pub charclass: i8,
    /// Loaded `srvdofunc`.
    pub srvdofunc: i16,
    /// `enhanceable` (bit 17).
    pub enhanceable: bool,
    /// Loaded `skilldesc`.
    pub skilldesc: u16,
    /// Loaded `EType`.
    pub etype: u8,
    /// Loaded `seqinput`.
    pub seqinput: u8,
    /// Loaded `range`, `@range`.
    pub range: u8,
    /// The flag columns reads, by `skills.txt`
    /// bit (`controls::click::skill_flag`).
    pub flags: u32,
    /// `itypea1..3`, `itypeb1..3` (i16, <= 0 = none): the item type sets
    /// of the skill use item type test.
    pub itypes: [[i16; 3]; 2],
    /// `etypea1..2`, `etypeb1..2` (i16, <= 0 = none).
    pub etypes: [[i16; 2]; 2],
    /// `InGame` (use state test 1).
    pub ingame: bool,
    /// `aura` (use state test 3).
    pub aura: bool,
    /// `mana`, `lvlmana` (i16 read signed) and `manashift` (low byte):
    /// the mana cost (use state test 6).
    pub mana: u16,
    pub lvlmana: u16,
    pub manashift: u16,
    /// Loaded `cltstfunc`.
    pub cltstfunc: u16,
    /// Loaded `Param1`, `Param3`, `cltdofunc`.
    pub param1: i32,
    pub param2: i32,
    pub param3: i32,
    /// Loaded `Param4`, `cltstfunc`.
    pub param4: i32,
    /// Loaded `restrict`, `State1`, `State3`.
    pub restrict: u8,
    /// Loaded `State1`, `State3`.
    pub shape_states: [i16; 3],
    /// Loaded `cltdofunc`.
    pub cltdofunc: i16,
    /// Loaded `cltmissile`, `cltmissilea`, `c`.
    pub cltmissile: i16,
    pub cltmissile_abc: [i16; 3],
    /// Loaded `castoverlay`.
    pub castoverlay: i16,
    /// Loaded `dosound`, `tgtoverlay`, `tgtsound`.
    pub dosound: i16,
    /// Loaded `dosound a`, `dosound b`.
    pub dosound_a: i16,
    pub dosound_b: i16,
    pub tgtoverlay: i16,
    pub tgtsound: i16,
    /// Loaded `cltcalc1`.
    pub cltcalc1: u32,
    /// Loaded `progressive`, `progstate`, `aurastate`, `aurastat1`.
    pub progressive: bool,
    /// Loaded `lob`, `cltmissile`.
    pub lob: bool,
    pub aurastate: i16,
    pub aurastat1: i16,
    /// Loaded `delay`, `None`.
    pub delay: Option<u32>,
    /// Loaded `startmana`.
    pub startmana: u16,
    /// Loaded `SelectProc`.
    pub selectproc: u8,
}

/// Loaded `Levels.txt`, `cmon1`, `4`, `cpct1`, `camt`, `camt4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Critters {
    pub cmon: [i16; 4],
    pub cpct: [i16; 4],
    pub camt: [u16; 4],
}

impl Default for Critters {
    fn default() -> Self {
        Self {
            cmon: [-1; 4],
            cpct: [0; 4],
            camt: [0; 4],
        }
    }
}

/// The `Levels.txt` fields the client reads of the player's level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelRow {
    /// Loaded `Pal`.
    pub pal: u8,
    /// Loaded `Act`.
    pub act: u8,
    /// Loaded `BlankScreen`.
    pub blank_screen: bool,
    /// `SoundEnv`: the `soundenviron` row of the level.
    pub sound_env: u8,
    /// Loaded `DrawEdges`.
    pub draw_edges: bool,
    /// Loaded `Rain`.
    pub rain: bool,
    /// Loaded `Mud`.
    pub mud: bool,
    /// The critter columns of the client room pass.
    pub critters: Critters,
    /// The level's `leveldefs` `Intensity`, `Red`, `Green`, `Blue`
    /// (the darkness base).
    pub ambient: Ambient,
}

/// The `missiles` columns the client create reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientMissileRow {
    pub vel: i32,
    pub vel_lev: i32,
    pub max_vel: i32,
    pub accel: i16,
    /// `Range`, `LevRange` (signed i16 columns).
    pub range: i16,
    pub lev_range: i16,
    pub sub_loop: u8,
    pub sub_start: u8,
    pub sub_stop: u8,
    pub activate: i32,
    pub init_steps: u8,
    pub anim_len: u8,
    pub anim_speed: u8,
    /// `Light` and `Red`, `Green`, `Blue`.
    pub light: u8,
    pub rgb: (u8, u8, u8),
    pub can_slow: bool,
    pub pierce: bool,
    pub last_collide: bool,
    /// `pCltDoFunc`.
    pub clt_do_func: u16,
    /// `LoopAnim`.
    pub loop_anim: bool,
    /// `Flicker`.
    pub flicker: u8,
    /// `CollideType` (mode).
    pub collide_type: u8,
    /// `AlwaysExplode`, `ExplosionMissile` (i16), `pCltHitFunc` (i16).
    pub always_explode: bool,
    pub explosion_missile: i16,
    pub clt_hit_func: i16,
    /// `CltSubMissile1`–`3` (i16) and `CltParam1`–`5` (i32).
    pub clt_sub: [i16; 3],
    pub clt_param: [i32; 5],
    /// Loaded `CltCalc1`, `misscode`.
    pub clt_calc1: u32,
    /// Loaded `SHitCalc1`.
    pub s_hit_calc1: u32,
    /// `Town` (flags bit 10) and `CltSrcTown`.
    pub town: bool,
    pub clt_src_town: u8,
    /// Loaded `Size`.
    pub size: u8,
    /// `RandStart` (the disc's frame offset).
    pub rand_start: i32,
    /// `ProgSound` (i16; function 47).
    pub prog_sound: i16,
    /// Loaded `ProgOverlay`.
    pub prog_overlay: i16,
    /// Loaded `Param1`, `Param2`.
    pub param: [i32; 2],
    /// `ClientCol`, `CollideKill`, `CollideFriend`, `NextHit`,
    /// `NextDelay`, `CanDestroy`.
    pub client_col: bool,
    pub collide_kill: bool,
    pub collide_friend: bool,
    pub next_hit: bool,
    pub next_delay: u8,
    pub can_destroy: bool,
    /// `CltHitSubMissile1`–`4` (i16) and `cHitPar1`–`3` (i32), the hit
    /// functions' H1–H4 and c1–c3.
    pub clt_hit_sub: [i16; 4],
    pub c_hit_par: [i32; 3],
    /// The server column `HitSubMissile1` (i16; hit 26 gates on it,
    /// Edge case 3).
    pub hit_sub1_server: i16,
    /// Loaded `SubMissile1`.
    pub sub1_server: i16,
    /// Loaded `SubMissile2`.
    pub sub2_server: i16,
    /// `TravelSound`, `HitSound` (i16).
    pub travel_sound: i16,
    pub hit_sound: i16,
    /// `NoMultiShot` (flags bit 12; the umod 29 hook).
    pub no_multishot: bool,
}

/// The `overlay` fields the light of a record reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayLightRow {
    /// Loaded `version`.
    pub version: i16,
    /// Loaded `Frames`.
    pub frames: u32,
    /// Loaded `AnimRate`.
    pub anim_rate: u32,
    /// Loaded `InitRadius`, `Radius`.
    pub init_radius: u32,
    pub radius: u32,
    /// Loaded `Red`, `Green`, `Blue`.
    pub rgb: (u8, u8, u8),
    /// Loaded `1ofN`.
    pub one_of_n: u32,
}

/// One `objects.txt` row as the client object code reads it
/// (Constants: columns read).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjClientRow {
    /// Loaded `ClientFn`.
    pub client_fn: u8,
    /// Loaded `Start0`, `Start7`.
    pub start: [u8; 8],
    /// Loaded `FrameCnt0`, `7`.
    pub frame_cnt: [u32; 8],
    /// Loaded `FrameDelta0`, `7`.
    pub frame_delta: [u16; 8],
    /// `CycleAnim0`–`7`.
    pub cycle_anim: [u8; 8],
    /// Loaded `Sync`, `FrameDelta`.
    pub sync: u8,
    /// Loaded `SizeX`.
    pub size_x: u32,
    /// Loaded `SizeY`.
    pub size_y: u32,
    /// Loaded `EnvEffect`.
    pub env_effect: u8,
    /// `Selectable0`–`7`.
    pub selectable: [u8; 8],
    /// Loaded `Lit0`, `7`.
    pub lit: [u8; 8],
    /// `Red`, `Green`, `Blue`.
    pub rgb: (u8, u8, u8),
    /// Loaded `IsDoor`.
    pub is_door: u8,
    /// Loaded `CollisionSubst`, `TotalPieces`.
    pub collision_subst: u8,
    pub total_pieces: u8,
    /// Loaded `OrderFlag2`.
    pub order_flag2: u8,
    /// Loaded `OrderFlag0`, `7`.
    pub order_flag: [u8; 8],
    /// Loaded `Parm7`.
    pub parm7: u32,
    /// Loaded `Overlay`.
    pub overlay: u8,
    /// `HasCollision0`–`7`.
    pub has_collision: [u8; 8],
    /// Loaded `Mode0`, `7`.
    pub mode_ok: [u8; 8],
    /// The footprint inputs of the path placement (`SizeX` ×
    /// `SizeY`, the mask from `IsDoor`, `BlocksVis`, `BlockMissile`,
    /// `SubClass`; `HasCollision0..7`), as the server reads them.
    pub shape: d2_sim::path::record::ObjectShape,
}

impl ObjClientRow {
    /// The fields of one decoded `objects` row.
    pub fn from_row(o: &d2_data::tables::Objects) -> Self {
        Self {
            client_fn: o.clientfn,
            start: [
                o.start0, o.start1, o.start2, o.start3, o.start4, o.start5, o.start6, o.start7,
            ],
            frame_cnt: [
                o.framecnt0,
                o.framecnt1,
                o.framecnt2,
                o.framecnt3,
                o.framecnt4,
                o.framecnt5,
                o.framecnt6,
                o.framecnt7,
            ],
            frame_delta: [
                o.framedelta0,
                o.framedelta1,
                o.framedelta2,
                o.framedelta3,
                o.framedelta4,
                o.framedelta5,
                o.framedelta6,
                o.framedelta7,
            ],
            cycle_anim: [
                o.cycleanim0,
                o.cycleanim1,
                o.cycleanim2,
                o.cycleanim3,
                o.cycleanim4,
                o.cycleanim5,
                o.cycleanim6,
                o.cycleanim7,
            ],
            sync: o.sync,
            size_x: o.sizex,
            size_y: o.sizey,
            env_effect: o.enveffect,
            selectable: [
                o.selectable0,
                o.selectable1,
                o.selectable2,
                o.selectable3,
                o.selectable4,
                o.selectable5,
                o.selectable6,
                o.selectable7,
            ],
            lit: [
                o.lit0, o.lit1, o.lit2, o.lit3, o.lit4, o.lit5, o.lit6, o.lit7,
            ],
            rgb: (o.red, o.green, o.blue),
            is_door: o.isdoor,
            collision_subst: o.collisionsubst,
            total_pieces: o.totalpieces,
            order_flag2: o.orderflag2,
            order_flag: [
                o.orderflag0,
                o.orderflag1,
                o.orderflag2,
                o.orderflag3,
                o.orderflag4,
                o.orderflag5,
                o.orderflag6,
                o.orderflag7,
            ],
            parm7: o.parm7,
            overlay: o.overlay,
            has_collision: [
                o.hascollision0,
                o.hascollision1,
                o.hascollision2,
                o.hascollision3,
                o.hascollision4,
                o.hascollision5,
                o.hascollision6,
                o.hascollision7,
            ],
            mode_ok: [
                o.mode0, o.mode1, o.mode2, o.mode3, o.mode4, o.mode5, o.mode6, o.mode7,
            ],
            shape: d2_sim::wiring::action::objects::object_shape(o),
        }
    }

    /// One row per decoded `objects` row, by class.
    pub fn rows(objects: &[d2_data::tables::Objects]) -> Vec<Self> {
        objects.iter().map(Self::from_row).collect()
    }

    /// on a row read from the raw table:
    /// `FrameCnt` := value << 8 (wrapping), frames in 1/256 units.
    pub fn frame_counts_fixed(mut self) -> Self {
        for c in &mut self.frame_cnt {
            *c = c.wrapping_shl(8);
        }
        self
    }
}

/// A room's ambient `(I, R, G, B)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Ambient {
    pub i: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Ambient {
    pub const ZERO: Ambient = Ambient {
        i: 0,
        r: 0,
        g: 0,
        b: 0,
    };

    /// Whether any of R, G, B ≠ 0 (the test that decides which light wins).
    pub fn has_color(&self) -> bool {
        self.r != 0 || self.g != 0 || self.b != 0
    }
}

/// One `objects.txt` row's label fields (`data/fields.tsv`).
#[derive(Clone, Debug, Default)]
pub struct ObjectLabelRow {
    /// Loaded `Name`.
    pub name: String,
    /// Loaded `SubClass`.
    pub subclass: u8,
    /// Loaded `NameOffset`.
    pub name_offset: i32,
    /// Loaded `IsDoor`.
    pub is_door: bool,
    /// Loaded `Lockable`.
    pub lockable: bool,
    /// Loaded `OpenWarp`.
    pub open_warp: bool,
}

/// One monster class' label facts.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonsterLabelRow {
    /// `monstats` `NameStr` (string index).
    pub name_str: u16,
    /// Loaded `monstats`, `interact`.
    pub interact: bool,
    /// `monstats` `killable`: with `NameStr` ≠ 0 and `interact` clear, the
    /// hostile branch.
    pub killable: bool,
    /// Loaded `monstats2`, `pixHeight`.
    pub pix_height: u8,
    /// Loaded `monstats`, `DescStr`.
    pub desc_str: u16,
    /// Loaded `monstats`, `demon`.
    pub demon: bool,
    /// Whether the monster belongs to either undead category.
    pub undead: bool,
}
