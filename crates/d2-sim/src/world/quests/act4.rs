// Spec: specs/world/quests-act4.md §1.3 (sequence chain), §2 (records)
//! Act IV quest records: The Fallen Angel ([`q1`], chain 22), Terror's
//! End ([`q2`], chain 23), Hell's Forge ([`q3`], chain 24) and the
//! Tyrael / Hadriel gossip records ([`gossip`], chains 21 and 29). This
//! module routes the shared machinery's calls (`quests.md` §4–§6) to
//! them and runs the sequence chain (§1.3).

pub mod gossip;
pub mod q1;
pub mod q2;
pub mod q3;

use super::{EventArgs, QuestControl, QuestError, QuestFlags, QuestRecord, QuestWorld, TextList};
use crate::units::UnitId;

/// Per-record extra data of the Act IV records (record +0x18).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    pub q1: q1::Extra,
    pub q2: q2::Extra,
    pub q3: q3::Extra,
}

/// Act IV quest timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    Q1(q1::Timer),
    Q2(q2::Timer),
}

/// True for the Act IV chains.
pub fn owns(chain: u8) -> bool {
    matches!(chain, 21..=24 | 29)
}

/// Record init beyond `quests.tsv` (§2): every Act IV record is active;
/// chain 22 starts at state 1.
pub fn init(r: &mut QuestRecord) {
    r.active = true;
    r.state = u8::from(r.chain == 22);
    match r.chain {
        22 => q1::init(r),
        23 => q2::init(r),
        24 => q3::init(r),
        21 | 29 => gossip::init(r),
        _ => {}
    }
}

/// The sequence functions of chains 22, 24, 23 (§1.3). Returns the
/// function's result (1 = true).
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return false;
    };
    let (state, not_intro) = (ctl.records[i].state, ctl.records[i].not_intro);
    let next = match chain {
        // `0x005B38E0`.
        22 => {
            if state != 5 && not_intro {
                return true;
            }
            24
        }
        // `0x005B5F40`.
        24 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
                return true;
            }
            if state != 5 && not_intro {
                return true;
            }
            23
        }
        // `0x005B4530`.
        23 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
            }
            return true;
        }
        _ => {
            let f = ctl.records[i].seq_fn.unwrap_or(0);
            w.unhandled(chain, f);
            return false;
        }
    };
    seq(ctl, w, next)
}

/// "Call seq(c)" (§1.3): look up chain `c` (absent → 0) and call its
/// sequence function, fatal when the target's +0xF0 is bad.
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

/// One callback of Act IV record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match ctl.records[i].chain {
        22 => q1::callback(ctl, w, i, args, list),
        23 => q2::callback(ctl, w, i, args, list),
        24 => q3::callback(ctl, w, i, args, list),
        21 | 29 => gossip::callback(ctl, w, i, args, list),
        _ => false,
    }
}

/// Active functions (§6.4 of `quests.md`).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    match ctl.records[i].chain {
        22 => q1::active(ctl, w, i, player, npc_class, f),
        23 => q2::active(ctl, w, i, player, npc_class, f),
        24 => q3::active(ctl, w, i, player, npc_class, f),
        _ => gossip::active(ctl, w, i, player, npc_class, f),
    }
}

/// Status functions (§2): chains 21 and 29 only.
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    match ctl.records[i].chain {
        22 => q1::status(ctl, w, i, player, pf, f),
        23 => q2::status(ctl, w, i, player, pf, f),
        24 => q3::status(ctl, w, i, player, pf, f),
        _ => gossip::status(ctl, w, i, player, pf, f),
    }
}

/// Runs an Act IV timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Q1(t) => q1::run_timer(ctl, w, t, chain),
        Timer::Q2(t) => q2::run_timer(ctl, w, t, chain),
    }
}
