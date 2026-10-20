//! The mini panel (UI state 0x15): variant, blocked sides, layouts,
//! buttons, tool tips, press and release.

use super::globes::Tip;
use crate::ui::panels::PanelOutput;

/// Mini panel UI state.
pub const UI_MINI: u8 = 0x15;
/// Button tool tip strings by function f.
pub const TIP_STRINGS: [u16; 8] = [4169, 4170, 4171, 4172, 4173, 4174, 4175, 4176];
/// The key bindings of the functions, `None` for the game menu.
pub const FUNCTION_BINDINGS: [Option<u8>; 8] = [
    Some(0),
    Some(1),
    Some(12),
    Some(2),
    Some(7),
    Some(3),
    Some(4),
    None,
];

/// The states that block the left side.
pub const LEFT_BLOCKERS: [u8; 15] = [
    0x17, 0x19, 0x1A, 0x0C, 0x1C, 0x1D, 0x18, 3, 0x10, 0x16, 0x24, 2, 0x0F, 0x14, 0x1B,
];
/// The states that block the right side, besides the belt rule and
/// states 1 and 4.
pub const RIGHT_BLOCKERS: [u8; 8] = [0x17, 0x19, 0x1C, 0x1A, 0x0C, 0x18, 0x1B, 3];

/// The layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Right blocked only: left of centre.
    One = 1,
    /// Neither blocked.
    Two = 2,
    /// Left blocked only.
    Three = 3,
}

/// The sides result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides {
    /// The left side is blocked.
    pub left_blocked: bool,
    /// The return value (the draw uses it).
    pub right_blocked: bool,
    /// 1 only when state 1 or 4 is open (the input uses
    /// it).
    pub flag_968: bool,
}

/// `open(ui)` tells whether a UI state
/// is open; `belt_extra_rows` and `belt_row_count` are the belt's.
pub fn sides(open: &dyn Fn(u8) -> bool, belt_extra_rows: bool, belt_row_count: u8) -> Sides {
    let left = LEFT_BLOCKERS.iter().any(|&s| open(s));
    let st14 = open(1) || open(4);
    let right = RIGHT_BLOCKERS.iter().any(|&s| open(s))
        || ((open(0x1F) || belt_extra_rows) && belt_row_count > 1)
        || st14;
    Sides {
        left_blocked: left,
        right_blocked: right,
        // The other cases set the flag and then clear it: it ends as
        // 1 only when state 1 or 4 is open.
        flag_968: st14,
    }
}

/// The layout of the draw from the draw's sides; `None`: nothing.
pub fn layout(left_blocked: bool, right_blocked: bool) -> Option<Layout> {
    match (left_blocked, right_blocked) {
        (false, false) => Some(Layout::Two),
        (true, false) => Some(Layout::Three),
        (false, true) => Some(Layout::One),
        (true, true) => None,
    }
}

/// One button of the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MiniButton {
    pub i: usize,
    /// The function f.
    pub f: usize,
    /// The art frame drawn: `minipanelbtn` frame b_i (+ 1 pressed).
    pub frame: u32,
    pub x: i32,
    pub y: i32,
}

/// The mini panel's state and variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniPanel {
    /// Single player.
    pub single: bool,
    /// Pressed flags, one per button.
    pub pressed: [bool; 8],
    /// The left side is blocked.
    pub left_blocked: bool,
    /// Set only when state 1 or 4 is open (the input reads it).
    pub flag_968: bool,
    /// Nothing drawn (both sides blocked).
    pub hidden: bool,
    /// The press latch.
    pub latch: bool,
    /// Spare flag, cleared with the latch.
    pub flag_974: bool,
    /// The layout of the last draw: the x table
    /// is written only by the draw; `None` before the first draw (all x 0).
    pub last_layout: Option<Layout>,
}

/// Why a press or release is not acted on.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerFacts {
    /// P is a living player.
    pub living: bool,
    /// P is blocked.
    pub blocked: bool,
    /// P is dead (mode 0x11).
    pub dead: bool,
}

/// What a release does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniAction {
    Ui(PanelOutput),
    /// The quest log.
    QuestLog,
    /// The game menu: close all, input reset, the menu opens.
    GameMenu,
    /// Input reset after a release that ran a function.
    InputReset,
    /// A cursor in mode ≠ 7 is reset.
    CursorReset,
    /// UI sound 4 on a press.
    Sound(u32),
}

fn toggle_state(ui: u8, open: &dyn Fn(u8) -> bool) -> PanelOutput {
    PanelOutput::SetUi {
        ui,
        // Open → off (1), else toggle (2).
        mode: if open(ui) { 1 } else { 2 },
        jump: false,
    }
}

/// The action on release of function f.
pub fn function_actions(f: usize, single: bool, open: &dyn Fn(u8) -> bool) -> Vec<MiniAction> {
    let t = |ui| MiniAction::Ui(toggle_state(ui, open));
    match f {
        0 => vec![t(2)],
        1 => vec![t(1)],
        2 => vec![t(4)],
        // Multiplayer only.
        3 if !single => vec![t(0x16)],
        3 => Vec::new(),
        4 => vec![t(0x0A)],
        5 => {
            let mut v = Vec::new();
            if open(0x1F) {
                v.push(MiniAction::Ui(PanelOutput::SetUi {
                    ui: 0x1F,
                    mode: 1,
                    jump: false,
                }));
            }
            v.push(t(0x18));
            v
        }
        6 => vec![MiniAction::QuestLog],
        7 => vec![MiniAction::GameMenu],
        _ => Vec::new(),
    }
}

/// The region of the tool tips, press and release: strict W/2 +
/// o − 84 < x < W/2 + o + 89, H − 69 < y < H − 47.
pub fn region(w: i32, h: i32, o: i32, x: i32, y: i32) -> bool {
    w / 2 + o - 84 < x && x < w / 2 + o + 89 && h - 69 < y && y < h - 47
}

impl MiniPanel {
    /// The variant at game start: multiplayer (game type ≠ 0) →
    /// `Panel\minipanel`, 8 buttons, single:= 0; single player →
    /// `Panel\minipanel_s`, 7 buttons (no party button), single:= 1.
    /// Pressed flags cleared.
    pub fn new(game_type: u32) -> Self {
        Self {
            single: game_type == 0,
            pressed: [false; 8],
            left_blocked: false,
            flag_968: false,
            hidden: false,
            latch: false,
            flag_974: false,
            last_layout: None,
        }
    }

    /// Buttons reachable by the current press/release layout (the last draw).
    pub fn ui_elements(
        &self,
        w: i32,
        h: i32,
        extra: bool,
        rows: u8,
    ) -> Vec<crate::ui::ui_ids::UiElement> {
        if self.hidden {
            return Vec::new();
        }
        let Some(l) = self.last_layout else {
            return Vec::new();
        };
        let names = [
            "character",
            "inventory",
            "skills",
            "party",
            "automap",
            "message_log",
            "quests",
            "game_menu",
        ];
        self.buttons(l, w, h)
            .into_iter()
            .filter_map(|b| {
                let r = crate::ui::geom::Rect::new(b.x + 1, h - 68, 19, 21);
                let e = crate::ui::ui_ids::UiElement::new(format!("minipanel.{}", names[b.f]), r);
                region(w, h, self.press_offset(extra, rows), e.at.x, e.at.y).then_some(e)
            })
            .collect()
    }

    /// The art file by variant.
    pub fn art(&self) -> &'static str {
        if self.single {
            "Panel\\minipanel_s"
        } else {
            "Panel\\minipanel"
        }
    }

    /// The button count by variant.
    pub fn button_count(&self) -> usize {
        if self.single {
            7
        } else {
            8
        }
    }

    /// Stores the sides of this frame (the draw's side effects).
    pub fn set_sides(&mut self, s: &Sides) {
        self.left_blocked = s.left_blocked;
        self.flag_968 = s.flag_968;
    }

    /// The art position by layout: frame 0 at (W/2 − 74 − 3
    /// (single) or W/2 − 84 − 3, H − 47) for layout 2; (W/2 + 56 − 2 or W/2 +
    /// 35 − 2, H − 47) for layout 3; (W/2 − 205, H − 47) for layout 1.
    pub fn art_pos(&self, l: Layout, w: i32, h: i32) -> (i32, i32) {
        let x = match l {
            Layout::Two => {
                if self.single {
                    w / 2 - 74 - 3
                } else {
                    w / 2 - 84 - 3
                }
            }
            Layout::Three => {
                if self.single {
                    w / 2 + 56 - 2
                } else {
                    w / 2 + 35 - 2
                }
            }
            Layout::One => w / 2 - 205,
        };
        (x, h - 47)
    }

    /// The draw: hidden:= 1 when both sides are blocked (nothing
    /// drawn), else 0.
    pub fn draw(
        &mut self,
        left_blocked: bool,
        right_blocked: bool,
        w: i32,
        h: i32,
    ) -> Option<((i32, i32), Vec<MiniButton>)> {
        let l = layout(left_blocked, right_blocked);
        self.hidden = l.is_none();
        let l = l?;
        self.last_layout = Some(l);
        Some((self.art_pos(l, w, h), self.buttons(l, w, h)))
    }

    /// The buttons: x0 = W/2 − 84 (multi) /
    /// W/2 − 74 (single); layout 1 → W/2 − 202; layout 3 → W/2 + 35 (multi)
    /// / W/2 + 57 (single). Button i at (x0 + 21 i, H − 50); multi f = i, b
    /// = 2 i; single f = i (+ 1 from i = 3), b = 2 i (+ 2 from i = 3).
    pub fn buttons(&self, l: Layout, w: i32, h: i32) -> Vec<MiniButton> {
        let x0 = match l {
            Layout::One => w / 2 - 202,
            Layout::Two => {
                if self.single {
                    w / 2 - 74
                } else {
                    w / 2 - 84
                }
            }
            Layout::Three => {
                if self.single {
                    w / 2 + 57
                } else {
                    w / 2 + 35
                }
            }
        };
        (0..self.button_count())
            .map(|i| {
                let skip = self.single && i >= 3;
                let b = 2 * i as u32 + if skip { 2 } else { 0 };
                MiniButton {
                    i,
                    f: i + usize::from(skip),
                    frame: b + u32::from(self.pressed[i]),
                    x: x0 + 21 * i as i32,
                    y: h - 50,
                }
            })
            .collect()
    }

    /// The tool tip: in the region of the
    /// layout the button with x_i < x < x_i + 20 gets its
    /// string and for the primary and secondary key of its binding ` (%s)`
    /// (4178) with the key name, at (x_i − 3, H − 76), color 0, centred.
    pub fn tip(
        &self,
        l: Layout,
        w: i32,
        h: i32,
        mouse: (i32, i32),
        keys: &dyn Fn(usize) -> [Option<Vec<u16>>; 2],
        strings: &dyn Fn(u16) -> Vec<u16>,
    ) -> Option<Tip> {
        let o = match l {
            Layout::Two => 0,
            Layout::One => -118,
            Layout::Three => 119,
        };
        if !region(w, h, o, mouse.0, mouse.1) {
            return None;
        }
        let b = self
            .buttons(l, w, h)
            .into_iter()
            .find(|b| b.x < mouse.0 && mouse.0 < b.x + 20)?;
        let mut text = strings(TIP_STRINGS[b.f]);
        for k in keys(b.f).iter().flatten() {
            let mut one = strings(4178);
            // " (%s)": substitute the key name.
            if let Some(p) = one.windows(2).position(|p| p == [37u16, 115]) {
                one.splice(p..p + 2, k.iter().copied());
            }
            text.extend(one);
        }
        Some(Tip {
            text,
            x: b.x - 3,
            y: h - 76,
            color: 0,
            centered: true,
        })
    }

    /// The region offset of press and release: −118 when the belt
    /// has extra rows and a row count > 1, −118 when `flag_968`, +119
    /// when the left side is blocked.
    pub fn press_offset(&self, belt_extra_rows: bool, belt_row_count: u8) -> i32 {
        let mut o = 0;
        if belt_extra_rows && belt_row_count > 1 {
            o -= 118;
        }
        if self.flag_968 {
            o -= 118;
        }
        if self.left_blocked {
            o += 119;
        }
        o
    }

    /// The button under x (strict x test) and its function, from the x
    /// table of the last draw; before any draw every x is 0.
    fn button_at(&self, w: i32, h: i32, x: i32) -> Option<MiniButton> {
        match self.last_layout {
            Some(l) => self.buttons(l, w, h),
            None => {
                let mut v = self.buttons(Layout::Two, w, h);
                v.iter_mut().for_each(|b| b.x = 0);
                v
            }
        }
        .into_iter()
        .find(|b| b.x < x && x < b.x + 20)
    }

    /// Press: no cursor item and not hidden.
    /// Returns the effects and whether the event is consumed.
    #[allow(clippy::too_many_arguments)]
    pub fn press(
        &mut self,
        w: i32,
        h: i32,
        mouse: (i32, i32),
        cursor_item: bool,
        belt_extra_rows: bool,
        belt_row_count: u8,
        state9_open: bool,
        p: &PlayerFacts,
    ) -> (Vec<MiniAction>, bool) {
        if cursor_item || self.hidden {
            return (Vec::new(), false);
        }
        let o = self.press_offset(belt_extra_rows, belt_row_count);
        if !region(w, h, o, mouse.0, mouse.1) {
            return (Vec::new(), false);
        }
        if let Some(b) = self.button_at(w, h, mouse.0) {
            let late = if self.single { 4 } else { 5 };
            if b.f == 7 || (p.living && (!state9_open || b.i >= late)) {
                self.pressed[b.i] = true;
            }
        }
        // In every case UI sound 4, the press latch:= 1, consumed.
        self.latch = true;
        (vec![MiniAction::Sound(4)], true)
    }

    /// Release: only with the latch; no cursor item
    /// (a cursor in mode ≠ 7 is reset); not hidden.
    #[allow(clippy::too_many_arguments)]
    pub fn release(
        &mut self,
        w: i32,
        h: i32,
        mouse: (i32, i32),
        cursor_item: bool,
        cursor_mode: u8,
        belt_extra_rows: bool,
        belt_row_count: u8,
        p: &PlayerFacts,
        open: &dyn Fn(u8) -> bool,
    ) -> (Vec<MiniAction>, bool) {
        if !self.latch {
            return (Vec::new(), false);
        }
        self.latch = false;
        if cursor_item || self.hidden {
            return (Vec::new(), false);
        }
        let mut eff = Vec::new();
        if cursor_mode != 7 {
            eff.push(MiniAction::CursorReset);
        }
        self.pressed = [false; 8];
        let o = self.press_offset(belt_extra_rows, belt_row_count);
        if !region(w, h, o, mouse.0, mouse.1) {
            self.flag_974 = false;
            return (eff, false);
        }
        if let Some(b) = self.button_at(w, h, mouse.0) {
            // The function runs whatever button was pressed.
            if b.f == 7 || (!p.blocked && !p.dead) {
                eff.extend(function_actions(b.f, self.single, open));
            }
        }
        eff.push(MiniAction::InputReset);
        (eff, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(_: u8) -> bool {
        false
    }

    #[test]
    fn variant_by_game_type() {
        let m = MiniPanel::new(1);
        assert_eq!(
            (m.art(), m.button_count(), m.single),
            ("Panel\\minipanel", 8, false)
        );
        let s = MiniPanel::new(0);
        assert_eq!(
            (s.art(), s.button_count(), s.single),
            ("Panel\\minipanel_s", 7, true)
        );
        assert_eq!(s.pressed, [false; 8]);
        assert_eq!(MiniPanel::new(2).button_count(), 8);
    }

    #[test]
    fn blocked_sides() {
        let one = |s: u8| move |x: u8| x == s;
        // Left blockers.
        for s in LEFT_BLOCKERS {
            assert!(sides(&one(s), false, 1).left_blocked, "{s:#x}");
        }
        for s in [1u8, 4, 5, 6, 7, 8, 9, 0x0A, 0x0B, 0x15, 0x1F] {
            assert!(!sides(&one(s), false, 1).left_blocked, "{s:#x}");
        }
        // Right blockers.
        for s in RIGHT_BLOCKERS {
            let r = sides(&one(s), false, 1);
            assert!(r.right_blocked, "{s:#x}");
            // `flag_968` only for states 1 and 4.
            assert!(!r.flag_968);
        }
        for s in [1u8, 4] {
            let r = sides(&one(s), false, 1);
            assert!(r.right_blocked && r.flag_968);
        }
        assert!(!sides(&closed, false, 1).right_blocked);
        // State 0x1F open or the belt has extra rows, with a row count > 1.
        assert!(sides(&one(0x1F), false, 2).right_blocked);
        assert!(!sides(&one(0x1F), false, 1).right_blocked);
        assert!(sides(&closed, true, 3).right_blocked);
        assert!(!sides(&closed, true, 1).right_blocked);
        assert!(!sides(&closed, false, 4).right_blocked);
        // The input flag differs from the draw's return value for the
        // other blockers (reproduced).
        let r = sides(&one(3), false, 1);
        assert_eq!((r.right_blocked, r.flag_968), (true, false));
    }

    // Test vectors of the layout.
    #[test]
    fn layouts_and_buttons() {
        // (no, no): layout 2; (yes, no): 3; (no, yes): 1; (yes, yes): nothing,
        // hidden:= 1.
        let mut s = MiniPanel::new(0);
        assert_eq!(layout(false, false), Some(Layout::Two));
        assert_eq!(layout(true, false), Some(Layout::Three));
        assert_eq!(layout(false, true), Some(Layout::One));
        assert_eq!(layout(true, true), None);
        assert!(s.draw(true, true, 800, 600).is_none());
        assert!(s.hidden);
        // Single player, 800 × 600, layout 2: art at (323, 553); buttons at
        // x 326, 347, 368, 389 (f 4), 410, 431, 452.
        let (art, b) = s.draw(false, false, 800, 600).unwrap();
        assert!(!s.hidden);
        assert_eq!(art, (323, 553));
        assert_eq!(
            b.iter().map(|b| b.x).collect::<Vec<_>>(),
            vec![326, 347, 368, 389, 410, 431, 452]
        );
        assert_eq!(
            b.iter().map(|b| b.f).collect::<Vec<_>>(),
            vec![0, 1, 2, 4, 5, 6, 7]
        );
        assert_eq!(
            b.iter().map(|b| b.frame).collect::<Vec<_>>(),
            vec![0, 2, 4, 8, 10, 12, 14]
        );
        assert!(b.iter().all(|b| b.y == 550));
        // A pressed button: frame b_i + 1.
        s.pressed[2] = true;
        assert_eq!(s.draw(false, false, 800, 600).unwrap().1[2].frame, 5);
        // Multiplayer, 640 × 480, layout 1: buttons x 118 + 21 i, y 430.
        let m = MiniPanel::new(1);
        let b = m.buttons(Layout::One, 640, 480);
        assert_eq!(b.len(), 8);
        for (i, b) in b.iter().enumerate() {
            assert_eq!(
                (b.x, b.y, b.f, b.frame),
                (118 + 21 * i as i32, 430, i, 2 * i as u32)
            );
        }
        // The art by layout and variant.
        let single = MiniPanel::new(0);
        assert_eq!(m.art_pos(Layout::Two, 800, 600), (400 - 84 - 3, 553));
        assert_eq!(m.art_pos(Layout::Three, 800, 600), (400 + 35 - 2, 553));
        assert_eq!(single.art_pos(Layout::Three, 800, 600), (400 + 56 - 2, 553));
        assert_eq!(m.art_pos(Layout::One, 800, 600), (195, 553));
        assert_eq!(single.art_pos(Layout::One, 800, 600), (195, 553));
        // Layout 3 buttons: W/2 + 35 (multi) / W/2 + 57 (single).
        assert_eq!(m.buttons(Layout::Three, 800, 600)[0].x, 435);
        assert_eq!(single.buttons(Layout::Three, 800, 600)[0].x, 457);
        assert_eq!(m.buttons(Layout::Two, 800, 600)[0].x, 316);
    }

    #[test]
    fn function_table() {
        let set = |ui: u8, mode: u8| {
            MiniAction::Ui(PanelOutput::SetUi {
                ui,
                mode,
                jump: false,
            })
        };
        // Open → off, else toggle.
        assert_eq!(function_actions(0, false, &closed), vec![set(2, 2)]);
        assert_eq!(function_actions(0, false, &|s| s == 2), vec![set(2, 1)]);
        assert_eq!(function_actions(1, false, &closed), vec![set(1, 2)]);
        assert_eq!(function_actions(2, false, &|s| s == 4), vec![set(4, 1)]);
        // Party screen: multiplayer only.
        assert_eq!(function_actions(3, false, &closed), vec![set(0x16, 2)]);
        assert!(function_actions(3, true, &closed).is_empty());
        assert_eq!(function_actions(4, false, &closed), vec![set(0x0A, 2)]);
        // Message log: state 0x1F off if open; state 0x18 likewise.
        assert_eq!(function_actions(5, false, &closed), vec![set(0x18, 2)]);
        assert_eq!(
            function_actions(5, false, &|s| s == 0x1F),
            vec![set(0x1F, 1), set(0x18, 2)]
        );
        assert_eq!(
            function_actions(5, false, &|s| s == 0x18),
            vec![set(0x18, 1)]
        );
        assert_eq!(
            function_actions(6, false, &closed),
            vec![MiniAction::QuestLog]
        );
        assert_eq!(
            function_actions(7, false, &closed),
            vec![MiniAction::GameMenu]
        );
        // The tool tip strings and bindings.
        assert_eq!(
            TIP_STRINGS,
            [4169, 4170, 4171, 4172, 4173, 4174, 4175, 4176]
        );
        assert_eq!(
            FUNCTION_BINDINGS,
            [
                Some(0),
                Some(1),
                Some(12),
                Some(2),
                Some(7),
                Some(3),
                Some(4),
                None
            ]
        );
    }

    fn strings(id: u16) -> Vec<u16> {
        let s = match id {
            4169 => "Character",
            4170 => "Inventory",
            4178 => " (%s)",
            _ => "?",
        };
        s.encode_utf16().collect()
    }

    #[test]
    fn tool_tips() {
        let m = MiniPanel::new(1);
        let keys = |f: usize| -> [Option<Vec<u16>>; 2] {
            match f {
                0 => [Some("C".encode_utf16().collect()), None],
                1 => [
                    Some("I".encode_utf16().collect()),
                    Some("F2".encode_utf16().collect()),
                ],
                _ => [None, None],
            }
        };
        // Layout 2, multiplayer 800 × 600: x0 316; button 0 at 316…336.
        let t = m
            .tip(Layout::Two, 800, 600, (320, 540), &keys, &strings)
            .unwrap();
        assert_eq!(
            t,
            Tip {
                text: "Character (C)".encode_utf16().collect(),
                x: 313,
                y: 524,
                color: 0,
                centered: true
            }
        );
        // The primary and secondary key.
        let t = m
            .tip(Layout::Two, 800, 600, (340, 540), &keys, &strings)
            .unwrap();
        assert_eq!(
            t.text,
            "Inventory (I) (F2)".encode_utf16().collect::<Vec<_>>()
        );
        // Strict: x_i < x < x_i + 20.
        assert!(m
            .tip(Layout::Two, 800, 600, (316, 540), &keys, &strings)
            .is_none());
        assert!(m
            .tip(Layout::Two, 800, 600, (336, 540), &keys, &strings)
            .is_none());
        // The region (strict): W/2 + o − 84 < x < W/2 + o + 89, H − 69 < y <
        // H − 47, o = 0 / −118 / +119.
        assert!(m
            .tip(Layout::Two, 800, 600, (320, 531), &keys, &strings)
            .is_none());
        assert!(m
            .tip(Layout::Two, 800, 600, (320, 553), &keys, &strings)
            .is_none());
        assert!(m
            .tip(Layout::Two, 800, 600, (320, 532), &keys, &strings)
            .is_some());
        assert!(m
            .tip(Layout::Two, 800, 600, (320, 552), &keys, &strings)
            .is_some());
        // Layout 1: x0 198, region 198 … 371; layout 3: x0 435, 435 … 608.
        assert!(m
            .tip(Layout::One, 800, 600, (205, 540), &keys, &strings)
            .is_some());
        assert!(m
            .tip(Layout::Three, 800, 600, (440, 540), &keys, &strings)
            .is_some());
        assert!(m
            .tip(Layout::Three, 800, 600, (205, 540), &keys, &strings)
            .is_none());
    }

    fn player() -> PlayerFacts {
        PlayerFacts {
            living: true,
            ..Default::default()
        }
    }

    /// A panel that has drawn layout 2 (the x table is written by the draw).
    fn drawn(game_type: u32) -> MiniPanel {
        let mut m = MiniPanel::new(game_type);
        m.last_layout = Some(Layout::Two);
        m
    }

    #[test]
    fn hit_test_uses_the_last_drawn_layout() {
        // State 0x1F open with belt row count 2: right blocked, the draw
        // chose layout 1, press offset o = 0 (the region is centred).
        let s = sides(&|x| x == 0x1F, false, 2);
        let mut m = MiniPanel::new(1);
        m.set_sides(&s);
        assert!(m.draw(s.left_blocked, s.right_blocked, 800, 600).is_some());
        assert_eq!(m.last_layout, Some(Layout::One));
        assert_eq!(m.press_offset(false, 1), 0);
        // Buttons are at W/2 - 202 + 21 i (layout 1): x = W/2 - 60 is
        // inside button 6 (324..344), not button 1 of layout 2 (337..357).
        // (The queue row's "hits no button" does not follow from the x
        // table: 198 + 21 * 6 < 340 < 198 + 21 * 6 + 20.)
        let (e, c) = m.press(
            800,
            600,
            (800 / 2 - 60, 600 - 55),
            false,
            false,
            1,
            false,
            &player(),
        );
        assert!(c && e == vec![MiniAction::Sound(4)]);
        assert_eq!(
            m.pressed,
            [false, false, false, false, false, false, true, false]
        );
        // Before the first draw every x is 0: x 5 hits button 0.
        let m = MiniPanel::new(1);
        assert_eq!(m.button_at(800, 600, 5).map(|b| b.i), Some(0));
        assert_eq!(m.button_at(800, 600, 316 + 25), None);
    }

    #[test]
    fn press_region_and_pressed_flags() {
        // Multiplayer, o = 0: x0 316, button i at 316 + 21 i.
        let mut m = drawn(1);
        // The press offset: −118 belt extra rows with a count > 1, −118
        // [7BC968], +119 [7BC96C].
        assert_eq!(m.press_offset(false, 1), 0);
        assert_eq!(m.press_offset(true, 2), -118);
        assert_eq!(m.press_offset(true, 1), 0);
        m.flag_968 = true;
        assert_eq!(m.press_offset(true, 2), -236);
        m.flag_968 = false;
        m.left_blocked = true;
        assert_eq!(m.press_offset(false, 1), 119);
        m.left_blocked = false;
        // A press on button 1 (x 337…357 → 340): sound 4, latch, consumed,
        // pressed (state 9 closed, living).
        let (e, c) = m.press(800, 600, (340, 540), false, false, 1, false, &player());
        assert_eq!(e, vec![MiniAction::Sound(4)]);
        assert!(c && m.latch && m.pressed[1]);
        // State 9 open: only buttons i ≥ 5 (multi) / ≥ 4 (single) and the
        // game menu (f = 7) press.
        let mut m = drawn(1);
        let (_, c) = m.press(800, 600, (340, 540), false, false, 1, true, &player());
        assert!(c && m.latch && m.pressed == [false; 8]);
        let x5 = 316 + 21 * 5 + 5;
        m.press(800, 600, (x5, 540), false, false, 1, true, &player());
        assert!(m.pressed[5]);
        let mut m = drawn(1);
        m.press(
            800,
            600,
            (316 + 21 * 4 + 5, 540),
            false,
            false,
            1,
            true,
            &player(),
        );
        assert!(!m.pressed[4]);
        let mut m = drawn(0);
        m.press(
            800,
            600,
            (326 + 21 * 4 + 5, 540),
            false,
            false,
            1,
            true,
            &player(),
        );
        assert!(m.pressed[4]);
        // The game menu (f = 7: single button 6) presses even in state 9 and
        // for a dead player.
        let mut m = drawn(0);
        let dead = PlayerFacts::default();
        m.press(
            800,
            600,
            (326 + 21 * 6 + 5, 540),
            false,
            false,
            1,
            true,
            &dead,
        );
        assert!(m.pressed[6]);
        // A non-living player: no pressed flag, still sound and latch.
        let mut m = drawn(1);
        let (e, c) = m.press(800, 600, (340, 540), false, false, 1, false, &dead);
        assert!(c && m.latch && m.pressed == [false; 8] && e.len() == 1);
        // A cursor item, or the panel hidden: nothing. Outside the region:
        // not consumed.
        let mut m = drawn(1);
        assert_eq!(
            m.press(800, 600, (340, 540), true, false, 1, false, &player()),
            (vec![], false)
        );
        m.hidden = true;
        assert!(
            !m.press(800, 600, (340, 540), false, false, 1, false, &player())
                .1
        );
        m.hidden = false;
        let (e, c) = m.press(800, 600, (100, 100), false, false, 1, false, &player());
        assert!(e.is_empty() && !c && !m.latch);
    }

    #[test]
    fn release_runs_the_button_under_the_mouse() {
        let open = closed;
        let set = |ui: u8, mode: u8| {
            MiniAction::Ui(PanelOutput::SetUi {
                ui,
                mode,
                jump: false,
            })
        };
        let mut m = drawn(1);
        m.press(800, 600, (320, 540), false, false, 1, false, &player());
        assert!(m.latch && m.pressed[0]);
        // Released over button 1 (x 337…357): that one runs (the function runs
        // whatever button was pressed); flags cleared, latch:= 0, the input
        // reset, consumed; a cursor in mode ≠ 7 is reset.
        let (e, c) = m.release(800, 600, (340, 540), false, 0, false, 1, &player(), &open);
        assert_eq!(
            e,
            vec![MiniAction::CursorReset, set(1, 2), MiniAction::InputReset]
        );
        assert!(c && !m.latch && m.pressed == [false; 8]);
        // Cursor mode 7: no reset.
        let mut m = drawn(1);
        m.latch = true;
        let (e, _) = m.release(800, 600, (340, 540), false, 7, false, 1, &player(), &open);
        assert_eq!(e, vec![set(1, 2), MiniAction::InputReset]);
        // A blocked or dead player runs nothing (the input reset still
        // runs); the game menu (f = 7) runs anyway.
        let blocked = PlayerFacts {
            living: true,
            blocked: true,
            ..Default::default()
        };
        let mut m = drawn(1);
        m.latch = true;
        let (e, c) = m.release(800, 600, (340, 540), false, 7, false, 1, &blocked, &open);
        assert_eq!(e, vec![MiniAction::InputReset]);
        assert!(c);
        let dead = PlayerFacts {
            dead: true,
            ..Default::default()
        };
        m.latch = true;
        let (e, _) = m.release(800, 600, (340, 540), false, 7, false, 1, &dead, &open);
        assert_eq!(e, vec![MiniAction::InputReset]);
        let x7 = 316 + 21 * 7 + 5;
        m.latch = true;
        let (e, _) = m.release(800, 600, (x7, 540), false, 7, false, 1, &dead, &open);
        assert_eq!(e, vec![MiniAction::GameMenu, MiniAction::InputReset]);
        // Outside the region: [7BC974]:= 0, not consumed. Without the latch:
        // nothing.
        let mut m = drawn(1);
        m.flag_974 = true;
        assert_eq!(
            m.release(800, 600, (340, 540), false, 7, false, 1, &player(), &open),
            (vec![], false)
        );
        m.latch = true;
        let (_, c) = m.release(800, 600, (10, 10), false, 7, false, 1, &player(), &open);
        assert!(!c && !m.flag_974 && !m.latch);
        // A cursor item: nothing run.
        m.latch = true;
        assert!(
            !m.release(800, 600, (340, 540), true, 0, false, 1, &player(), &open)
                .1
        );
    }
}
