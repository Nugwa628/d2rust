//! The run / walk and menu buttons, the skill buttons, the
//! new-stats and new-skills buttons and the help button of the
//! control panel.

use super::globes::Tip;
use crate::ui::geom::{Point, Rect};
use crate::ui::messages::LineDraw;
use crate::ui::panels::PanelOutput;

/// A cel draw: frame at (x, y), light 0xFF, mode 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonCel {
    pub frame: u32,
    pub x: i32,
    pub y: i32,
}

/// Current click rectangles, shared by input and the element registry.
pub fn run_rect(w: i32, h: i32) -> Rect {
    Rect::new(w / 2 - 145, h - 28, 18, 21)
}
pub fn menu_rect(w: i32, h: i32) -> Rect {
    Rect::new(w / 2 - 8, h - 39, 14, 27)
}
/// The skill buttons end at the frame's last row (H − 1): a rectangle
/// reaching row H would hang one row below the frame.
pub fn skill_rect(side: SkillSide, w: i32, h: i32) -> Rect {
    Rect::new(
        match side {
            SkillSide::Left => 117,
            SkillSide::Right => w - 165,
        },
        h - 47,
        49,
        47,
    )
}

fn fmt_s(fmt: &[u16], args: &[&[u16]]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut a = args.iter();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] == u16::from(b'%') && fmt.get(i + 1) == Some(&u16::from(b's')) {
            if let Some(v) = a.next() {
                out.extend_from_slice(v);
            }
            i += 2;
        } else {
            out.push(fmt[i]);
            i += 1;
        }
    }
    out
}

/// The run button rectangle x W/2 − 145…W/2 − 128, y H − 28…H − 8 (inclusive).
pub fn in_run_rect(w: i32, h: i32, x: i32, y: i32) -> bool {
    run_rect(w, h).contains(Point::new(x, y))
}

/// Frame = 2 while running, else 0; + 1 while pressed with the mouse inside the
/// rectangle; `runbutton` at (W/2 − 145, H − 10).
pub fn run_button(w: i32, h: i32, running: bool, pressed: bool, mouse: (i32, i32)) -> ButtonCel {
    let mut frame = if running { 2 } else { 0 };
    if pressed && in_run_rect(w, h, mouse.0, mouse.1) {
        frame += 1;
    }
    ButtonCel {
        frame,
        x: w / 2 - 145,
        y: h - 10,
    }
}

/// Hovering the rectangle shows `RunOn` (4179,
/// "Run") + for the primary and secondary key of binding 0x23 ` (%s)`
/// (4178) with the key name, at (W/2 − 145, H − 23), color 0, centred.
pub fn run_tip(
    w: i32,
    h: i32,
    mouse: (i32, i32),
    keys: [Option<Vec<u16>>; 2],
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Option<Tip> {
    if !in_run_rect(w, h, mouse.0, mouse.1) {
        return None;
    }
    let mut text = strings(4179);
    for k in keys.iter().flatten() {
        text.extend(fmt_s(&strings(4178), &[k]));
    }
    Some(Tip {
        text,
        x: w / 2 - 145,
        y: h - 23,
        color: 0,
        centered: true,
    })
}

/// The menu button rectangle x W/2 − 8…W/2 + 5, y H − 39…H − 13.
pub fn in_menu_rect(w: i32, h: i32, x: i32, y: i32) -> bool {
    menu_rect(w, h).contains(Point::new(x, y))
}

/// Frame = 2 while state 0x15 (mini panel) is open, else 0; + 1 while pressed
/// inside the rectangle; `menubutton` at (W/2 − 8, H − 16).
pub fn menu_button(
    w: i32,
    h: i32,
    mini_panel_open: bool,
    pressed: bool,
    mouse: (i32, i32),
) -> ButtonCel {
    let mut frame = if mini_panel_open { 2 } else { 0 };
    if pressed && in_menu_rect(w, h, mouse.0, mouse.1) {
        frame += 1;
    }
    ButtonCel {
        frame,
        x: w / 2 - 8,
        y: h - 16,
    }
}

/// The mouse in the menu-button rectangle shows
/// `panelcmini` (4168, "Close Mini Panel") while state 0x15 is open, else
/// `panelmini` (4167, "Open Mini Panel"), at (W/2 − 1, H − 39), color 0,
/// centred.
pub fn menu_tip(
    w: i32,
    h: i32,
    mini_panel_open: bool,
    mouse: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Option<Tip> {
    if !in_menu_rect(w, h, mouse.0, mouse.1) {
        return None;
    }
    Some(Tip {
        text: strings(if mini_panel_open { 4168 } else { 4167 }),
        x: w / 2 - 1,
        y: h - 39,
        color: 0,
        centered: true,
    })
}

/// Which skill button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillSide {
    Left,
    Right,
}

/// A skill whose level for P is ≤ 0 is replaced in the model
/// before the draw (the skill list's fallback, then the left / right skill
/// set to none, then re-read): a model write
/// in a draw (cross-file request, left to the host).
pub fn skill_needs_replacement(level: i32) -> bool {
    level <= 0
}

/// The icon position: left at (117, H), flag 1; right at (W −
/// 165, H), flag 0.
pub fn skill_icon_pos(side: SkillSide, w: i32, h: i32) -> (i32, i32) {
    match side {
        SkillSide::Left => (117, h),
        SkillSide::Right => (w - 165, h),
    }
}

/// The class prefix of the icon file `Spells\<CC>Skillicon`: `Am`, `So`, `Ne`,
/// `Pa`, `Ba`, `Dr`, `As` for classes 0–6.
pub const CLASS_ICON_PREFIX: [&str; 7] = ["Am", "So", "Ne", "Pa", "Ba", "Dr", "As"];

/// The icon file of the skill's class: a class above
/// 6 gives `Spells\Skillicon`.
pub fn skill_icon_file(class: u8) -> String {
    match CLASS_ICON_PREFIX.get(usize::from(class)) {
        Some(cc) => format!("Spells\\{cc}Skillicon"),
        None => "Spells\\Skillicon".to_string(),
    }
}

/// The frame of the icon: the skilldesc `IconCel`, byte +7 of the record.
pub fn skill_icon_cel(skilldesc: &[u8]) -> Option<u8> {
    skilldesc.get(7).copied()
}

/// The icon is drawn with `CelDrawColor`, light 0xFF, mode
/// 5, the state as the color argument.
pub const ICON_LIGHT: u8 = 0xFF;
pub const ICON_MODE: u8 = 5;

/// The icon state: from the skill button state (0, 1, 4), 1 also when the
/// skill's `InTown` flag bit is clear and P stands in town; while the
/// mouse is in x…x + 48, y − 48…y the state 4 stays 4, 0 stays 0, other →
/// 1.
pub fn skill_icon_state(
    base: u8,
    flag_bit_clear: bool,
    in_town: bool,
    mouse: (i32, i32),
    at: (i32, i32),
) -> u8 {
    let mut s = base;
    if flag_bit_clear && in_town {
        s = 1;
    }
    let hovered = (at.0..=at.0 + 48).contains(&mouse.0) && (at.1 - 48..=at.1).contains(&mouse.1);
    if hovered && s != 4 && s != 0 {
        s = 1;
    }
    s
}

/// Press: x 117…165 (left) or W − 165…W − 117 (right), y > H − 48
/// and < H (the frame's last row is H − 1).
pub fn skill_press_hit(w: i32, h: i32, x: i32, y: i32) -> Option<SkillSide> {
    [SkillSide::Left, SkillSide::Right]
        .into_iter()
        .find(|&side| skill_rect(side, w, h).contains(Point::new(x, y)))
}

/// Release: toggles state 3 (skill select) and runs the skill button's pick
/// (left or right).
pub fn skill_release(side: SkillSide) -> (PanelOutput, u32) {
    (
        PanelOutput::SetUi {
            ui: 3,
            mode: 2,
            jump: false,
        },
        u32::from(side == SkillSide::Left),
    )
}

/// The new-stats / new-skills buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewBtn {
    Stats,
    Skills,
}

/// The screen facts of the new-stats / new-skills buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtnEnv {
    pub w: i32,
    pub h: i32,
    /// Resolution mode 2 (800 × 600).
    pub res2: bool,
    pub open_mode: u8,
}

/// Hover (800 × 600, strict): new stats W/2 − 194 < x < W/2 − 160,
/// new skills W/2 + 163 < x < W/2 + 197, both H − 42 < y < H − 8.
pub fn new_rect(e: &BtnEnv, which: NewBtn) -> Rect {
    let (l, r) = match which {
        NewBtn::Stats => (e.w / 2 - 194, e.w / 2 - 160),
        NewBtn::Skills => (e.w / 2 + 163, e.w / 2 + 197),
    };
    Rect::new(l + 1, e.h - 41, (r - l - 1) as u16, 33)
}

pub fn hover_800(e: &BtnEnv, which: NewBtn, x: i32, y: i32) -> bool {
    new_rect(e, which).contains(Point::new(x, y))
}

/// `Panel\Level` at (W/2 − 194, H − 8)
/// (new skills at W/2 + 163): frame 2 while state 6 (7) is closed; while it
/// is open frame 1 when pressed with the mouse inside, else 0.
pub fn draw_800(
    e: &BtnEnv,
    which: NewBtn,
    state_open: bool,
    pressed: bool,
    mouse: (i32, i32),
) -> ButtonCel {
    let x = match which {
        NewBtn::Stats => e.w / 2 - 194,
        NewBtn::Skills => e.w / 2 + 163,
    };
    let frame = if !state_open {
        2
    } else if pressed && hover_800(e, which, mouse.0, mouse.1) {
        1
    } else {
        0
    };
    ButtonCel {
        frame,
        x,
        y: e.h - 8,
    }
}

/// The hover with state 9 closed shows `strlvlup` (3986, "New
/// Stats") at (W/2 − 179, H − 50) / `strnewskl` (3987, "New Skill") at
/// (W/2 + 178, H − 50), color 0, centred.
pub fn tip_800(
    e: &BtnEnv,
    which: NewBtn,
    mouse: (i32, i32),
    state9_open: bool,
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Option<Tip> {
    if state9_open || !hover_800(e, which, mouse.0, mouse.1) {
        return None;
    }
    let (id, x) = match which {
        NewBtn::Stats => (3986, e.w / 2 - 179),
        NewBtn::Skills => (3987, e.w / 2 + 178),
    };
    Some(Tip {
        text: strings(id),
        x,
        y: e.h - 50,
        color: 0,
        centered: true,
    })
}

/// The states the 640 × 480 hiding reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Open640 {
    pub st2: bool,
    pub st4: bool,
    pub st0c: bool,
    /// State 0x16 with 1 / with 4.
    pub st16_and_1: bool,
    pub st16_and_4: bool,
}

/// Hidden (and the pressed flag
/// cleared) when the open mode is 3, or for new stats while state 2, 0x0C,
/// (0x16 and 1) or (0x16 and 4) is open, for new skills while state 4,
/// 0x0C or (0x16 and 1) is open.
pub fn hidden_640(e: &BtnEnv, which: NewBtn, o: &Open640) -> bool {
    if e.open_mode == 3 {
        return true;
    }
    match which {
        NewBtn::Stats => o.st2 || o.st0c || o.st16_and_1 || o.st16_and_4,
        NewBtn::Skills => o.st4 || o.st0c || o.st16_and_1,
    }
}

/// X0 = 40 (new stats; W/2 + 40 in open mode 2) or W − 73 (new
/// skills; W − W/2 − 73 in open mode 1).
pub fn x0_640(e: &BtnEnv, which: NewBtn) -> i32 {
    match which {
        NewBtn::Stats => {
            if e.open_mode == 2 {
                e.w / 2 + 40
            } else {
                40
            }
        }
        NewBtn::Skills => {
            if e.open_mode == 1 {
                e.w - e.w / 2 - 73
            } else {
                e.w - 73
            }
        }
    }
}

/// Inside (strict): new stats x0 < x < x0 + 34 in open mode 2, else 41 ≤ x ≤
/// 73; new skills x0' − 73 < x < x0' − 40 with x0' = W (W − W/2 in open mode
/// 1); y H − 139 < y < H − 102 / H − 138 < y < H − 102.
pub fn inside_640(e: &BtnEnv, which: NewBtn, x: i32, y: i32) -> bool {
    match which {
        NewBtn::Stats => {
            let xin = if e.open_mode == 2 {
                let x0 = x0_640(e, which);
                x0 < x && x < x0 + 34
            } else {
                (41..=73).contains(&x)
            };
            xin && e.h - 139 < y && y < e.h - 102
        }
        NewBtn::Skills => {
            let x0p = if e.open_mode == 1 { e.w - e.w / 2 } else { e.w };
            x0p - 73 < x && x < x0p - 40 && e.h - 138 < y && y < e.h - 102
        }
    }
}

/// The 640 × 480 draw: the caption (3986 / 3987) in the current
/// font at (x0 + 1 + cw / 2 − tw / 2, H − 142), color 0 (cw = `Levelsocket`
/// frame width, tw = width A); `Levelsocket` frame 0 at (x0, H − 105);
/// `Level` frame (1 pressed and inside, else 0) at (x0 + 3, H − 109).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Btn640Draw {
    pub caption: LineDraw,
    pub socket: ButtonCel,
    pub level: ButtonCel,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_640(
    e: &BtnEnv,
    which: NewBtn,
    o: &Open640,
    state_open: bool,
    pressed: bool,
    mouse: (i32, i32),
    socket_w: i32,
    strings: &dyn Fn(u16) -> Vec<u16>,
    width_a: &dyn Fn(&[u16]) -> i32,
) -> Option<Btn640Draw> {
    if !state_open || hidden_640(e, which, o) {
        return None;
    }
    let x0 = x0_640(e, which);
    let text = strings(match which {
        NewBtn::Stats => 3986,
        NewBtn::Skills => 3987,
    });
    let tw = width_a(&text);
    let level = u32::from(pressed && inside_640(e, which, mouse.0, mouse.1));
    Some(Btn640Draw {
        caption: LineDraw {
            text,
            x: x0 + 1 + socket_w / 2 - tw / 2,
            y: e.h - 142,
            color: 0,
        },
        socket: ButtonCel {
            frame: 0,
            x: x0,
            y: e.h - 105,
        },
        level: ButtonCel {
            frame: level,
            x: x0 + 3,
            y: e.h - 109,
        },
    })
}

/// What the new-stats / new-skills handlers ask of the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtnEffect {
    Out(PanelOutput),
    /// The cursor press.
    CursorPress,
    /// The cursor release.
    CursorRelease,
}

/// The pressed flags of the two buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NewButtons {
    pub stats_pressed: bool,
    pub skills_pressed: bool,
}

impl NewButtons {
    fn flag(&mut self, which: NewBtn) -> &mut bool {
        match which {
            NewBtn::Stats => &mut self.stats_pressed,
            NewBtn::Skills => &mut self.skills_pressed,
        }
    }

    fn hit(e: &BtnEnv, which: NewBtn, x: i32, y: i32, mouse: (i32, i32)) -> bool {
        if e.res2 {
            hover_800(e, which, mouse.0, mouse.1)
        } else {
            inside_640(e, which, x, y)
        }
    }

    /// Press (WM_LBUTTONDOWN): nothing at 800 × 600 while state 9
    /// is open. A hit sets the pressed flag, plays sound 4, runs the cursor
    /// press and consumes the event, except when the open mode is 2 (new stats)
    /// / 1 (new skills) and the NPC menu is up: then not
    /// consumed. A miss is not consumed. `(x, y)` is the event, `mouse` the
    /// current mouse.
    #[allow(clippy::too_many_arguments)]
    pub fn press(
        &mut self,
        e: &BtnEnv,
        which: NewBtn,
        x: i32,
        y: i32,
        mouse: (i32, i32),
        state9_open: bool,
        npc_menu_up: bool,
    ) -> (Vec<BtnEffect>, bool) {
        if e.res2 && state9_open {
            return (Vec::new(), false);
        }
        if !Self::hit(e, which, x, y, mouse) {
            return (Vec::new(), false);
        }
        *self.flag(which) = true;
        let eff = vec![
            // Id 4.
            BtnEffect::Out(PanelOutput::Sound(4)),
            BtnEffect::CursorPress,
        ];
        let special = match which {
            NewBtn::Stats => e.open_mode == 2,
            NewBtn::Skills => e.open_mode == 1,
        };
        (eff, !(special && npc_menu_up))
    }

    /// Release (WM_LBUTTONUP): nothing at 800 × 600 while state 9
    /// is open; else the cursor release. Pressed and a hit → pressed:= 0 and,
    /// 800 × 600: `SetUIState(2, on, 0)` (new stats) / `SetUIState(4, on, 0)`
    /// (new skills); 640 × 480: `SetUIState(6, off, 0)` then `SetUIState(2, on,
    /// 0)` / `SetUIState(7, off, 0)` then `SetUIState(4, on, 0)`; consumed.
    /// Otherwise pressed:= 0, not consumed.
    pub fn release(
        &mut self,
        e: &BtnEnv,
        which: NewBtn,
        x: i32,
        y: i32,
        mouse: (i32, i32),
        state9_open: bool,
    ) -> (Vec<BtnEffect>, bool) {
        if e.res2 && state9_open {
            return (Vec::new(), false);
        }
        let mut eff = vec![BtnEffect::CursorRelease];
        let pressed = *self.flag(which);
        if pressed && Self::hit(e, which, x, y, mouse) {
            *self.flag(which) = false;
            let (off_ui, on_ui) = match which {
                NewBtn::Stats => (6, 2),
                NewBtn::Skills => (7, 4),
            };
            let set = |ui, mode| {
                BtnEffect::Out(PanelOutput::SetUi {
                    ui,
                    mode,
                    jump: false,
                })
            };
            if !e.res2 {
                eff.push(set(off_ui, 1));
            }
            eff.push(set(on_ui, 0));
            (eff, true)
        } else {
            *self.flag(which) = false;
            (eff, false)
        }
    }
}

/// The help button's state (ui state 0x22): it opens at game entry beside the
/// mini panel.
pub const UI_HELP_BUTTON: u8 = 0x22;
/// The help screen (toggled by the release).
pub const UI_HELP_SCREEN: u8 = 0x21;

/// The states that hide the button (hidden flag).
pub const HELP_HIDING: [u8; 6] = [4, 3, 1, 0x0C, 0x17, 0x19];

/// The help button's state: pressed, hidden at the last draw
/// and the `Help Menu` cache. D2RUST's own: there is no registry, so the value
/// is absent (0) at every start and the cache lives as long as the HUD.
#[derive(Clone, Debug, Default)]
pub struct HelpButton {
    pub pressed: bool,
    pub hidden: std::cell::Cell<bool>,
    pub setting: u32,
}

/// What the help button draws for its caption width `text_w`
/// (width A): the caption's left / top, the socket and the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpDraw {
    pub caption: (i32, i32),
    pub socket: ButtonCel,
    pub button: ButtonCel,
}

impl HelpButton {
    /// With state 0x22 open: `None` when hidden (one of
    /// [`HELP_HIDING`] open; the flag is kept for the later rules) or when the
    /// setting is ≠ 0 (the game then closes 0x22; in D2RUST the setting
    /// only becomes ≠ 0 through the button's own rules, which close it already).
    pub fn draw(&self, w: i32, h: i32, open: &dyn Fn(u8) -> bool, text_w: i32) -> Option<HelpDraw> {
        let hidden = HELP_HIDING.iter().any(|&u| open(u));
        self.hidden.set(hidden);
        if hidden || self.setting != 0 {
            return None;
        }
        Some(HelpDraw {
            caption: (w - 58 - text_w / 2, h - 197),
            socket: ButtonCel {
                frame: 0,
                x: w - 75,
                y: h - 160,
            },
            button: ButtonCel {
                frame: u32::from(self.pressed),
                x: w - 72,
                y: h - 164,
            },
        })
    }

    /// R4, r5: the hit box W − 75 ≤ x ≤ W − 40, H − 196 ≤ y ≤ H − 160
    /// (inclusive), at the current mouse.
    pub fn rect(w: i32, h: i32) -> Rect {
        Rect::new(w - 75, h - 196, 36, 37)
    }

    pub fn hit(w: i32, h: i32, x: i32, y: i32) -> bool {
        Self::rect(w, h).contains(Point::new(x, y))
    }

    /// Press: a hit sets pressed and is consumed (no sound, no
    /// cursor press); a miss or a hidden button is not.
    pub fn press(&mut self, w: i32, h: i32, mouse: (i32, i32)) -> bool {
        if self.hidden.get() || !Self::hit(w, h, mouse.0, mouse.1) {
            return false;
        }
        self.pressed = true;
        true
    }

    /// Release (pressed not checked): a hit closes 0x22, toggles
    /// the help screen 0x21, sets `Help Menu` and the cache to 1, clears
    /// pressed; `None` (not consumed) on a miss, pressed then stays.
    pub fn release(&mut self, w: i32, h: i32, mouse: (i32, i32)) -> Option<[PanelOutput; 2]> {
        if self.hidden.get() || !Self::hit(w, h, mouse.0, mouse.1) {
            return None;
        }
        self.setting = 1;
        self.pressed = false;
        Some([
            PanelOutput::SetUi {
                ui: UI_HELP_BUTTON,
                mode: 1,
                jump: false,
            },
            PanelOutput::SetUi {
                ui: UI_HELP_SCREEN,
                mode: 2,
                jump: false,
            },
        ])
    }
}

/// Caption: string 4177 ("Help") then, for command 6 (`CfgHelp`)
/// slot 1 then slot 0 when bound, ` (%s)` (4178) with the key's name.
pub fn help_caption(keys: [Option<Vec<u16>>; 2], strings: &dyn Fn(u16) -> Vec<u16>) -> Vec<u16> {
    let mut text = strings(4177);
    for k in [&keys[1], &keys[0]].into_iter().flatten() {
        text.extend(fmt_s(&strings(4178), &[k]));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(id: u16) -> Vec<u16> {
        let s = match id {
            4179 => "Run",
            4178 => " (%s)",
            4167 => "Open Mini Panel",
            4168 => "Close Mini Panel",
            3986 => "New Stats",
            3987 => "New Skill",
            _ => "?",
        };
        s.encode_utf16().collect()
    }

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn run_button_and_tip() {
        let (w, h) = (800, 600);
        // Frame 2 while running, 0 otherwise; + 1 while pressed with the
        // mouse inside x W/2 − 145…W/2 − 128, y H − 28…H − 8.
        assert_eq!(
            run_button(w, h, false, false, (0, 0)),
            ButtonCel {
                frame: 0,
                x: 255,
                y: 590
            }
        );
        assert_eq!(run_button(w, h, true, false, (0, 0)).frame, 2);
        assert_eq!(run_button(w, h, true, true, (260, 580)).frame, 3);
        assert_eq!(run_button(w, h, false, true, (260, 580)).frame, 1);
        assert_eq!(run_button(w, h, false, true, (0, 0)).frame, 0);
        for (x, y, inside) in [
            (255, 572, true),
            (272, 592, true),
            (254, 580, false),
            (273, 580, false),
            (260, 571, false),
            (260, 593, false),
        ] {
            assert_eq!(in_run_rect(w, h, x, y), inside, "({x}, {y})");
        }
        // The tool tip: "Run" + " (%s)" for the primary and secondary key.
        let t = run_tip(
            w,
            h,
            (260, 580),
            [Some(u("R")), Some(u("Shift+R"))],
            &strings,
        )
        .unwrap();
        assert_eq!(
            t,
            Tip {
                text: u("Run (R) (Shift+R)"),
                x: 255,
                y: 577,
                color: 0,
                centered: true
            }
        );
        let t = run_tip(w, h, (260, 580), [None, Some(u("R"))], &strings).unwrap();
        assert_eq!(t.text, u("Run (R)"));
        assert_eq!(
            run_tip(w, h, (260, 580), [None, None], &strings)
                .unwrap()
                .text,
            u("Run")
        );
        assert!(run_tip(w, h, (0, 0), [None, None], &strings).is_none());
    }

    #[test]
    fn menu_button_frames() {
        let (w, h) = (800, 600);
        assert_eq!(
            menu_button(w, h, false, false, (0, 0)),
            ButtonCel {
                frame: 0,
                x: 392,
                y: 584
            }
        );
        assert_eq!(menu_button(w, h, true, false, (0, 0)).frame, 2);
        assert_eq!(menu_button(w, h, true, true, (400, 570)).frame, 3);
        assert_eq!(menu_button(w, h, false, true, (400, 570)).frame, 1);
        assert_eq!(menu_button(w, h, false, true, (390, 570)).frame, 0);
        // x W/2 − 8…W/2 + 5, y H − 39…H − 13.
        assert!(in_menu_rect(w, h, 392, 561));
        assert!(in_menu_rect(w, h, 405, 587));
        assert!(!in_menu_rect(w, h, 391, 570));
        assert!(!in_menu_rect(w, h, 406, 570));
        assert!(!in_menu_rect(w, h, 400, 560));
        assert!(!in_menu_rect(w, h, 400, 588));
    }

    #[test]
    fn menu_tip_by_state() {
        let t = menu_tip(800, 600, false, (400, 570), &strings).unwrap();
        assert_eq!(
            t,
            Tip {
                text: u("Open Mini Panel"),
                x: 399,
                y: 561,
                color: 0,
                centered: true
            }
        );
        assert_eq!(
            menu_tip(800, 600, true, (400, 570), &strings).unwrap().text,
            u("Close Mini Panel")
        );
        assert!(menu_tip(800, 600, true, (300, 570), &strings).is_none());
    }

    #[test]
    fn skill_buttons() {
        // A level ≤ 0 is replaced before the draw.
        assert!(skill_needs_replacement(0));
        assert!(skill_needs_replacement(-1));
        assert!(!skill_needs_replacement(1));
        // Icons: left at (117, H), right at (W − 165, H).
        assert_eq!(skill_icon_pos(SkillSide::Left, 800, 600), (117, 600));
        assert_eq!(skill_icon_pos(SkillSide::Right, 800, 600), (635, 600));
        // State: 1 also in town with the flag bit clear; hovered: 4 and 0
        // stay, other → 1.
        assert_eq!(skill_icon_state(0, false, true, (0, 0), (117, 600)), 0);
        assert_eq!(skill_icon_state(0, true, true, (0, 0), (117, 600)), 1);
        assert_eq!(skill_icon_state(4, true, true, (0, 0), (117, 600)), 1);
        assert_eq!(skill_icon_state(4, true, false, (0, 0), (117, 600)), 4);
        assert_eq!(skill_icon_state(4, false, false, (120, 580), (117, 600)), 4);
        assert_eq!(skill_icon_state(0, false, false, (120, 580), (117, 600)), 0);
        assert_eq!(skill_icon_state(1, false, false, (120, 580), (117, 600)), 1);
        assert_eq!(skill_icon_state(7, false, false, (120, 580), (117, 600)), 1);
        assert_eq!(skill_icon_state(7, false, false, (0, 0), (117, 600)), 7);
        // The icon file of the skill's class and the frame IconCel.
        assert_eq!(skill_icon_file(0), "Spells\\AmSkillicon");
        assert_eq!(skill_icon_file(6), "Spells\\AsSkillicon");
        assert_eq!(skill_icon_file(7), "Spells\\Skillicon");
        assert_eq!(skill_icon_cel(&[0, 0, 0, 0, 0, 0, 0, 21, 9]), Some(21));
        assert_eq!(skill_icon_cel(&[0; 7]), None);
        assert_eq!((ICON_LIGHT, ICON_MODE), (0xFF, 5));
        // Press: x 117…165 or W − 165…W − 117, y > H − 48 and < H.
        for (x, y, side) in [
            (117, 599, Some(SkillSide::Left)),
            (117, 600, None),
            (165, 553, Some(SkillSide::Left)),
            (116, 580, None),
            (166, 580, None),
            (635, 580, Some(SkillSide::Right)),
            (683, 580, Some(SkillSide::Right)),
            (634, 580, None),
            (684, 580, None),
            (120, 552, None),
            (120, 601, None),
        ] {
            assert_eq!(skill_press_hit(800, 600, x, y), side, "({x}, {y})");
        }
        // Both buttons stay inside the frame (no row below H − 1).
        for (w, h) in [(640, 480), (800, 600)] {
            for side in [SkillSide::Left, SkillSide::Right] {
                let r = skill_rect(side, w, h);
                assert!(r.x >= 0 && r.y >= 0, "{side:?} {r:?}");
                assert!(
                    r.right() <= i64::from(w) && r.bottom() <= i64::from(h),
                    "{side:?} {r:?}"
                );
            }
        }
        // Release: toggles state 3 and runs the left / right skill button's pick.
        assert_eq!(
            skill_release(SkillSide::Left),
            (
                PanelOutput::SetUi {
                    ui: 3,
                    mode: 2,
                    jump: false
                },
                1
            )
        );
        assert_eq!(skill_release(SkillSide::Right).1, 0);
    }

    fn e800(open_mode: u8) -> BtnEnv {
        BtnEnv {
            w: 800,
            h: 600,
            res2: true,
            open_mode,
        }
    }

    fn e640(open_mode: u8) -> BtnEnv {
        BtnEnv {
            w: 640,
            h: 480,
            res2: false,
            open_mode,
        }
    }

    #[test]
    fn new_buttons_800() {
        let e = e800(0);
        // Hover (strict): stats W/2 − 194 < x < W/2 − 160, skills W/2 + 163
        // < x < W/2 + 197, y H − 42 < y < H − 8.
        assert!(hover_800(&e, NewBtn::Stats, 207, 559));
        assert!(hover_800(&e, NewBtn::Stats, 239, 591));
        assert!(!hover_800(&e, NewBtn::Stats, 206, 570));
        assert!(!hover_800(&e, NewBtn::Stats, 240, 570));
        assert!(!hover_800(&e, NewBtn::Stats, 220, 558));
        assert!(!hover_800(&e, NewBtn::Stats, 220, 592));
        assert!(hover_800(&e, NewBtn::Skills, 564, 570));
        assert!(!hover_800(&e, NewBtn::Skills, 563, 570));
        assert!(hover_800(&e, NewBtn::Skills, 596, 570));
        assert!(!hover_800(&e, NewBtn::Skills, 597, 570));
        // Frames: 2 while state 6 / 7 is closed; open: 1 when pressed with
        // the mouse inside, else 0; at (W/2 − 194, H − 8) / (W/2 + 163, …).
        assert_eq!(
            draw_800(&e, NewBtn::Stats, false, true, (220, 570)),
            ButtonCel {
                frame: 2,
                x: 206,
                y: 592
            }
        );
        assert_eq!(draw_800(&e, NewBtn::Stats, true, true, (220, 570)).frame, 1);
        assert_eq!(
            draw_800(&e, NewBtn::Stats, true, false, (220, 570)).frame,
            0
        );
        assert_eq!(draw_800(&e, NewBtn::Stats, true, true, (100, 570)).frame, 0);
        assert_eq!(draw_800(&e, NewBtn::Skills, false, false, (0, 0)).x, 563);
        // Tool tips with state 9 closed.
        let t = tip_800(&e, NewBtn::Stats, (220, 570), false, &strings).unwrap();
        assert_eq!(
            t,
            Tip {
                text: u("New Stats"),
                x: 221,
                y: 550,
                color: 0,
                centered: true
            }
        );
        let t = tip_800(&e, NewBtn::Skills, (580, 570), false, &strings).unwrap();
        assert_eq!((t.text, t.x, t.y), (u("New Skill"), 578, 550));
        assert!(tip_800(&e, NewBtn::Stats, (220, 570), true, &strings).is_none());
        assert!(tip_800(&e, NewBtn::Stats, (100, 570), false, &strings).is_none());
    }

    #[test]
    fn new_buttons_640() {
        let e = e640(0);
        // x0: 40 / W/2 + 40 (mode 2); W − 73 / W − W/2 − 73 (mode 1).
        assert_eq!(x0_640(&e, NewBtn::Stats), 40);
        assert_eq!(x0_640(&e640(2), NewBtn::Stats), 360);
        assert_eq!(x0_640(&e, NewBtn::Skills), 567);
        assert_eq!(x0_640(&e640(1), NewBtn::Skills), 247);
        // Inside (strict): stats 41 ≤ x ≤ 73 (x0 < x < x0 + 34 in mode 2),
        // H − 139 < y < H − 102; skills x0' − 73 < x < x0' − 40, H − 138 <
        // y < H − 102.
        assert!(inside_640(&e, NewBtn::Stats, 41, 342));
        assert!(inside_640(&e, NewBtn::Stats, 73, 377));
        assert!(!inside_640(&e, NewBtn::Stats, 40, 360));
        assert!(!inside_640(&e, NewBtn::Stats, 74, 360));
        assert!(!inside_640(&e, NewBtn::Stats, 50, 341));
        assert!(!inside_640(&e, NewBtn::Stats, 50, 378));
        assert!(inside_640(&e640(2), NewBtn::Stats, 361, 360));
        assert!(inside_640(&e640(2), NewBtn::Stats, 393, 360));
        assert!(!inside_640(&e640(2), NewBtn::Stats, 360, 360));
        assert!(!inside_640(&e640(2), NewBtn::Stats, 394, 360));
        assert!(inside_640(&e, NewBtn::Skills, 568, 343));
        assert!(inside_640(&e, NewBtn::Skills, 599, 377));
        assert!(!inside_640(&e, NewBtn::Skills, 567, 360));
        assert!(!inside_640(&e, NewBtn::Skills, 600, 360));
        assert!(!inside_640(&e, NewBtn::Skills, 580, 342));
        assert!(inside_640(&e640(1), NewBtn::Skills, 248, 360));
        assert!(!inside_640(&e640(1), NewBtn::Skills, 247, 360));
        assert!(!inside_640(&e640(1), NewBtn::Skills, 280, 360));
        // Hidden: open mode 3; stats with 2, 0x0C, (0x16, 1), (0x16, 4);
        // skills with 4, 0x0C, (0x16, 1).
        let none = Open640::default();
        assert!(!hidden_640(&e, NewBtn::Stats, &none));
        assert!(hidden_640(&e640(3), NewBtn::Stats, &none));
        assert!(hidden_640(&e640(3), NewBtn::Skills, &none));
        for (o, stats, skills) in [
            (Open640 { st2: true, ..none }, true, false),
            (Open640 { st4: true, ..none }, false, true),
            (Open640 { st0c: true, ..none }, true, true),
            (
                Open640 {
                    st16_and_1: true,
                    ..none
                },
                true,
                true,
            ),
            (
                Open640 {
                    st16_and_4: true,
                    ..none
                },
                true,
                false,
            ),
        ] {
            assert_eq!(hidden_640(&e, NewBtn::Stats, &o), stats, "{o:?}");
            assert_eq!(hidden_640(&e, NewBtn::Skills, &o), skills, "{o:?}");
        }
        // The draw: caption at (x0 + 1 + cw / 2 − tw / 2, H − 142),
        // Levelsocket frame 0 at (x0, H − 105), Level at (x0 + 3, H − 109).
        let width = |t: &[u16]| 8 * t.len() as i32;
        let d = draw_640(
            &e,
            NewBtn::Stats,
            &none,
            true,
            true,
            (50, 360),
            40,
            &strings,
            &width,
        )
        .unwrap();
        assert_eq!(
            d,
            Btn640Draw {
                caption: LineDraw {
                    text: u("New Stats"),
                    x: 40 + 1 + 20 - 36,
                    y: 338,
                    color: 0
                },
                socket: ButtonCel {
                    frame: 0,
                    x: 40,
                    y: 375
                },
                level: ButtonCel {
                    frame: 1,
                    x: 43,
                    y: 371
                },
            }
        );
        // Unpressed or outside: Level frame 0. Closed state or hidden: none.
        let d = draw_640(
            &e,
            NewBtn::Stats,
            &none,
            true,
            false,
            (50, 360),
            40,
            &strings,
            &width,
        )
        .unwrap();
        assert_eq!(d.level.frame, 0);
        assert!(draw_640(
            &e,
            NewBtn::Stats,
            &none,
            false,
            false,
            (0, 0),
            40,
            &strings,
            &width
        )
        .is_none());
        assert!(draw_640(
            &e640(3),
            NewBtn::Stats,
            &none,
            true,
            false,
            (0, 0),
            40,
            &strings,
            &width
        )
        .is_none());
    }

    #[test]
    fn new_buttons_press() {
        // 800 × 600: the hover rectangles with the current mouse.
        let e = e800(0);
        let mut b = NewButtons::default();
        let (eff, c) = b.press(&e, NewBtn::Stats, 0, 0, (220, 570), false, false);
        assert_eq!(
            eff,
            vec![
                BtnEffect::Out(PanelOutput::Sound(4)),
                BtnEffect::CursorPress
            ]
        );
        assert!(c && b.stats_pressed && !b.skills_pressed);
        // A miss is not consumed, nothing set.
        let mut b = NewButtons::default();
        let (eff, c) = b.press(&e, NewBtn::Stats, 0, 0, (100, 570), false, false);
        assert!(eff.is_empty() && !c && !b.stats_pressed);
        // Nothing at 800 × 600 while state 9 is open.
        let (eff, c) = b.press(&e, NewBtn::Stats, 0, 0, (220, 570), true, false);
        assert!(eff.is_empty() && !c && !b.stats_pressed);
        // 640 × 480: the inside rectangles with the event's x, y; state 9
        // does not matter.
        let e = e640(0);
        let mut b = NewButtons::default();
        let (_, c) = b.press(&e, NewBtn::Stats, 50, 360, (0, 0), true, false);
        assert!(c && b.stats_pressed);
        let mut b = NewButtons::default();
        let (_, c) = b.press(&e, NewBtn::Skills, 580, 360, (0, 0), false, false);
        assert!(c && b.skills_pressed);
        // Open mode 2 (stats) / 1 (skills) with the NPC menu up: armed and
        // sounded, but the event is left to the next handler.
        let mut b = NewButtons::default();
        let (eff, c) = b.press(&e640(2), NewBtn::Stats, 370, 360, (0, 0), false, true);
        assert!(!c && b.stats_pressed && eff.len() == 2);
        let mut b = NewButtons::default();
        let (_, c) = b.press(&e640(1), NewBtn::Skills, 250, 360, (0, 0), false, true);
        assert!(!c && b.skills_pressed);
        // Not for the other mode / the other button.
        let mut b = NewButtons::default();
        let (_, c) = b.press(&e640(1), NewBtn::Stats, 50, 360, (0, 0), false, true);
        assert!(c);
        let mut b = NewButtons::default();
        let (_, c) = b.press(&e640(2), NewBtn::Skills, 580, 360, (0, 0), false, true);
        assert!(c);
    }

    #[test]
    fn new_buttons_release() {
        let on = |ui, mode| {
            BtnEffect::Out(PanelOutput::SetUi {
                ui,
                mode,
                jump: false,
            })
        };
        // 800 × 600: SetUIState(2, on, 0) / SetUIState(4, on, 0); consumed.
        let e = e800(0);
        let mut b = NewButtons {
            stats_pressed: true,
            skills_pressed: true,
        };
        let (eff, c) = b.release(&e, NewBtn::Stats, 0, 0, (220, 570), false);
        assert_eq!(eff, vec![BtnEffect::CursorRelease, on(2, 0)]);
        assert!(c && !b.stats_pressed && b.skills_pressed);
        let (eff, c) = b.release(&e, NewBtn::Skills, 0, 0, (580, 570), false);
        assert_eq!(eff, vec![BtnEffect::CursorRelease, on(4, 0)]);
        assert!(c && !b.skills_pressed);
        // 640 × 480: SetUIState(6, off, 0) then (2, on, 0); (7, off) then
        // (4, on).
        let e = e640(0);
        let mut b = NewButtons {
            stats_pressed: true,
            skills_pressed: true,
        };
        let (eff, c) = b.release(&e, NewBtn::Stats, 50, 360, (0, 0), true);
        assert_eq!(eff, vec![BtnEffect::CursorRelease, on(6, 1), on(2, 0)]);
        assert!(c);
        let (eff, _) = b.release(&e, NewBtn::Skills, 580, 360, (0, 0), false);
        assert_eq!(eff, vec![BtnEffect::CursorRelease, on(7, 1), on(4, 0)]);
        // Pressed but released outside, or never pressed: cleared, not
        // consumed (no press check beyond the flag).
        let mut b = NewButtons {
            stats_pressed: true,
            ..Default::default()
        };
        let (eff, c) = b.release(&e, NewBtn::Stats, 200, 360, (0, 0), false);
        assert_eq!(eff, vec![BtnEffect::CursorRelease]);
        assert!(!c && !b.stats_pressed);
        let mut b = NewButtons::default();
        let (_, c) = b.release(&e, NewBtn::Stats, 50, 360, (0, 0), false);
        assert!(!c);
        // Nothing at 800 × 600 while state 9 is open.
        let mut b = NewButtons {
            stats_pressed: true,
            ..Default::default()
        };
        let (eff, c) = b.release(&e800(0), NewBtn::Stats, 0, 0, (220, 570), true);
        assert!(eff.is_empty() && !c && b.stats_pressed);
    }

    #[test]
    fn the_help_button_draws_at_the_recorded_places_and_opens_help_once() {
        // `a4-town-pandemonium-fortress` rows 258–267 (800 × 600, H bound
        // only): caption at y 403, socket (725, 440), button (728, 436).
        let mut b = HelpButton::default();
        let none = |_: u8| false;
        let d = b.draw(800, 600, &none, 56).unwrap();
        assert_eq!(d.caption, (714, 403));
        assert_eq!((d.socket.x, d.socket.y, d.socket.frame), (725, 440, 0));
        assert_eq!((d.button.x, d.button.y, d.button.frame), (728, 436, 0));
        // Hidden while the inventory is open; a hidden button takes no
        // press.
        assert!(b.draw(800, 600, &|u| u == 1, 56).is_none());
        assert!(!b.press(800, 600, (740, 420)));
        b.draw(800, 600, &none, 56);
        // The hit box edges (inclusive).
        assert!(HelpButton::hit(800, 600, 725, 404) && HelpButton::hit(800, 600, 760, 440));
        assert!(!HelpButton::hit(800, 600, 724, 420) && !HelpButton::hit(800, 600, 761, 420));
        assert!(!HelpButton::hit(800, 600, 740, 403) && !HelpButton::hit(800, 600, 740, 441));
        assert!(b.press(800, 600, (740, 420)));
        assert_eq!(b.draw(800, 600, &none, 56).unwrap().button.frame, 1);
        // A release elsewhere does nothing; pressed stays.
        assert!(b.release(800, 600, (100, 100)).is_none());
        assert!(b.pressed);
        let out = b.release(800, 600, (740, 420)).unwrap();
        assert!(matches!(
            out[0],
            PanelOutput::SetUi {
                ui: 0x22,
                mode: 1,
                ..
            }
        ));
        assert!(matches!(
            out[1],
            PanelOutput::SetUi {
                ui: 0x21,
                mode: 2,
                ..
            }
        ));
        assert!(!b.pressed);
        assert!(b.draw(800, 600, &none, 56).is_none(), "the setting is set");
        // 640 × 480: y 283, (565, 320), (568, 316).
        let d = HelpButton::default().draw(640, 480, &none, 0).unwrap();
        assert_eq!(
            (d.caption.1, d.socket.x, d.socket.y, d.button.x, d.button.y),
            (283, 565, 320, 568, 316)
        );
    }

    #[test]
    fn the_help_caption_names_slot_one_then_slot_zero() {
        let s = |id: u16| -> Vec<u16> {
            match id {
                4177 => "Help",
                4178 => " (%s)",
                _ => "",
            }
            .encode_utf16()
            .collect()
        };
        let u = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(help_caption([Some(u("H")), None], &s), u("Help (H)"));
        assert_eq!(
            help_caption([Some(u("H")), Some(u("F1"))], &s),
            u("Help (F1) (H)")
        );
        assert_eq!(help_caption([None, None], &s), u("Help"));
    }
}
