//! One module per screen. Each exposes `register(&mut Registry)`; a screen
//! session edits only its own module. [`register_all`] calls them all.

pub mod char_select;
pub mod controls;
pub mod create;
pub mod credits;
pub mod difficulty;
pub mod element_ids;
pub mod loading;
pub mod main_menu;
pub mod online;
pub mod options;
pub mod trademark;

use super::screen::Registry;

/// The screen ids (open set: a new screen adds a const in its own module).
pub mod ids {
    use crate::ui::front_end::ScreenId;

    pub const TRADEMARK: ScreenId = ScreenId("trademark");
    pub const MAIN_MENU: ScreenId = ScreenId("main_menu");
    pub const CHAR_SELECT: ScreenId = ScreenId("char_select");
    pub const CHAR_CREATE: ScreenId = ScreenId("char_create");
    pub const DIFFICULTY: ScreenId = ScreenId("difficulty");
    pub const CREDITS: ScreenId = ScreenId("credits");
    pub const LOADING: ScreenId = ScreenId("loading");
    pub const OPTIONS: ScreenId = ScreenId("options");
    pub const CONTROLS: ScreenId = ScreenId("controls");
}

pub fn register_all(reg: &mut Registry) {
    trademark::register(reg);
    main_menu::register(reg);
    char_select::register(reg);
    create::register(reg);
    difficulty::register(reg);
    loading::register(reg);
    options::register(reg);
    controls::register(reg);
    credits::register(reg);
    online::register(reg);
}
