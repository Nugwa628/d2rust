//! Mode schedules: the animation schedule of events 0 and 1 (the
//! main form and the variants) as a pure function of its inputs, and its application
//! to the timer queue; the every-tick movement event; the
//! sequence branch of the frame advance
//! ([`advance_sequence`]).

use thiserror::Error;

use crate::game::{Game, GameError};
use crate::tick::events::event;
use crate::tick::TickRate;

use super::record::{Anim, ANIM_EVENTS};
use super::UnitId;

/// One animation schedule: event type, expire, a1, a2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheduled {
    pub event: u8,
    pub expire: i32,
    pub a1: u32,
    pub a2: u32,
}

/// Which form runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Start index = the frame bonus.
    Main { bonus: i32 },
    /// Argument a percent.
    Percent(i32),
    /// Argument a frame count (p ≤ 0: nothing at all).
    Frames(i32),
    /// Argument a start frame.
    StartFrame(i32),
}

/// A computed schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// The variants cancel the unit's type-0 and type-1 events first.
    pub cancels: bool,
    pub events: Vec<Scheduled>,
    /// The new current frame +0x44, when the form sets it.
    pub frame: Option<i32>,
}

/// Inputs the animation schedule cannot proceed with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnimError {
    /// A negative speed never ends the loop in the base game (edge case 3).
    #[error("negative animation speed {0}: animation advancement would never finish")]
    NegativeSpeed(i32),
    /// The frame position `a += s` passes 2^31 − 1 before reaching the
    /// frame count: the base game's signed loop wraps and never ends, as with a
    /// negative speed (edge case 3).
    #[error(
        "animation speed {speed} wraps before frame count {frame_count}: animation advancement would never finish"
    )]
    Endless { speed: i32, frame_count: i32 },
    /// A variant read an event byte outside the AnimData record
    /// (index ≥ 144, or < −1): the record bytes past it are not known
    /// here. TODO: reproduce from the record set.
    #[error("event byte index {0} outside the animation record")]
    EventIndex(i32),
    /// No animation data to schedule from.
    #[error("no animation record")]
    NoRecord,
    #[error(transparent)]
    Game(#[from] GameError),
}

/// Where the event bytes come from: the AnimData record (index −1 is
/// +0x0F), or a sequence.
#[derive(Clone, Copy, Debug)]
pub enum Events<'a> {
    Record {
        byte_0f: u8,
        events: &'a [u8; ANIM_EVENTS],
    },
    Sequence(&'a [u8]),
}

impl Events<'_> {
    fn get(&self, i: i32) -> Result<u8, AnimError> {
        match *self {
            Events::Record { byte_0f, events } => match i {
                -1 => Ok(byte_0f),
                _ => usize::try_from(i)
                    .ok()
                    .and_then(|i| events.get(i).copied())
                    .ok_or(AnimError::EventIndex(i)),
            },
            // TODO(sequence): sequence event bytes outside the
            // sequence are not described.
            Events::Sequence(e) => usize::try_from(i)
                .ok()
                .and_then(|i| e.get(i).copied())
                .ok_or(AnimError::EventIndex(i)),
        }
    }
}

/// The schedule of `form` at frame `f` with speed `s`, frame
/// count `frame_count` (8.8) and current frame `cur` (+0x44, 8.8), at
/// the base tick rate.
pub fn schedule(
    form: Form,
    f: i32,
    s: i32,
    frame_count: i32,
    cur: i32,
    events: Events<'_>,
) -> Result<Option<Schedule>, AnimError> {
    schedule_at(TickRate::BASE, form, f, s, frame_count, cur, events)
}

/// [`schedule`] at tick rate `rate`. The speed `s` is stated per base
/// tick (8.8 frames); each tick `n` advances the animation by its share
/// [`TickRate::share`]`(s, n)`, as [`advance_frame_by`] does, so frame
/// events land at the same real time at any rate. Identity at 25 Hz.
pub fn schedule_at(
    rate: TickRate,
    form: Form,
    f: i32,
    s: i32,
    frame_count: i32,
    cur: i32,
    events: Events<'_>,
) -> Result<Option<Schedule>, AnimError> {
    let step = |tick: i32| rate.share(s, tick);
    let variant = !matches!(form, Form::Main { .. });
    if let Form::Frames(p) = form {
        if p <= 0 {
            return Ok(None);
        }
    }
    // TODO: "speed 0 gives event 1 at f + 1 in all
    // forms"; the variants' +0x44 write is read as skipped then, as in
    // the main form.
    if s == 0 {
        return Ok(Some(Schedule {
            cancels: variant,
            events: vec![Scheduled {
                event: event::END_ANIM,
                expire: f.wrapping_add(1),
                a1: 0,
                a2: 0,
            }],
            frame: None,
        }));
    }
    if s < 0 {
        return Err(AnimError::NegativeSpeed(s));
    }
    let cur_frame = cur >> 8;
    let (start, c) = match form {
        Form::Main { bonus } => (bonus, bonus),
        Form::Percent(p) => {
            let c = 100i32
                .wrapping_sub(p)
                .wrapping_mul(f.wrapping_sub(cur_frame))
                / 100;
            (c.wrapping_sub(1), c)
        }
        Form::Frames(p) => {
            let c = f.wrapping_sub(cur_frame).wrapping_sub(p);
            (c.wrapping_sub(1), c)
        }
        Form::StartFrame(p) => (p.wrapping_sub(1), p),
    };
    let mut n = f;
    let mut i = start;
    let mut k = 0u32;
    let mut a = c.wrapping_mul(256).wrapping_add(step(f.wrapping_add(1)));
    if a < frame_count {
        // The last a + s (i64) must stay a valid i32, else the loop wraps.
        // A tick's share is at most `s`, so the base-rate bound holds at
        // any rate.
        let (a0, s64, fc) = (i64::from(a), i64::from(s), i64::from(frame_count));
        let iterations = (fc - a0 + s64 - 1) / s64;
        if a0 + iterations * s64 > i64::from(i32::MAX) {
            return Err(AnimError::Endless {
                speed: s,
                frame_count,
            });
        }
    }
    let mut out = Vec::new();
    while a < frame_count {
        n = n.wrapping_add(1);
        while i <= a >> 8 && (variant || i < ANIM_EVENTS as i32) {
            let e = events.get(i)?;
            if variant {
                if matches!(e, 1..=4) {
                    out.push(Scheduled {
                        event: event::MODE_CHANGE,
                        expire: n,
                        a1: u32::from(e),
                        a2: 0,
                    });
                }
            } else if matches!(e, 1 | 2 | 4) {
                out.push(Scheduled {
                    event: event::MODE_CHANGE,
                    expire: n,
                    a1: u32::from(e),
                    a2: k,
                });
                k += 1;
            } else if e == 3 {
                out.push(Scheduled {
                    event: event::MODE_CHANGE,
                    expire: n,
                    a1: 3,
                    a2: 0,
                });
            }
            i += 1;
        }
        a = a.wrapping_add(step(n.wrapping_add(1)));
    }
    if n == f {
        n = f.wrapping_add(1);
    }
    out.push(Scheduled {
        event: event::END_ANIM,
        expire: n.wrapping_add(1),
        a1: 0,
        a2: 0,
    });
    let frame = if variant {
        f.wrapping_sub(c).wrapping_mul(256)
    } else {
        f.wrapping_mul(256)
    };
    Ok(Some(Schedule {
        cancels: variant,
        events: out,
        frame: Some(frame),
    }))
}

/// Cancel the unit's events of type 0, then type 1, any
/// argument.
pub fn cancel_mode_events(game: &mut Game, unit: UnitId) {
    game.timers
        .cancel_unit_events(unit, event::MODE_CHANGE, None);
    game.timers.cancel_unit_events(unit, event::END_ANIM, None);
}

/// Runs for a unit's animation fields: the speed and frame count
/// of its sequence if it has one, else +0x4C / +0x48; event bytes from
/// the sequence or the AnimData record. Cancels (variants), schedules,
/// then sets +0x44.
pub fn run(game: &mut Game, unit: UnitId, anim: &mut Anim, form: Form) -> Result<(), AnimError> {
    let (f, rate) = (game.frame, game.tick_rate);
    let sched = match &anim.sequence {
        Some(seq) => schedule_at(
            rate,
            form,
            f,
            seq.speed,
            seq.frame_count,
            anim.frame,
            Events::Sequence(&seq.events),
        )?,
        None => {
            let rec = anim.record.as_ref().ok_or(AnimError::NoRecord)?;
            schedule_at(
                rate,
                form,
                f,
                i32::from(anim.speed),
                anim.frame_count,
                anim.frame,
                Events::Record {
                    byte_0f: rec.byte_0f,
                    events: &rec.events,
                },
            )?
        }
    };
    let Some(sched) = sched else {
        return Ok(());
    };
    apply(game, unit, &sched)?;
    if let Some(frame) = sched.frame {
        anim.frame = frame;
    }
    Ok(())
}

/// Applies a schedule to the timer queue.
pub fn apply(game: &mut Game, unit: UnitId, sched: &Schedule) -> Result<(), GameError> {
    if sched.cancels {
        cancel_mode_events(game, unit);
    }
    for e in &sched.events {
        game.schedule_event(unit, u32::from(e.event), e.expire, None, e.a1, e.a2)?;
    }
    Ok(())
}

/// Cancel the unit's type-0 events, then schedule
/// an every-tick type-0 event, args (0, 0).
pub fn every_tick_movement(game: &mut Game, unit: UnitId) -> Result<(), GameError> {
    game.timers
        .cancel_unit_events(unit, event::MODE_CHANGE, None);
    let owner = game.owner(unit)?;
    game.timers
        .schedule_every_tick(owner, u32::from(event::MODE_CHANGE), None, 0, 0)
        .map_err(GameError::from)?;
    Ok(())
}

/// The sequence branch of the frame advance:
/// +0x4E := 0; p = +0x38 + +0x3C, minus
/// +0x34 once when ≥ +0x34; +0x38 := p; +0x48 −= +0x3C; then the frame
/// setup with b = the old +0x38 stores in +0x4E the event byte of frame
/// p (p = b), else the last non-zero event byte of frames (b >> 8) + 1 …
/// p >> 8 (0 when none or the range is empty). False (nothing done)
/// without a sequence. The drawn mode (+0x40) is not kept; the drawn frame
/// (+0x44) is, when the sequence holds the drawn frames.
pub fn advance_sequence(anim: &mut Anim) -> bool {
    advance_sequence_at(anim, TickRate::BASE, 0)
}

/// [`advance_sequence`] on tick `tick` at rate `rate`: the sequence moves
/// by this tick's share of its speed (stated per base tick). Identity at
/// 25 Hz.
pub fn advance_sequence_at(anim: &mut Anim, rate: TickRate, tick: i32) -> bool {
    let Some(seq) = anim.sequence.as_mut() else {
        return false;
    };
    let speed = rate.share(seq.speed, tick);
    let old = seq.pos;
    let mut p = old.wrapping_add(speed);
    if p >= seq.frame_count {
        p = p.wrapping_sub(seq.frame_count);
    }
    seq.pos = p;
    anim.frame_count = anim.frame_count.wrapping_sub(speed);
    let byte = |i: i32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| seq.events.get(i))
            .copied()
            .unwrap_or(0)
    };
    let (a, b) = (p >> 8, old >> 8);
    // Frame setup: +0x44 := the drawn frame of frame p · 256.
    if let Some(&d) = usize::try_from(p >> 8).ok().and_then(|i| seq.drawn.get(i)) {
        anim.frame = i32::from(d) << 8;
    }
    anim.action_frame = if p == old {
        byte(a)
    } else {
        (b + 1..=a).rev().map(byte).find(|&e| e != 0).unwrap_or(0)
    };
    true
}

/// The sequence-less branch of the frame advance: +0x4E := 0;
/// j = cur >> 8 (+ 1
/// when the speed is above 255); cur += speed; while cur ≥ the frame
/// count, the action bytes of the frames j .. (count >> 8) − 1 are noted,
/// cur −= count, j := 0 and cur += 256 · `bonus` (the frame bonus);
/// then those of the frames j..= cur >> 8. The last byte
/// in 1..=4 noted wins. A frame count of zero or less never wraps (the
/// base game's loop would not end).
pub fn advance_frame(anim: &mut Anim, bonus: i32) {
    advance_frame_by(anim, bonus, i32::from(anim.speed));
}

/// [`advance_frame`] by `step` (8.8 frames) instead of the speed: at a
/// tick rate other than the base rate, the tick's share of the speed
/// ([`TickRate::share`]).
pub fn advance_frame_by(anim: &mut Anim, bonus: i32, step: i32) {
    anim.action_frame = 0;
    let mut j = anim.frame >> 8;
    if step > 255 {
        j += 1;
    }
    anim.frame = anim.frame.wrapping_add(step);
    let events = anim.record.as_ref().map(|r| r.events);
    let note = |from: i32, to: i32, action: &mut u8| {
        let Some(e) = events.as_ref() else {
            return;
        };
        for i in from..to {
            if let Some(&b) = usize::try_from(i).ok().and_then(|i| e.get(i)) {
                if (1..=4).contains(&b) {
                    *action = b;
                }
            }
        }
    };
    while anim.frame_count > 0 && anim.frame_count <= anim.frame {
        note(j, anim.frame_count >> 8, &mut anim.action_frame);
        anim.frame = anim.frame.wrapping_sub(anim.frame_count);
        j = 0;
        anim.frame = anim.frame.wrapping_add(bonus.wrapping_mul(256));
    }
    note(j, (anim.frame >> 8) + 1, &mut anim.action_frame);
}

#[cfg(test)]
mod advance_frame_tests {
    use super::*;
    use crate::units::record::{Anim, AnimRecord, ANIM_EVENTS};

    #[test]
    fn baal_s2_frame_wraps_by_the_frame_count() {
        // Recorded (`gen-lvl-132` frame 97): +0x44 18688 → 1184 with the
        // speed 160 and the frame count 5888.
        let mut a = Anim {
            frame: 18688,
            frame_count: 5888,
            speed: 160,
            ..Anim::default()
        };
        advance_frame(&mut a, 0);
        assert_eq!(a.frame, 1184);
        assert_eq!(a.action_frame, 0);
    }

    #[test]
    fn the_last_action_byte_crossed_wins() {
        let mut events = [0u8; ANIM_EVENTS];
        events[1] = 2;
        events[2] = 4;
        let mut a = Anim {
            frame: 256,
            frame_count: 10 << 8,
            speed: 512,
            record: Some(AnimRecord {
                frames: 10,
                byte_0f: 0,
                events,
            }),
            ..Anim::default()
        };
        // Speed ≥ 256 starts one frame later: frames 2 ..= 3 are noted.
        advance_frame(&mut a, 0);
        assert_eq!((a.frame, a.action_frame), (768, 4));
        // A wrap adds the bonus and notes the tail frames first.
        a.frame = 9 << 8;
        a.action_frame = 9;
        advance_frame(&mut a, 1);
        assert_eq!(a.frame, (9 << 8) + 512 - (10 << 8) + 256);
    }

    #[test]
    fn a_zero_frame_count_never_wraps() {
        let mut a = Anim {
            frame: 0,
            frame_count: 0,
            speed: 256,
            ..Anim::default()
        };
        advance_frame(&mut a, 3);
        assert_eq!(a.frame, 256);
    }
}

#[cfg(test)]
mod tick_rate_tests {
    use super::*;
    use crate::units::record::{Anim, ANIM_EVENTS};

    fn rate(hz: u32) -> TickRate {
        TickRate::new(hz).unwrap()
    }

    #[test]
    fn frame_events_land_at_the_same_real_time() {
        // 12 frames at one frame per base tick, an attack event on frame 6.
        let mut events = [0u8; ANIM_EVENTS];
        events[6] = 1;
        let ev = Events::Record {
            byte_0f: 0,
            events: &events,
        };
        let at = |hz: u32| {
            let s = schedule_at(rate(hz), Form::Main { bonus: 0 }, 1000, 256, 12 << 8, 0, ev)
                .unwrap()
                .unwrap();
            let hit = s
                .events
                .iter()
                .find(|e| e.event == event::MODE_CHANGE)
                .unwrap();
            let end = s
                .events
                .iter()
                .find(|e| e.event == event::END_ANIM)
                .unwrap();
            (hit.expire - 1000, end.expire - 1000)
        };
        // The base rate: frame 6 is reached on the 6th tick, the end
        // event follows the 11th.
        assert_eq!(at(25), (6, 12));
        for hz in [30, 60, 100, 144] {
            let r = rate(hz);
            let (hit, end) = at(hz);
            assert!((hit - r.ticks(6)).abs() <= 1, "{hz} Hz: hit {hit}");
            assert!((end - r.ticks(12)).abs() <= 1, "{hz} Hz: end {end}");
        }
    }

    #[test]
    fn a_second_of_frames_is_the_same_at_any_rate() {
        for hz in [25, 60, 144] {
            let r = rate(hz);
            let mut a = Anim {
                frame: 0,
                frame_count: 1000 << 8,
                speed: 300,
                ..Anim::default()
            };
            for tick in 0..hz as i32 {
                advance_frame_by(&mut a, 0, r.share(300, tick));
            }
            assert_eq!(a.frame, 300 * 25, "{hz} Hz");
        }
    }
}
