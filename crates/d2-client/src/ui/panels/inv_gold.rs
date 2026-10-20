//! The inventory gold line (k = 1): the full stat 14 `gold` of
//! the player as `%d` in Font16, color 0, not centered, at (`W − sx − 212`, `H +
//! sy − 72`), and the gold button `Panel\goldcoinbtn` frame `p` at (`W − sx −
//! 236`, `H + sy − 71 + p`), `p` = the pressed flag. Both are the panel-1 rows
//! `gold` and `goldbtn` of `panel-layout.tsv`. The client draws the model value
//! only.
//!
//! Open:
//! - whether the gold line draws the value before or
//!   after the button is not settled; they do not overlap (the button ends at
//!   `W − sx − 217`, the value starts at `W − sx − 212`), so the order is
//!   not visible. Drawn here value first (the table's column order).
//! - the press / release of the gold button (sound 4, the gold
//!   dialog) is not wired: the pressed flag stays clear.

use super::super::draw::UiDrawSink;
use super::super::layout::{ColorSpec, PanelKey, RowKind};
use super::{cel, emit_static_draws, no_extra, text, utf16, PanelEnv, PanelTables};
use crate::ui::gold::{gold_text, GoldLayout, GoldLine};

/// The inventory panel's rows (ui 1).
const PANEL: PanelKey = PanelKey::Ui(1);

/// Stat 14 `gold` (k = 1).
pub const STAT_GOLD: u16 = 14;

/// The gold value (`gold`, `%d` of the full stat, `None` without a player or
/// fonts: not drawn), then the gold button with pressed flag `pressed` (k = 1).
pub fn draw_inventory_gold(
    t: &PanelTables,
    env: &PanelEnv,
    pressed: bool,
    gold: Option<i32>,
    out: &mut dyn UiDrawSink,
) {
    debug_assert_eq!(GoldLine::Inventory.stat(), STAT_GOLD);
    if let Some(v) = gold {
        let s = &env.screen;
        if let Some(r) = t.item(PANEL, "gold", RowKind::Text).next() {
            let color = match r.color {
                ColorSpec::Index(k) => k,
                _ => 0,
            };
            out.push(text(
                utf16(&gold_text(v)),
                r.x.eval(s),
                r.y.eval(s),
                r.font.unwrap_or(1),
                color,
            ));
        }
    }
    let cond = env.cond(pressed, &no_extra);
    emit_static_draws(t, PANEL, &cond, None, &|r| r.item == "goldbtn", out);
}

/// The stash gold line (k = 0; the stash modes 0x0C / 0x0D;
/// after the stash grid): the full stat 15 `goldbank` as `%d` in Font16, color
/// 0, at the stash pen, then the gold button `Panel\goldcoinbtn` frame
/// `pressed` ([`GoldLine::Stash`]). `None` (no player or fonts): only the
/// button, as the inventory line.
pub fn draw_stash_gold(
    t: &PanelTables,
    env: &PanelEnv,
    pressed: bool,
    gold: Option<i32>,
    out: &mut dyn UiDrawSink,
) {
    let s = &env.screen;
    let l = GoldLayout {
        sx: s.sx(),
        sy: s.sy(),
        w: s.w,
        h: s.h,
        expansion: env.exp,
    };
    if let Some(v) = gold {
        let at = GoldLine::Stash.value_pen(&l, 0);
        out.push(text(utf16(&gold_text(v)), at.x, at.y, 1, 0));
    }
    if let (Some(file), Some(at)) = (
        t.files.id("panel\\goldcoinbtn"),
        GoldLine::Stash.button(&l, pressed),
    ) {
        out.push(cel(file, u32::from(pressed), at.x, at.y));
    }
}

/// String 3315 `stash` "Stash", the shop line's label (k = 2).
pub const STR_STASH: u16 = 3315;

/// The shop gold line (k = 2; right after the shop
/// draw): the full stat 15 `goldbank` as `%d` in Font16, color 0, right-aligned
/// to `sx + 198` by `value_width` (width A of the value), then `label` (string
/// 3315) at (`sx + 21`, `H + sy − 106`), color 0. No button. `None` (no player
/// or fonts): only the label.
pub fn draw_shop_gold(
    env: &PanelEnv,
    gold: Option<(i32, i32)>,
    label: Option<&[u16]>,
    out: &mut dyn UiDrawSink,
) {
    let s = &env.screen;
    let l = GoldLayout {
        sx: s.sx(),
        sy: s.sy(),
        w: s.w,
        h: s.h,
        expansion: env.exp,
    };
    if let Some((v, value_width)) = gold {
        let at = GoldLine::Shop.value_pen(&l, value_width);
        out.push(text(utf16(&gold_text(v)), at.x, at.y, 1, 0));
    }
    if let Some(t) = label {
        let at = crate::ui::gold::shop_label_pos(&l);
        out.push(text(t.to_vec(), at.x, at.y, 1, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::draw::UiDraw;
    use super::super::super::geom::Point;
    use super::super::super::layout::Screen;
    use super::*;
    use crate::ui::gold::GoldLayout;

    fn env(screen: Screen) -> PanelEnv {
        PanelEnv {
            screen,
            open_mode: 1,
            exp: true,
        }
    }

    fn layout(s: &Screen) -> GoldLayout {
        GoldLayout {
            sx: s.sx(),
            sy: s.sy(),
            w: s.w,
            h: s.h,
            expansion: true,
        }
    }

    #[test]
    fn gold_value_and_button_800() {
        let t = PanelTables::load().unwrap();
        let s = Screen::R800;
        let btn = t.files.id("panel\\goldcoinbtn").expect("goldcoinbtn file");
        let mut out: Vec<UiDraw> = Vec::new();
        draw_inventory_gold(&t, &env(s), false, Some(12345), &mut out);
        // W − sx − 212 = 800 − 80 − 212; H + sy − 72 = 600 − 60 − 72.
        match &out[0] {
            UiDraw::Text(x) => {
                assert_eq!(String::from_utf16_lossy(&x.text), "12345");
                assert_eq!(x.at, Point::new(508, 468));
                assert_eq!((x.style.font, x.style.color), (1, 0));
                assert_eq!(x.at, GoldLine::Inventory.value_pen(&layout(&s), 0));
            }
            d => panic!("text first: {d:?}"),
        }
        match &out[1] {
            UiDraw::Image(i) => {
                assert_eq!((i.image.file, i.image.frame), (btn, 0));
                assert_eq!(i.at, Point::new(484, 469));
                assert_eq!(Some(i.at), GoldLine::Inventory.button(&layout(&s), false));
            }
            d => panic!("button second: {d:?}"),
        }
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn pressed_button_is_frame_1_one_row_lower_and_no_value_without_player() {
        let t = PanelTables::load().unwrap();
        let s = Screen::R640;
        let mut out: Vec<UiDraw> = Vec::new();
        draw_inventory_gold(&t, &env(s), true, None, &mut out);
        assert_eq!(out.len(), 1);
        match &out[0] {
            UiDraw::Image(i) => {
                assert_eq!(i.image.frame, 1);
                // 640 − 236, 480 − 71 + 1.
                assert_eq!(i.at, Point::new(404, 410));
                assert_eq!(Some(i.at), GoldLine::Inventory.button(&layout(&s), true));
            }
            d => panic!("{d:?}"),
        }
        // Negative values keep their sign (`%d`).
        let mut out: Vec<UiDraw> = Vec::new();
        draw_inventory_gold(&t, &env(s), false, Some(-5), &mut out);
        assert!(matches!(&out[0], UiDraw::Text(x) if x.text == utf16("-5")));
    }

    #[test]
    fn shop_line_is_the_right_aligned_value_then_the_stash_label() {
        let s = Screen::R800;
        let mut out: Vec<UiDraw> = Vec::new();
        draw_shop_gold(&env(s), Some((0, 12)), Some(&utf16("Stash")), &mut out);
        let at: Vec<_> = out
            .iter()
            .map(|d| match d {
                UiDraw::Text(x) => (String::from_utf16_lossy(&x.text), x.at, x.style.color),
                d => panic!("{d:?}"),
            })
            .collect();
        // 80 + 198 − 12, 600 − 60 − 106; then 80 + 21 (gen-ui-shop tick
        // 120 rows 44–49).
        assert_eq!(
            at,
            vec![
                ("0".to_string(), Point::new(266, 434), 0),
                ("Stash".to_string(), Point::new(101, 434), 0),
            ]
        );
    }
}
