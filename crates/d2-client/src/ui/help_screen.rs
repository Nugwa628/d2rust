//! The help screen (ui state 0x21, draw) at 800 × 600: the dim box, the border,
//! the close button, the bullets and the texts. The 640 × 480 layout is not
//! built.

use crate::ui::draw::CelCall;
use crate::ui::draw::{
    ImageRef, ImageRequest, LineRequest, RectRequest, TextRequest, TextStyle, UiDraw,
};
use crate::ui::geom::Point;
use crate::ui::original::FontMeasure;
use crate::ui::text::TextOpts;
use crate::ui::CelLook;

/// The help screen's ui state.
pub const STATE: u8 = 0x21;

/// The font of the two titles (the current font, Font16) and of the rest
/// (`D2Win_SetUnicodeTextFont(5)`, FontFormal12).
pub const TITLE_FONT: u16 = 1;
pub const TEXT_FONT: u16 = 5;

/// The close button's cel point at 800 × 600: the bottom left of its
/// 32 × 32 `panel\buysellbtn` cel (frame 10, 11 while pressed).
pub const CLOSE_AT: (i32, i32) = (0x2AD, 0x38);

/// The close button's 32 × 32 cel at [`CLOSE_AT`] (the point is its bottom
/// left): the hit box the HUD lists as `help.close`.
pub fn close_rect() -> crate::ui::Rect {
    crate::ui::Rect::new(CLOSE_AT.0, CLOSE_AT.1 - 32, 32, 32)
}

/// The 800 × 600 border frames of `800helpborder`.
const BORDER: [(u32, i32, i32); 7] = [
    (0, 0, 0x100),
    (1, 0, 0x200),
    (2, 0x42, 0x14),
    (3, 0x142, 0x14),
    (4, 0x217, 0x100),
    (5, 0x30C, 0x100),
    (6, 0x30C, 0x200),
];

/// The white bullets and their two-pixel-wide lines:
/// bullet (x, y), line x0 (the second line is x0 + 1), line y0, y1.
const WHITE: [((i32, i32), i32, i32, i32); 10] = [
    ((0x37, 0x217), 0x3A, 0x1E6, 0x214),
    ((0x82, 0x234), 0x85, 0x1BD, 0x231),
    ((0xD9, 0x23F), 0xDC, 0x177, 0x23C),
    ((0x104, 0x249), 0x107, 0x20E, 0x246),
    ((0x136, 0x249), 0x139, 0x1E6, 0x246),
    ((0x16D, 0x235), 0x170, 0x20D, 0x232),
    ((0x1BD, 0x21C), 0x1C0, 0x1C3, 0x219),
    ((0x212, 0x239), 0x215, 0x20D, 0x236),
    ((0x23C, 0x23F), 0x23F, 0x177, 0x23C),
    ((0x29E, 0x234), 0x2A1, 0x1BD, 0x231),
];
const WHITE_TAIL: ((i32, i32), i32, i32, i32) = ((0x2E4, 0x217), 0x2E7, 0x1E6, 0x214);

/// A text of the help screen after the two titles: the string, the column
/// (left edge x for the key lines, centre x otherwise) and y.
enum Item {
    /// String id with the key name of command `cmd` (slot 1) at x 117.
    Key(u16, i32, i32),
    /// String id at x 117.
    Left(u16, i32),
    /// String id centred on x.
    Centre(u16, i32, i32),
}

const ITEMS: [Item; 28] = [
    Item::Key(0x1067, 0x22, 0x4E),
    Item::Key(0x1068, 0x25, 0x62),
    Item::Key(0x1069, 0x24, 0x76),
    Item::Key(0x106A, 7, 0x8A),
    Item::Left(0x106B, 0x9E),
    Item::Left(0x106C, 0xB2),
    Item::Left(0x106D, 0xC6),
    Item::Key(0x106E, 6, 0xDA),
    Item::Centre(0x106F, 0x3B, 0x1E1),
    Item::Centre(0x1070, 0x86, 0x19A),
    Item::Centre(0x1071, 0x86, 0x1A9),
    Item::Centre(0x1072, 0x86, 0x1B8),
    Item::Centre(0xF92, 0xDE, 0x177),
    Item::Centre(0x1074, 0x108, 0x1FA),
    Item::Centre(0x1075, 0x108, 0x209),
    Item::Centre(0x1076, 0x13A, 0x1E1),
    Item::Centre(0x1077, 0x171, 0x1F9),
    Item::Centre(0x1078, 0x171, 0x208),
    Item::Centre(0x1079, 0x1C1, 0x191),
    Item::Centre(0x107A, 0x1C1, 0x1A0),
    Item::Centre(0x107B, 0x1C1, 0x1AF),
    Item::Centre(0x107C, 0x1C1, 0x1BE),
    Item::Centre(0x107D, 0x216, 0x208),
    Item::Centre(0xF93, 0x23C, 0x177),
    Item::Centre(0x1073, 0x2A2, 0x19A),
    Item::Centre(0x1071, 0x2A2, 0x1A9),
    Item::Centre(0x1072, 0x2A2, 0x1B8),
    Item::Centre(0x107E, 0x2E8, 0x1E1),
];

/// Everything the draw reads.
pub struct HelpIn<'a> {
    pub w: i32,
    pub h: i32,
    /// The close button is pressed.
    pub close_pressed: bool,
    pub strings: &'a dyn Fn(u16) -> Vec<u16>,
    /// The name of command `cmd`'s slot-1 key.
    pub key: &'a dyn Fn(i32) -> Vec<u16>,
    pub fonts: Option<&'a FontMeasure>,
    pub files: &'a crate::ui::panels::UiFiles,
}

/// The dim box drawn before the control panel: the screen from x `x0`,
/// `w` wide, down to the panel's top (H − 49). At 4:3 that is `(0, W)`;
/// in a wider frame the UI box's caller passes the whole frame's width.
pub fn dim_box(x0: i32, w: i32, h: i32) -> UiDraw {
    UiDraw::Rect(RectRequest::sized(x0, 0, w, h - 0x31, 0, 1))
}

fn image(
    files: &crate::ui::panels::UiFiles,
    name: &str,
    frame: u32,
    at: (i32, i32),
) -> Option<UiDraw> {
    Some(UiDraw::Image(ImageRequest {
        image: ImageRef {
            file: files.id(name)?,
            frame,
        },
        at: Point::new(at.0, at.1),
        clip: crate::ui::geom::Rect::new(0, 0, 800, 600),
        look: CelLook::PLAIN,
        call: CelCall::Draw,
    }))
}

/// Help keeps its title and key list at the top, and its HUD callouts
/// beside the HUD. Text and pointer boxes use the same committed canvas.
struct HelpCanvas<'a> {
    out: &'a mut dyn crate::ui::draw::UiDrawSink,
    height: i32,
}

impl crate::ui::draw::UiDrawSink for HelpCanvas<'_> {
    fn push(&mut self, mut d: UiDraw) {
        let y = |v: i32| if v > 300 { v + self.height - 600 } else { v };
        let clip = crate::ui::geom::Rect::new(0, 0, 800, self.height as u16);
        match &mut d {
            UiDraw::Image(r) => {
                r.at.y = y(r.at.y);
                r.clip = clip;
            }
            UiDraw::Text(r) => {
                r.at.y = y(r.at.y);
                r.clip = clip;
            }
            UiDraw::Line(r) => {
                r.y0 = y(r.y0);
                r.y1 = y(r.y1);
            }
            UiDraw::Rect(_) => {}
        }
        self.out.push(d);
    }
}

/// At 800 × 600, after the mini panel (UI pass step 9).
pub fn draw(i: &HelpIn<'_>, out: &mut dyn crate::ui::draw::UiDrawSink) {
    let push = |out: &mut dyn crate::ui::draw::UiDrawSink, d: Option<UiDraw>| {
        if let Some(d) = d {
            out.push(d);
        }
    };
    // Repeat the native side pieces through a taller canvas; the final
    // piece remains aligned with the bottom callouts. Short canvases overlap
    // the two pieces without stretching any cel.
    for (f, x, y) in BORDER {
        let bottom = 512 + i.h - 600;
        let ys: Vec<i32> = if f == 1 || f == 6 {
            (512..bottom)
                .step_by(256)
                .chain(std::iter::once(bottom))
                .collect()
        } else {
            vec![y]
        };
        for y in ys {
            if let Some(mut d) = image(i.files, "menu\\800helpborder", f, (x, y)) {
                d.widen_clip(
                    crate::ui::geom::FRAME,
                    crate::ui::geom::Rect::new(0, 0, 800, i.h as u16),
                );
                out.push(d);
            }
        }
    }
    let mut canvas = HelpCanvas { out, height: i.h };
    let out: &mut dyn crate::ui::draw::UiDrawSink = &mut canvas;
    push(
        out,
        image(
            i.files,
            "panel\\buysellbtn",
            10 + u32::from(i.close_pressed),
            CLOSE_AT,
        ),
    );
    for k in 0..8 {
        push(
            out,
            image(i.files, "menu\\helpyellowbullet", 0, (0x68, 0x49 + 20 * k)),
        );
    }
    for (b, x, y0, y1) in WHITE.iter().chain(std::iter::once(&WHITE_TAIL)).copied() {
        push(out, image(i.files, "menu\\helpwhitebullet", 0, b));
        for dx in 0..2 {
            out.push(UiDraw::Line(LineRequest {
                x0: x + dx,
                y0,
                x1: x + dx,
                y1,
                color: 0xFF,
            }));
        }
    }
    let Some(fonts) = i.fonts else {
        return;
    };
    let text =
        |out: &mut dyn crate::ui::draw::UiDrawSink, font: u16, t: Vec<u16>, x: i32, y: i32| {
            out.push(UiDraw::Text(TextRequest {
                text: t,
                at: Point::new(x, y),
                style: TextStyle { font, color: 0 },
                opts: TextOpts::default(),
                clip: crate::ui::geom::Rect::new(0, 0, 800, 600),
            }));
        };
    // The two titles, centred on x 380 / 701, in the current font.
    for (id, cx, y) in [(0x1066u16, 0x17C, 0x11), (0x1030, 0x2BD, 0x49)] {
        let t = (i.strings)(id);
        let Some(wd) = fonts.width_a(TITLE_FONT, &t) else {
            continue;
        };
        text(out, TITLE_FONT, t, cx - wd / 2, y);
    }
    for item in &ITEMS {
        match *item {
            Item::Key(id, cmd, y) => {
                let t = fmt_s(&(i.strings)(id), &(i.key)(cmd));
                text(out, TEXT_FONT, t, 0x75, y);
            }
            Item::Left(id, y) => text(out, TEXT_FONT, (i.strings)(id), 0x75, y),
            Item::Centre(id, cx, y) => {
                let t = (i.strings)(id);
                let Some(wd) = fonts.width_a(TEXT_FONT, &t) else {
                    continue;
                };
                text(out, TEXT_FONT, t, cx - wd / 2, y);
            }
        }
    }
}

/// `%s` replaced by `arg`.
fn fmt_s(fmt: &[u16], arg: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < fmt.len() {
        if fmt[k] == u16::from(b'%') && fmt.get(k + 1) == Some(&u16::from(b's')) {
            out.extend_from_slice(arg);
            k += 2;
        } else {
            out.push(fmt[k]);
            k += 1;
        }
    }
    out
}

#[cfg(test)]
mod adaptive_tests {
    use super::*;
    use crate::rules::{adaptive_integer::FrameSelection, camera::FrameSize};
    use crate::ui::ScaleMode;
    #[test]
    fn help_callouts_follow_the_hud_and_close_stays_visible_in_the_matrix() {
        let mut files = crate::ui::panels::UiFiles::new(&[]);
        files.extend(
            [
                "menu\\800helpborder",
                "panel\\buysellbtn",
                "menu\\helpyellowbullet",
                "menu\\helpwhitebullet",
            ]
            .map(str::to_owned),
        );
        let render = |h| {
            let mut draws = Vec::new();
            draw(
                &HelpIn {
                    w: 800,
                    h,
                    close_pressed: false,
                    strings: &|_| vec![],
                    key: &|_| vec![],
                    fonts: None,
                    files: &files,
                },
                &mut draws,
            );
            draws
        };
        let baseline = render(600);
        for (w, h) in [
            (1920, 1080),
            (3840, 2160),
            (2560, 1440),
            (1920, 1200),
            (2560, 1080),
            (3440, 1440),
            (5120, 2160),
            (1365, 767),
            (1919, 1079),
            (800, 1600),
            (7680, 2160),
            (801, 541),
        ] {
            let frame = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer).frame;
            let draws = render(frame.height);
            let points = |draws: &[UiDraw], file| {
                draws
                    .iter()
                    .filter_map(|d| match d {
                        UiDraw::Image(r) if r.image.file == file => Some(r.at),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                points(&draws, files.id("panel\\buysellbtn").unwrap()),
                vec![Point::new(CLOSE_AT.0, CLOSE_AT.1)]
            );
            let white = files.id("menu\\helpwhitebullet").unwrap();
            for (old, new) in points(&baseline, white).iter().zip(points(&draws, white)) {
                assert_eq!(new, Point::new(old.x, old.y + frame.height - 600));
                assert!((0..frame.height).contains(&new.y));
            }
            for d in &draws {
                if let UiDraw::Image(r) = d {
                    assert_eq!(i32::from(r.clip.h), frame.height);
                }
                if let UiDraw::Line(r) = d {
                    assert!(r.y0 >= 0 && r.y1 < frame.height);
                }
            }
        }
    }
}
