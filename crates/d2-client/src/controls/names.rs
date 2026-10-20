//! The closed name lists of the controls file: [`Key`] (inputs), [`Action`] (commands) and
//! [`Context`] (where an action is active).
//!
//! The lists are fixed and every item has a stable text name, so the controls file can be read and
//! written without guessing. Both lists use D2RUST's own names. Renaming an action goes through
//! `controls::migrate` so existing controls files keep working.

/// Where an action is active. One input triggers at most one action per context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Context {
    /// Playing in the world: movement, skills, panels, belt.
    World,
    /// Typing in the chat line.
    Chat,
    /// An open panel handling selection and close.
    Panel,
}

impl Context {
    /// Every context, in declaration order.
    pub const ALL: [Context; 3] = [Context::World, Context::Chat, Context::Panel];

    /// Name as written in the controls file.
    pub fn name(self) -> &'static str {
        match self {
            Context::World => "world",
            Context::Chat => "chat",
            Context::Panel => "panel",
        }
    }
}

macro_rules! keys {
    ($($variant:ident => $name:literal,)*) => {
        /// An input: a keyboard key, mouse button or wheel direction, by its
        /// portable name (never a platform scancode).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Key {
            $($variant,)*
        }

        impl Key {
            /// Every key, in enum order.
            pub const ALL: &'static [Key] = &[$(Key::$variant,)*];

            /// Name as written in the controls file.
            pub fn name(self) -> &'static str {
                match self {
                    $(Key::$variant => $name,)*
                }
            }

            /// Exact (case-sensitive) name lookup.
            pub fn from_name(name: &str) -> Option<Key> {
                match name {
                    $($name => Some(Key::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

keys! {
    A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G",
    H => "H", I => "I", J => "J", K => "K", L => "L", M => "M", N => "N",
    O => "O", P => "P", Q => "Q", R => "R", S => "S", T => "T", U => "U",
    V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
    Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3", Digit4 => "4",
    Digit5 => "5", Digit6 => "6", Digit7 => "7", Digit8 => "8", Digit9 => "9",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
    F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
    LeftShift => "LeftShift", RightShift => "RightShift",
    LeftCtrl => "LeftCtrl", RightCtrl => "RightCtrl",
    LeftAlt => "LeftAlt", RightAlt => "RightAlt",
    Space => "Space", Enter => "Enter", Escape => "Escape", Tab => "Tab",
    Backspace => "Backspace", Insert => "Insert", Delete => "Delete",
    Home => "Home", End => "End", PageUp => "PageUp", PageDown => "PageDown",
    Up => "Up", Down => "Down", Left => "Left", Right => "Right",
    Grave => "Grave", Minus => "Minus", Equals => "Equals",
    LeftBracket => "LeftBracket", RightBracket => "RightBracket",
    Backslash => "Backslash", Semicolon => "Semicolon", Quote => "Quote",
    Comma => "Comma", Period => "Period", Slash => "Slash",
    PrintScreen => "PrintScreen", Pause => "Pause",
    Numpad0 => "Numpad0", Numpad1 => "Numpad1", Numpad2 => "Numpad2",
    Numpad3 => "Numpad3", Numpad4 => "Numpad4", Numpad5 => "Numpad5",
    Numpad6 => "Numpad6", Numpad7 => "Numpad7", Numpad8 => "Numpad8",
    Numpad9 => "Numpad9", NumpadAdd => "NumpadAdd",
    NumpadSubtract => "NumpadSubtract", NumpadMultiply => "NumpadMultiply",
    NumpadDivide => "NumpadDivide", NumpadEnter => "NumpadEnter",
    MouseLeft => "MouseLeft", MouseRight => "MouseRight",
    MouseMiddle => "MouseMiddle", Mouse4 => "Mouse4", Mouse5 => "Mouse5",
    MouseWheelUp => "MouseWheelUp", MouseWheelDown => "MouseWheelDown",
}

macro_rules! actions {
    ($($variant:ident => $name:literal, $ctx:ident;)*) => {
        /// A bindable command. Enum order is the order the controls file is written in.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Action {
            $($variant,)*
        }

        impl Action {
            /// Every action, in enum order.
            pub const ALL: &'static [Action] = &[$(Action::$variant,)*];

            /// Name as written in the controls file.
            pub fn name(self) -> &'static str {
                match self {
                    $(Action::$variant => $name,)*
                }
            }

            /// The context the action belongs to.
            pub fn context(self) -> Context {
                match self {
                    $(Action::$variant => Context::$ctx,)*
                }
            }

            /// Exact (case-sensitive) name lookup.
            pub fn from_name(name: &str) -> Option<Action> {
                match name {
                    $($name => Some(Action::$variant),)*
                    _ => None,
                }
            }

            /// Position in [`Action::ALL`].
            pub fn index(self) -> usize {
                self as usize
            }
        }
    };
}

actions! {
    MoveAttack => "move_attack", World;
    UseRightSkill => "use_right_skill", World;
    StandStill => "stand_still", World;
    ShowItems => "show_items", World;
    ToggleRun => "toggle_run", World;
    SwapWeapons => "swap_weapons", World;
    ToggleInventory => "toggle_inventory", World;
    ToggleCharacter => "toggle_character", World;
    ToggleSkillTree => "toggle_skill_tree", World;
    ToggleSkillMenuLeft => "toggle_skill_menu_left", World;
    ToggleSkillMenuRight => "toggle_skill_menu_right", World;
    ToggleQuests => "toggle_quests", World;
    ToggleParty => "toggle_party", World;
    ToggleAutomap => "toggle_automap", World;
    ToggleAutomapFade => "toggle_automap_fade", World;
    CenterAutomap => "center_automap", World;
    ToggleBelt => "toggle_belt", World;
    ShowPortraits => "show_portraits", World;
    OpenChat => "open_chat", World;
    ClearScreen => "clear_screen", World;
    GameMenu => "game_menu", World;
    ToggleHelp => "toggle_help", World;
    Screenshot => "screenshot", World;
    SkillSlot1 => "skill_slot_1", World;
    SkillSlot2 => "skill_slot_2", World;
    SkillSlot3 => "skill_slot_3", World;
    SkillSlot4 => "skill_slot_4", World;
    SkillSlot5 => "skill_slot_5", World;
    SkillSlot6 => "skill_slot_6", World;
    SkillSlot7 => "skill_slot_7", World;
    SkillSlot8 => "skill_slot_8", World;
    SkillSlot9 => "skill_slot_9", World;
    SkillSlot10 => "skill_slot_10", World;
    SkillSlot11 => "skill_slot_11", World;
    SkillSlot12 => "skill_slot_12", World;
    SkillSlot13 => "skill_slot_13", World;
    SkillSlot14 => "skill_slot_14", World;
    SkillSlot15 => "skill_slot_15", World;
    SkillSlot16 => "skill_slot_16", World;
    BeltSlot1 => "belt_slot_1", World;
    BeltSlot2 => "belt_slot_2", World;
    BeltSlot3 => "belt_slot_3", World;
    BeltSlot4 => "belt_slot_4", World;
    ChatSend => "chat_send", Chat;
    ChatCancel => "chat_cancel", Chat;
    ChatHistoryPrev => "chat_history_prev", Chat;
    ChatHistoryNext => "chat_history_next", Chat;
    PanelSelect => "panel_select", Panel;
    PanelAlt => "panel_alt", Panel;
    PanelClose => "panel_close", Panel;
    // Commands added after the list above; they are appended so the indexes of the earlier
    // actions stay unchanged.
    ToggleMessageLog => "toggle_message_log", World;
    ToggleAutomapParty => "toggle_automap_party", World;
    ToggleAutomapNames => "toggle_automap_names", World;
    Say0 => "say_0", World;
    Say1 => "say_1", World;
    Say2 => "say_2", World;
    Say3 => "say_3", World;
    Say4 => "say_4", World;
    Say5 => "say_5", World;
    Say6 => "say_6", World;
    Say7X => "say_7x", World;
    Run => "run", World;
    SkillUp => "skill_up", World;
    SkillDown => "skill_down", World;
    ClearTextMessages => "clear_text_messages", World;
    ToggleMinimap => "toggle_minimap", World;
    ToggleHireling => "toggle_hireling", World;
}

/// The action a script names (`action toggle_inventory`): its
/// controls-file name, in any case, with `-` read as `_`
/// (`Toggle-Run` is `toggle_run`). Lookup only: the names themselves are
/// [`Action::name`]'s.
pub fn action_by_name(name: &str) -> Option<Action> {
    let want = name.replace('-', "_").to_ascii_lowercase();
    Action::ALL.iter().copied().find(|a| a.name() == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_by_name_reads_the_controls_names_in_any_case() {
        assert_eq!(
            action_by_name("toggle_inventory"),
            Some(Action::ToggleInventory)
        );
        assert_eq!(action_by_name("Toggle-Run"), Some(Action::ToggleRun));
        assert_eq!(action_by_name("SKILL_SLOT_16"), Some(Action::SkillSlot16));
        assert_eq!(action_by_name("toggle inventory"), None);
        assert_eq!(action_by_name(""), None);
        for a in Action::ALL {
            assert_eq!(action_by_name(a.name()), Some(*a));
        }
    }
}
