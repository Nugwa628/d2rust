//! The Bevy input edge of the UI: the only part of `ui` that touches
//! Bevy. It reads the cursor from a `Window` in physical pixels and maps it
//! into the frame with [`Presentation`], the same placement the present
//! stage draws with; everything after is plain Rust. Key and button
//! bindings to actions are the controls', which implement
//! [`super::UiInput`] on top of this.

use bevy::input::keyboard::KeyCode;
use bevy::math::Vec2;
use bevy::window::Window;

use super::frame::{FrameError, FramePos, Presentation};
use super::panel::{ActionId, UiEvent};
use crate::controls::original::{WheelAccumulator, WHEEL_UP};
use crate::controls::{Action, Bindings, Context, Key};

/// The presentation of the play frame
/// ([`crate::rules::camera::FrameSize::play`]) for a window's current
/// physical size, in the scale mode in force.
pub fn presentation(window: &Window) -> Result<Presentation, FrameError> {
    let frame = crate::rules::camera::FrameSize::play();
    present_in(
        window.physical_width(),
        window.physical_height(),
        frame.width as u32,
        frame.height as u32,
        super::frame::scale_mode(),
    )
}

/// A `frame_w × frame_h` frame in a `window_w × window_h` window in scale
/// mode `mode`. Integer scaling in a window smaller than the frame has no
/// whole-number scale, so it shows the frame as fit does.
pub fn present_in(
    window_w: u32,
    window_h: u32,
    frame_w: u32,
    frame_h: u32,
    mode: super::frame::ScaleMode,
) -> Result<Presentation, FrameError> {
    Presentation::with_mode(window_w, window_h, frame_w, frame_h, mode).or_else(|_| {
        Presentation::with_mode(
            window_w,
            window_h,
            frame_w,
            frame_h,
            super::frame::ScaleMode::Fit,
        )
    })
}

/// The window pixel holding a physical cursor position: Bevy reports it
/// as floats; the pixel is the floor of each coordinate.
pub fn window_pixel(physical: Vec2) -> (i64, i64) {
    (physical.x.floor() as i64, physical.y.floor() as i64)
}

/// The physical window position of a frame pixel, for warping the cursor
/// (the cursor jump of the ui state change). `Err` below the minimum window.
pub fn frame_to_window(window: &Window, at: super::geom::Point) -> Result<Vec2, FrameError> {
    let (x, y) = presentation(window)?.from_frame(at.x, at.y);
    Ok(Vec2::new(x as f32, y as f32))
}

/// Where the window's cursor is in the frame; `Outside` when the cursor
/// is not in the window.
pub fn cursor_frame_pos(window: &Window) -> Result<FramePos, FrameError> {
    let p = presentation(window)?;
    Ok(match window.physical_cursor_position() {
        Some(c) => {
            let (x, y) = window_pixel(c);
            p.to_frame(x, y)
        }
        None => FramePos::Outside,
    })
}

/// Bevy key codes of the controls' keyboard [`Key`]s: one row
/// per key; the mouse inputs are the pointer events. Keys are a D2RUST
/// table (the game's key table is `crate::controls::keymap`).
pub const KEY_CODES: &[(KeyCode, Key)] = {
    use KeyCode as C;
    &[
        (C::KeyA, Key::A),
        (C::KeyB, Key::B),
        (C::KeyC, Key::C),
        (C::KeyD, Key::D),
        (C::KeyE, Key::E),
        (C::KeyF, Key::F),
        (C::KeyG, Key::G),
        (C::KeyH, Key::H),
        (C::KeyI, Key::I),
        (C::KeyJ, Key::J),
        (C::KeyK, Key::K),
        (C::KeyL, Key::L),
        (C::KeyM, Key::M),
        (C::KeyN, Key::N),
        (C::KeyO, Key::O),
        (C::KeyP, Key::P),
        (C::KeyQ, Key::Q),
        (C::KeyR, Key::R),
        (C::KeyS, Key::S),
        (C::KeyT, Key::T),
        (C::KeyU, Key::U),
        (C::KeyV, Key::V),
        (C::KeyW, Key::W),
        (C::KeyX, Key::X),
        (C::KeyY, Key::Y),
        (C::KeyZ, Key::Z),
        (C::Digit0, Key::Digit0),
        (C::Digit1, Key::Digit1),
        (C::Digit2, Key::Digit2),
        (C::Digit3, Key::Digit3),
        (C::Digit4, Key::Digit4),
        (C::Digit5, Key::Digit5),
        (C::Digit6, Key::Digit6),
        (C::Digit7, Key::Digit7),
        (C::Digit8, Key::Digit8),
        (C::Digit9, Key::Digit9),
        (C::F1, Key::F1),
        (C::F2, Key::F2),
        (C::F3, Key::F3),
        (C::F4, Key::F4),
        (C::F5, Key::F5),
        (C::F6, Key::F6),
        (C::F7, Key::F7),
        (C::F8, Key::F8),
        (C::F9, Key::F9),
        (C::F10, Key::F10),
        (C::F11, Key::F11),
        (C::F12, Key::F12),
        (C::ShiftLeft, Key::LeftShift),
        (C::ShiftRight, Key::RightShift),
        (C::ControlLeft, Key::LeftCtrl),
        (C::ControlRight, Key::RightCtrl),
        (C::AltLeft, Key::LeftAlt),
        (C::AltRight, Key::RightAlt),
        (C::Space, Key::Space),
        (C::Enter, Key::Enter),
        (C::Escape, Key::Escape),
        (C::Tab, Key::Tab),
        (C::Backspace, Key::Backspace),
        (C::Insert, Key::Insert),
        (C::Delete, Key::Delete),
        (C::Home, Key::Home),
        (C::End, Key::End),
        (C::PageUp, Key::PageUp),
        (C::PageDown, Key::PageDown),
        (C::ArrowUp, Key::Up),
        (C::ArrowDown, Key::Down),
        (C::ArrowLeft, Key::Left),
        (C::ArrowRight, Key::Right),
        (C::Backquote, Key::Grave),
        (C::Minus, Key::Minus),
        (C::Equal, Key::Equals),
        (C::BracketLeft, Key::LeftBracket),
        (C::BracketRight, Key::RightBracket),
        (C::Backslash, Key::Backslash),
        (C::Semicolon, Key::Semicolon),
        (C::Quote, Key::Quote),
        (C::Comma, Key::Comma),
        (C::Period, Key::Period),
        (C::Slash, Key::Slash),
        (C::PrintScreen, Key::PrintScreen),
        (C::Pause, Key::Pause),
        (C::Numpad0, Key::Numpad0),
        (C::Numpad1, Key::Numpad1),
        (C::Numpad2, Key::Numpad2),
        (C::Numpad3, Key::Numpad3),
        (C::Numpad4, Key::Numpad4),
        (C::Numpad5, Key::Numpad5),
        (C::Numpad6, Key::Numpad6),
        (C::Numpad7, Key::Numpad7),
        (C::Numpad8, Key::Numpad8),
        (C::Numpad9, Key::Numpad9),
        (C::NumpadAdd, Key::NumpadAdd),
        (C::NumpadSubtract, Key::NumpadSubtract),
        (C::NumpadMultiply, Key::NumpadMultiply),
        (C::NumpadDivide, Key::NumpadDivide),
        (C::NumpadEnter, Key::NumpadEnter),
    ]
};

/// The controls key of a Bevy key code; `None` for a key the controls
/// do not name.
pub fn key_of(code: KeyCode) -> Option<Key> {
    KEY_CODES.iter().find(|(c, _)| *c == code).map(|(_, k)| *k)
}

/// The UI events of the keys pressed this frame: each key bound in the world
/// context to an action becomes [`UiEvent::Action`], in `pressed` order
/// (actions come from the bindings, never from raw keys).
pub fn key_actions(bindings: &Bindings, pressed: &[KeyCode]) -> Vec<UiEvent> {
    let keys: Vec<Key> = pressed.iter().filter_map(|&c| key_of(c)).collect();
    input_actions(bindings, &keys)
}

/// [`key_actions`] under key mode `mode`:
/// mode 0 registers no key-down handler, mode 2 runs only the commands
/// with the M2 flag; mode 1 runs all. Actions that are no key command
/// (the fixed buttons, chat and panel inputs) are not filtered, nor is
/// Esc (command 56): the panels' own Esc close runs through it (Esc
/// passes in key modes 2 and 0, the
/// bound `i` / `m` do not).
pub fn key_actions_in_mode(bindings: &Bindings, pressed: &[KeyCode], mode: u8) -> Vec<UiEvent> {
    let mut v = key_actions(bindings, pressed);
    v.retain(|e| match e {
        UiEvent::Action(a) => Action::ALL
            .get(usize::from(a.0))
            .is_none_or(|&act| crate::controls::keymap::runs_in_key_mode(act, mode)),
        _ => true,
    });
    v
}

/// The world actions of portable inputs, in order: keys, the middle / X buttons
/// and the wheel steps (they call the bound command's handler, no panel takes
/// them). VK 0x10–0x12 cover both sides: a right Shift / Ctrl / Alt not bound
/// itself is its left key.
pub fn input_actions(bindings: &Bindings, keys: &[Key]) -> Vec<UiEvent> {
    keys.iter()
        .filter_map(|&k| {
            bindings
                .action_for(Context::World, k)
                .or_else(|| bindings.action_for(Context::World, left_side(k)?))
        })
        .filter_map(|a| u16::try_from(a.index()).ok())
        .map(|i| UiEvent::Action(ActionId(i)))
        .collect()
}

/// The left key of a right modifier (VK 0x10–0x12 name both).
pub fn left_side(k: Key) -> Option<Key> {
    match k {
        Key::RightShift => Some(Key::LeftShift),
        Key::RightCtrl => Some(Key::LeftCtrl),
        Key::RightAlt => Some(Key::LeftAlt),
        _ => None,
    }
}

/// Whether an input bound to `action` is held: `down` answers per key
/// (both sides for the modifiers).
pub fn action_held(bindings: &Bindings, action: Action, down: &dyn Fn(Key) -> bool) -> bool {
    bindings.inputs(action).iter().any(|&k| {
        down(k)
            || match k {
                Key::LeftShift => down(Key::RightShift),
                Key::LeftCtrl => down(Key::RightCtrl),
                Key::LeftAlt => down(Key::RightAlt),
                _ => false,
            }
    })
}

/// A wheel event of `delta` (Windows units, 120 per notch) through the
/// accumulator: the wheel key it fires, if
/// any (one per event).
pub fn wheel_key(acc: &mut WheelAccumulator, delta: i32) -> Option<Key> {
    match acc.event(delta, true)? {
        WHEEL_UP => Some(Key::MouseWheelUp),
        _ => Some(Key::MouseWheelDown),
    }
}

/// The typed characters of the keys pressed this frame, for the text
/// the UI asks for (the gold dialog's digits): digit keys → the digit,
/// Backspace → 8, Enter → 0x0D, Escape → 0x1B, as [`UiEvent::Char`].
/// D2RUST's own, unverified (D1): a raw key map (no layout, no shift); the
/// panels that take no text ignore these events.
pub fn key_chars(pressed: &[KeyCode]) -> Vec<UiEvent> {
    use KeyCode as C;
    const DIGITS: [(KeyCode, u16); 20] = [
        (C::Digit0, b'0' as u16),
        (C::Digit1, b'1' as u16),
        (C::Digit2, b'2' as u16),
        (C::Digit3, b'3' as u16),
        (C::Digit4, b'4' as u16),
        (C::Digit5, b'5' as u16),
        (C::Digit6, b'6' as u16),
        (C::Digit7, b'7' as u16),
        (C::Digit8, b'8' as u16),
        (C::Digit9, b'9' as u16),
        (C::Numpad0, b'0' as u16),
        (C::Numpad1, b'1' as u16),
        (C::Numpad2, b'2' as u16),
        (C::Numpad3, b'3' as u16),
        (C::Numpad4, b'4' as u16),
        (C::Numpad5, b'5' as u16),
        (C::Numpad6, b'6' as u16),
        (C::Numpad7, b'7' as u16),
        (C::Numpad8, b'8' as u16),
        (C::Numpad9, b'9' as u16),
    ];
    pressed
        .iter()
        .filter_map(|c| match c {
            C::Backspace => Some(8),
            C::Enter | C::NumpadEnter => Some(0x0D),
            C::Escape => Some(0x1B),
            // The Esc menu's arrows (private-use units, D2RUST's own).
            C::ArrowLeft => Some(0xF025),
            C::ArrowUp => Some(0xF026),
            C::ArrowRight => Some(0xF027),
            C::ArrowDown => Some(0xF028),
            c => DIGITS.iter().find(|(d, _)| d == c).map(|&(_, u)| u),
        })
        .map(UiEvent::Char)
        .collect()
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::controls::Preset;

    #[test]
    fn every_keyboard_key_has_one_code() {
        for &k in Key::ALL {
            let n = KEY_CODES.iter().filter(|(_, x)| *x == k).count();
            let mouse = k.name().starts_with("Mouse");
            assert_eq!(n, usize::from(!mouse), "{}", k.name());
        }
        for (i, (c, _)) in KEY_CODES.iter().enumerate() {
            assert!(KEY_CODES[..i].iter().all(|(d, _)| d != c));
        }
    }

    #[test]
    fn typed_keys_become_chars() {
        let e = key_chars(&[
            KeyCode::Digit4,
            KeyCode::KeyX,
            KeyCode::Backspace,
            KeyCode::Enter,
        ]);
        assert_eq!(
            e,
            vec![
                UiEvent::Char(b'4' as u16),
                UiEvent::Char(8),
                UiEvent::Char(0x0D)
            ]
        );
    }

    #[test]
    fn bound_keys_become_world_actions_in_order() {
        let b = Preset::Dev.bindings().unwrap();
        let e = key_actions(&b, &[KeyCode::KeyI, KeyCode::KeyX, KeyCode::KeyT]);
        assert_eq!(
            e,
            vec![
                UiEvent::Action(ActionId(Action::ToggleInventory.index() as u16)),
                UiEvent::Action(ActionId(Action::ToggleSkillTree.index() as u16)),
            ]
        );
    }

    fn act(a: Action) -> UiEvent {
        UiEvent::Action(ActionId(a.index() as u16))
    }

    // The default preset: A / C character, B / I inventory, M message
    // log, N clear text, V minimap, O hireling, F9–F12 automap, Ctrl run,
    // the middle button automap, X1 show items, X2 run lock; right Ctrl
    // is Ctrl.
    #[test]
    fn original_keys_become_their_commands() {
        let b = Preset::Original.bindings().unwrap();
        use Key as K;
        let keys = [
            K::A,
            K::B,
            K::M,
            K::N,
            K::V,
            K::O,
            K::F9,
            K::F10,
            K::F11,
            K::F12,
            K::RightCtrl,
            K::MouseMiddle,
            K::Mouse4,
            K::Mouse5,
        ];
        assert_eq!(
            input_actions(&b, &keys),
            vec![
                act(Action::ToggleCharacter),
                act(Action::ToggleInventory),
                act(Action::ToggleMessageLog),
                act(Action::ClearTextMessages),
                act(Action::ToggleMinimap),
                act(Action::ToggleHireling),
                act(Action::CenterAutomap),
                act(Action::ToggleAutomapFade),
                act(Action::ToggleAutomapParty),
                act(Action::ToggleAutomapNames),
                act(Action::Run),
                act(Action::ToggleAutomap),
                act(Action::ShowItems),
                act(Action::ToggleRun),
            ]
        );
        assert!(action_held(&b, Action::Run, &|k| k == K::RightCtrl));
        assert!(!action_held(&b, Action::Run, &|k| k == K::LeftShift));
    }

    // Wheel +240 in one event: one step, command 39 (skill up) once.
    #[test]
    fn a_wheel_event_fires_one_skill_step() {
        let b = Preset::Original.bindings().unwrap();
        let mut acc = WheelAccumulator::default();
        let k = wheel_key(&mut acc, 240);
        assert_eq!(k, Some(Key::MouseWheelUp));
        assert_eq!(input_actions(&b, &[k.unwrap()]), vec![act(Action::SkillUp)]);
        assert_eq!(wheel_key(&mut acc, 60), None);
        assert_eq!(wheel_key(&mut acc, -200), Some(Key::MouseWheelDown));
        let down = input_actions(&b, &[Key::MouseWheelDown]);
        assert_eq!(down, vec![act(Action::SkillDown)]);
    }

    // Mode 0 runs no key command, mode 2 only the M2 ones (cmd 1 no, cmd 41
    // clear text messages yes), mode 1 all; Esc and the non-command inputs
    // always pass.
    #[test]
    fn key_modes_filter_the_key_actions() {
        let b = Preset::Original.bindings().unwrap();
        let keys = [KeyCode::KeyI, KeyCode::KeyV, KeyCode::Escape];
        let all = key_actions_in_mode(&b, &keys, 1);
        assert_eq!(all, key_actions(&b, &keys));
        assert!(all.contains(&act(Action::ToggleInventory)));
        let m0 = key_actions_in_mode(&b, &keys, 0);
        assert_eq!(m0, vec![act(Action::GameMenu)]);
        let m2 = key_actions_in_mode(&b, &keys, 2);
        assert!(!m2.contains(&act(Action::ToggleInventory)));
        assert!(m2.contains(&act(Action::GameMenu)));
        assert!(m2.contains(&act(Action::ToggleMinimap)));
    }
}
