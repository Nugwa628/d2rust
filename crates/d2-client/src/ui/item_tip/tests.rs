use super::*;
use crate::ui::item_tip_build::{pfx, tid, PriceText, SetCtx, TipUnit};
use crate::ui::item_tip_desc::sid;
use crate::ui::UiDraw;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::Itemtypes;
use d2_proto::item_bits::hflag;
use d2_sim::items::tables::{ItemRec, SetItemRec};
use std::collections::HashMap;

struct Strs {
    keys: HashMap<String, Vec<u16>>,
    ids: HashMap<u16, Vec<u16>>,
}

impl StringLookup for Strs {
    fn get(&self, key: &str) -> Option<&[u16]> {
        self.keys.get(key).map(Vec::as_slice)
    }
    fn get_id(&self, id: u16) -> Option<&[u16]> {
        self.ids.get(&id).map(Vec::as_slice)
    }
}

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Itemtypes row (all zero, class 7 none).
fn itype() -> Itemtypes {
    let mut t = <Itemtypes as Record>::decode(&[0u8; 228]);
    t.class = 7;
    t
}

/// Type rows the fixture uses: 37 `helm` (is-a 50 `armo`), 51
/// `shld` (is-a 50).
const HELM: i16 = 37;
const ARMO: usize = 50;
const SHLD: i16 = 51;
/// Type 4 `gold`.
const GOLD: i16 = 4;
/// Type 22 `scro`.
const SCRO: i16 = 22;

fn equiv(pairs: &[(usize, usize)]) -> EquivMatrix {
    let n: usize = 90;
    let words = n.div_ceil(32);
    let mut bits = vec![0u32; n * words];
    for i in 0..n {
        bits[i * words + i / 32] |= 1 << (i % 32);
    }
    for &(i, j) in pairs {
        bits[i * words + j / 32] |= 1 << (j % 32);
    }
    EquivMatrix { n, words, bits }
}

/// A cap (name id 7, `levelreq` 3, type `helm`), magicprefix row 0
/// `Sturdy`, magicsuffix row 0 `Fox` (wire ids 1), stat 1 described by
/// string 9 (`descfunc` 1, value first). String texts are the game's
/// English ones.
pub(crate) fn tips() -> ItemTips {
    tips_with(ItemTables::default())
}

fn tips_with(items: ItemTables) -> ItemTips {
    let items = ItemTables {
        items: vec![
            ItemRec {
                code: *b"cap ",
                type_: HELM,
                ..ItemRec::default()
            },
            ItemRec {
                code: *b"buc ",
                type_: SHLD,
                ..ItemRec::default()
            },
            ItemRec {
                code: *b"gld ",
                type_: GOLD,
                ..ItemRec::default()
            },
            ItemRec {
                code: *b"isc ",
                type_: SCRO,
                ..ItemRec::default()
            },
            ItemRec {
                code: *b"tsc ",
                type_: SCRO,
                ..ItemRec::default()
            },
        ],
        itemtypes: (0..90).map(|_| itype()).collect(),
        equiv: equiv(&[(HELM as usize, ARMO), (SHLD as usize, ARMO)]),
        // The `books` rows: 0 Town Portal, 1 Identify.
        books: vec![(*b"tsc ", *b"tbk "), (*b"isc ", *b"ibk ")],
        ..items
    };
    let mut codes = BTreeMap::new();
    codes.insert(
        *b"cap ",
        ItemText {
            name_id: 7,
            levelreq: 3,
            ..ItemText::default()
        },
    );
    codes.insert(
        *b"buc ",
        ItemText {
            name_id: 8,
            ..ItemText::default()
        },
    );
    codes.insert(
        *b"gld ",
        ItemText {
            name_id: 14,
            ..ItemText::default()
        },
    );
    for code in [*b"isc ", *b"tsc "] {
        codes.insert(
            code,
            ItemText {
                name_id: 15,
                ..ItemText::default()
            },
        );
    }
    let strs = Strs {
        keys: [
            ("Sturdy", "Sturdy"),
            ("Fox", "of the Fox"),
            ("Greymaker", "Greymaker"),
            ("Sigon's Visor", "Sigon's Visor"),
            ("Sigon's Gage", "Sigon's Gage"),
        ]
        .map(|(k, v)| (k.to_owned(), u16s(v)))
        .into(),
        ids: [
            (7, "Cap"),
            (8, "Buckler"),
            (9, "to Life"),
            (11, "Sigon's Complete Steel"),
            (12, "to Mana"),
            (13, "Defense"),
            (14, "Gold"),
            (15, "Scroll"),
            (tid::TP_SCROLL, "Scroll of Town Portal"),
            (tid::ID_SCROLL, "Scroll of Identify"),
            (sid::SP, " "),
            (sid::PLUS, "+"),
            (sid::PCT, "%"),
            (sid::NL, "\n"),
            (tid::SOCKETED, "Socketed"),
            (tid::UNIDENTIFIED, "Unidentified"),
            (tid::DURABILITY, "Durability:"),
            (tid::REQ_STR, "Required Strength:"),
            (tid::REQ_DEX, "Required Dexterity:"),
            (tid::DEFENSE, "Defense:"),
            (tid::OF, "of"),
            (tid::REQ_LEVEL, "Required Level:"),
            (tid::ETHEREAL, "Ethereal (Cannot be Repaired)"),
            (tid::NAME_MAGIC, "%0 %1 %2"),
            (tid::NAME_SET, "%0"),
            (tid::COST, "Cost: "),
            (tid::NOT_TRADED, "Item cannot be traded here."),
            (tid::CANNOT_REPAIR, "This item cannot be repaired."),
        ]
        .map(|(k, v)| (k, u16s(v)))
        .into(),
    };
    ItemTips {
        lookup: Arc::new(items),
        codes,
        desc_order: vec![3, 2, 1],
        stats: vec![
            StatDesc::default(),
            StatDesc {
                priority: 5,
                func: 1,
                val: 1,
                pos: 9,
                ..StatDesc::default()
            },
            StatDesc {
                priority: 5,
                func: 1,
                val: 1,
                pos: 12,
                ..StatDesc::default()
            },
            StatDesc {
                priority: 5,
                func: 3,
                val: 1,
                pos: 13,
                ..StatDesc::default()
            },
        ],
        magic_prefix: vec!["Sturdy".into()],
        magic_suffix: vec!["Fox".into()],
        rare_prefix: Vec::new(),
        rare_suffix: Vec::new(),
        unique: vec!["Greymaker".into()],
        set: vec!["Sigon's Visor".into(), "Sigon's Gage".into()],
        low_quality: Vec::new(),
        set_names: vec![11],
        skills: Vec::new(),
        class_strings: Vec::new(),
        block_factors: Vec::new(),
        class_names: Vec::new(),
        montype: Vec::new(),
        monstats: Vec::new(),
        gem_letters: Vec::new(),
        prefix_color: vec![0xFF],
        suffix_color: vec![0xFF],
        set_inv_color: vec![0xFF],
        unique_inv_color: vec![0xFF],
        strings: Arc::new(strs),
        mods: Default::default(),
    }
}

fn text(l: &TipLine) -> String {
    String::from_utf16_lossy(&l.text)
}

fn got(lines: &[TipLine]) -> Vec<(String, u16)> {
    lines.iter().map(|l| (text(l), l.color)).collect()
}

fn pairs(v: &[(&str, u16)]) -> Vec<(String, u16)> {
    v.iter().map(|&(s, c)| (s.to_owned(), c)).collect()
}

fn magic_cap(flags: u32) -> ItemBits {
    let mut b = ItemBits {
        flags,
        code: *b"cap ",
        quality: 4,
        ..ItemBits::default()
    };
    b.quality_fields.magic = Some((1, 1));
    b.lists = vec![Some(vec![Stat {
        stat: 1,
        param: 0,
        raw: 25,
        save_add: 10,
    }])];
    b
}

/// A unit with base stats and set-state lists.
#[derive(Default)]
struct Unit {
    class: u32,
    stats: BTreeMap<u16, i32>,
    lists: BTreeMap<u8, StatList>,
}

impl TipUnit for Unit {
    fn unit_type(&self) -> u8 {
        0
    }
    fn class(&self) -> u32 {
        self.class
    }
    fn stat(&self, id: u16, _: u16) -> i32 {
        self.stats.get(&id).copied().unwrap_or(0)
    }
    fn state_list(&self, state: u8) -> Option<StatList> {
        self.lists.get(&state).cloned()
    }
}

fn unit(level: i32, str_: i32) -> Unit {
    Unit {
        stats: [(0, str_), (2, 100), (12, level)].into(),
        ..Unit::default()
    }
}

fn lines_with(t: &ItemTips, b: &ItemBits, ctx: &TipCtx) -> Vec<(String, u16)> {
    let tt = t.tip_of(b, ctx);
    got(&text_lines(&tt.text, tt.color))
}

#[test]
fn pfx_vectors() {
    assert!(pfx(Vec::new(), 3).is_empty());
    assert_eq!(
        String::from_utf16_lossy(&pfx(u16s("Defense: 3\n"), 0)),
        "\u{ff}c0Defense: 3\n"
    );
}

// Block order (name on top, then requirements, then properties) and the
// requirement colour from U's level.
#[test]
fn an_identified_magic_item_shows_name_requirements_and_properties() {
    let t = tips();
    let low = unit(2, 50);
    let ctx = TipCtx {
        unit: Some(&low),
        ..TipCtx::default()
    };
    assert_eq!(
        lines_with(&t, &magic_cap(hflag::IDENTIFIED), &ctx),
        pairs(&[
            ("Sturdy Cap of the Fox", color::BLUE),
            ("Required Level: 3", color::RED),
            ("+15 to Life", color::BLUE),
        ])
    );
    let high = unit(3, 50);
    let ctx = TipCtx {
        unit: Some(&high),
        ..TipCtx::default()
    };
    assert_eq!(
        lines_with(&t, &magic_cap(hflag::IDENTIFIED), &ctx)[1],
        ("Required Level: 3".to_owned(), color::WHITE)
    );
}

// "Required Level:" only when the level is above 1.
#[test]
fn a_level_1_requirement_shows_no_line() {
    let mut t = tips();
    t.codes.get_mut(b"cap ").expect("cap").levelreq = 1;
    let lines = t.lines_of(&magic_cap(hflag::IDENTIFIED));
    assert!(lines.iter().all(|l| !text(l).starts_with("Required Level")));
    t.codes.get_mut(b"cap ").expect("cap").levelreq = 2;
    let lines = t.lines_of(&magic_cap(hflag::IDENTIFIED));
    assert!(lines.iter().any(|l| text(l) == "Required Level: 2"));
}

// `Unidentified` sits under the requirements, in red; no properties.
#[test]
fn an_unidentified_item_shows_its_base_name_and_no_properties() {
    assert_eq!(
        got(&tips().lines_of(&magic_cap(0))),
        pairs(&[("Cap", color::BLUE), ("Unidentified", color::RED)])
    );
}

// reqstr 100, stat 91 = −20, ethereal, U strength 60 → red
// `Required Strength: 70`; the ethereal line at the bottom.
#[test]
fn requirement_bonuses_and_the_failed_test_colour() {
    let mut t = tips();
    t.codes.get_mut(b"cap ").expect("cap").req_str = 100;
    let mut b = magic_cap(hflag::IDENTIFIED | 0x40_0000);
    b.lists = vec![Some(vec![Stat {
        stat: 91,
        param: 0,
        raw: (-20i32) as u32,
        save_add: 0,
    }])];
    let u = unit(10, 60);
    let ctx = TipCtx {
        unit: Some(&u),
        ..TipCtx::default()
    };
    let lines = lines_with(&t, &b, &ctx);
    assert!(lines.contains(&("Required Strength: 70".to_owned(), color::RED)));
    assert_eq!(
        lines.last(),
        Some(&("Ethereal (Cannot be Repaired)".to_owned(), color::BLUE))
    );
    let strong = unit(10, 70);
    let ctx = TipCtx {
        unit: Some(&strong),
        ..TipCtx::default()
    };
    assert!(lines_with(&t, &b, &ctx).contains(&("Required Strength: 70".to_owned(), color::WHITE)));
}

// Store: the price is the top line and continues the name's colour; a
// failed price says the item cannot be traded; repair mode notes an
// ethereal item.
#[test]
fn store_lines_on_top() {
    let t = tips();
    let b = magic_cap(hflag::IDENTIFIED);
    let buy = TipCtx {
        mode: 1,
        price: PriceText::Price {
            label: tid::COST,
            price: 500,
        },
        ..TipCtx::default()
    };
    assert_eq!(
        lines_with(&t, &b, &buy)[0],
        ("Cost: 500".to_owned(), color::BLUE)
    );
    let fail = TipCtx {
        mode: 1,
        ..TipCtx::default()
    };
    assert_eq!(
        lines_with(&t, &b, &fail)[0],
        ("Item cannot be traded here.".to_owned(), color::RED)
    );
    let repair = TipCtx {
        mode: 4,
        price: PriceText::Empty,
        ..TipCtx::default()
    };
    let eth = magic_cap(hflag::IDENTIFIED | 0x40_0000);
    assert_eq!(
        lines_with(&t, &eth, &repair)[0],
        ("This item cannot be repaired.".to_owned(), color::RED)
    );
    // The shop helper asks for the cost line.
    assert_eq!(
        t.shop_lines(&[], 1, true),
        Vec::new(),
        "a stream that does not decode has no tip"
    );
}

// Each line starts in the colour in effect at its start; codes run on
// across LF and leading codes fold into the line's colour.
#[test]
fn text_lines_follow_the_colour_codes() {
    let s = u16s("\u{ff}c3a\nb\n\u{ff}c1c\u{ff}c2d");
    assert_eq!(
        got(&text_lines(&s, 0)),
        pairs(&[("c\u{ff}c2d", 1), ("b", 3), ("a", 3)])
    );
}

#[test]
fn a_compact_stream_decodes_to_its_name() {
    // flags (compact | identified), version 0x65, mode 0, location
    // 0 / 0 / 0 / page 0, code `cap `.
    let bits: Vec<(u32, u32)> = vec![
        (hflag::COMPACT | hflag::IDENTIFIED, 32),
        (0x65, 10),
        (0, 3),
        (0, 4),
        (0, 4),
        (0, 4),
        (0, 3),
        (u32::from_le_bytes(*b"cap "), 32),
    ];
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    let lines = tips().lines(&out);
    assert_eq!(lines.first().map(text).as_deref(), Some("Cap"));
    assert!(tips().lines(&[1, 2]).is_empty(), "garbage gives no tip");
}

/// A compact, identified record of `code` (no quality fields on the wire).
fn compact_stream(code: &[u8; 4]) -> Vec<u8> {
    let bits: Vec<(u32, u32)> = vec![
        (hflag::COMPACT | hflag::IDENTIFIED, 32),
        (0x65, 10),
        (0, 3),
        (0, 4),
        (0, 4),
        (0, 4),
        (0, 3),
        (u32::from_le_bytes(*code), 32),
    ];
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

#[test]
fn a_compact_scroll_is_named_by_its_code() {
    // The starting Scroll of Identify is a compact record: no spell slot on
    // the wire, so its code names it (it once read as Town Portal).
    let name = |code| tips().lines(&compact_stream(code)).first().map(text);
    assert_eq!(name(b"isc ").as_deref(), Some("Scroll of Identify"));
    assert_eq!(name(b"tsc ").as_deref(), Some("Scroll of Town Portal"));
}

#[test]
fn a_gold_pile_is_named_by_its_amount() {
    let pile = |gold| ItemBits {
        flags: hflag::COMPACT | hflag::IDENTIFIED,
        code: *b"gld ",
        gold,
        ..ItemBits::default()
    };
    let t = tips();
    let name = |b: &ItemBits| {
        let b = ItemTips::as_built(b);
        String::from_utf16_lossy(&Build::new(&t, &b, &TipCtx::default()).name())
    };
    assert_eq!(name(&pile(Some(1234))), "1234 Gold");
    // The ground label and the hover tip both show the amount.
    assert_eq!(
        got(&t.lines_of(&pile(Some(5))))
            .first()
            .map(|l| l.0.as_str()),
        Some("5 Gold")
    );
    // Without an amount the pile keeps its plain name.
    assert_eq!(name(&pile(None)), "Gold");
}

/// A 256-record font of advance `adv` and height 10 (the pop-up
/// measures only widths and the height).
fn flat_font() -> FontMeasure {
    use d2_formats::font::{FontTable, Glyph};
    let glyphs = (0..256u16)
        .map(|i| Glyph {
            code: 255 - i,
            unknown1: 0,
            width: 6,
            height: 10,
            unknown2: 1,
            unknown3: 0,
            frame: i,
            unknown5: 0,
        })
        .collect();
    let mut m = FontMeasure::default();
    m.insert(
        FONT,
        FontTable {
            version: 1,
            unknown: 0,
            count: 256,
            height: 10,
            width: 0,
            glyphs,
        },
    );
    m
}

#[test]
fn the_tip_is_one_popup_text_bottom_line_first() {
    let lines = tips().lines_of(&magic_cap(hflag::IDENTIFIED));
    assert!(lines.len() > 1);
    let files = UiFiles::new(&[]);
    let mut out: Vec<UiDraw> = Vec::new();
    draw_tip(
        &lines,
        Point::new(300, 200),
        (800, 600),
        None,
        &files,
        &mut out,
    );
    let texts: Vec<&crate::ui::TextRequest> = out
        .iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1, "one pop-up text, not a draw per line");
    // The last (top) line of the text is the item name; lines are
    // separated by LF with the bottom line first.
    let shown = String::from_utf16_lossy(&texts[0].text);
    let rows: Vec<&str> = shown.split('\n').collect();
    assert_eq!(rows.len(), lines.len());
    assert!(
        rows.last().unwrap().ends_with(&text(&lines[0])),
        "{shown:?}"
    );
}

#[test]
fn the_popup_sits_above_the_item_unless_it_does_not_fit() {
    let lines = tips().lines_of(&magic_cap(hflag::IDENTIFIED));
    let fonts = flat_font();
    let at = |top: i32| {
        let mut out: Vec<UiDraw> = Vec::new();
        let a = TipAnchor {
            x: 400,
            top,
            bottom: top + 56,
        };
        draw_tip_at(&lines, a, (800, 600), Some(&fonts), &mut out);
        out
    };
    let rect = |out: &[UiDraw]| match &out[0] {
        UiDraw::Rect(r) => (r.x0, r.y0, r.x1, r.y1),
        other => panic!("{other:?}"),
    };
    // The box height of n lines of height 10 (16 / 10 of the font height per
    // line).
    let h = 16 * lines.len() as i32;
    // Room above: y = top, the box bottom at top + 2.
    let (_, y0, _, y1) = rect(&at(300));
    assert_eq!((y0, y1), (300 + 2 - h, 300 + 2), "above the item");
    // No room (top − H ≤ 0): y = bottom + H, the box below the item.
    let (_, y0, _, y1) = rect(&at(5));
    assert_eq!((y0, y1), (5 + 56 + h + 2 - h, 5 + 56 + h + 2), "below it");
}

#[test]
fn the_anchor_is_the_cell_rectangle_of_the_item() {
    // Grid at (418, 313), 29 × 29 cells, an item at (2, 1) of 2 × 3 cells.
    let a = TipAnchor::of_cell((418, 313), (29, 29), (2, 1), (2, 3));
    assert_eq!(
        a,
        TipAnchor {
            x: 418 + 58 + 29,
            top: 313 + 29,
            bottom: 313 + 29 * 4
        }
    );
}

#[test]
fn an_empty_tip_draws_nothing() {
    let files = UiFiles::new(&[]);
    let mut none: Vec<UiDraw> = Vec::new();
    draw_tip(&[], Point::new(1, 1), (800, 600), None, &files, &mut none);
    assert!(none.is_empty());
}

fn quality_cap(quality: u8, index: u32) -> ItemBits {
    let mut b = ItemBits {
        flags: hflag::IDENTIFIED,
        code: *b"cap ",
        quality,
        ..ItemBits::default()
    };
    b.quality_fields.file_index = Some(index);
    b
}

#[test]
fn the_name_colour_follows_quality_flags_codes_and_broken() {
    let t = tips();
    let c = |quality: u8, flags: u32, code: &[u8; 4]| {
        t.name_color(&ItemBits {
            flags,
            code: *code,
            quality,
            ..ItemBits::default()
        })
    };
    // r1: low / normal / superior are white, grey when socketed or
    // ethereal; tempered is `ÿc:`.
    assert_eq!(c(1, 0, b"cap "), color::WHITE);
    assert_eq!(c(2, 0, b"cap "), color::WHITE);
    assert_eq!(c(1, hflag::SOCKETED, b"cap "), color::GREY);
    assert_eq!(c(3, 0x40_0000, b"cap "), color::GREY);
    assert_eq!(c(4, hflag::SOCKETED, b"cap "), color::BLUE);
    assert_eq!(c(9, 0, b"cap "), color::TEMPERED);
    // r3: the listed codes are orange; r4: broken is red (last).
    assert_eq!(c(2, 0, b"pk1 "), color::ORANGE);
    assert_eq!(c(7, 0, b"toa "), color::ORANGE);
    assert_eq!(c(4, 0x100, b"cap "), color::RED);
    assert_eq!(c(2, 0x100, b"mbr "), color::RED);
    // r2: an unidentified store item in a store mode is white.
    let store = TipCtx {
        mode: 1,
        ..TipCtx::default()
    };
    let b = magic_cap(0);
    assert_eq!(
        crate::ui::item_tip_build::Build::new(&t, &b, &store).name_color(),
        color::WHITE
    );
}

#[test]
fn a_unique_item_shows_its_name_above_the_base_in_gold() {
    assert_eq!(
        got(&tips().lines_of(&quality_cap(7, 0)))[..2],
        pairs(&[("Greymaker", color::GOLD), ("Cap", color::GOLD)])
    );
}

/// A two-piece set 0 (`Sigon's Complete Steel`): row 0 `Sigon's Visor`
/// (add func 2), row 1 `Sigon's Gage`. The visor's partial lists: 165
/// gives +4 life, 166 +3 mana.
fn set_fixture() -> (ItemTips, ItemBits) {
    let t = tips_with(ItemTables {
        setitems: vec![
            SetItemRec {
                set: 0,
                add_func: 2,
                ..SetItemRec::default()
            },
            SetItemRec {
                set: 0,
                ..SetItemRec::default()
            },
        ],
        ..ItemTables::default()
    });
    let mut b = quality_cap(5, 0);
    let st = |stat, raw| Stat {
        stat,
        param: 0,
        raw,
        save_add: 0,
    };
    b.lists = vec![
        Some(vec![st(3, 7)]),
        Some(vec![st(1, 4)]),
        Some(vec![st(2, 3)]),
    ];
    (t, b)
}

// Only the lists the client received show: the partial lists the worn
// pieces switch on (add func 2: lists 165 … 165 + b − 2), the owner's
// state lists on an equipped piece, the set name and the members.
#[test]
fn a_set_item_shows_only_received_bonus_lists() {
    let (t, b) = set_fixture();
    let owner = Unit {
        class: 0,
        stats: [(12, 50)].into(),
        lists: [(165u8, {
            let mut l = StatList::default();
            l.add(71, 0, 0);
            l.add(3, 0, 5);
            l
        })]
        .into(),
    };
    // Two pieces worn (bits 0, 1), the visor owned; not equipped.
    let ctx = TipCtx {
        unit: Some(&owner),
        set: SetCtx {
            owned: vec![0],
            worn_without: 0b10,
            worn_with: 0b11,
        },
        ..TipCtx::default()
    };
    assert_eq!(
        lines_with(&t, &b, &ctx),
        pairs(&[
            ("Sigon's Visor", color::GREEN),
            ("Cap", color::GREEN),
            ("Required Level: 3", color::WHITE),
            ("7 Defense", color::BLUE),
            ("+4 to Life", color::GREEN),
            // r4's NL, in the set name's colour.
            ("", color::GOLD),
            ("Sigon's Complete Steel", color::GOLD),
            // Members in setitems order from the bottom.
            ("Sigon's Gage", color::RED),
            ("Sigon's Visor", color::GREEN),
        ])
    );
    // One piece worn: no partial list.
    let ctx1 = TipCtx {
        unit: Some(&owner),
        set: SetCtx {
            owned: vec![0],
            worn_without: 0,
            worn_with: 0b1,
        },
        ..TipCtx::default()
    };
    assert!(!lines_with(&t, &b, &ctx1).contains(&("+4 to Life".to_owned(), color::GREEN)));
    // Equipped: the owner's state-165 list of set 0 shows in gold.
    let mut worn = b.clone();
    worn.mode = 1;
    let lines = lines_with(&t, &worn, &ctx);
    assert!(
        lines.contains(&("5 Defense".to_owned(), color::GOLD)),
        "{lines:?}"
    );
}

// An alt-code record (a gamble item) reads as its base code with item
// level 1 and quality 1: an unidentified white base name, whatever the
// server item is.
#[test]
fn an_alt_code_record_shows_a_white_base_name() {
    let b = ItemBits {
        flags: hflag::ALT_CODE,
        base_code: Some(*b"cap "),
        ..ItemBits::default()
    };
    assert_eq!(
        got(&tips().lines_of(&b)),
        pairs(&[("Cap", color::WHITE), ("Unidentified", color::RED)])
    );
}

#[test]
fn the_inventory_colour_follows_invtrans_and_the_quality_source() {
    let mut t = tips();
    let identified = hflag::IDENTIFIED;
    // No colour anywhere: no map.
    assert_eq!(t.inv_color_of(&magic_cap(identified)), None);
    // `InvTrans` 6 (invgrey); the prefix's transformcolor 9, then the
    // suffix's 4, which is read first (affix id 1 = row 0).
    t.codes.get_mut(b"cap ").unwrap().inv_trans = 6;
    t.prefix_color[0] = 9;
    assert_eq!(t.inv_color_of(&magic_cap(identified)), Some((6, 9)));
    t.suffix_color[0] = 4;
    assert_eq!(t.inv_color_of(&magic_cap(identified)), Some((6, 4)));
    // Files 3 and 4 are never selected.
    t.codes.get_mut(b"cap ").unwrap().inv_trans = 3;
    assert_eq!(t.inv_color_of(&magic_cap(identified)), None);
    t.codes.get_mut(b"cap ").unwrap().inv_trans = 8;
    assert_eq!(t.inv_color_of(&magic_cap(identified)), Some((8, 4)));
    // The suffix's 21 is the first hit (0xFF alone means none): no map.
    t.suffix_color[0] = 21;
    assert_eq!(t.inv_color_of(&magic_cap(identified)), None);
    // Unique / set: the row's invtransform.
    let mut u = ItemBits {
        flags: identified,
        code: *b"cap ",
        quality: 7,
        ..ItemBits::default()
    };
    u.quality_fields.file_index = Some(0);
    assert_eq!(t.inv_color_of(&u), None);
    t.unique_inv_color[0] = 2;
    assert_eq!(t.inv_color_of(&u), Some((8, 2)));
    u.quality = 5;
    t.set_inv_color[0] = 17;
    assert_eq!(t.inv_color_of(&u), Some((8, 17)));
    // Unidentified: no file index in the stream, the client keeps 0.
    u.quality_fields.file_index = None;
    assert_eq!(t.inv_color_of(&u), Some((8, 17)));
}

/// On a game install: the starting Scroll of Identify (a compact record)
/// is named by its code through the `books` table.
/// `D2_GAME_DIR=... cargo test -p d2-client --lib item_tip::tests::live -- --ignored`
#[test]
#[ignore = "needs D2_GAME_DIR"]
fn live_compact_scroll_names() {
    use d2_formats::mpq::ArchiveSet;
    use d2_server::world_data::game::GameTables;
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let set = ArchiveSet::open_dir(dir).expect("archives open");
    let items = GameTables::load(&set)
        .expect("tables")
        .item_tables()
        .expect("item tables");
    let isc = items.find_code(*b"isc ").and_then(|i| items.item(i));
    assert!(isc.is_some_and(|r| r.compactsave != 0), "isc is compact");
    let t = crate::app::items::item_tips(&set, items).expect("tips");
    let name = |code| t.lines(&compact_stream(code)).first().map(text);
    assert_eq!(name(b"isc ").as_deref(), Some("Scroll of Identify"));
    assert_eq!(name(b"tsc ").as_deref(), Some("Scroll of Town Portal"));
}
