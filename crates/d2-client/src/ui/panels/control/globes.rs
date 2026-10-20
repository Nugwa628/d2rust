//! The life and mana globes (value smoothing, windows, text toggles,
//! numbers) and the experience and stamina bars. Values are plain
//! inputs (the player's stats ×256 and states); the draws are requests.

use crate::ui::messages::LineDraw;

/// Rows of a full globe (the window rows are counted from the cel's bottom row,
/// so the fill grows upward; 80 rows = full).
pub const GLOBE_ROWS: i32 = 80;

/// The art files of the control panel drawn here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobeFile {
    /// `Panel\Hlthmana`.
    Hlthmana,
    /// `Panel\overlap`.
    Overlap,
}

/// A globe draw request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobeDraw {
    /// The vertical window of the
    /// cel `file` frame `frame`; rows counted from the cel's bottom row.
    Window {
        file: GlobeFile,
        frame: u32,
        x: i32,
        y: i32,
        skip: i32,
        lines: i32,
        mode: u8,
    },
    /// `CelDraw` at (x, y), light 0xFF, mode 5.
    Cel {
        file: GlobeFile,
        frame: u32,
        x: i32,
        y: i32,
    },
}

/// A smoothing record: last value +0, last change C
/// +4, start value +8, start C +0x0C.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Smoother {
    pub last: i32,
    pub last_c: u32,
    pub start: i32,
    pub start_c: u32,
}

impl Smoother {
    /// The shown value: `v` = the player's
    /// stat, `m` its max, `c` the client update counter, `life` for stat
    /// 6. The ramp is binary64 (x87 at PC = 53):
    /// trunc(fl(fl((e + 1) / d) · Δ)), not the integer
    /// product.
    pub fn shown(&mut self, v: i32, m: i32, c: u32, life: bool) -> i32 {
        if m == 0 {
            return 0;
        }
        // 1. C ≤ 1: start:= last:= v.
        if c <= 1 {
            self.start = v;
            self.last = v;
        }
        // 2. Life only: v < last → start:= last:= v and both times:= C.
        if life && v < self.last {
            self.start = v;
            self.last = v;
            self.last_c = c;
            self.start_c = c;
        }
        // 3. v ≠ last.
        if v != self.last {
            if (self.last - v).abs() >= m / 8 {
                self.start = v;
                self.start_c = c;
            } else {
                self.start_c = self.last_c;
                if v == m {
                    self.start = v;
                }
                // else: start:= the old last.
                else {
                    self.start = self.last;
                }
            }
            self.last = v;
            self.last_c = c;
        }
        // 4. e = C − last change C; d = last change C − start C clamped to
        // 7…15.
        let e = i64::from(c.wrapping_sub(self.last_c) as i32);
        let d = i64::from(self.last_c.wrapping_sub(self.start_c) as i32).clamp(7, 15);
        let shown = if v > 0 && e < d && self.last != self.start {
            let delta = i64::from(self.last) - i64::from(self.start);
            i64::from(self.start) + ((e + 1) as f64 / d as f64 * delta as f64) as i64
        } else {
            i64::from(v)
        };
        // Upper bound first, then the floor: a poked negative maximum
        // (`sys-stats-clamp`) gives 0 instead of a `clamp` panic.
        shown.min(i64::from(m)).max(0) as i32
    }
}

/// The three smoothing records: life, mana, stamina.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlobeSmoothing {
    pub records: [Smoother; 3],
}

/// What the life globe reads of the player.
#[derive(Clone, Copy, Debug, Default)]
pub struct LifeIn {
    /// The shown value ([`Smoother::shown`]).
    pub shown: i32,
    pub max: i32,
    /// P is a living player (type 0, mode ≠ 0x11).
    pub living_player: bool,
    /// State 100 `healthpot`.
    pub health_potion: bool,
    /// Stat 74.
    pub stat74: i32,
    /// State 2 (poisoned).
    pub poisoned: bool,
}

/// `Hlthmana` x of the potion and life windows: 28 in video mode 6, else
/// 29.
fn life_potion_x(video6: bool) -> i32 {
    if video6 {
        28
    } else {
        29
    }
}

/// The life globe: `m` = 0 draws nothing.
pub fn life_globe(i: &LifeIn, h: i32, video6: bool) -> Vec<GlobeDraw> {
    if i.max == 0 {
        return Vec::new();
    }
    let mut f = GLOBE_ROWS * i.shown / i.max;
    if (f == 1 || f == 2) && i.living_player {
        f = 2;
    }
    let mut out = Vec::new();
    if i.health_potion {
        let s = (i.stat74 * 80 / 100).min(GLOBE_ROWS);
        if s > f {
            out.push(GlobeDraw::Window {
                file: GlobeFile::Hlthmana,
                frame: 0,
                x: life_potion_x(video6),
                y: h - 13,
                skip: f,
                lines: s - f,
                mode: 0,
            });
        }
    }
    if f != 0 {
        out.push(GlobeDraw::Window {
            file: GlobeFile::Hlthmana,
            frame: if i.poisoned { 2 } else { 0 },
            x: 29,
            y: h - 13,
            skip: 0,
            lines: f,
            mode: 5,
        });
    }
    out.push(GlobeDraw::Cel {
        file: GlobeFile::Overlap,
        frame: 0,
        x: 28,
        y: h - 5,
    });
    out
}

/// What the mana globe reads of the player.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManaIn {
    pub shown: i32,
    pub max: i32,
    /// State 106 `manapot`.
    pub mana_potion: bool,
    /// Stat 26.
    pub stat26: i32,
}

/// The mana globe: shown:= min(shown, m); f = 80 · shown / m (no minimum).
pub fn mana_globe(i: &ManaIn, w: i32, h: i32, video6: bool) -> Vec<GlobeDraw> {
    if i.max == 0 {
        return Vec::new();
    }
    let shown = i.shown.min(i.max);
    let f = GLOBE_ROWS * shown / i.max;
    let mut out = Vec::new();
    if i.mana_potion {
        let s = (i.stat26 * 80 / 100).min(GLOBE_ROWS);
        if s > f {
            out.push(GlobeDraw::Window {
                file: GlobeFile::Hlthmana,
                frame: 1,
                x: if video6 { w - 112 } else { w - 111 },
                y: h - 13,
                skip: f,
                lines: s - f,
                mode: 0,
            });
        }
    }
    if f != 0 {
        out.push(GlobeDraw::Window {
            file: GlobeFile::Hlthmana,
            frame: 1,
            x: w - 111,
            y: h - 13,
            skip: 0,
            lines: f,
            mode: 5,
        });
    }
    out.push(GlobeDraw::Cel {
        file: GlobeFile::Overlap,
        frame: 1,
        x: w - 110,
        y: h - 9,
    });
    out
}

/// The text toggles of the globes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextToggle {
    /// `Show HP Text`.
    Hp,
    /// `Show MP Text`.
    Mp,
}

/// Mouse down on a toggle rectangle: x 30…110 (HP) or W − 111…W − 31 (MP),
/// y H − 75…H − 15, all inclusive.
pub fn text_toggle(w: i32, h: i32, x: i32, y: i32) -> Option<TextToggle> {
    if !(h - 75..=h - 15).contains(&y) {
        return None;
    }
    if (30..=110).contains(&x) {
        Some(TextToggle::Hp)
    } else if (w - 111..=w - 31).contains(&x) {
        Some(TextToggle::Mp)
    } else {
        None
    }
}

/// What the numbers read.
#[derive(Clone, Copy, Debug, Default)]
pub struct NumbersIn {
    pub show_hp: bool,
    pub show_mp: bool,
    pub mouse: (i32, i32),
    pub life_shown: i32,
    pub life_max: i32,
    pub mana_shown: i32,
    pub mana_max: i32,
    pub living_player: bool,
}

/// What the stamina tip reads: shown stamina, max (×256), state 136.
#[derive(Clone, Copy, Debug, Default)]
pub struct StaminaIn {
    pub shown: i32,
    pub max: i32,
    pub shrine: bool,
}

/// `%d` conversions in order.
fn fmt_d(fmt: &[u16], args: &[i64]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut a = args.iter();
    let mut i = 0;
    while i < fmt.len() {
        let is_conv = fmt[i] == u16::from(b'%')
            && matches!(fmt.get(i + 1), Some(&c) if c == u16::from(b'd') || c == u16::from(b'u'));
        if is_conv {
            if let Some(v) = a.next() {
                out.extend(v.to_string().encode_utf16());
            }
            i += 2;
        } else {
            out.push(fmt[i]);
            i += 1;
        }
    }
    out
}

/// The numbers, current font 1: `panelhealth` (4165,
/// "Life: %d / %d") with shown >> 8 (raised to 1 when ≤ 1 and P is a living
/// player) and m >> 8, at (65 − w / 2, H − 95), color 0; `panelmana`
/// (4166) at (W − 80 − w / 2, H − 95). `width_a` measures a string.
pub fn globe_numbers(
    i: &NumbersIn,
    w: i32,
    h: i32,
    strings: &dyn Fn(u16) -> Vec<u16>,
    width_a: &dyn Fn(&[u16]) -> i32,
) -> Vec<LineDraw> {
    let mut out = Vec::new();
    let in_life = text_toggle(w, h, i.mouse.0, i.mouse.1) == Some(TextToggle::Hp);
    let in_mana = text_toggle(w, h, i.mouse.0, i.mouse.1) == Some(TextToggle::Mp);
    if i.show_hp || in_life {
        let mut shown = i.life_shown >> 8;
        if shown <= 1 && i.living_player {
            shown = 1;
        }
        let t = fmt_d(
            &strings(4165),
            &[i64::from(shown), i64::from(i.life_max >> 8)],
        );
        let x = 65 - width_a(&t) / 2;
        out.push(LineDraw {
            text: t,
            x,
            y: h - 95,
            color: 0,
        });
    }
    if i.show_mp || in_mana {
        let t = fmt_d(
            &strings(4166),
            &[i64::from(i.mana_shown >> 8), i64::from(i.mana_max >> 8)],
        );
        let x = w - 80 - width_a(&t) / 2;
        out.push(LineDraw {
            text: t,
            x,
            y: h - 95,
            color: 0,
        });
    }
    out
}

/// A tool tip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tip {
    pub text: Vec<u16>,
    pub x: i32,
    pub y: i32,
    pub color: u8,
    pub centered: bool,
}

/// What the experience bar reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExpIn {
    /// Stat 12.
    pub level: u32,
    /// Stat 13.
    pub exp: u32,
    /// The experience value of the class at the base level and at L − 1.
    pub next: u32,
    pub prev: u32,
    /// The class max level.
    pub max_level: u32,
}

/// A line of the bar: `DrawLine(x0, y, x1, y, color)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarLine {
    pub x0: i32,
    pub x1: i32,
    pub y: i32,
    pub color: u8,
}

/// The experience bar's pixels: `span` ≠ 0; when cur ≥ 36,092,162
/// both cur and span are shifted right by 7; px = 119 · cur / span
/// (unsigned), px > 119 → 0.
pub fn exp_pixels(i: &ExpIn) -> u32 {
    let mut span = i.next.wrapping_sub(i.prev);
    let mut cur = i.exp.saturating_sub(i.prev);
    if span == 0 {
        return 0;
    }
    if cur >= 36_092_162 {
        cur >>= 7;
        span >>= 7;
        if span == 0 {
            return 0;
        }
    }
    let px = 119u32.wrapping_mul(cur) / span;
    if px > 119 {
        0
    } else {
        px
    }
}

/// The two lines of the bar: only while L < the class max level
/// and px > 0: color 0xFF from (W/2 − 144, y) to (W/2 − 144 + px, y) for y
/// = H − 38 and H − 37.
pub fn exp_bar(i: &ExpIn, w: i32, h: i32) -> Vec<BarLine> {
    let px = exp_pixels(i);
    if i.level >= i.max_level || px == 0 {
        return Vec::new();
    }
    [h - 38, h - 37]
        .iter()
        .map(|&y| BarLine {
            x0: w / 2 - 144,
            x1: w / 2 - 144 + px as i32,
            y,
            color: 0xFF,
        })
        .collect()
}

/// The experience tool tip: hover x W/2 − 146…W/2 − 23, y H −
/// 43…H − 34 (inclusive): `panelexp` (4163, "Experience: %u / %u") with X
/// and next at (W/2 − 146, H − 51), color 0, centred.
pub fn exp_tip(
    i: &ExpIn,
    w: i32,
    h: i32,
    mouse: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Option<Tip> {
    let (x, y) = mouse;
    if !((w / 2 - 146..=w / 2 - 23).contains(&x) && (h - 43..=h - 34).contains(&y)) {
        return None;
    }
    Some(Tip {
        text: fmt_d(&strings(4163), &[i64::from(i.exp), i64::from(i.next)]),
        x: w / 2 - 146,
        y: h - 51,
        color: 0,
        centered: true,
    })
}

/// The stamina bar colors: red (255, 0, 0), gold (244, 192,
/// 76), blue (0, 0, 255).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaminaColor {
    Red,
    Gold,
    Blue,
}

impl StaminaColor {
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            StaminaColor::Red => (255, 0, 0),
            StaminaColor::Gold => (244, 192, 76),
            StaminaColor::Blue => (0, 0, 255),
        }
    }
}

/// The stamina rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaminaBar {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub color: StaminaColor,
    pub mode: u8,
}

/// `v` = shown stamina, `m` = max (×256). Color gold, scale m; if m + 5 < v or
/// P has a state of group 24 (`stambarblue`), scale:= v, color blue. w = 0 when
/// scale ≤ 0, else 102 · v / scale; w < 25 → red.
pub fn stamina_bar(v: i32, m: i32, blue_state: bool, win_w: i32, h: i32) -> StaminaBar {
    let (mut scale, mut color) = (m, StaminaColor::Gold);
    if m + 5 < v || blue_state {
        scale = v;
        color = StaminaColor::Blue;
    }
    let w = if scale <= 0 { 0 } else { 102 * v / scale };
    if w < 25 {
        color = StaminaColor::Red;
    }
    StaminaBar {
        x: win_w / 2 - 127,
        y: h - 27,
        w,
        h: 18,
        color,
        mode: 2,
    }
}

/// The stamina tool tip: hover x W/2 − 127…W/2 − 25, y H − 27…H −
/// 9: `panelstamina` (4164, "Stamina: %d / %d") with v >> 8 and m >> 8 at
/// (W/2 − 76, H − 52), centred, color 0 — or color 3 with the value shown
/// as the max when v >> 8 > m >> 8 or P has state 136 (`shrine_stamina`).
pub fn stamina_tip(
    v: i32,
    m: i32,
    shrine_state: bool,
    w: i32,
    h: i32,
    mouse: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Option<Tip> {
    let (x, y) = mouse;
    if !((w / 2 - 127..=w / 2 - 25).contains(&x) && (h - 27..=h - 9).contains(&y)) {
        return None;
    }
    let over = (v >> 8) > (m >> 8) || shrine_state;
    let shown = if over { m >> 8 } else { v >> 8 };
    Some(Tip {
        text: fmt_d(&strings(4164), &[i64::from(shown), i64::from(m >> 8)]),
        x: w / 2 - 76,
        y: h - 52,
        color: if over { 3 } else { 0 },
        centered: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(id: u16) -> Vec<u16> {
        let s = match id {
            4165 => "Life: %d / %d",
            4166 => "Mana: %d / %d",
            4163 => "Experience: %u / %u",
            4164 => "Stamina: %d / %d",
            _ => "?",
        };
        s.encode_utf16().collect()
    }

    #[test]
    fn smoothing_ramp_is_binary64() {
        let m = 25_600;
        // d = 11 (last C 11, start C 0), e + 1 = 3, Δ = ±55: 14, not 15.
        for (last, want) in [(55, 14), (-55, -14)] {
            let mut s = Smoother {
                last: last.max(0),
                last_c: 11,
                start: if last > 0 { 0 } else { 1000 },
                start_c: 0,
            };
            if last < 0 {
                s.last = 1000 + last;
            }
            let v = s.last;
            let got = s.shown(v, m, 13, false) - s.start;
            assert_eq!(got, want);
        }
        // Both agree: d = 10, e + 1 = 5, Δ = 30 -> 15.
        let mut s = Smoother {
            last: 30,
            last_c: 10,
            start: 0,
            start_c: 0,
        };
        assert_eq!(s.shown(30, m, 14, false), 15);
    }

    #[test]
    fn shown_value_smoothing() {
        let m = 25_600;
        // C ≤ 1: start:= last:= v. No change for 20 updates: shown = v.
        let mut s = Smoother::default();
        assert_eq!(s.shown(12_800, m, 1, true), 12_800);
        for c in 2..=21 {
            assert_eq!(s.shown(12_800, m, c, true), 12_800);
        }
        // A rise below m / 8 (3,200): start:= the old last, start C:= the last
        // change C, last:= v; the shown value climbs by (e + 1) · Δ / d (d
        // clamped to 7…15).
        let mut s = Smoother::default();
        s.shown(5000, m, 1, false);
        assert_eq!(s.shown(5500, m, 2, false), 5000 + 500 / 7);
        assert_eq!((s.last, s.last_c, s.start, s.start_c), (5500, 2, 5000, 0));
        assert_eq!(s.shown(5500, m, 3, false), 5000 + 2 * 500 / 7);
        assert_eq!(s.shown(5500, m, 8, false), 5000 + 7 * 500 / 7);
        // e = d: shown = v.
        assert_eq!(s.shown(5500, m, 9, false), 5500);
        // A jump of m / 8 or more: shown at once (start:= v).
        assert_eq!(s.shown(10_000, m, 10, false), 10_000);
        assert_eq!(
            (s.start, s.start_c, s.last, s.last_c),
            (10_000, 10, 10_000, 10)
        );
        // A small rise to v = m: start:= v too.
        let mut s = Smoother::default();
        s.shown(m - 100, m, 1, false);
        assert_eq!(s.shown(m, m, 2, false), m);
        assert_eq!(s.start, m);
        // Life only: a drop shows at once and sets both times.
        let mut s = Smoother::default();
        s.shown(5000, m, 1, true);
        assert_eq!(s.shown(4900, m, 5, true), 4900);
        assert_eq!((s.start, s.last, s.last_c, s.start_c), (4900, 4900, 5, 5));
        // Mana (not life) smooths a small drop: start:= the old last.
        let mut s = Smoother::default();
        s.shown(5000, m, 1, false);
        assert_eq!(s.shown(4900, m, 5, false), 5000 - 100 / 7);
        // d clamps up to 15: last change C − start C = 20.
        let mut s = Smoother {
            last: 1000,
            last_c: 30,
            start: 900,
            start_c: 10,
        };
        assert_eq!(s.shown(1000, m, 31, false), 900 + 2 * 100 / 15);
        // The result is clamped to 0…m; m = 0 → 0.
        assert_eq!(Smoother::default().shown(100, 0, 5, false), 0);
        let mut s = Smoother::default();
        assert_eq!(s.shown(m + 500, m, 1, false), m);
        // v = 0 shows 0 at once.
        let mut s = Smoother::default();
        s.shown(500, m, 1, false);
        assert_eq!(s.shown(0, m, 2, false), 0);
    }

    // Globe test vectors.
    #[test]
    fn life_globe_rows() {
        let h = 600;
        let life = |shown: i32, max: i32| LifeIn {
            shown,
            max,
            living_player: true,
            ..Default::default()
        };
        let fill = |d: &[GlobeDraw]| -> Vec<(u32, i32, i32, i32)> {
            d.iter()
                .filter_map(|d| match d {
                    GlobeDraw::Window {
                        frame,
                        x,
                        skip,
                        lines,
                        mode: 5,
                        ..
                    } => Some((*frame, *x, *skip, *lines)),
                    _ => None,
                })
                .collect()
        };
        // life 50 · 256 of max 100 · 256: f = 40 rows at x 29, H − 13.
        let d = life_globe(&life(12_800, 25_600), h, false);
        assert_eq!(fill(&d), vec![(0, 29, 0, 40)]);
        assert_eq!(
            d[0],
            GlobeDraw::Window {
                file: GlobeFile::Hlthmana,
                frame: 0,
                x: 29,
                y: 587,
                skip: 0,
                lines: 40,
                mode: 5
            }
        );
        // The cover: overlap frame 0 at (28, H − 5).
        assert_eq!(
            d.last(),
            Some(&GlobeDraw::Cel {
                file: GlobeFile::Overlap,
                frame: 0,
                x: 28,
                y: 595
            })
        );
        // life 1 (×256 = 256) of max 25,600: f = 0 → 0, no fill.
        assert!(fill(&life_globe(&life(256, 25_600), h, false)).is_empty());
        // life 512: f = 1 → 2.
        assert_eq!(
            fill(&life_globe(&life(512, 25_600), h, false)),
            vec![(0, 29, 0, 2)]
        );
        // 1 or 2 stays as is for a non-player.
        let mut l = life(512, 25_600);
        l.living_player = false;
        assert_eq!(fill(&life_globe(&l, h, false)), vec![(0, 29, 0, 1)]);
        // Full: 80 rows. Poisoned: frame 2.
        let mut l = life(25_600, 25_600);
        l.poisoned = true;
        assert_eq!(fill(&life_globe(&l, h, false)), vec![(2, 29, 0, 80)]);
        // m = 0: nothing at all.
        assert!(life_globe(&life(0, 0), h, false).is_empty());
        // Healing potion, stat 74 = 30, f = 10: the potion window skip 10,
        // lines 14 (s = 24), frame 0, mode 0, x 29 (28 in video mode 6).
        let mut l = life(3200, 25_600);
        l.health_potion = true;
        l.stat74 = 30;
        let d = life_globe(&l, h, false);
        assert_eq!(
            d[0],
            GlobeDraw::Window {
                file: GlobeFile::Hlthmana,
                frame: 0,
                x: 29,
                y: 587,
                skip: 10,
                lines: 14,
                mode: 0
            }
        );
        let d = life_globe(&l, h, true);
        assert!(matches!(d[0], GlobeDraw::Window { x: 28, .. }));
        // No potion window when s ≤ f; s is capped at 80.
        l.stat74 = 10;
        assert_eq!(life_globe(&l, h, false).len(), 2);
        l.stat74 = 500;
        assert!(matches!(
            life_globe(&l, h, false)[0],
            GlobeDraw::Window {
                skip: 10,
                lines: 70,
                ..
            }
        ));
    }

    #[test]
    fn mana_globe_rows() {
        let (w, h) = (800, 600);
        let mana = |shown: i32, max: i32| ManaIn {
            shown,
            max,
            ..Default::default()
        };
        let d = mana_globe(&mana(6400, 25_600), w, h, false);
        assert_eq!(
            d,
            vec![
                GlobeDraw::Window {
                    file: GlobeFile::Hlthmana,
                    frame: 1,
                    x: 689,
                    y: 587,
                    skip: 0,
                    lines: 20,
                    mode: 5
                },
                GlobeDraw::Cel {
                    file: GlobeFile::Overlap,
                    frame: 1,
                    x: 690,
                    y: 591
                }
            ]
        );
        // No minimum: a small value is 0 rows. shown is capped at max.
        assert_eq!(mana_globe(&mana(100, 25_600), w, h, false).len(), 1);
        assert!(matches!(
            mana_globe(&mana(99_999, 25_600), w, h, false)[0],
            GlobeDraw::Window { lines: 80, .. }
        ));
        assert!(mana_globe(&mana(1, 0), w, h, false).is_empty());
        // Mana potion (state 106): stat 26 · 80 / 100 above f → frame 1
        // window at W − 111 (W − 112 in video mode 6), skip f, lines s − f.
        let mut m = mana(6400, 25_600);
        m.mana_potion = true;
        m.stat26 = 50;
        let d = mana_globe(&m, w, h, false);
        assert_eq!(
            d[0],
            GlobeDraw::Window {
                file: GlobeFile::Hlthmana,
                frame: 1,
                x: 689,
                y: 587,
                skip: 20,
                lines: 20,
                mode: 0
            }
        );
        assert!(matches!(
            mana_globe(&m, w, h, true)[0],
            GlobeDraw::Window { x: 688, .. }
        ));
    }

    #[test]
    fn window_rows_grow_upward_from_the_bottom() {
        // 80 rows = full; a fill of f rows is the window (skip 0, lines
        // f) — rows counted from the cel's bottom row — and the potion
        // underlay starts above it (skip f).
        let l = LifeIn {
            shown: 25_600,
            max: 25_600,
            living_player: true,
            health_potion: true,
            stat74: 100,
            ..Default::default()
        };
        let d = life_globe(&l, 600, false);
        assert!(matches!(
            d[0],
            GlobeDraw::Window {
                skip: 0,
                lines: 80,
                mode: 5,
                ..
            }
        ));
        assert_eq!(GLOBE_ROWS, 80);
        let l = LifeIn {
            shown: 12_800,
            health_potion: true,
            stat74: 100,
            ..l
        };
        let d = life_globe(&l, 600, false);
        // Underlay: skip 40, lines 40 (the part above the fill); fill: skip
        // 0, lines 40.
        assert!(matches!(
            d[0],
            GlobeDraw::Window {
                skip: 40,
                lines: 40,
                mode: 0,
                ..
            }
        ));
        assert!(matches!(
            d[1],
            GlobeDraw::Window {
                skip: 0,
                lines: 40,
                mode: 5,
                ..
            }
        ));
    }

    #[test]
    fn text_toggle_rectangles() {
        let (w, h) = (800, 600);
        // HP: x 30…110; MP: W − 111…W − 31; y H − 75…H − 15 inclusive.
        for (x, y, want) in [
            (30, 525, Some(TextToggle::Hp)),
            (110, 585, Some(TextToggle::Hp)),
            (29, 550, None),
            (111, 550, None),
            (50, 524, None),
            (50, 586, None),
            (689, 550, Some(TextToggle::Mp)),
            (769, 585, Some(TextToggle::Mp)),
            (688, 550, None),
            (770, 550, None),
        ] {
            assert_eq!(text_toggle(w, h, x, y), want, "({x}, {y})");
        }
        assert_eq!(text_toggle(640, 480, 529, 410), Some(TextToggle::Mp));
    }

    #[test]
    fn globe_numbers_text() {
        let width = |t: &[u16]| 10 * t.len() as i32;
        let base = NumbersIn {
            life_shown: 50 * 256,
            life_max: 100 * 256,
            mana_shown: 20 * 256,
            mana_max: 40 * 256,
            living_player: true,
            ..Default::default()
        };
        // Neither shown without the option or the hover.
        assert!(globe_numbers(&base, 800, 600, &strings, &width).is_empty());
        // Show HP Text: "Life: 50 / 100" (14 units) at (65 − 70, H − 95).
        let mut n = base;
        n.show_hp = true;
        let d = globe_numbers(&n, 800, 600, &strings, &width);
        assert_eq!(
            d,
            vec![LineDraw {
                text: "Life: 50 / 100".encode_utf16().collect(),
                x: 65 - 70,
                y: 505,
                color: 0
            }]
        );
        // Show MP Text, or the mouse in the mana toggle rectangle.
        let mut n = base;
        n.mouse = (700, 550);
        let d = globe_numbers(&n, 800, 600, &strings, &width);
        assert_eq!(
            d,
            vec![LineDraw {
                text: "Mana: 20 / 40".encode_utf16().collect(),
                x: 720 - 65,
                y: 505,
                color: 0
            }]
        );
        // The mouse in the life rectangle shows the life.
        n.mouse = (50, 550);
        assert_eq!(globe_numbers(&n, 800, 600, &strings, &width).len(), 1);
        // shown >> 8 ≤ 1 is raised to 1 for a living player only.
        let mut n = base;
        n.show_hp = true;
        n.life_shown = 100;
        let t = |d: &[LineDraw]| String::from_utf16(&d[0].text).unwrap();
        assert_eq!(
            t(&globe_numbers(&n, 800, 600, &strings, &width)),
            "Life: 1 / 100"
        );
        n.living_player = false;
        assert_eq!(
            t(&globe_numbers(&n, 800, 600, &strings, &width)),
            "Life: 0 / 100"
        );
        // Arithmetic shifts.
        n.life_shown = -512;
        n.living_player = true;
        assert_eq!(
            t(&globe_numbers(&n, 800, 600, &strings, &width)),
            "Life: 1 / 100"
        );
    }

    // Test vector "800 × 600, X = 1,000, prev 500, next 1,500".
    #[test]
    fn experience_bar() {
        let i = ExpIn {
            level: 10,
            exp: 1000,
            next: 1500,
            prev: 500,
            max_level: 99,
        };
        assert_eq!(exp_pixels(&i), 59);
        assert_eq!(
            exp_bar(&i, 800, 600),
            vec![
                BarLine {
                    x0: 256,
                    x1: 315,
                    y: 562,
                    color: 0xFF
                },
                BarLine {
                    x0: 256,
                    x1: 315,
                    y: 563,
                    color: 0xFF
                }
            ]
        );
        // X below prev: cur 0 → no bar; at the max level no bar; span 0
        // none.
        assert!(exp_bar(&ExpIn { exp: 400, ..i }, 800, 600).is_empty());
        assert!(exp_bar(&ExpIn { level: 99, ..i }, 800, 600).is_empty());
        assert_eq!(exp_pixels(&ExpIn { next: 500, ..i }), 0);
        // X = next: px = 119; X beyond: px > 119 → 0.
        assert_eq!(exp_pixels(&ExpIn { exp: 1500, ..i }), 119);
        assert_eq!(exp_pixels(&ExpIn { exp: 1600, ..i }), 0);
        // Large values: cur ≥ 36,092,162 shifts both right by 7.
        let big = ExpIn {
            level: 80,
            exp: 40_000_000,
            next: 80_000_000,
            prev: 0,
            max_level: 99,
        };
        assert_eq!(
            exp_pixels(&big),
            119 * (40_000_000 >> 7) / (80_000_000 >> 7)
        );
        // 640 × 480: x from 176.
        assert_eq!(exp_bar(&i, 640, 480)[0].x0, 176);
        // The tool tip: x W/2 − 146…W/2 − 23, y H − 43…H − 34.
        let tip = exp_tip(&i, 800, 600, (254, 557), &strings).unwrap();
        assert_eq!(
            tip,
            Tip {
                text: "Experience: 1000 / 1500".encode_utf16().collect(),
                x: 254,
                y: 549,
                color: 0,
                centered: true
            }
        );
        assert!(exp_tip(&i, 800, 600, (377, 566), &strings).is_some());
        assert!(exp_tip(&i, 800, 600, (253, 557), &strings).is_none());
        assert!(exp_tip(&i, 800, 600, (378, 557), &strings).is_none());
        assert!(exp_tip(&i, 800, 600, (254, 556), &strings).is_none());
        assert!(exp_tip(&i, 800, 600, (254, 567), &strings).is_none());
    }

    // Test vectors "stamina v = m = 25,600" and "v = 5,000, m = 25,600".
    #[test]
    fn stamina_bar_and_tip() {
        let b = stamina_bar(25_600, 25_600, false, 800, 600);
        assert_eq!(
            b,
            StaminaBar {
                x: 273,
                y: 573,
                w: 102,
                h: 18,
                color: StaminaColor::Gold,
                mode: 2
            }
        );
        let b = stamina_bar(5000, 25_600, false, 800, 600);
        assert_eq!((b.w, b.color), (19, StaminaColor::Red));
        // 25 px is not red; 24 is.
        assert_eq!(stamina_bar(6274, 25_600, false, 800, 600).w, 24);
        assert_eq!(
            stamina_bar(6274, 25_600, false, 800, 600).color,
            StaminaColor::Red
        );
        assert_eq!(stamina_bar(6275, 25_600, false, 800, 600).w, 25);
        assert_eq!(
            stamina_bar(6275, 25_600, false, 800, 600).color,
            StaminaColor::Gold
        );
        // v above m + 5: scale:= v, blue, full width (102).
        let b = stamina_bar(30_000, 25_600, false, 800, 600);
        assert_eq!((b.w, b.color), (102, StaminaColor::Blue));
        // m + 5 = v is not above.
        assert_eq!(
            stamina_bar(25_605, 25_600, false, 800, 600).color,
            StaminaColor::Gold
        );
        // A group-24 state (`stambarblue`): scale:= v, blue.
        let b = stamina_bar(12_800, 25_600, true, 800, 600);
        assert_eq!((b.w, b.color), (102, StaminaColor::Blue));
        // scale ≤ 0: w = 0 → red.
        let b = stamina_bar(0, 0, false, 800, 600);
        assert_eq!((b.w, b.color), (0, StaminaColor::Red));
        assert_eq!(StaminaColor::Gold.rgb(), (244, 192, 76));
        // The tool tip.
        let t = stamina_tip(5000, 25_600, false, 800, 600, (273, 580), &strings).unwrap();
        assert_eq!(
            t,
            Tip {
                text: "Stamina: 19 / 100".encode_utf16().collect(),
                x: 324,
                y: 548,
                color: 0,
                centered: true
            }
        );
        // v >> 8 > m >> 8 or state 136: color 3, the value shown as the max.
        let t = stamina_tip(30_000, 25_600, false, 800, 600, (273, 580), &strings).unwrap();
        assert_eq!(
            (t.color, String::from_utf16(&t.text).unwrap()),
            (3, "Stamina: 100 / 100".into())
        );
        let t = stamina_tip(5000, 25_600, true, 800, 600, (273, 580), &strings).unwrap();
        assert_eq!(
            (t.color, String::from_utf16(&t.text).unwrap()),
            (3, "Stamina: 100 / 100".into())
        );
        // Hover x W/2 − 127…W/2 − 25, y H − 27…H − 9.
        assert!(stamina_tip(1, 2, false, 800, 600, (375, 591), &strings).is_some());
        assert!(stamina_tip(1, 2, false, 800, 600, (376, 580), &strings).is_none());
        assert!(stamina_tip(1, 2, false, 800, 600, (272, 580), &strings).is_none());
        assert!(stamina_tip(1, 2, false, 800, 600, (300, 592), &strings).is_none());
        assert!(stamina_tip(1, 2, false, 800, 600, (300, 572), &strings).is_none());
    }
}
