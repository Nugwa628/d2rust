// Spec: specs/world/quests-act3.md §6 (A3Q4 The Golden Bird, chain 18); specs/world/quests-act3-2.md §11.3, §11.4
//! A3Q4: events 0, 2, 3, 4, 8, 9, 10, 11, 13, 14, the status and active
//! functions, the boss choice and removal hooks, Alkor's map-AI hooks and
//! the Potion of Life.

use super::{
    add_guid, add_state, guid_listed, in_act3, install, list_remove, npc, npc_of, pf, quick_remove,
    refresh, send_reward_ack, sequence, set, sound, status_all, DOCKS,
};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, send_player_flags, EventArgs, QuestControl, QuestFlags, QuestWorld,
    TextList,
};

const SLOT: u8 = 20;
const CHAIN: u8 = 18;
/// The jade figurine.
pub const FIGURINE: [u8; 4] = *b"j34 ";
/// The golden bird.
pub const BIRD: [u8; 4] = *b"g34 ";
/// The Potion of Life.
pub const POTION: [u8; 4] = *b"xyz ";
/// `0x006CE280` (0x40) on monstats flags byte +0x0D: flag word bit 14,
/// the `flying` column (`quests-act3-2.md` §11.4).
pub const FLYING_0D: u8 = 0x40;
/// Stat 7 (`maxhp`) and the potion's bonus (20 << 8).
const MAXHP: u16 = 7;
const LIFE_BONUS: i32 = 0x1400;

/// Chain 18's extra data (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the bird was brought to Alkor (Alkor's map AI).
    pub bird_brought: bool,
    /// +0x01: a boss may be chosen (1 at init).
    pub may_choose: bool,
    /// +0x02: a boss is chosen.
    pub chosen: bool,
    /// +0x04: its GUID.
    pub boss_guid: u32,
    /// +0x08 / +0x09 / +0x0A / +0x0B: chat end pending for Alkor, Cain's
    /// first talk, Cain's second talk, Meshif.
    pub pend_alkor: bool,
    pub pend_cain1: bool,
    pub pend_cain2: bool,
    pub pend_meshif: bool,
    /// +0x0C: the figurine is still to drop (1 at init).
    pub to_drop: bool,
    /// +0x10: figurines plus birds held in the game.
    pub held: i32,
    /// +0x14: the figurine was dropped.
    pub dropped: bool,
    /// +0x15: the last holder left.
    pub holder_left: bool,
    /// +0x18: the bit for the party iterate.
    pub party_bit: u8,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q4
}

/// Dispatches the chain's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => {
            // `0x005BA0A0`.
            if args.a == DOCKS {
                if let Some(p) = args.player {
                    quick_remove(ctl, w, i, p);
                }
            }
        }
        event::ITEM_PICKED_UP => picked_up(ctl, w, i, args),
        event::MONSTER_KILLED => killed(ctl, w, i, args),
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x005BA9A0`.
            let r = &ctl.records[i];
            let (not_intro, state) = (r.not_intro, r.state);
            let e = x(ctl, i);
            e.held -= 1;
            if e.held == 0 && not_intro && e.dropped && state < 4 {
                e.holder_left = true;
                status_all(ctl, w, i, 6);
            }
        }
        // `0x005BA990`.
        event::PLAYER_LEAVES_GAME => list_remove(ctl, w, i, &args),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x005BA9E0`.
            let Some(p) = args.player else { return true };
            let (j, g) = (w.has_item(p, FIGURINE), w.has_item(p, BIRD));
            let not_intro = ctl.records[i].not_intro;
            let e = x(ctl, i);
            e.held += i32::from(j) + i32::from(g);
            if e.held == 1 && not_intro && e.holder_left {
                e.holder_left = false;
                status_all(ctl, w, i, if g { 3 } else { 1 });
            }
        }
        _ => return false,
    }
    true
}

/// Event 0 `0x005B9ED0` (§6.4; edge case 8: table state 4 is never
/// selected).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let class = npc_of(w, &args);
    let k = if f.get(SLOT, bit::REWARD_GRANTED) {
        guid_listed(ctl, w, i, p).then_some(6)
    } else if class == Some(npc::ALKOR) && ctl.records[i].extra.act3.q4.bird_brought {
        None
    } else if f.get(SLOT, bit::REWARD_PENDING) {
        Some(5)
    } else if w.has_item(p, BIRD) {
        // Alkor's 2 needs 20.0 and 20.1 clear, which hold here.
        if class == Some(npc::CAIN3) && !f.get(SLOT, 4) {
            Some(3)
        } else {
            Some(2)
        }
    } else if w.has_item(p, FIGURINE) {
        if matches!(class, Some(npc::CAIN3 | npc::ASHEARA)) {
            Some(if f.get(SLOT, bit::STARTED) { 7 } else { 0 })
        } else {
            Some(1)
        }
    } else {
        None
    };
    if let Some(k) = k {
        add_state(ctl, w, i, list, args.target, k);
    }
}

/// Active function `0x005B9D90` ("wants to talk", §6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let _ = (ctl, i);
    let f = pf(w, player);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        return false;
    }
    let (j, g) = (w.has_item(player, FIGURINE), w.has_item(player, BIRD));
    match npc_class {
        npc::ALKOR => g,
        npc::MESHIF2 => j,
        npc::CAIN3 => (j && !f.get(SLOT, bit::STARTED)) || (g && !f.get(SLOT, 4)),
        _ => false,
    }
}

/// The party iterate `0x005BA290`: members lacking 20.0 and 20.1 get
/// the bit +0x18.
fn party_iterate<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId, b: u8) {
    x(ctl, i).party_bit = b;
    for m in w.party_members(p).unwrap_or_default() {
        let f = pf(w, m);
        if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
            set(w, m, SLOT, &[b]);
        }
    }
}

/// Event 11 `0x005BA320` (§6.5).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    let is = |c: u16| args.a == u32::from(c);
    match args.b {
        527 if is(npc::CAIN3) => {
            set(w, p, SLOT, &[bit::STARTED]);
            if ctl.records[i].state == 1 {
                ctl.records[i].state = 2;
                x(ctl, i).pend_cain1 = true;
                install(ctl, i, event::NPC_DEACTIVATE);
            }
            refresh(ctl, w, p, &args);
            party_iterate(ctl, w, i, p, bit::STARTED);
        }
        531 if is(npc::CAIN3) => {
            set(w, p, SLOT, &[4]);
            x(ctl, i).pend_cain2 = true;
            refresh(ctl, w, p, &args);
            party_iterate(ctl, w, i, p, 4);
        }
        529 if is(npc::MESHIF2) => {
            if w.has_item(p, FIGURINE) {
                install(ctl, i, event::NPC_DEACTIVATE);
                w.delete_item(p, FIGURINE);
                x(ctl, i).held -= 1;
                if w.reward_item(p, BIRD, 0, 2, true).is_some() {
                    x(ctl, i).held += 1;
                    ctl.records[i].state = 3;
                    x(ctl, i).pend_meshif = true;
                }
            }
        }
        534 if is(npc::ALKOR) => {
            if w.has_item(p, BIRD) && !f.get(SLOT, bit::REWARD_PENDING) {
                w.delete_item(p, BIRD);
                x(ctl, i).held -= 1;
                if ctl.records[i].not_intro && ctl.records[i].state != 4 {
                    ctl.records[i].state = 4;
                }
                set(w, p, SLOT, &[bit::REWARD_PENDING]);
                x(ctl, i).pend_alkor = true;
                install(ctl, i, event::NPC_DEACTIVATE);
                party_iterate(ctl, w, i, p, bit::REWARD_PENDING);
            }
        }
        538 if is(npc::ALKOR) => {
            if f.get(SLOT, bit::REWARD_PENDING) {
                if let Some(r) = flags_of(w, p) {
                    r.clear(SLOT, bit::REWARD_PENDING);
                    r.set(SLOT, bit::REWARD_GRANTED);
                    r.set(SLOT, bit::PRIMARY_GOAL_DONE);
                    r.reset_progress(SLOT);
                    r.set(SLOT, 5);
                }
                if ctl.records[i].not_intro {
                    ctl.records[i].state = 5;
                }
                w.reward_item(p, POTION, 0, 2, true);
                send_player_flags(w, p, 6, 0);
                add_guid(ctl, w, i, p);
            }
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                sequence(ctl, w, CHAIN);
            }
        }
        _ => {}
    }
}

/// Event 2 `0x005BA0B0` (never cleared, edge case 6).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    match npc_of(w, &args) {
        Some(npc::ALKOR) if x(ctl, i).pend_alkor => {
            status_all(ctl, w, i, 5);
            let e = x(ctl, i);
            e.pend_alkor = false;
            e.bird_brought = true;
        }
        Some(npc::CAIN3) => {
            if x(ctl, i).pend_cain1 {
                status_all(ctl, w, i, 2);
                x(ctl, i).pend_cain1 = false;
            }
            if x(ctl, i).pend_cain2 {
                status_all(ctl, w, i, 4);
                x(ctl, i).pend_cain2 = false;
            }
        }
        Some(npc::MESHIF2) if x(ctl, i).pend_meshif => {
            status_all(ctl, w, i, 3);
            x(ctl, i).pend_meshif = false;
        }
        _ => {}
    }
}

/// Event 4 `0x005BA6A0` (§6.7).
fn picked_up<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        return;
    }
    match args.target.and_then(|t| w.item_code(t)) {
        Some(BIRD) => {
            ctl.records[i].state = 3;
            status_all(ctl, w, i, 3);
        }
        Some(FIGURINE) => {
            status_all(ctl, w, i, 1);
            ctl.records[i].state = 1;
            if !f.get(SLOT, 6) {
                set(w, p, SLOT, &[6]);
                w.attach_sound(p, sound::FIGURINE);
            }
        }
        _ => {}
    }
}

/// Event 8 `0x005BAB60` (§6.3; installed by the boss choice). The
/// figurine's item level is the victim's stat 12 (`quests-act3-2.md`
/// §11.3).
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let e = &ctl.records[i].extra.act3.q4;
    if !ctl.records[i].not_intro || !e.chosen || !e.to_drop {
        return;
    }
    if let Some(p) = args.player {
        if pf(w, p).get(SLOT, bit::REWARD_GRANTED) {
            return;
        }
    }
    let Some(victim) = args.target else { return };
    if w.quest_drop(victim, FIGURINE, 2, None, false).is_some() {
        ctl.records[i].clear_callback(event::MONSTER_KILLED);
        let e = x(ctl, i);
        e.held += 1;
        e.to_drop = false;
        e.dropped = true;
        if ctl.records[i].state == 0 {
            ctl.records[i].state = 1;
        }
        if ctl.records[i].status == 0 {
            status_all(ctl, w, i, 1);
        }
    } else {
        x(ctl, i).may_choose = true;
    }
    x(ctl, i).chosen = false;
}

/// Event 13 `0x005BA870` (§6.8). The three tests run in turn (no
/// "stop" in the spec).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        ctl.records[i].not_intro = false;
        let e = x(ctl, i);
        e.to_drop = false;
        e.may_choose = false;
    }
    if w.has_item(p, BIRD) {
        let e = x(ctl, i);
        e.may_choose = false;
        e.to_drop = false;
        let r = &mut ctl.records[i];
        r.state = 3;
        r.status = if f.get(SLOT, 4) { 4 } else { 3 };
    }
    if w.has_item(p, FIGURINE) {
        let e = x(ctl, i);
        e.may_choose = false;
        e.to_drop = false;
        let r = &mut ctl.records[i];
        (r.state, r.status) = if f.get(SLOT, bit::STARTED) {
            (2, 2)
        } else {
            (1, 1)
        };
    }
}

/// Status function `0x005BA170` (always reports, §6.9).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    r: &QuestFlags,
) -> u8 {
    let rec = &ctl.records[i];
    if !r.get(15, bit::REWARD_GRANTED) {
        0
    } else if r.get(SLOT, bit::REWARD_PENDING) {
        5
    } else if r.get(SLOT, bit::REWARD_GRANTED) {
        11 + 2 * u8::from(r.get(SLOT, bit::PRIMARY_GOAL_DONE))
    } else if w.has_item(player, BIRD) {
        3 + u8::from(r.get(SLOT, 4))
    } else if w.has_item(player, FIGURINE) {
        1 + u8::from(r.get(SLOT, bit::STARTED))
    } else if rec.not_intro {
        if rec.status > 5 {
            6 + u8::from(w.game_type() != 3)
        } else {
            rec.status
        }
    } else {
        0
    }
}

/// `0x00544E80` → `0x005BAC70` (§6.2): from special monster creation
/// for a monster in Act III; `flags_0d` is its monstats flags byte
/// +0x0D, `None` when the class has no monstats row (class ≥ count).
/// Flying monsters (`flying`, [`FLYING_0D`]) never carry the Golden Bird
/// (`quests-act3-2.md` §11.4).
pub fn choose_bird_boss<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    unit: UnitId,
    class: u16,
    flags_0d: Option<u8>,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !in_act3(w, unit) || !ctl.records[i].not_intro || class == npc::FETISH11 {
        return;
    }
    // A class with no monstats row fails the whole test (`0x00544ED3`):
    // nothing is chosen or linked.
    let Some(flags_0d) = flags_0d else {
        return;
    };
    if flags_0d & FLYING_0D != 0 {
        return;
    }
    let e = &ctl.records[i].extra.act3.q4;
    // No Gidbinn boss being spawned (chain 17's +0x04).
    let gidbinn_spawning = ctl
        .record(17)
        .is_some_and(|r| r.extra.act3.q3.boss_spawning);
    if !(e.to_drop && e.may_choose) || gidbinn_spawning {
        return;
    }
    ctl.add_link(w, unit, CHAIN, None);
    install(ctl, i, event::MONSTER_KILLED);
    let g = w.guid(unit);
    let e = x(ctl, i);
    e.may_choose = false;
    e.chosen = true;
    e.boss_guid = g;
}

/// `0x005BACF0` (§6.2): a monster linked to chain 18 is removed.
pub fn bird_boss_removed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(unit);
    let e = x(ctl, i);
    if e.chosen && e.boss_guid == g {
        e.chosen = false;
        e.may_choose = true;
    }
}

/// `0x005BAD20` (Alkor's map AI): the +0x00 test.
pub fn alkor_bird_brought(ctl: &QuestControl) -> bool {
    ctl.record(CHAIN)
        .is_some_and(|r| r.extra.act3.q4.bird_brought)
}

/// `0x005BAD40` (Alkor's map AI): clear +0x00.
pub fn alkor_bird_clear(ctl: &mut QuestControl) {
    if let Some(r) = ctl.record_mut(CHAIN) {
        r.extra.act3.q4.bird_brought = false;
    }
}

/// Using `xyz ` (`0x0055E170`, §6.6): true when used (consume it).
pub fn potion_of_life<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) -> bool {
    let _ = ctl;
    // `0x0055CC90`: usable only with 20.5.
    let Some(f) = flags_of(w, player) else {
        return false;
    };
    if !f.get(SLOT, 5) {
        return false;
    }
    f.clear(SLOT, 5);
    w.add_stat(player, MAXHP, LIFE_BONUS);
    send_reward_ack(w, player, CHAIN);
    true
}
