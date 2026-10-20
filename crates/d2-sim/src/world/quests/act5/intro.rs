// Spec: specs/world/quests-act5-2.md §9 (Act V intro, chain 40), specs/world/quests.md §6.7
//! The Act V intro record: per-NPC first-talk text kept in the player's
//! NPC intro record (events 0 and 11, the active function), and the
//! Siege's start from Malah's intro line.
//!
//! The init `0x0058EA50` stores the callbacks, functions and table
//! itself (§9), so [`init`] writes them on the record, and the table's
//! rows are [`TABLE`] here. `quests.tsv` row 40 and the table
//! `0x00732FF8` in `quest-messages.tsv` state the same values
//! (`quests.md` open question 6, answered); the test
//! `act5_intro_matches_its_rows` keeps both equal.

use super::super::{EventArgs, QuestControl, QuestFlags, QuestRecord, QuestWorld, TextList};
use crate::units::UnitId;

/// NPC classes of the table (§9).
pub const DREHYA: u16 = 512;
pub const MALAH: u16 = 513;
pub const NIHLATHAK: u16 = 514;
pub const QUAL_KEHK: u16 = 515;
pub const CAIN6: u16 = 520;
/// Functions `0x0058EA50` stores (§9); its callbacks are event 0
/// `0x00586B50` and event 11 `0x0058E990`.
const STATUS_FN: u32 = 0x0058_6C40;
const ACTIVE_FN: u32 = 0x0058_6C50;
/// The message table's address (+0xDC).
pub const TABLE_ADDR: u32 = 0x0073_2FF8;
/// §9's table `0x00732FF8`: (table state, NPC, message), slots in the
/// order listed; every menu 0; state 3 is empty.
pub const TABLE: [(u8, u16, u16); 15] = [
    (0, DREHYA, 20014),
    (0, MALAH, 20037),
    (0, NIHLATHAK, 20053),
    (0, QUAL_KEHK, 20065),
    (0, CAIN6, 20003),
    (1, DREHYA, 20014),
    (1, MALAH, 20039),
    (1, NIHLATHAK, 20054),
    (1, QUAL_KEHK, 20067),
    (1, CAIN6, 20003),
    (2, DREHYA, 20014),
    (2, MALAH, 20038),
    (2, NIHLATHAK, 20055),
    (2, QUAL_KEHK, 20066),
    (2, CAIN6, 20003),
];
/// Player classes (`charstats.txt` rows).
const SORCERESS: u8 = 1;
const NECROMANCER: u8 = 2;
const PALADIN: u8 = 3;
const BARBARIAN: u8 = 4;
const DRUID: u8 = 5;
const ASSASSIN: u8 = 6;

/// Quest extra data: none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {}

/// Timers this quest makes: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init `0x0058EA50` (§9): events 0 and 11, the table, active 1, state 0,
/// status 0, filter 42, the status and active functions.
pub fn init(r: &mut QuestRecord) {
    r.active = true;
    r.state = 0;
    r.status = 0;
    r.filter = 42;
    r.callbacks |= 1 << super::super::event::NPC_ACTIVATE;
    r.callbacks |= 1 << super::super::event::SCROLL_MESSAGE;
    r.status_fn = Some(STATUS_FN);
    r.active_fn = Some(ACTIVE_FN);
    r.msgs = Some(TABLE_ADDR);
}

/// `0x005723C0`: the NPC's intro bit.
fn heard<W: QuestWorld>(w: &mut W, player: UnitId, class: u16) -> bool {
    let d = usize::from(w.difficulty());
    w.quests(player).is_some_and(|q| q.heard(d, class))
}

/// `0x00572360`: set the NPC's intro bit.
fn hear<W: QuestWorld>(w: &mut W, player: UnitId, class: u16) {
    let d = usize::from(w.difficulty());
    if let Some(q) = w.quests(player) {
        q.hear(d, class);
    }
}

/// The table's lines for (state, NPC), as `0x00543790` adds them.
pub fn lines(state: u8, npc: u16) -> Vec<(u16, u32)> {
    TABLE
        .iter()
        .filter(|e| e.0 == state && e.1 == npc)
        .map(|e| (e.2, 0))
        .collect()
}

/// Event 0's table state for the NPC and player class (§9).
pub fn table_state(npc: u16, player_class: u8) -> Option<u8> {
    Some(match (npc, player_class) {
        (MALAH, BARBARIAN) | (NIHLATHAK, ASSASSIN) | (QUAL_KEHK, DRUID) => 1,
        (MALAH, SORCERESS) | (NIHLATHAK, NECROMANCER) | (QUAL_KEHK, PALADIN) => 2,
        (DREHYA | MALAH | NIHLATHAK | QUAL_KEHK | CAIN6, _) => 0,
        _ => return None,
    })
}

/// One callback of record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    let _ = i;
    match args.event {
        super::super::event::NPC_ACTIVATE => {
            // `0x00586B50`.
            let (Some(p), Some(n)) = (args.player, args.target) else {
                return true;
            };
            let Some(class) = w.monster_class(n) else {
                return true;
            };
            if heard(w, p, class) {
                return true;
            }
            let pc = w.player_class(p);
            if let (Some(k), Some(list)) = (table_state(class, pc), list) {
                list.extend(lines(k, class));
            }
        }
        super::super::event::SCROLL_MESSAGE => {
            // `0x0058E990`: jump on NPC − 512.
            let Some(p) = args.player else { return true };
            let Ok(npc) = u16::try_from(args.a) else {
                return true;
            };
            let hit = match npc {
                DREHYA => args.b == 20014,
                CAIN6 => args.b == 20003,
                NIHLATHAK => (20053..=20055).contains(&args.b),
                QUAL_KEHK => (20065..=20067).contains(&args.b),
                MALAH => (20037..=20039).contains(&args.b),
                _ => false,
            };
            if !hit {
                return true;
            }
            hear(w, p, npc);
            if npc == MALAH {
                // The Siege starts.
                if let Some(r) = ctl.record_mut(31) {
                    if r.not_intro && r.state == 0 {
                        r.state = 1;
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

/// Active function `0x00586C50`: malah with its intro bit clear.
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = (ctl, i, f);
    npc_class == MALAH && !heard(w, player, MALAH)
}

/// Status function `0x00586C40`: returns false (nothing reported).
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = (ctl, w, i, player, pf, f);
    None
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let _ = (ctl, w, chain);
    match t {}
}

#[cfg(test)]
#[path = "intro_tests.rs"]
mod tests;
