//! The UI consumer of the bridge outputs: [`OriginalUi::apply_output`] runs the
//! UI dispatch of S→C 0x5D, 0x63 and 0x77 at delivery, on the UI's own
//! state ([`MsgUiState`]) and flags; it keeps 0x27's NPC text list and chooses
//! the case of 0x28's dialog branch, which it hands back to the bridge
//! ([`OriginalUi::take_dialog_answer`] → `Bridge::npc_dialog_branch`: the model
//! writes and C→S 0x31 are the bridge's). Sounds
//! go through the UI's request path ([`UiOutcome::sounds`]); a part whose input
//! or callee is not known yet is not guessed: it is skipped and named in
//! [`UiOutcome::skipped`]. Nothing here writes the model.

use d2_sim::world::waypoints::WaypointRecord;

use super::{OriginalUi, OriginalUiError};
use crate::audio::driver::SoundRequest;
use crate::bridge::msg::ui::{quest_row, QuestRow};
use crate::bridge::msg::ui_npc::DialogCase;
use crate::bridge::output::{Consumer, NpcDialog, Output};
use crate::bridge::world::{ClientWorld, UnitKey, PLAYER};
use crate::rules::lighting::environment::act_index;

/// `SetUIState` modes.
const ON: u32 = 0;
const OFF: u32 = 1;

/// UI states the dispatch uses.
pub const UI_CHAT: u32 = 5;
pub const UI_QUESTSCREEN: u8 = 15;
pub const UI_QUESTLOG: u32 = 17;
pub const UI_WAYPOINT: u32 = 20;
pub const UI_MPTRADE: u32 = 23;
pub const UI_STASH: u32 = 25;
pub const UI_CUBE: u32 = 26;

/// Inventory modes.
pub const MODE_STASH: u8 = 0x0C;
pub const MODE_STASH_2: u8 = 0x0D;
pub const MODE_CUBE: u8 = 0x0E;

/// The trade state value "refused".
pub const TRADE_REFUSED: u8 = 7;

/// Named parts the UI skipped (their input or callee is not specified).
pub mod skip {
    pub const MONSTER_EFFECT: &str = "0x5D: monster effect (211, 1) (msg-ui open question 1)";
    pub const VIDEO_7: &str = "0x5D: the video-7 flag (render/composition.md §4)";
    pub const DEN_COUNTER: &str =
        "0x5D: the client quest flags of the Den counter path (msg-ui OQ 4)";
    pub const QUEST_LOG_TABLE: &str =
        "0x5D: the quest-log table (41 entries) is not in the specs (msg-ui §1 r6)";
    pub const WAYPOINT_TAB_GATE: &str =
        "0x63: the waypoint tab gate reads the client quest flags (msg-ui OQ 4)";
    pub const WAYPOINT_ROWS: &str = "0x63: the row rebuild (ui/panels.md §13 r5)";
    pub const TRADE: &str = "0x77: the trade helpers of codes 0x00–0x06 (msg-ui OQ 5)";
    pub const NO_LOCAL_PLAYER: &str = "0x77 code 9: no local player";
    pub const TRADE_CLOSE_HELPER: &str = "0x77: the trade close helper (msg-ui OQ 5)";
    pub const NOT_APPLIED: &str =
        "a UI output whose dispatch (msg-ui §4–§22, msg-units §8; ui/*) is not written yet";
    pub const NPC_TEXT_SHOW: &str =
        "0x27: the overhead text, list start, box and panel (msg-ui §5 r2; ui/*)";
    pub const NPC_DIALOG_UI: &str =
        "0x28: NPC dialog setup and the chosen case's UI calls (msg-ui §16 r4.2–r4.3; ui/*)";
    pub const NPC_DIALOG_END_OTHER: &str =
        "0x62: T = 2, 4, 6: NPC dialog end (1, 0) (msg-ui §17 r3; ui/*)";
    pub const NPC_DIALOG_END_PANEL: &str =
        "0x62: the panel close of menu states 3-7 (msg-ui §17 r2)";
    pub const NPC_DIALOG_M: &str =
        "0x28: the NPC text list dialog selector is not specified (msg-ui §16 r4.3, OQ 10): no case handed back, no C→S 0x31";
}

/// The NPC text list as 0x27 rebuilt it: bytes
/// 6–39 of the message (count u8@0, entry k < 8: kind u8@2+4k, string
/// id u16@4+4k).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpcTextList {
    pub bytes: [u8; 34],
}

impl NpcTextList {
    /// The list rebuilt from a 0x27 record (its 40 bytes).
    pub fn from_record(record: &[u8; 40]) -> Self {
        let mut bytes = [0; 34];
        bytes.copy_from_slice(&record[6..40]);
        Self { bytes }
    }

    pub fn count(&self) -> u8 {
        self.bytes[0]
    }

    /// Entry k's kind (k < 8).
    pub fn kind(&self, k: usize) -> u8 {
        self.bytes[2 + 4 * k]
    }

    /// Entry k's string id (k < 8).
    pub fn string(&self, k: usize) -> u16 {
        u16::from_le_bytes([self.bytes[4 + 4 * k], self.bytes[5 + 4 * k]])
    }

    /// The list as the client's list build and start leave
    /// it: nodes (kind, string id) prepended in message order,
    /// so reversed; a list of two or more nodes is stable-insertion
    /// sorted by string id, ascending (unsigned u16). A count past 8 is
    /// not a list (the build fails at count ≥ 8, [`Self::checked`]).
    pub fn nodes(&self) -> Vec<(u8, u16)> {
        let n = usize::from(self.count()).min(8);
        let mut v: Vec<(u8, u16)> = (0..n)
            .rev()
            .map(|k| (self.kind(k), self.string(k)))
            .collect();
        // Stable insertion sort: `sort_by_key` is stable.
        v.sort_by_key(|&(_, id)| id);
        v
    }

    /// The list build's check: a count of 8 or
    /// more is fatal.
    pub fn checked(self) -> Result<Self, OriginalUiError> {
        if self.count() >= 8 {
            Err(OriginalUiError::NpcTextCount(self.count()))
        } else {
            Ok(self)
        }
    }

    /// The m of 0x28's dialog branch: the first
    /// node of kind 0 in list order (so the smallest kind-0 string id),
    /// 0xFFFF when none.
    pub fn m(&self) -> u16 {
        self.nodes()
            .into_iter()
            .find(|&(k, _)| k == 0)
            .map_or(0xFFFF, |(_, id)| id)
    }

    /// M2: the same for kind 1.
    pub fn m2(&self) -> u16 {
        self.nodes()
            .into_iter()
            .find(|&(k, _)| k == 1)
            .map_or(0xFFFF, |(_, id)| id)
    }
}

/// 0x28's dialog branch, the case the client takes (the first that holds), from the
/// UI state, the NPC text list and the captured inputs. B3–B6 are one case for
/// the bridge ([`DialogCase::Rest`]): they write the model alike.
///
/// Reading taken (as the bridge's): B0 ends the branch, the "(always
/// next)" rows run only when B0 does not hold.
pub fn dialog_case(
    ui_7c0c68: u8,
    txt: Option<&NpcTextList>,
    d: &NpcDialog,
) -> Result<Option<DialogCase>, OriginalUiError> {
    if ui_7c0c68 != 0 {
        return Ok(Some(DialogCase::B0));
    }
    // txt:= the NPC text list: none → fatal 0x1060.
    let txt = txt.ok_or(OriginalUiError::NoNpcText)?;
    let m = txt.m();
    if d.cursor_item {
        return Ok(Some(DialogCase::B1));
    }
    Ok(Some(if m != 0xFFFF {
        DialogCase::B2 { m: u32::from(m) }
    } else {
        DialogCase::Rest
    }))
}

/// The waypoint menu's stored state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaypointMenuState {
    /// The waypoint unit's GUID.
    pub guid: u32,
    /// The record buffer after the load copy.
    pub record: WaypointRecord,
    /// The tab; `None` when its gate needs the client quest flags (skipped).
    pub tab: Option<u8>,
    /// The close latch.
    pub close_latch: bool,
}

/// The UI state the three dispatches write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MsgUiState {
    /// The quest-log latch.
    pub quest_log_latch: bool,
    /// The quest value of S→C 0x5D (f bit 5, c 32).
    pub quest_7bf2ac: Option<i16>,
    /// Act end flag (f bit 1, c 23, expansion game).
    pub act_end_7bc9d8: bool,
    /// Act end flag (f bit 1, c 23, classic game).
    pub act_end_7bc9d4: bool,
    /// The waypoint menu, once a 0x63 opened it.
    pub waypoint: Option<WaypointMenuState>,
    /// The trade state.
    pub trade_state: u8,
    /// Trade closed flag (codes 0x0E / 0x0F, close trade).
    pub trade_7bce28: bool,
    /// Trade value cleared at the trade close.
    pub trade_7c0e80: u32,
    /// The inventory mode.
    pub inventory_mode: u8,
    /// Read by 0x28's dialog branch (case B0). It stays 0:
    /// nothing but the UI-state clear writes it, so B0 never
    /// holds.
    pub ui_7c0c68: u8,
}

impl OriginalUi {
    /// Applies one bridge output at delivery: the UI dispatch of its message,
    /// with the gate facts of `world` now. A `ServerSound` is the audio
    /// layer's: no effect here.
    pub fn apply_output(&mut self, o: &Output, world: &ClientWorld) -> Result<(), OriginalUiError> {
        let r = self.apply_output_inner(o, world);
        self.sync_quest_inputs(o);
        r
    }

    fn apply_output_inner(
        &mut self,
        o: &Output,
        world: &ClientWorld,
    ) -> Result<(), OriginalUiError> {
        self.refresh_facts(world);
        self.imbue_output(o);
        if matches!(o, Output::ChatLine { .. }) {
            // The screen message; the overhead record
            // of type 5 is `chat_line`'s.
            self.game_message(o, world.frames);
            return Ok(());
        }
        if let Output::NpcIntro { slots } = *o {
            // 0x91 also sets the talk's return-greeting flags.
            let rows = self.more.monstats_rows;
            let mut st = self.npcm.borrow_mut();
            for v in slots.into_iter().filter(|&v| u32::from(v) < rows) {
                st.talk.intro().on_0x91(u32::from(v));
            }
        }
        if self.apply_more(o)? {
            return Ok(());
        }
        match *o {
            Output::QuestUi {
                chain,
                flags,
                status,
                extra,
            } => self.quest_ui(chain, flags, status, extra, world),
            Output::WaypointMenu { guid, record } => self.waypoint_menu(guid, &record, world),
            Output::TradeAction {
                code,
                dead_or_absent,
            } => self.trade_action(code, dead_or_absent, world),
            Output::NpcText {
                ref bytes, present, ..
            } => self.npc_text_record(bytes, present),
            Output::NpcDialog(ref d) => self.npc_dialog(d, world),
            Output::NpcTransaction { ref bytes, .. } => {
                self.npc_transaction(bytes);
                Ok(())
            }
            Output::NpcDialogEnd { kind } => {
                self.npc_dialog_end(kind);
                Ok(())
            }
            // Not UI outputs.
            Output::ServerSound { .. } | Output::ShrineSound { .. } => Ok(()),
            _ if o.consumer() != Consumer::Ui => Ok(()),
            // The other UI dispatches (the rest of S→C 0x5D's codes and the unit
            // UI messages) are not written yet.
            _ => {
                self.skip(skip::NOT_APPLIED);
                Ok(())
            }
        }
    }

    /// The UI globals the outputs wrote.
    pub fn msg_state(&self) -> &MsgUiState {
        &self.msg
    }

    /// Sets the B0 flag: the seam for the UI writer not known
    /// yet ([`MsgUiState::ui_7c0c68`]).
    pub fn set_ui_7c0c68(&mut self, v: u8) {
        self.msg.ui_7c0c68 = v;
    }

    /// The NPC text list is freed when the interaction ends
    /// and on game exit.
    pub fn free_npc_text(&mut self) {
        self.npc_text = None;
        self.talk_list();
    }

    /// The NPC text list, `None` when freed.
    pub fn npc_text(&self) -> Option<&NpcTextList> {
        self.npc_text.as_ref()
    }

    /// The case of 0x28's dialog branch chosen since the last call, with its
    /// output, for `Bridge::npc_dialog_branch` (open question 10 decided as A).
    /// Taken by the output dispatcher right after the `NpcDialog` it answers,
    /// before the next output.
    pub fn take_dialog_answer(&mut self) -> Option<(Box<NpcDialog>, DialogCase)> {
        self.dialog_answer.take()
    }

    /// At delivery: the list part of the NPC text record; what is shown is
    /// skipped.
    fn npc_text_record(&mut self, bytes: &[u8; 40], present: bool) -> Result<(), OriginalUiError> {
        let list = NpcTextList::from_record(bytes);
        match bytes[1] {
            // r2.1: an overhead number, the list stays.
            1 if present && list.count() == 1 && list.kind(0) == 3 => {
                let guid = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
                let text = list.string(0).to_string();
                self.set_overhead(UnitKey::new(1, guid), text.as_bytes(), self.more.own_lang);
            }
            1 => self.npc_text = Some(list.checked()?),
            // r2.2: the box or the panel; the list stays.
            2 => {}
            // r2.3.
            _ => self.npc_text = None,
        }
        self.talk_list();
        self.skip(skip::NPC_TEXT_SHOW);
        Ok(())
    }

    /// (T = 1): the NPC panel closes by the menu state. State 1 (the menu box
    /// up): the menu box closes, then the interaction's end (C→S 0x30
    /// [1][GUID] of the open interaction, whatever 0x62's GUID). Any state but
    /// 1 and 3–7, with the B2 talk flag set (a B2 dialog still talking): the
    /// same. The other panels (3–7) and the T = 2, 4, 6 call are skipped.
    fn npc_dialog_end(&mut self, kind: u8) {
        if kind != 1 {
            self.skip(skip::NPC_DIALOG_END_OTHER);
            return;
        }
        let mut st = self.npcm.borrow_mut();
        match st.menu_state {
            1 => {
                st.talk.menu_after = None;
                if let Some(g) = st.interaction_guid() {
                    let out = st.end_interaction(g);
                    st.push_pending(out);
                }
            }
            3..=7 => {
                drop(st);
                self.skip(skip::NPC_DIALOG_END_PANEL);
            }
            _ => {
                if let Some(out) = st.end_b2_dialog() {
                    st.push_pending(out);
                }
            }
        }
    }

    /// At delivery: the UI-only calls are skipped; the branch case
    /// is chosen and kept for the bridge.
    fn npc_dialog(&mut self, d: &NpcDialog, world: &ClientWorld) -> Result<(), OriginalUiError> {
        // r4.1: overlay 72 off on each captured `npc` monster.
        for &unit in &d.npc_monsters {
            self.more.overlays.push(OverlayCall {
                unit,
                id: more::OVERLAY_NPC_WANTS,
                on: false,
            });
        }
        // The client quest flags:= Q.
        self.more.client_quest = d.quest_flags;
        self.skip(skip::NPC_DIALOG_UI);
        // The menu box (`npc_box`) opens here. The facts
        // are captured at receive, not read
        // from the end-of-frame model.
        let (level, n) = (d.level, d.unidentified);
        // The Resurrect edit while the mercenary is
        // dead (the hireling slot set, S→C 0x9B) in an expansion game.
        let expansion = d.expansion && self.shared.borrow().config.expansion_installed;
        self.npcm.borrow_mut().resurrect = (expansion && self.more.merc_state != 0xFFFF)
            .then_some(u32::from(self.more.merc_7c0dd0));
        self.npcm.borrow_mut().merc_name = self.more.merc_state;
        self.hire.borrow_mut().merc_state = self.more.merc_state;
        let case = dialog_case(self.msg.ui_7c0c68, self.npc_text.as_ref(), d)?;
        // The 0x28 handler sets the interact NPC
        // (set in B0 and B2 directly and in B3 by the menu build, by the menu open
        // in B6, its own writes in B1 and B5); B4's end
        // clears it again with its C→S 0x30.
        if case.is_some() {
            self.more.interact_active = true;
            self.more.interact_npc = d.guid;
        }
        if let Some(DialogCase::B2 { m }) = case {
            // B2 plays the dialog line and the
            // menu opens when it ends.
            let mut st = self.npcm.borrow_mut();
            st.talk.pending_open = Some((d.guid, m as u16));
            st.talk.menu_after = Some((d.guid, d.class, level, n));
            // The current B2 line:= m and m is the first played line.
            st.talk.cur = m as u16;
            st.talk.played = vec![m as u16];
        } else {
            self.open_npc_menu_with(d.guid, d.class, level, n, world);
        }
        match case {
            Some(case) => {
                self.npc_speech(d, case);
                self.dialog_answer = Some((Box::new(d.clone()), case));
            }
            None => self.skip(skip::NPC_DIALOG_M),
        }
        Ok(())
    }

    /// The speech of 0x28's dialog branch (in the order of the table): B2 plays
    /// the dialog line of `m` (the unit dialog open with place 1), B3 and B6 the NPC's
    /// greeting; B0, B1, B4 and B5 make none. B3 holds when m2 is not 0xFFFF.
    fn npc_speech(&mut self, d: &NpcDialog, case: DialogCase) {
        let request = match case {
            DialogCase::B2 { m } => SoundRequest::NpcDialogLine {
                npc: d.unit,
                class: d.class,
                key: m as i32,
            },
            DialogCase::Rest => {
                let m2 = self.npc_text.as_ref().map_or(0xFFFF, NpcTextList::m2);
                let b3 = m2 != 0xFFFF;
                let b5 = d.f4b1a10.is_some_and(|v| v != 0);
                if !b3 && !(d.interact && !b5) {
                    return;
                }
                SoundRequest::NpcGreeting {
                    npc: d.unit,
                    class: d.class,
                }
            }
            DialogCase::B0 | DialogCase::B1 => return,
        };
        self.outcome.sounds.push(request);
    }

    fn skip(&mut self, what: &'static str) {
        self.outcome.skipped.push(what);
    }

    fn sound(&mut self, id: i32) {
        self.outcome.sounds.push(SoundRequest::Ui(id));
    }

    /// Rule 2's output rows, at delivery.
    fn quest_ui(
        &mut self,
        c: u8,
        f: u8,
        // The status byte is read only by the quest-log table parts, which are
        // skipped.
        _s: u8,
        v: i16,
        world: &ClientWorld,
    ) -> Result<(), OriginalUiError> {
        match quest_row(c, f) {
            QuestRow::ScreenMessage(id) => self.game_message_id(id),
            QuestRow::MonsterEffect211 => self.skip(skip::MONSTER_EFFECT),
            QuestRow::Sounds(ids) => {
                for &id in ids {
                    self.sound(id);
                }
            }
            QuestRow::SoundExtra => self.sound(i32::from(v)),
            QuestRow::ActEnd => {
                // An expansion game.
                if world.expansion != 0 {
                    self.msg.act_end_7bc9d8 = true;
                } else {
                    self.msg.act_end_7bc9d4 = true;
                }
            }
            QuestRow::Video7 => self.skip(skip::VIDEO_7),
            QuestRow::SetThenTail => {
                self.msg.quest_7bf2ac = Some(v);
                self.quest_log_tail()?;
            }
            QuestRow::DenCounter => self.skip(skip::DEN_COUNTER),
            QuestRow::Tail => self.quest_log_tail()?,
            // Model rows ran at receive; "nothing" rows.
            QuestRow::Untargetable(_)
            | QuestRow::Eclipse
            | QuestRow::ExitRequested
            | QuestRow::Nothing => {}
        }
        Ok(())
    }

    /// The quest-log tail T.
    fn quest_log_tail(&mut self) -> Result<(), OriginalUiError> {
        if !self.msg.quest_log_latch {
            if self.set_ui(UI_QUESTLOG, ON, false)? {
                // r6.1: the entry of c sets its act's selected slot.
                self.skip(skip::QUEST_LOG_TABLE);
                self.msg.quest_log_latch = true;
            }
        } else if self.is_open(UI_QUESTSCREEN) {
            // r6.2: the entry of c's status byte and slot.
            self.skip(skip::QUEST_LOG_TABLE);
        }
        Ok(())
    }

    /// The waypoint menu.
    fn waypoint_menu(
        &mut self,
        guid: u32,
        record: &[u8; 16],
        world: &ClientWorld,
    ) -> Result<(), OriginalUiError> {
        // r2.1: jump 1; refused → nothing stored.
        if !self.set_ui(UI_WAYPOINT, ON, true)? {
            return Ok(());
        }
        // r2.2: the input reset, applied by the host to the
        // world clicks ([`OriginalUi::take_input_reset`]).
        self.shared.borrow_mut().input_reset = true;
        // r2.3: the act index of the local player's room's level; none → tab
        // 0.: a ≥ 5 → 0; tab 0 has no gate; the others read the client quest
        // flags.
        let quest = self.more.client_quest;
        let gate = |record: u32| crate::bridge::objects::quest_bit(&quest, record as u8, 0);
        let tab = match world.player_level() {
            None => 0,
            Some(level) => match act_index(u32::from(level)) {
                a if a >= WAYPOINT_TABS => 0,
                a => crate::ui::panels::waypoint::set_tab(a as u8, &gate),
            },
        };
        // r2.4–r2.6: the row rebuild before and after the store.
        self.skip(skip::WAYPOINT_ROWS);
        let record = WaypointRecord::load_copy(record).map_err(OriginalUiError::Waypoint)?;
        self.msg.waypoint = Some(WaypointMenuState {
            guid,
            record,
            tab: Some(tab),
            close_latch: false,
        });
        // The installed menu's rows (`waypoint_ui`).
        let mut sh = self.shared.borrow_mut();
        let seq = sh.waypoint_open.map_or(1, |o| o.seq.wrapping_add(1));
        sh.waypoint_open = Some(super::WaypointOpen {
            guid,
            record,
            current: world.player_level().map_or(0, u32::from),
            seq,
            tab,
        });
        Ok(())
    }

    /// The UI action.
    fn trade_action(
        &mut self,
        code: u8,
        dead_or_absent: bool,
        world: &ClientWorld,
    ) -> Result<(), OriginalUiError> {
        match code {
            0x00 | 0x01 | 0x02 | 0x05 | 0x06 => self.skip(skip::TRADE),
            0x09 => match world.local_player {
                // Player event sound 23 on the local player.
                Some(p) => self
                    .outcome
                    .sounds
                    .push(SoundRequest::PlayerEvent { unit: p, event: 23 }),
                None => self.skip(skip::NO_LOCAL_PLAYER),
            },
            // The trade partner (0x78): player (0, GUID)
            // in S → player event sound 23 on it.
            0x0A => {
                let p = UnitKey::new(PLAYER, self.more.partner_guid);
                if world.units.contains_key(&p) {
                    self.outcome
                        .sounds
                        .push(SoundRequest::PlayerEvent { unit: p, event: 23 });
                }
            }
            0x0C | 0x0D => {
                self.msg.trade_state = 0;
                self.close_trade(code == 0x0D, dead_or_absent)?;
            }
            0x0E => self.msg.trade_7bce28 = true,
            0x0F => self.msg.trade_7bce28 = false,
            // Closes the trade's stash view; the button states it
            // clears belong to the stash panel, not wired.
            0x10 => {
                self.set_ui(UI_STASH, ON, false)?;
                self.msg.inventory_mode = MODE_STASH;
            }
            // Frees the trade's item lists; the lists are not held here.
            0x11 => {
                if matches!(self.msg.inventory_mode, MODE_STASH | MODE_STASH_2) {
                    self.msg.inventory_mode = 0;
                    self.set_ui(UI_STASH, OFF, false)?;
                }
            }
            0x15 => {
                self.set_ui(UI_CUBE, ON, false)?;
                self.msg.inventory_mode = MODE_CUBE;
                // The open clears the close latch and the animation flag.
                let mut sh = self.shared.borrow_mut();
                sh.cube_opened = true;
                sh.cube_anim.set(Default::default());
            }
            // 0x03, 0x04, 0x07, 0x08, 0x0B, 0x12–0x14 and past 0x15.
            _ => {}
        }
        Ok(())
    }

    /// Close trade. The lists it frees are
    /// not held.
    fn close_trade(&mut self, x: bool, dead_or_absent: bool) -> Result<(), OriginalUiError> {
        self.msg.trade_7c0e80 = 0;
        if self.msg.trade_state != TRADE_REFUSED {
            if self.is_open(UI_MPTRADE as u8) {
                self.set_ui(UI_MPTRADE, OFF, false)?;
                self.msg.trade_7bce28 = false;
                self.skip(skip::TRADE_CLOSE_HELPER);
            }
            if x && !dead_or_absent {
                // No item on the cursor (captured) → `SetUIState(1 inventory,
                // toggle, 0)`.
                self.set_ui(1, 2, false)?;
            }
            if self.is_open(UI_CHAT as u8) {
                self.set_ui(UI_CHAT, OFF, false)?;
            }
        }
        // The state is 0 here for both callers: no decline.
        Ok(())
    }
}

/// The waypoint tab count.
pub const WAYPOINT_TABS: u32 = 5;

#[path = "msg_ui_more.rs"]
mod more;
pub use more::{ChatAction, IntroEntry, MsgUiMore, OverheadText, OverlayCall};

#[cfg(test)]
#[path = "msg_ui_tests.rs"]
mod tests;
