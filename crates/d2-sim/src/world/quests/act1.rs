//! Act I quest callbacks and the quest function router: every Act I
//! chain callback by callback ([`q1`]–[`q6`], the intro record
//! [`intro`]), A1Q0, Flavie and the respec record here, with the shared
//! shorthands (broadcast, every player, add state, the sequence walk).
//! [`callback`], [`active_fn`], [`status_fn`] and [`sequence`] route
//! every chain to its act's module; a mod game's
//! [`QuestProvider`](super::QuestProvider) answers first.
//! A callback, status, active or sequence function that `quests.tsv`
//! registers but D2RUST does not have is reported through
//! `QuestWorld::unhandled` with its function id.

pub mod intro;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod q4;
pub mod q5;
pub mod q6;

pub use q3::{imbue_granted, malus_init, malus_operate};
pub use q4::{send_stone_order, stone_order, stone_order_from, wirt_body};
pub use q6::{gem_code, CHIPPED_GEMS, NORMAL_GEMS};

use super::{
    bit, event, flags_of, npc, EventArgs, GuidList, QuestControl, QuestError, QuestFlags,
    QuestRecord, QuestWorld, TextList, TimerFn,
};
use crate::units::UnitId;

/// Object class of Wirt's body (0x10C).
pub const WIRT_BODY: u16 = 0x10C;
/// Paladin class id.
pub const PALADIN: u8 = 3;
/// Per-quest extra data, the fields the quests keep. One
/// struct for every Act I record; each quest uses its own fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// A1Q1: Den of Evil cleared (written, never read).
    pub done: bool,
    /// A1Q1: level 8 entered (written, never read).
    pub entered: bool,
    /// The start message was given, chat end not yet handled (A1Q1,
    /// A1Q2, A1Q3).
    pub talked: bool,
    /// A1Q1: the status timer is pending.
    pub timer: bool,
    /// A1Q1: monsters left in level 8 (0x50 / 0x5D extra).
    pub monsters_left: i32,
    /// Player GUIDs: A1Q1 killers, A1Q3 players who brought the
    /// Malus.
    pub guids: GuidList,
    /// A1Q2: Blood Raven killed.
    pub killed: bool,
    /// A1Q2: two kill flags (both set on the kill; the second gates J3).
    pub kill_b1: bool,
    pub kill_b2: bool,
    /// A1Q2: set to 1 on the kill, never read by chain 2.
    pub kill_d4: u32,
    /// A1Q2: the victim's GUID.
    pub victim: u32,
    /// A1Q3: the Malus object is known.
    pub malus_known: bool,
    /// A1Q3: message 163 finished the quest, chat end not yet
    /// handled.
    pub rewarded: bool,
    /// A1Q3: the Malus object's GUID.
    pub malus_guid: u32,
    /// A1Q3: a byte cleared by the reset.
    pub b08: u8,
    /// A1Q3: the Malus object mode to restore (2: taken).
    pub malus_mode: i32,
    /// A1Q3: Malus items in the game.
    pub malus_items: i32,
    /// A1Q3: a player started the game carrying `hdm `.
    pub started_with_malus: bool,
    /// A1Q4: Cairn stone order, once computed.
    pub stone_order: Option<[u8; 5]>,
    /// A1Q4: gold piles left on Wirt's body.
    pub wirt_piles: Option<i32>,
    /// The Act IV records' fields.
    pub a4: super::act4::Extra,
    /// The Act V records' fields.
    pub a5: super::act5::Extra,
    /// A2Q6 (chain 13): the true tomb's level.
    pub tomb_level: u32,
    /// A1Q4's other fields.
    pub q4: q4::Extra4,
    /// A1Q5's fields.
    pub q5: q5::Extra5,
    /// A1Q6's fields.
    pub q6: q6::Extra6,
    /// The Act II records' fields.
    pub a2: super::act2::Extra,
    /// The Act III records' fields.
    pub act3: super::act3::Extra3,
}

/// Per-record init beyond the `quests.tsv` columns.
pub fn init(r: &mut QuestRecord) {
    match r.chain {
        1 => {
            r.active = true;
            r.state = 1;
        }
        2..=6 => {
            r.active = true;
            r.state = 0;
        }
        c if super::act4::owns(c) => super::act4::init(r),
        c if super::act5::owns(c) => super::act5::init(r),
        _ => {}
    }
}

/// Den of Evil monsters left (the u16 field of 0x50 and
/// 0x5D carries its low 16 bits).
pub fn den_monsters_left(r: &QuestRecord) -> u16 {
    r.extra.monsters_left as u16
}

/// The sequence function of `chain`, called
/// with its own record: the own step, then the pass test, then the walk
/// to the `seq_id` record. Returns the function's result (1 = true).
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8) -> bool {
    // The mod's sequence function first; `None` runs D2RUST's.
    let rules = w.rules().clone();
    if let Some(p) = rules.quests() {
        if let Some(r) = p.sequence(ctl, w, chain) {
            return r;
        }
    }
    let Some(i) = ctl.find(chain) else {
        return false;
    };
    let r = &ctl.records[i];
    let (state, not_intro, seq_fn) = (r.state, r.not_intro, r.seq_fn);
    let pass = match chain {
        1 => 5,
        2..=4 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
                return true;
            }
            if chain == 4 {
                6
            } else {
                5
            }
        }
        5 => {
            if state < 2 && not_intro {
                return true;
            }
            5
        }
        6 => {
            if state == 0 && not_intro {
                if let Err(e) = ctl.add_timer(6, TimerFn::SlaughterOpen, 20) {
                    ctl.faults.push(e);
                }
            }
            return true;
        }
        8 | 10 | 11 | 13 => return super::act2::sequence(ctl, w, chain),
        15..=20 => return super::act3::sequence(ctl, w, chain),
        c if super::act4::owns(c) => return super::act4::sequence(ctl, w, c),
        c if super::act5::owns(c) => return super::act5::sequence(ctl, w, c),
        _ => {
            w.unhandled(chain, seq_fn.unwrap_or(0));
            return false;
        }
    };
    if state != pass && not_intro {
        return true;
    }
    let Some(next) = ctl.records[i].seq_id.and_then(|s| ctl.find(s)) else {
        return false;
    };
    let next_chain = ctl.records[next].chain;
    if ctl.records[next].seq_fn.is_none() {
        ctl.faults.push(QuestError::NoSequenceFn(next_chain));
        return false;
    }
    sequence(ctl, w, next_chain)
}

fn player_flags<W: QuestWorld>(w: &mut W, p: UnitId) -> QuestFlags {
    flags_of(w, p).copied().unwrap_or_default()
}

/// Callback 13 restores state from the starting player's bits.
fn restore<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, player: UnitId) {
    let slot = ctl.records[i].filter;
    let f = player_flags(w, player);
    if f.get(slot, bit::REWARD_GRANTED) || f.get(slot, bit::COMPLETED_BEFORE) {
        return;
    }
    let r = &mut ctl.records[i];
    if f.get(slot, bit::ENTER_AREA) {
        (r.state, r.status) = (3, 2);
    } else if f.get(slot, bit::LEAVE_TOWN) {
        (r.state, r.status) = (3, 1);
    } else if f.get(slot, bit::STARTED) {
        (r.state, r.status) = (2, 1);
    }
}

/// R: the event player's current-difficulty record (zero when there is
/// no player record).
fn rec<W: QuestWorld>(w: &mut W, p: Option<UnitId>) -> QuestFlags {
    p.map_or_else(QuestFlags::default, |p| player_flags(w, p))
}

/// "add state k" for the NPC `target`, if it has a class.
fn add_state<W: QuestWorld>(
    ctl: &QuestControl,
    w: &W,
    i: usize,
    list: Option<&mut TextList>,
    target: Option<UnitId>,
    k: u8,
) {
    let class = target.and_then(|n| w.monster_class(n));
    if let (Some(t), Some(list), Some(c)) = (ctl.records[i].msgs, list, class) {
        ctl.add_messages(t, list, c, k);
    }
}

/// The status iterate of chains 1–3 (I1, J1, K1): 0x5D to `p` if
/// it has neither bit 0 nor 15 of the record's slot, or has 13 or 14.
fn status_iterate<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let (chain, slot) = (ctl.records[i].chain, ctl.records[i].filter);
    let f = player_flags(w, p);
    if (!f.get(slot, bit::REWARD_GRANTED) && !f.get(slot, bit::COMPLETED_BEFORE))
        || f.get(slot, bit::PRIMARY_GOAL_DONE)
        || f.get(slot, bit::COMPLETED_NOW)
    {
        if let Err(e) = ctl.send_status(w, p, chain) {
            ctl.faults.push(e);
        }
    }
}

/// status(S) with the status iterate for every player, flags as they
/// are.
fn status_all<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, status: u8) {
    ctl.records[i].status = status;
    for p in w.players() {
        status_iterate(ctl, w, i, p);
    }
}

/// broadcast(S, f) for chains 1–3.
fn broadcast<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, status: u8, flags: u8) {
    ctl.records[i].flags = flags;
    status_all(ctl, w, i, status);
}

/// Sends `5D chain 00 0C 0000` to `p` when
/// the player has no room, or its room's level is ≠ 0 and in an act ≥
/// `act`.
fn send_completed_now<W: QuestWorld>(w: &mut W, p: UnitId, chain: u8, act: u8) {
    let send = match w.unit_level(p) {
        None => true,
        Some(0) => false,
        Some(_) => w.unit_act(p).is_none_or(|a| a >= act),
    };
    if send {
        w.send(p, &[0x5D, chain, 0, 12, 0, 0]);
    }
}

/// I3's member step and A1Q2's J4: a
/// member with neither bit 0 nor 1 of `slot` and a room whose level is ≠
/// 0 and in Act I gets 13, then 1 (no 0x28).
fn member_goal<W: QuestWorld>(w: &mut W, m: UnitId, slot: u8) {
    let in_act1 = w.unit_level(m).is_some_and(|l| l != 0) && w.unit_act(m) == Some(0);
    let Some(f) = flags_of(w, m) else { return };
    if !f.get(slot, bit::REWARD_GRANTED) && !f.get(slot, bit::REWARD_PENDING) && in_act1 {
        f.set(slot, bit::PRIMARY_GOAL_DONE);
        f.set(slot, bit::REWARD_PENDING);
    }
}

/// I3 / J7: if `p` has bit 13 of `slot` and a party, the
/// member step for each member.
fn party_goal<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8) {
    if !player_flags(w, p).get(slot, bit::PRIMARY_GOAL_DONE) {
        return;
    }
    if let Some(members) = w.party_members(p) {
        for m in members {
            member_goal(w, m, slot);
        }
    }
}

/// The value `m` of a state's message-state table:
/// `None` for −1 or a state past the table.
fn table_state(table: &[i8], state: u8) -> Option<u8> {
    match table.get(usize::from(state)) {
        Some(&m) if m >= 0 => Some(m as u8),
        _ => None,
    }
}

/// Dispatches one callback of record `i`.
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    mut list: Option<&mut TextList>,
    force: bool,
) {
    let chain = ctl.records[i].chain;
    let ev = args.event;
    // The mod's callback first; `None` runs D2RUST's.
    let rules = w.rules().clone();
    let by_mod = match rules.quests() {
        Some(p) => p.callback(ctl, w, i, args, list.as_deref_mut(), force),
        None => None,
    };
    let handled = match by_mod {
        Some(handled) => handled,
        None => match (chain, ev) {
            (0, event::NPC_ACTIVATE) => warriv_gossip_text(ctl, w, i, args, list),
            (0, event::SCROLL_MESSAGE) => {
                if args.a == u32::from(npc::WARRIV1) && args.b <= 1 {
                    if let Some(f) = args.player.and_then(|p| flags_of(w, p)) {
                        f.set(0, bit::REWARD_GRANTED);
                    }
                }
                true
            }
            (25 | 30, event::NPC_ACTIVATE) => flavie_text(ctl, w, i, args, list),
            // An empty callback: nothing to do.
            (25 | 30, event::MONSTER_KILLED) => true,
            (1, _) => q1::callback(ctl, w, i, args, list),
            (2, _) => q2::callback(ctl, w, i, args, list),
            (3, _) => q3::callback(ctl, w, i, args, list),
            (4, _) => q4::callback(ctl, w, i, args, list),
            (5, _) => q5::callback(ctl, w, i, args, list),
            (6, _) => q6::callback(ctl, w, i, args, list),
            (37, _) => intro::callback(ctl, w, i, args, list),
            (7..=13 | 26 | 27 | 38, _) => super::act2::callback(ctl, w, i, args, list),
            (14..=20 | 28 | 39, _) => super::act3::callback(ctl, w, i, args, list),
            (c, _) if super::act4::owns(c) => super::act4::callback(ctl, w, i, args, list),
            (c, _) if super::act5::owns(c) => super::act5::callback(ctl, w, i, args, list),
            _ => false,
        },
    };
    if !handled {
        let f = ctl
            .rows
            .iter()
            .find(|r| r.chain == chain)
            .and_then(|r| r.callbacks.iter().find(|c| c.0 == ev))
            .map_or(0, |c| c.1);
        w.unhandled(chain, f);
    }
}

/// A1Q0 event 0: Warriv's gossip text.
fn warriv_gossip_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    let (Some(p), Some(n), Some(list)) = (args.player, args.target, list) else {
        return true;
    };
    if w.monster_class(n) != Some(npc::WARRIV1) || player_flags(w, p).get(0, bit::REWARD_GRANTED) {
        return true;
    }
    let state = u8::from(w.player_class(p) == PALADIN);
    if let Some(t) = ctl.records[i].msgs {
        ctl.add_messages(t, list, npc::WARRIV1, state);
    }
    true
}

/// Flavie's event 0 (records 25 and 30 alike).
fn flavie_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return true;
    };
    if w.monster_class(n) != Some(npc::NAVI) {
        return true;
    }
    let state = if flavie_open(ctl, w, p) {
        w.unit_seed(p).roll(2) as u8
    } else {
        (w.unit_seed(p).step() % 3) as u8 + 2
    };
    ctl.records[i].state = state;
    if let (Some(t), Some(list)) = (ctl.records[i].msgs, list) {
        ctl.add_messages(t, list, npc::NAVI, state);
    }
    true
}

/// Game slot 1 bit 13 clear, chain 1 not-intro, the player lacks slot 1
/// bits 0, 13, 1.
fn flavie_open<W: QuestWorld>(ctl: &QuestControl, w: &mut W, p: UnitId) -> bool {
    let f = player_flags(w, p);
    !ctl.game.get(1, bit::PRIMARY_GOAL_DONE)
        && ctl.record(1).is_some_and(|r| r.not_intro)
        && !f.get(1, bit::REWARD_GRANTED)
        && !f.get(1, bit::PRIMARY_GOAL_DONE)
        && !f.get(1, bit::REWARD_PENDING)
}

/// The active functions D2RUST has; others are unhandled (false).
pub fn active_fn<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let rules = w.rules().clone();
    if let Some(p) = rules.quests() {
        if let Some(a) = p.active(ctl, w, i, player, npc_class) {
            return a;
        }
    }
    match ctl.records[i].chain {
        0 => npc_class == npc::WARRIV1 && !player_flags(w, player).get(0, bit::REWARD_GRANTED),
        25 | 30 => npc_class == npc::NAVI && flavie_open(ctl, w, player),
        1 => q1::active(ctl, w, i, player, npc_class),
        2 => q2::active(ctl, w, i, player, npc_class),
        3 => q3::active(ctl, w, i, player, npc_class),
        4 => q4::active(ctl, w, i, player, npc_class),
        5 => q5::active(ctl, w, i, player, npc_class),
        6 => q6::active(ctl, w, i, player, npc_class),
        37 => intro::active(w, player, npc_class),
        7..=13 | 26 | 27 | 38 => super::act2::active_fn(ctl, w, i, player, npc_class, f),
        14..=20 | 28 | 39 => super::act3::active(ctl, w, i, player, npc_class),
        c if super::act4::owns(c) => super::act4::active(ctl, w, i, player, npc_class, f),
        c if super::act5::owns(c) => super::act5::active(ctl, w, i, player, npc_class, f),
        c => {
            w.unhandled(c, f);
            false
        }
    }
}

/// Status functions. A1Q0, Flavie and the intros
/// return false; A1Q3 reports; others are unhandled (nothing reported).
pub fn status_fn<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let rules = w.rules().clone();
    if let Some(p) = rules.quests() {
        if let Some(s) = p.status(ctl, w, i, player, pf) {
            return s;
        }
    }
    match ctl.records[i].chain {
        0 | 25 | 30 | 37..=39 => None,
        3 => Some(q3::status(ctl, w, i, player, pf)),
        7..=13 | 26 | 27 => super::act2::status_fn(ctl, w, i, player, pf, f),
        14..=18 | 28 => super::act3::status(ctl, w, i, player, pf),
        c if super::act4::owns(c) => super::act4::status(ctl, w, i, player, pf, f),
        c if super::act5::owns(c) => super::act5::status(ctl, w, i, player, pf, f),
        c => {
            w.unhandled(c, f);
            None
        }
    }
}

/// Runs a timer callback; true = remove the timer.
pub fn run_timer<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    func: TimerFn,
    chain: u8,
) -> bool {
    match func {
        TimerFn::DenOfEvilStatus => {
            if let Some(i) = ctl.find(chain) {
                q1::timer(ctl, w, i);
            }
            true
        }
        TimerFn::BurialStatus => {
            if let Some(i) = ctl.find(chain) {
                broadcast(ctl, w, i, 3, 0);
            }
            true
        }
        TimerFn::CainRemoval => {
            if let Some(i) = ctl.find(chain) {
                q4::removal_timer(ctl, w, i);
            }
            true
        }
        TimerFn::TristramPortal => ctl
            .find(chain)
            .is_none_or(|i| q4::tristram_portal_timer(ctl, w, i)),
        TimerFn::TowerStatus => {
            if let Some(i) = ctl.find(chain) {
                q5::timer(ctl, w, i);
            }
            true
        }
        TimerFn::AndarielPortals => ctl.find(chain).is_none_or(|i| q6::portal_timer(ctl, w, i)),
        TimerFn::SlaughterOpen => {
            if let Some(r) = ctl.record_mut(chain) {
                if r.state == 0 {
                    r.state = 1;
                }
            }
            true
        }
        TimerFn::Act2(t) => super::act2::run_timer(ctl, w, t, chain),
        TimerFn::Act3(t) => super::act3::run_timer(ctl, w, t, chain),
        TimerFn::Act4(t) => super::act4::run_timer(ctl, w, t, chain),
        TimerFn::Act5(t) => super::act5::run_timer(ctl, w, t, chain),
        #[cfg(test)]
        TimerFn::Probe => {
            w.unhandled(chain, ctl.tick);
            false
        }
    }
}

/// Offer the respec (set 41.13, 41.1).
pub fn respec_offer<W: QuestWorld>(w: &mut W, player: UnitId) {
    if let Some(f) = flags_of(w, player) {
        f.set(41, bit::PRIMARY_GOAL_DONE);
        f.set(41, bit::REWARD_PENDING);
    }
}

/// After the respec (set 41.0, clear 41.1; if 41.15 is
/// clear, chain 30's record gets active := 0, fatal if absent).
pub fn respec_done<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(f) = flags_of(w, player) else { return };
    f.set(41, bit::REWARD_GRANTED);
    f.clear(41, bit::REWARD_PENDING);
    if !f.get(41, bit::COMPLETED_BEFORE) {
        match ctl.record_mut(30) {
            Some(r) => r.active = false,
            None => ctl.faults.push(QuestError::Fatal(0x0058_FD50)),
        }
    }
}
