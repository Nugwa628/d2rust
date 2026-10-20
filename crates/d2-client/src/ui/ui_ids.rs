//! UI element ids: a stable name for each clickable thing on screen, so a
//! test script can say `click ui:menu.single_player` instead of a pixel.
//!
//! Each screen names its clickable elements where it already lays out
//! their hit rectangles, and reports them with the rectangle in force on
//! that frame ([`UiElement`]). A script step looks the id up in the list of
//! the current frame ([`find`]) and clicks the centre of its rectangle, so
//! the script keeps working when the layout, the resolution or the frame
//! shape changes. An id that is not on screen is an error that lists the
//! ids that are, so a stale script says what it could click instead.
//!
//! Id grammar: lower-case words joined by dots, `screen.element`, with an
//! optional argument list in parentheses for elements of a list or grid
//! (`char_select.char(Name)`, later `inventory.slot(3,2)`). The front end
//! names its screens' buttons (`menu.*`, `char_select.*`, `create.*`,
//! `difficulty.*`, `options.<page>.<row>`, `credits.*`).

use super::geom::{Point, Rect};

/// The word that marks an id in a script: `click ui:<id>`.
pub const PREFIX: &str = "ui:";

/// One clickable element on the current frame: its id, its hit rectangle
/// and the point a click on it uses, in frame pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiElement {
    pub id: String,
    pub rect: Rect,
    /// Where a click on the element lands: the rectangle's centre, unless
    /// the screen found that point covered and picked another one inside.
    pub at: Point,
    /// Slider endpoints in frame pixels; explore probes only these values.
    pub ends: Option<[Point; 2]>,
    /// Why a click on the element does nothing in this state on purpose
    /// (the tab already shown, a list already at its end): explore does
    /// not report it as `no_response`. `None`: a click should respond.
    pub idle: Option<&'static str>,
}

impl UiElement {
    /// An element clicked at its rectangle's centre.
    pub fn new(id: impl Into<String>, rect: Rect) -> Self {
        UiElement {
            id: id.into(),
            at: centre(rect),
            ends: None,
            idle: None,
            rect,
        }
    }

    /// The element marked idle for `reason` when `when` holds.
    pub fn idle_if(mut self, when: bool, reason: &'static str) -> Self {
        if when {
            self.idle = Some(reason);
        }
        self
    }
}

/// The centre of `r`, rounded down, so inside any non-empty rectangle.
pub fn centre(r: Rect) -> Point {
    Point::new(r.x + i32::from(r.w) / 2, r.y + i32::from(r.h) / 2)
}

/// The id of a `ui:<id>` word, or `None` when the word is not one.
pub fn parse_ref(word: &str) -> Option<&str> {
    word.strip_prefix(PREFIX).filter(|id| valid_id(id))
}

/// Whether `id` follows the grammar: dot-separated words of letters,
/// digits and `_`, the last one with an optional `(args)` tail.
pub fn valid_id(id: &str) -> bool {
    let (head, args) = match id.split_once('(') {
        Some((h, rest)) => match rest.strip_suffix(')') {
            Some(a) => (h, Some(a)),
            None => return false,
        },
        None => (id, None),
    };
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    head.split('.').all(word)
        && args.is_none_or(|a| {
            !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ','))
        })
}

/// The element `id` among `elements` (the current frame's), or an error
/// naming the ids that are on screen.
pub fn find<'a>(elements: &'a [UiElement], id: &str) -> Result<&'a UiElement, String> {
    elements
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| unknown(elements, id))
}

/// The error for an id that is not on screen: the ids that are.
pub fn unknown(elements: &[UiElement], id: &str) -> String {
    if elements.is_empty() {
        format!("unknown ui id `{id}`: no ui ids on screen")
    } else {
        let ids: Vec<&str> = elements.iter().map(|e| e.id.as_str()).collect();
        format!("unknown ui id `{id}`; on screen: {}", ids.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_parse_with_and_without_arguments() {
        assert_eq!(
            parse_ref("ui:menu.single_player"),
            Some("menu.single_player")
        );
        assert_eq!(
            parse_ref("ui:options.video.vsync"),
            Some("options.video.vsync")
        );
        assert_eq!(
            parse_ref("ui:char_select.char(Ab-c_d)"),
            Some("char_select.char(Ab-c_d)")
        );
        assert_eq!(
            parse_ref("ui:inventory.slot(3,2)"),
            Some("inventory.slot(3,2)")
        );
        for bad in [
            "menu.x", "ui:", "ui:a..b", "ui:a.b(", "ui:a()", "ui:a b", "ui:.a",
        ] {
            assert_eq!(parse_ref(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_click_lands_inside_the_rectangle() {
        let e = UiElement::new("a", Rect::new(264, 289, 272, 35));
        assert_eq!(e.at, Point::new(400, 306));
        assert!(e.rect.contains(e.at));
        let one = UiElement::new("b", Rect::new(5, 6, 1, 1));
        assert_eq!(one.at, Point::new(5, 6));
    }

    #[test]
    fn an_unknown_id_lists_the_ids_on_screen() {
        let els = vec![
            UiElement::new("menu.single_player", Rect::new(0, 0, 10, 10)),
            UiElement::new("menu.exit", Rect::new(0, 20, 10, 10)),
        ];
        assert_eq!(find(&els, "menu.exit").unwrap().rect.y, 20);
        let e = find(&els, "menu.nope").unwrap_err();
        assert!(
            e.contains("menu.nope") && e.contains("menu.single_player menu.exit"),
            "{e}"
        );
        assert_eq!(
            find(&[], "x").unwrap_err(),
            "unknown ui id `x`: no ui ids on screen"
        );
    }
}
