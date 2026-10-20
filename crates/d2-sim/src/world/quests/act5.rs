// Spec: specs/world/quests-act5.md §1.3 (sequence chain), §2 (records); specs/world/quests-act5-2.md §6.10, §7.10, §8.9
//! Act V quest records: Siege on Harrogath ([`q1`], chain 31), Rescue
//! on Mount Arreat ([`q2`], 32), Prison of Ice ([`q3`], 33), Betrayal
//! of Harrogath ([`q4`], 34), Rite of Passage ([`q5`], 35), Eve of
//! Destruction ([`q6`], 36) and the Act V intro ([`intro`], 40). This
//! module routes the shared machinery's calls (`quests.md` §4–§6) to
//! them and runs the sequence chain (§1.3).

pub mod intro;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod q4;
pub mod q5;
pub mod q6;

use super::{EventArgs, QuestControl, QuestError, QuestFlags, QuestRecord, QuestWorld, TextList};
use crate::units::UnitId;

/// Per-record extra data of the Act V records (record +0x18).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    pub q1: q1::Extra,
    pub q2: q2::Extra,
    pub q3: q3::Extra,
    pub q4: q4::Extra,
    pub q5: q5::Extra,
    pub q6: q6::Extra,
}

/// Act V quest timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    Q1(q1::Timer),
    Q2(q2::Timer),
    Q3(q3::Timer),
    Q4(q4::Timer),
    Q5(q5::Timer),
    Q6(q6::Timer),
}

/// True for the Act V chains.
pub fn owns(chain: u8) -> bool {
    matches!(chain, 31..=36 | 40)
}

/// Record init beyond `quests.tsv` (§2): every Act V record is active,
/// state 0.
pub fn init(r: &mut QuestRecord) {
    r.active = true;
    r.state = 0;
    match r.chain {
        31 => q1::init(r),
        32 => q2::init(r),
        33 => q3::init(r),
        34 => q4::init(r),
        35 => q5::init(r),
        36 => q6::init(r),
        40 => intro::init(r),
        _ => {}
    }
}

/// The sequence functions of chains 31–35 (§1.3, part 2 §6.10, §7.10);
/// chain 36's (part 2 §8.9) is [`q6::sequence`]. Returns the function's
/// result (1 = true).
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return false;
    };
    let (state, not_intro) = (ctl.records[i].state, ctl.records[i].not_intro);
    // (open: the record holds; opens: state 0 → 1 while open).
    let (open, opens, next) = match chain {
        // `0x00587560`: no state change.
        31 => (state != 5, false, 32),
        // `0x005883E0`.
        32 => (state != 5, true, 33),
        // `0x00589160`.
        33 => (state < 5, true, 34),
        // `0x0058B140`.
        34 => (state < 4, true, 35),
        // `0x0058CD30`.
        35 => (state != 5, true, 36),
        36 => return q6::sequence(ctl, w, i),
        _ => {
            let f = ctl.records[i].seq_fn.unwrap_or(0);
            w.unhandled(chain, f);
            return false;
        }
    };
    if open && not_intro {
        if opens && state == 0 {
            ctl.records[i].state = 1;
        }
        return true;
    }
    seq(ctl, w, next)
}

/// "Call seq(c)" (§1.3): look up chain `c` (absent → 0) and call that
/// record's sequence function, fatal when its +0xF0 is bad.
fn seq<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, c: u8) -> bool {
    let Some(next) = ctl.find(c) else {
        return false;
    };
    if ctl.records[next].seq_fn.is_none() {
        ctl.faults.push(QuestError::NoSequenceFn(c));
        return false;
    }
    super::act1::sequence(ctl, w, c)
}

/// One callback of Act V record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match ctl.records[i].chain {
        31 => q1::callback(ctl, w, i, args, list),
        32 => q2::callback(ctl, w, i, args, list),
        33 => q3::callback(ctl, w, i, args, list),
        34 => q4::callback(ctl, w, i, args, list),
        35 => q5::callback(ctl, w, i, args, list),
        36 => q6::callback(ctl, w, i, args, list),
        40 => intro::callback(ctl, w, i, args, list),
        _ => false,
    }
}

/// Active functions (`quests.md` §6.4).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    match ctl.records[i].chain {
        31 => q1::active(ctl, w, i, player, npc_class, f),
        32 => q2::active(ctl, w, i, player, npc_class, f),
        33 => q3::active(ctl, w, i, player, npc_class, f),
        34 => q4::active(ctl, w, i, player, npc_class, f),
        35 => q5::active(ctl, w, i, player, npc_class, f),
        36 => q6::active(ctl, w, i, player, npc_class, f),
        _ => intro::active(ctl, w, i, player, npc_class, f),
    }
}

/// Status functions (§2): chains 31, 33, 34 and the intro.
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    match ctl.records[i].chain {
        31 => q1::status(ctl, w, i, player, pf, f),
        32 => q2::status(ctl, w, i, player, pf, f),
        33 => q3::status(ctl, w, i, player, pf, f),
        34 => q4::status(ctl, w, i, player, pf, f),
        35 => q5::status(ctl, w, i, player, pf, f),
        36 => q6::status(ctl, w, i, player, pf, f),
        _ => intro::status(ctl, w, i, player, pf, f),
    }
}

/// Runs an Act V timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Q1(t) => q1::run_timer(ctl, w, t, chain),
        Timer::Q2(t) => q2::run_timer(ctl, w, t, chain),
        Timer::Q3(t) => q3::run_timer(ctl, w, t, chain),
        Timer::Q4(t) => q4::run_timer(ctl, w, t, chain),
        Timer::Q5(t) => q5::run_timer(ctl, w, t, chain),
        Timer::Q6(t) => q6::run_timer(ctl, w, t, chain),
    }
}

/// `0x00588C50`: barbarians left for the u16 extra of 0x50 / 0x5D
/// (filter 36, `quests.md` §6.2, §6.3).
pub fn barbarians_left<W: QuestWorld>(ctl: &QuestControl, w: &mut W) -> u16 {
    q2::barbarians_left(ctl, w)
}
