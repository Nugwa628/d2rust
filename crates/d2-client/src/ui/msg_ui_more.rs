//! The UI consumer of the bridge outputs that only store UI globals or make a
//! small, fully specified UI call: 0x50 (hire popup), 0x8A (NPC interact),
//! 0x91, 0x78, 0x29, 0x52, 0x5E, 0x9B, 0x28 (client quest record) and the
//! overhead text store of 0x26. It is a child of [`super`] so it reads the same
//! private state. Calls whose callee is not known yet are named in the outcome's
//! `skipped` list, as in [`super::skip`].

use super::{OriginalUi, OriginalUiError, ON};
use crate::bridge::output::Output;
use crate::bridge::world::{UnitKey, MONSTER};

/// UI state 0x23 (UI_HIREICONS).
pub const UI_HIREICONS: u32 = 0x23;

/// Class `act5pow`.
pub const CLASS_ACT5POW: u32 = 534;
/// Class `act2guard2`.
pub const CLASS_ACT2GUARD2: u32 = 331;
/// Overlay shown on an NPC that wants to talk.
pub const OVERLAY_NPC_WANTS: u16 = 72;

/// One overlay call of the UI on a unit, in call order: `on` = the kind-3
/// create, else the remove by id. The world
/// view's effect layer runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCall {
    pub unit: UnitKey,
    pub id: u16,
    pub on: bool,
}

/// Parts the dispatches of this module skip, named.
pub mod skip {
    pub const QUEST_LOG_TAIL: &str = "0x52: quest UI update and tab rebuild (msg-ui §13 r2; ui/*)";
    pub const MERC_MENU: &str = "0x9B: the NPC-menu edit (ui/panels.md §14; msg-ui §15 r2)";
    pub const CHAT_FILTER: &str =
        "0x26: the ignore list (msg-ui §4 r3.2.1-r3.2.4) is empty without an ignorelist file or ignore command: nothing is filtered";
}

/// One entry of the NPC intro table: the NPC
/// class (+0) and the flag byte (+0x12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntroEntry {
    pub class: u32,
    pub flag: u8,
}

/// The overhead text record of a unit: duration,
/// end, language and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverheadText {
    /// +0: d = 8 · min(n, 254) + 125 for a text of n characters.
    pub duration: u32,
    /// +4: the overhead counter at creation plus d.
    pub end: u32,
    /// +8.
    pub lang: u8,
    /// +0x10: at most 254 characters.
    pub text: Vec<u8>,
}

/// The UI state these dispatches write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsgUiMore {
    /// The intro table (input: its rows come from the
    /// game data).
    pub intro_table: Vec<IntroEntry>,
    /// The monstats row count (data +0xA80), the bound of the monster id check.
    pub monstats_rows: u32,
    /// The registry value `PopupHireling` (missing = 0).
    pub popup_hireling: u32,
    /// Hireling state, written by the hire dispatch.
    pub hire_7beecc: u32,
    /// The one-time hireling-panel open.
    pub hire_7beee4: u32,
    /// The three quest words of quest code 1.
    pub quest_words: [i32; 3],
    /// Bytes 3–14 of quest code 4.
    pub quest_special_bytes: [u8; 12],
    /// Quest value written by the quest dispatch.
    pub quest_7bf254: u32,
    /// The two flags of quest code 23.
    pub flag_70ee8c: u32,
    pub flag_7a0674: u32,
    /// The two latches of quest code 36.
    pub zoo_latch: bool,
    pub zoo_word: u16,
    /// The trade partner name (byte 15 forced to 0); the wide conversion
    /// keeps the bytes as characters.
    pub partner_name: [u8; 16],
    /// The trade partner's GUID.
    pub partner_guid: u32,
    /// The game quest record.
    pub game_quest_record: [u8; 96],
    /// The client quest record: zeroed at game
    /// UI start and on a local-player change, overwritten by 0x28 only.
    pub client_quest: [u8; 96],
    /// The quest-log status bytes.
    pub quest_log_status: [u8; 41],
    /// Quest-log press latch.
    pub quest_7bf2b0: u32,
    /// The quest availability bytes and their set flag.
    pub quest_avail: [u8; 37],
    pub quest_avail_set: bool,
    /// The hireling state (−1: the hireling is alive) and its second word.
    pub merc_state: u16,
    pub merc_7c0dd0: u16,
    /// The hireling menu values run so far, in order.
    pub merc_menu_calls: Vec<u8>,
    /// The interact NPC and whether an
    /// interaction is active (UI state).
    pub interact_npc: u32,
    pub interact_active: bool,
    /// A UI state the dispatch reads.
    pub f4b1620: bool,
    /// UI sounds played on a unit.
    pub unit_sounds: Vec<(i32, UnitKey)>,
    /// The overlay calls on units since the last [`OverlayCall`] take,
    /// in call order.
    pub overlays: Vec<OverlayCall>,
    /// The overhead text records by unit key.
    pub overhead: std::collections::BTreeMap<UnitKey, OverheadText>,
    /// Steps once per overhead draw.
    pub overhead_counter: u32,
    /// The client's own language id.
    pub own_lang: u8,
}

impl Default for MsgUiMore {
    fn default() -> Self {
        Self {
            intro_table: Vec::new(),
            monstats_rows: 0,
            popup_hireling: 0,
            hire_7beecc: 0,
            hire_7beee4: 0,
            quest_words: [0; 3],
            quest_special_bytes: [0; 12],
            quest_7bf254: 0,
            flag_70ee8c: 0,
            flag_7a0674: 0,
            zoo_latch: false,
            zoo_word: 0,
            partner_name: [0; 16],
            partner_guid: 0,
            game_quest_record: [0; 96],
            client_quest: [0; 96],
            quest_log_status: [0; 41],
            quest_7bf2b0: 0,
            quest_avail: [0; 37],
            quest_avail_set: false,
            // The hireling state before any S→C 0x9B: 0xFFFF, the game's data
            // initializer and the reset.
            merc_state: 0xFFFF,
            merc_7c0dd0: 0,
            merc_menu_calls: Vec::new(),
            interact_npc: 0,
            interact_active: false,
            f4b1620: false,
            unit_sounds: Vec::new(),
            overlays: Vec::new(),
            overhead: std::collections::BTreeMap::new(),
            overhead_counter: 0,
            own_lang: 0,
        }
    }
}

/// The language refusal of the text conversion: languages 6, 7, 9,
/// 12 convert with `MultiByteToWideChar`; when `lang` differs from the
/// client's own language id and the pair is not {7, 12}, a text with a
/// character ≥ 0x80 is refused. `true`: the text is converted.
pub fn text_converts(text: &[u8], lang: u8, own_lang: u8) -> bool {
    let pair_7_12 = matches!((lang, own_lang), (7, 12) | (12, 7));
    lang == own_lang || pair_7_12 || text.iter().all(|&c| c < 0x80)
}

/// What a delivered `ChatLine` asks the screen to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    /// Type 4, or type 1 with u8@3 ∈ {0, 1}: a screen message.
    ScreenMessage(Vec<u8>),
    /// Type 6: a formatted line, then the screen message.
    Formatted(Vec<u8>),
    /// Types 2 and 1 otherwise: a line with the name.
    Named { name: Vec<u8>, text: Vec<u8> },
    /// Type 5 with the unit present: overhead text of the unit.
    Overhead(UnitKey, Vec<u8>),
    /// Type 7.
    Type7(u8),
}

impl OriginalUi {
    /// The UI globals of [`MsgUiMore`].
    pub fn more(&self) -> &MsgUiMore {
        &self.more
    }

    /// Writable inputs of [`MsgUiMore`] (tables, registry values).
    pub fn more_mut(&mut self) -> &mut MsgUiMore {
        &mut self.more
    }

    /// The UI dispatch of the outputs this module owns; `Ok(false)`: not
    /// one of them.
    pub(super) fn apply_more(&mut self, o: &Output) -> Result<bool, OriginalUiError> {
        match *o {
            Output::QuestSpecial { code, words } => self.quest_special(code, &words)?,
            Output::NpcInteract {
                unit,
                present,
                class,
                mdata_3c,
                blocker_open,
            } => self.npc_interact(unit, present, class, mdata_3c, blocker_open),
            Output::NpcIntro { slots } => self.npc_intro(&slots),
            Output::TradePartner { name, guid } => {
                self.more.partner_name = name;
                self.more.partner_name[15] = 0;
                self.more.partner_guid = guid;
            }
            Output::GameQuestFlags { record } => self.more.game_quest_record = record,
            Output::QuestLog { status } => {
                self.more.quest_log_status = status;
                self.more.quest_7bf2b0 = 0;
                self.skip(skip::QUEST_LOG_TAIL);
            }
            Output::QuestAvailability { bytes } => {
                self.more.quest_avail = bytes;
                self.more.quest_avail_set = true;
            }
            Output::HireListReset => self.hire.borrow_mut().reset(),
            Output::HireOffer { name, seed } => self.hire.borrow_mut().offer(name, seed),
            Output::MercRevive { state, value } => self.merc_revive(state, value),
            Output::QuestFlags { record } => {
                self.more.client_quest = record;
                self.shared.borrow_mut().client_quest = record;
            }
            Output::HoradricItem { code } => self.horadric_item(code),
            // Every call, changed item or not.
            Output::CursorRefresh { item } => {
                let sh = self.shared.borrow();
                sh.cursor.borrow_mut().set_item(item);
            }
            Output::HotkeyAssign {
                slot,
                skill,
                left,
                item,
            } => crate::ui::original::hud_hotkeys::server_assign(
                &mut self.shared.borrow_mut().hud.hotkeys,
                slot,
                skill,
                left,
                item,
            ),
            Output::OverheadClear { unit } => {
                self.more.overhead.remove(&unit);
                self.shared.borrow_mut().bubbles.clear(unit);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// 0x50: the UI part of the codes the handler emits.
    fn quest_special(&mut self, code: u16, w: &[u16; 6]) -> Result<(), OriginalUiError> {
        match code {
            1 => {
                self.more.quest_words = [i32::from(w[0]), i32::from(w[1]), i32::from(w[2])];
            }
            2 => {
                // r4: the entry scan never matters; the hire popup only.
                self.more.hire_7beecc = 0;
                self.set_ui(UI_HIREICONS, ON, false)?;
                if self.more.popup_hireling == 0 {
                    self.more.hire_7beee4 = 1;
                }
            }
            // Two fields are written that nothing reads; D2RUST
            // keeps neither.
            3 => {}
            4 => {
                let mut b = [0u8; 12];
                for (i, v) in b.chunks_mut(2).enumerate() {
                    v.copy_from_slice(&w[i].to_le_bytes());
                }
                self.more.quest_special_bytes = b;
                self.more.quest_7bf254 = 0;
            }
            23 => {
                self.more.flag_70ee8c = 0;
                self.more.flag_7a0674 = 1;
            }
            36 => {
                self.more.zoo_latch = true;
                self.more.zoo_word = w[0];
            }
            _ => {}
        }
        Ok(())
    }

    /// 0x8A at delivery.
    fn npc_interact(
        &mut self,
        unit: UnitKey,
        present: bool,
        class: u32,
        mdata_3c: Option<i32>,
        blocker_open: bool,
    ) {
        if !present {
            return;
        }
        if class == CLASS_ACT5POW {
            let id = if mdata_3c.is_some_and(|v| v != -1) {
                4603
            } else {
                4607
            };
            self.more.unit_sounds.push((id, unit));
            return;
        }
        // An interaction is active and the GUID equals the interact NPC
        // (the message's unit is type 1).
        let is_interact_npc = self.more.interact_active
            && unit.unit_type == MONSTER
            && unit.guid == self.more.interact_npc;
        if !is_interact_npc {
            self.more.overlays.push(OverlayCall {
                unit,
                id: OVERLAY_NPC_WANTS,
                on: true,
            });
        }
        let q = &self.more.client_quest;
        let quest_clear = !crate::bridge::objects::quest_bit(q, 12, 8)
            && !crate::bridge::objects::quest_bit(q, 12, 1);
        if class == CLASS_ACT2GUARD2 && !self.more.f4b1620 && quest_clear && !blocker_open {
            self.more.unit_sounds.push((3983, unit));
        }
    }

    /// Item quest bit (`code`): the quest
    /// bit of the item in the client quest record (`hst ` 10.11, `qf2 `
    /// 18.11) already set → nothing; else it is set and the Horadric
    /// animation starts at the cube's next draw (its stamp is the draw's
    /// tick, [`super::cube_ui`]).
    fn horadric_item(&mut self, code: [u8; 4]) {
        let q = if &code == b"hst " { 10 } else { 18 };
        if crate::bridge::objects::quest_bit(&self.more.client_quest, q, 11) {
            return;
        }
        let n = 16 * usize::from(q) + 11;
        if let Some(v) = self.more.client_quest.get_mut(n >> 3) {
            *v |= 1 << (n & 7);
        }
        self.shared.borrow().horadric_start.set(true);
    }

    /// 0x91: every intro entry whose class equals a slot value below the
    /// monstats row count gets its flag byte:= 1.
    fn npc_intro(&mut self, slots: &[u16; 12]) {
        let rows = self.more.monstats_rows;
        for &v in slots {
            if u32::from(v) >= rows {
                continue;
            }
            for e in &mut self.more.intro_table {
                if e.class == u32::from(v) {
                    e.flag = 1;
                }
            }
        }
    }

    /// 0x9B.
    fn merc_revive(&mut self, state: u16, value: u16) {
        self.more.merc_state = state;
        self.more.merc_7c0dd0 = value;
        if state == 0xFFFF {
            self.more.merc_menu_calls.extend([11, 8, 24, 21, 43]);
            self.skip(skip::MERC_MENU);
        }
    }

    /// The overhead text: an empty
    /// text frees the unit's record; otherwise a new record replaces the
    /// old one.
    pub fn set_overhead(&mut self, unit: UnitKey, text: &[u8], lang: u8) {
        self.shared.borrow_mut().bubbles.set(unit, text, lang);
        if text.is_empty() {
            self.more.overhead.remove(&unit);
            return;
        }
        let n = text.len().min(254) as u32;
        let duration = 8 * n + 125;
        let rec = OverheadText {
            duration,
            end: self.more.overhead_counter.wrapping_add(duration),
            lang,
            text: text[..n as usize].to_vec(),
        };
        self.more.overhead.insert(unit, rec);
    }

    /// One overhead draw: the counter steps once, and a
    /// record whose end has passed is freed.
    pub fn overhead_draw(&mut self) {
        self.more.overhead_counter = self.more.overhead_counter.wrapping_add(1);
        let now = self.more.overhead_counter;
        self.more.overhead.retain(|_, r| r.end >= now);
    }

    /// 0x26 at delivery. `roster_squelched`: the
    /// roster test is 0 (an entry with relation 4
    /// set, only from out-of-scope messages): a player line shows
    /// nothing.
    pub fn chat_line(&mut self, o: &Output, roster_squelched: bool) -> Option<ChatAction> {
        let Output::ChatLine {
            kind,
            lang,
            unit,
            b8,
            name,
            text,
            present,
            ..
        } = o
        else {
            return None;
        };
        // r3.1.
        if unit.unit_type == 0 && roster_squelched {
            return None;
        }
        // r3.2 (r3.2.1–r3.2.4): the filter object is the persistent
        // ignore list, filled only from an `ignorelist` file and the chat
        // ignore commands; D2RUST has neither, so nothing is filtered (the
        // spec's single-player case).
        self.skip(skip::CHAT_FILTER);
        // r3.3.
        if !text_converts(text, *lang, self.more.own_lang) {
            return None;
        }
        // r3.4.
        match *kind {
            4 => Some(ChatAction::ScreenMessage(text.clone())),
            6 => Some(ChatAction::Formatted(text.clone())),
            1 if matches!(unit.unit_type, 0 | 1) => Some(ChatAction::ScreenMessage(text.clone())),
            1 | 2 => Some(ChatAction::Named {
                name: name.clone(),
                text: text.clone(),
            }),
            5 if *present => {
                self.set_overhead(*unit, text, *lang);
                Some(ChatAction::Overhead(*unit, text.clone()))
            }
            7 => Some(ChatAction::Type7(*b8)),
            _ => None,
        }
    }
}
