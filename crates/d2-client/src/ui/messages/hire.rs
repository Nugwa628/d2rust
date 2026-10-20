//! The hire popup (0x50 code 2) and the other 0x50 codes: the
//! Inifuss scroll (code 4).

use super::msg_u32s;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::PanelOutput;

/// UI state ids: the hireling icon (expansion only) and the hireling inventory.
pub const UI_HIRELING_ICON: u8 = 0x23;
pub const UI_HIRELING_INV: u8 = 0x24;

/// The hire popup flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HirePopup {
    /// The popup state (2 holds the portrait pass off).
    pub state: u8,
    /// The hireling panel is to pop up.
    pub popup: bool,
}

/// What the portrait pass needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortraitEnv {
    /// Open mode 2 or 3.
    pub open_mode_2_or_3: bool,
    /// State 9 or 0x0B open.
    pub state_9_or_b: bool,
    /// State 0x24 open.
    pub state_24_open: bool,
}

impl HirePopup {
    /// 0x50 code 2: state:= 0; `SetUIState(0x23, on,
    /// 0)` (expansion only); the registry value `PopupHireling` read (missing =
    /// 0): 0 → popup:= 1. The hire-table entry it finds is
    /// never read.
    pub fn open(&mut self, popup_hireling: Option<u32>) -> PanelOutput {
        self.state = 0;
        if popup_hireling.unwrap_or(0) == 0 {
            self.popup = true;
        }
        PanelOutput::SetUi {
            ui: UI_HIRELING_ICON,
            mode: 0,
            jump: false,
        }
    }

    /// The portrait pass (UI pass 0x13): not in open modes 2 /
    /// 3, not with state 9 or 0x0B open, not with state = 2. With the
    /// popup flag set and state 0x24 closed, `SetUIState(0x24, on, 0)`
    /// (`set_ui` runs it and says if it succeeded); on success the registry
    /// `PopupHireling`:= 1 (returned) and the flag is cleared.
    pub fn portrait_pass(
        &mut self,
        env: PortraitEnv,
        set_ui: &mut dyn FnMut(u8, u8) -> bool,
    ) -> Option<u32> {
        if env.open_mode_2_or_3 || env.state_9_or_b || self.state == 2 {
            return None;
        }
        if self.popup && !env.state_24_open && set_ui(UI_HIRELING_INV, 0) {
            self.popup = false;
            return Some(1);
        }
        None
    }
}

/// Failure of a 0x50 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScrollError {
    /// A symbol of 5 is fatal 0x1673.
    #[error("stone symbol 5 (fatal 0x1673)")]
    Symbol5,
}

/// The item code of the Inifuss scroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollCode {
    /// `bks `: the plain scroll, no stones.
    Bks,
    /// `bkd `: the scroll with stones once opened.
    Bkd,
    /// `tr1 `: the plain scroll.
    Tr1,
}

/// What the scroll draw shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDraw {
    Plain,
    /// The plain scroll, then the stones when `a`.
    WithStones(bool),
}

/// Stone start counters.
pub const STONE_START: [u32; 5] = [0, 12, 24, 36, 48];
/// Scroin2 positions by symbol.
pub const SCROIN2_POS: [(i32, i32); 5] = [(242, 104), (255, 222), (148, 310), (47, 222), (75, 104)];
/// Scroin3 positions by symbol.
pub const SCROIN3_POS: [(i32, i32); 5] =
    [(303, 161), (322, 242), (254, 290), (190, 238), (211, 162)];
/// UI sound 2671 `shrine_portal`.
pub const SOUND_STONE: i32 = 2671;
/// Animation step: > 50 ms of `GetTickCount`.
pub const STEP_MS: u32 = 50;

/// The animation counter with its time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoneAnim {
    pub counter: u32,
    pub time: u32,
}

impl StoneAnim {
    /// Steps once per > 50 ms; reset to 0 when the stored time is 0, and
    /// the reset stores `now` as the time.
    pub fn step(&mut self, now: u32) {
        if self.time == 0 {
            self.counter = 0;
            self.time = now;
        } else if now.wrapping_sub(self.time) > STEP_MS {
            self.counter += 1;
            self.time = now;
        }
    }
}

/// One drawn stone: the `scroin2` icon, then the `scroin3`
/// glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoneDraw {
    pub stone: usize,
    /// `scroin2` frame i at `icon_at`, light 0xFF, mode `mode`.
    pub icon_frame: u32,
    pub icon_at: (i32, i32),
    pub mode: u8,
    /// `scroin3` frame at `glyph_at`, mode 3.
    pub glyph_frame: u32,
    pub glyph_at: (i32, i32),
    /// UI sound requested at f = 1.
    pub sound: Option<i32>,
}

/// The Inifuss scroll state: the 12 stone bytes and the opened flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InifussScroll {
    pub bytes: [u8; 12],
    /// The `bkd ` scroll was opened.
    pub opened: bool,
    pub anim: StoneAnim,
}

impl InifussScroll {
    /// The symbol of stone `i`: u16 at byte 2i.
    pub fn symbol(&self, i: usize) -> u16 {
        u16::from_le_bytes([self.bytes[2 * i], self.bytes[2 * i + 1]])
    }

    /// Code 4 writes the 12 bytes and clears.
    pub fn code4(&mut self, bytes: [u8; 12]) {
        self.bytes = bytes;
        self.opened = false;
    }

    /// The scroll draw by item code: `bks ` →
    /// the stones; `bkd ` only while
    /// not opened, else the plain scroll; `tr1 ` → plain.
    pub fn draw_kind(&self, code: ScrollCode) -> ScrollDraw {
        match code {
            ScrollCode::Bks => ScrollDraw::WithStones(false),
            ScrollCode::Bkd if !self.opened => ScrollDraw::WithStones(true),
            ScrollCode::Bkd | ScrollCode::Tr1 => ScrollDraw::Plain,
        }
    }

    /// Opening `bkd `: opened:= 1 and C→S 0x3E with the item GUID. So
    /// the stones appear when code 4 answers.
    pub fn open_bkd(&mut self, item_guid: u32) -> ClientIntent {
        self.opened = true;
        msg_u32s(0x3E, &[item_guid])
    }

    /// The stones for the animation counter: stone i with
    /// symbol s (s ≥ 6 skipped, s = 5 fatal) starts when the counter is above
    /// the stone's start time [i] (f = counter − start ≥ 1); draw mode 0 for f < 5, 1 for
    /// f < 10, 2 for f < 15, else 5. The symbol is read only for a started
    /// stone.
    pub fn stones(&self, counter: u32, sx: i32, sy: i32) -> Result<Vec<StoneDraw>, ScrollError> {
        let mut out = Vec::new();
        for (i, &start) in STONE_START.iter().enumerate() {
            // The symbol is read only for a started stone.
            let Some(f) = counter.checked_sub(start).filter(|&f| f >= 1) else {
                continue;
            };
            let s = usize::from(self.symbol(i));
            if s >= 6 {
                continue;
            }
            if s == 5 {
                return Err(ScrollError::Symbol5);
            }
            let mode = match f {
                0..=4 => 0,
                5..=9 => 1,
                10..=14 => 2,
                _ => 5,
            };
            let (ix, iy) = SCROIN2_POS[s];
            let (gx, gy) = SCROIN3_POS[s];
            out.push(StoneDraw {
                stone: i,
                icon_frame: i as u32,
                icon_at: (ix + sx, iy - sy),
                mode,
                glyph_frame: if f < 21 { f } else { 0 },
                glyph_at: (gx + sx, gy - sy),
                sound: (f == 1).then_some(SOUND_STONE),
            });
        }
        Ok(out)
    }
}

/// The 0x50 code table: the UI effect of each code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeEffect {
    /// 1: quest-log values (two quest words).
    QuestLogValues,
    /// 3: nothing shown (no instruction reads the globals).
    Nothing,
    /// 4: the Inifuss scroll stones.
    InifussStones,
    /// 23, 36: none in the UI.
    NoUi,
}

/// Table: the UI effect of a 0x50 code (code 2 is the hire popup).
pub fn code_effect(code: u8) -> Option<CodeEffect> {
    Some(match code {
        1 => CodeEffect::QuestLogValues,
        3 => CodeEffect::Nothing,
        4 => CodeEffect::InifussStones,
        23 | 36 => CodeEffect::NoUi,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_open() {
        let mut h = HirePopup {
            state: 2,
            popup: false,
        };
        let o = h.open(None);
        assert_eq!(
            o,
            PanelOutput::SetUi {
                ui: 0x23,
                mode: 0,
                jump: false
            }
        );
        // The registry value missing (= 0): the popup flag is set.
        assert_eq!((h.state, h.popup), (0, true));
        // PopupHireling already 1: the flag stays as it was.
        let mut h = HirePopup::default();
        h.open(Some(1));
        assert!(!h.popup);
        let mut h = HirePopup::default();
        h.open(Some(0));
        assert!(h.popup);
    }

    #[test]
    fn portrait_pass_opens_the_hireling_panel_once() {
        let mut h = HirePopup {
            state: 0,
            popup: true,
        };
        let mut calls = Vec::new();
        let mut ok = |ui: u8, mode: u8| {
            calls.push((ui, mode));
            true
        };
        // Blocked cases: open mode 2/3, state 9 or 0x0B, [7BEECC] = 2,
        // state 0x24 already open.
        for env in [
            PortraitEnv {
                open_mode_2_or_3: true,
                ..Default::default()
            },
            PortraitEnv {
                state_9_or_b: true,
                ..Default::default()
            },
            PortraitEnv {
                state_24_open: true,
                ..Default::default()
            },
        ] {
            assert_eq!(h.portrait_pass(env, &mut ok), None);
        }
        h.state = 2;
        assert_eq!(h.portrait_pass(PortraitEnv::default(), &mut ok), None);
        assert!(h.popup);
        h.state = 0;
        // Success: PopupHireling:= 1 and the flag clears.
        assert_eq!(h.portrait_pass(PortraitEnv::default(), &mut ok), Some(1));
        assert!(!h.popup);
        assert_eq!(h.portrait_pass(PortraitEnv::default(), &mut ok), None);
        // A refused SetUIState keeps the flag.
        let mut h = HirePopup {
            state: 0,
            popup: true,
        };
        assert_eq!(
            h.portrait_pass(PortraitEnv::default(), &mut |_, _| false),
            None
        );
        assert!(h.popup);
        drop(ok);
        assert_eq!(calls, vec![(0x24, 0)]);
    }

    // (the scroll text is exempt)
    #[test]
    fn code_table() {
        assert_eq!(code_effect(1), Some(CodeEffect::QuestLogValues));
        assert_eq!(code_effect(3), Some(CodeEffect::Nothing));
        assert_eq!(code_effect(4), Some(CodeEffect::InifussStones));
        assert_eq!(code_effect(23), Some(CodeEffect::NoUi));
        assert_eq!(code_effect(36), Some(CodeEffect::NoUi));
        assert_eq!(code_effect(2), None);
    }

    #[test]
    fn inifuss_scroll_codes() {
        let mut s = InifussScroll::default();
        s.code4([1; 12]);
        assert!(!s.opened);
        assert_eq!(s.bytes, [1; 12]);
        // bks: no stones; bkd: stones until opened; tr1: plain.
        assert_eq!(s.draw_kind(ScrollCode::Bks), ScrollDraw::WithStones(false));
        assert_eq!(s.draw_kind(ScrollCode::Bkd), ScrollDraw::WithStones(true));
        assert_eq!(s.draw_kind(ScrollCode::Tr1), ScrollDraw::Plain);
        // Opening bkd sets opened and sends C→S 0x3E with the GUID.
        assert_eq!(
            s.open_bkd(0x0102_0304),
            ClientIntent(vec![0x3E, 4, 3, 2, 1])
        );
        assert!(s.opened);
        assert_eq!(s.draw_kind(ScrollCode::Bkd), ScrollDraw::Plain);
        // Code 4 clears it again: the stones appear when code 4 answers.
        s.code4([0; 12]);
        assert_eq!(s.draw_kind(ScrollCode::Bkd), ScrollDraw::WithStones(true));
    }

    fn scroll(symbols: [u16; 5]) -> InifussScroll {
        let mut b = [0u8; 12];
        for (i, s) in symbols.iter().enumerate() {
            b[2 * i..2 * i + 2].copy_from_slice(&s.to_le_bytes());
        }
        InifussScroll {
            bytes: b,
            ..Default::default()
        }
    }

    #[test]
    fn stones_and_animation() {
        let s = scroll([0, 1, 2, 3, 4]);
        // Counter 0: nothing has started (f ≥ 1).
        assert!(s.stones(0, 0, 0).unwrap().is_empty());
        // Counter 1: stone 0 at f = 1: mode 0, glyph frame 1, the sound.
        let d = s.stones(1, 80, 60).unwrap();
        assert_eq!(
            d,
            vec![StoneDraw {
                stone: 0,
                icon_frame: 0,
                icon_at: (322, 44),
                mode: 0,
                glyph_frame: 1,
                glyph_at: (383, 101),
                sound: Some(2671),
            }]
        );
        // Modes by f: 0 (< 5), 1 (< 10), 2 (< 15), else 5; the glyph frame
        // f below 21, else 0; stone 1 starts at counter 12.
        for (counter, mode, glyph) in [
            (4, 0, 4),
            (5, 1, 5),
            (9, 1, 9),
            (10, 2, 10),
            (14, 2, 14),
            (15, 5, 15),
            (20, 5, 20),
            (21, 5, 0),
        ] {
            let d = s.stones(counter, 0, 0).unwrap();
            let st = d.iter().find(|d| d.stone == 0).unwrap();
            assert_eq!((st.mode, st.glyph_frame), (mode, glyph), "f = {counter}");
        }
        // Stone 1 (symbol 1) at counter 13: f = 1, icon frame 1 at
        // (255, 222), glyph (322, 242).
        let d = s.stones(13, 0, 0).unwrap();
        let st = d.iter().find(|d| d.stone == 1).unwrap();
        assert_eq!(
            (st.icon_frame, st.icon_at, st.glyph_at, st.sound),
            (1, (255, 222), (322, 242), Some(2671))
        );
        // Symbols ≥ 6 are skipped; symbol 5 is fatal 0x1673.
        let s6 = scroll([6, 7, 2, 3, 4]);
        assert!(s6.stones(1, 0, 0).unwrap().is_empty());
        assert_eq!(
            scroll([5, 0, 0, 0, 0]).stones(1, 0, 0),
            Err(ScrollError::Symbol5)
        );
        // A stone not started yet never reads its symbol: symbol 5 on
        // stone 1 (start 12) is harmless at counter 12.
        assert!(scroll([0, 5, 0, 0, 0]).stones(12, 0, 0).is_ok());
        // The position tables of the spec.
        assert_eq!(SCROIN2_POS[3], (47, 222));
        assert_eq!(SCROIN3_POS[4], (211, 162));
        // The counter steps once per > 50 ms; reset when the time is 0.
        let mut a = StoneAnim::default();
        a.step(1000);
        assert_eq!((a.counter, a.time), (0, 1000));
        a.step(1050);
        assert_eq!(a.counter, 0);
        a.step(1051);
        assert_eq!((a.counter, a.time), (1, 1051));
        a.step(1102);
        assert_eq!(a.counter, 2);
    }
}
