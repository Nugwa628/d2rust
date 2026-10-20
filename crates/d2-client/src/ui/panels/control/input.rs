//! The control panel mouse input: down and up, and the NPC-menu-up test.

use super::buttons::{in_menu_rect, in_run_rect, skill_press_hit, SkillSide};
use super::globes::{text_toggle, TextToggle};
use crate::ui::geom::{Point, Rect};
use crate::ui::messages::msg_u32s;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::PanelOutput;

/// Mini panel and skill select states.
const UI_MINI: u8 = 0x15;
const UI_SKILL_SELECT: u8 = 3;

/// Globe text toggle hit boxes within the routed control-panel strip.
pub fn text_toggle_rect(w: i32, h: i32, which: TextToggle) -> Rect {
    Rect::new(
        match which {
            TextToggle::Hp => 30,
            TextToggle::Mp => w - 111,
        },
        h - 47,
        81,
        33,
    )
}

/// Inputs shared by down and up.
#[derive(Clone, Copy, Debug)]
pub struct InputEnv {
    pub w: i32,
    pub h: i32,
    /// ≠ 0: input blocked.
    pub blocked: bool,
    /// P exists and is alive.
    pub alive: bool,
    /// The mouse is over the belt.
    pub over_belt: bool,
}

/// What the handlers ask of the rest of the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtrlEffect {
    /// UI sound 4.
    Sound(u32),
    /// A text toggle stored at once in the registry.
    StoreRegistry {
        which: TextToggle,
        on: bool,
    },
    Ui(PanelOutput),
    /// Cursor mode 6 → the cursor item's drop.
    CursorMode6,
    /// Cursor mode 8 → C→S 0x4C (−1).
    Send(ClientIntent),
    /// The belt click.
    BeltClick,
    SkillSelect(bool),
    /// Run:= !run.
    ToggleRun,
}

/// The mouse handlers' globals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CtrlInput {
    /// A press was recorded.
    pub press_recorded: bool,
    /// The skill buttons pressed.
    pub left_skill: bool,
    pub right_skill: bool,
    /// The menu button pressed.
    pub menu_pressed: bool,
    /// The run button pressed.
    pub run_pressed: bool,
    /// A press below y = H − 48 was consumed.
    pub flag_722358: bool,
    /// `Show HP Text`, `Show MP Text`.
    pub show_hp: bool,
    pub show_mp: bool,
}

/// The facts of a mouse up.
#[derive(Clone, Copy, Debug)]
pub struct UpFacts {
    /// The belt is popped.
    pub belt_popped: bool,
    pub cursor_mode: u8,
    /// State 3 (skill select) and 0x15 (mini panel) are open.
    pub state3_open: bool,
    pub state15_open: bool,
}

impl CtrlInput {
    /// Down: ignored while input is blocked or P is
    /// dead / absent. Returns the effects and whether the event is
    /// consumed.
    pub fn mouse_down(&mut self, e: &InputEnv, x: i32, y: i32) -> (Vec<CtrlEffect>, bool) {
        if e.blocked || !e.alive {
            return (Vec::new(), false);
        }
        // Over the belt: press recorded:= 1, consumed.
        if e.over_belt {
            self.press_recorded = true;
            return (Vec::new(), true);
        }
        // y ≤ H − 48: not consumed.
        if y <= e.h - 48 {
            return (Vec::new(), false);
        }
        let mut eff = Vec::new();
        match skill_press_hit(e.w, e.h, x, y) {
            // Left skill → press recorded:= left skill:= 1; right skill →
            // right skill.
            Some(SkillSide::Left) => {
                self.press_recorded = true;
                self.left_skill = true;
            }
            Some(SkillSide::Right) => {
                self.press_recorded = true;
                self.right_skill = true;
            }
            None => {
                if let Some(t) = text_toggle(e.w, e.h, x, y)
                    .filter(|&t| text_toggle_rect(e.w, e.h, t).contains(Point::new(x, y)))
                {
                    let flag = match t {
                        TextToggle::Hp => &mut self.show_hp,
                        TextToggle::Mp => &mut self.show_mp,
                    };
                    *flag = !*flag;
                    eff.push(CtrlEffect::StoreRegistry {
                        which: t,
                        on: *flag,
                    });
                } else if in_menu_rect(e.w, e.h, x, y) {
                    self.menu_pressed = true;
                    self.press_recorded = true;
                    eff.push(CtrlEffect::Sound(4));
                } else if in_run_rect(e.w, e.h, x, y) {
                    self.run_pressed = true;
                    self.press_recorded = true;
                    eff.push(CtrlEffect::Sound(4));
                }
            }
        }
        // Every case below y = H − 48 sets the low-press flag:= 1 and is consumed.
        self.flag_722358 = true;
        (eff, true)
    }

    /// The cursor-mode steps before a belt click: mode 6 →
    /// Mode 8 → C→S 0x4C (−1).
    fn cursor_steps(mode: u8, eff: &mut Vec<CtrlEffect>) {
        match mode {
            6 => eff.push(CtrlEffect::CursorMode6),
            8 => eff.push(CtrlEffect::Send(msg_u32s(0x4C, &[0xFFFF_FFFF]))),
            _ => {}
        }
    }

    fn state_toggle(ui: u8, open: bool) -> CtrlEffect {
        CtrlEffect::Ui(PanelOutput::SetUi {
            ui,
            // Open → off (1), else toggle (2).
            mode: if open { 1 } else { 2 },
            jump: false,
        })
    }

    /// Up (same guards as down).
    pub fn mouse_up(
        &mut self,
        e: &InputEnv,
        f: &UpFacts,
        x: i32,
        y: i32,
    ) -> (Vec<CtrlEffect>, bool) {
        if e.blocked || !e.alive {
            return (Vec::new(), false);
        }
        self.left_skill = false;
        self.right_skill = false;
        let side = skill_press_hit(e.w, e.h, x, y);
        let (eff, consumed) = if !self.press_recorded {
            // No press recorded: over a skill button with state 3 open →
            // `SetUIState(3, off, 0)`; not consumed.
            let mut eff = Vec::new();
            if side.is_some() && f.state3_open {
                eff.push(CtrlEffect::Ui(PanelOutput::SetUi {
                    ui: UI_SKILL_SELECT,
                    mode: 1,
                    jump: false,
                }));
            }
            (eff, false)
        } else if f.belt_popped {
            let mut eff = Vec::new();
            let mut consumed = false;
            if e.over_belt {
                // Over the belt: cursor mode 6 / 8, then the belt click;
                // consumed.
                Self::cursor_steps(f.cursor_mode, &mut eff);
                eff.push(CtrlEffect::BeltClick);
                consumed = true;
            } else if in_menu_rect(e.w, e.h, x, y) {
                // Menu button → state 0x15 off if open, else toggle (not
                // consumed).
                eff.push(Self::state_toggle(UI_MINI, f.state15_open));
            } else if let Some(s) = side {
                eff.push(CtrlEffect::Ui(PanelOutput::SetUi {
                    ui: UI_SKILL_SELECT,
                    mode: 2,
                    jump: false,
                }));
                eff.push(CtrlEffect::SkillSelect(s == SkillSide::Left));
            } else if in_run_rect(e.w, e.h, x, y) {
                eff.push(CtrlEffect::ToggleRun);
            }
            (eff, consumed)
        } else if y <= e.h - 48 {
            // Belt not popped: y ≤ H − 48 → not consumed.
            (Vec::new(), false)
        } else {
            let mut eff = Vec::new();
            if e.over_belt {
                Self::cursor_steps(f.cursor_mode, &mut eff);
                eff.push(CtrlEffect::BeltClick);
            } else if let Some(s) = side {
                Self::cursor_steps(f.cursor_mode, &mut eff);
                eff.push(Self::state_toggle(UI_SKILL_SELECT, f.state3_open));
                eff.push(CtrlEffect::SkillSelect(s == SkillSide::Left));
            } else if in_run_rect(e.w, e.h, x, y) {
                eff.push(CtrlEffect::ToggleRun);
            } else if in_menu_rect(e.w, e.h, x, y) {
                // State 0x15 off if open, else toggle; if it was pressed,
                // its pressed flag flips (no reader: omitted).
                eff.push(Self::state_toggle(UI_MINI, f.state15_open));
            }
            // Consumed; the low-press flag:= 0.
            self.flag_722358 = false;
            (eff, true)
        };
        // Always: press recorded:= menu pressed:= run pressed:= 0.
        self.press_recorded = false;
        self.menu_pressed = false;
        self.run_pressed = false;
        (eff, consumed)
    }
}

/// "an NPC menu is up": an NPC interaction is
/// active, its NPC exists and the NPC menu state = 1.
pub fn npc_menu_up(active: bool, npc_exists: bool, menu_state: u32) -> bool {
    active && npc_exists && menu_state == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(over_belt: bool) -> InputEnv {
        InputEnv {
            w: 800,
            h: 600,
            blocked: false,
            alive: true,
            over_belt,
        }
    }

    fn facts() -> UpFacts {
        UpFacts {
            belt_popped: false,
            cursor_mode: 0,
            state3_open: false,
            state15_open: false,
        }
    }

    fn set(ui: u8, mode: u8) -> CtrlEffect {
        CtrlEffect::Ui(PanelOutput::SetUi {
            ui,
            mode,
            jump: false,
        })
    }

    #[test]
    fn mouse_down_cases() {
        // Ignored while input is blocked or P is dead / absent.
        let mut c = CtrlInput::default();
        let mut e = env(false);
        e.blocked = true;
        assert_eq!(c.mouse_down(&e, 400, 580), (vec![], false));
        e.blocked = false;
        e.alive = false;
        assert_eq!(c.mouse_down(&e, 400, 580), (vec![], false));
        assert_eq!(c, CtrlInput::default());
        // Over the belt: press recorded:= 1, consumed.
        let (eff, cons) = c.mouse_down(&env(true), 400, 580);
        assert!(eff.is_empty() && cons && c.press_recorded);
        // y ≤ H − 48: not consumed, nothing set.
        let mut c = CtrlInput::default();
        assert_eq!(c.mouse_down(&env(false), 400, 552), (vec![], false));
        assert_eq!(c, CtrlInput::default());
        // Left skill (x 117…165) / right skill (W − 165…W − 117).
        let (eff, cons) = c.mouse_down(&env(false), 130, 580);
        assert!(eff.is_empty() && cons);
        assert!(c.press_recorded && c.left_skill && !c.right_skill && c.flag_722358);
        let mut c = CtrlInput::default();
        c.mouse_down(&env(false), 650, 580);
        assert!(c.press_recorded && c.right_skill && !c.left_skill);
        // The text toggles: stored at once, no press recorded, consumed.
        let mut c = CtrlInput::default();
        let (eff, cons) = c.mouse_down(&env(false), 50, 560);
        assert_eq!(
            eff,
            vec![CtrlEffect::StoreRegistry {
                which: TextToggle::Hp,
                on: true
            }]
        );
        assert!(cons && c.show_hp && !c.press_recorded && c.flag_722358);
        let (eff, _) = c.mouse_down(&env(false), 50, 560);
        assert_eq!(
            eff,
            vec![CtrlEffect::StoreRegistry {
                which: TextToggle::Hp,
                on: false
            }]
        );
        let (eff, _) = c.mouse_down(&env(false), 700, 560);
        assert_eq!(
            eff,
            vec![CtrlEffect::StoreRegistry {
                which: TextToggle::Mp,
                on: true
            }]
        );
        // Menu button (x 392…405, y 561…587) and run button (x 255…272, y
        // 572…592): the flag, press recorded:= 1 and UI sound 4.
        let mut c = CtrlInput::default();
        let (eff, cons) = c.mouse_down(&env(false), 398, 570);
        assert_eq!(eff, vec![CtrlEffect::Sound(4)]);
        assert!(cons && c.menu_pressed && c.press_recorded && !c.run_pressed);
        let mut c = CtrlInput::default();
        let (eff, _) = c.mouse_down(&env(false), 260, 580);
        assert_eq!(eff, vec![CtrlEffect::Sound(4)]);
        assert!(c.run_pressed && c.press_recorded && !c.menu_pressed);
        // Anywhere else below y = H − 48: the low-press flag:= 1, consumed.
        let mut c = CtrlInput::default();
        let (eff, cons) = c.mouse_down(&env(false), 500, 580);
        assert!(eff.is_empty() && cons && c.flag_722358 && !c.press_recorded);
    }

    #[test]
    fn mouse_up_cases() {
        let s6 = |mode| UpFacts {
            cursor_mode: mode,
            ..facts()
        };
        // Guards as down.
        let mut c = CtrlInput {
            press_recorded: true,
            ..Default::default()
        };
        let mut e = env(false);
        e.alive = false;
        assert_eq!(c.mouse_up(&e, &facts(), 400, 580), (vec![], false));
        assert!(c.press_recorded);
        // No press recorded: over a skill button with state 3 open →
        // SetUIState(3, off, 0), not consumed; elsewhere nothing.
        let mut c = CtrlInput::default();
        let mut f = facts();
        f.state3_open = true;
        let (eff, cons) = c.mouse_up(&env(false), &f, 130, 580);
        assert_eq!(eff, vec![set(3, 1)]);
        assert!(!cons);
        assert!(c.mouse_up(&env(false), &f, 500, 580).0.is_empty());
        f.state3_open = false;
        assert!(c.mouse_up(&env(false), &f, 130, 580).0.is_empty());
        // Always: the skill flags, press recorded, menu, run:= 0.
        let mut c = CtrlInput {
            press_recorded: true,
            left_skill: true,
            right_skill: true,
            menu_pressed: true,
            run_pressed: true,
            ..Default::default()
        };
        c.mouse_up(&env(false), &facts(), 500, 580);
        assert_eq!(c, CtrlInput::default());
        // Belt not popped: y ≤ H − 48 → not consumed.
        let mut c = CtrlInput {
            press_recorded: true,
            ..Default::default()
        };
        assert_eq!(c.mouse_up(&env(false), &facts(), 400, 500), (vec![], false));
        assert!(!c.press_recorded);
        // The belt strip: cursor mode 6 → the drop, mode 8 → C→S 0x4C (−1),
        // then the belt click; consumed; the low-press flag:= 0.
        let press = || CtrlInput {
            press_recorded: true,
            flag_722358: true,
            ..Default::default()
        };
        let mut c = press();
        let (eff, cons) = c.mouse_up(&env(true), &s6(0), 400, 580);
        assert_eq!(eff, vec![CtrlEffect::BeltClick]);
        assert!(cons && !c.flag_722358);
        let (eff, _) = press().mouse_up(&env(true), &s6(6), 400, 580);
        assert_eq!(eff, vec![CtrlEffect::CursorMode6, CtrlEffect::BeltClick]);
        let (eff, _) = press().mouse_up(&env(true), &s6(8), 400, 580);
        assert_eq!(
            eff,
            vec![
                CtrlEffect::Send(ClientIntent(vec![0x4C, 0xFF, 0xFF, 0xFF, 0xFF])),
                CtrlEffect::BeltClick
            ]
        );
        // Skill buttons: modes 6 / 8, then state 3 off if open else on
        // (toggle), the skill button's pick.
        let (eff, cons) = press().mouse_up(&env(false), &s6(6), 130, 580);
        assert_eq!(
            eff,
            vec![
                CtrlEffect::CursorMode6,
                set(3, 2),
                CtrlEffect::SkillSelect(true)
            ]
        );
        assert!(cons);
        let mut open = facts();
        open.state3_open = true;
        let (eff, _) = press().mouse_up(&env(false), &open, 650, 580);
        assert_eq!(eff, vec![set(3, 1), CtrlEffect::SkillSelect(false)]);
        // Run button → run:= !run. Menu button → state 0x15 off if open, else
        // toggle; consumed.
        let (eff, cons) = press().mouse_up(&env(false), &facts(), 260, 580);
        assert_eq!(eff, vec![CtrlEffect::ToggleRun]);
        assert!(cons);
        let (eff, cons) = press().mouse_up(&env(false), &facts(), 398, 570);
        assert_eq!(eff, vec![set(0x15, 2)]);
        assert!(cons);
        let mut open = facts();
        open.state15_open = true;
        let (eff, _) = press().mouse_up(&env(false), &open, 398, 570);
        assert_eq!(eff, vec![set(0x15, 1)]);
        // Elsewhere below y = H − 48: consumed, nothing.
        let (eff, cons) = press().mouse_up(&env(false), &facts(), 500, 580);
        assert!(eff.is_empty() && cons);
        // Popped belt: over the belt → mode 6 / 8 steps, belt click,
        // consumed.
        let popped = |m| UpFacts {
            belt_popped: true,
            cursor_mode: m,
            ..facts()
        };
        let (eff, cons) = press().mouse_up(&env(true), &popped(8), 400, 400);
        assert_eq!(
            eff,
            vec![
                CtrlEffect::Send(ClientIntent(vec![0x4C, 0xFF, 0xFF, 0xFF, 0xFF])),
                CtrlEffect::BeltClick
            ]
        );
        assert!(cons);
        // Popped: the menu button is not consumed; skill buttons toggle
        // state 3; run toggles; nothing else (not consumed).
        let (eff, cons) = press().mouse_up(&env(false), &popped(0), 398, 570);
        assert_eq!(eff, vec![set(0x15, 2)]);
        assert!(!cons);
        let (eff, cons) = press().mouse_up(&env(false), &popped(0), 130, 580);
        assert_eq!(eff, vec![set(3, 2), CtrlEffect::SkillSelect(true)]);
        assert!(!cons);
        let (eff, _) = press().mouse_up(&env(false), &popped(0), 260, 580);
        assert_eq!(eff, vec![CtrlEffect::ToggleRun]);
        let (eff, cons) = press().mouse_up(&env(false), &popped(0), 500, 400);
        assert!(eff.is_empty() && !cons);
        // Release tests do not check that the press was on the same button
        // (reproduced): a press on run released over the menu button
        // toggles the mini panel.
        let mut c = CtrlInput {
            press_recorded: true,
            run_pressed: true,
            ..Default::default()
        };
        let (eff, _) = c.mouse_up(&env(false), &facts(), 398, 570);
        assert_eq!(eff, vec![set(0x15, 2)]);
    }

    #[test]
    fn npc_menu_up_rule() {
        assert!(npc_menu_up(true, true, 1));
        assert!(!npc_menu_up(false, true, 1));
        assert!(!npc_menu_up(true, false, 1));
        assert!(!npc_menu_up(true, true, 0));
        assert!(!npc_menu_up(true, true, 5));
    }
}
