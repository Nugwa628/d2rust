//! Translation between the numeric command and key codes of the `controls::original` scheme and
//! the portable [`Action`] / [`Key`] names used by the controls file.
//!
//! Commands are numbered 0-56; key codes are Windows virtual keys, with 0x100-0x104 for the
//! middle/extra mouse buttons and wheel directions. This module also decides which commands run
//! in each key mode.

use super::{Action as Act, Key};

/// Maps a command number (0-56) of the `controls::original` scheme to its action. Every
/// command has one; numbers outside the range give `None`.
pub fn action_of_cmd(cmd: i32) -> Option<Act> {
    use Act::*;
    const SKILLS: [Act; 16] = [
        SkillSlot1,
        SkillSlot2,
        SkillSlot3,
        SkillSlot4,
        SkillSlot5,
        SkillSlot6,
        SkillSlot7,
        SkillSlot8,
        SkillSlot9,
        SkillSlot10,
        SkillSlot11,
        SkillSlot12,
        SkillSlot13,
        SkillSlot14,
        SkillSlot15,
        SkillSlot16,
    ];
    Some(match cmd {
        0 => ToggleCharacter,
        1 => ToggleInventory,
        2 => ToggleParty,
        4 => ToggleQuests,
        5 => OpenChat,
        6 => ToggleHelp,
        7 => ToggleAutomap,
        8 => CenterAutomap,
        9 => ToggleAutomapFade,
        12 => ToggleSkillTree,
        13 => ToggleSkillMenuRight,
        14..=21 => SKILLS[(cmd - 14) as usize],
        22 => ToggleBelt,
        23 => BeltSlot1,
        24 => BeltSlot2,
        25 => BeltSlot3,
        26 => BeltSlot4,
        35 => ToggleRun,
        36 => StandStill,
        37 => ShowItems,
        38 => ClearScreen,
        42 => Screenshot,
        43 => ShowPortraits,
        44 => SwapWeapons,
        46..=53 => SKILLS[(cmd - 46 + 8) as usize],
        3 => ToggleMessageLog,
        10 => ToggleAutomapParty,
        11 => ToggleAutomapNames,
        27 => Say0,
        28 => Say1,
        29 => Say2,
        30 => Say3,
        31 => Say4,
        32 => Say5,
        33 => Say6,
        34 => Run,
        39 => SkillUp,
        40 => SkillDown,
        41 => ClearTextMessages,
        45 => ToggleMinimap,
        54 => ToggleHireling,
        55 => Say7X,
        56 => GameMenu,
        _ => return None,
    })
}

/// Windows virtual key (or 0x100…0x104 mouse) → portable [`Key`].
pub fn vk_to_key(vk: u16) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Digit0,
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
        Key::Digit7,
        Key::Digit8,
        Key::Digit9,
    ];
    const F: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    const PAD: [Key; 10] = [
        Key::Numpad0,
        Key::Numpad1,
        Key::Numpad2,
        Key::Numpad3,
        Key::Numpad4,
        Key::Numpad5,
        Key::Numpad6,
        Key::Numpad7,
        Key::Numpad8,
        Key::Numpad9,
    ];
    Some(match vk {
        0x41..=0x5A => LETTERS[usize::from(vk - 0x41)],
        0x30..=0x39 => DIGITS[usize::from(vk - 0x30)],
        0x70..=0x7B => F[usize::from(vk - 0x70)],
        0x60..=0x69 => PAD[usize::from(vk - 0x60)],
        0x10 => Key::LeftShift,
        0x11 => Key::LeftCtrl,
        0x12 => Key::LeftAlt,
        0x20 => Key::Space,
        0x0D => Key::Enter,
        // Command 56 (game menu) is fixed to Esc and cannot be reassigned.
        0x1B => Key::Escape,
        0x09 => Key::Tab,
        0x08 => Key::Backspace,
        0x13 => Key::Pause,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::Left,
        0x26 => Key::Up,
        0x27 => Key::Right,
        0x28 => Key::Down,
        0x2C => Key::PrintScreen,
        0x2D => Key::Insert,
        0x2E => Key::Delete,
        0x6A => Key::NumpadMultiply,
        0x6B => Key::NumpadAdd,
        0x6D => Key::NumpadSubtract,
        0x6F => Key::NumpadDivide,
        0xBA => Key::Semicolon,
        0xBB => Key::Equals,
        0xBC => Key::Comma,
        0xBD => Key::Minus,
        0xBE => Key::Period,
        0xBF => Key::Slash,
        0xC0 => Key::Grave,
        0xDB => Key::LeftBracket,
        0xDC => Key::Backslash,
        0xDD => Key::RightBracket,
        0xDE => Key::Quote,
        0x100 => Key::MouseMiddle,
        0x101 => Key::Mouse4,
        0x102 => Key::Mouse5,
        0x103 => Key::MouseWheelUp,
        0x104 => Key::MouseWheelDown,
        _ => return None,
    })
}

/// The inverse of [`vk_to_key`]; keys the table cannot hold give `None`.
pub fn key_to_vk(k: Key) -> Option<u16> {
    (0u16..=0x104).find(|&vk| vk_to_key(vk) == Some(k))
}

/// Whether the key command of `action` runs in key mode `mode`
/// Mode 1 runs every command, mode 0 runs none, and mode 2 runs only the commands whose
/// `full_ok` flag is set. Actions that are not commands, and the close commands (Esc and
/// Space/ClearScreen), always run, because the panel and chat closes go through them. So Esc
/// and Space still close the stash in key mode 2 and the key config in mode 0, while `i` and
/// `m` (no flag) do nothing in modes 0 and 2, and letters typed in the chat reach it as text.
pub fn runs_in_key_mode(action: Act, mode: u8) -> bool {
    // The close commands (Esc and Space/ClearScreen) always run: they close
    // panels in the modes that gate other commands.
    if mode == 1 || matches!(action, Act::GameMenu | Act::ClearScreen) {
        return true;
    }
    match (0..super::original::COMMAND_COUNT as i32).find(|&c| action_of_cmd(c) == Some(action)) {
        None => true,
        Some(c) => mode == 2 && super::original::COMMANDS[c as usize].full_ok,
    }
}

#[cfg(test)]
mod mode_tests {
    //! Tests for the key-mode gate of commands.

    use super::*;

    #[test]
    fn key_modes_gate_commands_by_the_m2_flag() {
        // Cmd 1 (inventory) has no `full_ok` flag; cmd 45 (minimap) and 5 (chat) do.
        assert!(runs_in_key_mode(Act::ToggleInventory, 1));
        assert!(!runs_in_key_mode(Act::ToggleInventory, 0));
        assert!(!runs_in_key_mode(Act::ToggleInventory, 2));
        assert!(runs_in_key_mode(Act::ToggleMinimap, 2));
        assert!(!runs_in_key_mode(Act::ToggleMinimap, 0));
        assert!(runs_in_key_mode(Act::GameMenu, 0));
        assert!(runs_in_key_mode(Act::ChatSend, 0));
        // The close commands run in every mode: Space closes the shop in
        // key mode 2 and panels in the key config's mode 0.
        assert!(runs_in_key_mode(Act::ClearScreen, 0));
        assert!(runs_in_key_mode(Act::ClearScreen, 2));
    }
}
