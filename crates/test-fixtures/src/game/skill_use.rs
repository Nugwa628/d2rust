//! Minimal skill-use adapter for synthetic sessions. Skill entries, missiles,
//! mana, delay lists and timers use the sim's providers. Unprovided operations
//! fail explicitly, so a report cannot silently measure a stub skill body.
use super::Seams;
use d2_sim::game::Game;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::SkillEntry;
use d2_sim::units::UnitId;
use d2_sim::wiring::interaction::UseRest;

impl UseRest for Seams {
    fn start_mode(&mut self, _: &mut Game, _: UnitId, _: u32, _: ModeTarget<UnitId>) {
        panic!("fixture skill seam: start_mode")
    }
    fn event_arg(&self, _: UnitId) -> i32 {
        0
    }

    fn send(&mut self, _u: UnitId, _msg: ServerMsg) {
        panic!("fixture skill seam: send")
    }
    fn has_player_data(&self, _u: UnitId) -> bool {
        panic!("fixture skill seam: has_player_data")
    }
    fn last_point_frame(&self, _u: UnitId) -> i32 {
        panic!("fixture skill seam: last_point_frame")
    }
    fn set_last_point_frame(&mut self, _u: UnitId, _frame: i32) {
        panic!("fixture skill seam: set_last_point_frame")
    }
    fn cursor_item(&self, _u: UnitId) -> bool {
        panic!("fixture skill seam: cursor_item")
    }
    fn in_own_inventory(&self, _u: UnitId, _item: UnitId) -> bool {
        panic!("fixture skill seam: in_own_inventory")
    }
    fn within_reach(&self, _a: UnitId, _b: UnitId) -> bool {
        panic!("fixture skill seam: within_reach")
    }
    fn owner(&self, _u: UnitId) -> Option<UnitId> {
        panic!("fixture skill seam: owner")
    }
    fn is_pet(&self, _a: UnitId, _b: UnitId) -> bool {
        panic!("fixture skill seam: is_pet")
    }
    fn is_ally(&self, _a: UnitId, _b: UnitId) -> bool {
        panic!("fixture skill seam: is_ally")
    }
    fn left_skill(&self, _u: UnitId) -> Option<SkillEntry> {
        panic!("fixture skill seam: left_skill")
    }
    fn right_skill(&self, _u: UnitId) -> Option<SkillEntry> {
        panic!("fixture skill seam: right_skill")
    }
    fn set_left_skill(&mut self, _u: UnitId, _e: SkillEntry) {
        panic!("fixture skill seam: set_left_skill")
    }
    fn set_right_skill(&mut self, _u: UnitId, _e: SkillEntry) {
        panic!("fixture skill seam: set_right_skill")
    }
    fn find_entry(&self, _u: UnitId, _skill: i32) -> Option<SkillEntry> {
        panic!("fixture skill seam: find_entry")
    }
    fn find_entry_owned(&self, _u: UnitId, _skill: i32, _owner: i32) -> Option<SkillEntry> {
        panic!("fixture skill seam: find_entry_owned")
    }
    fn owns_skill(&self, _u: UnitId, _skill: i32) -> bool {
        panic!("fixture skill seam: owns_skill")
    }
    fn set_used_skill(&mut self, _u: UnitId, _e: Option<SkillEntry>) {
        panic!("fixture skill seam: set_used_skill")
    }
    fn used_skill_flags(&self, _u: UnitId) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _u: UnitId, _f: u32) {
        panic!("fixture skill seam: set_used_skill_flags")
    }
    fn entry_mode(&self, _u: UnitId, _e: &SkillEntry) -> u32 {
        panic!("fixture skill seam: entry_mode")
    }
    fn attack_param4(&self, _u: UnitId) -> i32 {
        panic!("fixture skill seam: attack_param4")
    }
    fn set_attack_param4(&mut self, _u: UnitId, _v: i32) {
        panic!("fixture skill seam: set_attack_param4")
    }
    fn use_state(&mut self, _u: UnitId, _e: &SkillEntry) -> UseState {
        panic!("fixture skill seam: use_state")
    }
    fn shapeshifted(&self, _u: UnitId) -> bool {
        panic!("fixture skill seam: shapeshifted")
    }
    fn consume_charges(&mut self, _u: UnitId, _e: &SkillEntry) -> bool {
        panic!("fixture skill seam: consume_charges")
    }
    fn pay_life(&mut self, _u: UnitId, _cost: i32) -> bool {
        panic!("fixture skill seam: pay_life")
    }
    fn can_dual_wield(&self, _u: UnitId) -> bool {
        panic!("fixture skill seam: can_dual_wield")
    }
    fn equippable(&self, _item: UnitId) -> bool {
        panic!("fixture skill seam: equippable")
    }
    fn bow_equipped(&self, _u: UnitId) -> bool {
        panic!("fixture skill seam: bow_equipped")
    }
    fn state_mask(&self, _u: UnitId, _mask: u32) -> bool {
        panic!("fixture skill seam: state_mask")
    }
    fn run_to(&mut self, _u: UnitId, _target: UnitId, _e: SkillEntry) {
        panic!("fixture skill seam: run_to")
    }
    fn target(&self, _u: UnitId) -> Option<UnitId> {
        panic!("fixture skill seam: target")
    }
    fn clear_target(&mut self, _u: UnitId) {
        panic!("fixture skill seam: clear_target")
    }
    fn set_event_arg(&mut self, _u: UnitId, _a: i32) {
        panic!("fixture skill seam: set_event_arg")
    }
    fn step_path(&mut self, _u: UnitId) -> i32 {
        panic!("fixture skill seam: step_path")
    }
    fn target_position(&self, _u: UnitId) -> Option<(i32, i32)> {
        self.skill_target
    }
    fn line_clear(&self, _u: UnitId, _to: (i32, i32), _mask: u32) -> bool {
        panic!("fixture skill seam: line_clear")
    }
    fn set_aura_state(&mut self, _u: UnitId, _state: u16, _skill: i32, _lvl: i32) {
        panic!("fixture skill seam: set_aura_state")
    }
    fn srvst(&mut self, _index: u16, _u: UnitId, _skill: i32, _lvl: i32) -> i32 {
        panic!("fixture skill seam: srvst")
    }
    fn srvdo(
        &mut self,
        _index: u16,
        _u: UnitId,
        _skill: i32,
        _lvl: i32,
        _charge: bool,
        _item: bool,
        _aim: bool,
    ) -> i32 {
        panic!("fixture skill seam: srvdo")
    }
}
