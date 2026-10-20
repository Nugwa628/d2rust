//! The facts a grid click reads about the items: the stack test, the spell of a
//! scroll or book, and the room the cube's grid has. They are the same rules
//! the sim applies (`items/inventory/equip.rs` `stack_test`, `grid.rs`), read
//! over the stream the model holds of each item. The server decides the
//! outcome.
//!
//! D2RUST's own, unverified: a compact record (potion, scroll, gem, key) has
//! no quality on the wire; it reads as quality 2 (normal), file index 0,
//! no sockets and no damage stats, which is what the server's own units
//! hold for them.

use d2_proto::item_bits::{hflag, ItemBits};
use d2_sim::items::ty;

use crate::ui::item_tip::ItemTips;
use crate::ui::item_tip_build::scroll_spell;

/// Item flag 0x400000: ethereal.
const ETHEREAL: u32 = 0x0040_0000;

/// The damage stats the stack test compares.
const DAMAGE: [u16; 6] = [21, 22, 23, 24, 159, 160];

/// What the decisions read from the tables and streams.
pub trait GridInfo {
    fn bits(&self, stream: &[u8]) -> Option<ItemBits>;
    /// Items `stackable` ≠ 0 of `code`.
    fn stackable(&self, code: [u8; 4]) -> bool;
    /// The item with `code` is of type `ty` (equivalence included).
    fn is_type(&self, code: [u8; 4], ty: u16) -> bool;
    /// The sell test as far as the client knows it: the items
    /// `quest` column is 0 and the type is not 39. The item flag 0x1000 is not
    /// in the stream; the server refuses such a sale (D2RUST's own).
    fn sellable(&self, code: [u8; 4]) -> bool;
    /// The `books` table: (scroll code, book code) per spell row.
    fn books(&self) -> &[([u8; 4], [u8; 4])];
}

impl GridInfo for ItemTips {
    fn bits(&self, stream: &[u8]) -> Option<ItemBits> {
        ItemTips::bits(self, stream)
    }
    fn stackable(&self, code: [u8; 4]) -> bool {
        let t = self.tables();
        t.find_code(code)
            .and_then(|i| t.item(i))
            .is_some_and(|r| r.stackable != 0)
    }
    fn is_type(&self, code: [u8; 4], ty: u16) -> bool {
        let t = self.tables();
        t.find_code(code).is_some_and(|i| t.is_type(i, ty as i16))
    }
    fn sellable(&self, code: [u8; 4]) -> bool {
        let t = self.tables();
        t.find_code(code)
            .and_then(|i| Some((i, t.item(i)?)))
            .is_some_and(|(i, r)| r.quest == 0 && !t.is_type(i, 39))
    }
    fn books(&self) -> &[([u8; 4], [u8; 4])] {
        &self.lookup.books
    }
}

fn quality(b: &ItemBits) -> u8 {
    if b.flags & hflag::COMPACT != 0 {
        2
    } else {
        b.quality
    }
}

/// The damage stats of an item, in list order.
fn damage(b: &ItemBits) -> Vec<(u16, u32)> {
    b.lists
        .iter()
        .flatten()
        .flatten()
        .filter(|s| DAMAGE.contains(&s.stat))
        .map(|s| (s.stat, s.raw))
        .collect()
}

/// The stack test of two item
/// streams: same class, quality and file index, stackable, equal
/// ethereal bits, both qualities 1–3, equal damage stats, no sockets.
pub fn stack_test(info: &dyn GridInfo, a: &[u8], b: &[u8]) -> bool {
    let (Some(x), Some(y)) = (info.bits(a), info.bits(b)) else {
        return false;
    };
    let (qa, qb) = (quality(&x), quality(&y));
    x.code == y.code
        && qa == qb
        && x.quality_fields.file_index.unwrap_or(0) == y.quality_fields.file_index.unwrap_or(0)
        && info.stackable(x.code)
        && (x.flags & ETHEREAL) == (y.flags & ETHEREAL)
        && (1..=3).contains(&qa)
        && (1..=3).contains(&qb)
        && damage(&x) == damage(&y)
        && x.sockets.unwrap_or(0) == 0
        && y.sockets.unwrap_or(0) == 0
}

/// The spell of a scroll (type 22) or book (type 18) stream
/// ([`scroll_spell`]: the quality fields' spell slot, or the code of a
/// compact scroll). `None` for any other item.
pub fn spell_of(info: &dyn GridInfo, stream: &[u8], item_type: u16) -> Option<u32> {
    let b = info.bits(stream)?;
    if !info.is_type(b.code, item_type) {
        return None;
    }
    Some(u32::from(scroll_spell(info.books(), &b)))
}

/// The scroll kind of a cursor item (type 22) and the book kind of the
/// item under it (type 18), as the scroll use rule reads them.
pub fn scroll_kind(info: &dyn GridInfo, stream: &[u8]) -> Option<u32> {
    spell_of(info, stream, ty::SCRO)
}

/// See [`scroll_kind`].
pub fn book_kind(info: &dyn GridInfo, stream: &[u8]) -> Option<u32> {
    spell_of(info, stream, ty::BOOK)
}

/// Whether a `w` × `h` item fits somewhere in a `gw` × `gh` grid whose occupied
/// footprints are `taken` (x, y, w, h): the free-position search Succeeds
/// exactly when some in-bounds position has all its cells empty (a weighted
/// search that finds a fit finds one touching an edge or a neighbour).
pub fn has_room(grid: (i32, i32), taken: &[(i32, i32, i32, i32)], size: (i32, i32)) -> bool {
    let (gw, gh) = grid;
    let (w, h) = size;
    if w <= 0 || h <= 0 {
        return false;
    }
    let used = |cx: i32, cy: i32| {
        taken
            .iter()
            .any(|&(x, y, tw, th)| (x..x + tw).contains(&cx) && (y..y + th).contains(&cy))
    };
    (0..=gh - h)
        .any(|y| (0..=gw - w).any(|x| (y..y + h).all(|cy| (x..x + w).all(|cx| !used(cx, cy)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cube_has_room_when_a_footprint_is_free() {
        // An empty 3 x 4 grid holds a 2 x 3 item.
        assert!(has_room((3, 4), &[], (2, 3)));
        // A 1 x 1 item in the middle column of rows 0-3 blocks every 2 x 3.
        let taken = [(1, 0, 1, 1), (1, 3, 1, 1)];
        assert!(!has_room((3, 4), &taken, (2, 3)));
        // But a 1 x 1 still fits.
        assert!(has_room((3, 4), &taken, (1, 1)));
        // A full grid has no room; a zero-size item never fits.
        assert!(!has_room((3, 4), &[(0, 0, 3, 4)], (1, 1)));
        assert!(!has_room((3, 4), &[], (0, 1)));
    }

    use d2_proto::item_bits::{QualityFields, Stat};
    use std::collections::BTreeMap;

    /// Streams are one byte: an index into `items`.
    struct Mock {
        items: BTreeMap<u8, ItemBits>,
        stackable: Vec<[u8; 4]>,
        scrolls: Vec<[u8; 4]>,
        books: Vec<[u8; 4]>,
        book_rows: Vec<([u8; 4], [u8; 4])>,
    }

    impl GridInfo for Mock {
        fn bits(&self, stream: &[u8]) -> Option<ItemBits> {
            self.items.get(stream.first()?).cloned()
        }
        fn stackable(&self, code: [u8; 4]) -> bool {
            self.stackable.contains(&code)
        }
        fn is_type(&self, code: [u8; 4], t: u16) -> bool {
            match t {
                22 => self.scrolls.contains(&code),
                18 => self.books.contains(&code),
                _ => false,
            }
        }
        fn sellable(&self, _: [u8; 4]) -> bool {
            true
        }
        fn books(&self) -> &[([u8; 4], [u8; 4])] {
            &self.book_rows
        }
    }

    fn full(code: &[u8; 4], quality: u8) -> ItemBits {
        ItemBits {
            code: *code,
            quality,
            ..ItemBits::default()
        }
    }

    fn mock(items: Vec<(u8, ItemBits)>) -> Mock {
        Mock {
            items: items.into_iter().collect(),
            stackable: vec![*b"jav ", *b"key "],
            scrolls: vec![*b"isc ", *b"tsc "],
            books: vec![*b"ibk ", *b"tbk "],
            // The `books` rows: 0 Town Portal, 1 Identify.
            book_rows: vec![(*b"tsc ", *b"tbk "), (*b"isc ", *b"ibk ")],
        }
    }

    #[test]
    fn the_stack_test_follows_the_sims_rules() {
        let dmg = |v: u32| {
            let mut b = full(b"jav ", 2);
            b.lists = vec![Some(vec![Stat {
                stat: 21,
                param: 0,
                raw: v,
                save_add: 0,
            }])];
            b
        };
        let mut eth = full(b"jav ", 2);
        eth.flags |= ETHEREAL;
        let mut sock = full(b"jav ", 2);
        sock.sockets = Some(1);
        let mut idx = full(b"jav ", 3);
        idx.quality_fields = QualityFields {
            file_index: Some(2),
            ..QualityFields::default()
        };
        let mut compact = full(b"key ", 0);
        compact.flags |= hflag::COMPACT;
        let m = mock(vec![
            (0, full(b"jav ", 2)),
            (1, full(b"jav ", 2)),
            (2, full(b"jav ", 3)),
            (3, full(b"jav ", 4)),
            (4, dmg(5)),
            (5, dmg(6)),
            (6, eth),
            (7, sock),
            (8, full(b"cap ", 2)),
            (9, idx),
            (10, compact.clone()),
            (11, compact),
            (12, full(b"jav ", 4)),
        ]);
        let t = |a: u8, b: u8| stack_test(&m, &[a], &[b]);
        assert!(t(0, 1), "equal normal javelins stack");
        assert!(!t(0, 2), "different quality");
        assert!(!t(3, 12), "magic quality (4) never stacks");
        assert!(!t(4, 5), "different damage stat 21");
        assert!(t(4, 4));
        assert!(!t(0, 6), "ethereal differs");
        assert!(!t(0, 7) && !t(7, 7), "sockets");
        assert!(!t(8, 8), "not stackable");
        assert!(!t(2, 9), "different file index");
        assert!(t(10, 11), "compact records read as normal quality");
        assert!(!t(0, 99), "an undecodable stream");
    }

    #[test]
    fn the_spell_kind_is_only_for_scrolls_and_books() {
        let mut isc = full(b"isc ", 2);
        isc.quality_fields.spell = Some(7);
        let mut tbk = full(b"tbk ", 2);
        tbk.quality_fields.spell = Some(7);
        let m = mock(vec![(0, isc), (1, tbk), (2, full(b"jav ", 2))]);
        assert_eq!(scroll_kind(&m, &[0]), Some(7));
        assert_eq!(book_kind(&m, &[1]), Some(7));
        assert_eq!(book_kind(&m, &[0]), None, "a scroll is not a book");
        assert_eq!(scroll_kind(&m, &[2]), None);
    }

    #[test]
    fn a_compact_identify_scroll_goes_into_the_identify_tome() {
        // A compact record has no quality fields: the code names the spell.
        let compact = |code: &[u8; 4]| {
            let mut b = full(code, 0);
            b.flags |= hflag::COMPACT;
            b
        };
        let mut ibk = full(b"ibk ", 2);
        ibk.quality_fields.spell = Some(1);
        let mut tbk = full(b"tbk ", 2);
        tbk.quality_fields.spell = Some(0);
        let m = mock(vec![
            (0, compact(b"isc ")),
            (1, compact(b"tsc ")),
            (2, ibk),
            (3, tbk),
        ]);
        assert_eq!(scroll_kind(&m, &[0]), book_kind(&m, &[2]), "isc into ibk");
        assert_eq!(scroll_kind(&m, &[1]), book_kind(&m, &[3]), "tsc into tbk");
        assert_ne!(
            scroll_kind(&m, &[0]),
            book_kind(&m, &[3]),
            "isc not into tbk"
        );
    }
}
