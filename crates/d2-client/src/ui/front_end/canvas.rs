//! Canvas layout shared by front-end drawing and control hit regions.

use super::{Control, ControlKind, DrawItem, ScreenId, CHAR_CREATE, MAIN_MENU};
use crate::rules::camera::FrameSize;
use crate::ui::geom::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontCanvas {
    pub width: i32,
    pub height: i32,
}

impl Default for FrontCanvas {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
        }
    }
}

impl FrontCanvas {
    pub fn new(frame: FrameSize) -> Self {
        Self {
            width: frame.width,
            height: frame.height,
        }
    }
    pub fn left(self) -> i32 {
        (self.width - 800) / 2
    }
    pub fn local(self, p: Point) -> Point {
        Point::new(p.x - self.left(), p.y)
    }

    /// Controls retain canonical coordinates, so state updates and repeated
    /// canvas switches never accumulate a layout offset.
    pub fn control(self, screen: ScreenId, c: &Control) -> Control {
        let mut c = c.clone();
        c.x += self.left();
        let footer = c.y >= 500 && c.h < 200;
        let menu_button = screen == MAIN_MENU && c.kind == ControlKind::Button;
        if footer || menu_button {
            c.y += self.height - 600;
        }
        if screen == CHAR_CREATE && c.late {
            // The fire ends above the name-entry group even at 540 rows.
            c.y += (self.height - 600).min(0);
        }
        c
    }

    pub fn shift_item(self, item: &mut DrawItem) {
        let at = match item {
            DrawItem::Art { at, .. }
            | DrawItem::Text { at, .. }
            | DrawItem::Blend { at, .. }
            | DrawItem::Doll { at, .. }
            | DrawItem::UiCel { at, .. }
            | DrawItem::Shade { at, .. }
            | DrawItem::Rect { at, .. }
            | DrawItem::Border { at, .. } => at,
            DrawItem::Dim { .. } => return,
        };
        at.x += self.left();
    }
}
