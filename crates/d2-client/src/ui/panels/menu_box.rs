//! The NPC menu box: an object with a position, a size
//! and up to ten text items, drawn as a framed box with the selected item
//! in color 3 (style 1) or between two spinning pentagrams (style 2).
//! The NPC menu, the talk topic box, the hire
//! list, the confirm dialog and the "waiting for confirmation" note are
//! all this one object. Handlers are the caller's type `H`.

use crate::ui::messages::{Ltrb, Metrics, RectDraw, FONT_16};

/// Items per box; more is fatal.
pub const MAX_ITEMS: usize = 10;
/// Item text is cut to 119 units.
pub const MAX_TEXT: usize = 119;
/// Pentagram frames: `% 7` on an 8-frame file.
pub const PENTSPIN_MOD: u32 = 7;

/// Failures the game treats as fatal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MenuError {
    #[error("more than 10 items")]
    TooManyItems,
    #[error("color 3 with style 1")]
    Color3Style1,
    #[error("auto layout wider than W − 80 ({0})")]
    TooWide(i32),
    #[error("auto layout taller than H − 98 ({0})")]
    TooTall(i32),
}

/// One item (stride 0x110 from +0x68).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem<H> {
    pub text: Vec<u16>,
    /// +0x158.
    pub height: i32,
    /// +0x164.
    pub a2: i32,
    /// +0x168.
    pub font: u16,
    /// +0x16C.
    pub color: u8,
    /// +0x170.
    pub handler: Option<H>,
    /// +0x174.
    pub selectable: bool,
    /// The item's x offset.
    pub x_off: i32,
}

/// The constructor arguments p1 … p9.
#[derive(Clone, Debug)]
pub struct MenuParams<H> {
    /// Callback p1 (+0x60): required, else no box.
    pub p1: Option<H>,
    /// p2 (+0x64).
    pub p2: Option<H>,
    /// p3 (+0x18).
    pub p3: i32,
    /// p4 (+0x14): the y anchor rises by 21 per non-selectable item.
    pub p4: bool,
    /// p5 (+0x58): auto layout.
    pub p5: bool,
    /// p6 width (+0x34).
    pub p6: i32,
    /// p7 height (+0x38).
    pub p7: i32,
    /// p8 (+0x1C).
    pub p8: i32,
    /// p9 (+0x20).
    pub p9: i32,
}

impl<H> Default for MenuParams<H> {
    fn default() -> Self {
        Self {
            p1: None,
            p2: None,
            p3: 0,
            p4: false,
            p5: false,
            p6: 0,
            p7: 0,
            p8: 0,
            p9: 0,
        }
    }
}

/// The menu object.
#[derive(Clone, Debug)]
pub struct MenuBox<H> {
    /// Anchor x (+0x24, +0x2C), y (+0x28, +0x30).
    pub anchor: (i32, i32),
    pub params: MenuParams<H>,
    /// Selected item (+0x44), −1 for none.
    pub selected: i32,
    /// Style (+0x5C): 2 by default.
    pub style: u8,
    pub items: Vec<MenuItem<H>>,
    /// Top-left of the drawn box and its size.
    pub pos: (i32, i32),
    pub size: (i32, i32),
    /// Number of selectable items (+0x4C).
    pub selectable: usize,
    /// A cel background (+0x0C ≠ 0).
    pub has_cel: bool,
}

impl<H: Clone + PartialEq> MenuBox<H> {
    /// Without p1 there is no box. Style 2, nothing
    /// selected.
    pub fn new(anchor: (i32, i32), params: MenuParams<H>) -> Option<Self> {
        params.p1.as_ref()?;
        Some(Self {
            anchor,
            pos: if params.p5 { (0, 0) } else { anchor },
            size: (params.p6, params.p7),
            params,
            selected: -1,
            style: 2,
            items: Vec::new(),
            selectable: 0,
            has_cel: false,
        })
    }

    pub fn set_style(&mut self, style: u8) {
        self.style = style;
    }

    /// at most 10 items, text cut to 119 units, color 3 with style 1
    /// fatal. The first selectable item added while none is selected
    /// becomes selected. With p5 = 0 only the item's x
    /// offset is computed (against the fixed width p6).
    #[allow(clippy::too_many_arguments)]
    pub fn add_item(
        &mut self,
        text: &[u16],
        height: i32,
        a2: i32,
        color: u8,
        font: u16,
        handler: Option<H>,
        selectable: bool,
        m: &dyn Metrics,
    ) -> Result<(), MenuError> {
        if self.items.len() >= MAX_ITEMS {
            return Err(MenuError::TooManyItems);
        }
        if color == 3 && self.style == 1 {
            return Err(MenuError::Color3Style1);
        }
        let text: Vec<u16> = text.iter().copied().take(MAX_TEXT).collect();
        let x_off = if self.params.p5 {
            0
        } else {
            item_x_off(self.params.p6, m.width_a(font, &text))
        };
        self.items.push(MenuItem {
            text,
            height,
            a2,
            font,
            color,
            handler,
            selectable,
            x_off,
        });
        if selectable {
            self.selectable += 1;
            if self.selected < 0 {
                self.selected = self.items.len() as i32 - 1;
            }
        }
        Ok(())
    }

    /// Auto layout (p5 ≠ 0; measured in font 1) in a frame W × H.
    pub fn layout(&mut self, w_frame: i32, h_frame: i32, m: &dyn Metrics) -> Result<(), MenuError> {
        let widths: Vec<i32> = self
            .items
            .iter()
            .map(|i| m.width_a(i.font, &i.text))
            .collect();
        let w = widths.iter().copied().max().unwrap_or(0) + 20;
        let h = self.items.iter().map(|i| i.height).sum::<i32>() + 15;
        if w > w_frame - 80 {
            return Err(MenuError::TooWide(w));
        }
        if h > h_frame - 98 {
            return Err(MenuError::TooTall(h));
        }
        let mut x = self.anchor.0 - w / 2;
        let rise = if self.params.p4 {
            21 * self.items.iter().filter(|i| !i.selectable).count() as i32
        } else {
            h / 3
        };
        let mut y = self.anchor.1 - rise;
        if x + w > w_frame - 10 {
            x = w_frame - w;
        }
        if y + h > h_frame - 58 {
            y = h_frame - h - 48;
        }
        if x <= 10 {
            x = 10;
        }
        if y <= 10 {
            y = 10;
        }
        self.pos = (x, y);
        self.size = (w, h);
        for (item, width) in self.items.iter_mut().zip(widths) {
            item.x_off = item_x_off(w, width);
        }
        Ok(())
    }

    /// The box as a rectangle.
    pub fn rect(&self) -> Ltrb {
        Ltrb::xywh(self.pos.0, self.pos.1, self.size.0, self.size.1)
    }

    /// The draw. `pentspin` is the
    /// pentagram counter, stepped once per style-2 selected item drawn.
    pub fn draw(&self, pentspin: &mut u32, m: &dyn Metrics) -> Vec<MenuDraw> {
        let (x, y) = self.pos;
        let (w, h) = self.size;
        let mut out = Vec::new();
        if self.has_cel {
            out.push(MenuDraw::Cel {
                frame: 0,
                x,
                y: y + h,
                mode: 5,
            });
        } else {
            out.push(MenuDraw::Frame(RectDraw {
                x,
                y,
                w,
                h,
                color: 0,
                mode: 1,
            }));
            out.push(MenuDraw::Border {
                l: x,
                t: y,
                r: x + w - 1,
                b: y + h - 1,
            });
        }
        let mut pen_y = y;
        for (i, item) in self.items.iter().enumerate() {
            pen_y += item.height;
            let pen_x = x + item.x_off;
            let mut color = item.color;
            if self.selected == i as i32 {
                if self.style == 1 {
                    color = 3;
                } else if self.style == 2 {
                    let width = m.width_a(item.font, &item.text);
                    let frame = *pentspin % PENTSPIN_MOD;
                    out.push(MenuDraw::Pentspin {
                        frame,
                        x: pen_x - 24,
                        y: pen_y + 4,
                    });
                    out.push(MenuDraw::Pentspin {
                        frame,
                        x: pen_x + width + 2,
                        y: pen_y + 4,
                    });
                    *pentspin += 1;
                }
            }
            out.push(MenuDraw::Text {
                text: item.text.clone(),
                x: pen_x,
                y: pen_y,
                color,
                font: item.font,
            });
        }
        out
    }
}

/// An item's x offset: `(w − width + 1) / 2 + 1` when the width is below
/// the box's, else 0.
pub fn item_x_off(w: i32, width: i32) -> i32 {
    if width < w {
        (w - width + 1) / 2 + 1
    } else {
        0
    }
}

/// One draw request of a menu box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuDraw {
    /// Frame 0 of the box's cel at (x, y + h), mode 5.
    Cel {
        frame: u32,
        x: i32,
        y: i32,
        mode: u8,
    },
    /// The framed box.
    Frame(RectDraw),
    /// The border (`menu\boxpieces`) around (l, t, r, b),
    /// both edges inclusive: the call that follows the framed box in.
    Border { l: i32, t: i32, r: i32, b: i32 },
    /// `cursor\Pentspin` frame at (x, y), mode 5.
    Pentspin { frame: u32, x: i32, y: i32 },
    Text {
        text: Vec<u16>,
        x: i32,
        y: i32,
        color: u8,
        font: u16,
    },
}

/// The anchor: (px, py) is the NPC's client pixel
/// point, `origin` the tile origin (view +0x24, +0x28). Without the
/// interaction's NPC there is no box.
pub fn anchor(npc_px: Option<(i32, i32)>, origin: (i32, i32)) -> Option<(i32, i32)> {
    let (px, py) = npc_px?;
    Some((px - origin.0, (py - origin.1 - 150).max(20)))
}

/// Handler of the waiting note (p1): nothing but the tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteHandler;

/// The "waiting for confirmation" note with its deadline.
#[derive(Clone, Debug)]
pub struct WaitingNote {
    pub bx: MenuBox<NoteHandler>,
    /// The deadline: now + 60,000 ms.
    pub deadline: u32,
}

/// `TransactionResults1` (3353).
pub const STR_WAITING: u16 = 3353;
/// The note lives 60 s.
pub const NOTE_MS: u32 = 60_000;

impl WaitingNote {
    /// A box at `anchor`, p5 = 1, p9 = 1, style 0, one item
    /// `TransactionResults1`, height 15, font 1, color 0, not selectable.
    /// A second one while one exists is fatal: pass the existing one in
    /// `existing`.
    pub fn open(
        existing: Option<&WaitingNote>,
        anchor: (i32, i32),
        text: &[u16],
        now: u32,
        m: &dyn Metrics,
    ) -> Result<Self, NoteError> {
        if existing.is_some() {
            return Err(NoteError::Second);
        }
        let mut bx = MenuBox::new(
            anchor,
            MenuParams {
                p1: Some(NoteHandler),
                p5: true,
                p9: 1,
                ..Default::default()
            },
        )
        .ok_or(NoteError::NoBox)?;
        bx.set_style(0);
        bx.add_item(text, 15, 0, 0, FONT_16, None, false, m)
            .map_err(NoteError::Menu)?;
        Ok(Self {
            bx,
            deadline: now.wrapping_add(NOTE_MS),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NoteError {
    #[error("a second waiting note is fatal")]
    Second,
    #[error("no box")]
    NoBox,
    #[error(transparent)]
    Menu(MenuError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::messages::testutil::{w, Fixed};

    /// Width = 10 · units.
    struct Ten;
    impl Metrics for Ten {
        fn wrap(&self, _: u16, t: &[u16], _: i32) -> Vec<Vec<u16>> {
            vec![t.to_vec()]
        }
        fn width_a(&self, _: u16, t: &[u16]) -> i32 {
            10 * t.len() as i32
        }
        fn width_c(&self, _: u16, t: &[u16]) -> i32 {
            10 * t.len() as i32
        }
        fn font_height(&self, _: u16) -> i32 {
            16
        }
    }

    fn params(p5: bool) -> MenuParams<u8> {
        MenuParams {
            p1: Some(1),
            p5,
            ..Default::default()
        }
    }

    #[test]
    fn object_items_limits_and_selection() {
        let m = Ten;
        // No p1 → no box.
        assert!(MenuBox::<u8>::new((0, 0), MenuParams::default()).is_none());
        let mut b = MenuBox::new((100, 100), params(true)).unwrap();
        assert_eq!((b.selected, b.style), (-1, 2));
        // A non-selectable first item leaves nothing selected; the first
        // selectable one added becomes selected, a later one does not.
        b.add_item(&w("cap"), 21, 0, 4, 1, None, false, &m).unwrap();
        assert_eq!(b.selected, -1);
        b.add_item(&w("a"), 15, 0, 0, 1, Some(7), true, &m).unwrap();
        b.add_item(&w("b"), 15, 0, 0, 1, Some(8), true, &m).unwrap();
        assert_eq!((b.selected, b.selectable), (1, 2));
        // Text is cut to 119 units.
        b.add_item(&vec![65u16; 200], 15, 0, 0, 1, None, false, &m)
            .unwrap();
        assert_eq!(b.items[3].text.len(), 119);
        // At most 10 items.
        for _ in 4..10 {
            b.add_item(&w("x"), 15, 0, 0, 1, None, false, &m).unwrap();
        }
        assert_eq!(
            b.add_item(&w("x"), 15, 0, 0, 1, None, false, &m),
            Err(MenuError::TooManyItems)
        );
        // Style 1 with color 3 is fatal.
        let mut b = MenuBox::new((0, 0), params(true)).unwrap();
        b.set_style(1);
        assert_eq!(
            b.add_item(&w("x"), 15, 0, 3, 1, None, false, &m),
            Err(MenuError::Color3Style1)
        );
        // p5 = 0: only the x offset against the fixed width p6.
        let mut b = MenuBox::new(
            (5, 6),
            MenuParams {
                p1: Some(1u8),
                p6: 490,
                p7: 350,
                ..Default::default()
            },
        )
        .unwrap();
        b.add_item(&w("abcd"), 21, 0, 4, 1, None, false, &m)
            .unwrap();
        assert_eq!(b.items[0].x_off, (490 - 40 + 1) / 2 + 1);
        assert_eq!(b.size, (490, 350));
    }

    // Test vector "auto box, 3 items of widths 60, 90, 40".
    #[test]
    fn auto_layout() {
        let m = Ten;
        let mut b = MenuBox::new((320, 200), params(true)).unwrap();
        for (n, h) in [(6, 21), (9, 15), (4, 15)] {
            b.add_item(&vec![65u16; n], h, 0, 0, 1, None, false, &m)
                .unwrap();
        }
        b.layout(640, 480, &m).unwrap();
        assert_eq!(b.size, (110, 66));
        assert_eq!(b.pos, (265, 178));
        // Item x offsets: (w − width + 1) / 2 + 1 when narrower.
        assert_eq!(
            b.items.iter().map(|i| i.x_off).collect::<Vec<_>>(),
            vec![
                (110 - 60 + 1) / 2 + 1,
                (110 - 90 + 1) / 2 + 1,
                (110 - 40 + 1) / 2 + 1
            ]
        );
        assert_eq!(item_x_off(100, 100), 0);
        // p4: y rises 21 per non-selectable item, not h / 3.
        let mut p = params(true);
        p.p4 = true;
        let mut b = MenuBox::new((320, 200), p).unwrap();
        for (n, h, sel) in [(6, 21, false), (9, 15, true), (4, 15, true)] {
            b.add_item(&vec![65u16; n], h, 0, 0, 1, None, sel, &m)
                .unwrap();
        }
        b.layout(640, 480, &m).unwrap();
        assert_eq!(b.pos, (265, 179));
        // Clamping: right, bottom, then the 10 minimum.
        let mut b = MenuBox::new((630, 470), params(true)).unwrap();
        b.add_item(&[65u16; 9], 21, 0, 0, 1, None, false, &m)
            .unwrap();
        b.layout(640, 480, &m).unwrap();
        // w 110, h 36: x = 575 → 575 + 110 > 630 → 530; y = 470 − 12 = 458,
        // 458 + 36 > 422 → 480 − 36 − 48 = 396.
        assert_eq!((b.pos, b.size), ((530, 396), (110, 36)));
        let mut b = MenuBox::new((0, 0), params(true)).unwrap();
        b.add_item(&w("x"), 21, 0, 0, 1, None, false, &m).unwrap();
        b.layout(640, 480, &m).unwrap();
        assert_eq!(b.pos, (10, 10));
        // Fatal sizes.
        let mut b = MenuBox::new((0, 0), params(true)).unwrap();
        b.add_item(&[65u16; 56], 21, 0, 0, 1, None, false, &m)
            .unwrap();
        assert_eq!(b.layout(640, 480, &m), Err(MenuError::TooWide(580)));
        let mut b = MenuBox::new((0, 0), params(true)).unwrap();
        for _ in 0..8 {
            b.add_item(&w("x"), 48, 0, 0, 1, None, false, &m).unwrap();
        }
        assert!(matches!(b.layout(640, 480, &m), Err(MenuError::TooTall(_))));
    }

    #[test]
    fn draw_style_color_and_pentspin() {
        let m = Ten;
        let mut b = MenuBox::new((320, 200), params(true)).unwrap();
        b.set_style(1);
        b.add_item(&w("name"), 21, 0, 4, 1, None, false, &m)
            .unwrap();
        b.add_item(&w("talk"), 15, 0, 0, 1, Some(1), true, &m)
            .unwrap();
        b.layout(640, 480, &m).unwrap();
        let mut c = 0;
        let d = b.draw(&mut c, &m);
        // Style 1: the framed box, then the selected item in color 3.
        assert!(matches!(d[0], MenuDraw::Frame(_)));
        // The border follows the framed box.
        let (x, y) = b.pos;
        assert_eq!(
            d[1],
            MenuDraw::Border {
                l: x,
                t: y,
                r: x + b.size.0 - 1,
                b: y + b.size.1 - 1
            }
        );
        assert!(matches!(&d[2], MenuDraw::Text { y: ty, color: 4, .. } if *ty == y + 21));
        assert!(matches!(&d[3], MenuDraw::Text { y: ty, color: 3, .. } if *ty == y + 36));
        assert_eq!(c, 0);
        // Pen x = x + the item's offset.
        match &d[3] {
            MenuDraw::Text { x: tx, .. } => assert_eq!(*tx, x + b.items[1].x_off),
            _ => unreachable!(),
        }
        // Style 2: pentspin frame c % 7 on both sides, then c += 1; frame 7
        // is never drawn.
        b.set_style(2);
        b.has_cel = true;
        let mut c = 7;
        let d = b.draw(&mut c, &m);
        assert!(matches!(
            d[0],
            MenuDraw::Cel {
                frame: 0,
                mode: 5,
                ..
            }
        ));
        let pen_x = x + b.items[1].x_off;
        assert_eq!(
            d[2],
            MenuDraw::Pentspin {
                frame: 0,
                x: pen_x - 24,
                y: y + 36 + 4
            }
        );
        assert_eq!(
            d[3],
            MenuDraw::Pentspin {
                frame: 0,
                x: pen_x + 40 + 2,
                y: y + 36 + 4
            }
        );
        assert_eq!(c, 8);
        let mut c = 6;
        b.draw(&mut c, &m);
        let d = b.draw(&mut c, &m);
        assert!(matches!(d[2], MenuDraw::Pentspin { frame: 0, .. }));
        assert!(matches!(d[0], MenuDraw::Cel { y: yy, .. } if yy == y + b.size.1));
    }

    #[test]
    fn anchor_from_the_npc_point() {
        assert_eq!(anchor(None, (0, 0)), None);
        assert_eq!(anchor(Some((500, 400)), (100, 50)), Some((400, 200)));
        // Raised to 20 when below 20.
        assert_eq!(anchor(Some((500, 100)), (100, 50)), Some((400, 20)));
        assert_eq!(anchor(Some((0, 0)), (0, 0)), Some((0, 20)));
    }

    #[test]
    fn waiting_note() {
        let m = Fixed;
        let n = WaitingNote::open(None, (150, 130), &w("Waiting..."), 1000, &m).unwrap();
        assert_eq!(n.deadline, 61_000);
        assert_eq!((n.bx.style, n.bx.params.p5, n.bx.params.p9), (0, true, 1));
        assert_eq!(n.bx.items.len(), 1);
        let it = &n.bx.items[0];
        assert_eq!(
            (it.height, it.font, it.color, it.selectable),
            (15, 1, 0, false)
        );
        assert_eq!(n.bx.anchor, (150, 130));
        assert_eq!(
            WaitingNote::open(Some(&n), (0, 0), &w("x"), 0, &m).unwrap_err(),
            NoteError::Second
        );
        assert_eq!(STR_WAITING, 3353);
    }
}
