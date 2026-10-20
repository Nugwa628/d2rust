//! The scroll panel (ui 0x10), the symbol animation of a deciphered scroll
//! and the recipe scroll (ui 0x25), as draw decisions.

use crate::ui::geom::Point;
use crate::ui::layout::Screen;

/// What a scroll item draws by its item code and the flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDraw {
    Nothing,
    /// The base only.
    Base,
    /// The base and the symbols.
    BaseAndSymbols,
}

/// `bks` → the base; `bkd` → the base and the
/// symbols while the flag is 0, else the base; `tr1` → the base; any other
/// code → nothing. Drawn only while the scroll item is set.
pub fn scroll_draw(item_set: bool, code: &[u8; 3], flag: bool) -> ScrollDraw {
    if !item_set {
        return ScrollDraw::Nothing;
    }
    match code {
        b"bks" | b"tr1" => ScrollDraw::Base,
        b"bkd" if flag => ScrollDraw::Base,
        b"bkd" => ScrollDraw::BaseAndSymbols,
        _ => ScrollDraw::Nothing,
    }
}

/// The filled rectangle of the base: (`sx`, `H + sy − 224`,
/// `W / 2`, `H − 48`), color 0, mode 5; the bottom edge has no `sy`. The
/// right edge is the left side's end (`Screen::left_end`, `W / 2` at 4:3).
pub fn base_rect(s: &Screen) -> (i32, i32, i32, i32) {
    (s.sx(), s.h + s.sy() - 224, s.left_end(), s.h - 48)
}

/// The four `UI\MENU\scroin` frames' positions.
pub fn base_frames(s: &Screen) -> [Point; 4] {
    let (sx, y) = (s.sx(), s.h + s.sy());
    [
        Point::new(sx, y - 109),
        Point::new(sx + 256, y - 109),
        Point::new(sx, y - 49),
        Point::new(sx + 256, y - 49),
    ]
}

/// The close button (frame 10, no pressed frame) position.
pub fn base_close(s: &Screen) -> Point {
    Point::new(s.sx() + 277, s.h + s.sy() - 61)
}

/// Stone start counters `S[i]`.
pub const STONE_START: [u32; 5] = [0, 12, 24, 36, 48];
/// `X1`, `Y1` and `X2`, `Y2`.
pub const XY1: [(i32, i32); 5] = [(242, 104), (255, 222), (148, 310), (47, 222), (75, 104)];
pub const XY2: [(i32, i32); 5] = [(303, 161), (322, 242), (254, 290), (190, 238), (211, 162)];

/// The symbol counter `n` and its stamp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SymbolClock {
    started: bool,
    stamp: u32,
    pub n: u32,
}

impl SymbolClock {
    /// The first call stamps now and sets n:= 0; later calls with now > stamp +
    /// 50 ms: stamp:= now, n += 1.
    pub fn tick(&mut self, now: u32) {
        if !self.started {
            self.started = true;
            self.stamp = now;
            self.n = 0;
        } else if now > self.stamp.wrapping_add(50) {
            self.stamp = now;
            self.n += 1;
        }
    }
}

/// One stone draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoneDraw {
    pub stone: usize,
    /// Draw mode of the `scroin2` frame `i`.
    pub mode: u8,
    /// Click sound 0 when `d` = 1.
    pub click: bool,
    /// `scroin2` frame `i` at (`sx + X1[u]`, `Y1[u] − sy`).
    pub pos1: (i32, i32),
    /// `scroin3` frame (`d` if ≤ 20, else 0) at (`sx + X2[u]`, `Y2[u] −
    /// sy`) in mode 3.
    pub frame3: u32,
    pub pos2: (i32, i32),
}

/// The stones drawn at counter `n` for the symbol slots `slots[i]` (u16;
/// `u` ≥ 6 → skipped, `u` = 5 fatal → `Err`).
pub fn stones(n: u32, slots: &[u16; 5], s: &Screen) -> Result<Vec<StoneDraw>, u16> {
    let mut out = Vec::new();
    for i in 0..5 {
        if STONE_START[i] >= n {
            continue;
        }
        let d = n - STONE_START[i];
        let mode = match d {
            0..=4 => 0,
            5..=9 => 1,
            10..=14 => 2,
            _ => 5,
        };
        let u = slots[i];
        if u >= 6 {
            continue;
        }
        if u == 5 {
            return Err(u);
        }
        let u = usize::from(u);
        out.push(StoneDraw {
            stone: i,
            mode,
            click: d == 1,
            pos1: (s.sx() + XY1[u].0, XY1[u].1 - s.sy()),
            frame3: if d <= 20 { d } else { 0 },
            pos2: (s.sx() + XY2[u].0, XY2[u].1 - s.sy()),
        });
    }
    Ok(out)
}

/// The recipe scroll text pen: (`sx + 80`, `130 − sy`) in font 4.
pub fn recipe_pen(s: &Screen) -> Point {
    Point::new(s.sx() + 80, 130 - s.sy())
}

/// The recipe scroll draws only in an expansion game with ui 0x25 open.
pub fn recipe_drawn(expansion: bool, ui_open: bool) -> bool {
    expansion && ui_open
}

/// The result of reading a scroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollRead {
    /// The scroll-read flag.
    pub flag: bool,
    /// C→S 0x3E [u's GUID u32, −1 when u is none].
    pub send_3e: Option<u32>,
    /// The second read flag.
    pub f1e0: bool,
}

pub fn read_scroll(code: &[u8; 4], user_guid: Option<u32>) -> ScrollRead {
    if code == b"bkd " {
        ScrollRead {
            flag: true,
            send_3e: Some(user_guid.unwrap_or(u32::MAX)),
            f1e0: false,
        }
    } else {
        ScrollRead {
            flag: false,
            send_3e: None,
            f1e0: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Screen = Screen::R800;

    #[test]
    fn which_scroll_draws_what() {
        assert_eq!(scroll_draw(false, b"bks", false), ScrollDraw::Nothing);
        assert_eq!(scroll_draw(true, b"bks", false), ScrollDraw::Base);
        assert_eq!(scroll_draw(true, b"tr1", false), ScrollDraw::Base);
        assert_eq!(scroll_draw(true, b"bkd", false), ScrollDraw::BaseAndSymbols);
        assert_eq!(scroll_draw(true, b"bkd", true), ScrollDraw::Base);
        assert_eq!(scroll_draw(true, b"xyz", false), ScrollDraw::Nothing);
        // reading: bkd sets the flag and sends 0x3E; others clear it
        assert_eq!(
            read_scroll(b"bkd ", Some(7)),
            ScrollRead {
                flag: true,
                send_3e: Some(7),
                f1e0: false
            }
        );
        assert_eq!(read_scroll(b"bkd ", None).send_3e, Some(u32::MAX));
        assert_eq!(
            read_scroll(b"bks ", Some(7)),
            ScrollRead {
                flag: false,
                send_3e: None,
                f1e0: true
            }
        );
        // consequence: after reading a bkd the flag is 1, so the
        // base only is drawn
        let r = read_scroll(b"bkd ", Some(1));
        assert_eq!(scroll_draw(true, b"bkd", r.flag), ScrollDraw::Base);
    }

    #[test]
    fn base_geometry() {
        // 800 × 600: sx 80, sy −60
        assert_eq!(base_rect(&S), (80, 600 - 60 - 224, 400, 552));
        let f = base_frames(&S);
        assert_eq!(f[0], Point::new(80, 540 - 109));
        assert_eq!(f[1], Point::new(336, 540 - 109));
        assert_eq!(f[2], Point::new(80, 540 - 49));
        assert_eq!(f[3], Point::new(336, 540 - 49));
        assert_eq!(base_close(&S), Point::new(357, 540 - 61));
        let r = base_rect(&Screen::R640);
        assert_eq!(r, (0, 480 - 224, 320, 432));
    }

    #[test]
    fn symbols() {
        let mut c = SymbolClock::default();
        c.tick(1000);
        assert_eq!(c.n, 0);
        c.tick(1050);
        assert_eq!(c.n, 0, "exactly 50 ms is not a step");
        c.tick(1051);
        assert_eq!(c.n, 1);
        c.tick(1100);
        assert_eq!(c.n, 1);
        c.tick(1102);
        assert_eq!(c.n, 2);
        // the slots are all 0: stone i draws with u = 0
        let slots = [0u16; 5];
        assert!(stones(0, &slots, &S).unwrap().is_empty());
        // n = 1: stone 0 only, d = 1: mode 0 and the click
        let v = stones(1, &slots, &S).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0],
            StoneDraw {
                stone: 0,
                mode: 0,
                click: true,
                pos1: (80 + 242, 104 + 60),
                frame3: 1,
                pos2: (80 + 303, 161 + 60),
            }
        );
        // n = 13: stone 0 d = 13 → mode 2; stone 1 d = 1 → mode 0, click
        let v = stones(13, &slots, &S).unwrap();
        assert_eq!(
            v.iter()
                .map(|d| (d.stone, d.mode, d.click))
                .collect::<Vec<_>>(),
            [(0, 2, false), (1, 0, true)]
        );
        // d: 5 → mode 1, 15 → mode 5; the second cel frame d ≤ 20 else 0
        let v = stones(25, &slots, &S).unwrap();
        assert_eq!((v[0].mode, v[0].frame3), (5, 0)); // d = 25
        assert_eq!((v[1].mode, v[1].frame3), (2, 13)); // d = 13
        assert_eq!((v[2].mode, v[2].frame3), (0, 1)); // d = 1
        let v = stones(5, &slots, &S).unwrap();
        assert_eq!(v[0].mode, 1);
        let v = stones(20, &slots, &S).unwrap();
        assert_eq!((v[0].mode, v[0].frame3), (5, 20));
        // symbol slots: ≥ 6 skipped, 5 fatal, others pick the position
        assert!(stones(1, &[6, 0, 0, 0, 0], &S).unwrap().is_empty());
        assert_eq!(stones(1, &[5, 0, 0, 0, 0], &S), Err(5));
        let v = stones(1, &[3, 0, 0, 0, 0], &S).unwrap();
        assert_eq!(v[0].pos1, (80 + 47, 222 + 60));
        assert_eq!(v[0].pos2, (80 + 190, 238 + 60));
    }

    #[test]
    fn recipe_scroll() {
        assert_eq!(recipe_pen(&S), Point::new(160, 190));
        assert_eq!(recipe_pen(&Screen::R640), Point::new(80, 130));
        assert!(recipe_drawn(true, true));
        assert!(!recipe_drawn(false, true));
        assert!(!recipe_drawn(true, false));
    }
}
