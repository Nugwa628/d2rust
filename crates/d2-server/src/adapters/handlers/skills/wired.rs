//! [`WiredSkills`]: the [`SkillHost`] of a game whose timer events run
//! the action wiring (`d2_sim::wiring::action::ActionSim`, alone or
//! inside another dispatch: [`ActionEvents`]). Each message builds a
//! [`World`] over the skill use pipeline's `d2-sim` provider
//! (`ActionSim::skill_use`) and the message's staged facts, and runs the
//! `d2-sim` handler that owns its rules. The skill and vitals tables are the
//! action wiring's (`ActionHooks::tables`, `ActionHooks::vitals`).

use d2_sim::combat::vitals::{self, VitalsTables};
use d2_sim::skills::use_::{self, MsgResult, ServerMsg, TargetError};
use d2_sim::skills::{check_skill_point, spend_skill_point, SkillPointCheck, SkillTables};
use d2_sim::units::UnitId;

use super::super::world::ActionEvents;
use super::world::World;
use super::{code, Call, Handled, LearnRest, SkillHost, SkillRest};
use crate::seams::{ClientId, ResultCode};

/// S→C 0x5A "can't do that", sent when a skill cast fails for lack of
/// mana: 40 bytes `5A 0E 01` and zeros to the caster's client, queued in
/// the frame of the C→S message (for example 0x0C).
pub const CANT_DO: [u8; 40] = {
    let mut m = [0u8; 40];
    m[0] = 0x5A;
    m[1] = 0x0E;
    m[2] = 0x01;
    m
};

/// The skill handlers on the wired sim.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WiredSkills {
    /// Messages the pipeline sent that are not queued as bytes: none
    /// today (0x5A is queued as [`CANT_DO`], 0x15 as a resync).
    pub unsent: Vec<(ClientId, ServerMsg)>,
}

impl<D: ActionEvents> SkillHost<D> for WiredSkills
where
    D::X: SkillRest,
{
    /// 0x3A needs the vitals tables (`ActionHooks::vitals`); without them
    /// it stays a stub.
    fn handle(&mut self, call: Call<'_, D>) -> Option<Handled> {
        let Call {
            game,
            events,
            client: _,
            msg,
            staged,
        } = call;
        let player = staged.player;
        let action = events.action();
        let tables = action.hooks().tables.clone();
        let vitals_t = action.hooks().vitals.clone();
        if msg.first() == Some(&0x3A) && vitals_t.is_none() {
            return None;
        }
        let (result, sends, point_accept) = action.skill_use(game, |u| {
            let mut w = World::new(u, staged);
            let r = run(&mut w, &tables.skills, vitals_t.as_deref(), player, msg);
            (r, std::mem::take(&mut w.sends), w.point_accept)
        });
        let (mut resync, mut cant_do) = (false, false);
        for m in sends {
            match m {
                ServerMsg::Resync => resync = true,
                ServerMsg::CantDo => cant_do = true,
            }
        }
        Some(Handled {
            code: result,
            point_accept,
            resync,
            cant_do,
        })
    }
}

/// The handler of `msg[0]` (one of the [`super::Status::Handled`] ids)
/// on `w`. `vitals_t`: the vitals tables (0x3A only).
pub fn run<X: SkillRest>(
    w: &mut World<'_, '_, X>,
    skills: &SkillTables,
    vitals_t: Option<&VitalsTables>,
    u: UnitId,
    msg: &[u8],
) -> ResultCode {
    match msg.first().copied() {
        // `client-messages.tsv`: the 0x0B handler does nothing, returns 0
        // (not a skill message: no `pierce_idx`).
        Some(0x0B) => ResultCode::Done,
        // The vitals' view is the action wiring's (`VitalsUnits` on its
        // `CombatView`: unit records, stat lists, the refresh that queues
        // the unit for its update).
        Some(0x3A) => match vitals_t {
            Some(t) => code(vitals::handle_add_stat_point(&mut w.u.cv, t, u, msg)),
            None => ResultCode::Malformed,
        },
        Some(0x3B) => add_skill_point(w, skills, u, msg),
        Some(use_::msg::SELECT_SKILL) => code(use_::select_skill(w, skills, u, msg)),
        _ => match use_::handle_message(w, skills, u, msg) {
            Some(MsgResult::Code(c)) => code(c),
            // The skill use rules leave these two codes open; the dispatcher's
            // target validation fixes them: bad type → 2, far → 1.
            Some(MsgResult::Unspecified(TargetError::BadType)) => ResultCode::Invalid,
            Some(MsgResult::Unspecified(_)) => ResultCode::Refused,
            None => ResultCode::Malformed,
        },
    }
}

/// 0x3B AddSkillPoint: size 3 (checked by the dispatcher), skill = u16 at
/// +1; validator → 2 / 3; spend; then the follow-up
/// ([`LearnRest::after_skill_point`]).
pub fn add_skill_point<X: SkillRest>(
    w: &mut World<'_, '_, X>,
    t: &SkillTables,
    u: UnitId,
    m: &[u8],
) -> ResultCode {
    let Some(b) = m.get(1..3) else {
        return ResultCode::Malformed;
    };
    let skill = i32::from(u16::from_le_bytes([b[0], b[1]]));
    match check_skill_point(w, t, u, skill) {
        SkillPointCheck::Code2 => return ResultCode::Invalid,
        SkillPointCheck::Code3 => return ResultCode::Malformed,
        SkillPointCheck::Ok => {}
    }
    spend_skill_point(w, t, u, skill);
    // TODO: the follow-up is read as running after the spend whether or
    // not a level was added; the result is 0.
    LearnRest::after_skill_point(w.x_mut(), u);
    ResultCode::Done
}
