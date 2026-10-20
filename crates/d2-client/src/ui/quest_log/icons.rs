//! Icon frames and the just-completed animation, and the client quest
//! check of the level-entry lines.

use super::tables::{chain_entry, ENTRY_COUNT};
use super::{derive_row, QuestFlags, RowCtx};
use super::{IconState, Row};

/// UI sound of the completion animation.
pub const COMPLETE_SOUND: u8 = 14;
/// Milliseconds between counter steps ("more than 100 ms").
pub const STEP_MS: u32 = 100;
/// Counter value at which the animation ends.
pub const END_COUNTER: u32 = 25;
/// The completed frame.
pub const COMPLETED_FRAME: u32 = 24;

/// What an animation step asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconEffect {
    /// Frame to draw.
    pub frame: u32,
    /// Play UI sound 14 (counter reached 1, expansion installed only).
    pub sound: bool,
    /// Set P.12 of the quest and send C→S 0x58 with this quest id.
    pub acknowledge: Option<u8>,
}

/// Per-row animation state: the counter and the stamp.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IconAnim {
    pub counter: u32,
    pub stamp: u32,
}

impl IconAnim {
    /// One draw of an icon in state 0:
    /// the frame and the acknowledge come from the counter as it is
    /// before this draw's step; then a zero stamp takes `now_ms`; then
    /// more than 100 ms since the stamp restamps and steps the counter
    /// (sound at counter 1, expansion installed only).
    pub fn step(&mut self, q: u8, now_ms: u32, expansion_installed: bool) -> IconEffect {
        let (frame, acknowledge) = if self.counter >= END_COUNTER {
            (COMPLETED_FRAME, Some(q))
        } else {
            (self.counter, None)
        };
        if self.stamp == 0 {
            self.stamp = now_ms;
        }
        let mut sound = false;
        if now_ms.wrapping_sub(self.stamp) > STEP_MS {
            self.stamp = now_ms;
            self.counter += 1;
            sound = self.counter == 1 && expansion_installed;
        }
        IconEffect {
            frame,
            sound,
            acknowledge,
        }
    }
}

/// The set-and-send at once for every row in state 0.
pub fn acknowledge_all(rows: &[Row], p: &mut QuestFlags) -> Vec<u8> {
    let mut sent = Vec::new();
    for r in rows {
        if r.icon == IconState::JustCompleted {
            p.set_bit(r.quest, 12);
            sent.push(r.quest);
        }
    }
    sent
}

/// Which cel an icon frame is taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconCel {
    /// The quest's own icon cel (`a1q1`…).
    Icon,
    /// `questdone`.
    QuestDone,
}

/// The icon draw of a row: cel and frame, `None` when nothing is drawn
/// (the animation frame comes from [`IconAnim`] for state 0).
pub fn icon_frame(
    state: IconState,
    pressed: bool,
    icon_index: u8,
    anim_frame: u32,
) -> (IconCel, u32) {
    match state {
        IconState::JustCompleted => (IconCel::Icon, anim_frame.min(COMPLETED_FRAME)),
        IconState::Completed => {
            if pressed {
                (IconCel::QuestDone, u32::from(icon_index))
            } else {
                (IconCel::Icon, 24)
            }
        }
        IconState::NotAvailable => (IconCel::Icon, 26),
        IconState::InProgress => (IconCel::Icon, if pressed { 25 } else { 0 }),
    }
}

/// State 2 draws no title or text.
pub fn draws_title_and_text(state: IconState) -> bool {
    state != IconState::NotAvailable
}

/// The `questsockets` selection frame drawn over every shown row.
pub fn selection_frame(selected: bool) -> u32 {
    u32::from(selected)
}

/// The Seven Tombs symbol (`invps`): frame Y (0–6, else 0) at
/// (x + 0x6E, y − 0x6E + screen height) when a row has title 928, shown
/// status 7 and the selected slot is 5.
pub fn tomb_symbol(
    row: &Row,
    selected_slot: u8,
    tomb: i32,
    x: i32,
    y: i32,
    screen_height: i32,
) -> Option<(u32, i32, i32)> {
    if row.title != 928 || row.shown != 7 || selected_slot != 5 {
        return None;
    }
    let frame = if (0..=6).contains(&tomb) {
        tomb as u32
    } else {
        0
    };
    Some((frame, x + 0x6E, y - 0x6E + screen_height))
}

/// Why a quest check is fatal in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestCheckError {
    /// Before any S→C 0x5E (error string 0x60).
    NoAvailability,
    /// c ≥ 37 (error string 0x65).
    OutOfRange,
}

/// Inputs of the client quest check.
#[derive(Debug, Clone, Copy)]
pub struct QuestCheckCtx<'a> {
    /// The last S→C 0x5E bytes `A[0..36]` (`None` before any 0x5E).
    pub availability: Option<&'a [u8; 37]>,
    pub p: Option<&'a QuestFlags>,
    pub g: Option<&'a QuestFlags>,
    pub multiplayer: bool,
    pub den: i32,
    pub barbarians: i32,
}

/// 1 = the quest is open and not done for this
/// player. `status` and `last` are the log's `S` and `last` (Den of Evil's
/// row rewrites `last[1]`).
pub fn quest_check(
    c: u8,
    ctx: &QuestCheckCtx<'_>,
    status: &[u8; ENTRY_COUNT],
    last: &mut [u8; ENTRY_COUNT],
) -> Result<bool, QuestCheckError> {
    // Rule 1: the 0x5E byte.
    let Some(avail) = ctx.availability else {
        return Err(QuestCheckError::NoAvailability);
    };
    if c >= 37 {
        return Err(QuestCheckError::OutOfRange);
    }
    if avail[c as usize] == 0 {
        return Ok(false);
    }
    // Rule 2: the first entry with this chain.
    let Some(q) = chain_entry(c) else {
        return Ok(false);
    };
    if q == 42 {
        return Ok(false);
    }
    // Rule 3: G received, G.13 clear.
    let Some(g) = ctx.g else { return Ok(false) };
    if g.bit(q, 13) {
        return Ok(false);
    }
    // Rule 4: P received, P.1, P.0, P.14 clear.
    let Some(p) = ctx.p else { return Ok(false) };
    if p.bit(q, 1) || p.bit(q, 0) || p.bit(q, 14) {
        return Ok(false);
    }
    // Rule 5: Den of Evil only.
    if c == 1 {
        let rc = RowCtx {
            p,
            g: ctx.g,
            multiplayer: ctx.multiplayer,
            den: ctx.den,
            barbarians: ctx.barbarians,
        };
        let row = derive_row(q, status[q as usize], &rc, &mut last[q as usize]);
        if row.shown >= 5 {
            return Ok(false);
        }
    }
    // Rule 6.
    Ok(true)
}
