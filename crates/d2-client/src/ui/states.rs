//! The 38 UI states and the open/close call, the conflict gate,
//! slots, the screen open mode and the cursor jump.
//!
//! [`UiStates::set`] is `SetUIState`. It changes only the
//! client's presentation state; what the game does at the same time
//! outside the flags (hooks, the open mode's camera shift, the cursor move,
//! the respawn request) is returned as [`UiEffect`]s, in call order, for
//! the caller to apply. Intents still leave only through the UI root and
//! the bridge.

use super::geom::Point;
use super::layout::{
    ui_states, Conflict, LayoutError, Screen, SlotKind, UiStateRow, UI_STATE_COUNT,
};
use crate::rules::camera::OpenMode;

/// Ids of the states the rules below name (D2MOO `D2C_UIvars`).
pub mod id {
    pub const GAME: u8 = 0;
    pub const INVENTORY: u8 = 1;
    pub const CHARACTER: u8 = 2;
    pub const SKILL_TREE: u8 = 4;
    pub const CHAT: u8 = 5;
    pub const NEW_STATS: u8 = 6;
    pub const NEW_SKILLS: u8 = 7;
    pub const NPC_MENU: u8 = 8;
    pub const ESC_MENU: u8 = 9;
    pub const AUTOMAP: u8 = 0x0A;
    pub const NPC_SHOP: u8 = 0x0C;
    pub const QUEST_SCREEN: u8 = 0x0F;
    pub const INI_SCROLL: u8 = 0x10;
    pub const QUEST_LOG: u8 = 0x11;
    pub const HIR_ICONS: u8 = 0x13;
    pub const WAYPOINT: u8 = 0x14;
    pub const PARTY: u8 = 0x16;
    pub const STASH: u8 = 0x19;
    pub const CUBE: u8 = 0x1A;
    pub const MERC_INV: u8 = 0x24;
    pub const RECIPE_SCROLL: u8 = 0x25;
}

/// The states whose flags make "left open".
pub const LEFT_STATES: [u8; 7] = [2, 0x14, 0x0F, 0x10, 0x24, 0x25, 0x16];
/// The states whose flags make "right open".
pub const RIGHT_STATES: [u8; 2] = [1, 4];
/// The states that may open under a modal text screen.
pub const MODAL_ALLOWED: [u8; 5] = [0x0A, 0x13, 0x11, 6, 7];

/// `SetUIState` modes (D2MOO `UI_TURNON` / `UI_TURNOFF` / `UI_TOGGLE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetMode {
    On = 0,
    Off = 1,
    Toggle = 2,
}

impl SetMode {
    /// `None` for a mode above 2 (a fatal error in the game).
    pub fn from_u32(m: u32) -> Option<Self> {
        Some(match m {
            0 => SetMode::On,
            1 => SetMode::Off,
            2 => SetMode::Toggle,
            _ => return None,
        })
    }
}

/// The local player as the call reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerLife {
    /// "P alive": a player unit whose mode is neither 0
    /// nor 0x11 and whose unit flag 0x10000 is clear.
    pub alive: bool,
    /// Mode 0x11 (dead): the escape menu request respawns instead.
    pub dead: bool,
}

/// What the call reads besides the flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateEnv {
    /// Expansion game with the expansion installed.
    pub expansion: bool,
    /// `None`: no local player.
    pub player: Option<PlayerLife>,
    /// Set and the chat flag is 0 (chat refused).
    pub chat_blocked: bool,
    /// `GetTickCount` is before the input hold's end (wall-clock; read by the caller).
    pub input_hold: bool,
    /// A modal text screen is up.
    pub modal_text: bool,
    /// An NPC interaction is active.
    pub npc_active: bool,
    pub screen: Screen,
    /// The mouse before the call.
    pub mouse: Point,
}

/// A side effect of the call, in the order the game performs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEffect {
    /// Closed → open: the open hook (and the ui 5 extra hook).
    Opened(u8),
    /// Open → closed: the close hook (ui 0x0B also its extra hook).
    Closed(u8),
    /// Ui 1's, run before the slot logic on every call that
    /// passes the gate.
    InventoryHook,
    /// `SetScreenOpenMode(m)`: `shiftX` and the view rectangle per.
    OpenMode(OpenMode),
    /// The cursor moves to x.
    CursorX(i32),
    /// The escape menu was asked for while dead: the respawn path (C→S 0x41).
    Respawn,
    /// Gate action 4 ended the active NPC interaction.
    EndNpcInteraction,
}

/// The game's fatal errors (ui 0 among them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UiStateError {
    #[error("ui state {0} out of range (> 0x25)")]
    BadState(u32),
    #[error("SetUIState mode {0} out of range (> 2)")]
    BadMode(u32),
    #[error("conflict action 3 for ui 0 (state {0} open)")]
    Fatal3(u8),
}

/// The 38 UI flags and the screen open mode.
#[derive(Clone, Debug)]
pub struct UiStates {
    rows: Vec<UiStateRow>,
    flags: [bool; UI_STATE_COUNT],
    open_mode: OpenMode,
}

impl UiStates {
    /// All flags 0, open mode 0.
    pub fn new() -> Result<Self, LayoutError> {
        Ok(Self::from_rows(ui_states()?))
    }

    pub fn from_rows(rows: Vec<UiStateRow>) -> Self {
        Self {
            rows,
            flags: [false; UI_STATE_COUNT],
            open_mode: OpenMode::NONE,
        }
    }

    pub fn rows(&self) -> &[UiStateRow] {
        &self.rows
    }

    pub fn is_open(&self, ui: u8) -> bool {
        self.flags.get(usize::from(ui)).copied().unwrap_or(false)
    }

    pub fn flags(&self) -> &[bool; UI_STATE_COUNT] {
        &self.flags
    }

    pub fn open_mode(&self) -> OpenMode {
        self.open_mode
    }

    /// Sets a flag without the call (start-up resets; tests).
    pub fn force(&mut self, ui: u8, open: bool) {
        if let Some(f) = self.flags.get_mut(usize::from(ui)) {
            *f = open;
        }
    }

    pub fn left_open(&self) -> bool {
        LEFT_STATES.iter().any(|&i| self.is_open(i))
    }

    pub fn right_open(&self) -> bool {
        RIGHT_STATES.iter().any(|&i| self.is_open(i))
    }

    /// `SetUIState(ui, mode, jump)`. Returns the call's result
    /// (true = 1) and appends its effects to `fx`.
    pub fn set(
        &mut self,
        ui: u32,
        mode: u32,
        jump: bool,
        env: &mut GateEnv,
        fx: &mut Vec<UiEffect>,
    ) -> Result<bool, UiStateError> {
        if ui > 0x25 {
            return Err(UiStateError::BadState(ui));
        }
        let mode = SetMode::from_u32(mode).ok_or(UiStateError::BadMode(mode))?;
        let ui = ui as u8;
        if self.rows[usize::from(ui)].exp_only && !env.expansion {
            return Ok(false);
        }
        let was = self.is_open(ui);
        let gated = match mode {
            SetMode::On => true,
            SetMode::Toggle => !was,
            SetMode::Off => false,
        };
        if gated && !self.gate(ui, env, fx)? {
            return Ok(false);
        }
        // (the gate may have changed flags; read again)
        let cur = self.is_open(ui);
        let may = env.player.is_none_or(|p| p.alive);
        let new = match mode {
            SetMode::On => cur || may,
            SetMode::Off => false,
            SetMode::Toggle if may || ui == id::CHAT => !cur,
            SetMode::Toggle => cur,
        };
        self.flags[usize::from(ui)] = new;
        match (cur, new) {
            (false, true) => fx.push(UiEffect::Opened(ui)),
            (true, false) => fx.push(UiEffect::Closed(ui)),
            _ => {}
        }
        if ui == id::INVENTORY {
            fx.push(UiEffect::InventoryHook);
        }
        self.slot(ui, new, jump, env, fx);
        Ok(true)
    }

    /// The gate. True = pass.
    fn gate(
        &mut self,
        ui: u8,
        env: &mut GateEnv,
        fx: &mut Vec<UiEffect>,
    ) -> Result<bool, UiStateError> {
        if ui == id::CHAT && env.chat_blocked {
            return Ok(false);
        }
        if (ui == id::CHAT || ui == id::ESC_MENU) && env.input_hold {
            return Ok(false);
        }
        if ui == id::ESC_MENU {
            match env.player {
                None => return Ok(false),
                Some(p) if p.dead => {
                    fx.push(UiEffect::Respawn);
                    return Ok(false);
                }
                Some(_) => {}
            }
        }
        if env.modal_text && !MODAL_ALLOWED.contains(&ui) {
            return Ok(false);
        }
        // Every open state in id order; flags read as the loop goes.
        for i in 0..UI_STATE_COUNT as u8 {
            if !self.is_open(i) {
                continue;
            }
            match self.rows[usize::from(i)].conflicts[usize::from(ui)] {
                Conflict::Ignore => {}
                Conflict::Close => {
                    self.set(u32::from(i), SetMode::Off as u32, false, env, fx)?;
                }
                Conflict::Refuse => return Ok(false),
                Conflict::RefuseFatal0 if ui == 0 => return Err(UiStateError::Fatal3(i)),
                Conflict::RefuseFatal0 => return Ok(false),
                Conflict::EndNpc => {
                    if env.npc_active {
                        env.npc_active = false;
                        fx.push(UiEffect::EndNpcInteraction);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Slot logic, open mode and cursor jump.
    fn slot(&mut self, ui: u8, v: bool, jump: bool, env: &GateEnv, fx: &mut Vec<UiEffect>) {
        let kind = self.rows[usize::from(ui)].slot;
        let left = self.left_open();
        let right = self.right_open();
        let m = match (kind, v) {
            (SlotKind::Right, true) => {
                if left {
                    3
                } else {
                    1
                }
            }
            (SlotKind::Right, false) => {
                if left {
                    2
                } else {
                    0
                }
            }
            (SlotKind::Left, true) => {
                if right {
                    3
                } else {
                    2
                }
            }
            (SlotKind::Left, false) => {
                if right {
                    1
                } else {
                    0
                }
            }
            (SlotKind::Full, true) => 3,
            (SlotKind::Full, false) => 0,
            (SlotKind::Anvil, true) => 1,
            (SlotKind::Anvil, false) => 0,
            (SlotKind::None, _) => return,
        };
        // Mouse read before the mode change; left/right kinds only,
        // and only when the other side is closed.
        // The cursor follows the view's move (`Screen::panel_shift`, W / 4
        // at 4:3) when it is on the world side of the panel.
        let s = &env.screen;
        let (x, d) = (env.mouse.x, s.panel_shift());
        let jump_to = match (kind, v) {
            _ if !jump => None,
            (SlotKind::Right, true) if !left && x > s.frame_left() + d => Some(x - d),
            (SlotKind::Right, false) if !left && x < s.right_start() => Some(x + d),
            (SlotKind::Left, true) if !right && x < s.frame_right() - d => Some(x + d),
            (SlotKind::Left, false) if !right && x > s.left_end() => Some(x - d),
            _ => None,
        };
        self.open_mode = OpenMode::new(m).unwrap_or(OpenMode::NONE);
        fx.push(UiEffect::OpenMode(self.open_mode));
        if let Some(nx) = jump_to {
            fx.push(UiEffect::CursorX(nx));
        }
    }
}
