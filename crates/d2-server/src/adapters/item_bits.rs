//! [`d2_proto::item_bits::ItemLookup`] on the game's item tables
//! (`d2_sim::items::ItemTables`): the facts the reader of the 0x9C / 0x9D
//! item bit stream needs (type tests with equivalence, items and
//! itemtypes columns, itemstatcost save columns), from the same tables
//! the server's writer (`d2_sim::items::bitstream`) reads. `d2-proto`
//! holds no tables and `d2-sim` may not depend on `d2-proto`, so the
//! adapter lives here.
//!
//! The client side of the item wire format uses this lookup to decode the
//! items it receives; the server's writer reads the same tables directly.

use d2_proto::item_bits::{CodeFacts, IscSave, ItemLookup};
use d2_sim::items::{ty, ItemTables};

/// The reader's view of an [`ItemTables`].
#[derive(Clone, Copy)]
pub struct TablesLookup<'t>(pub &'t ItemTables);

impl ItemLookup for TablesLookup<'_> {
    /// The first items record with the code (the combined array:
    /// weapons, armor, misc).
    fn code(&self, code: [u8; 4]) -> Option<CodeFacts> {
        let t = self.0;
        let i = t.items.iter().position(|r| r.code == code)?;
        let rec = &t.items[i];
        let is = |k: u16| t.is_type(i, k as i16);
        Some(CodeFacts {
            armor: is(ty::ARMO),
            weapon: is(ty::WEAP),
            gold: is(ty::GOLD),
            charm: is(ty::CHAR),
            body_part: is(ty::BODY) && !is(ty::PLAY),
            scroll_or_book: is(ty::SCRO) || is(ty::BOOK),
            stackable: rec.stackable != 0,
            varinvgfx: t.itype_of(i).is_some_and(|y| y.varinvgfx != 0),
            quest_diff: rec.quest != 0 && rec.questdiffcheck != 0,
        })
    }

    /// A stat without a save row reads as `Save Bits` 0, as the writer
    /// reads it (`IscTable for [Isc]`); a stat outside the stat count
    /// has no row.
    fn isc(&self, stat: u16) -> Option<IscSave> {
        let t = self.0;
        if usize::from(stat) >= t.valshift.len().max(t.isc.len()) {
            return None;
        }
        let c = t.isc.get(usize::from(stat)).copied().unwrap_or_default();
        Some(IscSave {
            save_bits: c.save_bits,
            save_add: c.save_add,
            save_param_bits: c.save_param_bits,
        })
    }

    fn set_item_rows(&self) -> Option<usize> {
        Some(self.0.setitems.len())
    }
}
