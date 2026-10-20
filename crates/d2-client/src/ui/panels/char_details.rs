//! Character panel details: the stat-point block when
//! there are no points, the class and name lines, the damage and attack
//! rating block, the chance-to-hit formulas, the hovered monster classes,
//! the two popups and the draw order; plus the experience / next-level
//! text. Pure decisions over inputs the bridge
//! snapshot supplies; the panel (`character.rs`) draws what they return.

use crate::ui::geom::Point;
use crate::ui::layout::Screen;

// ---- r1: no stat points ---------------------------------------------

/// With the base `statpts` (stat 4) = 0 the points box, its two labels,
/// the number and the four add buttons are not drawn.
pub fn points_block_drawn(base_statpts: i32) -> bool {
    base_statpts != 0
}

/// Mouse down with no points: inside the panel area but
/// outside the close rectangle it is consumed without a press or sound.
pub fn down_without_points(in_area: bool, in_close: bool) -> bool {
    in_area && !in_close
}

// ---- r3, r4: class and name lines -------------------------------------

/// The y of the class and name lines: `H + sy − 455`.
pub fn line_y(s: &Screen) -> i32 {
    s.h + s.sy() - 455
}

/// The class line centering span [`sx + 193`, `sx + 310`].
pub fn class_span(s: &Screen) -> (i32, i32) {
    (s.sx() + 193, s.sx() + 310)
}

/// The name line span [`sx + 13`, `sx + 160`].
pub fn name_span(s: &Screen) -> (i32, i32) {
    (s.sx() + 13, s.sx() + 160)
}

/// The number of UTF-8 code points of the name (invalid bytes are not counted).
pub fn name_code_points(name: &[u8]) -> usize {
    name.utf8_chunks().map(|c| c.valid().chars().count()).sum()
}

/// The name font: Font16 (1) for n ≤ 10, Font8 (0) for 11 or 12,
/// Font6 (6) for n ≥ 13.
pub fn name_font(n: usize) -> u16 {
    match n {
        0..=10 => 1,
        11 | 12 => 0,
        _ => 6,
    }
}

// ---- r5: damage block ---------------------------------------------------

/// Entry table (x1, y, x2), e0–e11.
pub const ENTRIES: [(i32, i32, i32); 12] = [
    (162, 93, 258),
    (162, 101, 258),
    (263, 98, 307),
    (162, 160, 270),
    (162, 163, 270),
    (270, 160, 310),
    (162, 117, 258),
    (162, 125, 258),
    (263, 122, 307),
    (162, 184, 270),
    (162, 187, 270),
    (270, 184, 310),
];

/// A text of the block: centered in [x1, x2] (screen coordinates) at y.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockText {
    pub text: String,
    pub font: u16,
    pub color: u8,
    pub x1: i32,
    pub x2: i32,
    pub y: i32,
}

/// Upper-case for units < 0x100 (table; spec): a–z and 0xE0–0xFE except 0xF7
/// lose 0x20; 0x9A → 0x8A, 0x9C → 0x8C, 0xFF → 0x9F (the Windows-1252 capitals
/// of š, œ, ÿ); every other unit is kept.
pub fn upper_latin1(u: u16) -> u16 {
    match u {
        0x61..=0x7A => u - 0x20,
        0xE0..=0xFE if u != 0xF7 => u - 0x20,
        0x9A => 0x8A,
        0x9C => 0x8C,
        0xFF => 0x9F,
        _ => u,
    }
}

/// The skill name as drawn: upper-cased units < 0x100.
pub fn upper_name(name: &str) -> String {
    let u: Vec<u16> = name
        .encode_utf16()
        .map(|c| if c < 0x100 { upper_latin1(c) } else { c })
        .collect();
    String::from_utf16_lossy(&u)
}

/// What the `descdam` / `descatt` functions give for a hand.
#[derive(Clone, Debug, Default)]
pub struct HandBlock {
    /// Skill name (skilldesc `str name`).
    pub name: String,
    /// `d`: state in {0, 1, 6, 8}, `descdam` < 0x90 with a non-null entry.
    pub damage: bool,
    /// `a`: the same state test, `descatt` < 0x48 with a non-null entry.
    pub attack: bool,
    /// `descatt` outputs (v1, c1, v2, c2).
    pub v1: i32,
    pub c1: u8,
    pub v2: i32,
    pub c2: u8,
    /// The `strchratr` text (4063; 4065 without the skills record).
    pub attack_label: String,
    /// "Damage" (`strchrskm`, 4061).
    pub damage_label: String,
}

/// Where the value of the `descdam` function is drawn (owner: OQ 1): the
/// entry e`p + 2` span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockItem {
    Text(BlockText),
    DamageValue { x1: i32, x2: i32, y: i32 },
}

/// The block of one hand (`p` = 0 left, 6
/// right): in order, Font6. `lang69`: languages 6–9.
pub fn hand_block(
    h: &HandBlock,
    p: usize,
    s: &Screen,
    lang69: bool,
    once: &mut bool,
) -> Vec<BlockItem> {
    let base = s.h + s.sy() - 480;
    let sx = s.sx();
    let mut e = ENTRIES;
    if lang69 {
        // once per run (flag), e0 and e6 up by 1
        if !*once {
            *once = true;
        }
        e[0].1 -= 1;
        e[6].1 -= 1;
    }
    let ent = |i: usize| (sx + e[i].0, sx + e[i].2, base + e[i].1);
    let t = |text: &str, font: u16, color: u8, x1: i32, x2: i32, y: i32| {
        BlockItem::Text(BlockText {
            text: text.to_string(),
            font,
            color,
            x1,
            x2,
            y,
        })
    };
    let mut out = Vec::new();
    let (x1, x2, y) = ent(p);
    out.push(t(&upper_name(&h.name), 6, 0, x1, x2, y));
    if h.damage {
        let (x1, x2, y) = ent(p + 1);
        out.push(t(&h.damage_label, 6, 0, x1, x2, y));
        let (x1, x2, y) = ent(p + 2);
        out.push(BlockItem::DamageValue { x1, x2, y });
    }
    if h.attack && (h.v1 != 0 || h.v2 != 0) {
        let (x1, x2, y) = ent(p + 3);
        // the label: with a LF before its end the two parts at y − 4 and
        // y + 4, else the name alone at y
        let label = h.attack_label.as_str();
        match label.find('\n').filter(|&i| i + 1 < label.len()) {
            Some(i) => {
                out.push(t(&label[..i], 6, 0, x1, x2, y - 4));
                out.push(t(&label[i + 1..], 6, 0, x1, x2, y + 4));
            }
            None => out.push(t(label, 6, 0, x1, x2, y)),
        }
        let (x1, x2, y) = ent(p + 5);
        if h.v2 != 0 {
            let up = if lang69 { -7 } else { -6 };
            out.push(t(&h.v1.to_string(), 6, h.c1, x1, x2, y + up));
            out.push(t(&h.v2.to_string(), 6, h.c2, x1, x2, y + 2));
        } else {
            let font = if h.v1 < 1000 { 1 } else { 0 };
            out.push(t(&h.v1.to_string(), font, h.c1, x1, x2, y));
        }
    }
    out
}

// ---- r6, r7: chances ----------------------------------------------------

/// The monster row the formulas read.
#[derive(Clone, Copy, Debug)]
pub struct MonsterFacts {
    /// `Lm`: its `Level` for the difficulty.
    pub level: i32,
    /// `D` for the to-hit: its AC.
    pub ac: i32,
    /// `Align` ≠ 1.
    pub align_not_1: bool,
}

fn pct(mut a: i32, mut d: i32) -> i32 {
    if d < 0 {
        a -= d;
        d = 0;
    }
    if a < 0 {
        d -= a;
        a = 0;
    }
    if a == 0 && d == 0 {
        100
    } else {
        100 * a / (a + d)
    }
}

/// Chance to hit: `D` from the monster AC, ×
/// 10 / 12 in a classic game when the difficulty ≠ 0 and `Align` ≠ 1; `A`
/// = AR + the class's ToHitFactor; negative values move across; pct =
/// 100 when both are 0, else 100 A / (A + D); chance = 2 × pct × Lp / (Lm +
/// Lp), clamped to [5, 95] (C division).
pub fn chance_to_hit(
    ar: i32,
    m: &MonsterFacts,
    classic: bool,
    difficulty: u8,
    to_hit_factor: i32,
    player_level: i32,
) -> i32 {
    let mut d = m.ac;
    if classic && difficulty != 0 && m.align_not_1 {
        d = d * 10 / 12;
    }
    let a = ar + to_hit_factor;
    let p = pct(a, d);
    (2 * p * player_level / (m.level + player_level)).clamp(5, 95)
}

/// Chance to be hit: no monster row → 0; `D` =
/// player defense + stat 33; `A` = the first non-zero to-hit of flags 8,
/// 0x10, 0x20; classic, difficulty ≠ 0, `Align` ≠ 1: A × 10 / 15; value =
/// 2 × pct × Lm / (Lp + Lm). The popup clamps it to [5, 95].
pub fn chance_to_be_hit(
    m: Option<&MonsterFacts>,
    defense: i32,
    stat33: i32,
    to_hits: [i32; 3],
    classic: bool,
    difficulty: u8,
    player_level: i32,
) -> i32 {
    let Some(m) = m else { return 0 };
    let d = defense + stat33;
    let mut a = to_hits.into_iter().find(|&v| v != 0).unwrap_or(0);
    if classic && difficulty != 0 && m.align_not_1 {
        a = a * 10 / 15;
    }
    let p = pct(a, d);
    2 * p * m.level / (player_level + m.level)
}

/// The popup clamp [5, 95].
pub fn popup_clamp(v: i32) -> i32 {
    v.clamp(5, 95)
}

// ---- r8: hovered monster classes ------------------------------------------

/// The unit under the mouse when it changes.
#[derive(Clone, Copy, Debug)]
pub struct HoverUnit {
    pub is_monster: bool,
    pub class: u32,
    /// `monstats` `npc` clear, `Align` ≠ 1, `monstats2` `isAtt` set.
    pub attackable: bool,
    /// `monstats2` `inert` clear.
    pub not_inert: bool,
}

/// The to-hit and to-be-hit levels; both start as
/// 19 at game start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoveredClasses {
    pub to_hit: u32,
    pub to_be_hit: u32,
}

impl Default for HoveredClasses {
    fn default() -> Self {
        Self {
            to_hit: 19,
            to_be_hit: 19,
        }
    }
}

impl HoveredClasses {
    pub fn update(&mut self, u: &HoverUnit) {
        if u.is_monster && u.attackable {
            self.to_hit = u.class;
            if u.not_inert {
                self.to_be_hit = u.class;
            }
        }
    }
}

// ---- r9: popups -----------------------------------------------------------

/// The to-hit popup hand under the mouse: x in [`sx + 162`, `sx +
/// 320`] and y in [`H + sy − 334`, `H + sy − 315`] → left (0), or [`H +
/// sy − 308`, `H + sy − 289`] → right (1).
pub fn to_hit_hand(s: &Screen, m: Point) -> Option<usize> {
    if !(s.sx() + 162..=s.sx() + 320).contains(&m.x) {
        return None;
    }
    let hy = s.h + s.sy();
    if (hy - 334..=hy - 315).contains(&m.y) {
        Some(0)
    } else if (hy - 308..=hy - 289).contains(&m.y) {
        Some(1)
    } else {
        None
    }
}

/// The to-hit popup rectangle and lines (table): box,
/// line 1, line 2, all + (`sx`, `H + sy − 480`).
pub fn to_hit_popup(s: &Screen, hand: usize) -> ((i32, i32, i32, i32), Point, Point) {
    let (bx, by, l1, l2) = if hand == 0 {
        (154, 115, (159, 128), (159, 140))
    } else {
        (154, 130, (159, 143), (159, 155))
    };
    let (ox, oy) = (s.sx(), s.h + s.sy() - 480);
    (
        (ox + bx, oy + by, ox + bx + 155, oy + by + 30),
        Point::new(ox + l1.0, oy + l1.1),
        Point::new(ox + l2.0, oy + l2.1),
    )
}

/// The rectangle's fill: color 0 (never written), mode 2.
pub const POPUP_FILL: (u8, u8) = (0, 2);
/// `charavghit` and `charmonsterX` ("%s: %d%%").
pub const STR_AVG_HIT: u16 = 4159;
pub const STR_MONSTER_X: u16 = 10103;

/// The to-be-hit popup hit: x in [`sx + 173`, `sx + 311`], y in
/// [`H + sy − 286`, `H + sy − 267`].
pub fn to_be_hit_hit(s: &Screen, m: Point) -> bool {
    (s.sx() + 173..=s.sx() + 311).contains(&m.x)
        && (s.h + s.sy() - 286..=s.h + s.sy() - 267).contains(&m.y)
}

/// The framed text position `DrawFramedText(text, sx + 139, H + sy − 315,
/// 0, mode 2, k 0)`.
pub fn to_be_hit_text_at(s: &Screen) -> Point {
    Point::new(s.sx() + 139, s.h + s.sy() - 315)
}

/// The string and its arguments: `b` ≠ 0 → 10105
/// (`charmontohit2X`) with (b, chance, name, chance); else 10104
/// (`charmontohit1X`) with (name, chance).
pub fn to_be_hit_text_id(block: i32) -> u16 {
    if block != 0 {
        10105
    } else {
        10104
    }
}

/// The block value `b`; when ≤ 0, walk the stat 348
/// (`passive_weaponblock`) entries (≤ 32): an entry with layer 0 sets `b` to
/// its value; an entry whose layer matches the item in body location 5 or 4
/// raises `b` to its value if larger; `b` is kept only when the weapon class is
/// 13 (else 0).
pub fn block_value(
    base: i32,
    entries: &[(u32, i32)],
    hand_layers: [Option<u32>; 2],
    weapon_class_13: bool,
) -> i32 {
    if base > 0 {
        return base;
    }
    let mut b = 0;
    for &(layer, v) in entries.iter().take(32) {
        // layer 0 sets; a matching hand layer raises when larger
        if layer == 0 || (hand_layers.iter().flatten().any(|&l| l == layer) && v > b) {
            b = v;
        }
    }
    if weapon_class_13 {
        b
    } else {
        0
    }
}

// ---- r10: draw order ------------------------------------------------------

/// The draw order of the panel; each covers what
/// was drawn before it.
pub const DRAW_ORDER: [&str; 12] = [
    "quads",
    "close hover (queued)",
    "points box, labels, number",
    "add buttons",
    "15 labels",
    "damage block",
    "to-hit popup",
    "to-be-hit popup",
    "close button",
    "class line",
    "name line",
    "18 values",
];

// ----: experience and next level ----------------------------------

/// The value as unsigned decimal with `,` after
/// every 3 digits from the right (only with more than 3 digits; no
/// locale); `*` when it does not fit the buffer.
pub fn group_digits(v: u32, buf: usize) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if out.len() + 1 > buf {
        "*".to_string()
    } else {
        out
    }
}

/// The next-level value: with `L` the player's base level and `c` the
/// class (outside 0–6 read as 0): if `L` ≠ MaxLvl of `c`, the
/// `experience.txt` entry of `c` on the row whose `Level` is `L`, else
/// stat 30.
pub fn next_level_value(
    base_level: u32,
    class: u32,
    max_lvl: &dyn Fn(u32) -> u32,
    entry: &dyn Fn(u32, u32) -> u32,
    stat30: u32,
) -> u32 {
    let c = if class > 6 { 0 } else { class };
    if base_level != max_lvl(c) {
        entry(c, base_level)
    } else {
        stat30
    }
}

#[cfg(test)]
mod tests;
