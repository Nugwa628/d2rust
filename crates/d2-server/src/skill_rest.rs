//! The skill-pipeline seams no written spec provides yet (`UseRest`,
//! `LearnRest`), answered for the play preview on
//! [`super::local_seams::LocalSeams`], the action wiring's `Pending`
//! value.
//!
//! The skill lists themselves are d2-sim's (`ActionHooks::skill_lists`; the
//! join's native skills run in `crate::adapters::character`): the skill-use
//! wiring answers every list call from them when the unit has one, so the list
//! answers here are reached only for a unit without a list (the narrowest:
//! none). [`SkillStore`] keeps the per-unit fields of the use pipeline that are
//! not list fields.
//!
//! Everything else here is a preview fill (decision D1): the narrowest
//! answer unless noted, each marked `// D2RUST's own, unverified`.

use std::collections::BTreeMap;

use crate::adapters::handlers::skills::LearnRest;
use d2_data::tables::Charstats;
use d2_sim::game::Game;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::SkillEntry;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::interaction::UseRest;

use super::local_seams::LocalSeams;

/// The per-unit fields the skill use pipeline reads and writes besides
/// the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitSkills {
    /// Used-skill flags (bit 0 moving, bit 1 arrived).
    pub used_flags: u32,
    /// Params 1…4 of the entries of a unit without a skill list (a
    /// monster: Charge keeps its target and hit mode there), one block per
    /// entry, keyed by (skill, owner GUID): each skill entry
    /// owns its params, so one skill's write never reaches
    /// another's (Armageddon's param 1 survives a later cast).
    pub entry_params: BTreeMap<(i32, i32), [i32; 4]>,
    /// Param4 of the Attack entry.
    pub attack_param4: i32,
    /// Unit +0x38 bits 8+.
    pub event_arg: i32,
    /// Player data +0x168 (the handlers use the staged value; kept for
    /// the timer path).
    pub last_point_frame: i32,
    /// The target of the last mode start (`UseRest::keep_target`).
    pub target: Option<ModeTarget<UnitId>>,
}

/// The use pipeline's per-unit fields of the game (module docs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillStore {
    pub units: BTreeMap<UnitId, UnitSkills>,
    /// The seams' unanswered calls, in call order.
    pub log: Vec<String>,
    /// The players in a shapeshift form (werewolf / werebear), refreshed
    /// by [`sync_shapes`] (q-druid).
    pub shifted: std::collections::BTreeSet<UnitId>,
    /// The `InGame` / `aura` / `passive` flags of every `skills` row,
    /// copied by [`sync_shapes`] (the `use_state` tests 1, 3, 4 read them);
    /// empty until the first sync.
    pub flags: Vec<SkillFlagsRow>,
}

/// One `skills` row's flags for `use_state` (the in-game, aura and passive tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillFlagsRow {
    pub in_game: bool,
    pub aura: bool,
    pub passive: bool,
}

/// The players whose state is the `aurastate` of a `srvdofunc` 116 skill
/// (Werewolf, Werebear), for [`SkillStore::shifted`]:
/// `UseRest::shapeshifted` has no game to read. D2RUST's own, unverified.
pub fn sync_shapes(game: &Game, sim: &mut d2_sim::wiring::worldgen::WorldSim<LocalSeams>) {
    let hooks = sim.action.sys.hooks.tables.clone();
    let forms: Vec<u32> = hooks
        .skills
        .skills
        .iter()
        .filter(|r| r.srvdofunc == 116)
        .map(|r| u32::from(r.aurastate))
        .collect();
    let shifted = game
        .lists
        .units_of_type(UnitType::Player)
        .into_iter()
        .filter(|&u| forms.iter().any(|&s| sim.action.sys.stats.has_state(u, s)))
        .collect();
    sim.action.sys.hooks.x.skills.shifted = shifted;
    if sim.action.sys.hooks.x.skills.flags.len() != hooks.skills.skills.len() {
        sim.action.sys.hooks.x.skills.flags = hooks
            .skills
            .skills
            .iter()
            .map(|r| SkillFlagsRow {
                in_game: r.ingame,
                aura: r.aura,
                passive: r.passive,
            })
            .collect();
    }
}

/// A `charstats` row's `Skill 1`…`Skill 10`.
pub fn class_skills(c: &Charstats) -> [u16; 10] {
    [
        c.skill_1, c.skill_2, c.skill_3, c.skill_4, c.skill_5, c.skill_6, c.skill_7, c.skill_8,
        c.skill_9, c.skill_10,
    ]
}

impl SkillStore {
    pub(super) fn unit(&self, u: UnitId) -> Option<&UnitSkills> {
        self.units.get(&u)
    }

    fn unit_mut(&mut self, u: UnitId) -> &mut UnitSkills {
        self.units.entry(u).or_default()
    }
}

impl LocalSeams {
    fn note(&mut self, s: String) {
        self.skills.log.push(s);
    }

    // q-barb: unit +0x38 bits 8+ is the frame event index the use pipeline
    // already keeps as `event_arg`; Double Swing and the monster bodies read it
    // back.
    pub(super) fn unit_entry_param(&self, u: UnitId, e: &SkillEntry, i: u8) -> i32 {
        let k = usize::from(i.wrapping_sub(1));
        self.skills
            .unit(u)
            .and_then(|s| s.entry_params.get(&(e.skill, e.owner_guid)))
            .and_then(|p| p.get(k).copied())
            .unwrap_or(0)
    }

    pub(super) fn set_unit_entry_param(&mut self, u: UnitId, e: &SkillEntry, i: u8, v: i32) {
        if let Some(p) = self
            .skills
            .unit_mut(u)
            .entry_params
            .entry((e.skill, e.owner_guid))
            .or_default()
            .get_mut(usize::from(i.wrapping_sub(1)))
        {
            *p = v;
        }
    }

    pub(super) fn frame_event_index(&self, u: UnitId) -> i32 {
        self.skills.unit(u).map_or(0, |s| s.event_arg)
    }

    pub(super) fn set_frame_event_index(&mut self, u: UnitId, i: i32) {
        self.skills.unit_mut(u).event_arg = i;
    }

    // q-barb: the paths of skill moves are not provided (Leap, Whirlwind:
    // not provided yet), but a body that retargets the unit (Double Swing's
    // second swing) is kept as the unit's mode target, which `UseRest::target`
    // reads back. D2RUST's own, unverified.
    pub(super) fn path_op(&mut self, u: UnitId, op: d2_sim::skills::use_::bodies::PathOp<UnitId>) {
        use d2_sim::skills::use_::bodies::PathOp;
        match op {
            PathOp::TargetUnit(Some(t)) => {
                self.skills.unit_mut(u).target = Some(ModeTarget::Unit(t))
            }
            PathOp::TargetUnit(None) => self.skills.unit_mut(u).target = None,
            _ => {}
        }
    }
}

/// The skill use pipeline's rest on the app's seams. The skill list and
/// the per-unit skill fields are [`SkillStore`]'s; the message handlers'
/// player data, positions, messages and mode start are the server's own
/// (`crate::adapters::handlers::skills::world::World`), so the
/// answers below for those are reached only on the timer path.
impl UseRest for LocalSeams {
    // D2RUST's own, unverified: the skill messages have no layout yet;
    // logged.
    fn send(&mut self, u: UnitId, msg: ServerMsg) {
        self.note(format!("skill send {} {msg:?}", u.0));
    }
    // D2RUST's own, unverified: players have player data (the loader sets
    // it), no other unit does.
    fn has_player_data(&self, u: UnitId) -> bool {
        self.sides
            .get(&u)
            .is_some_and(|&(ty, _, _)| ty == UnitType::Player)
    }
    fn last_point_frame(&self, u: UnitId) -> i32 {
        self.skills.unit(u).map_or(0, |s| s.last_point_frame)
    }
    fn set_last_point_frame(&mut self, u: UnitId, frame: i32) {
        self.skills.unit_mut(u).last_point_frame = frame;
    }
    // D2RUST's own, unverified: no cursor item (the inventory model is the
    // wired host's, not reachable here).
    fn cursor_item(&self, _: UnitId) -> bool {
        false
    }
    // D2RUST's own, unverified (relations and reach not specified): the
    // narrowest answers.
    fn in_own_inventory(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn within_reach(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn owner(&self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn is_pet(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn is_ally(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    // The list calls: reached only for a unit without a d2-sim list
    // (module docs); D2RUST's own, unverified: none.
    fn left_skill(&self, _: UnitId) -> Option<SkillEntry> {
        None
    }
    fn right_skill(&self, _: UnitId) -> Option<SkillEntry> {
        None
    }
    fn set_left_skill(&mut self, _: UnitId, _: SkillEntry) {}
    fn set_right_skill(&mut self, _: UnitId, _: SkillEntry) {}
    fn find_entry(&self, _: UnitId, _: i32) -> Option<SkillEntry> {
        None
    }
    fn find_entry_owned(&self, _: UnitId, _: i32, _: i32) -> Option<SkillEntry> {
        None
    }
    fn owns_skill(&self, _: UnitId, _: i32) -> bool {
        false
    }
    /// A monster has no skill list: its used skill is the request's
    /// current skill (`monsters::used_skill`); a clear (Charge's melee
    /// swing) leaves none, so the mode start
    /// it requests does not start the skill again.
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>) {
        match e {
            None => self.monsters.clear_current(u),
            Some(e) => {
                self.monsters.set_current(u, e.skill);
            }
        }
    }
    fn used_skill_flags(&self, u: UnitId) -> u32 {
        self.skills.unit(u).map_or(0, |s| s.used_flags)
    }
    fn set_used_skill_flags(&mut self, u: UnitId, f: u32) {
        self.skills.unit_mut(u).used_flags = f;
    }
    fn entry_mode(&self, _: UnitId, _: &SkillEntry) -> u32 {
        0
    }
    fn attack_param4(&self, u: UnitId) -> i32 {
        self.skills.unit(u).map_or(0, |s| s.attack_param4)
    }
    fn set_attack_param4(&mut self, u: UnitId, v: i32) {
        self.skills.unit_mut(u).attack_param4 = v;
    }
    // The use state, first failure wins: (1) record
    // missing or `InGame` clear → 3; (2) level 0 → 7; (3) `aura` → 6; (4)
    // `passive` → 5. Tests 5–10 (item test, mana, shape, start stat,
    // charges, cooldown) are not run here (no item, mana or cooldown
    // provider on this seam), so the rest is "usable". This does not show on
    // the wire: with mana 1 below Fire Bolt's cost the client sends the same
    // messages either way (the refusal is the
    // server's), and the HUD button colour comes from the model's use state
    // (`char_desc::hand_use_code`).
    // Before the first [`sync_shapes`] the flag table is empty and only the
    // level test runs. D2RUST's own.
    fn use_state(&mut self, _: UnitId, e: &SkillEntry) -> UseState {
        if !self.skills.flags.is_empty() {
            let f = usize::try_from(e.skill)
                .ok()
                .and_then(|i| self.skills.flags.get(i));
            if !f.is_some_and(|f| f.in_game) {
                return UseState::Disabled;
            }
        }
        if e.base + e.level_bonus <= 0 {
            return UseState::NoLevel;
        }
        let f = usize::try_from(e.skill)
            .ok()
            .and_then(|i| self.skills.flags.get(i));
        match f {
            Some(f) if f.aura => UseState::Aura,
            Some(f) if f.passive => UseState::Passive,
            _ => UseState::Usable,
        }
    }
    // D2RUST's own, unverified: the player has the state of a Werewolf /
    // Werebear row ([`sync_shapes`], refreshed before each intent and tick).
    fn shapeshifted(&self, u: UnitId) -> bool {
        self.skills.shifted.contains(&u)
    }
    // D2RUST's own, unverified: a native entry has no charges to spend; an
    // item entry's charges are refused (no item provider).
    fn consume_charges(&mut self, _: UnitId, e: &SkillEntry) -> bool {
        e.is_native() && !e.has_charges
    }
    // D2RUST's own, unverified: a life cost is refused (the stat write is not
    // reachable from here).
    fn pay_life(&mut self, u: UnitId, cost: i32) -> bool {
        if cost <= 0 {
            return true;
        }
        self.note(format!("pay life {} {cost}: refused", u.0));
        false
    }
    // Dual wield on the weapons
    // copy: player class 4 or 6 and the usable test
    // (neither flag 0x100 nor 0x4000); Left Hand Swing comes first.
    fn can_dual_wield(&self, u: UnitId) -> bool {
        self.weapons.hands.get(&u).is_some_and(|h| h.dual)
    }
    fn equippable(&self, item: UnitId) -> bool {
        self.weapons.items.get(&item).is_some_and(|f| !f.invalid)
    }
    // q-amazon: the weapons of [`super::weapons`].
    fn bow_equipped(&self, u: UnitId) -> bool {
        matches!(
            self.weapons.hand_class(u),
            super::weapons::class::BOW | super::weapons::class::CROSSBOW
        )
    }
    fn skill_weapon(&self, u: UnitId) -> Option<UnitId> {
        self.weapons.weapon(u)
    }
    fn skill_damage_weapon(
        &self,
        u: UnitId,
        weapsel: u8,
        idx: i32,
        flag: bool,
    ) -> Option<Option<UnitId>> {
        self.weapons
            .hands
            .contains_key(&u)
            .then(|| self.weapons.damage_weapon(u, weapsel, idx, flag))
    }
    fn skill_item_at(&self, u: UnitId, loc: u8) -> Option<UnitId> {
        self.weapons.item_at(u, loc)
    }
    fn hand_item_facts(
        &self,
        item: UnitId,
    ) -> Option<d2_sim::wiring::interaction::skill_use::HandItemFacts> {
        let f = self.weapons.items.get(&item)?;
        Some(d2_sim::wiring::interaction::skill_use::HandItemFacts {
            ammo_free: f.invalid,
            shoots: i32::from(f.ammo),
            stackable: f.stackable,
        })
    }
    fn state_mask(&self, _: UnitId, _: u32) -> bool {
        false
    }
    // The message handlers run the start on the unit records (`World`);
    // on the timer path it is logged. D2RUST's own, unverified.
    fn start_mode(&mut self, _: &mut Game, u: UnitId, mode: u32, _: ModeTarget<UnitId>) {
        self.note(format!("start mode {} {mode}", u.0));
    }
    // D2RUST's own, unverified (path spec not written): logged.
    fn run_to(&mut self, u: UnitId, target: UnitId, e: SkillEntry) {
        self.note(format!("run to {} {} skill {}", u.0, target.0, e.skill));
    }
    fn target(&self, u: UnitId) -> Option<UnitId> {
        // The monster side: the AI's request target (the same path target
        // unit); the start's drop clears both.
        let kept = self.skills.unit(u).and_then(|s| match s.target? {
            ModeTarget::Unit(t) => Some(t),
            ModeTarget::Point(..) => None,
        });
        kept.or_else(|| self.monsters.target(u))
    }
    /// The one path target
    /// unit, so the AI's request target goes too.
    fn clear_target(&mut self, u: UnitId) {
        self.skills.unit_mut(u).target = None;
        self.monsters.clear_target(u);
    }
    // D2RUST's own, unverified: the mode start's target kept per
    // unit.
    fn keep_target(&mut self, u: UnitId, target: ModeTarget<UnitId>) {
        self.skills.unit_mut(u).target = Some(target);
    }
    fn event_arg(&self, u: UnitId) -> i32 {
        self.skills.unit(u).map_or(0, |s| s.event_arg)
    }
    fn set_event_arg(&mut self, u: UnitId, a: i32) {
        self.skills.unit_mut(u).event_arg = a;
    }
    // D2RUST's own, unverified: no path step here (0: not finished).
    fn step_path(&mut self, _: UnitId) -> i32 {
        0
    }
    // D2RUST's own, unverified: the kept point, or the kept unit's position.
    fn target_position(&self, u: UnitId) -> Option<(i32, i32)> {
        match self.skills.unit(u)?.target? {
            ModeTarget::Point(x, y) => Some((x, y)),
            ModeTarget::Unit(t) => self
                .sides
                .get(&t)
                .map(|s| s.2)
                .or_else(|| self.pos.get(&t).copied()),
        }
    }
    // The game always tests the room line. The play host
    // turns the path provider on (`enable_paths` in `single_player`), so the
    // use view answers from the DRLG collision (`rooms_line_clear`) and never
    // asks here; only a host without rooms (tests) reaches this "clear"
    // (a "blocked" answer would refuse every `lineofsight` skill).
    fn line_clear(&self, _: UnitId, _: (i32, i32), _: u32) -> bool {
        true
    }
    fn set_aura_state(&mut self, u: UnitId, state: u16, skill: i32, lvl: i32) {
        self.note(format!("aura state {} {state} {skill} {lvl}", u.0));
    }
    // Every skill function row is
    // implemented except the null slots and the 3 unreferenced rows, and
    // the wiring (`UseView`) runs the bodies it implements before it asks
    // this seam. A slot the wiring does not run reaches this seam and is
    // only logged, result 0. The play tests with every
    // `srvst` / `srvdo` note printed (`--ignored`) reach it with no index,
    // Leap and Whirlwind included. D2RUST's own.
    fn srvst(&mut self, index: u16, u: UnitId, skill: i32, lvl: i32) -> i32 {
        self.note(format!("srvst {index} {} {skill} {lvl}", u.0));
        0
    }
    fn srvdo(&mut self, i: u16, u: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.note(format!("srvdo {i} {} {s} {l} {c} {it} {a}", u.0));
        0
    }
}

/// The skill-point calls.
// The class-skill test and the skill-point spend run in
// the server's `World` (`LearnUnits::is_class_skill` ORs this seam with the
// `skills` row's `charclass` against the player's class; `add_skill_level`
// takes stat 5 and adds the level), so this seam adds nothing: `false`
// leaves the table test as the only one.
impl LearnRest for LocalSeams {
    fn is_class_skill(&self, _: UnitId, _: i32) -> bool {
        // The table test is the server `World`'s (see above).
        false
    }
    fn add_skill_level(&mut self, u: UnitId, skill: i32, cost: i32) {
        self.note(format!("add skill level {} {skill} {cost}", u.0));
    }
    fn after_skill_point(&mut self, u: UnitId) {
        self.note(format!("after skill point {}", u.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(skill: i32, base: i32) -> SkillEntry {
        SkillEntry {
            skill,
            base,
            level_bonus: 0,
            owner_guid: -1,
            charges: 0,
            has_charges: false,
        }
    }

    fn seams() -> LocalSeams {
        let mut s = LocalSeams::default();
        s.skills.flags = vec![
            SkillFlagsRow {
                in_game: true,
                aura: false,
                passive: false,
            }, // 0 plain
            SkillFlagsRow {
                in_game: true,
                aura: false,
                passive: true,
            }, // 1 passive
            SkillFlagsRow {
                in_game: true,
                aura: true,
                passive: false,
            }, // 2 aura
            SkillFlagsRow {
                in_game: false,
                aura: false,
                passive: false,
            }, // 3 not in game
        ];
        s
    }

    #[test]
    fn the_start_drop_clears_a_monsters_request_target() {
        let mut s = seams();
        let (m, t) = (UnitId(5), UnitId(9));
        s.monsters
            .set_target(m, d2_sim::monsters::ai::ModeTarget::Unit(t));
        assert_eq!(s.target(m), Some(t));
        s.clear_target(m);
        assert_eq!(
            s.target(m),
            None,
            "clearing a skill target clears the path target"
        );
    }

    #[test]
    fn a_monsters_new_request_replaces_the_target_an_earlier_start_kept() {
        use d2_sim::monsters::ai::ModeTarget as AiTarget;
        use d2_sim::wiring::action::pending::Pending;
        let mut s = seams();
        let (m, old, new) = (UnitId(5), UnitId(9), UnitId(12));
        // An earlier cast's start kept `old` (the Shaman's f328 Resurrect).
        s.keep_target(m, ModeTarget::Unit(old));
        // The next request targets `new`: one path target, so `old` goes.
        Pending::set_mode_target(&mut s, m, AiTarget::Unit(new));
        assert_eq!(s.target(m), Some(new));
        // A point request leaves no target unit at all.
        s.keep_target(m, ModeTarget::Unit(old));
        Pending::set_mode_target(&mut s, m, AiTarget::Point(10, 20));
        assert_eq!(s.target(m), None);
    }

    #[test]
    fn use_state_order() {
        let mut s = seams();
        let u = UnitId(1);
        assert_eq!(s.use_state(u, &entry(0, 1)), UseState::Usable);
        assert_eq!(s.use_state(u, &entry(0, 0)), UseState::NoLevel);
        // passive with a level: 5; aura: 6
        assert_eq!(s.use_state(u, &entry(1, 1)), UseState::Passive);
        assert_eq!(s.use_state(u, &entry(2, 1)), UseState::Aura);
        // level 0 is tested before aura / passive
        assert_eq!(s.use_state(u, &entry(1, 0)), UseState::NoLevel);
        // InGame clear and no level → 3 (InGame first)
        assert_eq!(s.use_state(u, &entry(3, 0)), UseState::Disabled);
        // a missing record → 3
        assert_eq!(s.use_state(u, &entry(99, 1)), UseState::Disabled);
    }
}
