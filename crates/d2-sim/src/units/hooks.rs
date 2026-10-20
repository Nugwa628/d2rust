//! The seams of the unit code: [`Sim`], the state a unit operation
//! works on; [`UnitData`], the tables it reads; and [`UnitHooks`], every
//! call into a system another module owns (animation rate, path, AI,
//! skills, objects, items, missiles, trade, messages). Each hook's
//! default is the narrowest reading of the rules: nothing happens, or
//! the value that makes the caller do nothing. The provider of each is
//! named in its doc.

use d2_data::bin::BinTable;
use d2_data::tables::{decode_all, Monstats, Monstats2, WrongTable};

use crate::game::Game;
use crate::stats::{StatHost, StatLists};

use super::record::{AnimRecord, Sequence, Units};
use super::UnitId;

/// The monstats facts the unit code reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonsterInfo {
    /// `enabled` (bit 25 of +0x0C).
    pub enabled: bool,
    /// `aidel`, `aidel(N)`, `aidel(H)` (+0x4F, +0x50, +0x51).
    pub aidel: [u8; 3],
    /// The monstats2 move bits (+0x104, bit = mode) of the class's
    /// `MonStatsEx` row.
    pub moves: u32,
    /// `SplGetModeChart` (+0x1A5): the class may use a per-class mode
    /// record.
    pub mode_chart: bool,
}

/// Tables and game settings the unit code reads.
#[derive(Clone, Debug, Default)]
pub struct UnitData {
    /// monstats rows by class.
    pub monsters: Vec<MonsterInfo>,
    /// Game +0x6D.
    pub difficulty: u8,
    /// Game +0x6A or game +0x74 non-zero.
    pub aidel_by_difficulty: bool,
    /// Game +0x70 non-zero.
    pub expansion: bool,
}

impl UnitData {
    /// From the monstats and monstats2 tables.
    pub fn new(monstats: &BinTable, monstats2: &BinTable) -> Result<Self, WrongTable> {
        let m2 = decode_all::<Monstats2>(monstats2)?;
        let monsters = decode_all::<Monstats>(monstats)?
            .iter()
            .map(|m| {
                let moves = m2.get(usize::from(m.monstatsex)).map_or(0, |x| {
                    [
                        (4, x.a1mv),
                        (5, x.a2mv),
                        (7, x.scmv),
                        (8, x.s1mv),
                        (9, x.s2mv),
                        (10, x.s3mv),
                        (11, x.s4mv),
                    ]
                    .iter()
                    .filter(|(_, b)| *b)
                    .fold(0u32, |a, (bit, _)| a | 1 << bit)
                });
                MonsterInfo {
                    enabled: m.enabled,
                    aidel: [m.aidel, m.aidel_n, m.aidel_h],
                    moves,
                    mode_chart: m.splgetmodechart != 0,
                }
            })
            .collect();
        Ok(Self {
            monsters,
            ..Self::default()
        })
    }

    pub fn monster(&self, class: u32) -> Option<&MonsterInfo> {
        self.monsters.get(class as usize)
    }
}

/// What a unit operation works on.
pub struct Sim<'a> {
    pub game: &'a mut Game,
    pub units: &'a mut Units,
    pub stats: &'a mut StatLists,
    pub data: &'a UnitData,
}

/// Calls into systems other specs own. Also the [`StatHost`] of the
/// stat writes the unit code makes.
#[allow(unused_variables)]
pub trait UnitHooks: StatHost {
    // ---- animation (animation-rate and sequence) ----

    /// The unit's AnimData record for its mode.
    fn anim_record(&mut self, sim: &Sim<'_>, unit: UnitId) -> Option<AnimRecord> {
        None
    }

    /// The animation rate (+0x4C).
    fn anim_rate(&mut self, sim: &mut Sim<'_>, unit: UnitId) -> i16 {
        0
    }

    /// In the sequence branch of the prepare step,
    /// run whether or not the unit has a path:
    /// the speed +0x4C and, when the rate's case writes it (cast, attack:
    /// and 8), the sequence speed +0x3C.
    /// `frame_count` is +0x48 (the sequence's count · 256).
    fn sequence_rate(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        frame_count: i32,
    ) -> (i16, Option<i32>) {
        let _ = frame_count;
        (self.anim_rate(sim, unit), None)
    }

    /// The path-velocity half of the animation rate,
    /// run right after [`UnitHooks::anim_rate`]. Provider: path.
    fn anim_velocity(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// The frame bonus (start index of the animation schedule).
    fn frame_bonus(&mut self, sim: &Sim<'_>, unit: UnitId) -> i32 {
        0
    }

    /// Sequence load for player mode 18 / monster mode 14;
    /// `None` when the unit has no sequence.
    fn load_sequence(&mut self, sim: &Sim<'_>, unit: UnitId) -> Option<Sequence> {
        None
    }

    /// The unit has a path (+0x2C). Provider: path.
    fn has_path(&mut self, sim: &Sim<'_>, unit: UnitId) -> bool {
        false
    }

    /// A static path's position, +0x0C / +0x10:
    /// a ground item's sub-tile. `None`: no static path record
    /// (no path provider, or none for the unit). Provider: path.
    fn static_position(&self, unit: UnitId) -> Option<(i32, i32)> {
        None
    }

    /// The rest of the animation-field
    /// re-initialisation at a mode change. Provider: animation.
    fn reinit_anim(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    // ---- combat, rooms ------------------------------------------------

    /// Drop the unit's own entries from its combat list.
    /// Provider: combat.
    fn drop_combat_entries(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// on the unit's room (town test for player modes;
    /// monster regeneration). Provider: DRLG/world.
    fn room_flag(&mut self, sim: &Sim<'_>, unit: UnitId) -> bool {
        false
    }

    /// Outside a tick (the NPC interaction start):
    /// stop the unit's path. False: no path
    /// provider (the caller's own seam answers). Provider: path.
    fn stop_path_now(&mut self, unit: UnitId) -> bool {
        false
    }

    /// AI param 0 (control +0x14) := v
    /// (the AI's NPC interaction). False: no AI store.
    fn set_ai_param0(&mut self, unit: UnitId, v: i32) -> bool {
        false
    }

    /// AI param `index` (0..=2) of the
    /// unit's AI control := v (the C→S 0x59
    /// entry). False: no AI store or no control.
    fn set_ai_param(&mut self, unit: UnitId, index: usize, v: i32) -> bool {
        false
    }

    // ---- players --------------------------------

    /// The mode request check. Provider:
    /// player/path. Default: accepted.
    fn player_request_check(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u32) -> bool {
        true
    }

    /// Before the death animation: death bookkeeping.
    /// Provider: player/combat.
    fn player_death(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Corpse mode: corpse and character save. Provider:
    /// player.
    fn player_corpse(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Steps 1–2: the corpse pickup of
    /// C→S 0x16 type 0:
    /// the state and permission tests and
    /// the experience return. True when the routine goes on to the item
    /// take-back (the caller's). Provider: player/combat.
    fn player_corpse_pickup(&mut self, sim: &mut Sim<'_>, player: UnitId, corpse: UnitId) -> bool {
        false
    }

    /// Knockback start: path values 8 and 5. Provider: path.
    fn player_knockback_path(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// After an attack/cast/skill mode start. Provider:
    /// skills.
    fn player_skill_start(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// One movement step; the action result (2 runs the
    /// ENDANIM handler at once). Provider: path.
    fn player_movement_step(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) -> u32 {
        1
    }

    /// The action frame of attack, cast and skill modes;
    /// the action result. Provider: skills.
    fn player_action_frame(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) -> u32 {
        1
    }

    /// The unit's item row is flagged (attack-mode cleanup
    /// at the end of the animation). Provider: items.
    fn player_item_row_flagged(&mut self, sim: &Sim<'_>, unit: UnitId) -> bool {
        false
    }

    /// Attack-mode cleanup. Provider: skills.
    fn player_attack_cleanup(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Event 11: party refresh and pet refresh.
    /// Provider: party and pet specs.
    fn player_refresh(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Event 13. Provider: trade.
    fn update_trade(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    // ---- monsters -------------------------------

    /// Path and AI-state bookkeeping before a mode start
    /// (every mode but GH). Provider: monster.
    fn monster_mode_bookkeeping(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u32) {}

    /// The footprint add with (U) for a live unit,
    /// every requested mode (GH included), before the AI-state half.
    /// Provider: path.
    fn monster_mode_footprint(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// The AI-state half of the monster mode set, run for every
    /// requested mode (GH included) while the unit's mode is not neutral.
    fn monster_leave_mode(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// The monster mode damage,
    /// after the bookkeeping and before umod
    /// mode 0, every mode but GH.
    /// Provider: skills.
    fn monster_mode_damage(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u32) {}

    /// Per-class mode records (classes with
    /// monstats +0x1A5): the record to use instead of the table's.
    /// Provider: monster.
    fn monster_class_record(
        &mut self,
        sim: &Sim<'_>,
        unit: UnitId,
        mode: u32,
    ) -> Option<super::modes::MonsterModeRecord> {
        None
    }

    /// A monster mode function at `address` (start functions return
    /// whether the mode started; event functions' results are
    /// ignored). The neutral start is D2RUST's own.
    /// Provider: monster. Default: started / nothing.
    fn monster_mode_function(&mut self, sim: &mut Sim<'_>, unit: UnitId, address: u32) -> bool {
        true
    }

    /// The state-54 check before an AI event is
    /// scheduled. Provider: monster.
    fn uninterruptable_check(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Event 2: AI think. Provider: AI.
    fn ai_think(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    /// The umod dispatcher's mode-set sites inside:
    /// `mode` 0
    /// (before the start function, never for GH) and 1
    /// (after the animation prepare, before the cancel of
    /// events 0 / 1). Provider: monster (`monsters::init::dispatch`).
    fn monster_umods(&mut self, sim: &mut Sim<'_>, unit: UnitId, mode: u8) {}

    /// Event 7. Provider: monster.
    fn monster_umod(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    /// Event 10. Provider: AI.
    fn ai_reset(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    /// Monster death by regeneration:`(game, unit, killer)
    /// and the death events. Provider: monster.
    fn monster_death(&mut self, sim: &mut Sim<'_>, unit: UnitId, killer: Option<UnitId>) {}

    // ---- skills -----------

    /// Event 5: active-state function `f` (< 191) of table
    /// (null entries: nothing). Provider: skills.
    fn active_state(&mut self, sim: &mut Sim<'_>, unit: UnitId, f: u16, skill: u32, a2: u32) {}

    /// Event 8. Provider: skills.
    fn periodic_skills(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    /// Event 9: aura application `(game, unit, skill, l, 1, 1, 0)` and
    /// the aura event `(game, unit, a1, skill, l, 0)`. Provider: skills.
    fn apply_item_aura(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        a1: u32,
        skill: u32,
        level: i32,
    ) {
    }

    /// Event 14 callback: skill cooldown end. Provider:
    /// skills.
    fn cooldown_end(&mut self, sim: &mut Sim<'_>, unit: UnitId, a1: u32, a2: u32) {}

    // ---- other kinds ----------------------------------------------------

    /// Missile events (every type): the server-do function.
    /// Provider: missile.
    fn missile_do(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Object events 0–11 (handlers of `unit-handlers.tsv`). Provider:
    /// objects.
    fn object_event(&mut self, sim: &mut Sim<'_>, unit: UnitId, event: u8) {}

    /// Item event 3 (replenish). Provider:
    /// items; [`super::dispatch::replenish_delay`] gives the reschedule.
    fn item_replenish(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// Message`(unit, fraction): the life fraction.
    /// Provider: messages (d2-server).
    fn send_life_fraction(&mut self, sim: &mut Sim<'_>, unit: UnitId, fraction: i32) {}

    /// Hover free (when timed out). Provider:
    /// hover/chat.
    fn free_hover(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// After the type-12 expiry walk of `unit`,
    /// and after the regeneration tick frees a potion list: the
    /// remove callbacks of the lists freed run here.
    fn lists_expired(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}
}
