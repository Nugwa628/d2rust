//! Composite token, mode and weapon naming shared by simulation and presentation.
pub type Code = [u8; 4];
pub const HTH: Code = *b"hth ";
/// The name part of a code: its first 3 bytes, ending at the first space
/// or 0 (every 0x20 turned into 0).
pub fn part(code: &Code) -> &[u8] {
    let end = code[..3]
        .iter()
        .position(|&b| b == b' ' || b == 0)
        .unwrap_or(3);
    &code[..end]
}

/// A code from a name of up to 4 bytes, space-padded (`code(b"1hs")`).
pub fn code(name: &[u8]) -> Code {
    let mut c = *b"    ";
    for (d, s) in c.iter_mut().zip(name) {
        *d = *s;
    }
    c
}

fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The draw path's unit kinds with a composite or a file root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompositeKind {
    Player,
    Monster,
    Object,
}

impl CompositeKind {
    /// Unit types 0–2; 3, 4 draw one cel, 5 nothing.
    pub fn of_unit_type(unit_type: u32) -> Option<CompositeKind> {
        match unit_type {
            0 => Some(CompositeKind::Player),
            1 => Some(CompositeKind::Monster),
            2 => Some(CompositeKind::Object),
            _ => None,
        }
    }

    /// The `<root>` folder of the unit kind's COF paths.
    pub fn root(self) -> &'static str {
        match self {
            CompositeKind::Player => "DATA\\GLOBAL\\CHARS",
            CompositeKind::Monster => "DATA\\GLOBAL\\MONSTERS",
            CompositeKind::Object => "DATA\\GLOBAL\\OBJECTS",
        }
    }

    /// Mode DT / DD of the kind (players 0 / 17, monsters 0 / 12).
    pub fn is_death_mode(self, mode: u8) -> bool {
        match self {
            CompositeKind::Player => mode == 0 || mode == 17,
            CompositeKind::Monster => mode == 0 || mode == 12,
            CompositeKind::Object => false,
        }
    }
}

/// Player mode TH (throw).
pub const PLAYER_MODE_TH: u8 = 11;

/// The COF name parts: unit token `T`, mode token `M` (after the
/// mode override tables) and weapon class `W`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CofName {
    pub kind: CompositeKind,
    pub token: Code,
    pub mode_token: Code,
    pub weapon_class: Code,
}

impl CofName {
    /// The COF name of a player in `mode` (a TH with a weapon class other than
    /// `1hs`, `1ht`, `1js`, `1jt`, `1ss`, `1st` uses `hth`).
    pub fn player(token: Code, mode: u8, mode_token: Code, weapon_class: Code) -> CofName {
        let throwable = [b"1hs", b"1ht", b"1js", b"1jt", b"1ss", b"1st"];
        let weapon_class =
            if mode == PLAYER_MODE_TH && !throwable.iter().any(|w| part(&weapon_class) == &w[..]) {
                HTH
            } else {
                weapon_class
            };
        CofName {
            kind: CompositeKind::Player,
            token,
            mode_token,
            weapon_class,
        }
    }

    /// Short form `<T><M><W>`: the AnimData key.
    pub fn short(&self) -> String {
        let mut s = text(part(&self.token));
        s.push_str(&text(part(&self.mode_token)));
        s.push_str(&text(part(&self.weapon_class)));
        s
    }

    /// Full form `<root>\<T>\COF\<T><M><W>.COF`.
    pub fn full(&self) -> String {
        format!(
            "{}\\{}\\COF\\{}.COF",
            self.kind.root(),
            text(part(&self.token)),
            self.short()
        )
    }
}

/// The player mode override table: SQ (18) and KB (19) use the GH COF.
pub const PLAYER_MODE_OVERRIDES: [(Code, u8); 2] = [(*b"gh  ", 18), (*b"gh  ", 19)];
/// The monster mode override table: KB (13) uses the GH COF.
pub const MONSTER_MODE_OVERRIDES: [(Code, u8); 1] = [(*b"gh  ", 13)];

/// The mode override tables of the COF name by kind; objects have
/// none.
pub fn mode_overrides(kind: CompositeKind) -> &'static [(Code, u8)] {
    match kind {
        CompositeKind::Player => &PLAYER_MODE_OVERRIDES,
        CompositeKind::Monster => &MONSTER_MODE_OVERRIDES,
        CompositeKind::Object => &[],
    }
}

/// The mode override: every pair whose mode equals the unit's
/// replaces the mode token, the last match wins. `overrides` is
/// [`mode_overrides`] of the unit's kind.
pub fn mode_token(token: Code, mode: u8, overrides: &[(Code, u8)]) -> Code {
    overrides
        .iter()
        .rfind(|(_, m)| *m == mode)
        .map_or(token, |(t, _)| *t)
}

// ---------------------------------------------------------------------------
// The weapon class.

/// Monster weapon class: `hth` in DT/DD without `compositeDeath`,
/// else monstats2 BaseW; no monstats2 row: `hth`.
pub fn monster_weapon_class(mode: u8, composite_death: bool, base_w: Option<Code>) -> Code {
    if CompositeKind::Monster.is_death_mode(mode) && !composite_death {
        return HTH;
    }
    base_w.unwrap_or(HTH)
}
