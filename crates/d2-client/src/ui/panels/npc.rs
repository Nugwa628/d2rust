//! The NPC menu (ui 8). The option table (48 records), its per-interaction
//! edits, the builder additions known exactly, and the C→S message
//! each option handler sends.
//!
//! Slot indices are 0-based, as in `npc-menus.tsv` (slot 0 is the first
//! option): "Kashya slot 1 = hire, count 3" (talk, hire, cancel)
//! and "Cain slot 1 = Identify Items, count 3" (the static Cain records
//! hold identify in slot 1) only read consistently that way.
//!
//! Not implemented yet:
//! - The Resurrect insert / remove for npc 150,
//!   198, 252, 367, 515 in an expansion game: the slot it is written to and
//!   the count change are not stated.
//! - Cain's count is set again later: value and
//!   moment not stated.
//! - Other runtime inserts (imbue, sockets, personalize, act
//!   travel east).
//! - The menu box (position, item metrics, `pentspin`
//!   placement inputs). No draw here.
//! - Talk ("C→S 0x2F / 0x30"): which of the two is sent, when, and
//!   with which bytes 1–4 is not known.
//! - Hire: sender not confirmed.
//! - Which record `menu(npc)` uses when an npc class has more
//!   than one record (npc 257: records 22 and 29): the lookup rule is not
//!   known; `menu` returns the first.
//! - The record flag byte @0x26: carried, unused.

use d2_proto::client::{EntityAction, IdentifyWithNpc, ResurrectMerc};

use super::PanelOutput;
use crate::ui::layout::{npc_menus, LayoutError, MenuOption, NpcMenuRecord, OptionKind};
use crate::ui::panel::ClientIntent;

/// String id of "Identify Items".
pub const STR_IDENTIFY: u16 = 4020;
/// String id of "Hire".
pub const STR_HIRE: u16 = 3397;
/// Kashya's npc class.
pub const NPC_KASHYA: u32 = 150;
/// The Cain npc classes.
pub const NPC_CAIN: [u32; 5] = [244, 265, 245, 246, 520];
/// Character level stat id (stat 12).
pub const STAT_LEVEL: u16 = 12;

/// The option table, as edited at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcMenus {
    records: Vec<NpcMenuRecord>,
}

impl NpcMenus {
    /// The 48 static records of `npc-menus.tsv`.
    pub fn load() -> Result<Self, LayoutError> {
        Ok(Self {
            records: npc_menus()?,
        })
    }

    pub fn from_records(records: Vec<NpcMenuRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[NpcMenuRecord] {
        &self.records
    }

    /// Record `i`, to edit (the build's caption counts).
    pub fn record_mut(&mut self, i: usize) -> Option<&mut NpcMenuRecord> {
        self.records.get_mut(i)
    }

    /// Applies `f` to every record of npc class `npc` (an npc class may
    /// have more than one record, e.g. 257; the menu edits are applied
    /// to each of them).
    fn each(&mut self, npc: u32, mut f: impl FnMut(&mut NpcMenuRecord)) {
        for r in self.records.iter_mut().filter(|r| r.npc == npc) {
            f(r);
        }
    }

    /// The per-interaction edits, applied literally. Slots are
    /// 0-based.
    pub fn reset_for_interaction(&mut self) {
        for npc in [150, 155, 210] {
            self.each(npc, |r| {
                r.count = 2;
                r.options[2] = None;
            });
        }
        self.each(154, |r| {
            r.count = 3;
            r.options[3] = None;
        });
        self.each(515, |r| {
            let res = matches!(
                r.options[1],
                Some(MenuOption {
                    kind: OptionKind::Resurrect,
                    ..
                })
            );
            r.count = if res { 3 } else { 2 };
            r.options[2] = None;
        });
        self.each(511, |r| {
            r.count = 3;
            r.options[3] = None;
        });
        self.each(512, |r| {
            r.count = 4;
            r.options[4] = None;
        });
        self.each(367, |r| {
            r.count = 2;
            r.options[1] = None;
            r.options[2] = None;
        });
        for npc in NPC_CAIN {
            self.each(npc, |r| {
                r.options[1] = Some(MenuOption {
                    string: STR_IDENTIFY,
                    kind: OptionKind::Identify,
                });
                r.count = 3;
            });
        }
    }

    /// The builder additions that are known exactly: Kashya
    /// (150) with character level (stat 12) > 7 gets slot 1 = hire (3397),
    /// count 3. The Resurrect insert / remove is [`Self::apply_resurrect`].
    pub fn apply_builder(&mut self, char_level: i32) {
        if char_level > 7 {
            self.each(NPC_KASHYA, |r| {
                r.options[1] = Some(MenuOption {
                    string: STR_HIRE,
                    kind: OptionKind::Hire,
                });
                r.count = 3;
            });
        }
    }

    /// The builder's Resurrect edit:
    /// for 150, 198, 252, 367 and 515 in an expansion game
    /// inserts Resurrect when the mercenary is dead (hireling state ≠ −1)
    /// and removes it otherwise ([`resurrect_edit`]).
    pub fn apply_resurrect(&mut self, merc_dead: bool, expansion_game: bool) {
        for npc in RESURRECT_SELLERS {
            self.each(npc, |r| resurrect_edit(r, merc_dead, expansion_game));
        }
    }

    /// The first record of npc class `npc` (lookup rule for npc classes
    /// with more than one record: module Open).
    pub fn menu(&self, npc: u32) -> Option<&NpcMenuRecord> {
        self.records.iter().find(|r| r.npc == npc)
    }
}

/// The option slots a record shows: slots `0 … count − 2` (count =
/// options + 1 for the trailing cancel). A cleared slot inside
/// that range is `None`.
pub fn shown_options(r: &NpcMenuRecord) -> Vec<Option<MenuOption>> {
    let n = (r.count as usize).saturating_sub(1).min(r.options.len());
    r.options[..n].to_vec()
}

/// The C→S message an option's handler sends. `None` where the
/// rule is not known exactly (talk, hire: module doc).
pub fn option_intent(kind: OptionKind, npc_guid: u32) -> Option<Vec<PanelOutput>> {
    let action = |action: u32, item: u32| {
        ClientIntent::from_message(&EntityAction {
            action,
            npc: npc_guid,
            item,
        })
    };
    let i = match kind {
        OptionKind::Talk | OptionKind::Hire | OptionKind::Imbue => return None,
        OptionKind::Trade => action(1, 0),
        OptionKind::Gamble => action(2, 0),
        OptionKind::TravelWest => action(0, 1),
        OptionKind::SailWest => action(0, 0x28),
        OptionKind::Identify => ClientIntent::from_message(&IdentifyWithNpc { npc: npc_guid }),
        OptionKind::Resurrect => ClientIntent::from_message(&ResurrectMerc { npc: npc_guid }),
    };
    Some(vec![PanelOutput::Intent(i)])
}

/// The NPC classes whose menu the builder gives the Resurrect edit (Kashya,
/// Greiz, Asheara, Qual-Kehk 367 and 515).
pub const RESURRECT_SELLERS: [u32; 5] = [150, 198, 252, 367, 515];

/// The inserted Resurrect string id, a placeholder replaced at build time.
pub const STR_RESURRECT_PLACEHOLDER: u16 = 0x1507;
/// Cain's record ids whose count is reset.
pub const CAIN_RECORDS: [u8; 5] = [16, 17, 18, 19, 38];

/// Record lookup: no unit → `None` (−1); else the
/// first record whose npc class matches; no match → record 0.
pub fn record_index(records: &[NpcMenuRecord], unit_class: Option<u32>) -> Option<usize> {
    let c = unit_class?;
    Some(records.iter().position(|r| r.npc == c).unwrap_or(0))
}

/// Resurrect insert / remove. Does
/// nothing unless the expansion is installed and the game is an expansion
/// game.
pub fn resurrect_edit(r: &mut NpcMenuRecord, insert: bool, expansion_game: bool) {
    if !expansion_game {
        return;
    }
    let res = |o: &Option<MenuOption>| matches!(o, Some(m) if m.kind == OptionKind::Resurrect);
    let n = (r.count as usize).min(r.options.len());
    let item = Some(MenuOption {
        string: STR_RESURRECT_PLACEHOLDER,
        kind: OptionKind::Resurrect,
    });
    if insert {
        if r.options[..n].iter().any(res) {
            return;
        }
        let at = match r.options[..n]
            .iter()
            .position(|o| matches!(o, Some(m) if m.kind == OptionKind::Hire))
        {
            Some(j) => {
                for k in (j..n.min(r.options.len() - 1)).rev() {
                    r.options[k + 1] = r.options[k];
                }
                j
            }
            None => n.saturating_sub(1),
        };
        r.options[at] = item;
        r.count += 1;
    } else if let Some(j) = r.options[..n].iter().position(res) {
        for k in j..r.options.len() - 1 {
            r.options[k] = r.options[k + 1];
        }
        let last = r.options.len() - 1;
        r.options[last] = None;
        r.count -= 1;
    }
}

/// What a talk sequence ending does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TalkEnd {
    /// Menu state:= 1 and the NPC menu is built again.
    RebuildMenu,
    /// Menu state 0, interaction ends (C→S 0x30), `SetUIState(8, off, 0)`.
    EndInteraction,
}

/// Classes whose talk ends rebuild the menu.
pub const TALK_END_CLASSES: [u32; 13] = [
    146, 251, 266, 331, 377, 378, 406, 408, 521, 527, 537, 538, 539,
];

/// (count 2): {message, next} pairs that
/// fix the next message of a talk sequence.
pub const NEXT_PAIRS: [(u16, u16); 2] = [(123, 126), (125, 126)];
/// The played list holds at most 9 ids.
pub const PLAYED_MAX: usize = 9;

/// The message after `cur`. A pair of
/// [`NEXT_PAIRS`] for `cur` wins; else the first kind-0 entry of the NPC
/// text list (in list order) that is neither `cur` nor played; none →
/// 0xFFFF.
pub fn next_message(cur: u16, kind0: impl IntoIterator<Item = u16>, played: &[u16]) -> u16 {
    if let Some(&(_, next)) = NEXT_PAIRS.iter().find(|&&(m, _)| m == cur) {
        return next;
    }
    kind0
        .into_iter()
        .find(|&m| m != cur && !played.contains(&m))
        .unwrap_or(0xFFFF)
}

/// A talk sequence ended with no next message.
pub fn talk_end(npc_present: bool, npc_class: u32, flag: u8) -> TalkEnd {
    if npc_present && !TALK_END_CLASSES.contains(&npc_class) && flag != 0 {
        TalkEnd::RebuildMenu
    } else {
        TalkEnd::EndInteraction
    }
}

/// Cain's count reset: result 3 or 6 with the interaction NPC one
/// of Cain's classes sets the counts of records 16, 17, 18, 19, 38 to 2.
pub fn cain_count_reset(records: &mut [NpcMenuRecord], interaction_npc: u32, result: u8) {
    if (result == 3 || result == 6) && NPC_CAIN.contains(&interaction_npc) {
        for r in records
            .iter_mut()
            .filter(|r| CAIN_RECORDS.contains(&r.record))
        {
            r.count = 2;
        }
    }
}

/// The 9-byte talk messages (helper).
pub fn talk_msg_bytes(id: u8, a: u32, b: u32) -> [u8; 9] {
    let mut o = [0u8; 9];
    o[0] = id;
    o[1..5].copy_from_slice(&a.to_le_bytes());
    o[5..9].copy_from_slice(&b.to_le_bytes());
    o
}
/// C→S 0x2F: unit type @1, GUID @5.
pub fn msg_chat_start(unit_type: u32, guid: u32) -> [u8; 9] {
    talk_msg_bytes(0x2F, unit_type, guid)
}
/// C→S 0x30 (NPC not found): `a4` low byte zero-extended @1, GUID @5.
pub fn msg_chat_not_found(a4: u32, guid: u32) -> [u8; 9] {
    talk_msg_bytes(0x30, a4 & 0xFF, guid)
}
/// C→S 0x30 (interaction end): 1 @1, GUID @5.
pub fn msg_chat_end(guid: u32) -> [u8; 9] {
    talk_msg_bytes(0x30, 1, guid)
}
/// C→S 0x31: NPC GUID @1, message id (u16, zero-extended) @5.
pub fn msg_quest(guid: u32, msg: u16) -> [u8; 9] {
    talk_msg_bytes(0x31, guid, msg as u32)
}

#[cfg(test)]
mod tests_c2ui;

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(string: u16, kind: OptionKind) -> Option<MenuOption> {
        Some(MenuOption { string, kind })
    }

    fn bytes(o: &[PanelOutput]) -> Vec<u8> {
        match o {
            [PanelOutput::Intent(ClientIntent(b))] => b.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn next_message_pairs_then_unplayed_kind0() {
        // A pair wins, even when its next was played.
        assert_eq!(next_message(123, [126, 300], &[126]), 126);
        assert_eq!(next_message(125, [], &[]), 126);
        // Else the first kind-0 entry that is neither cur nor played.
        assert_eq!(next_message(300, [300, 301, 302], &[300, 301]), 302);
        assert_eq!(next_message(300, [300, 301], &[301]), 0xFFFF);
        assert_eq!(next_message(0xFFFF, [], &[]), 0xFFFF);
    }

    // Test vector "Akara (148) menu": talk, trade, cancel (count 3).
    #[test]
    fn akara_menu_talk_trade_cancel() {
        let mut m = NpcMenus::load().unwrap();
        assert_eq!(m.records().len(), 48);
        m.reset_for_interaction();
        m.apply_builder(99);
        let r = m.menu(148).unwrap();
        assert_eq!(r.count, 3);
        assert_eq!(
            shown_options(r),
            vec![opt(3381, OptionKind::Talk), opt(3396, OptionKind::Trade)]
        );
    }

    // Partial: the fixed edits; Resurrect insert not implemented.
    #[test]
    fn reset_edits_literal() {
        let mut m = NpcMenus::load().unwrap();
        // Dirty every edited slot first.
        let junk = opt(1, OptionKind::Gamble);
        for r in &mut m.records {
            r.options = [junk; 5];
            r.count = 5;
        }
        let before = m.clone();
        m.reset_for_interaction();
        let rec = |npc| m.menu(npc).unwrap().clone();
        for npc in [150, 155, 210] {
            let r = rec(npc);
            assert_eq!(r.count, 2);
            assert_eq!(r.options, [junk, junk, None, junk, junk]);
        }
        let r = rec(154);
        assert_eq!((r.count, r.options), (3, [junk, junk, junk, None, junk]));
        let r = rec(515);
        assert_eq!((r.count, r.options), (2, [junk, junk, None, junk, junk]));
        let r = rec(511);
        assert_eq!((r.count, r.options), (3, [junk, junk, junk, None, junk]));
        let r = rec(512);
        assert_eq!((r.count, r.options), (4, [junk, junk, junk, junk, None]));
        let r = rec(367);
        assert_eq!((r.count, r.options), (2, [junk, None, None, junk, junk]));
        for npc in NPC_CAIN {
            let r = rec(npc);
            assert_eq!(r.count, 3);
            assert_eq!(
                r.options,
                [junk, opt(4020, OptionKind::Identify), junk, junk, junk]
            );
        }
        // Every other record untouched (257 has two records).
        let edited = [
            150, 155, 210, 154, 515, 511, 512, 367, 244, 265, 245, 246, 520,
        ];
        for (a, b) in m.records().iter().zip(before.records()) {
            if !edited.contains(&a.npc) {
                assert_eq!(a, b);
            }
        }
    }

    // Partial: 515 with Resurrect in slot 1.
    #[test]
    fn reset_515_with_resurrect_keeps_count_3() {
        let mut m = NpcMenus::load().unwrap();
        for r in m.records.iter_mut().filter(|r| r.npc == 515) {
            r.options[1] = opt(22695, OptionKind::Resurrect);
            r.options[2] = opt(1, OptionKind::Talk);
        }
        m.reset_for_interaction();
        let r = m.menu(515).unwrap();
        assert_eq!(r.count, 3);
        assert_eq!(r.options[1], opt(22695, OptionKind::Resurrect));
        assert_eq!(r.options[2], None);
    }

    // Partial: Kashya hire builder.
    #[test]
    fn kashya_hire_above_level_7() {
        let mut m = NpcMenus::load().unwrap();
        m.reset_for_interaction();
        m.apply_builder(7);
        assert_eq!(m.menu(150).unwrap().count, 2);
        assert_eq!(
            shown_options(m.menu(150).unwrap()),
            vec![opt(3381, OptionKind::Talk)]
        );
        m.apply_builder(8);
        let r = m.menu(150).unwrap();
        assert_eq!(r.count, 3);
        assert_eq!(
            shown_options(r),
            vec![opt(3381, OptionKind::Talk), opt(3397, OptionKind::Hire)]
        );
        // The next interaction's reset hides it again (count 2).
        m.reset_for_interaction();
        assert_eq!(m.menu(150).unwrap().count, 2);
    }

    // (every handler; talk sends nothing, hire opens the list).
    #[test]
    fn option_handlers_bytes() {
        let g = 0x1122_3344u32;
        let ea = |a: u8, item: u8| vec![0x38, a, 0, 0, 0, 0x44, 0x33, 0x22, 0x11, item, 0, 0, 0];
        assert_eq!(
            bytes(&option_intent(OptionKind::Trade, g).unwrap()),
            ea(1, 0)
        );
        assert_eq!(
            bytes(&option_intent(OptionKind::Gamble, g).unwrap()),
            ea(2, 0)
        );
        assert_eq!(
            bytes(&option_intent(OptionKind::TravelWest, g).unwrap()),
            ea(0, 1)
        );
        assert_eq!(
            bytes(&option_intent(OptionKind::SailWest, g).unwrap()),
            ea(0, 0x28)
        );
        assert_eq!(
            bytes(&option_intent(OptionKind::Identify, g).unwrap()),
            vec![0x34, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(
            bytes(&option_intent(OptionKind::Resurrect, g).unwrap()),
            vec![0x62, 0x44, 0x33, 0x22, 0x11]
        );
        assert_eq!(option_intent(OptionKind::Talk, g), None);
        assert_eq!(option_intent(OptionKind::Hire, g), None);
    }
}
