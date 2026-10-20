//! The front end's UI element ids ([`crate::ui::ui_ids`]): the name of each
//! clickable control on the current screen, and the screens' own rows.
//!
//! A control's id follows from the screen it is on and what it does (its
//! [`Action`]), so the screens' layouts stay as they are and an id names
//! the same button wherever the layout puts it:
//!
//! | Screen | Ids |
//! |---|---|
//! | trademark | `trademark.continue` |
//! | main menu | `menu.single_player`, `menu.display_options`, `menu.credits`, `menu.exit` |
//! | character select | `char_select.char(NAME)` (a listed character), `char_select.ok`, `char_select.exit`, `char_select.create_new`, `char_select.convert`, `char_select.delete`, the pop-ups' `char_select.yes` / `char_select.no` / `char_select.message_ok` |
//! | difficulty (pop-up or screen) | `difficulty.normal`, `difficulty.nightmare`, `difficulty.hell` |
//! | character create | `create.class.<class>` (`create.class.amazon`...), `create.ok`, `create.exit`, `create.hardcore`, `create.expansion`, `create.warning_ok`, `create.warning_cancel`, `create.message_ok` |
//! | options | `options.<page>.<row>` (`options.video.vsync`; [`super::options`]) |
//! | credits | `credits.exit` |
//! | online login | `online_login.login`, `.new_account`, `.server`, `.back`, `.field(N)`, `.message_ok`, `.cancel` |
//! | online characters | the character select ids under `online_chars.` (`online_chars.char(NAME)`, `.ok`, `.exit`, `.create_new`, `.delete`, `.yes`, `.no`), and `.delete_ok`, `.delete_cancel`, `.message_ok`, `.cancel` |
//! | online create | the create ids under `online_create.`, and `.message_ok`, `.cancel` |
//! | online lobby | `online_lobby.back`, `.primary` (JOIN GAME / CREATE GAME), `.tab_create`, `.tab_join`, `.game(NAME)` (a listed game), `.difficulty.<name>`, `.field(N)`, `.prompt_ok`, `.prompt_cancel`, `.message_ok`, `.cancel`, `.scroll_up`, `.scroll_down` |
//!
//! Only what a click would reach is listed: enabled, visible controls that
//! take a click, and only when the click point is not covered by another
//! control (the list a pop-up covers is not on screen). Configure Controls
//! lists its visible bindings and buttons through its screen adapter.

use super::char_select::{ids as select, strings as select_strings};
use super::create::{act as create, Class};
use super::ids::{CHAR_CREATE, CHAR_SELECT, CREDITS, DIFFICULTY, MAIN_MENU, TRADEMARK};
use crate::ui::front_end::control::{Action, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::{FrontEnd, ScreenId};
use crate::ui::geom::Point;
use crate::ui::ui_ids::UiElement;

impl FrontEnd {
    /// The clickable elements on screen now, with their ids: the controls
    /// a click reaches, then the rows the screen lays out itself. Each is
    /// listed once (the first control of an id wins).
    pub fn ui_elements(&self) -> Vec<UiElement> {
        let mut out: Vec<UiElement> = Vec::new();
        for i in 0..self.controls.len() {
            let Some(id) = control_id(self.current, &self.controls, i) else {
                continue;
            };
            if out.iter().any(|e| e.id == id) {
                continue;
            }
            let Some(at) = self.click_point(i) else {
                continue;
            };
            let mut e = UiElement::new(
                id,
                self.canvas
                    .control(self.current, &self.controls[i])
                    .hit_box(),
            );
            e.at = at;
            out.push(e);
        }
        for mut e in self.registry.ui_elements(self.current) {
            e.rect.x += self.canvas.left();
            e.at.x += self.canvas.left();
            if !out.iter().any(|o| o.id == e.id) {
                out.push(e);
            }
        }
        out
    }

    /// A point that clicks control `i`: its hit box's centre when the
    /// click lands on it there, else the first point of a 4-pixel grid
    /// over the box that does (a control partly under another one, or a
    /// hero whose neighbour overlaps it). `None`: nothing of it can be
    /// clicked.
    fn click_point(&self, i: usize) -> Option<Point> {
        let r = self
            .canvas
            .control(self.current, &self.controls[i])
            .hit_box();
        let centre = crate::ui::ui_ids::centre(r);
        if self.hit(centre) == Some(i) {
            return Some(centre);
        }
        (0..i32::from(r.h))
            .step_by(4)
            .flat_map(|dy| {
                (0..i32::from(r.w))
                    .step_by(4)
                    .map(move |dx| Point::new(r.x + dx, r.y + dy))
            })
            .find(|&p| self.hit(p) == Some(i))
    }
}

/// The id of control `i` of `controls` on screen `screen`, or `None` when
/// it has none.
fn control_id(screen: ScreenId, controls: &[Control], i: usize) -> Option<String> {
    let c = &controls[i];
    if let Some(name) = super::online::control_name(screen, c.action) {
        return Some(format!("{}.{name}", screen.0));
    }
    // The online list and create screens are the select and create screens
    // on the server's characters: the same ids under their own prefix.
    let screen = match screen {
        s if s == super::online::ONLINE_CHARS => {
            return control_id(CHAR_SELECT, controls, i)
                .map(|id| id.replacen("char_select.", "online_chars.", 1))
        }
        s if s == super::online::ONLINE_CREATE => {
            return control_id(CHAR_CREATE, controls, i)
                .map(|id| id.replacen("create.", "online_create.", 1))
        }
        s => s,
    };
    if let Action::Trigger(Trigger::Difficulty(d)) = c.action {
        let name = ["normal", "nightmare", "hell"].get(usize::from(d))?;
        return Some(format!("difficulty.{name}"));
    }
    let prefix = match screen {
        s if s == MAIN_MENU => "menu",
        s if s == CHAR_CREATE => "create",
        s => s.0,
    };
    let name: String = match c.action {
        Action::None => return None,
        Action::Trigger(t) => match t {
            Trigger::Continue if screen == TRADEMARK => "continue".into(),
            Trigger::SinglePlayer => "single_player".into(),
            Trigger::Online => "online".into(),
            Trigger::Credits => "credits".into(),
            Trigger::Options => "display_options".into(),
            Trigger::CreateNew => "create_new".into(),
            Trigger::Exit
                if [MAIN_MENU, CHAR_SELECT, CHAR_CREATE, CREDITS, DIFFICULTY].contains(&screen) =>
            {
                "exit".into()
            }
            Trigger::Ok => "ok".into(),
            _ => return None,
        },
        Action::Custom(a) if screen == CHAR_SELECT => match a {
            select::OK => "ok".into(),
            select::CONVERT => "convert".into(),
            select::DELETE => "delete".into(),
            select::POPUP_YES => "yes".into(),
            // The message box's only button closes it as No does.
            select::POPUP_NO if c.string_id == select_strings::OK => "message_ok".into(),
            select::POPUP_NO => "no".into(),
            a if (select::SLOT_TEXT..select::SLOT_TEXT + 8).contains(&a) => {
                format!("char({})", slot_name(controls, c)?)
            }
            _ => return None,
        },
        Action::Custom(a) if screen == CHAR_CREATE => match a {
            create::OK => "ok".into(),
            create::HARDCORE => "hardcore".into(),
            create::EXPANSION => "expansion".into(),
            create::WARN_OK => "warning_ok".into(),
            create::WARN_CANCEL => "warning_cancel".into(),
            create::MESSAGE_CLOSE => "message_ok".into(),
            a if a >= create::CLASS => {
                let class = Class::from_id(u8::try_from(a - create::CLASS).ok()?)?;
                format!("class.{}", class_name(class))
            }
            _ => return None,
        },
        Action::Custom(_) => return None,
    };
    Some(format!("{prefix}.{name}"))
}

/// The character name a character select slot's click control stands
/// for: the slot's text control, at the same place, holds the name as its
/// first added row (after the title row).
fn slot_name<'a>(controls: &'a [Control], slot: &Control) -> Option<&'a str> {
    controls
        .iter()
        .find(|t| t.kind == ControlKind::Text && t.x == slot.x && t.y == slot.y)
        .and_then(|t| t.more_rows.first())
        .map(|r| r.text.as_str())
        .filter(|n| !n.is_empty())
}

/// A class's name in ids: its English name in lower case, as `--new`
/// takes it.
pub fn class_name(c: Class) -> &'static str {
    match c {
        Class::Amazon => "amazon",
        Class::Sorceress => "sorceress",
        Class::Necromancer => "necromancer",
        Class::Paladin => "paladin",
        Class::Barbarian => "barbarian",
        Class::Druid => "druid",
        Class::Assassin => "assassin",
    }
}

#[cfg(test)]
#[path = "element_ids_tests.rs"]
mod tests;
