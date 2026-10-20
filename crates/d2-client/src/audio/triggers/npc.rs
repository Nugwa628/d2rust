//! NPC greetings, dialog lines and the NPC Speech option.

use std::collections::BTreeMap;

use super::tables::{NpcGreetings, NpcSpeech};
use super::{detach_skill_voices, Ctx, Unit};
use crate::audio::calls::{Handle, FLAG_EXACT};
use crate::bridge::world::UnitKey;

/// One NPC greeting record (6 dwords: greet, inactive, time, return,
/// last, tick). The records are `npc-greetings.tsv`, held at run time by
/// [`GreetingRecords`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NpcGreeting {
    pub greet: i32,
    pub inactive: i32,
    pub time: i32,
    pub ret: i32,
    /// Last pick.
    pub last: i32,
    /// C of the last pick.
    pub tick: u32,
}

/// The greeting records of `npc-greetings.tsv` with their run
/// time `last` / `tick`: one per record address, so classes that share a
/// record (e.g. the six Cain classes) share them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GreetingRecords {
    by_class: BTreeMap<i32, u32>,
    records: BTreeMap<u32, NpcGreeting>,
}

impl GreetingRecords {
    pub fn new(table: &NpcGreetings) -> Self {
        let mut by_class = BTreeMap::new();
        let mut records = BTreeMap::new();
        for r in table.rows() {
            by_class.insert(r.class, r.record);
            records.entry(r.record).or_insert(NpcGreeting {
                greet: r.greet,
                inactive: r.inactive,
                time: r.time,
                ret: r.ret,
                last: 0,
                tick: 0,
            });
        }
        GreetingRecords { by_class, records }
    }

    /// The records of the spec table.
    pub fn spec() -> Self {
        Self::new(NpcGreetings::spec())
    }

    /// The record of NPC class `class`; `None` = no record
    /// (the greeting is then id 0).
    pub fn for_class(&mut self, class: i32) -> Option<&mut NpcGreeting> {
        let addr = *self.by_class.get(&class)?;
        self.records.get_mut(&addr)
    }

    /// Distinct records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Greeting attempts.
pub const GREET_ATTEMPTS: u32 = 20;

/// The greeting mode argument of the greeting pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetMode {
    /// Mode 0: may pick the inactive line.
    Idle,
    /// Mode 1 (event 18).
    Event,
    /// Mode 2: the return line.
    Return,
}

/// Greeting pick (N, mode). Returns the id the
/// caller requests with flags 1 (exact). Mode 2 returns `return`
/// before the attempt loop and writes neither `last` nor `tick`; modes 0
/// and 1 set them after the last attempt.
pub fn greet(cx: &mut Ctx, g: &mut NpcGreeting, mode: GreetMode, day_phase: u8) -> i32 {
    if mode == GreetMode::Return {
        return g.ret;
    }
    let mut pick = 0;
    for _ in 0..GREET_ATTEMPTS {
        // 1.
        let mut s = g.greet;
        if mode == GreetMode::Idle && g.inactive != 0 && cx.s.roll(2) == 0 {
            s = g.inactive;
        }
        // 2.
        if g.time != 0 && (s == 0 || cx.s.roll(3) == 0) {
            s = g.time
                + match day_phase {
                    1 => 0,
                    2 | 3 => 1,
                    _ => 2,
                };
        }
        // 3, 4.
        pick = cx.s.variant(s);
        if pick != g.last {
            break;
        }
    }
    g.last = pick;
    g.tick = cx.c;
    pick
}

/// Greeting from NPC interaction: detach N's
/// skill voices with force, pick, request on P with flags 1. Returns the
/// handle (the interaction keeps it).
pub fn interact_greeting(
    cx: &mut Ctx,
    g: &mut NpcGreeting,
    npc: &Unit,
    local: Option<UnitKey>,
    mode: GreetMode,
    day_phase: u8,
) -> Handle {
    detach_skill_voices(cx.s, npc);
    let pick = greet(cx, g, mode, day_phase);
    cx.s.request(pick, local, 0, FLAG_EXACT, 0)
}

/// Dialog line state: NPC speech enabled and the remembered line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogState {
    /// NPC speech enabled (starts 1).
    pub enabled: bool,
    pub handle: Handle,
    pub id: i32,
}

impl Default for DialogState {
    fn default() -> Self {
        DialogState {
            enabled: true,
            handle: 0,
            id: 0,
        }
    }
}

/// Dialog line delay (T) and the fade of the previous line.
pub const DIALOG_DELAY: u32 = 5;
pub const DIALOG_FADE: u32 = 4;

/// Dialog line (N, key). "Playing" for the previous
/// line is read as [`SoundCalls::is_active`](crate::audio::calls::SoundCalls::is_active).
pub fn dialog_line(
    cx: &mut Ctx,
    st: &mut DialogState,
    table: &NpcSpeech,
    npc: &Unit,
    local: Option<UnitKey>,
    key: i32,
) {
    if st.handle != 0 && cx.s.is_active(st.handle) {
        cx.s.fade(st.handle, 0, 0, DIALOG_FADE);
    }
    cx.s.stop_speech();
    if !st.enabled {
        return;
    }
    let s = table.sound(key);
    if s != 0 {
        detach_skill_voices(cx.s, npc);
        st.handle = cx.req(s, local, DIALOG_DELAY);
        st.id = s;
    }
}

/// The dialog text closed before its line ended (a skip, Esc, the
/// interaction's end): the line stops with it. A line already playing on
/// `local` fades over [`DIALOG_FADE`]; one still waiting for its start
/// delay is stopped so it never starts. The remembered handle is cleared.
pub fn dialog_close(cx: &mut Ctx, st: &mut DialogState, local: Option<UnitKey>) {
    let h = std::mem::take(&mut st.handle);
    if h == 0 || !cx.s.is_active(h) {
        return;
    }
    if local.is_some_and(|p| cx.s.speaking(p)) {
        cx.s.fade(h, 0, 0, DIALOG_FADE);
    } else {
        cx.s.stop_handle(h);
    }
}

/// NPC Speech option: 0 → enabled (text off),
/// 1 → disabled (text on), 2 → both on. Returns whether text is shown.
pub fn set_npc_speech_option(st: &mut DialogState, value: u8) -> bool {
    st.enabled = value != 1;
    value != 0
}

/// Fixed NPC lines (their conditions belong to the calling features).
pub const WUSSIE_CHEER: i32 = 4603;
pub const WUSSIE_HELP_ME: i32 = 4607;
pub const WUSSIE_CLASS: i32 = 534;
pub const GUARD_HALT: i32 = 3983;
pub const NIHLATHAK_HURRYUP: i32 = 4560;
