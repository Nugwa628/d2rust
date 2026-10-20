//! The client player's animation and its mode end: the player part of
//! the client update that returns a player to neutral when
//! its attack, cast or hit animation is over.
//!
//! A player mode set restarts the frame at
//! the frame bonus, with the frame
//! count of the mode's AnimData record (frames · 256) and the rate;
//! the same mode again restarts nothing. Each
//! client update then runs, for a player in an ending mode
//! ([`ENDING_MODES`]): advance the frame unless the animation is
//! complete (frame + speed ≥ count), then, when it is
//! complete, the neutral end (mode 1, 5 in town). The
//! server sends a player's own client nothing when it returns to
//! neutral (the NU row skips the own client),
//! so without this step the local player stayed in the attack mode and
//! the click gate (`bridge::click::can_act`, modes 7–12 refused) dropped
//! every later click.
//!
//! The rows of table for the modes in [`ENDING_MODES`]
//! (read from the base-game image): advance 1
//! and end 2 (4, 9, 13) or 3 (7, 8, 10–12, 14–16), so a player ends the
//! mode on the first update whose advanced frame + speed reaches the
//! count (the end test of 2 and 3 is the same "complete"). Frost Nova
//! (mode 10) matches the client state 70/70.
//! PROVISIONAL (the timing part of the other modes): settled by the
//! local player's mode, frame, count and speed per client update over a
//! Multiple Shot.
//! The rate reads the used skill, the item rate stats and the dual-wield
//! hands ([`rate`]; measured with Frenzy, Double Swing and Double Throw).
//! PROVISIONAL: a player without a base
//! stat 67–69 reads 100, as the audio feed's `player_rate`; the frame
//! bonus reads no dual wield; the were-form speed is not read. Oracle: a
//! Frost Nova with item rate stat 105; the dual-wield and were-form
//! parts need a Double Swing match (dual wield, measured) and a
//! were-form case (none yet).

use d2_sim::units::anim_rate::{anim_rate, frame_bonus, mode_row, Rate, RateInput};

use super::attack_weapon;
use super::disguise::Identity;
use super::dispatch::HandlerError;
use super::world::{ClientWorld, ModelInputs, UnitKey, MONSTER, PLAYER};

/// The modes the client update ends in neutral: GH 4, A1 7,
/// A2 8, BL 9, SC 10, TH 11, KK 12, S1–S4 13–16 (the modes the server's
/// event 1 ends in neutral).
pub const ENDING_MODES: [u32; 11] = [4, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// A player's animation in one mode, as the host's tables give it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerAnim {
    /// AnimData frames of the mode's COF name.
    pub frames: u32,
    /// AnimData speed (+0x0C).
    pub speed: i32,
    /// The COF weapon class as its
    /// `sequences::CLASSES` index (0 `hth`).
    pub weapon: i32,
    /// The record's event bytes (+0x10 + i),
    /// read by the frame advance for +0x4E.
    pub events: Option<[u8; d2_formats::animdata::EVENTS]>,
}

/// The animation lookup of the client player update: the AnimData
/// record of a model player's COF name in `mode`, its weapon class from
/// the model's worn items. `None`: no record (the mode ends on the next
/// update).
pub trait PlayerAnims: std::fmt::Debug + Send + Sync {
    fn anim(&self, w: &ClientWorld, key: UnitKey, mode: u32) -> Option<PlayerAnim>;

    /// The AnimData frames and speed of a player `class`'s attack 1 (mode
    /// 7) with the COF weapon class `weapon` (`hth `, `1hs `, ...), the
    /// query of the tool tip's speed line.
    /// `None`: no record.
    fn attack1(&self, class: u32, weapon: [u8; 4]) -> Option<(i32, i32)> {
        let _ = (class, weapon);
        None
    }

    /// The COF weapon class of a model player (its `sequences::CLASSES`
    /// index), the record index of the mode-18 sequence load,
    /// whether or not the mode has an
    /// AnimData record. `None`: not known.
    fn weapon(&self, w: &ClientWorld, key: UnitKey) -> Option<i32> {
        self.anim(w, key, 1).map(|a| a.weapon)
    }
}

/// The animation lookup the model carries for the tool tips. Compares
/// equal always: a handle, not model state.
#[derive(Clone, Default)]
pub struct PlayerAnimsRef(pub Option<std::sync::Arc<dyn PlayerAnims>>);

impl std::fmt::Debug for PlayerAnimsRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlayerAnimsRef({})", self.0.is_some())
    }
}

impl PartialEq for PlayerAnimsRef {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for PlayerAnimsRef {}

/// The type class of a COF weapon class index
/// (`bow` 1 … `xbw` 7, `ht1` 12,
/// otherwise 0).
pub(crate) fn type_class(weapon: i32) -> u32 {
    match weapon {
        c @ (1..=7 | 12) => c as u32,
        _ => 0,
    }
}

/// The used skill's `skills` row: the current entry's.
pub(super) fn used_row<'a>(
    w: &ClientWorld,
    inputs: &'a ModelInputs,
    key: UnitKey,
) -> Option<&'a d2_data::tables::Skills> {
    let l = w.units.get(&key)?.skills.as_ref()?;
    let skill = l.entries.get(l.current?)?.skill;
    inputs
        .skill_tables
        .as_deref()?
        .skills
        .get(usize::from(skill))
}

/// The state Holy Shield (101), the rate's step 5 test.
const HOLY_SHIELD: u8 = 101;

/// The rate of a player drawn as
/// `id` (type, class, mode; [`super::disguise::identity`]) with AnimData
/// speed `s` (steps 3–5 and 8–10): the walk and run modes read
/// the path step `w`, 213 / 101, or drawn as a monster the class's run
/// speed in RN (15), else its walk speed; the used skill's `seqtrans` and
/// `UseAttackRate`, the totals of stats 67–69 (a player without a base
/// value reads 100) and of the item rate stats 93, 99, 105, 102, 96, and
/// for a dual-capable unit the dual-wield stat toggle (step 8.1,
/// [`attack_weapon::dual_wield_stat_toggle`], which may switch a hand's
/// list off) and the two hands' attack rates (step 8.3).
/// PROVISIONAL: the were-form speed of step 8.6 is not read
/// (no client were-form model); settled by the were-form test and a
/// were-form case (none).
fn rate(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    id: Identity,
    s: i32,
) -> (i32, Option<i32>) {
    let (t, c, mode) = id;
    if !w.units.contains_key(&key) {
        return (s, None);
    }
    let used =
        used_row(w, inputs, key).map(|r| (i32::from(r.seqtrans as i8), r.useattackrate, r.weapsel));
    let row = mode_row(t, c, mode);
    let dual_capable = attack_weapon::dual_capable(w, key);
    if dual_capable && (row.a || (row.a_skill && used.is_some_and(|u| u.1))) {
        attack_weapon::dual_wield_stat_toggle(w, key, used.map(|u| u.2));
    }
    let Some(u) = w.units.get(&key) else {
        return (s, None);
    };
    let monster = (t == MONSTER)
        .then(|| {
            inputs
                .tables
                .monsters
                .get(c as usize)
                .and_then(|m| m.as_ref())
        })
        .flatten();
    let total = |stat: u16| {
        let base = if u.stats.contains_key(&stat) { 0 } else { 100 };
        base + w.total(key, stat, 0)
    };
    let item = |stat: u16| w.total(key, stat, 0);
    let dual = dual_capable
        .then(|| {
            let a = attack_weapon::usable_weapon_at(w, key, 4)?;
            let b = attack_weapon::usable_weapon_at(w, key, 5)?;
            Some((
                attack_weapon::item_total(w, a.key, 68),
                attack_weapon::item_total(w, b.key, 68),
            ))
        })
        .flatten();
    let i = RateInput {
        applies: true,
        t,
        c,
        m: mode,
        s,
        item: [item(93), item(99), item(105), item(102), item(96)],
        velocitypercent: total(67),
        attackrate: total(68),
        other_animrate: total(69),
        used_seqtrans: used.map(|u| u.0),
        use_attack_rate: used.is_some_and(|u| u.1),
        holyshield: u.states.contains(&HOLY_SHIELD),
        has_path: true,
        w: match monster {
            Some(m) => i32::from(if mode == 15 {
                m.run_speed
            } else {
                m.walk_speed
            }),
            None if mode == 3 => 101,
            None => 213,
        },
        // The mode table's velocity flag, or its skill-velocity
        // flag with the used entry a moving skill under way (entry flags bit
        // 0, not 0x1000): Whirlwind in SQ until its path ends (the
        // state-off's refresh writes 304 there).
        velocity_mode: row.v
            || (row.v_skill
                && super::skills::used_entry(w, key)
                    .is_some_and(|e| e.flags & 1 != 0 && e.flags & 0x1000 == 0)),
        dual,
        player_mode_18: u.mode == 18,
        ..RateInput::default()
    };
    match anim_rate(&i) {
        Ok(Rate::Set {
            speed, seq_speed, ..
        }) => (speed, seq_speed),
        _ => (s, None),
    }
}

/// Writes a rate: the speed (+0x4C) and, in the cast and attack cases,
/// the sequence step (+0x3C) of a loaded sequence.
fn set_rate(w: &mut ClientWorld, key: UnitKey, (speed, seq_speed): (i32, Option<i32>)) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.speed = Some(speed);
    if let (Some(v), Some(q)) = (seq_speed, u.sequence.as_mut()) {
        q.step = v;
    }
}

/// The animation-rate refresh with (U) outside a mode set (a
/// stat with `updateanimrate`, a state off, a TEMPONLY list made): a
/// model player's speed := the rate of its current mode with
/// that mode's AnimData speed; frame and count stay. Others: nothing.
pub fn refresh_rate(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    if key.unit_type != PLAYER {
        return;
    }
    let Some(mode) = w.units.get(&key).map(|u| u.mode) else {
        return;
    };
    let id = super::disguise::identity(w, inputs, key, mode);
    let anim = anim_of(w, inputs, key, mode, id);
    let r = rate(w, inputs, key, id, anim.speed);
    set_rate(w, key, r);
}

/// The animation record of a player in `mode` drawn as `id`
/// ([`super::disguise::identity`]): shown as a monster, that class's
/// record in the mapped mode; else the player's own.
fn anim_of(
    w: &ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    mode: u32,
    id: Identity,
) -> PlayerAnim {
    if id.0 == MONSTER {
        let row = inputs
            .tables
            .monsters
            .get(id.1 as usize)
            .and_then(|c| c.as_ref());
        let rec = row.and_then(|c| c.anims.get(id.2 as usize).copied().flatten());
        PlayerAnim {
            frames: rec.map_or(0, |(f, _)| f),
            speed: rec.map_or(0, |(_, s)| s as i32),
            weapon: 0,
            events: row.and_then(|c| c.anim_events.get(id.2 as usize).copied().flatten()),
        }
    } else {
        inputs
            .player_anims
            .as_ref()
            .and_then(|a| a.anim(w, key, mode))
            .unwrap_or_default()
    }
}

/// The movement kind of each player mode (table, entry +0
/// of 12-byte rows; read from the base-game image): 1 path step, 2 skill
/// (the do of [`step`]), 0 neither.
const MOVE_KIND: [u8; 20] = [0, 0, 1, 1, 0, 0, 1, 2, 2, 0, 2, 2, 2, 0, 2, 2, 2, 0, 2, 1];

/// The client player mode set: flag 0x40
/// is cleared; a new mode is written and
/// its animation restarted (frame := frame bonus · 256, count := frames
/// · 256, speed := the rate, +0x4E := 0); the same mode changes nothing
/// else. A unit not in the model or not a player: nothing.
pub fn mode_set(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    if key.unit_type != PLAYER {
        return;
    }
    u.flag_40 = false;
    if u.mode == mode {
        return;
    }
    mode_init(w, inputs, key, mode);
    // A new mode: the hit overlays of the mode set.
    super::modes::hit_overlays(w, inputs, key, mode);
}

/// The animation part of the mode set unconditionally (player creation:
/// the allocation stored the mode already, then the init's set-up ran;
/// the base-game client state of a new player shows the mode's count and
/// rate).
pub fn mode_init(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) {
    let Some(u) = w.units.get(&key) else {
        return;
    };
    let _ = u;
    // The mode is written, then frees the TEMPONLY lists,
    // then the animation set-up reads the rate.
    w.units
        .get_mut(&key)
        .expect("checked above")
        .temp_lists
        .clear();
    // The draw identity:
    // a player shown as a monster takes that class's record in the mapped
    // mode, and the rate and frame bonus read it.
    let id = super::disguise::identity(w, inputs, key, mode);
    let anim = anim_of(w, inputs, key, mode, id);
    let bonus = frame_bonus(id.0, id.1, id.2, false, Some(type_class(anim.weapon)));
    let u = w.units.get_mut(&key).expect("checked above");
    u.mode = mode;
    u.frame = bonus * 256;
    u.frame_count = (anim.frames as i32).wrapping_mul(256);
    u.action_frame = 0;
    u.anim_events = anim.events;
    u.sequence = None;
    // Mode 18 with a frame list for the used skill: the sequence load:
    // the count is the list's,
    // the frame and +0x4E those of its frame 0.
    if mode == 18 {
        if let Some((frames, hth_len)) = sequence_of(w, inputs, key) {
            let u = w.units.get_mut(&key).expect("checked above");
            super::monster_anim::sequence_load(u, frames);
            // The length and count getters of the load
            // read the class-0 (`hth`) record of the
            // sequence while the list getter reads the weapon class's
            // (measured: Jab with a javelin plays the `1ht` list for
            // the `hth` count 13, not 18).
            if let (Some(n), Some(seq)) = (hth_len, u.sequence.as_mut()) {
                seq.length = n << 8;
                u.frame_count = n << 8;
            }
        }
    }
    // The rate last, as the mode set runs it after the sequence load
    // (the attack weapon reads the sequence position).
    let r = rate(w, inputs, key, id, anim.speed);
    set_rate(w, key, r);
}

/// The frame list of a player's used skill (step 2): the `skills` row's `seqnum` (+0x13) by the COF weapon class.
/// With it, the frame count of the sequence's class-0 (`hth`) record,
/// which the load takes for its length and count (`None`: no such
/// record).
#[allow(clippy::type_complexity)]
fn sequence_of(
    w: &ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Option<(&'static [d2_sim::skills::sequences::SeqFrame], Option<i32>)> {
    use d2_sim::skills::sequences::lookup;
    let l = w.units.get(&key)?.skills.as_ref()?;
    let skill = l.entries.get(l.current?)?.skill;
    let row = inputs.tables.skills.get(usize::from(skill))?;
    let class = inputs.player_anims.as_ref()?.weapon(w, key)?;
    let frames = lookup(row.seqnum, usize::try_from(class).ok()?)?;
    let hth = lookup(row.seqnum, 0).and_then(|f| i32::try_from(f.len()).ok());
    Some((frames, hth))
}

/// The player part of the client update
/// for a player in an ending mode: in a skill mode (kind 2) with
/// a used skill, flag 0x40 clear and +0x4E ∈ {1, 2, 3} (the previous
/// advance's byte) the client do (`client_do::generic_do`); then advance
/// unless complete (+0x4E := the last event byte
/// crossed); then, when complete, the mode end: for a player
/// other than the local player with flag 0x40 still clear the do, used
/// skill := none, mode set neutral.
///
/// With E-flag bit 0 on the used skill (a moving skill:
/// Charge's start sets it) the path step runs first: the step
/// ending the path ([`ClientUnit::path_live`] false) sets E-flag bit 1
/// and runs the do; the mode end's skill end does the same for bit 0.
///
/// [`ClientUnit::path_live`]: super::world::ClientUnit::path_live
pub fn step(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) -> Result<(), HandlerError> {
    let (rate, frame) = (w.tick_rate, w.share_frame());
    let complete = |u: &super::world::ClientUnit| super::monster_anim::complete_at(u, rate, frame);
    let advance =
        |u: &mut super::world::ClientUnit| super::monster_anim::advance_at(u, rate, frame);
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    if key.unit_type != PLAYER {
        return Ok(());
    }
    // The looping neutral modes (1 and 5): each client update adds the
    // speed to the frame and a frame at the count wraps to the overshoot
    // (measured on the base game: the neutral mode set by
    // the 0x15 placement of tick 4 reads 128, 256, ... 1920, 0, 128 at
    // ticks 4 ... 20 with speed 128 and count 2048). Only after a mode set
    // (`speed` known).
    // The walk and run modes (2, 3, 6) advance the same way (anim kind
    // 0); measured: speed 213 a client update.
    if matches!(u.mode, 1 | 2 | 3 | 5 | 6) {
        if let (Some(s), true) = (u.speed, u.frame_count > 0) {
            let u = w.units.get_mut(&key).expect("present");
            u.frame = u.frame.wrapping_add(rate.share(s, frame));
            if u.frame >= u.frame_count {
                u.frame -= u.frame_count;
            }
        }
        return Ok(());
    }
    // DT (0) and DD (17): path kind 0, anim kind 2, end kind 0:
    // complete → hold the last
    // frame (+0x44 := +0x48 − 0x100, speed 0), else advance; never ended
    // (the server's codes 0x09 / 0x07 change the mode). Measured on the
    // base game: DT reads 0, 256, ... 5888 with speed
    // 256 and count 6144, then DD 0 with speed 256 and speed 0 from the
    // next update.
    if matches!(u.mode, 0 | 17) {
        if u.speed.is_some() && u.frame_count > 0 {
            let u = w.units.get_mut(&key).expect("present");
            if complete(u) {
                u.frame = u.frame_count - 0x100;
                u.speed = Some(0);
            } else {
                advance(u);
            }
        }
        return Ok(());
    }
    // Mode 19 (KB, row {path 1, anim 3, end 1}):
    // the frame held at 0x100 with speed 0; ended when the path
    // step ends (A), read from the server's path words of this tick (the
    // single-player client steps its copy of the server's path:
    // `critter_path::player_step_result`); then the KB end: used
    // skill := none, mode set 4 (GH), which ends. In the base game: KB
    // at f77 (fr 0), f78 (fr 256), GH from f79 (fr 0, 128, ...).
    if u.mode == super::modes::player_mode::SEQUENCE {
        let ended = u.path_live == Some(false);
        let u = w.units.get_mut(&key).expect("present");
        u.frame = 0x100;
        u.speed = Some(0);
        if ended {
            u.path_live = None;
            if let Some(l) = u.skills.as_mut() {
                l.current = None;
            }
            mode_set(w, inputs, key, super::modes::player_mode::GET_HIT);
        }
        return Ok(());
    }
    // Mode 18 runs the machine of its row (kind 2, advance 1, end 3;
    // "Client mode-18 machine"), on its sequence
    // when the load found one, else on the AnimData animation.
    if !ENDING_MODES.contains(&u.mode) && u.mode != 18 {
        return Ok(());
    }
    let has_skill = u.skills.as_ref().is_some_and(|l| l.current.is_some());
    let kind = MOVE_KIND.get(u.mode as usize).copied().unwrap_or(0);
    // A do that cannot model its branch (Charge's search pick, Leap Attack's
    // landing face) reports the gap but must not stall the animation: the
    // frame advance and the mode end still run, and the error is reported
    // after them so the client returns to neutral.
    let mut do_error = None;
    // A used skill with E-flag bit 0 steps the path; a step that
    // ends it sets E-flag bit 1 and runs the do, and no second do follows.
    let mut ended = false;
    if kind == 2 && entry_flags(w, key).is_some_and(|f| f & 1 != 0) {
        let live = w.units.get(&key).and_then(|u| u.path_live);
        if live == Some(false) {
            set_entry_flags(w, key, |f| f | 2);
            if let Err(e) = super::client_do::generic_do(w, inputs, key) {
                do_error = Some(e);
            }
            ended = true;
        }
    }
    let u = w.units.get(&key).expect("present");
    if !ended && kind == 2 && has_skill && !u.flag_40 && matches!(u.action_frame, 1..=3) {
        if let Err(e) = super::client_do::generic_do(w, inputs, key) {
            do_error = Some(e);
        }
    }
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    // Not complete: the frame advance, its
    // sequence branch with a sequence. Without one the frame does not
    // reach the count, so the advance never wraps and the bonus is not
    // read.
    if !complete(u) {
        advance(u);
    }
    // The client motion record's update: a Leap Attack's arc.
    if let Some(a) = u.leap_arc.as_mut() {
        let _ = a.update_at(false, None, rate, frame);
    }
    if complete(u) {
        // The skill end: E-flag bit 0 → bit 1 and the do;
        // else a player other than the local one with flag 0x40 clear →
        // the do.
        let remote = w.local_player != Some(key);
        let u = w.units.get(&key).expect("present");
        if entry_flags(w, key).is_some_and(|f| f & 1 != 0) {
            set_entry_flags(w, key, |f| f | 2);
            if let Err(e) = super::client_do::generic_do(w, inputs, key) {
                do_error = Some(e);
            }
        } else if remote && !u.flag_40 && has_skill {
            if let Err(e) = super::client_do::generic_do(w, inputs, key) {
                do_error = Some(e);
            }
        }
        if let Some(l) = w.units.get_mut(&key).and_then(|u| u.skills.as_mut()) {
            l.current = None;
        }
        let (neutral, _) = super::modes::neutral_walk(w, key);
        mode_set(w, inputs, key, neutral);
    }
    if let Some(e) = do_error {
        return Err(e);
    }
    Ok(())
}

/// The used skill entry's flags.
fn entry_flags(w: &ClientWorld, key: UnitKey) -> Option<u32> {
    let l = w.units.get(&key)?.skills.as_ref()?;
    Some(l.entries.get(l.current?)?.flags)
}

fn set_entry_flags(w: &mut ClientWorld, key: UnitKey, f: impl FnOnce(u32) -> u32) {
    if let Some(l) = w.units.get_mut(&key).and_then(|u| u.skills.as_mut()) {
        if let Some(e) = l.current.and_then(|i| l.entries.get_mut(i)) {
            e.flags = f(e.flags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::ClientUnit;
    use std::sync::Arc;

    /// Synthetic lookup: every mode 10 frames at speed 256, weapon `hth`.
    #[derive(Debug)]
    struct Ten;
    impl PlayerAnims for Ten {
        fn anim(&self, _: &ClientWorld, _: UnitKey, _: u32) -> Option<PlayerAnim> {
            Some(PlayerAnim {
                frames: 10,
                speed: 256,
                weapon: 0,
                events: None,
            })
        }
    }

    fn world() -> (ClientWorld, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(key);
        u.class = 1;
        u.mode = 1;
        w.units.insert(key, u);
        w.local_player = Some(key);
        (w, key)
    }

    // The client mode-18 machine's end on an AnimData mode.
    #[test]
    fn a_cast_mode_ends_in_neutral_when_its_animation_completes() {
        let (mut w, key) = world();
        let inputs = ModelInputs {
            player_anims: Some(Arc::new(Ten)),
            ..ModelInputs::default()
        };
        mode_set(&mut w, &inputs, key, 10);
        assert_eq!((w.units[&key].frame, w.units[&key].frame_count), (0, 2560));
        // Updates 1–8 advance to 2048; frame + speed reaches 2560 after
        // update 9's advance (2304 + 256): neutral on update 9.
        for n in 1..=8 {
            step(&mut w, &inputs, key).unwrap();
            assert_eq!(w.units[&key].mode, 10, "update {n}");
        }
        step(&mut w, &inputs, key).unwrap();
        assert_eq!(w.units[&key].mode, 1);
        // Neutral is not an ending mode: nothing more.
        step(&mut w, &inputs, key).unwrap();
        assert_eq!(w.units[&key].mode, 1);
    }

    // DT and DD: anim kind 2 holds the last frame at speed 0, end kind 0
    // never ends the mode.
    #[test]
    fn death_advances_then_holds_its_last_frame() {
        let (mut w, key) = world();
        let inputs = ModelInputs {
            player_anims: Some(Arc::new(Ten)),
            ..ModelInputs::default()
        };
        for mode in [0, 17] {
            mode_set(&mut w, &inputs, key, mode);
            for n in 1..=9 {
                step(&mut w, &inputs, key).unwrap();
                assert_eq!(w.units[&key].frame, 256 * n, "mode {mode} update {n}");
            }
            // 2304 + 256 reaches 2560: hold 2304 at speed 0, mode kept.
            for _ in 0..3 {
                step(&mut w, &inputs, key).unwrap();
                let u = &w.units[&key];
                assert_eq!((u.mode, u.frame, u.speed), (mode, 2304, Some(0)));
            }
        }
    }

    #[test]
    fn the_same_mode_does_not_restart_and_no_record_ends_at_once() {
        let (mut w, key) = world();
        let inputs = ModelInputs {
            player_anims: Some(Arc::new(Ten)),
            ..ModelInputs::default()
        };
        mode_set(&mut w, &inputs, key, 7);
        step(&mut w, &inputs, key).unwrap();
        let f = w.units[&key].frame;
        mode_set(&mut w, &inputs, key, 7);
        assert_eq!(w.units[&key].frame, f);
        // Without a lookup the count is 0: complete on the first update.
        let (mut w, key) = world();
        let bare = ModelInputs::default();
        mode_set(&mut w, &bare, key, 7);
        step(&mut w, &bare, key).unwrap();
        assert_eq!(w.units[&key].mode, 1);
    }

    /// A barbarian using skill `skill` of a 160-row table whose row has
    /// `seqtrans` 7 (A1) and `UseAttackRate`.
    fn swinging(skill: u16) -> (ClientWorld, ModelInputs, UnitKey) {
        use crate::bridge::skills::{SkillEntry, SkillList};
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let (mut w, key) = world();
        let u = w.units.get_mut(&key).unwrap();
        u.class = 4;
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill,
                ..SkillEntry::default()
            }],
            current: Some(0),
            ..SkillList::default()
        });
        let mut row: Skills = blank();
        row.seqtrans = 7;
        row.useattackrate = true;
        row.weapsel = 3;
        let mut skills = vec![blank::<Skills>(); 160];
        skills[usize::from(skill)] = row;
        let inputs = ModelInputs {
            player_anims: Some(Arc::new(Ten)),
            skill_tables: Some(Arc::new(d2_sim::skills::SkillTables {
                skills,
                skilldesc: vec![blank::<Skilldesc>()],
                missiles: vec![blank::<Missiles>()],
                skills_code: Vec::new(),
                miss_code: Vec::new(),
                level_cap: 99,
                stat_count: 359,
            })),
            ..ModelInputs::default()
        };
        (w, inputs, key)
    }

    // Frenzy without items (base-game speed 179 from
    // frame 21): the attack branch of the rate with the used skill's
    // `UseAttackRate`, 256 · (100 − 30) / 100.
    #[test]
    fn a_player_mode_18_attack_skill_reads_the_attack_rate() {
        let (mut w, inputs, key) = swinging(147);
        mode_set(&mut w, &inputs, key, 18);
        assert_eq!(w.units[&key].speed, Some(179));
        // A TEMPONLY attack-rate list (Double Swing's start) counts until
        // the next mode change: 256 · (100 + 50 − 30) / 100.
        w.units
            .get_mut(&key)
            .unwrap()
            .temp_lists
            .push([((68, 0), 50)].into());
        refresh_rate(&mut w, &inputs, key);
        assert_eq!(w.units[&key].speed, Some(307));
        mode_set(&mut w, &inputs, key, 1);
        assert!(w.units[&key].temp_lists.is_empty());
    }

    /// A javelin (`1ht`) Jab: the load plays sequence 1's `1ht` list (18
    /// frames) for the `hth` record's count, 13 (base-game count 3328).
    #[test]
    fn a_sequence_load_counts_the_hth_records_frames() {
        use crate::bridge::skills::{SkillEntry, SkillList};
        #[derive(Debug)]
        struct OneHt;
        impl PlayerAnims for OneHt {
            fn anim(&self, _: &ClientWorld, _: UnitKey, _: u32) -> Option<PlayerAnim> {
                Some(PlayerAnim {
                    frames: 10,
                    speed: 256,
                    // `sequences::CLASSES` index of `1ht`.
                    weapon: 3,
                    events: None,
                })
            }
        }
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 10,
                ..SkillEntry::default()
            }],
            current: Some(0),
            ..SkillList::default()
        });
        let mut inputs = ModelInputs {
            player_anims: Some(Arc::new(OneHt)),
            ..ModelInputs::default()
        };
        let mut rows = vec![crate::bridge::world::SkillRow::default(); 11];
        rows[10].seqnum = 1;
        inputs.tables.skills = rows;
        mode_set(&mut w, &inputs, key, 18);
        let u = &w.units[&key];
        let seq = u.sequence.as_ref().expect("loaded");
        assert_eq!(seq.frames.len(), 18, "the 1ht list");
        assert_eq!((u.frame_count, seq.length), (13 << 8, 13 << 8));
    }

    /// A mode-18 do that reports an unmodelled branch (Charge's search pick)
    /// must not stall the sequence: the frame advance and the neutral end
    /// still run, and only then is the error reported.
    #[test]
    fn a_mode_18_do_error_still_advances_and_ends_the_sequence() {
        use crate::bridge::skills::{SkillEntry, SkillList, NATIVE};
        const CHARGE: u16 = 43;
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().position = Some((100, 100));
        let near = UnitKey::new(MONSTER, 9);
        let mut monster = ClientUnit::new(near);
        monster.position = Some((101, 100));
        monster.mode = 1;
        w.units.insert(near, monster);
        let u = w.units.get_mut(&key).unwrap();
        u.mode = 18;
        u.speed = Some(256);
        u.frame_count = 0x200;
        u.action_frame = 1;
        u.path_live = Some(false);
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: CHARGE,
                base: 1,
                owner: NATIVE,
                flags: 3,
                param: 6,
                ..SkillEntry::default()
            }],
            current: Some(0),
            ..SkillList::default()
        });
        let mut inputs = ModelInputs::default();
        let mut rows = vec![crate::bridge::world::SkillRow::default(); usize::from(CHARGE) + 1];
        rows[usize::from(CHARGE)].cltdofunc = 37;
        inputs.tables.skills = rows;
        assert!(step(&mut w, &inputs, key).is_err());
        assert_eq!(w.units[&key].mode, 1, "the sequence returned to neutral");
    }
}
