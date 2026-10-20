//! Unit overlays: the record, create, the per-update advance
//! and the rolls. Plain Rust over an [`OverlayEnv`] seam; the
//! caller owns what the base game reads from the unit (mode, states, flags,
//! monster rows), the overlay table, the cel frame counts and the
//! local-player seed.
//!
//! Not here (other owners): the light of a row with `Radius` ≠ 0,
//! the graphics-record fatal, the call
//! site of the per-unit update, the wall clock of kind 6 (the
//! caller passes `now_ms`) and the create call sites.

use d2_sim::tick::TickRate;

/// Stacking ids: creating one does not remove an earlier record.
pub const STACKING_IDS: [i32; 4] = [140, 141, 142, 158];
/// Kind-8 cycle length in base-rate updates (2 s).
pub const CYCLE_LENGTH: i32 = 50;
/// Default monster height when `overlayHeight` is 0.
pub const MONSTER_DEFAULT_HEIGHT: i32 = 75;
/// Overlay id of kind 7's follow-up.
const KIND7_ID: i32 = 140;
const KIND7_A: i32 = 141;
const KIND7_B: i32 = 142;

/// The record (0xA8 bytes, zeroed at allocation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overlay {
    /// +0x00 kind 0–9.
    pub kind: u8,
    /// +0x04 overlay id.
    pub id: i32,
    /// +0x08 arg A (kind 0 mode; kind 1 countdown; kind 2 / 6 running flag;
    /// kind 4 / 9 first follow-up; kind 8 cycle counter).
    pub a: i32,
    /// +0x0C arg B.
    pub b: i32,
    /// +0x10 arg C (kind 9 state).
    pub c: i32,
    /// +0x14 rate, 1/256 frame per update.
    pub rate: i32,
    /// +0x18 frame position, 1/256 frame.
    pub frame: i32,
    /// +0x1C frame count × 256.
    pub frames: i32,
    /// +0x20, +0x24 draw offsets.
    pub x: i32,
    pub y: i32,
    /// +0x28 `LoopWaitTime` (ms).
    pub loop_wait: i32,
    /// +0x2C update stamp.
    pub stamp: u32,
    /// +0x30 kind 6 wait start (ms).
    pub wait_start: u32,
    /// +0x34 `PreDraw` (back when set).
    pub pre_draw: bool,
    /// +0x38 kind 8 state.
    pub state: i32,
    /// +0x3C kind 8 active flag.
    pub active: bool,
    /// +0x40 light handle (0 = none; set by the lighting owner).
    pub light: u32,
    /// D2RUST's own: the creation order ([`next_seq`]), shared with the
    /// effect layer's own overlays so that one unit's overlays of both
    /// kinds draw newest first, as the base game's one list (head push).
    pub seq: u64,
}

/// The next creation number of an overlay (records and the effect
/// layer's cast overlays alike): later creations get larger numbers.
pub fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Overlay {
    /// The drawn frame `f = +0x18 >> 8`.
    pub fn frame_index(&self) -> i32 {
        self.frame >> 8
    }
}

/// The `overlay.txt` columns the engine reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlayRow {
    /// `version` (+0x42, signed 16-bit).
    pub version: i16,
    pub pre_draw: bool,
    pub xoffset: i32,
    pub yoffset: i32,
    /// `Height1`–`Height4`.
    pub height: [i32; 4],
    pub anim_rate: i32,
    pub init_radius: i32,
    pub radius: i32,
    pub loop_wait_time: i32,
}

/// What kind of unit the overlay is on, for the height choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightUnit {
    Player,
    /// A monster; `overlay_height` of its `monstats2` row, `None` = no row.
    Monster {
        overlay_height: Option<u8>,
    },
    /// Any other type, or no unit.
    Other,
}

/// What create and update read from the world.
pub trait OverlayEnv {
    fn expansion(&self) -> bool;
    /// The `overlay.txt` row count (+0xBC0).
    fn row_count(&self) -> i32;
    fn row(&self, id: i32) -> Option<OverlayRow>;
    /// U's flag-ex word has bit 0x40000.
    fn unit_blocks_overlays(&self) -> bool;
    /// U is a monster whose `monstats2` row has `noOvly`.
    fn unit_no_ovly(&self) -> bool;
    fn height_unit(&self) -> HeightUnit;
    /// Frame count of the overlay cel file for U's direction byte; `None`
    /// when the file does not resolve or load.
    fn frame_count(&self, id: i32) -> Option<i32>;
    /// on the local player's client unit seed.
    fn roll(&mut self, n: i32) -> i32;
    /// U's mode (after the draw identity substitution).
    fn mode(&self) -> i32;
    /// U has state s.
    fn has_state(&self, s: i32) -> bool;
    /// The state set with (U, s, on).
    fn set_state(&mut self, s: i32, on: bool);
    /// U has any state with the `aura` flag.
    fn has_aura(&self) -> bool;
}

/// Fatal conditions of the base game, as errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayError {
    /// 0xCA9: remove by an invalid id.
    InvalidId(i32),
    /// 0x205: a record with the id has another kind.
    WrongKind(i32),
}

/// A unit's overlay list: first element = list head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayList {
    pub records: Vec<Overlay>,
}

impl OverlayList {
    pub fn new() -> Self {
        Self::default()
    }

    /// The overlay create. Arguments in the stack order: kind, a, A, B, C, b.
    /// Returns whether a record was created. The light (step 9) and the
    /// graphics-record check (step 10) belong to the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        env: &mut dyn OverlayEnv,
        id: i32,
        kind: u8,
        a: i32,
        arg_a: i32,
        arg_b: i32,
        arg_c: i32,
        b: i32,
    ) -> bool {
        // Step 1.
        if id < 0 || id >= env.row_count() {
            return false;
        }
        let Some(row) = env.row(id) else {
            return false;
        };
        // Step 2.
        if !env.expansion() && row.version >= 100 {
            return false;
        }
        // Step 3.
        if env.unit_blocks_overlays() || env.unit_no_ovly() {
            return false;
        }
        // Step 4.
        if kind == 8 && self.records.iter().any(|r| r.id == id) {
            return false;
        }
        // Step 5.
        if !STACKING_IDS.contains(&id) {
            // The id was validated above.
            let _ = self.remove_by_id(env, id);
        }
        // Step 6.
        let mut rec = Overlay {
            kind,
            id,
            frames: env.frame_count(id).map_or(1, |n| n * 256),
            seq: next_seq(),
            ..Overlay::default()
        };
        // Step 7.
        match kind {
            2 => rec.a = 1,
            6 => rec.frame = env.roll(rec.frames),
            _ => {
                rec.a = arg_a;
                rec.b = arg_b;
                rec.c = arg_c;
            }
        }
        // Step 8.
        rec.x = row.xoffset;
        rec.y = row.yoffset + height_added(env.height_unit(), &row);
        // Step 10 (the list part): push at the head, with PreDraw.
        rec.pre_draw = row.pre_draw;
        self.records.insert(0, rec);
        // Step 11.
        if kind == 8 {
            self.records[0].state = arg_a;
            self.sync_kind8();
        }
        // Step 12.
        if a != 0 {
            self.records[0].frame = env.roll(a * 256);
        }
        // Step 13.
        self.records[0].rate = row.anim_rate * 16;
        if b != 0 {
            self.records[0].rate += env.roll(b * 16);
        }
        // Step 14.
        self.records[0].loop_wait = row.loop_wait_time;
        true
    }

    /// The kind-8 sync of the head record.
    fn sync_kind8(&mut self) {
        let state = self.records[0].state;
        let mut found = None;
        for r in self.records.iter().skip(1).filter(|r| r.kind == 8) {
            if r.state == state {
                found = Some((r.active, r.a));
                break;
            }
            if r.active {
                found = Some((false, self.records[0].a));
                break;
            }
        }
        let (active, a) = found.unwrap_or((true, 0));
        self.records[0].active = active;
        self.records[0].a = a;
    }

    /// Remove the record at `i`: the light is
    /// freed by the caller through `Overlay::light`; a kind-8 record runs
    /// the cycle step for its state.
    pub fn remove_at(&mut self, env: &mut dyn OverlayEnv, i: usize) -> Option<Overlay> {
        if i >= self.records.len() {
            return None;
        }
        let rec = self.records.remove(i);
        if rec.kind == 8 {
            self.cycle_step(env, rec.state, None);
        }
        Some(rec)
    }

    /// Remove by id: the first record in list
    /// order with `id`, or of kind 4 / 9 with A or B = id.
    pub fn remove_by_id(
        &mut self,
        env: &mut dyn OverlayEnv,
        id: i32,
    ) -> Result<bool, OverlayError> {
        if id < 0 || id >= env.row_count() {
            return Err(OverlayError::InvalidId(id));
        }
        let pos = self
            .records
            .iter()
            .position(|r| r.id == id || (matches!(r.kind, 4 | 9) && (r.a == id || r.b == id)));
        match pos {
            Some(i) => {
                self.remove_at(env, i);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Remove all: every record in list order.
    pub fn remove_all(&mut self, env: &mut dyn OverlayEnv) {
        while !self.records.is_empty() {
            self.remove_at(env, 0);
        }
    }

    /// Set A of every kind-1 record with `id`.
    pub fn set_countdown(&mut self, id: i32, n: i32) -> Result<(), OverlayError> {
        for r in self.records.iter_mut().filter(|r| r.id == id) {
            if r.kind != 1 {
                return Err(OverlayError::WrongKind(id));
            }
            r.a = n;
        }
        Ok(())
    }

    /// The kind-8 cycle step for state `s`; `this` is the index
    /// of the record that reached the end of its cycle (`None` for a
    /// removed record).
    fn cycle_step(&mut self, env: &mut dyn OverlayEnv, s: i32, this: Option<usize>) {
        if env.has_state(s) {
            env.set_state(s, false);
            if !env.has_aura() {
                if let Some(i) = this {
                    self.records[i].a = 0;
                }
            }
            env.set_state(s, true);
        }
        for r in self
            .records
            .iter_mut()
            .filter(|r| r.kind == 8 && r.state == s)
        {
            r.active = false;
            r.a = -1;
        }
        // State 0 cannot be chosen: 0 means none.
        let states = self
            .records
            .iter()
            .filter(|r| r.kind == 8 && r.state != 0)
            .map(|r| r.state);
        let next = states
            .clone()
            .filter(|&t| t > s)
            .min()
            .or_else(|| states.min());
        if let Some(n) = next {
            for r in self
                .records
                .iter_mut()
                .filter(|r| r.kind == 8 && r.state == n)
            {
                r.active = true;
                r.a = 0;
            }
        }
    }

    /// The per-update advance. `update` is the client update
    /// number (stamps compare with it; edge case 1), `now_ms` the wall clock
    /// of kind 6 (the caller's choice), `rate` the game's tick rate: the
    /// row rates and the kind-8 cycle are stated per base-rate tick, so a
    /// record plays the same real time at every rate.
    pub fn advance(&mut self, env: &mut dyn OverlayEnv, update: u32, now_ms: u32, rate: TickRate) {
        loop {
            let Some(i) = self.records.iter().position(|r| r.stamp != update) else {
                return;
            };
            self.records[i].stamp = update;
            self.run_one(env, i, now_ms, (rate, update));
        }
    }

    fn run_one(
        &mut self,
        env: &mut dyn OverlayEnv,
        i: usize,
        now_ms: u32,
        (rate, update): (TickRate, u32),
    ) {
        let kind = self.records[i].kind;
        // Step 3: keep test.
        match kind {
            0 => {
                if env.mode() != self.records[i].a {
                    self.remove_at(env, i);
                    return;
                }
            }
            1 => {
                if self.records[i].a != 0 {
                    self.records[i].a -= 1;
                } else {
                    self.remove_at(env, i);
                    return;
                }
            }
            2 => {
                if self.records[i].a == 0 {
                    self.remove_at(env, i);
                    return;
                }
            }
            6 => {
                if self.records[i].a == 0 {
                    let waited = now_ms.wrapping_sub(self.records[i].wait_start);
                    if i64::from(waited) < i64::from(self.records[i].loop_wait) {
                        return;
                    }
                    self.records[i].a = 1;
                }
            }
            8 => {
                if !env.has_state(self.records[i].state) {
                    self.remove_at(env, i);
                    return;
                }
                if self.records[i].active {
                    if self.records[i].a < rate.ticks(CYCLE_LENGTH) {
                        self.records[i].a += 1;
                    } else {
                        let s = self.records[i].state;
                        self.cycle_step(env, s, Some(i));
                    }
                }
            }
            _ => {}
        }
        // Step 4: advance.
        self.records[i].frame += rate.share(self.records[i].rate, update as i32);
        // Step 5: end.
        let rec = self.records[i];
        if rec.frame < rec.frames {
            return;
        }
        match kind {
            1 | 3 | 8 => self.records[i].frame -= rec.frames,
            0 => {}
            2 => self.records[i].a = 0,
            4 => {
                self.remove_at(env, i);
                for id in [rec.a, rec.b] {
                    if id != -1 {
                        self.create(env, id, 3, 0, 0, 0, 0, 0);
                    }
                }
            }
            5 => {
                let r = &mut self.records[i];
                r.rate = 0;
                r.frame = r.frames - 1;
                r.a = 1;
            }
            6 => {
                let r = &mut self.records[i];
                r.frame -= r.frames;
                r.a = 0;
                r.wait_start = now_ms;
            }
            7 => {
                self.remove_at(env, i);
                self.create(env, KIND7_ID, 4, 0, KIND7_A, KIND7_B, 0, 0);
            }
            9 => {
                self.remove_at(env, i);
                if env.has_state(rec.c) {
                    for id in [rec.a, rec.b] {
                        if id != -1 {
                            self.create(env, id, 8, 0, rec.c, 0, 0, 0);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;

/// The height chosen by U.
fn height_added(u: HeightUnit, row: &OverlayRow) -> i32 {
    match u {
        HeightUnit::Player => row.height[1],
        HeightUnit::Monster { overlay_height } => match overlay_height {
            None => row.height[0],
            Some(0) => MONSTER_DEFAULT_HEIGHT,
            Some(h @ 1..=4) => row.height[h as usize - 1],
            Some(_) => 0,
        },
        HeightUnit::Other => row.height[0],
    }
}

/// The light an overlay row asks for: `Radius`
/// (+0x74) ≠ 0 → one light; when `Radius` ≠ `InitRadius` (+0x70) its
/// target radius is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayLight {
    pub init_radius: i32,
    pub target_radius: Option<i32>,
}

/// The light of a new overlay record, if any.
pub fn overlay_light(row: &OverlayRow) -> Option<OverlayLight> {
    if row.radius == 0 {
        return None;
    }
    Some(OverlayLight {
        init_radius: row.init_radius,
        target_radius: (row.radius != row.init_radius).then_some(row.radius),
    })
}

/// The overlay draw split: whether the overlay draw
/// draws `rec` in the back call (`back`, records with `PreDraw` ≠ 0) or the
/// front call. Skipped: kind 6 with +0x08 = 0, kind 8 with +0x3C = 0;
/// drawn only while `f < +0x1C >> 8`.
pub fn overlay_draws(rec: &Overlay, back: bool) -> bool {
    if rec.pre_draw != back {
        return false;
    }
    if (rec.kind == 6 && rec.a == 0) || (rec.kind == 8 && !rec.active) {
        return false;
    }
    rec.frame_index() < (rec.frames >> 8)
}

/// The wall clock of kind 6: the real time of the client updates since
/// game start at the game's rate (40 ms each at the base rate), as the
/// camera does for the shake.
pub fn kind6_clock_ms(updates_since_start: u32, rate: TickRate) -> u32 {
    rate.ms_for_ticks(updates_since_start) as u32
}

#[cfg(test)]
#[path = "overlay_c2ui_tests.rs"]
mod tests_c2ui;
