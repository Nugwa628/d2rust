//! How fast each unit's animation is drawn: the drawn-frame clock behind
//! the `anim_rate` check.
//!
//! Every built world frame records, per unit with a pose, which COF frame
//! it drew and of how many ([`DrawnFrame`], [`super::WorldFrame::unit_frames`]).
//! [`DrawnAnim`] follows those frames over the server ticks the frames
//! show: while a unit stays in one mode with one frame count, each drawn
//! tick adds the frames the picture moved on since the tick before (a
//! loop's wrap counts forward; a segment whose first step went one frame
//! back in one tick, a sequence played backwards, counts each frame back),
//! so a mode's segment knows how many frames it advanced in how many ticks. That is the rate a player sees, in frames
//! per second, whatever made it (the model's frame, the walk clock, the
//! tick clock of a fallback).
//!
//! The rule it serves: a unit's animation runs at its speed table's rate,
//! `speed / 256` frames per base tick of 25 Hz, whatever the logic tick
//! rate. The repro query `anim_rate(TYPE, CLASS, MODE)` compares a
//! segment's rate with the server's speed for the unit.
//!
//! Only reads what the frame drew; it never changes a pose or a tick.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::bridge::world::UnitKey;

/// The COF frame a unit was drawn with, in the built frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawnFrame {
    pub class: u32,
    pub mode: u32,
    /// The COF frame index drawn ([`super::UnitPose::frame`]).
    pub frame: u16,
    /// The COF's frames per direction.
    pub frames: u16,
}

/// One stretch of a unit in one mode: frames advanced over ticks drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub frames: u16,
    /// Server ticks from the segment's first drawn tick to its last.
    pub ticks: u64,
    /// Frames the drawn picture advanced in that time.
    pub advances: u64,
}

impl Segment {
    /// Frames per second at `hz` server ticks per second.
    pub fn fps(&self, hz: u32) -> Option<f64> {
        (self.ticks > 0).then(|| self.advances as f64 * f64::from(hz) / self.ticks as f64)
    }
}

#[derive(Clone, Debug)]
struct Track {
    last: DrawnFrame,
    tick: u64,
    /// The current mode's segment.
    now: Segment,
    /// The latest segment of each mode the unit was drawn in.
    modes: BTreeMap<u32, Segment>,
    /// The current segment's direction once its first frame step showed
    /// it: `true` when the frames run backwards.
    backward: Option<bool>,
}

/// The drawn-frame clock of every unit in view.
#[derive(Clone, Debug, Default)]
pub struct DrawnAnim {
    units: BTreeMap<UnitKey, Track>,
    /// Server ticks per second of the frames observed.
    pub hz: u32,
}

impl DrawnAnim {
    /// One built frame showing server tick `tick`. Units the frame did not
    /// draw are forgotten (they left the view).
    pub fn observe(&mut self, tick: u64, hz: u32, drawn: &BTreeMap<UnitKey, DrawnFrame>) {
        self.hz = hz;
        self.units.retain(|k, _| drawn.contains_key(k));
        for (key, d) in drawn {
            let t = self.units.entry(*key).or_insert_with(|| Track {
                last: *d,
                tick,
                now: Segment {
                    frames: d.frames,
                    ..Segment::default()
                },
                modes: BTreeMap::new(),
                backward: None,
            });
            if d.mode != t.last.mode || d.frames != t.last.frames || d.class != t.last.class {
                // A new mode (or new art) starts a new segment.
                t.now = Segment {
                    frames: d.frames,
                    ..Segment::default()
                };
                t.backward = None;
            } else if tick > t.tick && d.frames > 0 {
                let n = u64::from(d.frames);
                let forward = (u64::from(d.frame) + n - u64::from(t.last.frame) % n) % n;
                // A sequence may play its frames backwards (a tentacle
                // head's submerge, 20 down to 0). The segment's first step
                // sets its direction: backwards only when that step went one
                // frame back in one tick (no rate draws a loop of 3 or more
                // frames n − 1 frames on in a tick); a step over several
                // ticks counts forward. The direction then holds for the
                // segment, so a fast short loop is never read backwards.
                if forward != 0 && t.backward.is_none() {
                    t.backward = Some(n > 2 && forward == n - 1 && tick - t.tick == 1);
                }
                t.now.advances += match t.backward {
                    Some(true) => (n - forward) % n,
                    _ => forward,
                };
                t.now.ticks += tick - t.tick;
            }
            t.last = *d;
            t.tick = t.tick.max(tick);
            t.modes.insert(d.mode, t.now);
        }
    }

    /// The latest segment of `key` in `mode`, if drawn in it.
    pub fn segment(&self, key: UnitKey, mode: u32) -> Option<Segment> {
        self.units
            .get(&key)
            .and_then(|t| t.modes.get(&mode).copied())
    }

    /// The state reply's `client.anim`: one entry per unit in view, with
    /// its drawn frame and per-mode segments (`fps` once a tick passed).
    pub fn json(&self) -> Value {
        Value::Array(
            self.units
                .iter()
                .map(|(k, t)| {
                    let modes: serde_json::Map<String, Value> = t
                        .modes
                        .iter()
                        .map(|(m, s)| {
                            (
                                m.to_string(),
                                json!({"frames": s.frames, "ticks": s.ticks,
                                "advances": s.advances, "fps": s.fps(self.hz)}),
                            )
                        })
                        .collect();
                    json!({"t": k.unit_type, "g": k.guid, "class": t.last.class,
                        "mode": t.last.mode, "frame": t.last.frame, "frames": t.last.frames,
                        "hz": self.hz, "modes": modes})
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mode: u32, frame: u16, frames: u16) -> BTreeMap<UnitKey, DrawnFrame> {
        BTreeMap::from([(
            UnitKey::new(0, 1),
            DrawnFrame {
                class: 1,
                mode,
                frame,
                frames,
            },
        )])
    }

    #[test]
    fn a_loop_at_its_table_rate_measures_that_rate() {
        // speed 256 at 25 Hz: 25 frames a second, drawn at 60 Hz ticks.
        let mut a = DrawnAnim::default();
        for tick in 1..=121u64 {
            let frame = (((tick - 1) * 256 * 25 / 60) >> 8) % 8;
            a.observe(tick, 60, &at(1, frame as u16, 8));
        }
        let s = a.segment(UnitKey::new(0, 1), 1).unwrap();
        assert_eq!(s.ticks, 120);
        let fps = s.fps(60).unwrap();
        assert!((fps - 25.0).abs() < 0.6, "{fps}");
    }

    #[test]
    fn a_per_tick_advance_tuned_for_25_hz_runs_2_4_times_too_fast() {
        let mut a = DrawnAnim::default();
        for tick in 1..=61u64 {
            a.observe(tick, 60, &at(2, ((tick - 1) % 8) as u16, 8));
        }
        assert_eq!(
            a.segment(UnitKey::new(0, 1), 2).unwrap().fps(60),
            Some(60.0)
        );
    }

    #[test]
    fn a_sequence_played_backwards_measures_its_rate() {
        // 21 frames from 20 down to 0 at speed 256 (25 a second), drawn at
        // 60 Hz ticks: 25 frames a second, not 20 a step.
        let mut a = DrawnAnim::default();
        for tick in 1..=49u64 {
            let back = ((tick - 1) * 256 * 25 / 60) >> 8;
            a.observe(tick, 60, &at(14, (20 - back) as u16, 21));
        }
        let fps = a.segment(UnitKey::new(0, 1), 14).unwrap().fps(60).unwrap();
        assert!((fps - 25.0).abs() < 1.5, "{fps}");
    }

    #[test]
    fn a_short_loop_seen_every_few_ticks_counts_forward() {
        // 4 frames, one a tick, drawn every 3rd tick: each step is 3 frames
        // on (one back the short way), 60 frames a second.
        let mut a = DrawnAnim::default();
        for tick in (1..=61u64).step_by(3) {
            a.observe(tick, 60, &at(4, ((tick - 1) % 4) as u16, 4));
        }
        let fps = a.segment(UnitKey::new(0, 1), 4).unwrap().fps(60).unwrap();
        assert!((fps - 60.0).abs() < 0.1, "{fps}");
    }

    #[test]
    fn a_mode_change_starts_a_segment_and_keeps_the_last_one() {
        let mut a = DrawnAnim::default();
        for tick in 1..=11 {
            a.observe(tick, 60, &at(1, (tick % 4) as u16, 4));
        }
        for tick in 12..=15 {
            a.observe(tick, 60, &at(3, 0, 12));
        }
        let key = UnitKey::new(0, 1);
        assert_eq!(a.segment(key, 1).unwrap().advances, 10);
        assert_eq!(
            a.segment(key, 3).unwrap(),
            Segment {
                frames: 12,
                ticks: 3,
                advances: 0
            }
        );
        // Gone from the frame: forgotten.
        a.observe(16, 60, &BTreeMap::new());
        assert_eq!(a.segment(key, 1), None);
        assert_eq!(a.json(), json!([]));
    }

    #[test]
    fn a_tick_drawn_twice_counts_once() {
        let mut a = DrawnAnim::default();
        a.observe(1, 60, &at(1, 0, 8));
        a.observe(2, 60, &at(1, 1, 8));
        a.observe(2, 60, &at(1, 1, 8));
        a.observe(4, 60, &at(1, 3, 8));
        assert_eq!(
            a.segment(UnitKey::new(0, 1), 1).unwrap(),
            Segment {
                frames: 8,
                ticks: 3,
                advances: 3
            }
        );
    }
}
