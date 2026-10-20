//! The character-select paper doll: the figure's sprite object, built from
//! the save's component and colour bytes. Pure logic: file reads and pixels
//! are the host's (`app::front_host`).

use d2_formats::d2s::appearance::{AppearanceTables, ItemGfx};

/// The unit token of class' 0..=9 (the playable classes, the
/// fallback figure, and the dead-hardcore figures).
pub const CLASS_TOKENS: [&str; 10] = ["AM", "SO", "NE", "PA", "BA", "DZ", "AI", "RO", "RH", "RH"];
/// (20 entries; the front end builds modes NU 1 and TN 5).
pub const MODE_TOKENS: [&str; 20] = [
    "DT", "NU", "WL", "RN", "GH", "TN", "TW", "A1", "A2", "BL", "SC", "TH", "KK", "S1", "S2", "S3",
    "S4", "DD", "GH", "GH",
];
/// The weapon class names by class number (0 = none).
pub const WEAPON_CLASSES: [&str; 15] = [
    "", "hth", "1ht", "2ht", "1hs", "2hs", "bow", "xbw", "stf", "1js", "1jt", "1ss", "1st", "ht1",
    "ht2",
];
/// The component tokens.
pub const COMPONENT_TOKENS: [&str; 16] = [
    "HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8",
];
/// The hit class names, id 1..=13 (0: none).
const HIT_NAMES: [&[u8; 3]; 13] = [
    b"bow", b"1hs", b"1ht", b"stf", b"2hs", b"2ht", b"xbw", b"1js", b"1jt", b"1ss", b"1st", b"ht1",
    b"ht2",
];
/// Hit class id → weapon class number.
const HIT_TO_CLASS: [u8; 14] = [1, 6, 4, 2, 8, 5, 3, 7, 9, 10, 11, 12, 13, 14];

/// Class number of a hit class name (entry +4 / the 2-handed column): none → 1
/// (`hth`).
fn hit_class(name: [u8; 4]) -> u8 {
    HIT_NAMES
        .iter()
        .position(|n| name[..3] == n[..] && matches!(name[3], b' ' | 0))
        .map_or(HIT_TO_CLASS[0], |i| HIT_TO_CLASS[i + 1])
}

/// What the doll reads from the data tables.
#[derive(Clone, Debug)]
pub struct DollTables {
    app: AppearanceTables,
}

impl DollTables {
    pub fn new(app: AppearanceTables) -> Self {
        Self { app }
    }

    /// The token table's code of a component byte (0: none).
    fn token(&self, b: u8) -> [u8; 4] {
        // The component gate: 0, >= 0xFF (the table's
        // count after build) and 0xFF draw the default (`lit`).
        if b == 0 || b == 0xFF {
            return [0; 4];
        }
        self.app
            .tokens
            .entry(usize::from(b))
            .map_or([0; 4], |e| e.0)
    }

    /// The (one-handed, two-handed) class numbers of token entry `b` (entry +4
    /// / the class table).
    fn hits(&self, b: u8) -> (u8, u8) {
        let Some((code, _)) = self.app.tokens.entry(usize::from(b)) else {
            return (1, 1);
        };
        let row: Option<&ItemGfx> = self.app.items.iter().find(|r| {
            let g = if r.alternategfx != [0; 4] {
                r.alternategfx
            } else {
                r.code
            };
            g == code
        });
        row.map_or((1, 1), |r| (hit_class(r.wclass), hit_class(r.wclass2)))
    }

    /// The item type of token entry `b`.
    fn item_type(&self, b: u8) -> i32 {
        self.app.tokens.entry(usize::from(b)).map_or(0, |e| e.1)
    }

    /// Over the component bytes: the weapon class
    /// number, or 0 when no class fits (the build fails).
    pub fn weapon_class(&self, class: u8, comp: &[u8; 16]) -> u8 {
        let (r, l, sh) = (comp[5], comp[6], comp[7]);
        let both = r != 0xFF && l != 0xFF;
        let armo = |b: u8| self.app.types.is_a(self.item_type(b), 50);
        // One hand's class: the two-handed column when both hands hold items (or a
        // lone weapon has a two-handed class and the shield hand is empty). An
        // armour item or an assassin-only class on another class takes the static
        // 1.00 table's class there: taken as none (measured: DollBow, DollTwo,
        // DollCol and DollDead match pixel for pixel; the DollShd figure differs by
        // 25 px in one of 12
        // shots).
        let hand = |b: u8, right: bool| -> u8 {
            let (one, two) = self.hits(b);
            let mut c = if both { two } else { one };
            if right && !both && l == 0xFF && sh == 0xFF && two != one {
                c = two;
            }
            if ((c == 13 || c == 14) && class != 6) || armo(b) {
                c = 0;
            }
            c
        };
        let a = if r == 0xFF { 0 } else { hand(r, true) };
        let b = if l == 0xFF { 0 } else { hand(l, false) };
        combine(a, b)
    }

    /// The doll's sprite object: tokens, colours and weapon class for class'
    /// `class`, mode `mode`, from the entry's bytes (padded with 0xFF).
    /// `None`: no weapon class fits (the caller retries class 7 / mode 5).
    pub fn build(&self, class: u8, mode: u8, comp: &[u8; 16], colours: &[u8; 16]) -> Option<Doll> {
        let wclass = if class < 7 {
            match self.weapon_class(class, comp) {
                0 => return None,
                w => w,
            }
        } else {
            1
        };
        let mut tokens = [[0u8; 4]; 16];
        for (t, &b) in tokens.iter_mut().zip(comp) {
            *t = self.token(b);
        }
        // Loop: colour 0xFF stays, others minus one.
        let mut stored = [0xFFu8; 16];
        for (s, &c) in stored.iter_mut().zip(colours) {
            *s = if c == 0xFF { 0xFF } else { c.wrapping_sub(1) };
        }
        Some(Doll {
            class,
            mode,
            wclass,
            tokens,
            colours: stored,
        })
    }
}

/// The left / right hit classes → the weapon class.
fn combine(a: u8, b: u8) -> u8 {
    if a == 0 {
        return if b == 0 { 1 } else { b };
    }
    if a == b && (a == 6 || a == 7) {
        return a;
    }
    if a == 8 {
        return 8;
    }
    if b == 0 {
        return a;
    }
    let to_cae = |a: u8, b: u8| -> u8 {
        // LAB_00504cae
        if b != 5 {
            if a == 2 {
                return if b != 2 { 0 } else { 10 };
            }
            return c8f(a, b);
        }
        x(a, b)
    };
    fn x(a: u8, b: u8) -> u8 {
        if a == 2 {
            return 0xb;
        }
        c8f(a, b)
    }
    fn c8f(a: u8, b: u8) -> u8 {
        if a == 5 {
            if b != 4 && b != 5 {
                return 0;
            }
        } else {
            if a != 4 {
                if a == 0xd {
                    return if b == 0xd { 0xd } else { 0 };
                }
                if a == 0xe {
                    if b == 0xe {
                        return 0xd;
                    }
                } else if a == 1 && b == 1 {
                    return 1;
                }
                return 0;
            }
            if b != 5 {
                return 0;
            }
        }
        0xb
    }
    if a == 4 {
        if b == 4 {
            return 0xb;
        }
        if b == 2 {
            return 9;
        }
        return if b != 4 { to_cae(a, b) } else { x(a, b) };
    }
    if a != 2 {
        if a == 5 && b == 2 {
            return 0xb;
        }
        return if b != 4 { to_cae(a, b) } else { x(a, b) };
    }
    if b == 4 {
        return 0xc;
    }
    to_cae(a, b)
}

/// One draw's animation step: the 8.8 frame phase grows by
/// the COF's rate (`cof +0x18`); at frame `frames − 1` or later it wraps to
/// 0, so the last frame is never shown.
pub fn advance(phase: u32, frames: u32, step: u32) -> u32 {
    let p = phase.wrapping_add(step);
    if i64::from(p >> 8) >= i64::from(frames) - 1 {
        0
    } else {
        p
    }
}

/// One doll, ready to draw (the doll's sprite object).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doll {
    pub class: u8,
    pub mode: u8,
    pub wclass: u8,
    /// Armour token per component (`[0; 4]`: `lit`).
    pub tokens: [[u8; 4]; 16],
    /// Stored colour per component (0xFF: no map).
    pub colours: [u8; 16],
}

impl Doll {
    /// The retry figure: class' 7, mode 5, `hth`, no
    /// equipment.
    pub fn fallback() -> Doll {
        Doll {
            class: 7,
            mode: 5,
            wclass: 1,
            tokens: [[0; 4]; 16],
            colours: [0xFF; 16],
        }
    }
    /// `chars` for class' <= 6, `monsters` for the fallback `RO` and the dead figures `RH`.
    pub fn root(&self) -> &'static str {
        if self.class <= 6 {
            "chars"
        } else {
            "monsters"
        }
    }
    pub fn class_token(&self) -> &'static str {
        CLASS_TOKENS
            .get(usize::from(self.class))
            .copied()
            .unwrap_or("RO")
    }
    pub fn mode_token(&self) -> &'static str {
        MODE_TOKENS
            .get(usize::from(self.mode))
            .copied()
            .unwrap_or("TN")
    }
    pub fn weapon_class_name(&self) -> &'static str {
        WEAPON_CLASSES
            .get(usize::from(self.wclass))
            .copied()
            .unwrap_or("hth")
    }
    /// The armour class of component `c` (`lit` default).
    pub fn armor_token(&self, c: usize) -> String {
        let t = self.tokens[c];
        if t == [0; 4] {
            "lit".to_owned()
        } else {
            String::from_utf8_lossy(&t).trim_end().to_owned()
        }
    }
    /// The COF path (`data\global\chars\<T>\cof\<T><M><W>.cof`).
    pub fn cof_path(&self) -> String {
        let t = self.class_token();
        let root = self.root();
        format!(
            r"data\global\{root}\{t}\cof\{t}{}{}.cof",
            self.mode_token(),
            self.weapon_class_name()
        )
    }
    /// The DCC of component `c` with the COF layer's weapon class.
    pub fn dcc_path(&self, c: usize, layer_wclass: &str) -> String {
        let t = self.class_token();
        let root = self.root();
        let comp = COMPONENT_TOKENS[c];
        format!(
            r"data\global\{root}\{t}\{comp}\{t}{comp}{}{}{layer_wclass}.dcc",
            self.armor_token(c),
            self.mode_token()
        )
    }
}

/// The item colour map file of a stored colour: file name and map index; `None`
/// for no map.
pub fn colour_map(c: u8) -> Option<(&'static str, usize)> {
    const FILES: [&str; 9] = [
        "invgreybrown",
        "grey",
        "grey2",
        "gold",
        "brown",
        "greybrown",
        "invgrey",
        "invgrey2",
        "invgreybrown",
    ];
    let (t, i) = (usize::from(c >> 5), usize::from(c & 0x1F));
    if i >= 0x15 || t > 8 || (3..=4).contains(&t) {
        return None;
    }
    Some((FILES[t], i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_weapon_is_hand_to_hand() {
        assert_eq!(combine(0, 0), 1);
    }

    #[test]
    fn dual_one_handed_swords() {
        assert_eq!(combine(4, 4), 0xb);
        assert_eq!(combine(4, 2), 9);
        assert_eq!(combine(2, 4), 0xc);
    }

    #[test]
    fn bows_and_staffs() {
        assert_eq!(combine(6, 0), 6);
        assert_eq!(combine(6, 6), 6);
        assert_eq!(combine(8, 3), 8);
    }

    #[test]
    fn hit_names_map_to_weapon_classes() {
        assert_eq!(hit_class(*b"bow "), 6);
        assert_eq!(hit_class(*b"1hs "), 4);
        assert_eq!(hit_class([0; 4]), 1);
    }

    #[test]
    fn animation_wraps_before_the_last_frame() {
        // Sorceress TN: 16 frames, rate 0x50: frame 10 after 32..=35 draws,
        // back to 0 after 48.
        let at = |n: u32| (0..n).fold(0, |p, _| advance(p, 16, 0x50)) >> 8;
        assert_eq!(at(0), 0);
        assert_eq!(at(32), 10);
        assert_eq!(at(35), 10);
        assert_eq!(at(47), 14);
        assert_eq!(at(48), 0);
    }

    #[test]
    fn colour_files() {
        assert_eq!(colour_map(0xFF), None);
        assert_eq!(colour_map(0x21), Some(("grey", 1)));
        assert_eq!(colour_map(5), Some(("invgreybrown", 5)));
        assert_eq!(colour_map(0x60), None);
        assert_eq!(colour_map(0x15), None);
    }

    #[test]
    fn default_doll_paths() {
        let d = Doll {
            class: 1,
            mode: 5,
            wclass: 1,
            tokens: [[0; 4]; 16],
            colours: [0xFF; 16],
        };
        assert_eq!(d.cof_path(), r"data\global\chars\SO\cof\SOTNhth.cof");
        assert_eq!(
            d.dcc_path(1, "hth"),
            r"data\global\chars\SO\TR\SOTRlitTNhth.dcc"
        );
    }
}
