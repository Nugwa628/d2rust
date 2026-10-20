//! The hover text builder of the item tool tip: one UTF-16 text, bottom
//! line first, with a colour code in front of each non-empty block, as
//! Builds it. [`super::item_tip::ItemTips::tip`] is the
//! entry; the inputs beyond the item's own record are a [`TipCtx`].
//!
//! Seams the host fills (none of them invents a value; a missing seam
//! leaves the part out):
//! - the units P and U ([`TipUnit`]): stat totals, class, skills and
//!   states, the state lists 165–170;
//! - the attack animation of the speed line, the skill calcs of Holy Shield and
//!   the one-or-two-handed answer ([`TipUnit`] methods;
//!   that answer's rule is not known);
//! - the spell value of `spelldesc` 2–4 ([`TipCtx::spell_value`]);
//! - the store price text ([`PriceText`]).
//!
//! The item's own totals are its base values plus the summed list L and the
//! op-13 percents (the only item-owned ops); the full
//! stat engine is not run on the client copy (a +50 % enhanced
//! damage short sword's tip agrees on every row). The possessive form
//! (English only) is built for the ear name. PROVISIONAL: the
//! personalized names (the whole-name form, and the set / unique / rare names
//! in their tips) show unchanged; to be settled once the
//! personalized name rule is known.

use d2_proto::item_bits::ItemBits;
use d2_sim::items::inventory::levelreq::{level_requirement, AffixReq, LevelReqItem, LevelReqUnit};
use d2_sim::stats::muldiv;

use super::item_tip::{color, ItemTips};
use super::item_tip_desc::{sid, DescNames, Viewer};
use super::item_tip_props::{self as props, StatList};
use super::wformat::{format, Arg};

/// Item flags.
pub mod flag {
    pub const IDENTIFIED: u32 = 0x10;
    pub const BROKEN: u32 = 0x100;
    pub const SOCKETED: u32 = 0x800;
    pub const HARDCORE: u32 = 0x8000;
    pub const EAR: u32 = 0x1_0000;
    pub const ETHEREAL: u32 = 0x40_0000;
    pub const PERSONALIZED: u32 = 0x100_0000;
    pub const RUNEWORD: u32 = 0x400_0000;
}

/// Item types the builder tests (`itemtypes` rows).
pub mod ty {
    pub const GOLD: i16 = 4;
    pub const PLAY: i16 = 7;
    pub const CHAR: i16 = 13;
    pub const BOOT: i16 = 15;
    pub const BOOK: i16 = 18;
    pub const GEM: i16 = 20;
    pub const SCRO: i16 = 22;
    pub const BOW: i16 = 27;
    pub const XBOW: i16 = 35;
    pub const TPOT: i16 = 38;
    pub const BODY: i16 = 40;
    pub const WEAP: i16 = 45;
    pub const ARMO: i16 = 50;
    pub const SHLD: i16 = 51;
    pub const SOCK: i16 = 53;
    pub const BLUN: i16 = 57;
    pub const RUNE: i16 = 74;
    pub const ELIX: i16 = 11;
}

/// String ids of the tip lines.
pub mod tid {
    pub const SOCKETED: u16 = 3453;
    pub const UNIDENTIFIED: u16 = 3455;
    pub const DURABILITY: u16 = 3457;
    pub const REQ_STR: u16 = 3458;
    pub const REQ_DEX: u16 = 3459;
    pub const DEFENSE: u16 = 3461;
    pub const QUANTITY: u16 = 3462;
    pub const OF: u16 = 3463;
    pub const TO: u16 = 3464;
    pub const ONE_HAND: u16 = 3465;
    pub const TWO_HAND: u16 = 3466;
    pub const THROW: u16 = 3467;
    pub const SMITE: u16 = 3468;
    pub const REQ_LEVEL: u16 = 3469;
    pub const KICK: u16 = 21782;
    pub const BLOCK: u16 = 11018;
    pub const CLASS_ONLY: u16 = 10917;
    pub const CHARM: u16 = 20438;
    pub const ETHEREAL: u16 = 22745;
    pub const QUOTE: u16 = 20506;
    pub const INSERT: u16 = 11080;
    pub const SLOT_LABELS: [(u16, usize); 4] = [(11074, 2), (11073, 1), (11076, 1), (11075, 0)];
    pub const SPEEDS: [u16; 6] = [4088, 4089, 4090, 4091, 4092, 4093];
    pub const RUNEWORD_NONE: u16 = 0xFFFF;
    pub const READ: u16 = 2205;
    pub const OPEN: u16 = 2204;
    pub const USE: u16 = 2203;
    pub const INSERT_SCROLLS: u16 = 2206;
    pub const NAME_LOW: u16 = 1712;
    pub const NAME_SUPERIOR: u16 = 1711;
    pub const SUPERIOR: u16 = 1727;
    pub const NAME_MAGIC: u16 = 1714;
    pub const NAME_GEMMED: u16 = 1715;
    pub const GEMMED: u16 = 1728;
    pub const NAME_BODY: u16 = 1716;
    pub const NAME_RARE: u16 = 1718;
    pub const NAME_SET: u16 = 10089;
    pub const TP_TOME: u16 = 2199;
    pub const TP_SCROLL: u16 = 2200;
    pub const ID_TOME: u16 = 2201;
    pub const ID_SCROLL: u16 = 2202;
    pub const HARDCORE: u16 = 5126;
    pub const LEVEL: u16 = 4141;
    pub const CANNOT_REPAIR: u16 = 22746;
    pub const NOT_TRADED: u16 = 3333;
    pub const REPAIR_UNID: u16 = 4022;
    pub const COST: u16 = 3329;
    pub const REPAIR_COST: u16 = 3330;
    pub const SELL: u16 = 3331;
    pub const IDENTIFY_COST: u16 = 3332;
}

/// Weapon class words (table): (type, string).
const CLASS_WORDS: [(i16, u16); 15] = [
    (26, 4085),
    (28, 4078),
    (30, 4079),
    (32, 4080),
    (38, 4081),
    (44, 4082),
    (33, 4083),
    (27, 4084),
    (34, 4086),
    (35, 4087),
    (67, 21258),
    (88, 21258),
    (68, 4085),
    (25, 4085),
    (57, 4077),
];

/// Speed categories (table), rows s = 10 … 27.
const SPEED_TABLE: [[u8; 5]; 18] = [
    [1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1],
    [1, 1, 2, 1, 1],
    [2, 1, 2, 2, 1],
    [2, 1, 2, 2, 2],
    [2, 2, 3, 2, 2],
    [3, 2, 3, 3, 2],
    [3, 2, 3, 3, 3],
    [3, 2, 4, 3, 3],
    [4, 3, 4, 4, 3],
    [4, 3, 4, 4, 4],
    [4, 3, 5, 4, 4],
    [5, 4, 5, 5, 4],
    [5, 4, 5, 5, 5],
    [5, 4, 5, 5, 5],
    [5, 5, 5, 5, 5],
    [5, 5, 5, 5, 5],
];

/// Speed column k by class (b = 0 / 1, table).
const SPEED_COLUMN: [[usize; 2]; 7] = [[0, 2], [1, 4], [1, 4], [0, 3], [0, 3], [1, 4], [0, 3]];

/// Gamble class lines by primary type 60–88.
const GAMBLE_CLASS: [(u32, &[i16]); 7] = [
    (0, &[60, 85, 86, 87]),
    (4, &[61, 71]),
    (2, &[62, 69]),
    (3, &[63, 70]),
    (1, &[64, 68]),
    (6, &[65, 67, 88]),
    (5, &[66, 72, 73]),
];

/// Codes whose names are orange.
const SPECIAL: [&[u8; 4]; 11] = [
    b"ceh ", b"bet ", b"fed ", b"tes ", b"toa ", b"dhn ", b"bey ", b"mbr ", b"pk1 ", b"pk2 ",
    b"pk3 ",
];

/// A unit the tip reads (the panel unit U or the local player P).
pub trait TipUnit {
    /// Unit type (0 player, 1 monster).
    fn unit_type(&self) -> u8;
    /// Class (unit +4).
    fn class(&self) -> u32;
    /// Unit total of (stat, layer).
    fn stat(&self, id: u16, layer: u16) -> i32;
    /// Unit flag 0x2000000 (expansion).
    fn expansion(&self) -> bool {
        true
    }
    /// State `state` is on.
    fn has_state(&self, state: u8) -> bool {
        let _ = state;
        false
    }
    /// The unit's stat list of `state` (165–170).
    fn state_list(&self, state: u8) -> Option<StatList> {
        let _ = state;
        None
    }
    /// Frames F and speed V of the attack-1 animation for a weapon class;
    /// `None`: query not found.
    fn attack_anim(&self, weapon_class: [u8; 4]) -> Option<(i32, i32)> {
        let _ = weapon_class;
        None
    }
    /// Holy Shield (skill 117 with state 101): the smite min / max adds
    /// (the smite skill calcs >> 8) and the block add at the skill's
    /// level; `None` without the skill.
    fn holy_shield(&self) -> Option<(i32, i32, i32)> {
        None
    }
    /// The item is both one- and two-handed for this unit.
    fn one_or_two_handed(&self, item_one_or_two: bool) -> bool {
        let _ = item_one_or_two;
        false
    }
}

/// The store price text, decided by the host
/// from the store state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PriceText {
    /// The function fails (gold, no NPC, not sellable, …).
    #[default]
    Fail,
    /// `label` + `%d` of `price` (`Cost: `, `Sell value: `, `Repair
    /// cost: `, `Identify cost: `).
    Price { label: u16, price: i32 },
    /// Success with an empty text (an identified item at an identify
    /// NPC).
    Empty,
    /// `Pfx(4022, 1)`: an unidentified item at a repair NPC.
    RepairUnidentified,
}

/// What the builder reads outside the item's own record.
#[derive(Default)]
pub struct TipCtx<'a> {
    /// Local player P.
    pub player: Option<&'a dyn TipUnit>,
    /// Panel unit U (the requirement and class tests, defense).
    pub unit: Option<&'a dyn TipUnit>,
    /// The items of I's inventory (socket fillers), in list order.
    pub fillers: Vec<ItemBits>,
    /// Inventory mode (1–9 store states).
    pub mode: u8,
    /// The hovered item is the player's own.
    pub own_item: bool,
    /// Gamble store.
    pub gamble: bool,
    pub price: PriceText,
    /// Cursor mode.
    pub cursor_mode: u8,
    /// Current difficulty.
    pub difficulty: u8,
    /// The client act's base time (by-time values; `None`: no act).
    pub act_time: Option<i32>,
    /// `spelldesc` 2–4 value.
    pub spell_value: Option<i32>,
    /// Set tip inputs.
    pub set: SetCtx,
}

/// The set facts the host reads from U''s inventory.
#[derive(Clone, Debug, Default)]
pub struct SetCtx {
    /// setitems rows U' owns.
    pub owned: Vec<u32>,
    /// Set slots of U''s body items of the set
    /// without I.
    pub worn_without: u8,
    pub worn_with: u8,
}

/// The built tip: the text and the pop-up colour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TipText {
    pub text: Vec<u16>,
    pub color: u16,
}

/// `Pfx(B, k)`: a colour code in front of a non-empty block.
pub fn pfx(mut b: Vec<u16>, k: u16) -> Vec<u16> {
    if !b.is_empty() {
        let mut out = vec![0xFF, u16::from(b'c'), u16::from(b'0') + k];
        out.append(&mut b);
        b = out;
    }
    b
}

/// The spell of a scroll or book record: its row in the `books` table
/// (`books`: the (scroll code, book code) pairs). A full record carries it
/// in its quality fields. A compact record (the stackless scrolls, like the
/// Scroll of Identify every new character starts with) carries no quality
/// fields at all, so its code names the row, the same row the server gives
/// the scroll when it creates it. The name line and the scroll-into-tome
/// rule both read this.
pub fn scroll_spell(books: &[([u8; 4], [u8; 4])], b: &ItemBits) -> u8 {
    match b.quality_fields.spell {
        Some(s) => s,
        None if b.flags & d2_proto::item_bits::hflag::COMPACT != 0 => books
            .iter()
            .position(|&(scroll, _)| scroll == b.code)
            .map_or(0, |n| n as u8),
        None => 0,
    }
}

/// The per-code columns the builder reads (items records).
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemText {
    pub name_id: u16,
    pub req_str: u16,
    pub req_dex: u16,
    pub levelreq: u8,
    pub block: u8,
    pub mindam: u8,
    pub maxdam: u8,
    pub mindam2: u8,
    pub maxdam2: u8,
    pub minmisdam: u8,
    pub maxmisdam: u8,
    pub durability: u8,
    pub nodurability: u8,
    pub quest: u8,
    pub questdiffcheck: u8,
    pub skipname: u8,
    pub gemoffset: u32,
    pub spelldesc: u8,
    pub spelldescstr: u16,
    pub stat1: u16,
    pub hasinv: u8,
    pub twohanded: u8,
    pub one_or_two: u8,
    pub transmogrify: u8,
    pub tmogtype: [u8; 4],
    pub maxstack: u32,
    pub wclass: [u8; 4],
    pub wclass2: [u8; 4],
    /// `InvTrans` (items `+0x142`): the inventory picture's palette file
    /// `t`.
    pub inv_trans: u8,
}

/// One build: the item, its tables and the context.
pub(super) struct Build<'a> {
    t: &'a ItemTips,
    b: &'a ItemBits,
    ctx: &'a TipCtx<'a>,
    rec: ItemText,
    idx: Option<usize>,
    /// The shown list L.
    l: StatList,
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The text buffer size the item name builder passes to the possessive.
const POSSESSIVE_SIZE: usize = 512;

/// The English possessive (language 0 and 12): the player name, then `' `
/// when the name ends in a lower-case `s`, else `'s `, then the
/// text. The name, suffix and text must fit with 5 spare characters in the buffer; if
/// they do not, the text alone is the result (the buffer is left as it was when the text
/// itself fills it: modelled as empty). Other languages build the form with their own
/// suffixes and are not modelled (the client is English).
fn possessive(name: &[u8], text: &[u16]) -> Vec<u16> {
    if name.len() + text.len() + 5 > POSSESSIVE_SIZE {
        return if text.len() < POSSESSIVE_SIZE {
            text.to_vec()
        } else {
            Vec::new()
        };
    }
    let mut t: Vec<u16> = name.iter().map(|&c| u16::from(c)).collect();
    t.extend(w(if name.last() == Some(&b's') {
        "' "
    } else {
        "'s "
    }));
    t.extend(text);
    t
}

impl<'a> Build<'a> {
    pub(super) fn new(t: &'a ItemTips, b: &'a ItemBits, ctx: &'a TipCtx<'a>) -> Self {
        let rec = t.codes.get(&b.code).copied().unwrap_or_default();
        let idx = t.lookup.find_code(b.code);
        let mut l = StatList::default();
        let main = b.lists.first().and_then(Option::as_ref);
        l.add_list(&t.stat_list(main.map_or(&[][..], Vec::as_slice)));
        if b.flags & flag::RUNEWORD != 0 {
            if let Some(Some(rw)) = b.lists.last().filter(|_| b.lists.len() > 1) {
                l.add_list(&t.stat_list(rw));
            }
        }
        for f in &ctx.fillers {
            if let Some(Some(fl)) = f.lists.first() {
                l.add_list(&t.stat_list(fl));
            }
        }
        Build {
            t,
            b,
            ctx,
            rec,
            idx,
            l,
        }
    }

    fn s(&self, id: u16) -> Vec<u16> {
        self.t.string(id)
    }

    fn nl(&self) -> Vec<u16> {
        self.s(sid::NL)
    }

    fn sp(&self) -> Vec<u16> {
        self.s(sid::SP)
    }

    fn is(&self, t: i16) -> bool {
        self.idx.is_some_and(|i| self.t.lookup.is_type(i, t))
    }

    fn primary(&self) -> i16 {
        self.idx
            .and_then(|i| self.t.lookup.item(i))
            .map_or(-1, |r| r.type_)
    }

    fn itype(&self) -> Option<&d2_data::tables::Itemtypes> {
        self.idx.and_then(|i| self.t.lookup.itype_of(i))
    }

    /// Item class: the itemtypes `class` byte when it is
    /// below 7, else 7 (none; an empty cell is 0xFF).
    fn item_class(&self) -> u8 {
        self.itype()
            .map_or(7, |t| if t.class < 7 { t.class } else { 7 })
    }

    fn throwable(&self) -> bool {
        self.itype().is_some_and(|t| t.throwable != 0)
    }

    fn quiver(&self) -> bool {
        self.itype().is_some_and(|t| t.quiver != 0)
    }

    fn identified(&self) -> bool {
        self.b.flags & flag::IDENTIFIED != 0
    }

    fn viewer(&self) -> Viewer<'_> {
        Viewer {
            player: None,
            player_class: self.ctx.player.map(|p| p.class() as u8),
            unit_class: self
                .ctx
                .unit
                .filter(|u| u.unit_type() == 0)
                .map(|u| u.class() as u8),
            act_time: self.ctx.act_time,
        }
    }

    /// The item's own base value of `s` (layer 0): the record's fields
    /// the stream carries and the weapon damage columns.
    fn base(&self, s: u16) -> i32 {
        let st = |x: Option<d2_proto::item_bits::Stat>| x.map_or(0, |v| v.value() as i32);
        let r = &self.rec;
        match s {
            31 => st(self.b.defense),
            72 => st(self.b.durability),
            73 => st(self.b.max_durability),
            70 => i32::from(self.b.quantity.unwrap_or(0)),
            194 => i32::from(self.b.sockets.unwrap_or(0)),
            21 if self.is(ty::WEAP) => i32::from(r.mindam),
            22 if self.is(ty::WEAP) => i32::from(r.maxdam),
            23 if self.is(ty::WEAP) => i32::from(r.mindam2),
            24 if self.is(ty::WEAP) => i32::from(r.maxdam2),
            159 if self.is(ty::WEAP) => i32::from(r.minmisdam),
            160 if self.is(ty::WEAP) => i32::from(r.maxmisdam),
            _ => 0,
        }
    }

    /// The item's own total of `s` (it agrees with the
    /// full tip): base + L + the op-13 percents of the
    /// item base.
    fn total(&self, s: u16) -> i32 {
        let base = self.base(s);
        let mut v = base.wrapping_add(self.l.get(s, 0));
        for (src, d) in self.t.op13_sources(s) {
            let _ = d;
            let r = self.l.get(src, 0);
            if r != 0 {
                v = v.wrapping_add(muldiv(base, r, 100));
            }
        }
        v
    }

    /// "Bonus of stat s" = total − base.
    fn bonus(&self, s: u16) -> i32 {
        self.total(s).wrapping_sub(self.base(s))
    }

    // -----------------------------------------------------------

    fn ethereal_sockets(&self) -> Vec<u16> {
        let eth = self.b.flags & flag::ETHEREAL != 0;
        let sock = self.b.flags & flag::SOCKETED != 0;
        let mut t = Vec::new();
        if eth {
            t.extend(self.s(tid::ETHEREAL));
        }
        if sock {
            if eth {
                t.extend(self.s(props::pid::COMMA));
                t.extend(self.sp());
            }
            t.extend(self.s(tid::SOCKETED));
            t.extend(self.sp());
            t.extend(w(&format!("({})", self.b.sockets.unwrap_or(0))));
        }
        if eth || sock {
            t.extend(self.nl());
        }
        t
    }

    fn speed_line(&self) -> Vec<u16> {
        let Some(p) = self.ctx.player else {
            return Vec::new();
        };
        let wclass = if self.rec.twohanded != 0 {
            self.rec.wclass2
        } else {
            self.rec.wclass
        };
        let a = 100 + self.total(93) + self.total(68);
        let s = match p.attack_anim(wclass) {
            Some((f, v)) if (a * v) / 100 != 0 => (f * 256) / ((a * v) / 100),
            _ => 45,
        };
        let c = if s >= 28 {
            5
        } else if s < 10 {
            1
        } else {
            let b = usize::from(self.is(ty::BOW) || self.is(ty::XBOW));
            let k = SPEED_COLUMN.get(p.class() as usize).map_or(0, |c| c[b]);
            SPEED_TABLE[(s - 10) as usize][k] as usize
        };
        let mut speed = self.s(tid::SPEEDS[c]);
        if self.bonus(93) != 0 {
            speed = pfx(speed, color::BLUE);
        }
        let mut line = Vec::new();
        if let Some(&(_, word)) = CLASS_WORDS.iter().find(|(t, _)| self.is(*t)) {
            line.extend(self.s(word));
            line.extend(self.sp());
            line.extend(self.s(sid::DASH));
            line.extend(self.sp());
        }
        line.extend(speed);
        line.extend(self.nl());
        line
    }

    /// Inputs of item bits `b` (affix ids as sent;
    /// Prefixes and the auto affix offset by their part).
    fn levelreq_item(&self, b: &ItemBits) -> LevelReqItem {
        let t = &*self.t.lookup;
        let aff = |id: u16| {
            d2_sim::items::affixes::affix(t, id).map(|a| AffixReq {
                levelreq: a.levelreq,
                class: a.class,
                classlevelreq: a.classlevelreq,
            })
        };
        let pre = |p: u16| {
            if p == 0 {
                None
            } else {
                aff(p + t.n_suffix as u16)
            }
        };
        let q = &b.quality_fields;
        let mut prefixes = [None; 3];
        let mut suffixes = [None; 3];
        if let Some((p, s)) = q.magic {
            prefixes[0] = pre(p);
            suffixes[0] = aff(s);
        }
        if let Some(slots) = q.rare_slots {
            for (i, (p, s)) in slots.into_iter().enumerate() {
                prefixes[i] = pre(p);
                suffixes[i] = aff(s);
            }
        }
        let row = q.file_index.map(|i| i as usize);
        let list = b.lists.first().and_then(Option::as_ref);
        let entries = |s: u16| -> Vec<u16> {
            list.into_iter()
                .flatten()
                .filter(|e| e.stat == s)
                .map(|e| e.param as u16)
                .take(64)
                .collect()
        };
        let skill = |l: u16| t.skills.get(usize::from(l));
        LevelReqItem {
            quality: b.quality,
            automagic: b
                .auto_affix
                .filter(|&a| a != 0)
                .and_then(|a| aff(a + t.first_auto() as u16)),
            prefixes,
            suffixes,
            set_lvlreq: row.and_then(|i| t.setitems.get(i)).map_or(0, |s| s.lvl_req),
            unique_lvlreq: row.and_then(|i| t.uniques.get(i)).map(|r| r.lvl_req),
            version: b.version,
            class_levelreq: self
                .t
                .codes
                .get(&b.code)
                .map_or(0, |c| i32::from(c.levelreq)),
            fillers: Vec::new(),
            single_skills: entries(107)
                .into_iter()
                .filter_map(|l| skill(l).map(|s| s.reqlevel))
                .collect(),
            nonclass_skills: entries(97)
                .into_iter()
                .filter_map(|l| skill(l).map(|s| (s.reqlevel, i32::from(s.charclass))))
                .collect(),
            stat_levelreq: list
                .into_iter()
                .flatten()
                .filter(|e| e.stat == 92)
                .map(|e| e.value() as i32)
                .sum(),
        }
    }

    /// For U.
    fn level_requirement(&self) -> i32 {
        let mut item = self.levelreq_item(self.b);
        item.fillers = self
            .ctx
            .fillers
            .iter()
            .map(|f| self.levelreq_item(f))
            .collect();
        let unit = self.ctx.unit.map(|u| LevelReqUnit {
            class: u.class(),
            player: u.unit_type() == 0,
            expansion: u.expansion(),
        });
        level_requirement(&item, unit.as_ref())
    }

    /// Out flags (str_ok, dex_ok, lvl_ok) with "equipping"
    /// 0 (the equip requirement tests).
    fn requirement_tests(&self, bs: i32, bd: i32) -> (bool, bool, bool) {
        let Some(u) = self.ctx.unit else {
            return (false, false, false);
        };
        let s = u.stat(0, 0);
        let d = u.stat(2, 0);
        let str_ok = s >= 1 && s >= i32::from(self.rec.req_str) + bs;
        let dex_ok = d >= 1 && d >= i32::from(self.rec.req_dex) + bd;
        let lvl_ok = u.stat(12, 0) >= self.level_requirement();
        (str_ok, dex_ok, lvl_ok)
    }

    /// The requirement bonuses.
    fn req_bonuses(&self) -> (i32, i32) {
        let p = self.total(91);
        let (mut bs, mut bd) = (0, 0);
        if p != 0 {
            bs = muldiv(i32::from(self.rec.req_str), p, 100);
            bd = muldiv(i32::from(self.rec.req_dex), p, 100);
        }
        if self.b.flags & flag::ETHEREAL != 0 {
            bs -= 10;
            bd -= 10;
        }
        (bs, bd)
    }

    /// One requirement line.
    fn req_line(&self, id: u16, v: i32) -> Vec<u16> {
        if v <= 0 {
            return Vec::new();
        }
        [&self.s(id)[..], &self.sp(), &w(&v.to_string()), &self.nl()].concat()
    }

    fn class_line(&self, class: u8) -> Vec<u16> {
        [&self.s(tid::CLASS_ONLY + u16::from(class))[..], &self.nl()].concat()
    }

    fn durability(&self) -> Vec<u16> {
        let has = self.rec.nodurability == 0 && self.rec.durability != 0 && self.total(152) < 1;
        let m = self.total(73);
        if !has || m <= 0 || self.throwable() {
            return Vec::new();
        }
        let mut mx = w(&m.to_string());
        if self.bonus(75) != 0 {
            mx = pfx(mx, color::BLUE);
        }
        [
            &self.s(tid::DURABILITY)[..],
            &self.sp(),
            &w(&self.total(72).to_string()),
            &self.sp(),
            &self.s(tid::OF),
            &self.sp(),
            &mx,
            &self.nl(),
        ]
        .concat()
    }

    fn socket_filler(&self) -> Vec<u16> {
        let gem = self.is(ty::GEM);
        let rune = self.is(ty::RUNE);
        let mut t = Vec::new();
        if gem || rune {
            t.extend(self.nl());
            for (i, &(label, slot)) in tid::SLOT_LABELS.iter().enumerate() {
                let l = self.t.filler_list(self.b.code, slot);
                let label = [&self.s(label)[..], &self.sp()].concat();
                let args = props::PropArgs {
                    undead: false,
                    multi: rune,
                    label: &label,
                };
                let mut text = props::property_block(
                    self.t,
                    &self.viewer(),
                    &props::PropItem::default(),
                    &l,
                    &args,
                );
                if text.last() == Some(&u16::from(b'\n')) {
                    text.pop();
                }
                t.extend(text);
                t.extend(self.nl());
                if i == tid::SLOT_LABELS.len() - 1 {
                    t.extend(self.nl());
                }
            }
        }
        t.extend(self.s(tid::INSERT));
        t.extend(self.nl());
        t
    }

    fn quantity_spell(&self) -> Vec<u16> {
        let mut t = Vec::new();
        if self.identified() && self.b.flags & flag::SOCKETED == 0 {
            t.extend(self.quantity());
        }
        let r = &self.rec;
        if r.spelldesc != 0 && self.ctx.player.is_some() && r.spelldescstr != sid::EVIL {
            let str_ = self.s(r.spelldescstr);
            match (r.spelldesc, self.ctx.spell_value) {
                (1, _) => {
                    t.extend(str_);
                    t.extend(self.nl());
                }
                (2 | 3, Some(v)) => {
                    // TODO(item-tips): the stat1 adjustments
                    // Are not specified.
                    t.extend(str_);
                    t.extend(self.sp());
                    t.extend(w(&v.to_string()));
                    t.extend(self.nl());
                }
                (4, Some(v)) => {
                    t.extend(format(1024, Some(&str_), &[Arg::Int(v)]).unwrap_or_default());
                    t.extend(self.nl());
                }
                _ => {}
            }
        }
        t
    }

    fn quantity(&self) -> Vec<u16> {
        let q = self.total(70);
        if q > 0 || self.rec.maxstack > 0 {
            return [
                &self.s(tid::QUANTITY)[..],
                &self.sp(),
                &w(&q.to_string()),
                &self.nl(),
            ]
            .concat();
        }
        Vec::new()
    }

    /// `D(smin, smax)`: (min, max, blue).
    fn pair(&self, smin: u16, smax: u16) -> (i32, i32, bool) {
        if self.ctx.player.is_none() {
            return (1, 2, false);
        }
        let (min, mut max) = (self.total(smin), self.total(smax));
        let mut blue = self.base(smin) < min || self.base(smax) < max;
        max = max.max(min);
        if let Some(t) = self.ctx.act_time {
            let a = d2_sim::stats::by_time(self.l.get(272, 0), t);
            if self.l.get(272, 0) != 0 {
                max += a;
                blue |= a != 0;
            }
            if self.l.get(273, 0) != 0 {
                let p = d2_sim::stats::by_time(self.l.get(273, 0), t);
                max += muldiv(max, p, 100);
                blue |= p != 0;
            }
        }
        (min, max, blue)
    }

    fn damage_line(&self, label: u16, (min, max, blue): (i32, i32, bool)) -> Vec<u16> {
        let cut = |v: i32| {
            let mut s = w(&v.to_string());
            s.truncate(4);
            s
        };
        let mut t = pfx(self.s(label), color::WHITE);
        t.extend(self.sp());
        if blue {
            t = [t, w("\u{ff}c3")].concat();
        }
        t.extend(cut(min));
        t.extend(self.sp());
        t.extend(self.s(tid::TO));
        t.extend(self.sp());
        t.extend(cut(max));
        t.extend(self.nl());
        t
    }

    /// (thrown potions need the skill elemental calcs; left out).
    fn damage(&self) -> Vec<u16> {
        let mut t = Vec::new();
        if self.is(ty::TPOT) {
            // TODO(item-tips): the elemental calcs
            // … need the skill calc seam.
            return t;
        }
        let both = self
            .ctx
            .player
            .is_some_and(|p| p.one_or_two_handed(self.rec.one_or_two != 0));
        if both {
            t.extend(self.damage_line(tid::TWO_HAND, self.pair(23, 24)));
            t.extend(self.damage_line(tid::ONE_HAND, self.pair(21, 22)));
        } else {
            let (label, mut d) = if self.rec.twohanded != 0 {
                (tid::TWO_HAND, self.pair(23, 24))
            } else {
                (tid::ONE_HAND, self.pair(21, 22))
            };
            if d.1 <= d.0 {
                d.1 = d.0 + 1;
            }
            t.extend(self.damage_line(label, d));
        }
        if self.throwable() {
            let (min, max, b2) = self.pair(159, 160);
            let blue = [18, 17, 159, 160].iter().any(|&s| self.bonus(s) != 0) || b2;
            let k = if blue { color::BLUE } else { color::WHITE };
            t.extend(pfx(self.s(tid::THROW), color::WHITE));
            t.extend(self.sp());
            t.extend(pfx(w(&min.to_string()), k));
            t.extend(self.sp());
            t.extend(self.s(tid::TO));
            t.extend(self.sp());
            t.extend(pfx(w(&max.to_string()), k));
            t.extend(self.nl());
        }
        t
    }

    fn smite_kick(&self, unit_class: Option<u32>) -> Vec<u16> {
        let shield =
            self.is(ty::SHLD) && unit_class == Some(3) && matches!(self.item_class(), 0 | 3 | 7);
        let boots = self.is(ty::BOOT) && unit_class == Some(6);
        if !shield && !boots {
            return Vec::new();
        }
        let (mut min, mut max) = (i32::from(self.rec.mindam), i32::from(self.rec.maxdam));
        if let Some((a, b, _)) = self
            .ctx
            .player
            .filter(|p| p.has_state(101))
            .and_then(|p| p.holy_shield())
        {
            min += a;
            max += b;
        }
        let label = if shield { tid::SMITE } else { tid::KICK };
        [
            &self.s(label)[..],
            &self.sp(),
            &w(&min.to_string()),
            &self.sp(),
            &self.s(tid::TO),
            &self.sp(),
            &w(&max.to_string()),
            &self.nl(),
        ]
        .concat()
    }

    fn block(&self) -> Vec<u16> {
        let mut b = self.total(20);
        if let Some(p) = self.ctx.player {
            b += self.t.block_factor(p.class());
            if let Some((_, _, add)) = Some(p)
                .filter(|p| p.has_state(101))
                .and_then(|p| p.holy_shield())
            {
                b += add;
            }
        }
        b = b.min(75);
        if b == 0 {
            return Vec::new();
        }
        let mut n = format(64, Some(&w("%d%%")), &[Arg::Int(b), Arg::Int(0)]).unwrap_or_default();
        n.extend(self.nl());
        if b > i32::from(self.rec.block) {
            n = pfx(n, color::BLUE);
        }
        [pfx(self.s(tid::BLOCK), color::WHITE), n].concat()
    }

    fn defense(&self) -> Vec<u16> {
        let mut v = self.total(31);
        let mut blue = self.base(31) != v;
        if let Some(t) = self.ctx.act_time {
            let a = self.l.get(268, 0);
            if a != 0 {
                let x = d2_sim::stats::by_time(a, t);
                v += x;
                blue |= x != 0;
            }
            let p = self.l.get(269, 0);
            if p != 0 {
                let x = d2_sim::stats::by_time(p, t);
                v += muldiv(v, x, 100);
                blue |= x != 0;
            }
        }
        let mut n = w(&v.to_string());
        if blue {
            n = pfx(n, color::BLUE);
        }
        [&self.s(tid::DEFENSE)[..], &self.sp(), &n, &self.nl()].concat()
    }

    fn rune_letters(&self) -> Vec<u16> {
        let mut t = Vec::new();
        for f in &self.ctx.fillers {
            let Some(letters) = self.t.rune_letters(f.code) else {
                continue;
            };
            if t.is_empty() {
                t.extend(self.s(tid::QUOTE));
            }
            t.extend(letters);
        }
        if !t.is_empty() {
            t.extend(self.s(tid::QUOTE));
            t.extend(self.nl());
        }
        t
    }

    // ---------------------------------------------------------------

    pub(super) fn name_color(&self) -> u16 {
        let b = self.b;
        let mut c = match b.quality {
            4 => color::BLUE,
            5 => color::GREEN,
            6 => color::YELLOW,
            7 => color::GOLD,
            8 => color::ORANGE,
            9 => color::TEMPERED,
            1..=3 if b.flags & (flag::SOCKETED | flag::ETHEREAL) != 0 => color::GREY,
            _ => color::WHITE,
        };
        if !self.identified() && (1..=9).contains(&self.ctx.mode) && !self.ctx.own_item {
            c = color::WHITE;
        }
        if self.is(ty::RUNE) || SPECIAL.contains(&&b.code) {
            c = color::ORANGE;
        }
        if b.flags & flag::BROKEN != 0 {
            c = color::RED;
        }
        c
    }

    /// The text colour of a ground-item label (the Show Items drawer): magic 3,
    /// set 2, rare 9, unique 4, crafted 8, tempered 10; any other quality grey
    /// (5) when socketed or ethereal; a quest item other than `leg ` gold (4).
    /// The game's socketed test also reads the base item's socket capacity;
    /// the item flag alone is read here because every `hasinv` row of
    /// weapons.txt / armor.txt has a non-zero MaxSock1/25/40 on its item type
    /// (Open questions).
    pub(super) fn label_color(&self) -> u16 {
        let b = self.b;
        let mut c = match b.quality {
            4 => color::BLUE,
            5 => color::GREEN,
            6 => color::YELLOW,
            7 => color::GOLD,
            8 => color::ORANGE,
            9 => color::TEMPERED,
            _ if b.flags & (flag::SOCKETED | flag::ETHEREAL) != 0 => color::GREY,
            _ => color::WHITE,
        };
        let quest = self.t.codes.get(&b.code).is_some_and(|r| r.quest != 0);
        if quest && &b.code != b"leg " {
            c = color::GOLD;
        }
        c
    }

    /// Fills `%n` with `args`.
    fn fill(&self, template: u16, args: &[&[u16]]) -> Vec<u16> {
        let tpl = self.s(template);
        let sp = u16::from(b' ');
        let mut out: Vec<u16> = Vec::new();
        let mut i = 0;
        while i < tpl.len() {
            let u = tpl[i];
            let n = tpl.get(i + 1).copied().unwrap_or(0);
            if u == u16::from(b'%') && (u16::from(b'0')..=u16::from(b'9')).contains(&n) {
                if let Some(a) = args.get(usize::from(n - u16::from(b'0'))) {
                    out.extend_from_slice(a);
                }
                i += 2;
                if out.last() == Some(&sp) && tpl.get(i) == Some(&sp) {
                    i += 1;
                }
                continue;
            }
            out.push(u);
            i += 1;
        }
        out
    }

    /// `base`: `namestr` text without a leading grammar tag.
    fn base_name(&self) -> Vec<u16> {
        let t = self.s(self.rec.name_id);
        let close = t.iter().rposition(|&u| u == u16::from(b']'));
        match close {
            Some(k) if k >= 3 && t[k - 3] == u16::from(b'[') => t[k + 1..].to_vec(),
            _ => t,
        }
    }

    pub(super) fn name(&self) -> Vec<u16> {
        let b = self.b;
        let base = self.base_name();
        let nl = self.nl();
        let q = &b.quality_fields;
        let key = |names: &[String], i: Option<usize>| -> Vec<u16> {
            i.and_then(|i| self.t.key_text(names, i))
                .unwrap_or_default()
        };
        // A gold pile is named by its amount: "1234 Gold".
        if self.is(ty::GOLD) {
            if let Some(n) = b.gold {
                return [w(&n.to_string()), self.sp(), base].concat();
            }
        }
        let mut name = if b.flags & flag::RUNEWORD != 0 {
            let id = b.runeword.unwrap_or(tid::RUNEWORD_NONE);
            [base.clone(), nl.clone(), pfx(self.s(id), color::GOLD)].concat()
        } else if !self.identified() {
            base.clone()
        } else {
            match b.quality {
                1 => match q
                    .file_index
                    .and_then(|i| self.t.key_text(&self.t.low_quality, i as usize))
                {
                    Some(n) => self.fill(tid::NAME_LOW, &[&n, &base]),
                    None => Vec::new(),
                },
                2 => self.normal_name(&base),
                3 => self.fill(tid::NAME_SUPERIOR, &[&self.s(tid::SUPERIOR), &base]),
                4 => {
                    let (p, s) = q.magic.unwrap_or((0, 0));
                    let pre = self
                        .t
                        .magic_name(&self.t.magic_prefix, p)
                        .unwrap_or_default();
                    let suf = self
                        .t
                        .magic_name(&self.t.magic_suffix, s)
                        .unwrap_or_default();
                    self.fill(tid::NAME_MAGIC, &[&w(&pre), &base, &w(&suf)])
                }
                5 => match q.file_index.filter(|&i| (i as usize) < self.t.set.len()) {
                    None => Vec::new(),
                    Some(i) => {
                        let n = key(&self.t.set, Some(i as usize));
                        [base.clone(), nl.clone(), self.fill(tid::NAME_SET, &[&n])].concat()
                    }
                },
                7 => match q.file_index.filter(|&i| (i as usize) < self.t.unique.len()) {
                    None => base.clone(),
                    Some(i) => {
                        let mut t = Vec::new();
                        if self.rec.skipname == 0 {
                            t.extend(base.clone());
                            t.extend(nl.clone());
                        }
                        t.extend(key(&self.t.unique, Some(i as usize)));
                        t
                    }
                },
                6 | 8 | 9 => {
                    let (a, c) = q.rare_names.unwrap_or((0, 0));
                    let pre = self.t.rare_name(a, true).unwrap_or_default();
                    let suf = self.t.rare_name(c, false).unwrap_or_default();
                    [
                        base.clone(),
                        nl.clone(),
                        self.fill(tid::NAME_RARE, &[&w(&pre), &w(&suf)]),
                    ]
                    .concat()
                }
                // Edge case 5: a fatal error in the game; no name.
                _ => Vec::new(),
            }
        };
        // r6: quest items.
        if self.rec.quest != 0 {
            if self.rec.questdiffcheck != 0 && self.total(356) < i32::from(self.ctx.difficulty) {
                name = pfx(name, color::RED);
            } else if &b.code != b"leg " {
                name = pfx(name, color::GOLD);
            }
        }
        name
    }

    fn normal_name(&self, base: &[u16]) -> Vec<u16> {
        let b = self.b;
        if self.is(ty::SCRO) || self.is(ty::BOOK) {
            let tome = self.primary() == ty::BOOK;
            return match scroll_spell(&self.t.lookup.books, b) {
                0 => self.s(if tome { tid::TP_TOME } else { tid::TP_SCROLL }),
                1 => self.s(if tome { tid::ID_TOME } else { tid::ID_SCROLL }),
                _ => Vec::new(),
            };
        }
        if self.is(ty::PLAY) {
            if let Some(ear) = &b.ear {
                let mut t = Vec::new();
                if b.flags & flag::HARDCORE != 0 {
                    t.extend(self.s(tid::HARDCORE));
                    t.extend(self.nl());
                }
                t.extend(self.s(tid::LEVEL));
                t.extend(self.sp());
                t.extend(w(&ear.level.to_string()));
                t.extend(self.nl());
                t.extend(self.t.class_name(u32::from(ear.class)));
                t.extend(self.nl());
                t.extend(possessive(&ear.name, base));
                return t;
            }
        }
        if self.is(ty::BODY) {
            let id = b
                .quality_fields
                .file_index
                .and_then(|i| self.t.monstats.get(i as usize).copied());
            let n = id.map(|i| self.s(i)).unwrap_or_default();
            return self.fill(tid::NAME_BODY, &[&n, base]);
        }
        if b.flags & flag::SOCKETED != 0 && !self.ctx.fillers.is_empty() {
            return self.fill(tid::NAME_GEMMED, &[&self.s(tid::GEMMED), base]);
        }
        base.to_vec()
    }

    // ---------------------------------------------------------------------

    /// The store lines appended to `text` (modes 1–9).
    fn store_lines(&self, mut text: Vec<u16>, repair_note: bool) -> Vec<u16> {
        if !(1..=9).contains(&self.ctx.mode) {
            return text;
        }
        let nl = self.nl();
        if repair_note && self.ctx.mode == 4 && self.b.flags & flag::ETHEREAL != 0 {
            text.extend(&nl);
            text.extend(pfx(self.s(tid::CANNOT_REPAIR), color::RED));
        }
        match self.price_text() {
            Some(p) if !p.is_empty() => {
                text.extend(&nl);
                text.extend(p);
            }
            Some(_) => {}
            None if self.ctx.mode != 4 => {
                text.extend(&nl);
                text.extend(pfx(self.s(tid::NOT_TRADED), color::RED));
            }
            None => {}
        }
        text
    }

    /// `None` on failure.
    fn price_text(&self) -> Option<Vec<u16>> {
        if self.primary() == 4 {
            return None;
        }
        match self.ctx.price {
            PriceText::Fail => None,
            PriceText::Empty => Some(Vec::new()),
            PriceText::RepairUnidentified => Some(pfx(self.s(tid::REPAIR_UNID), color::RED)),
            PriceText::Price { label, price } => {
                let mut t = self.s(label);
                if t.len() + 10 > 64 {
                    return None;
                }
                t.extend(w(&price.to_string()));
                Some(t)
            }
        }
    }

    /// The property block of the item (B).
    fn properties(&self, undead: bool) -> Vec<u16> {
        let item = props::PropItem {
            elixir: (self.primary() == ty::ELIX).then(|| {
                (
                    self.b.quality_fields.file_index.unwrap_or(0),
                    self.total(71),
                )
            }),
            blunt: self.is(ty::BLUN),
            undead_stat: self.total(122),
            indestructible: self.rec.nodurability == 0
                && self.rec.durability != 0
                && self.total(152) < 1
                && self.total(73) == 0,
        };
        let args = props::PropArgs {
            undead,
            multi: true,
            label: &[],
        };
        props::property_block(self.t, &self.viewer(), &item, &self.l, &args)
    }

    /// The whole tip.
    pub(super) fn tip(&self) -> TipText {
        let store = (1..=9).contains(&self.ctx.mode);
        if store && !self.ctx.own_item && self.ctx.gamble {
            return self.gamble_tip();
        }
        if self.ctx.cursor_mode == 8 || self.rec.transmogrify != 0 {
            return self.transmogrify_tip();
        }
        if self.b.quality == 5 && self.identified() {
            return super::item_tip_set::set_tip(self);
        }
        if self.primary() == ty::BOOK {
            return self.tome_tip();
        }
        self.main_tip()
    }

    fn main_tip(&self) -> TipText {
        let weap = self.is(ty::WEAP);
        let armo = self.is(ty::ARMO);
        let ident = self.identified();
        let mut t = Vec::new();
        if weap || armo {
            t.extend(pfx(self.ethereal_sockets(), color::BLUE));
        }
        if ident {
            t.extend(pfx(self.properties(true), color::BLUE));
        } else {
            t.extend(pfx(
                [self.s(tid::UNIDENTIFIED), self.nl()].concat(),
                color::RED,
            ));
        }
        if weap {
            t.extend(pfx(self.speed_line(), color::WHITE));
        }
        let (bs, bd) = self.req_bonuses();
        let (str_ok, dex_ok, lvl_ok) = self.requirement_tests(bs, bd);
        let red = |ok: bool| if ok { color::WHITE } else { color::RED };
        if ident {
            let r = self.level_requirement();
            if r > 1 {
                t.extend(pfx(self.req_line(tid::REQ_LEVEL, r), red(lvl_ok)));
            }
        }
        if (weap || armo) && self.rec.req_str != 0 {
            let v = i32::from(self.rec.req_str) + bs;
            t.extend(pfx(self.req_line(tid::REQ_STR, v), red(str_ok)));
        }
        if (weap || armo) && self.rec.req_dex != 0 {
            let v = i32::from(self.rec.req_dex) + bd;
            t.extend(pfx(self.req_line(tid::REQ_DEX, v), red(dex_ok)));
        }
        let class = self.item_class();
        let unit_class = self.ctx.unit.map(|u| u.class());
        if class != 7 {
            let k = if unit_class == Some(u32::from(class)) {
                color::WHITE
            } else {
                color::RED
            };
            t.extend(pfx(self.class_line(class), k));
        }
        if !self.quiver() {
            t.extend(pfx(self.durability(), color::WHITE));
        }
        if self.is(ty::SOCK) {
            t.extend(pfx(self.socket_filler(), color::WHITE));
        }
        if self.is(ty::CHAR) {
            t.extend(pfx([self.s(tid::CHARM), self.nl()].concat(), color::WHITE));
        }
        t.extend(pfx(self.quantity_spell(), color::WHITE));
        if weap {
            t.extend(pfx(self.damage(), color::WHITE));
        }
        t.extend(pfx(self.smite_kick(unit_class), color::WHITE));
        if self.is(ty::SHLD) {
            t.extend(pfx(self.block(), color::WHITE));
        }
        if armo && self.total(31) > 0 {
            t.extend(pfx(self.defense(), color::WHITE));
        }
        if self.rec.hasinv != 0 {
            t.extend(pfx(self.rune_letters(), color::GOLD));
        }
        t.extend(pfx(self.name(), self.name_color()));
        let mut text = self.store_lines(t, true);
        let mut pop = color::WHITE;
        if self.rec.quest != 0 && &self.b.code != b"leg " {
            if self.ctx.mode == 0 && self.b.mode == 0 {
                let id = match &self.b.code {
                    b"bkd " => Some(tid::READ),
                    b"box " => Some(tid::OPEN),
                    _ => None,
                };
                if let Some(id) = id {
                    text = [self.s(id), self.nl(), text].concat();
                }
            }
            text = pfx(text, color::GOLD);
            pop = color::GOLD;
        }
        text.truncate(1023);
        TipText { text, color: pop }
    }

    fn tome_tip(&self) -> TipText {
        let mut t = self.quantity();
        if self.ctx.mode == 0 {
            t.extend(self.s(tid::USE));
            t.extend(self.nl());
            t.extend(self.s(tid::INSERT_SCROLLS));
            t.extend(self.nl());
        }
        let mut n = self.name();
        n.truncate(127);
        t.extend(n);
        TipText {
            text: self.store_lines(t, false),
            color: color::WHITE,
        }
    }

    /// (no stock item reaches it; the target row's name only).
    fn transmogrify_tip(&self) -> TipText {
        let target = self
            .t
            .codes
            .get(&self.rec.tmogtype)
            .map(|c| self.s(c.name_id))
            .unwrap_or_default();
        let mut t = target;
        t.extend(self.nl());
        t.extend(self.s(5387));
        t.extend(self.nl());
        t.extend(self.name());
        TipText {
            text: self.store_lines(t, false),
            color: color::WHITE,
        }
    }

    fn gamble_tip(&self) -> TipText {
        let mut t = pfx(
            [self.s(tid::UNIDENTIFIED), self.nl()].concat(),
            color::WHITE,
        );
        let p = self.primary();
        if let Some(&(class, _)) = GAMBLE_CLASS.iter().find(|(_, ts)| ts.contains(&p)) {
            t.extend(pfx(self.class_line(class as u8), color::WHITE));
        }
        let mut n = self.name();
        n.truncate(127);
        t.extend(pfx(n, color::WHITE));
        TipText {
            text: self.store_lines(t, false),
            color: color::WHITE,
        }
    }

    // ------------------------------------------------------------ set tips

    pub(super) fn tips(&self) -> &ItemTips {
        self.t
    }
    pub(super) fn bits(&self) -> &ItemBits {
        self.b
    }
    pub(super) fn ctx(&self) -> &TipCtx<'a> {
        self.ctx
    }
    pub(super) fn view(&self) -> Viewer<'_> {
        self.viewer()
    }
    pub(super) fn own_props(&self) -> Vec<u16> {
        self.properties(true)
    }
    pub(super) fn eth_sockets(&self) -> Vec<u16> {
        if self.b.flags & flag::SOCKETED != 0 {
            self.ethereal_sockets()
        } else {
            Vec::new()
        }
    }

    /// The base block, bottom first (E, F, G, H, I, D, M, N, P,
    /// O, name).
    pub(super) fn set_base_block(&self, u_player_other_class: bool) -> Vec<u16> {
        let weap = self.is(ty::WEAP);
        let armo = self.is(ty::ARMO);
        let (bs, bd) = self.req_bonuses();
        let (str_ok, dex_ok, lvl_ok) = self.requirement_tests(bs, bd);
        let red = |ok: bool| if ok { color::WHITE } else { color::RED };
        let mut t = Vec::new();
        let r = self.level_requirement();
        if r > 1 {
            t.extend(pfx(self.req_line(tid::REQ_LEVEL, r), red(lvl_ok)));
        }
        if (weap || armo) && self.rec.req_str != 0 {
            let v = i32::from(self.rec.req_str) + bs;
            t.extend(pfx(self.req_line(tid::REQ_STR, v), red(str_ok)));
        }
        if (weap || armo) && self.rec.req_dex != 0 {
            let v = i32::from(self.rec.req_dex) + bd;
            t.extend(pfx(self.req_line(tid::REQ_DEX, v), red(dex_ok)));
        }
        let class = self.item_class();
        if class != 7 {
            let k = if u_player_other_class {
                color::RED
            } else {
                color::WHITE
            };
            t.extend(pfx(self.class_line(class), k));
        }
        if !self.quiver() {
            t.extend(pfx(self.durability(), color::WHITE));
        }
        if weap {
            t.extend(pfx(self.speed_line(), color::WHITE));
            t.extend(pfx(self.damage(), color::WHITE));
        }
        let unit_class = self.ctx.unit.map(|u| u.class());
        if self.is(ty::SHLD) && unit_class == Some(3) {
            t.extend(pfx(self.smite_kick(unit_class), color::WHITE));
        }
        if self.is(ty::SHLD) {
            t.extend(pfx(self.block(), color::WHITE));
        }
        if armo && self.total(31) > 0 {
            t.extend(pfx(self.defense(), color::WHITE));
        }
        let k = if self.b.flags & flag::BROKEN != 0 {
            color::RED
        } else {
            color::GREEN
        };
        t.extend(pfx(self.name(), k));
        t
    }

    /// The store price lines of a set item.
    pub(super) fn set_store_lines(&self, mut text: Vec<u16>) -> Vec<u16> {
        if !(1..=9).contains(&self.ctx.mode) {
            return text;
        }
        let nl = self.nl();
        match self.price_text() {
            Some(p) if !p.is_empty() => {
                text.extend(&nl);
                text.extend(p);
            }
            Some(_) => {}
            None if self.ctx.mode != 4 => {
                text.extend(&nl);
                text.extend(pfx(self.s(tid::NOT_TRADED), color::RED));
            }
            None => {}
        }
        text
    }
}

#[cfg(test)]
mod possessive_tests {
    use super::*;

    #[test]
    fn the_english_possessive_follows_the_last_letter() {
        assert_eq!(possessive(b"Bob", &w("Ear")), w("Bob's Ear"));
        assert_eq!(possessive(b"Chris", &w("Ear")), w("Chris' Ear"));
        // Only a lower-case s ends a name in the set.
        assert_eq!(possessive(b"CHRIS", &w("Ear")), w("CHRIS's Ear"));
    }

    #[test]
    fn a_text_that_does_not_fit_is_kept_alone() {
        let text = vec![u16::from(b'x'); 509];
        assert_eq!(possessive(b"Bob", &text), text);
        let full = vec![u16::from(b'x'); 512];
        assert!(possessive(b"Bob", &full).is_empty());
    }
}
