//! The client's quest-log state and the tab build: the received
//! status list `S`, the last shown status `last`, the Den of Evil / staff
//! tomb / barbarian counters, the remembered tab slots, and the row list of
//! a shown act tab with the selected slot.

use super::tables::{chain_entry, entry, ENTRY_COUNT};
use super::{derive_row, IconState, QuestFlags, Row, RowCtx};

/// Highest icon index with a cel ("its icon ≤ 26").
pub const MAX_ICON: u8 = 26;
/// Rows built per tab.
pub const MAX_ROWS: usize = 6;
/// Act tabs.
pub const TAB_COUNT: usize = 5;
/// The Act V tab.
pub const ACT_V_TAB: u8 = 4;

/// What the 0x52 handler asks the panel layer to do afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogEffect {
    /// Load the panel cels.
    LoadPanelCels,
    /// Open quest tab `tab` ([`QuestLog::open_tab`]); `reset` clears the
    /// remembered slots after the selection is chosen.
    OpenTab { tab: u8, reset: bool },
}

/// A built row with its entry's slot and icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabRow {
    pub slot: u8,
    pub icon: u8,
    pub row: Row,
}

/// Gates of the tab build the caller knows (installs, loaded cels).
#[derive(Clone, Copy)]
pub struct TabGate<'a> {
    /// Expansion installed.
    pub expansion_installed: bool,
    /// Expansion game.
    pub expansion_game: bool,
    /// Whether icon cel k is loaded.
    pub cel_loaded: &'a dyn Fn(u8) -> bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestLog {
    /// `S[0..40]`: each entry's status.
    pub status: [u8; ENTRY_COUNT],
    /// `last[0..40]`: each entry's last shown status.
    pub last: [u8; ENTRY_COUNT],
    /// D, Y, B.
    pub den: i32,
    pub tomb: i32,
    pub barbarians: i32,
    /// Quest-log latch.
    pub latch: u32,
    /// Remembered slot per tab (`None` = −1).
    pub remembered: [Option<u8>; TAB_COUNT],
    /// The tab's last clicked selection.
    pub clicked: [Option<u8>; TAB_COUNT],
    /// Selected slot (`None` = −1, set at every tab open).
    pub selected: Option<u8>,
    /// A flag zeroed by every 0x52.
    pub flag_2b0: u8,
}

impl Default for QuestLog {
    fn default() -> Self {
        QuestLog {
            status: [0; ENTRY_COUNT],
            last: [0; ENTRY_COUNT],
            den: 0,
            tomb: 0,
            barbarians: 0,
            latch: 0,
            remembered: [None; TAB_COUNT],
            clicked: [None; TAB_COUNT],
            selected: Some(0),
            flag_2b0: 0,
        }
    }
}

impl QuestLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// S→C 0x52: the 42 message bytes; byte 1 + i is
    /// `S[i]`. `panel_ready` is "the panel is not ready"; `shown_tab` is
    /// the tab on screen. Effects in call order.
    pub fn receive_0x52(
        &mut self,
        bytes: &[u8; 42],
        panel_ready: bool,
        shown_tab: u8,
    ) -> Vec<LogEffect> {
        self.status.copy_from_slice(&bytes[1..42]);
        self.flag_2b0 = 0;
        let mut out = Vec::new();
        if panel_ready && self.latch == 2 {
            out.push(LogEffect::LoadPanelCels);
            out.push(LogEffect::OpenTab {
                tab: shown_tab,
                reset: true,
            });
        }
        out
    }

    /// S→C 0x50 type 1: D:= u16@3, Y:= u16@5, B:= u16@7. Other types are not
    /// quest-log state.
    pub fn receive_0x50(&mut self, kind: u16, d: u16, y: u16, b: u16) {
        if kind == 1 {
            self.den = i32::from(d);
            self.tomb = i32::from(y);
            self.barbarians = i32::from(b);
        }
    }

    /// S→C 0x5D status write: `S[e]` of the entry whose chain is `chain`,
    /// only while the quest screen (UI state 15) is open. A chain with no
    /// entry writes nothing. Returns the entry written.
    pub fn receive_0x5d(&mut self, chain: u8, status: u8, quest_screen_open: bool) -> Option<u8> {
        if !quest_screen_open {
            return None;
        }
        let e = chain_entry(chain)?;
        self.status[e as usize] = status;
        Some(e)
    }

    /// Zero `S` and `last`.
    pub fn reset_status(&mut self) {
        self.status = [0; ENTRY_COUNT];
        self.last = [0; ENTRY_COUNT];
    }

    /// (game start): also the counters, the latch and the
    /// selection.
    pub fn reset_game(&mut self) {
        self.reset_status();
        self.den = 0;
        self.tomb = 0;
        self.barbarians = 0;
        self.latch = 0;
        self.remembered = [None; TAB_COUNT];
        self.clicked = [None; TAB_COUNT];
        self.selected = Some(0);
    }

    fn ctx<'a>(&self, p: &'a QuestFlags, g: Option<&'a QuestFlags>, mp: bool) -> RowCtx<'a> {
        RowCtx {
            p,
            g,
            multiplayer: mp,
            den: self.den,
            barbarians: self.barbarians,
        }
    }

    fn tab_open(tab: u8, gate: &TabGate<'_>) -> bool {
        tab != ACT_V_TAB || (gate.expansion_installed && gate.expansion_game)
    }

    /// Tab draw: entries in table order; shown when the tab
    /// matches, enabled, icon ≤ 26, its cel loaded and fewer than 6 rows
    /// were built. Each row is derived by the row rule (rewriting `last`).
    pub fn build_tab(
        &mut self,
        tab: u8,
        p: &QuestFlags,
        g: Option<&QuestFlags>,
        multiplayer: bool,
        gate: &TabGate<'_>,
    ) -> Vec<TabRow> {
        let mut rows = Vec::new();
        if !Self::tab_open(tab, gate) {
            return rows;
        }
        let ctx = self.ctx(p, g, multiplayer);
        for i in 0..ENTRY_COUNT as u8 {
            let e = entry(i);
            if e.tab != tab
                || !e.enabled
                || e.icon > MAX_ICON
                || !(gate.cel_loaded)(e.icon)
                || rows.len() >= MAX_ROWS
            {
                continue;
            }
            let row = derive_row(i, self.status[i as usize], &ctx, &mut self.last[i as usize]);
            rows.push(TabRow {
                slot: e.slot,
                icon: e.icon,
                row,
            });
        }
        rows
    }

    /// Tab open: returns the selected
    /// slot (also stored in `selected`), `None` when the tab is not built
    /// (Act V without the expansion) or nothing selects.
    pub fn open_tab(
        &mut self,
        tab: u8,
        reset: bool,
        p: &QuestFlags,
        g: Option<&QuestFlags>,
        multiplayer: bool,
        gate: &TabGate<'_>,
    ) -> Option<u8> {
        // Order: selected:= none first, always; a refused tab ends there; the
        // selection is chosen (a remembered slot without a scan); only then
        // does `reset` clear the slots.
        self.selected = None;
        if !Self::tab_open(tab, gate) {
            return None;
        }
        let pick = self.choose(tab, p, g, multiplayer, gate);
        self.selected = pick;
        if reset {
            self.remembered = [None; TAB_COUNT];
        }
        pick
    }

    fn choose(
        &mut self,
        tab: u8,
        p: &QuestFlags,
        g: Option<&QuestFlags>,
        multiplayer: bool,
        gate: &TabGate<'_>,
    ) -> Option<u8> {
        if let Some(slot) = self.remembered[tab as usize] {
            return Some(slot);
        }
        let ctx = self.ctx(p, g, multiplayer);
        // The scan visits all 41 entries (no 6-row limit).
        let mut any_state_1_to_3 = false;
        let mut first_state3 = None;
        for i in 0..ENTRY_COUNT as u8 {
            let e = entry(i);
            if e.tab != tab || !e.enabled || e.icon > MAX_ICON || !(gate.cel_loaded)(e.icon) {
                continue;
            }
            let row = derive_row(i, self.status[i as usize], &ctx, &mut self.last[i as usize]);
            if row.changed || row.icon == IconState::JustCompleted {
                return Some(e.slot);
            }
            if matches!(
                row.icon,
                IconState::Completed | IconState::NotAvailable | IconState::InProgress
            ) {
                any_state_1_to_3 = true;
            }
            if row.icon == IconState::InProgress && first_state3.is_none() {
                first_state3 = Some(e.slot);
            }
        }
        if !any_state_1_to_3 {
            return None;
        }
        // The clicked slot when one was recorded, otherwise the first
        // state-3 entry (the reading is confirmed).
        self.clicked[tab as usize].or(first_state3)
    }
}

/// The tab strip cels: for tab k = 0 … 4 in an expansion game
/// (0 … 3 otherwise), `(x, frame, dy)` with x the strip offset (5, 0x43, 0x81,
/// 0xBF, 0xFD in an expansion game; 5, 0x52, 0x9F, 0xEC otherwise), dy 1 in an
/// expansion game, else 0. The shown tab draws frame 2k; any other tab draws 2k +
/// 1 when its act is reachable: tab 0 always, tab 1 / 2 / 3 / 4 when the
/// player's quest flag (7 / 15 / 23 / 26, bit 0)
/// is set.
pub fn tab_strip(shown: u8, p: &QuestFlags, expansion_game: bool) -> Vec<(i32, u32, i32)> {
    let (xs, dy): (&[i32], i32) = if expansion_game {
        (&[5, 0x43, 0x81, 0xBF, 0xFD], 1)
    } else {
        (&[5, 0x52, 0x9F, 0xEC], 0)
    };
    let mut out = Vec::new();
    for (k, &x) in xs.iter().enumerate() {
        let frame = if k == usize::from(shown) {
            2 * k as u32
        } else {
            let reachable = match k {
                0 => true,
                1 => p.bit(7, 0),
                2 => p.bit(15, 0),
                3 => p.bit(23, 0),
                _ => p.bit(26, 0),
            };
            if !reachable {
                continue;
            }
            2 * k as u32 + 1
        };
        out.push((x, frame, dy));
    }
    out
}

/// The selected row's title is drawn unless 3724.
pub fn title_drawn(title: u16) -> bool {
    title != super::tables::PLACEHOLDER_STRING
}

/// The text is drawn (wrapped at 270 px) unless a speech replay
/// runs.
pub const TEXT_WRAP_PX: i32 = 0x10E;

pub fn text_drawn(replay_running: bool) -> bool {
    !replay_running
}

/// The replay button plays the row's speech unless it is 3724 or
/// 3725.
pub fn replay_plays(speech: u16) -> bool {
    speech != super::tables::PLACEHOLDER_STRING && speech != super::tables::NULL_STRING
}

/// Entry range of a tab's icon loads.
pub fn tab_icon_range(tab: u8) -> Option<(u8, u8)> {
    super::tables::TAB_ICON_RANGE.get(tab as usize).copied()
}

#[cfg(test)]
mod tab_strip_tests {
    use super::*;

    #[test]
    fn the_strip_draws_one_cel_per_reachable_tab() {
        let mut p = QuestFlags::default();
        // Act I shown, nothing reached: tab 0 selected only.
        assert_eq!(tab_strip(0, &p, true), vec![(5, 0, 1)]);
        // Act II shown with its flag set: tab 0 unselected, tab 1 selected.
        p.set_bit(7, 0);
        assert_eq!(tab_strip(1, &p, true), vec![(5, 1, 1), (0x43, 2, 1)]);
        // Act IV shown: the selected tab draws whatever its flag.
        assert_eq!(
            tab_strip(3, &p, true),
            vec![(5, 1, 1), (0x43, 3, 1), (0xBF, 6, 1)]
        );
        // Without the expansion four tabs at the classic offsets.
        assert_eq!(tab_strip(0, &p, false), vec![(5, 0, 0), (0x52, 3, 0)]);
    }
}
