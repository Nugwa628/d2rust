//! The scripted ambient overrides: moments where the story takes over a
//! room's light.
//!
//! - **The Den of Evil glow**: while the Den's quest is open (client quest
//!   byte 1 set) its rooms glow a dim red; when the quest completes the
//!   glow pulses out over 30 updates and the Den's lights are placed.
//! - **The red glow of the River of Flame and the Chaos Sanctuary**:
//!   switched on and off by unique events.
//! - **Darkness events**: a fade of the player room's ambient to black and
//!   back (fade in, hold, fade out).
//!
//! The unique-event message (S→C 0x89) is the trigger for these; its
//! dispatcher lives here too.
//!
//! The sine table and the fades use doubles: client-only state that never
//! feeds `d2-sim`.

use std::sync::OnceLock;

use d2_sim::rng::Seed;
use thiserror::Error;

use super::environment::{Ambient, PI_F};

/// The Den of Evil.
pub const LEVEL_DEN_OF_EVIL: u32 = 8;
/// The River of Flame and the Chaos Sanctuary: the red glow levels.
pub const LEVELS_RED_GLOW: [u32; 2] = [107, 108];

/// The sine table `W` (512 floats): `W[i]` = float(`sin(i · π_f / 256)`),
/// the pulse of the glowing overrides.
pub fn sine_table() -> &'static [f32; 512] {
    static W: OnceLock<[f32; 512]> = OnceLock::new();
    W.get_or_init(|| std::array::from_fn(|i| (i as f64 * PI_F / 256.0).sin() as f32))
}

/// `base · cos(angle)` from the sine table, truncated: the brightness of a
/// fade at table angle `a`.
fn wave(a: i32, base: f64) -> i32 {
    let w = sine_table()[((a + 128) & 511) as usize];
    (f64::from(w) * base) as i32
}

/// The red glow color of the glowing overrides.
const RED_GLOW: (u8, u8, u8) = (255, 64, 48);

/// A running darkness event: the player room's ambient fades to black
/// over `fade_in` updates, stays black for `hold`, and fades back over
/// `fade_out`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DarknessEvent {
    pub fade_in: i32,
    pub hold: i32,
    /// The fade-out length; a request of 0 becomes 25.
    pub fade_out: i32,
    /// The level the event belongs to: leaving it ends the event.
    pub level: u32,
    /// Updates since the event started.
    pub c: i32,
    /// The fade's table angle; it lags one update behind `c`.
    pub a: i32,
    /// The ambient the event gives this update.
    pub out: Ambient,
}

impl DarknessEvent {
    /// A new event in `level`; the counter and angle start at 0.
    pub fn new(fade_in: i32, hold: i32, fade_out: i32, level: u32) -> Self {
        DarknessEvent {
            fade_in,
            hold,
            fade_out: if fade_out == 0 { 25 } else { fade_out },
            level,
            c: 0,
            a: 0,
            out: Ambient::ZERO,
        }
    }

    fn total(&self) -> i32 {
        self.fade_in + self.hold + self.fade_out
    }
}

/// What the unique-event dispatcher did beyond its state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UniqueEventEffect {
    /// Id 0: the Den was cleared; its glow counter started.
    DenCounterStarted,
    /// Id 12: the red glow switched on. `in_level_108`: the player is in
    /// the Chaos Sanctuary, where the event also creates client missile
    /// 372 at the local player and plays its effect; the caller owns
    /// those.
    RedGlowStarted { in_level_108: bool },
    /// Id 13: the red glow's counter started; it switches the glow off
    /// after 30 updates.
    RedGlowCounterStarted,
    /// Ids 1, 3, 6, 14, 16, 17, 19: a client effect of its own that the
    /// caller handles (id 1, the cairn stones, starts a darkness event
    /// from missile 288's fields). Nothing here changes beyond the id's
    /// bit.
    OtherClientEffect(u8),
    /// The other ids below 20: the bit only.
    None,
}

/// A unique-event id the dispatcher cannot accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OverrideError {
    /// The id does not fit the 32-bit set of received events.
    #[error("S→C 0x89: id {0} ≥ 32 (fatal 0x1D9)")]
    Fatal1D9(u8),
    #[error("S→C 0x89: id {0} ≥ 20 (fatal 0x1DA)")]
    Fatal1DA(u8),
}

/// The client globals of the overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Overrides {
    /// The Den's lights have been placed (the Den is cleared and its glow
    /// is over).
    pub den_flag: bool,
    /// The Den glow's fade counter, −1 when idle.
    pub den_counter: i32,
    /// The River / Sanctuary glow is on.
    pub glow_flag: bool,
    /// The River / Sanctuary glow's counter, −1 when idle; while it runs
    /// the glow is off, and when it ends the glow flag clears.
    pub glow_counter: i32,
    /// The wall-clock second the red glow's counter started, plus 90.
    pub glow_value: i32,
    /// The unique-event ids received, one bit per id.
    pub unique_bits: u32,
    /// The darkness event, `None` when none runs.
    pub darkness: Option<DarknessEvent>,
}

impl Default for Overrides {
    fn default() -> Self {
        Overrides {
            den_flag: false,
            den_counter: -1,
            glow_flag: false,
            glow_counter: -1,
            glow_value: 0,
            unique_bits: 0,
            darkness: None,
        }
    }
}

impl Overrides {
    /// A new game: both flags off, both counters idle. Everything else is
    /// kept.
    pub fn game_start(&mut self) {
        self.den_flag = false;
        self.den_counter = -1;
        self.glow_flag = false;
        self.glow_counter = -1;
    }

    /// The override of a room of level `level`; `quest_byte1` is the
    /// client's quest byte 1 (set while the Den's quest is open). A result
    /// without colour means "no override" and the room falls through to
    /// its own light (see [`super::environment::room_ambient`]).
    ///
    /// - The Den, its quest open and its lights not yet placed: red at 80, or
    ///   while its counter runs, red pulsing out from 80 to 0 over the 30
    ///   updates.
    /// - The River / Sanctuary: red at 160 while the glow is on and its
    ///   counter idle.
    /// - Any other level: the running darkness event's ambient.
    pub fn ambient(&self, level: u32, quest_byte1: u8) -> Ambient {
        let (r, g, b) = RED_GLOW;
        if level == LEVEL_DEN_OF_EVIL {
            if !self.den_flag && quest_byte1 != 0 {
                let i = if self.den_counter == -1 {
                    80
                } else {
                    let a = self.den_counter * 128 / 30;
                    wave(a, 80.0)
                };
                return Ambient {
                    i: i as u8,
                    r,
                    g,
                    b,
                };
            }
            return Ambient::ZERO;
        }
        if LEVELS_RED_GLOW.contains(&level) {
            if self.glow_flag && self.glow_counter == -1 {
                return Ambient { i: 160, r, g, b };
            }
            return Ambient::ZERO;
        }
        self.darkness.map_or(Ambient::ZERO, |e| e.out)
    }

    /// Steps the counters once per client update. Returns `true` when the
    /// Den's counter ended on this update: the Den's lights are now placed
    /// in every loaded Den room ([`den_light_points`]).
    ///
    /// The red glow's counter runs while it is not idle; when it ends the
    /// glow switches off and the counter goes idle.
    pub fn update_counters(&mut self) -> bool {
        let mut den_lights = false;
        if self.den_counter >= 0 && !self.den_flag {
            self.den_counter += 1;
            if self.den_counter > 29 {
                self.den_flag = true;
                den_lights = true;
            }
        }
        if self.glow_counter >= 0 {
            self.glow_counter += 1;
            if self.glow_counter > 29 {
                self.glow_flag = false;
                self.glow_counter = -1;
            }
        }
        den_lights
    }

    /// Whether a room of `level` loaded now gets the Den's lights: Den
    /// rooms do once its lights have been placed.
    pub fn room_load_gets_den_lights(&self, level: u32) -> bool {
        level == LEVEL_DEN_OF_EVIL && self.den_flag
    }

    /// Starts a darkness event, replacing any running one.
    pub fn start_darkness(&mut self, fade_in: i32, hold: i32, fade_out: i32, level: u32) {
        self.darkness = Some(DarknessEvent::new(fade_in, hold, fade_out, level));
    }

    /// Steps the darkness event once per client update. `base` is the
    /// player room's ambient without override (what the event darkens);
    /// `level` the player room's level id.
    ///
    /// The intensity is `base · cos(a)`: during the fade-in `a` rises to a
    /// quarter turn, during the hold the room is black, during the
    /// fade-out `a` falls back to 0. The angle is computed after the
    /// intensity, so it lags one update. The colour stays the base's. The
    /// event ends after its total length or as soon as the player is in
    /// another level.
    pub fn update_darkness(&mut self, base: Ambient, level: u32) {
        let Some(e) = self.darkness.as_mut() else {
            return;
        };
        let i0 = f64::from(base.i);
        let total = e.total();
        let i = if e.c < e.fade_in {
            let i = wave(e.a, i0);
            e.a = e.c * 128 / e.fade_in;
            i
        } else if e.c < total - e.fade_out {
            0
        } else {
            let i = wave(e.a, i0);
            e.a = 128 - (e.c - total + e.fade_out) * 128 / e.fade_out;
            i
        };
        e.out = Ambient {
            i: i as u8,
            r: base.r,
            g: base.g,
            b: base.b,
        };
        e.c += 1;
        if e.c > total || level != e.level {
            self.darkness = None;
        }
    }

    /// The unique-event message (S→C 0x89) with event `id`. `level` is the
    /// local player's level id (read by id 12); `wall_seconds` gives the
    /// wall-clock seconds and is called only for id 13.
    ///
    /// Ids from 20 up are rejected. Otherwise the id's bit is recorded and
    /// the id's override state changes: 0 starts the Den's counter, 12
    /// switches the red glow on, 13 starts the red glow's counter.
    pub fn unique_event(
        &mut self,
        id: u8,
        level: u32,
        draw_0x00410a80: impl FnOnce() -> i32,
    ) -> Result<UniqueEventEffect, OverrideError> {
        if id >= 32 {
            return Err(OverrideError::Fatal1D9(id));
        }
        if id >= 20 {
            return Err(OverrideError::Fatal1DA(id));
        }
        self.unique_bits |= 1 << id;
        Ok(match id {
            0 => {
                self.den_counter = 0;
                UniqueEventEffect::DenCounterStarted
            }
            12 => {
                self.glow_flag = true;
                UniqueEventEffect::RedGlowStarted {
                    in_level_108: level == 108,
                }
            }
            13 => {
                self.glow_counter = 0;
                self.glow_value = draw_0x00410a80().wrapping_add(90);
                UniqueEventEffect::RedGlowCounterStarted
            }
            1 | 3 | 6 | 14 | 16 | 17 | 19 => UniqueEventEffect::OtherClientEffect(id),
            _ => UniqueEventEffect::None,
        })
    }
}

/// Placement tries per Den room.
pub const DEN_LIGHT_TRIES: u32 = 25;
/// Den lights placed per room at most.
pub const DEN_LIGHTS_PER_ROOM: usize = 3;
/// The client missile a Den light is (`denofevillight`).
pub const DEN_LIGHT_MISSILE: u32 = 287;

/// The Den's lights in one room: up to 25 tries until 3 are placed. Each
/// try draws x = room x + rnd(w), then y = room y + rnd(h), from the local
/// player unit's seed; the point is taken when `point_test(x, y)` (the
/// collision test for walls and missile barriers) reports nothing. A
/// cramped room may get fewer than 3 lights, or none. Returns the placed
/// points in order; each becomes client missile
/// [`DEN_LIGHT_MISSILE`].
pub fn den_light_points(
    room: (i32, i32, i32, i32),
    seed: &mut Seed,
    mut point_test: impl FnMut(i32, i32) -> u32,
) -> Vec<(i32, i32)> {
    let (rx, ry, w, h) = room;
    let mut placed = Vec::with_capacity(DEN_LIGHTS_PER_ROOM);
    for _ in 0..DEN_LIGHT_TRIES {
        if placed.len() >= DEN_LIGHTS_PER_ROOM {
            break;
        }
        let x = rx.wrapping_add(seed.roll(w) as i32);
        let y = ry.wrapping_add(seed.roll(h) as i32);
        if point_test(x, y) == 0 {
            placed.push((x, y));
        }
    }
    placed
}
