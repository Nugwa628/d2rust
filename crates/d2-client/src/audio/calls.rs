//! The call surface the trigger rules and the
//! environment state machines use to reach the
//! sound layer. The rules only say *when* and
//! *with which arguments* a call is made; what the call does is the sound
//! table's (implemented by [`super::sound_table`]). Keeping the calls behind
//! a trait lets each rule set be tested against a fake that logs the calls.

use crate::bridge::world::UnitKey;

/// A request handle (from a pre-incremented counter). `0` means "no
/// request".
pub type Handle = u32;

/// Request flag bit 0: exact id, no variant pick.
pub const FLAG_EXACT: u32 = 1;
/// Request flag bit 1: no fade-in.
pub const FLAG_NO_FADE_IN: u32 = 2;

/// The sound layer as the rule functions call it, one method per sound
/// table entry point.
pub trait SoundCalls {
    /// The game rate; request and fade durations are stated in base ticks.
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }

    /// A new request (id, unit, delay, flags, offset). `delay` is in sound
    /// base-rate ticks (40 ms each). Returns
    /// the handle, or 0 when no request is made.
    fn request(
        &mut self,
        id: i32,
        unit: Option<UnitKey>,
        delay: u32,
        flags: u32,
        offset: u32,
    ) -> Handle;

    /// Volume := v if
    /// the handle exists.
    fn set_volume(&mut self, h: Handle, v: i32);

    /// Fades `h` toward `target` after `delay` over `len`.
    fn fade(&mut self, h: Handle, target: i32, delay: u32, len: u32);

    /// Detaches `h` from `unit` (`force` ends it too).
    fn detach(&mut self, h: Handle, unit: UnitKey, force: bool);

    /// Stops request `h`.
    fn stop_handle(&mut self, h: Handle);
    /// Stops every request of that id.
    fn stop_id(&mut self, id: i32);
    /// Stops every request in the song range 4,657–4,684.
    fn stop_songs(&mut self);
    /// Stops ids 52–71 except group bases `a`, `b`.
    fn stop_52_71_except(&mut self, a: i32, b: i32);
    /// Stops ids 72–201 except group bases `a`, `b`.
    fn stop_72_201_except(&mut self, a: i32, b: i32);
    /// Stops ids 2,934–4,656 (speech).
    fn stop_speech(&mut self);

    /// `speaking(U)`.
    fn speaking(&self, unit: UnitKey) -> bool;
    /// `any_speech`.
    fn any_speech(&self) -> bool;
    /// Whether a request with this handle exists and has not ended.
    fn is_active(&self, h: Handle) -> bool;

    /// The sound tick T.
    fn sound_tick(&self) -> u32;

    /// `roll(n)` on the local player's client unit seed.
    fn roll(&mut self, n: i32) -> u32;
}

/// `uniform(lo, hi)` = lo + roll(hi − lo + 1).
pub fn uniform(s: &mut dyn SoundCalls, lo: i32, hi: i32) -> i32 {
    lo.wrapping_add(s.roll(hi.wrapping_sub(lo).wrapping_add(1)) as i32)
}

/// `jitter(r)` = roll(2r + 1) − r.
pub fn jitter(s: &mut dyn SoundCalls, r: i32) -> i32 {
    (s.roll(r.wrapping_mul(2).wrapping_add(1)) as i32).wrapping_sub(r)
}
