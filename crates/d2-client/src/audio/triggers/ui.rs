//! UI clicks (no unit, delay 0), the S→C 0x5D sound actions
//! and the fixed ids, whose conditions are the owning features'.

use super::Ctx;
use crate::audio::calls::Handle;
use crate::audio::sound_table::volume::ftol;
use crate::bridge::world::UnitKey;

/// UI sound ids (the UI owns which control is which).
pub mod id {
    pub const CURSOR_PASS: i32 = 1;
    pub const CURSOR_SELECT: i32 = 2;
    pub const CURSOR_ERROR: i32 = 3;
    pub const CURSOR_BUTTON_CLICK: i32 = 4;
    pub const CURSOR_POINT_DROP: i32 = 5;
    pub const CURSOR_SWITCH: i32 = 6;
    pub const CURSOR_REPAIR_ITEM: i32 = 15;
    pub const CURSOR_HOSTILE: i32 = 16;
}

/// The UI sound table: (id, name).
pub const UI_SOUNDS: [(i32, &str); 8] = [
    (1, "cursor_pass"),
    (2, "cursor_select"),
    (3, "cursor_error"),
    (4, "cursor_button_click"),
    (5, "cursor_point_drop"),
    (6, "cursor_switch"),
    (15, "cursor_repair_item"),
    (16, "cursor_hostile"),
];

/// A UI sound: request(id, none), delay 0.
pub fn ui_sound(cx: &mut Ctx, id: i32) {
    cx.req(id, None, 0);
}

/// S→C 0x5D sound actions: flags f, code c, value v.
/// First match wins: with f bit 0 set only c =
/// 33 plays (237) and the bit-1 rows are not reached (f = 3, c = 33 plays
/// 237 only); the bit-1 rows need bit 0 clear; the f = 0x10 rows need
/// bits 0 and 1 clear.
pub fn ui_action(cx: &mut Ctx, f: u8, c: u8, v: i16) {
    let id = if f & 1 != 0 {
        if c == 33 {
            237
        } else {
            0
        }
    } else if f & 2 != 0 {
        match c {
            4 => 241,
            8 | 15 | 18 | 22 | 35 => 7,
            32 => 217,
            33 => 243,
            _ => 0,
        }
    } else if f == 0x10 {
        match c {
            10 => {
                cx.req(2456, None, 0);
                2474
            }
            33 => i32::from(v),
            _ => 0,
        }
    } else {
        0
    };
    if id != 0 {
        cx.req(id, None, 0);
    }
}

/// Fixed ids (on a unit or none as the table says; conditions are
/// the owning features').
pub mod fixed {
    pub const IMPACT_STEAL_LIFE: i32 = 396;
    pub const IMPACT_STEAL_MANA: i32 = 397;
    pub const EVENT_THUNDER_1: i32 = 202;
    pub const ANDARIEL_QUAKE_LOOP: i32 = 452;
    pub const PLAYER_TOWNPORTAL_ENTER: i32 = 2231;
    pub const SHRINE_PORTAL: i32 = 2671;
    pub const MONSTER_DIABLO_TAUNT_1: i32 = 4638;
    pub const NECROMANCER_CORPSEEXP_1: i32 = 2458;
    pub const MINION_DEATHS: [i32; 4] = [1308, 1311, 1314, 1317];
    pub const FIREBALL_IMPACT: i32 = 2419;
    pub const DRUID_POD_DEATH: i32 = 790;
    pub const BARBARIAN_LEAP_LAND: i32 = 2517;
    pub const SPIDER_WEB_1: i32 = 1830;
}

// ---------------------------------------------------------------------------
// Fixed-id conditions
// ---------------------------------------------------------------------------

/// The overlay sound (U, overlay o, n) after it created overlay `o` on
/// U: o = 151 → 396 `impact_steal_life`, o = 152 → 397
/// `impact_steal_mana`, on U.
pub fn impact_overlay(cx: &mut Ctx, unit: UnitKey, overlay: i32) {
    match overlay {
        151 => cx.req(fixed::IMPACT_STEAL_LIFE, Some(unit), 0),
        152 => cx.req(fixed::IMPACT_STEAL_MANA, Some(unit), 0),
        _ => 0,
    };
}

/// Volume step of the quake loop per call.
pub const QUAKE_STEP: i32 = 6;

/// The quake loop state: the quake loop request (0 = none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuakeLoop {
    pub handle: Handle,
}

impl QuakeLoop {
    /// The shake level `l = trunc(a × 255 / 20)` clamped to 0–255 (a = the
    /// camera shake amplitude).
    pub fn level(amplitude: f32) -> i32 {
        ftol(amplitude * 255.0 / 20.0).clamp(0, 255)
    }

    /// The quake update (called twice per drawn frame): with no loop request
    /// and l > 0, request 452 with no unit and volume := l; with a
    /// request, its volume moves toward l by at most 6 (a gone request
    /// reads 0); a new volume of 0 stops the request and clears the
    /// state, otherwise volume := the new value. Wall-clock driven by the
    /// shake envelope.
    pub fn update(&mut self, cx: &mut Ctx, amplitude: f32) {
        let l = Self::level(amplitude);
        if self.handle == 0 {
            if l > 0 {
                self.handle = cx.req(fixed::ANDARIEL_QUAKE_LOOP, None, 0);
                cx.s.set_volume(self.handle, l);
            }
            return;
        }
        let w = cx.s.request_volume(self.handle).unwrap_or(0);
        let w = if w < l {
            (w + QUAKE_STEP).min(l)
        } else {
            (w - QUAKE_STEP).max(l)
        };
        if w == 0 {
            cx.s.stop_handle(self.handle);
            self.handle = 0;
        } else {
            cx.s.set_volume(self.handle, w);
        }
    }
}

/// The waypoint panel row choice plays 2,231
/// `player_townportal_enter`, no unit.
pub fn waypoint_row_chosen(cx: &mut Ctx) {
    cx.req(fixed::PLAYER_TOWNPORTAL_ENTER, None, 0);
}

/// The deciphered Scroll of Inifuss panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InifussPanel {
    /// The step counter.
    pub counter: u32,
    /// `GetTickCount` of the last step (ms).
    pub last_ms: u32,
    /// Per symbol, the counter value at its start.
    pub starts: [u32; 5],
}

/// Wall-clock gap of a panel step (ms; strictly more advances).
pub const INIFUSS_STEP_MS: u32 = 50;

impl InifussPanel {
    /// One panel update at wall-clock `now_ms`: the counter advances when
    /// more than 50 ms passed since the last step; on a step, each of the
    /// 5 symbols whose counter − start = 1 plays 2,671 (no unit).
    pub fn update(&mut self, cx: &mut Ctx, now_ms: u32) {
        if now_ms.wrapping_sub(self.last_ms) <= INIFUSS_STEP_MS {
            return;
        }
        self.last_ms = now_ms;
        self.counter = self.counter.wrapping_add(1);
        for i in 0..5 {
            if self.counter.wrapping_sub(self.starts[i]) == 1 {
                cx.req(fixed::SHRINE_PORTAL, None, 0);
            }
        }
    }
}

/// Client missile function 37 (missile 372 `diablo
/// appears`). Returns whether a screen shake starts (frames left 150);
/// at frames left 50 it requests 4,638, no unit.
pub fn diablo_appears(cx: &mut Ctx, frames_left: u32) -> bool {
    if frames_left == 50 {
        cx.req(fixed::MONSTER_DIABLO_TAUNT_1, None, 0);
    }
    frames_left == 150
}
