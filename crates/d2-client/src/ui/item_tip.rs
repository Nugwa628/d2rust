//! Item tool tips: the tables and strings of an item's hover text
//! ([`ItemTips`]), the text built by [`super::item_tip_build`]
//! ([`ItemTips::tip`]) and its lines for the box ([`ItemTips::tip_lines`],
//! [`draw_tip`]).
//!
//! The text is built from the item's last item stream from the server,
//! decoded with `d2_proto::item_bits::decode` over the game's item tables.
//! The box is the UI's pop-up text box, placed at the item's hover anchor
//! ([`draw_tip_at`]). A mod game's item hooks (`crate::mods::ItemLines`)
//! run over the finished lines ([`ItemTips::tip_lines_of`]), and its label
//! filters over ground item labels ([`ItemTips::label_of`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::bin::BinSet;
use d2_data::tables::{
    decode_all, Armor, Charstats, Gems, Itemstatcost, Lowqualityitems, Magicprefix, Magicsuffix,
    Misc, Monstats, Montype, Rareprefix, Raresuffix, Record, Setitems, Sets, Skilldesc, Skills,
    Uniqueitems, Weapons,
};
use d2_proto::item_bits::{decode, ItemBits, Stat};
use d2_server::adapters::item_bits::TablesLookup;
use d2_sim::items::ItemTables;

use super::draw::UiDrawSink;
use super::geom::Point;
use super::item_tip_build::{Build, ItemText, PriceText, TipCtx, TipText};
use super::item_tip_desc::{self as desc, StatDesc};
use super::item_tip_props::StatList;
use super::original::FontMeasure;
use super::panel::StringLookup;
use super::panels::UiFiles;
use crate::mods::{ClientMods, ItemLabel};

/// Text colour indexes (`ÿc` codes).
pub mod color {
    pub const WHITE: u16 = 0;
    pub const RED: u16 = 1;
    pub const GREEN: u16 = 2;
    pub const BLUE: u16 = 3;
    pub const GOLD: u16 = 4;
    pub const GREY: u16 = 5;
    pub const ORANGE: u16 = 8;
    pub const YELLOW: u16 = 9;
    /// `ÿc:` (tempered names).
    pub const TEMPERED: u16 = 10;
}

/// The font of the pop-up (Font16, `hud_tips::TIP_FONT`).
const FONT: u16 = 1;

/// One line of a tip: UTF-16 text and its colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipLine {
    pub text: Vec<u16>,
    pub color: u16,
}

impl TipLine {
    pub fn new(s: impl AsRef<str>, color: u16) -> Self {
        TipLine {
            text: s.as_ref().encode_utf16().collect(),
            color,
        }
    }
}

/// What the tips read from the tables and strings.
#[derive(Clone)]
pub struct ItemTips {
    pub(super) lookup: Arc<ItemTables>,
    pub(super) codes: BTreeMap<[u8; 4], ItemText>,
    pub(super) stats: Vec<StatDesc>,
    /// The description list.
    pub(super) desc_order: Vec<u16>,
    /// String-table keys by table row.
    pub(super) magic_prefix: Vec<String>,
    pub(super) magic_suffix: Vec<String>,
    pub(super) rare_prefix: Vec<String>,
    pub(super) rare_suffix: Vec<String>,
    pub(super) unique: Vec<String>,
    pub(super) set: Vec<String>,
    pub(super) low_quality: Vec<String>,
    /// Each set's name string id.
    pub(super) set_names: Vec<u16>,
    /// Per skill: its skilldesc name string id (`None`: no skilldesc
    /// row) and its `charclass` (255: none).
    pub(super) skills: Vec<(Option<u16>, u8)>,
    /// Per charstats row: the class strings, the
    /// `BlockFactor` and the class name.
    pub(super) class_strings: Vec<desc::ClassStrings>,
    pub(super) block_factors: Vec<u8>,
    pub(super) class_names: Vec<String>,
    /// Montype `strplur` and monstats `NameStr` per row.
    pub(super) montype: Vec<u16>,
    pub(super) monstats: Vec<u16>,
    /// Gems `letter` per gems row.
    pub(super) gem_letters: Vec<String>,
    /// `transformcolor` of each magic prefix / suffix row, and
    /// `invtransform` of each set / unique row.
    pub(super) prefix_color: Vec<u8>,
    pub(super) suffix_color: Vec<u8>,
    pub(super) set_inv_color: Vec<u8>,
    pub(super) unique_inv_color: Vec<u8>,
    pub(super) strings: Arc<dyn StringLookup + Send + Sync>,
    /// The mod game's client hooks over the tips (`crate::mods`); empty in
    /// D2RUST's own game.
    pub(crate) mods: ClientMods,
}

impl std::fmt::Debug for ItemTips {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ItemTips({} codes)", self.codes.len())
    }
}

fn key(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn rows<T: Record>(set: &BinSet) -> Result<Vec<T>, String> {
    let t = set
        .table(T::TABLE)
        .ok_or_else(|| format!("{} not loaded", T::TABLE))?;
    decode_all(t).map_err(|e| e.to_string())
}

impl ItemTips {
    /// The item tables the tips decode with (shared with the model's
    /// item lists, `bridge::item_lists`).
    pub fn tables(&self) -> Arc<ItemTables> {
        self.lookup.clone()
    }

    /// The tips over `lookup` (the game's item tables, for the stream
    /// reader), the loaded tables `set` and `strings`.
    pub fn new(
        lookup: Arc<ItemTables>,
        set: &BinSet,
        strings: Arc<dyn StringLookup + Send + Sync>,
    ) -> Result<Self, String> {
        let mut codes = BTreeMap::new();
        macro_rules! add {
            ($t:ty) => {
                for r in rows::<$t>(set)? {
                    codes.entry(r.code).or_insert(ItemText {
                        name_id: r.namestr,
                        req_str: r.reqstr,
                        req_dex: r.reqdex,
                        levelreq: r.levelreq,
                        block: r.block,
                        mindam: r.mindam,
                        maxdam: r.maxdam,
                        mindam2: r.f_2handmindam,
                        maxdam2: r.f_2handmaxdam,
                        minmisdam: r.minmisdam,
                        maxmisdam: r.maxmisdam,
                        durability: r.durability,
                        nodurability: r.nodurability,
                        quest: r.quest,
                        questdiffcheck: r.questdiffcheck,
                        skipname: r.skipname,
                        gemoffset: r.gemoffset,
                        spelldesc: r.spelldesc,
                        spelldescstr: r.spelldescstr,
                        stat1: r.stat1,
                        hasinv: r.hasinv,
                        twohanded: r.f_2handed,
                        one_or_two: r.f_1or2handed,
                        transmogrify: r.transmogrify,
                        tmogtype: r.tmogtype,
                        maxstack: r.maxstack,
                        wclass: r.wclass,
                        wclass2: r.f_2handedwclass,
                        inv_trans: r.invtrans,
                    });
                }
            };
        }
        add!(Weapons);
        add!(Armor);
        add!(Misc);
        let desc_order = set
            .table(Itemstatcost::TABLE)
            .map(d2_data::fixup::maps::desc_list)
            .unwrap_or_default();
        let stats = rows::<Itemstatcost>(set)?
            .iter()
            .map(|r| StatDesc {
                priority: r.descpriority,
                func: r.descfunc,
                val: r.descval,
                pos: r.descstrpos,
                neg: r.descstrneg,
                str2: r.descstr2,
                dgrp: r.dgrp,
                dgrpfunc: r.dgrpfunc,
                dgrpval: r.dgrpval,
                dgrppos: r.dgrpstrpos,
                dgrpneg: r.dgrpstrneg,
                dgrpstr2: r.dgrpstr2,
                op: r.op,
                op_param: r.op_param,
                op_base: r.op_base,
                valshift: r.valshift,
                op_stats: [r.op_stat1, r.op_stat2, r.op_stat3],
            })
            .collect();
        Ok(ItemTips {
            lookup,
            codes,
            stats,
            desc_order,
            magic_prefix: rows::<Magicprefix>(set)?
                .iter()
                .map(|r| key(&r.name))
                .collect(),
            magic_suffix: rows::<Magicsuffix>(set)?
                .iter()
                .map(|r| key(&r.name))
                .collect(),
            rare_prefix: rows::<Rareprefix>(set)?
                .iter()
                .map(|r| key(&r.name))
                .collect(),
            rare_suffix: rows::<Raresuffix>(set)?
                .iter()
                .map(|r| key(&r.name))
                .collect(),
            unique: rows::<Uniqueitems>(set)?
                .iter()
                .map(|r| key(&r.index))
                .collect(),
            set: rows::<Setitems>(set)?
                .iter()
                .map(|r| key(&r.index))
                .collect(),
            low_quality: rows::<Lowqualityitems>(set)?
                .iter()
                .map(|r| key(&r.name))
                .collect(),
            set_names: rows::<Sets>(set)?.iter().map(|r| r.name).collect(),
            skills: {
                let descs = rows::<Skilldesc>(set)?;
                rows::<Skills>(set)?
                    .iter()
                    .map(|r| {
                        let name = descs.get(usize::from(r.skilldesc)).map(|d| d.str_name);
                        (name, r.charclass)
                    })
                    .collect()
            },
            class_strings: rows::<Charstats>(set)?
                .iter()
                .map(|r| desc::ClassStrings {
                    all_skills: r.strallskills,
                    tabs: [r.strskilltab1, r.strskilltab2, r.strskilltab3],
                    class_only: r.strclassonly,
                })
                .collect(),
            block_factors: rows::<Charstats>(set)?
                .iter()
                .map(|r| r.blockfactor)
                .collect(),
            class_names: rows::<Charstats>(set)?
                .iter()
                .map(|r| key(&r.class))
                .collect(),
            gem_letters: rows::<Gems>(set)?.iter().map(|r| key(&r.letter)).collect(),
            prefix_color: rows::<Magicprefix>(set)?
                .iter()
                .map(|r| r.transformcolor)
                .collect(),
            suffix_color: rows::<Magicsuffix>(set)?
                .iter()
                .map(|r| r.transformcolor)
                .collect(),
            set_inv_color: rows::<Setitems>(set)?
                .iter()
                .map(|r| r.invtransform)
                .collect(),
            unique_inv_color: rows::<Uniqueitems>(set)?
                .iter()
                .map(|r| r.invtransform)
                .collect(),
            montype: rows::<Montype>(set)?.iter().map(|r| r.strplur).collect(),
            monstats: rows::<Monstats>(set)?.iter().map(|r| r.namestr).collect(),
            strings,
            mods: ClientMods::default(),
        })
    }

    fn text_by_key(&self, names: &[String], i: usize) -> Option<String> {
        let k = names.get(i)?;
        let t = self.strings.get(k)?;
        Some(String::from_utf16_lossy(t))
    }

    /// The inventory picture's item colour `(t, c)` (the item's inventory
    /// colour, shading rule):
    /// `t` = the code's `InvTrans`, `c` by quality: magic / rare affix
    /// `transformcolor` (suffix slots, then prefix slots; 0xFF none), set /
    /// unique `invtransform`; then [`crate::rules::shading::item_color`].
    /// None: no map. Not read (D2RUST's own, unverified: the model holds
    /// neither): the automagic affix and the socketed gem `transform` of
    /// qualities 1, 2, 3, 8, which give no map here.
    pub fn inv_color(&self, stream: &[u8]) -> Option<(u8, u8)> {
        self.inv_color_of(&self.bits(stream)?)
    }

    pub(super) fn inv_color_of(&self, b: &ItemBits) -> Option<(u8, u8)> {
        let t = self.codes.get(&b.code)?.inv_trans;
        let qf = &b.quality_fields;
        // Affix id = row + 1 (0: none, [`Self::magic_name`]).
        let affix = |table: &[u8], id: u16| {
            let row = usize::from(id).checked_sub(1)?;
            table.get(row).copied().filter(|&c| c != 0xFF)
        };
        // A set / unique item not shown (unidentified) sends no file index: the
        // client's item data keeps 0, so the colour is row 0's.
        let row = |table: &[u8]| {
            let i = qf.file_index.unwrap_or(0);
            table.get(usize::try_from(i).ok()?).copied()
        };
        let c = match b.quality {
            d2_sim::items::q::MAGIC => {
                let (p, x) = qf.magic?;
                affix(&self.suffix_color, x).or_else(|| affix(&self.prefix_color, p))
            }
            d2_sim::items::q::RARE => {
                let slots = qf.rare_slots?;
                slots
                    .iter()
                    .find_map(|&(_, x)| affix(&self.suffix_color, x))
                    .or_else(|| {
                        slots
                            .iter()
                            .find_map(|&(p, _)| affix(&self.prefix_color, p))
                    })
            }
            d2_sim::items::q::SET => row(&self.set_inv_color),
            d2_sim::items::q::UNIQUE => row(&self.unique_inv_color),
            _ => None,
        }?;
        crate::rules::shading::item_color(t, c)
    }

    /// The name of magic affix id `id` as sent (prefixes arrive as
    /// their magicprefix row +
    /// 1, suffixes as their magicsuffix row + 1; 0 = none).
    pub(super) fn magic_name(&self, names: &[String], id: u16) -> Option<String> {
        let row = usize::from(id).checked_sub(1)?;
        self.text_by_key(names, row)
    }

    /// The rare name of rare id `id` (combined index + 1 in the rare array,
    /// raresuffix rows first, then rareprefix; sent unchanged). `prefix` picks
    /// the part the slot holds; an id outside that part is none.
    pub(super) fn rare_name(&self, id: u8, prefix: bool) -> Option<String> {
        let combined = usize::from(id).checked_sub(1)?;
        let n = self.rare_suffix.len();
        if prefix {
            self.text_by_key(&self.rare_prefix, combined.checked_sub(n)?)
        } else if combined < n {
            self.text_by_key(&self.rare_suffix, combined)
        } else {
            None
        }
    }

    /// The text of the string with key `names[i]`.
    pub(super) fn key_text(&self, names: &[String], i: usize) -> Option<Vec<u16>> {
        Some(self.strings.get(names.get(i)?)?.to_vec())
    }

    /// Charstats `BlockFactor` of `class`.
    pub(super) fn block_factor(&self, class: u32) -> i32 {
        self.block_factors
            .get(class as usize)
            .map_or(0, |&b| i32::from(b))
    }

    /// The class name of an ear (not specified: the charstats `class` text).
    pub(super) fn class_name(&self, class: u32) -> Vec<u16> {
        self.class_names
            .get(class as usize)
            .map(|s| s.encode_utf16().collect())
            .unwrap_or_default()
    }

    /// The gems-row letters of a rune with `code`: is-a 74 with
    /// `gemoffset` > 0.
    pub(super) fn rune_letters(&self, code: [u8; 4]) -> Option<Vec<u16>> {
        let i = self.lookup.find_code(code)?;
        if !self.lookup.is_type(i, d2_sim::items::ty::RUNE as i16) {
            return None;
        }
        let g = self.codes.get(&code)?.gemoffset;
        if g == 0 {
            return None;
        }
        let l = self.gem_letters.get(g as usize)?;
        Some(l.encode_utf16().collect())
    }

    /// The stats whose op 13 (% of an item's own base) targets `s`.
    pub(super) fn op13_sources(&self, s: u16) -> Vec<(u16, StatDesc)> {
        self.stats
            .iter()
            .enumerate()
            .filter(|(_, d)| d.op == 13 && d.op_stats.contains(&s) && s != 0)
            .map(|(i, d)| (i as u16, *d))
            .collect()
    }

    /// The list a gem / rune of `code` gives through its gems-row
    /// mods block `slot` (the mods built on a temporary list; the mods are
    /// fixed values, so the roll seed does not matter). The probe is a
    /// expansion item (format 101): a format-0
    /// item would take the legacy property table.
    pub(super) fn filler_list(&self, code: [u8; 4], slot: usize) -> StatList {
        let mut l = StatList::default();
        let Some(record) = self.lookup.find_code(code) else {
            return l;
        };
        let mut item = d2_sim::items::Item {
            record,
            format: d2_sim::items::FORMAT_EXPANSION,
            ilvl: 0,
            quality: 2,
            file_index: -1,
            prefix: [0; 3],
            suffix: [0; 3],
            rare_prefix: 0,
            rare_suffix: 0,
            auto_affix: 0,
            flags: 0,
            inv_page: 0xFF,
            gfx: 0,
            unit_seed: d2_sim::rng::Seed { lo: 0, hi: 0 },
            init_seed: 0,
            item_seed: d2_sim::rng::Seed { lo: 0, hi: 0 },
            start_seed: 0,
            name: [0; 16],
            ear_level: 0,
            realm_data: [0; 2],
            ground_expiry: 0,
            fatal: None,
            stats: Recorder::default(),
        };
        d2_sim::items::props::apply_socket_filler(&self.lookup, &mut item, slot as u8);
        for (&(s, layer), &v) in &item.stats.0 {
            l.add(s, u32::from(layer), v);
        }
        l
    }

    /// The decoded record of an item stream (`None` when it does not decode, or
    /// decodes as a failed record: no item is made of it).
    pub fn bits(&self, stream: &[u8]) -> Option<ItemBits> {
        decode(stream, &TablesLookup(&self.lookup))
            .ok()
            .filter(|b| !b.failed)
    }

    /// D2RUST's own, unverified (the game's filler-type rule is not known): the item with
    /// `code` is a gem (20), rune (74) or jewel (58), as the server's
    /// socket-filler test.
    pub fn is_socket_filler(&self, code: [u8; 4]) -> bool {
        use d2_sim::items::ty;
        self.lookup.find_code(code).is_some_and(|i| {
            [ty::GEM, ty::RUNE, ty::JEWL]
                .into_iter()
                .any(|t| self.lookup.is_type(i, t as i16))
        })
    }

    /// The item the builder reads for decoded record `b`.
    fn as_built(b: &ItemBits) -> std::borrow::Cow<'_, ItemBits> {
        use std::borrow::Cow;
        // The item stream reader: an alt-code record is the base
        // code's item with item level 1 and quality 1; the flags lose
        // 0x2000000 and 0x80000 (edge case 9: a gamble item shows its
        // normal-tier base).
        if let Some(code) = b.base_code {
            return Cow::Owned(ItemBits {
                code,
                ilvl: 1,
                quality: 1,
                flags: b.flags & !(d2_proto::item_bits::hflag::ALT_CODE | 0x8_0000),
                ..b.clone()
            });
        }
        // A compact record carries no quality or item level; the compact reader
        // ends by storing item level 1 and quality 2 (normal).
        if b.flags & d2_proto::item_bits::hflag::COMPACT != 0 && b.quality == 0 {
            return Cow::Owned(ItemBits {
                ilvl: 1,
                quality: 2,
                ..b.clone()
            });
        }
        Cow::Borrowed(b)
    }

    /// The tip text of decoded item `b` with
    /// `ctx`.
    pub fn tip_of(&self, b: &ItemBits, ctx: &TipCtx<'_>) -> TipText {
        Build::new(self, &Self::as_built(b), ctx).tip()
    }

    /// The two parts of the belt's hover text of the item with last stream
    /// `stream`: the client item name and its property lines (the stat
    /// text with 0x100 units, multi-line, no label). `None` when the stream does
    /// not decode.
    pub fn name_and_properties(
        &self,
        stream: &[u8],
        ctx: &TipCtx<'_>,
    ) -> Option<(Vec<u16>, Vec<u16>)> {
        let b = decode(stream, &TablesLookup(&self.lookup)).ok()?;
        let b = Self::as_built(&b);
        let build = Build::new(self, &b, ctx);
        Some((build.name(), build.own_props()))
    }

    /// The ground-item label of the item with last stream `stream`: the
    /// item's name as the client shows it and the label's text colour,
    /// after the mod game's label filters (`crate::mods::ItemNameFilter`).
    /// `None` when the stream does not decode or a filter hides the label.
    pub fn label_name(&self, stream: &[u8], ctx: &TipCtx<'_>) -> Option<(Vec<u16>, u16)> {
        let b = decode(stream, &TablesLookup(&self.lookup)).ok()?;
        self.label_of(&b, ctx)
    }

    /// [`Self::label_name`] of decoded item `b`.
    pub fn label_of(&self, b: &ItemBits, ctx: &TipCtx<'_>) -> Option<(Vec<u16>, u16)> {
        let built = Self::as_built(b);
        let build = Build::new(self, &built, ctx);
        let (text, color) = (build.name(), build.label_color());
        if !self.mods.has_item_name_filter() {
            return Some((text, color));
        }
        let mut label = ItemLabel {
            text,
            color,
            shown: true,
        };
        self.mods.filter_item_name(b, &mut label);
        label.shown.then_some((label.text, label.color))
    }

    /// The tip text of the item with last stream `stream`; empty when the
    /// stream does not decode.
    pub fn tip(&self, stream: &[u8], ctx: &TipCtx<'_>) -> TipText {
        match self.bits(stream) {
            Some(b) => self.tip_of(&b, ctx),
            None => TipText::default(),
        }
    }

    /// The box lines of the tip of `stream` with `ctx`, top line first;
    /// empty when the stream does not decode.
    pub fn tip_lines(&self, stream: &[u8], ctx: &TipCtx<'_>) -> Vec<TipLine> {
        match self.bits(stream) {
            Some(b) => self.tip_lines_of(&b, ctx),
            None => Vec::new(),
        }
    }

    /// The box lines of the tip of decoded item `b` with `ctx`, top line
    /// first, after the mod game's item hooks (`crate::mods::ItemLines`)
    /// ran over them.
    pub fn tip_lines_of(&self, b: &ItemBits, ctx: &TipCtx<'_>) -> Vec<TipLine> {
        let t = self.tip_of(b, ctx);
        let mut lines = if t.text.is_empty() {
            Vec::new()
        } else {
            text_lines(&t.text, t.color)
        };
        if self.mods.has_item_lines() {
            self.mods.item_lines(b, &mut lines);
        }
        lines
    }

    /// The tip lines of `stream` without a store, unit or fillers.
    pub fn lines(&self, stream: &[u8]) -> Vec<TipLine> {
        self.tip_lines(stream, &TipCtx::default())
    }

    #[cfg(test)]
    fn lines_of(&self, b: &ItemBits) -> Vec<TipLine> {
        let t = self.tip_of(b, &TipCtx::default());
        text_lines(&t.text, t.color)
    }

    #[cfg(test)]
    fn name_color(&self, b: &ItemBits) -> u16 {
        Build::new(self, b, &TipCtx::default()).name_color()
    }

    /// The list values of a stream list: a
    /// stat's own entry is sent as value >> `ValShift`; the partners
    /// written after 17, 48, 50, 52, 54, 57 are sent unshifted.
    pub fn stat_list(&self, stats: &[Stat]) -> StatList {
        let mut l = StatList::default();
        let shift = |s: u16| self.stats.get(usize::from(s)).map_or(0, |d| d.valshift);
        for (s, p, v) in super::item_tip_props::stream_values(stats, shift) {
            l.add(s, p, v);
        }
        l
    }

    /// Whether a character with these base stats meets the requirements
    /// of an item with `code` (strength, dexterity, level; item
    /// stat modifiers of the requirements are not applied). Used by the
    /// grid tint, not by the tip.
    pub fn can_use(&self, code: [u8; 4], strength: i32, dexterity: i32, level: i32) -> bool {
        self.codes.get(&code).is_none_or(|c| {
            i32::from(c.req_str) <= strength
                && i32::from(c.req_dex) <= dexterity
                && i32::from(c.levelreq) <= level
        })
    }

    /// The tip of a store item in buy mode (`Cost: ` with
    /// `price`). The requirement colours come from the units of a
    /// [`TipCtx`] ([`ItemTips::tip_lines`]); `usable` is no longer read.
    pub fn shop_lines(&self, stream: &[u8], price: u32, usable: bool) -> Vec<TipLine> {
        let _ = usable;
        let ctx = TipCtx {
            mode: 1,
            price: PriceText::Price {
                label: super::item_tip_build::tid::COST,
                price: price as i32,
            },
            ..TipCtx::default()
        };
        self.tip_lines(stream, &ctx)
    }
}

/// The lines of a tip text, top line first (the first line of the text is the
/// bottom one). Each line's colour is the one in effect at its start (colour codes
/// run on across LF); leading codes are folded into it, inner ones stay in the
/// text.
pub fn text_lines(text: &[u16], start: u16) -> Vec<TipLine> {
    let mut out = Vec::new();
    let mut c = start;
    for line in text.split(|&u| u == u16::from(b'\n')) {
        let mut i = 0;
        while line.len() >= i + 3 && line[i] == 0xFF && line[i + 1] == u16::from(b'c') {
            c = line[i + 2].wrapping_sub(u16::from(b'0'));
            i += 3;
        }
        let rest = &line[i..];
        out.push(TipLine {
            text: rest.to_vec(),
            color: c,
        });
        let mut k = 0;
        while k + 2 < rest.len() {
            if rest[k] == 0xFF && rest[k + 1] == u16::from(b'c') {
                c = rest[k + 2].wrapping_sub(u16::from(b'0'));
                k += 3;
            } else {
                k += 1;
            }
        }
    }
    out.reverse();
    out
}

/// A stat list that records what the property rules write.
#[derive(Default)]
struct Recorder(BTreeMap<(u16, u16), i32>);

impl d2_sim::items::ItemStats for Recorder {
    fn has_stats(&self) -> bool {
        true
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        self.0.get(&(id, layer)).copied().unwrap_or(0)
    }
    fn base(&self, id: u16, layer: u16) -> i32 {
        self.stat(id, layer)
    }
    fn set_base(&mut self, id: u16, layer: u16, value: i32) {
        self.0.insert((id, layer), value);
    }
    fn has_list(&self, _: d2_sim::items::ListKey) -> bool {
        true
    }
    fn list_set(&mut self, _: d2_sim::items::ListKey, id: u16, layer: u16, value: i32) {
        self.0.insert((id, layer), value);
    }
    fn list_add(&mut self, _: d2_sim::items::ListKey, id: u16, layer: u16, value: i32) {
        *self.0.entry((id, layer)).or_default() += value;
    }
    fn list_get(&self, _: d2_sim::items::ListKey, id: u16, layer: u16) -> i32 {
        self.stat(id, layer)
    }
}

impl desc::DescNames for ItemTips {
    fn string(&self, id: u16) -> Vec<u16> {
        self.strings
            .get_id(id)
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    }
    fn stat_desc(&self, s: u16) -> Option<StatDesc> {
        self.stats.get(usize::from(s)).copied()
    }
    fn skill_name(&self, skill: u32) -> Option<u16> {
        self.skills.get(skill as usize)?.0
    }
    fn skill_class(&self, skill: u32) -> Option<i8> {
        let (_, class) = self.skills.get(skill as usize)?;
        Some(*class as i8)
    }
    fn skill_count(&self) -> u32 {
        self.skills.len() as u32
    }
    fn class_strings(&self, class: u32) -> Option<desc::ClassStrings> {
        self.class_strings.get(class as usize).copied()
    }
    fn montype_name(&self, row: u32) -> Option<u16> {
        self.montype.get(row as usize).copied()
    }
    fn monstats_name(&self, row: u32) -> Option<u16> {
        self.monstats.get(row as usize).copied()
    }
    fn desc_list(&self) -> Vec<u16> {
        self.desc_order.clone()
    }
    fn group_members(&self, g: u16) -> Vec<u16> {
        (0..self.stats.len() as u16)
            .filter(|&s| self.stats[usize::from(s)].dgrp == g)
            .collect()
    }
}

/// The hover anchor of an item (X; top; bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TipAnchor {
    pub x: i32,
    pub top: i32,
    pub bottom: i32,
}

impl TipAnchor {
    /// The anchor of an item at grid cell (`c`, `r`) of size `w` × `h` cells in
    /// a grid at (`left`, `top`) with cells `cell_w` × `cell_h`: x = left +
    /// cellW·c + (w·cellW)/2, top = top + cellH·r, bottom = top + cellH·(r +
    /// h).
    pub fn of_cell(
        (left, top): (i32, i32),
        (cell_w, cell_h): (i32, i32),
        (c, r): (i32, i32),
        (w, h): (i32, i32),
    ) -> Self {
        TipAnchor {
            x: left + cell_w * c + (w * cell_w) / 2,
            top: top + cell_h * r,
            bottom: top + cell_h * (r + h),
        }
    }

    /// PROVISIONAL (the store-tip check does not reach the tip yet, so it does
    /// not test the anchor; the inventory item tip agrees): a bare point
    /// (no item rectangle known to the caller) is a zero-height anchor.
    pub fn at_point(p: Point) -> Self {
        TipAnchor {
            x: p.x,
            top: p.y,
            bottom: p.y,
        }
    }
}

/// The one pop-up text of `lines` (top line first), bottom line first
/// with a colour code where the colour changes; the
/// colour of the first (bottom) line is the pop-up's.
fn popup_text(lines: &[TipLine]) -> (Vec<u16>, u16) {
    let mut out: Vec<u16> = Vec::new();
    let mut cur = None;
    for (i, l) in lines.iter().rev().enumerate() {
        if i > 0 {
            out.push(u16::from(b'\n'));
        }
        if cur != Some(l.color) {
            out.extend([0xFF, u16::from(b'c'), u16::from(b'0') + l.color]);
            cur = Some(l.color);
        }
        out.extend_from_slice(&l.text);
    }
    out.truncate(1023);
    (out, lines.last().map_or(0, |l| l.color))
}

/// Queues the tip of `lines` as the hover pop-up `(text, x, y,
/// colour, 1)`: x:= the anchor's x; y:= the anchor's top when top − H > 0, else
/// bottom + H, H the text height; the box and text are the pop-up draw of
/// [`super::hud_tips::push_popup`].
pub fn draw_tip_at(
    lines: &[TipLine],
    anchor: TipAnchor,
    screen: (i32, i32),
    fonts: Option<&FontMeasure>,
    out: &mut dyn UiDrawSink,
) {
    if lines.is_empty() {
        return;
    }
    let (text, color) = popup_text(lines);
    // H: the pop-up's rectangle height away from every clamp.
    let height = fonts
        .and_then(|f| f.popup(FONT, &text, Point::new(0, 5000), true, (10_000, 10_000)))
        .map_or(0, |fr| fr.rect.1.y - fr.rect.0.y);
    let y = if anchor.top - height > 0 {
        anchor.top
    } else {
        anchor.bottom + height
    };
    super::original::hud_tips::push_popup(
        text,
        Point::new(anchor.x, y),
        color,
        true,
        screen,
        fonts,
        out,
    );
}

/// [`draw_tip_at`] for a caller that knows only the mouse (the anchor is
/// the point; `files` is no longer read).
pub fn draw_tip(
    lines: &[TipLine],
    mouse: Point,
    screen: (i32, i32),
    fonts: Option<&FontMeasure>,
    files: &UiFiles,
    out: &mut dyn UiDrawSink,
) {
    let _ = files;
    draw_tip_at(lines, TipAnchor::at_point(mouse), screen, fonts, out);
}

#[cfg(test)]
pub(crate) mod tests;
