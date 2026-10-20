//! The NPC text list (0x27 type 1) and the talk topic
//! box.

use super::{Metrics, FONT_16};
use crate::ui::original::msg_ui::NpcTextList;
use crate::ui::panels::menu_box::{MenuBox, MenuError, MenuParams};

/// Entries a list holds: a count of 8 or more is fatal 0x112.
pub const MAX_ENTRIES: usize = 8;
/// Not found: 3724 "Invalid Quest Value".
pub const STR_INVALID_QUEST: u16 = 3724;
/// Caption string of the box (`talk`).
pub const STR_TALK: u16 = 3381;

/// One entry: string id u16 +0, kind u32 +4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEntry {
    pub string: u16,
    pub kind: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextListError {
    /// Count u8@6 ≥ 8 is fatal 0x112.
    #[error("count {0} ≥ 8 (fatal 0x112)")]
    Count(u8),
}

/// The list: header {count, head} and entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextList {
    /// Head first.
    pub entries: Vec<TextEntry>,
}

impl TextList {
    /// 0x27 type 1: frees the old list, builds a new one from the message bytes
    /// 6–39 by pushing each entry at the **head**, then sorts it (count > 1):
    /// an insertion sort by string id, ascending; an entry moves only past
    /// strictly greater ids, so equal ids keep the reversed order of the
    /// message.
    pub fn build(raw: &NpcTextList) -> Result<Self, TextListError> {
        let count = raw.count();
        if usize::from(count) >= MAX_ENTRIES {
            return Err(TextListError::Count(count));
        }
        let mut entries: Vec<TextEntry> = Vec::new();
        for k in 0..usize::from(count) {
            entries.insert(
                0,
                TextEntry {
                    string: raw.string(k),
                    kind: u32::from(raw.kind(k)),
                },
            );
        }
        if entries.len() > 1 {
            // Insertion sort: moves past strictly greater ids only.
            for i in 1..entries.len() {
                let mut j = i;
                while j > 0 && entries[j - 1].string > entries[j].string {
                    entries.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
        Ok(Self { entries })
    }

    /// How many entries are of `kind`.
    pub fn count_kind(&self, kind: u32) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }

    /// The i-th entry of `kind`.
    pub fn nth_of_kind(&self, kind: u32, i: usize) -> Option<&TextEntry> {
        self.entries.iter().filter(|e| e.kind == kind).nth(i)
    }
}

/// The caption id of the first pair (text id
/// u16, caption id u16) of the 527-entry caption table whose text id
/// equals `id`, searching entries 0–99 when `id` < entry 100's text id
/// (164), else entries 100–526; not found → 3724.
pub fn caption_id(table: &[(u16, u16)], id: u16) -> u16 {
    let (lo, hi) = match table.get(100) {
        Some(&(t100, _)) if id < t100 => (0, 100),
        Some(_) => (100, table.len()),
        None => (0, table.len()),
    };
    table[lo..hi]
        .iter()
        .find(|&&(t, _)| t == id)
        .map_or(STR_INVALID_QUEST, |&(_, c)| c)
}

/// The handlers of the talk topic box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopicHandler {
    Introduction,
    Gossip,
    /// Replays the text of the k-th kind-2 entry.
    Replay(usize),
    /// "about the merchants" (Jerhyn).
    Merchants,
    /// The Horadric Cube.
    Cube,
    /// Cancel (also p1).
    Cancel,
    /// (p2).
    P2,
}

/// NPC classes of the intro rules.
pub const NPC_JERHYN: u32 = 201;
pub const NPC_CAIN: [u32; 5] = [244, 245, 246, 265, 520];
pub const STR_INTRODUCTION: u16 = 3399;
pub const STR_GOSSIP: u16 = 3395;
pub const STR_MERCHANTS: u16 = 3392;
pub const STR_CUBE: u16 = 2231;
pub const STR_CANCEL: u16 = 3400;

/// What the topic box is built from.
pub struct TopicInput<'a> {
    /// The NPC text list; `None` = no list.
    pub list: Option<&'a TextList>,
    /// The intro entry's flag +0x14 (no introduction).
    pub no_intro: bool,
    pub npc_class: u32,
    /// The Horadric Cube (`box `) is in the local player's inventory.
    pub has_cube: bool,
    /// The 527-entry table.
    pub caption_table: &'a [(u16, u16)],
    /// A topic box is already up (a second one is fatal 0x76F).
    pub box_up: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TopicError {
    #[error("a second talk topic box (fatal 0x76F)")]
    Second,
    #[error(transparent)]
    Menu(#[from] MenuError),
}

/// The box with the caption, the
/// topic items and "cancel". `strings(id)` gives a string; `anchor` is.
pub fn topic_box(
    input: &TopicInput<'_>,
    anchor: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
    frame: (i32, i32),
    m: &dyn Metrics,
) -> Result<MenuBox<TopicHandler>, TopicError> {
    if input.box_up {
        return Err(TopicError::Second);
    }
    let mut b = MenuBox::new(
        anchor,
        MenuParams {
            p1: Some(TopicHandler::Cancel),
            p2: Some(TopicHandler::P2),
            p5: true,
            p9: 1,
            ..Default::default()
        },
    )
    .expect("p1 is set");
    b.set_style(1);
    // Caption item: height 21, color 4, font 1, not selectable.
    b.add_item(&strings(STR_TALK), 21, 0, 4, FONT_16, None, false, m)?;
    // The others: height 15, color 0, font 1, selectable.
    let item = |b: &mut MenuBox<TopicHandler>, s: u16, h: TopicHandler| {
        b.add_item(&strings(s), 15, 0, 0, FONT_16, Some(h), true, m)
    };
    if let Some(list) = input.list {
        if !input.no_intro {
            item(&mut b, STR_INTRODUCTION, TopicHandler::Introduction)?;
        }
        item(&mut b, STR_GOSSIP, TopicHandler::Gossip)?;
        for k in 0..list.count_kind(2) {
            if let Some(e) = list.nth_of_kind(2, k) {
                let cap = caption_id(input.caption_table, e.string);
                item(&mut b, cap, TopicHandler::Replay(k))?;
            }
        }
        if input.npc_class == NPC_JERHYN {
            item(&mut b, STR_MERCHANTS, TopicHandler::Merchants)?;
        }
        if NPC_CAIN.contains(&input.npc_class) && input.has_cube {
            item(&mut b, STR_CUBE, TopicHandler::Cube)?;
        }
    }
    item(&mut b, STR_CANCEL, TopicHandler::Cancel)?;
    b.layout(frame.0, frame.1, m)?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::messages::testutil::{w, Fixed};

    /// A 0x27 record: type 1; bytes 6–39 = count u8, entries (kind u8@2,
    /// id u16@4 relative to byte 6).
    fn raw(entries: &[(u8, u16)]) -> NpcTextList {
        let mut r = [0u8; 40];
        r[1] = 1;
        r[6] = entries.len() as u8;
        for (k, &(kind, id)) in entries.iter().enumerate() {
            r[6 + 2 + 4 * k] = kind;
            r[6 + 4 + 4 * k..6 + 6 + 4 * k].copy_from_slice(&id.to_le_bytes());
        }
        NpcTextList::from_record(&r)
    }

    #[test]
    fn list_is_pushed_at_the_head_and_insertion_sorted() {
        // Message order e0..e3 with ids 5, 3, 5, 3: pushed at the head the
        // list is [e3, e2, e1, e0]; an entry moves only past strictly
        // greater ids, so equal ids keep that reversed order.
        let l = TextList::build(&raw(&[(0, 5), (1, 3), (2, 5), (3, 3)])).unwrap();
        assert_eq!(
            l.entries,
            vec![
                TextEntry { string: 3, kind: 3 },
                TextEntry { string: 3, kind: 1 },
                TextEntry { string: 5, kind: 2 },
                TextEntry { string: 5, kind: 0 },
            ]
        );
        // One entry: no sort. None: empty.
        assert_eq!(TextList::build(&raw(&[(2, 9)])).unwrap().entries.len(), 1);
        assert!(TextList::build(&raw(&[])).unwrap().entries.is_empty());
        // Count 8 is fatal 0x112.
        let mut r = [0u8; 40];
        r[6] = 8;
        assert_eq!(
            TextList::build(&NpcTextList::from_record(&r)),
            Err(TextListError::Count(8))
        );
        // Kind helpers: the i-th entry of a kind, in list order.
        let l = TextList::build(&raw(&[(2, 30), (2, 10), (0, 20)])).unwrap();
        assert_eq!(l.count_kind(2), 2);
        assert_eq!(l.nth_of_kind(2, 0).unwrap().string, 10);
        assert_eq!(l.nth_of_kind(2, 1).unwrap().string, 30);
        assert!(l.nth_of_kind(2, 2).is_none());
    }

    fn table() -> Vec<(u16, u16)> {
        let mut t: Vec<(u16, u16)> = (0..527).map(|i| (i as u16, 5000 + i as u16)).collect();
        // Entry 100's text id is 164: 100 → 164 with a gap.
        for (i, e) in t.iter_mut().enumerate().skip(100) {
            e.0 = 164 + (i as u16 - 100);
        }
        t[5] = (50, 7050);
        t
    }

    #[test]
    fn caption_lookup() {
        let t = table();
        assert_eq!(t[100].0, 164);
        // id < 164: entries 0–99 only.
        assert_eq!(caption_id(&t, 50), 7050);
        assert_eq!(caption_id(&t, 163), STR_INVALID_QUEST);
        // id ≥ 164: entries 100–526.
        assert_eq!(caption_id(&t, 164), 5100);
        assert_eq!(caption_id(&t, 164 + 426), 5526);
        assert_eq!(caption_id(&t, 164 + 427), STR_INVALID_QUEST);
        // The first match wins.
        let mut t2 = table();
        t2[101] = (164, 1);
        assert_eq!(caption_id(&t2, 164), 5100);
    }

    fn s(id: u16) -> Vec<u16> {
        w(&format!("s{id}"))
    }

    fn input<'a>(
        list: Option<&'a TextList>,
        table: &'a [(u16, u16)],
        class: u32,
    ) -> TopicInput<'a> {
        TopicInput {
            list,
            no_intro: false,
            npc_class: class,
            has_cube: false,
            caption_table: table,
            box_up: false,
        }
    }

    fn texts(b: &MenuBox<TopicHandler>) -> Vec<String> {
        b.items
            .iter()
            .map(|i| String::from_utf16(&i.text).unwrap())
            .collect()
    }

    #[test]
    fn topic_box_items() {
        let t = table();
        let list = TextList::build(&raw(&[(2, 164), (0, 99), (2, 50)])).unwrap();
        let m = Fixed;
        // Caption, introduction, gossip, the kind-2 replays in list order
        // (ids 50 then 164 after the sort), cancel.
        let mut inp = input(Some(&list), &t, 148);
        let b = topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap();
        assert_eq!(
            texts(&b),
            ["s3381", "s3399", "s3395", "s7050", "s5100", "s3400"]
        );
        assert_eq!(b.style, 1);
        assert_eq!((b.params.p5, b.params.p9), (true, 1));
        assert_eq!(b.params.p1, Some(TopicHandler::Cancel));
        assert_eq!(b.params.p2, Some(TopicHandler::P2));
        assert_eq!(b.items[3].handler, Some(TopicHandler::Replay(0)));
        assert_eq!(b.items[4].handler, Some(TopicHandler::Replay(1)));
        // Caption item: height 21, color 4, font 1, not selectable; the
        // others height 15, color 0, font 1, selectable.
        let c = &b.items[0];
        assert_eq!((c.height, c.color, c.font, c.selectable), (21, 4, 1, false));
        for i in &b.items[1..] {
            assert_eq!((i.height, i.color, i.font, i.selectable), (15, 0, 1, true));
        }
        assert_eq!(b.selected, 1);
        // Flag +0x14: no introduction item.
        inp.no_intro = true;
        let b = topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap();
        assert!(!texts(&b).contains(&"s3399".to_string()));
        // Jerhyn: "about the merchants"; Cain with the Cube: 2231.
        let inp = input(Some(&list), &t, 201);
        assert!(
            texts(&topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap())
                .contains(&"s3392".to_string())
        );
        for class in [244, 245, 246, 265, 520] {
            let mut inp = input(Some(&list), &t, class);
            let b = topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap();
            assert!(!texts(&b).contains(&"s2231".to_string()));
            inp.has_cube = true;
            let b = topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap();
            let tx = texts(&b);
            assert_eq!(tx[tx.len() - 2], "s2231");
        }
        // No list: only the caption and cancel.
        let inp = input(None, &t, 201);
        let b = topic_box(&inp, (300, 200), &s, (800, 600), &m).unwrap();
        assert_eq!(texts(&b), ["s3381", "s3400"]);
        // A second box is fatal 0x76F.
        let mut inp = input(None, &t, 1);
        inp.box_up = true;
        assert_eq!(
            topic_box(&inp, (0, 0), &s, (800, 600), &m).unwrap_err(),
            TopicError::Second
        );
    }
}
