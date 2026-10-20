//! The 32 appearance bytes of a character save (header +0x88..+0xA7, see
//! [`super::Header`]): 16 component bytes, one graphics token per body
//! part, then 16 colour bytes. They let the character select screen and
//! the paper doll draw a hero in their gear without loading the items.
//!
//! Two pieces live here:
//! - the **token table**, which numbers every drawable item graphic (the
//!   three `armtype` weights, then weapons, torsos, shields and helms in
//!   items record order), and its lookup;
//! - the **fill**, which walks the equipped items and writes the token and
//!   colour of each part they cover: helm and hand items take their own
//!   part (or the right / left hand part when they are the hand owners),
//!   body armour writes its six armour parts.
//!
//! Everything is a pure function over plain inputs. The caller (d2-server
//! world data) projects the item table columns ([`ItemGfx`], one per
//! weapons / armor / misc row in record order), the `armtype` tokens, the
//! colour columns of the affix, set, unique, gem and state tables
//! ([`ColourTables`]) and the itemtypes is-a matrix ([`IsA`]) out of the
//! data tables; the character writer fills the equipped items and the
//! player's state inputs ([`Equipment`]). `d2-server` character storage
//! calls [`fill`] (through [`super::Header::rebuild_appearance`]) whenever
//! it writes a save.
//!
//! Token positions are pinned by tests against known token values; the
//! colour bytes and body armour parts are covered by unit tests only.
//!
//! Randomness: none.

#[cfg(test)]
mod tests;

/// Body parts: the index of a part's byte in the components and in the
/// colours (the order of the character's composite layers).
pub mod part {
    /// Head.
    pub const HD: usize = 0;
    /// Torso.
    pub const TR: usize = 1;
    /// Legs.
    pub const LG: usize = 2;
    /// Right arm.
    pub const RA: usize = 3;
    /// Left arm.
    pub const LA: usize = 4;
    /// Right hand.
    pub const RH: usize = 5;
    /// Left hand.
    pub const LH: usize = 6;
    /// Shield.
    pub const SH: usize = 7;
    /// Right shoulder pad.
    pub const S1: usize = 8;
    /// Left shoulder pad.
    pub const S2: usize = 9;
    /// Number of parts (16 component and 16 colour bytes).
    pub const COUNT: usize = 16;
}

/// The item types the fill asks about (itemtypes row indices).
pub mod ty {
    pub const TORS: i32 = 3;
    pub const GEM: i32 = 20;
    pub const HELM: i32 = 37;
    pub const WEAP: i32 = 45;
    pub const ARMO: i32 = 50;
    pub const SHLD: i32 = 51;
    pub const CIRC: i32 = 75;
}

/// Weapon class indices (see [`wclass_index`]).
pub mod wclass {
    pub const BOW: i32 = 1;
    pub const ONE_HS: i32 = 2;
    pub const ONE_HT: i32 = 3;
    pub const XBW: i32 = 7;
    pub const HT1: i32 = 12;
}

/// Item qualities.
pub mod quality {
    pub const MAGIC: u8 = 4;
    pub const SET: u8 = 5;
    pub const RARE: u8 = 6;
    pub const UNIQUE: u8 = 7;
}

/// Body locations an item can be equipped at.
pub mod body {
    pub const HEAD: u8 = 1;
    pub const TORSO: u8 = 3;
    pub const RIGHT_HAND: u8 = 4;
    pub const LEFT_HAND: u8 = 5;
}

/// The unit mode of an item that is worn.
pub const MODE_EQUIPPED: u8 = 1;
/// The item flag set on an item that has sockets.
pub const FLAG_SOCKETED: u32 = 0x800;
/// Entries of the token table (entry 0 is never used).
pub const TOKEN_ENTRIES: usize = 255;
/// Entries of the reference table of reserved token slots.
pub const REFERENCE_ENTRIES: usize = 256;
/// Body grid slots the hand-owner search reads (body slots 0–10).
pub const BODY_SLOTS: usize = 11;

/// The three `armtype` tokens the token table starts with.
pub const ARMTYPE_CODES: [[u8; 4]; 3] = [*b"lit ", *b"med ", *b"hvy "];

/// The `wclass` index of a `wclass` code: `bow` 1, `1hs` 2, `1ht` 3, `stf` 4, `2hs` 5, `2ht` 6,
/// `xbw` 7, `ht1` 12; any other code 0 (no table entry).
pub fn wclass_index(code: [u8; 4]) -> i32 {
    match &code {
        b"bow " => 1,
        b"1hs " => 2,
        b"1ht " => 3,
        b"stf " => 4,
        b"2hs " => 5,
        b"2ht " => 6,
        b"xbw " => 7,
        b"ht1 " => 12,
        _ => 0,
    }
}

fn code_u32(c: [u8; 4]) -> u32 {
    u32::from_le_bytes(c)
}

/// The itemtypes is-a matrix: bit (i, j) says "type i is a type j". Types
/// outside the matrix are never anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IsA {
    n: usize,
    bits: Vec<bool>,
}

impl IsA {
    /// From a predicate over the `n` × `n` type pairs.
    pub fn from_fn(n: usize, f: impl Fn(usize, usize) -> bool) -> Self {
        let mut bits = Vec::with_capacity(n * n);
        for i in 0..n {
            for j in 0..n {
                bits.push(f(i, j));
            }
        }
        Self { n, bits }
    }

    /// Whether type `ty` is a type `of`.
    pub fn is_a(&self, ty: i32, of: i32) -> bool {
        let (Ok(i), Ok(j)) = (usize::try_from(ty), usize::try_from(of)) else {
            return false;
        };
        i < self.n && j < self.n && self.bits[i * self.n + j]
    }
}

/// The items record columns the appearance reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemGfx {
    pub code: [u8; 4],
    /// The code the item draws as instead of its own; all zero when empty.
    pub alternategfx: [u8; 4],
    /// `type`, as an itemtypes row index.
    pub item_type: i32,
    /// The body part a helm or hand item draws on.
    pub component: u8,
    /// `Torso`, `Legs`, `rArm`, `lArm`, `rSPad`, `lSPad`: the `armtype`
    /// rows body armour draws on parts TR, LG, RA, LA, S1, S2.
    pub arm: [u8; 6],
    /// Whether the item has an inventory of its own (sockets).
    pub hasinv: u8,
    /// `Transform`: the palette family the item's colour picks from.
    pub transform: u8,
    /// A gem's row in the `gems` table.
    pub gemoffset: i32,
    pub wclass: [u8; 4],
    /// `2handedwclass`: the weapon class the front-end paper doll uses when
    /// the weapon is held in two hands.
    pub wclass2: [u8; 4],
}

/// One slot of the reference table: whether its item type is a `weap` and
/// whether it is an `armo`, the only two questions the token table build
/// asks of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RefSlot {
    pub weap: bool,
    pub armo: bool,
}

/// The reference table of reserved token slots (256 entries). Each slot
/// carries an item type; a weapon row never takes a slot whose type is a
/// weapon, nor an armour row a slot whose type is armour. This keeps the
/// stock token numbers fixed whatever the item tables hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceSlots(pub Vec<RefSlot>);

impl ReferenceSlots {
    /// From the item type of each slot and the is-a matrix.
    pub fn from_types(types: &[i32], m: &IsA) -> Self {
        Self(
            types
                .iter()
                .map(|&t| RefSlot {
                    weap: m.is_a(t, ty::WEAP),
                    armo: m.is_a(t, ty::ARMO),
                })
                .collect(),
        )
    }

    /// A table without reserved slots (every slot neither `weap` nor
    /// `armo`).
    pub fn none() -> Self {
        Self(vec![RefSlot::default(); REFERENCE_ENTRIES])
    }

    /// The game's reference table: [`REFERENCE_TYPES`] under the game's
    /// itemtypes is-a matrix `m`. `d2-server`'s `character_save` tests
    /// check it against the player's install.
    pub fn game(m: &IsA) -> Self {
        Self::from_types(&reference_types(), m)
    }

    /// Slot `i`; a slot past the table reserves nothing.
    fn get(&self, i: usize) -> RefSlot {
        self.0.get(i).copied().unwrap_or_default()
    }
}

/// The item type of each slot of the reference table, as (first slot,
/// last slot, type) runs covering 0..=255.
pub const REFERENCE_TYPES: [(u8, u8, i32); 91] = [
    (0, 3, 1),
    (4, 10, 37),
    (11, 26, 3),
    (27, 30, 2),
    (31, 33, 16),
    (34, 36, 15),
    (37, 39, 19),
    (40, 40, 37),
    (41, 42, 2),
    (43, 45, 25),
    (46, 46, 29),
    (47, 47, 43),
    (48, 49, 30),
    (50, 50, 36),
    (51, 51, 28),
    (52, 53, 30),
    (54, 54, 36),
    (55, 56, 31),
    (57, 58, 30),
    (59, 62, 38),
    (63, 66, 32),
    (67, 69, 33),
    (70, 71, 30),
    (72, 75, 33),
    (76, 79, 27),
    (80, 83, 26),
    (84, 84, 28),
    (85, 85, 34),
    (86, 87, 28),
    (88, 88, 34),
    (89, 89, 31),
    (90, 91, 34),
    (92, 93, 35),
    (94, 94, 43),
    (95, 95, 29),
    (96, 96, 30),
    (97, 97, 36),
    (98, 98, 24),
    (99, 100, 38),
    (101, 101, 42),
    (102, 102, 32),
    (103, 105, 30),
    (106, 106, 33),
    (107, 110, 27),
    (111, 111, 26),
    (112, 112, 28),
    (113, 113, 34),
    (114, 114, 28),
    (115, 116, 35),
    (117, 117, 1),
    (118, 119, 3),
    (120, 120, 2),
    (121, 121, 3),
    (122, 123, 40),
    (124, 125, 19),
    (126, 127, 16),
    (128, 129, 15),
    (130, 130, 30),
    (131, 131, 28),
    (132, 132, 43),
    (133, 133, 29),
    (134, 134, 3),
    (135, 136, 28),
    (137, 138, 36),
    (139, 140, 30),
    (141, 142, 32),
    (143, 145, 33),
    (146, 147, 34),
    (148, 148, 32),
    (149, 149, 33),
    (150, 151, 24),
    (152, 153, 26),
    (154, 154, 36),
    (155, 164, 28),
    (165, 168, 25),
    (169, 169, 36),
    (170, 172, 24),
    (173, 179, 36),
    (180, 193, 30),
    (194, 197, 32),
    (198, 198, 42),
    (199, 199, 43),
    (200, 200, 42),
    (201, 201, 43),
    (202, 211, 33),
    (212, 217, 34),
    (218, 222, 26),
    (223, 230, 27),
    (231, 234, 35),
    (235, 241, 37),
    (242, 255, 3),
];

/// [`REFERENCE_TYPES`] expanded to one item type per slot.
pub fn reference_types() -> [i32; 256] {
    let mut out = [0; 256];
    for &(a, b, t) in &REFERENCE_TYPES {
        for slot in &mut out[usize::from(a)..=usize::from(b)] {
            *slot = t;
        }
    }
    out
}

/// The token table: 255 entries of (code, item type), entry 0 unused. A
/// part's component byte is the number of the entry holding its graphic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenTable {
    entries: Vec<(u32, i32)>,
    /// The fill mark: entries below it are packed, and only they are
    /// searched for duplicates.
    n: usize,
}

impl TokenTable {
    /// Entries 1–3 are `lit`, `med`, `hvy` (type `tors`); then every
    /// eligible items record, in record order, takes a free entry.
    ///
    /// The rules, in order, for each record:
    /// - its graphic code is `alternategfx` when it has one, else `code`;
    /// - a code already present below the fill mark is skipped (an entry
    ///   placed above the mark is not seen, so a code can appear twice);
    /// - only weapons, torsos, shields and helms get an entry, and never
    ///   circlets;
    /// - it takes the first entry from the fill mark on that is empty and
    ///   not reserved for its kind by the reference table; when there is
    ///   none it takes the entry at the fill mark, whatever is there;
    /// - the fill mark moves up by one when the entry taken was the one at
    ///   the mark. Once the mark reaches the end, nothing more is placed.
    pub fn build(rows: &[ItemGfx], m: &IsA, reference: &ReferenceSlots) -> Self {
        let mut entries = vec![(0u32, 0i32); TOKEN_ENTRIES];
        for (k, c) in ARMTYPE_CODES.iter().enumerate() {
            entries[k + 1] = (code_u32(*c), 3);
        }
        let mut n = 4usize;
        for r in rows {
            // The code is the alternate graphics code when there is one.
            let g = if r.alternategfx != [0; 4] {
                code_u32(r.alternategfx)
            } else {
                code_u32(r.code)
            };
            // A code already in the table is skipped; only entries below n are
            // compared.
            if entries[..n].iter().any(|&(c, _)| c == g) {
                continue;
            }
            // Only weapons, torsos, shields and helms (not circlets) get an entry.
            let t = r.item_type;
            let eligible = (m.is_a(t, ty::WEAP)
                || m.is_a(t, ty::TORS)
                || m.is_a(t, ty::SHLD)
                || m.is_a(t, ty::HELM))
                && !m.is_a(t, ty::CIRC);
            if !eligible {
                continue;
            }
            // The first free entry from n on that is not reserved for this kind
            // of item by the reference table.
            let weap = m.is_a(t, ty::WEAP);
            let armo = m.is_a(t, ty::ARMO);
            let mut i = n;
            while i < TOKEN_ENTRIES {
                let s = reference.get(i);
                if (s.weap && weap) || (s.armo && armo) || entries[i].0 != 0 {
                    i += 1;
                } else {
                    break;
                }
            }
            if i >= TOKEN_ENTRIES {
                i = n;
            }
            if i >= TOKEN_ENTRIES {
                // The table is full: stop placing (a table this size is not
                // expected).
                continue;
            }
            // Place the entry; the count grows when it lands at n.
            entries[i] = (g, t);
            if i == n {
                n += 1;
            }
        }
        Self { entries, n }
    }

    /// The number of the first entry 1..254 whose code is `a` or `b`; 0 when
    /// none is. An empty entry has code 0, so a zero code matches the first
    /// empty entry; an all-zero `a` is compared like any other code.
    pub fn lookup(&self, a: [u8; 4], b: [u8; 4]) -> u8 {
        let (a, b) = (code_u32(a), code_u32(b));
        (1..TOKEN_ENTRIES)
            .find(|&i| self.entries[i].0 == a || self.entries[i].0 == b)
            .map_or(0, |i| i as u8)
    }

    /// Entry `i`: (code, item type).
    pub fn entry(&self, i: usize) -> Option<([u8; 4], i32)> {
        self.entries.get(i).map(|&(c, t)| (c.to_le_bytes(), t))
    }

    /// The fill mark after the build.
    pub fn count(&self) -> usize {
        self.n
    }
}

/// One `states` row that recolours items: every row whose `itemtype` is
/// above 0, in row order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateColour {
    /// The state's row id.
    pub state: u32,
    /// `itemtype`: the items the state recolours.
    pub item_type: i32,
    /// `itemtrans`: the colour it gives them (read signed).
    pub itemtrans: i32,
}

/// The colour columns of the data tables, read signed (−1 when the column
/// names no colour).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColourTables {
    /// `transformcolor` of the combined magic affix list (suffixes,
    /// prefixes, automagic), by combined index (affix id − 1).
    pub affix: Vec<i32>,
    /// `setitems` `chrtransform`, by row.
    pub set: Vec<i32>,
    /// `uniqueitems` `chrtransform`, by row.
    pub unique: Vec<i32>,
    /// `gems` `transform`, by row.
    pub gem: Vec<i32>,
    /// The states that recolour items.
    pub states: Vec<StateColour>,
}

/// Everything the fill reads from the data tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceTables {
    /// Items records in record order (weapons, armor, misc).
    pub items: Vec<ItemGfx>,
    /// `armtype` `token`, by row.
    pub armtype: Vec<[u8; 4]>,
    pub types: IsA,
    pub colours: ColourTables,
    pub tokens: TokenTable,
}

impl AppearanceTables {
    /// The tables, with the token table built from `items`.
    pub fn new(
        items: Vec<ItemGfx>,
        armtype: Vec<[u8; 4]>,
        types: IsA,
        colours: ColourTables,
        reference: &ReferenceSlots,
    ) -> Self {
        let tokens = TokenTable::build(&items, &types, reference);
        Self {
            items,
            armtype,
            types,
            colours,
            tokens,
        }
    }
}

/// One item of the player's inventory list, with what the fill reads of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquippedItem {
    /// Its items record.
    pub record: usize,
    /// Unit mode ([`MODE_EQUIPPED`] when worn).
    pub mode: u8,
    /// Body location (see [`body`]).
    pub body_loc: u8,
    /// Quality (see [`quality`]).
    pub quality: u8,
    /// Magic prefix ids (combined affix index + 1; 0 for none).
    pub prefix: [u16; 3],
    /// Magic suffix ids.
    pub suffix: [u16; 3],
    /// Automagic affix id.
    pub auto_affix: u16,
    /// Set or unique row.
    pub file_index: i32,
    /// Item flags (see [`FLAG_SOCKETED`]).
    pub flags: u32,
    /// The type's socket limit at the item's level.
    pub max_sockets: i32,
    /// Items record of the first item socketed into it.
    pub first_child: Option<usize>,
}

/// What the character writer gathers from the game for the fill.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Equipment {
    /// The inventory item list, in list order.
    pub items: Vec<EquippedItem>,
    /// The body grid (slots 0–10): indices into `items`.
    pub body_grid: [Option<usize>; BODY_SLOTS],
    /// The weapon in use: index into `items`.
    pub weapon_in_use: Option<usize>,
    /// The player's weapon class index.
    pub weapon_class: i32,
    /// The states the player has (only the recolouring ones matter).
    pub states: Vec<u32>,
}

/// The colour byte of palette family `t` and colour `k`:
/// (t × 32 + (k & 0x1F)) mod 256 for 1 ≤ t ≤ 8 and 0 ≤ k ≤ 20, else 0.
pub fn colour_byte(t: u8, k: i32) -> u8 {
    if (1..=8).contains(&t) && (0..=20).contains(&k) {
        (u32::from(t) * 32 + (k as u32 & 0x1F)) as u8
    } else {
        0
    }
}

/// Whether palette family `t` has colour `k`: 1 ≤ t ≤ 8 except families 3
/// and 4, and k < 21 (signed).
pub fn colour_ok(t: u8, k: i32) -> bool {
    (1..=8).contains(&t) && t != 3 && t != 4 && k < 21
}

/// A one-byte table link as the colour tables hold it (signed, so 0xFF
/// reads as −1, "none").
pub fn link8_signed(v: u8) -> i32 {
    i32::from(v as i8)
}

struct Fill<'a> {
    eq: &'a Equipment,
    t: &'a AppearanceTables,
    comps: &'a mut [u8; part::COUNT],
    cols: &'a mut [u8; part::COUNT],
}

impl Fill<'_> {
    fn rec(&self, i: &EquippedItem) -> ItemGfx {
        self.t.items.get(i.record).copied().unwrap_or_default()
    }

    fn token_of(&self, r: &ItemGfx) -> u8 {
        self.t.tokens.lookup(r.alternategfx, r.code)
    }

    /// The hand owners R and L, by index into the item list.
    fn hand_owners(&self) -> (Option<usize>, Option<usize>) {
        let (mut r, mut l) = (None, None);
        for &slot in &self.eq.body_grid {
            let Some(ix) = slot else { continue };
            let Some(it) = self.eq.items.get(ix) else {
                continue;
            };
            let g = self.rec(it);
            let wc = wclass_index(g.wclass);
            let (is_r, is_l) = if matches!(wc, wclass::ONE_HS | wclass::ONE_HT | wclass::HT1) {
                let in_use = self.eq.weapon_in_use == Some(ix);
                (in_use, !in_use)
            } else {
                (g.component == 5, g.component == 6)
            };
            if is_r && r.is_none() {
                r = Some(ix);
            }
            if is_l && l.is_none() {
                l = Some(ix);
            }
        }
        (r, l)
    }

    /// The colour of one item: writes the byte (or not) and returns whether
    /// the item has a palette.
    fn colour(&self, it: &EquippedItem, byte: &mut u8) -> bool {
        let g = self.rec(it);
        let tr = g.transform;
        // State colours come first.
        for s in &self.t.colours.states {
            if self.eq.states.contains(&s.state)
                && s.itemtrans < 21
                && self.t.types.is_a(g.item_type, s.item_type)
            {
                return colour_ok(tr, s.itemtrans);
            }
        }
        let affix_k = |id: u16| -> Option<i32> {
            let k = *self.t.colours.affix.get(usize::from(id).checked_sub(1)?)?;
            (k != -1).then_some(k)
        };
        let automagic = |byte: &mut u8| -> bool {
            match affix_k(it.auto_affix) {
                Some(k) => {
                    *byte = colour_byte(tr, k);
                    colour_ok(tr, k)
                }
                None => false,
            }
        };
        let row = |v: &Vec<i32>| -> Option<i32> {
            usize::try_from(it.file_index)
                .ok()
                .and_then(|r| v.get(r).copied())
        };
        match it.quality {
            quality::MAGIC | quality::RARE => {
                let first = it
                    .suffix
                    .iter()
                    .chain(it.prefix.iter())
                    .find_map(|&id| affix_k(id));
                match first {
                    Some(k) => {
                        *byte = colour_byte(tr, k);
                        colour_ok(tr, k)
                    }
                    None => automagic(byte),
                }
            }
            quality::SET | quality::UNIQUE => {
                let v = if it.quality == quality::SET {
                    &self.t.colours.set
                } else {
                    &self.t.colours.unique
                };
                match row(v) {
                    Some(k) if k >= 0 => {
                        *byte = colour_byte(tr, k);
                        colour_ok(tr, k)
                    }
                    _ => false,
                }
            }
            _ => {
                let gem = it.first_child.and_then(|c| {
                    let cg = self.t.items.get(c)?;
                    if !self.t.types.is_a(cg.item_type, ty::GEM) {
                        return None;
                    }
                    let r = usize::try_from(cg.gemoffset).ok()?;
                    self.t.colours.gem.get(r).copied()
                });
                match gem {
                    Some(k)
                        if g.hasinv != 0
                            && it.max_sockets != 0
                            && it.flags & FLAG_SOCKETED != 0 =>
                    {
                        *byte = colour_byte(tr, k);
                        colour_ok(tr, k)
                    }
                    _ => automagic(byte),
                }
            }
        }
    }

    /// Writes the colour of part `p` from the item: the colour byte plus
    /// one when the item's palette family has the colour, else 0xFF.
    fn set_colour(&mut self, it: &EquippedItem, p: usize) {
        let mut b = self.cols[p];
        let ok = self.colour(it, &mut b);
        self.cols[p] = if ok { b.wrapping_add(1) } else { 0xFF };
    }

    /// Helm and hand items: component and colour of the part they occupy.
    fn hand_or_helm(&mut self, ix: usize, r_owner: Option<usize>, l_owner: Option<usize>) {
        let it = self.eq.items[ix];
        let g = self.rec(&it);
        // An item without a token clears its part.
        let mut p = usize::from(g.component);
        let t = self.token_of(&g);
        if t == 0 {
            if p < part::COUNT {
                self.comps[p] = 0xFF;
                self.cols[p] = 0xFF;
            }
            return;
        }
        // The hand owners take the right / left hand part.
        if r_owner == Some(ix) {
            p = part::RH;
        } else if l_owner == Some(ix) {
            p = part::LH;
        } else if p >= part::COUNT {
            return;
        }
        self.comps[p] = t;
        // Crossbow: only for the hands.
        if matches!(it.body_loc, body::RIGHT_HAND | body::LEFT_HAND) {
            let w = self.eq.weapon_class;
            if w == wclass::XBW && matches!(g.component, 5 | 6) {
                if let Some(r) = r_owner {
                    let rg = self.rec(&self.eq.items[r]);
                    match self.token_of(&rg) {
                        0 => {
                            self.comps[part::LH] = 0xFF;
                            self.cols[part::LH] = 0xFF;
                        }
                        rt => self.comps[part::LH] = rt,
                    }
                }
            }
            // A `bow` with component 6 would look up the items record `lit `;
            // the game's tables have none, so nothing happens.
        }
        // Colour of the part.
        self.set_colour(&it, p);
    }

    /// Body armour: a token and colour for each of its six armour parts,
    /// looked up through the `armtype` row the part names. Record 0 (the
    /// first weapon row) never draws as armour.
    fn body_armour(&mut self, ix: usize) {
        const PARTS: [usize; 6] = [part::TR, part::LG, part::RA, part::LA, part::S1, part::S2];
        let it = self.eq.items[ix];
        let g = self.rec(&it);
        for (k, &c) in PARTS.iter().enumerate() {
            let v = usize::from(g.arm[k]);
            // A row past the `armtype` table counts as no token.
            let t = self
                .t
                .armtype
                .get(v)
                .map_or(0, |&tok| self.t.tokens.lookup(tok, tok));
            if it.record == 0 || t == 0 {
                self.comps[c] = 0xFF;
                self.cols[c] = 0xFF;
            } else {
                self.comps[c] = t;
                self.set_colour(&it, c);
            }
        }
    }
}

/// Writes the bytes of every equipped helm, body armour and hand item into
/// `components` / `colours`, walking the item list in order so later items
/// overwrite earlier ones. Circlets and other body locations draw nothing.
/// The caller has filled both rows with 0xFF first, as
/// [`super::Header::reset_appearance`] does.
pub fn fill(
    eq: &Equipment,
    t: &AppearanceTables,
    components: &mut [u8; part::COUNT],
    colours: &mut [u8; part::COUNT],
) {
    let mut f = Fill {
        eq,
        t,
        comps: components,
        cols: colours,
    };
    let (r, l) = f.hand_owners();
    for (ix, it) in eq.items.iter().enumerate() {
        if it.mode != MODE_EQUIPPED {
            continue;
        }
        match it.body_loc {
            body::HEAD | body::RIGHT_HAND | body::LEFT_HAND => {
                if t.types.is_a(f.rec(it).item_type, ty::CIRC) {
                    continue;
                }
                f.hand_or_helm(ix, r, l);
            }
            body::TORSO => f.body_armour(ix),
            _ => {}
        }
    }
}
