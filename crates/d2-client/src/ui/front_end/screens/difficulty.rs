//! The difficulty box and the rules that decide when it opens, which
//! difficulties it enables, the 0x67 flags of the game start and the start act.
//! The rules are pure functions of the save's u16 status word at +0x24;
//! character select calls [`difficulties_open`] and stores the result in
//! `FlowCtx::difficulties_open` before it fires `Ok`.

use crate::ui::front_end::control::vk;
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screens::ids::DIFFICULTY;
use crate::ui::front_end::{Action, Control, ControlKind, FrontCtx, Registry, Screen};

/// Status word bits (L1).
pub const STATUS_HARDCORE: u16 = 0x0004;
pub const STATUS_DEAD: u16 = 0x0008;
pub const STATUS_EXPANSION: u16 = 0x0020;

/// u32@0x27 of the C→S 0x67 (L2 r5).
pub const FLAG_BASE: u32 = 4;
pub const FLAG_HARDCORE: u32 = 0x800;
pub const FLAG_EXPANSION: u32 = 0x10_0000;

/// Progression p: status bits 8–12 (L1).
pub fn progression(status: u16) -> u16 {
    (status >> 8) & 0x1F
}

/// What OK does with a character (L1, L2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OkAction {
    /// Nothing: expansion character on a classic install (L1).
    Ignored,
    /// Message 5304 "You cannot create or join games with a dead hardcore
    /// character."; stays on character select.
    DeadHardcore,
    /// Start at once at Normal (L2 r1).
    Start,
    /// Open the difficulty box.
    Box,
}

/// / L2 r1 for `status` on an install that is
/// `expansion_install`.
pub fn ok_decision(status: u16, expansion_install: bool) -> OkAction {
    let e = status & STATUS_EXPANSION != 0;
    if e && !expansion_install {
        return OkAction::Ignored;
    }
    if status & (STATUS_HARDCORE | STATUS_DEAD) == (STATUS_HARDCORE | STATUS_DEAD) {
        return OkAction::DeadHardcore;
    }
    if d2_formats::d2s::status::difficulties(status) & d2_formats::d2s::status::NIGHTMARE_BIT != 0 {
        OkAction::Box
    } else {
        OkAction::Start
    }
}

/// Hell button enabled (L2 r2): (E = 0 and p ≥ 8) or p ≥ 10, the shared
/// rule [`d2_formats::d2s::status::difficulties`].
pub fn hell_enabled(status: u16) -> bool {
    d2_formats::d2s::status::difficulties(status) & d2_formats::d2s::status::HELL_BIT != 0
}

/// The value character select stores in `FlowCtx::difficulties_open`: 1 when
/// the game starts at once, 2 with Hell off, 3 with Hell on. The flow table
/// opens the box when it is above 1.
pub fn difficulties_open(status: u16, expansion_install: bool) -> u8 {
    match ok_decision(status, expansion_install) {
        OkAction::Box if hell_enabled(status) => 3,
        OkAction::Box => 2,
        _ => 1,
    }
}

/// The C→S 0x67 u32@0x27 (L2 r5).
pub fn start_flags(status: u16) -> u32 {
    let mut f = FLAG_BASE;
    if status & STATUS_HARDCORE != 0 {
        f |= FLAG_HARDCORE;
    }
    if status & STATUS_EXPANSION != 0 {
        f |= FLAG_EXPANSION;
    }
    f
}

/// Start act from the save byte `+0xA8 + difficulty` (L2 r4): `byte & 0x7F`, 0
/// when ≥ 5.
pub fn start_act(town_bytes: [u8; 3], difficulty: u8) -> u8 {
    let a = town_bytes
        .get(usize::from(difficulty))
        .map_or(0, |b| b & 0x7F);
    if a >= 5 {
        0
    } else {
        a
    }
}

const HK_R: u16 = b'R' as u16;
const HK_N: u16 = b'N' as u16;
const HK_H: u16 = b'H' as u16;

/// The difficulty box.
#[derive(Debug, Default)]
pub struct DifficultyScreen;

impl Screen for DifficultyScreen {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        let mut v = vec![Control::new(ControlKind::Image, 237, 399, 326, 200)
            .with_art(r"CharSelect\DifficultyLevels")];
        v.push(
            Control::new(ControlKind::Text, 264, 260, 272, 35)
                .with_string(10019)
                .with_font(7, 2)
                .with_color(7),
        );
        let btn = |y, s, k, d| {
            Control::new(ControlKind::Button, 264, y, 272, 35)
                .with_art(r"FrontEnd\WideButtonBlank")
                .with_string(s)
                .with_hotkey(k)
                .with_action(Action::Trigger(Trigger::Difficulty(d)))
        };
        v.push(btn(297, 10018, HK_R, 0));
        v.push(btn(340, 10017, HK_N, 1));
        let mut hell = btn(383, 10016, HK_H, 2);
        hell.enabled = ctx.flow.difficulties_open >= 3;
        v.push(hell);
        // Off-screen Esc control (0xAD): back to the list.
        let mut esc = Control::new(ControlKind::Button, 900, 900, 10, 10)
            .with_hotkey(vk::ESC)
            .with_action(Action::Trigger(Trigger::Exit));
        esc.visible = false;
        v.push(esc);
        v
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(DIFFICULTY, Box::new(DifficultyScreen));
}
