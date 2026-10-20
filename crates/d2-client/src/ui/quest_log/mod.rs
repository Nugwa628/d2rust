//! Quest log, client status meanings: the client's copy of the status list,
//! the row derivation, the tab build and selection, the icon
//! states and the level-entry quest check. Plain Rust: the Bevy
//! layer calls it with the received messages and draws the rows.
//!
//! The server only sends one status byte per quest; this module combines it
//! with the client's copies of the player's and the game's quest flag records
//! and picks the status row (text, speech, icon state). It decides no outcome:
//! the only message it asks for is the C→S 0x58 acknowledge, returned as an
//! [`IconEffect`].

pub mod icons;
pub mod state;
pub mod tables;

use tables::{table_of, NULL_STRING, PLACEHOLDER_STRING};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_entries;

/// A 96-byte quest flag record as received (S→C 0x28 / 0x29): 48 u16 slots,
/// kept exactly as received. "P.b" of slot q is bit b of slot q.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestFlags {
    words: [u16; 48],
}

impl Default for QuestFlags {
    fn default() -> Self {
        QuestFlags { words: [0; 48] }
    }
}

impl QuestFlags {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: slot q's word.
    pub fn with(mut self, q: u8, word: u16) -> Self {
        self.set_word(q, word);
        self
    }

    pub fn set_word(&mut self, q: u8, word: u16) {
        if let Some(w) = self.words.get_mut(q as usize) {
            *w = word;
        }
    }

    pub fn word(&self, q: u8) -> u16 {
        self.words.get(q as usize).copied().unwrap_or(0)
    }

    pub fn bit(&self, q: u8, b: u8) -> bool {
        self.word(q) & (1 << b) != 0
    }

    pub fn set_bit(&mut self, q: u8, b: u8) {
        let w = self.word(q) | (1 << b);
        self.set_word(q, w);
    }
}

/// What the row's description pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowText {
    /// No text (title only or nothing).
    Empty,
    /// String id.
    Id(u16),
    /// String id followed by the count in decimal (`%d` append).
    Append(u16, i32),
    /// String id used as a `%d` format filled with the value.
    Format(u16, i32),
}

/// The four icon states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// 0: just completed (animation, then P.12 and C→S 0x58).
    JustCompleted = 0,
    /// 1: completed.
    Completed = 1,
    /// 2: not available.
    NotAvailable = 2,
    /// 3: in progress.
    InProgress = 3,
}

/// One quest-log row (the 0x26A-byte row buffer, the fields
/// derives).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub quest: u8,
    pub title: u16,
    pub text: RowText,
    pub speech: u16,
    pub shown: u8,
    pub changed: bool,
    pub icon: IconState,
}

/// What a derivation reads besides the received status.
#[derive(Debug, Clone, Copy)]
pub struct RowCtx<'a> {
    /// Player record P.
    pub p: &'a QuestFlags,
    /// Game record G (`None` = not received).
    pub g: Option<&'a QuestFlags>,
    /// Game type ≠ 0.
    pub multiplayer: bool,
    /// Den of Evil monsters left (D).
    pub den: i32,
    /// Barbarians left (B).
    pub barbarians: i32,
}

/// Title 22622 (Rescue on Mount Arreat) and its `%d` text 22624.
const RESCUE_TITLE: u16 = 22622;
const RESCUE_FORMAT: u16 = 22624;
/// `qststhankyoucomeagain`: cannot complete in this game.
const THANK_YOU: u16 = 3729;
const PREVIOUS: u16 = 3728;
const COMPLETE: u16 = 3726;
const OTHER: u16 = 3727;
/// `qstsa1q140`: one Den of Evil monster left.
const DEN_ONE_LEFT: u16 = 3739;
/// `qstsa3q53`.
const BLACKENED_TEXT: u16 = 989;

/// Entries the derivation names.
const DEN_OF_EVIL: u8 = 1;
const SEVEN_TOMBS: u8 = 14;
const BLADE: u8 = 19;
const BLACKENED_TEMPLE: u8 = 21;
const FALLEN_ANGEL: u8 = 25;
const HELLS_FORGE: u8 = 27;
const SIEGE: u8 = 35;
const PRISON_OF_ICE: u8 = 37;
const BETRAYAL: u8 = 38;

#[derive(Clone, Copy)]
enum Step {
    Rule4,
    Rule5,
    Rule6,
}

/// Derive the row of quest `q` from the received status
/// `received` (S\[q\]), the flag records and `last[q]`. `last` is
/// rewritten as the game does.
pub fn derive_row(q: u8, received: u8, ctx: &RowCtx<'_>, last: &mut u8) -> Row {
    let mut row = Row {
        quest: q,
        title: PLACEHOLDER_STRING,
        text: RowText::Empty,
        speech: PLACEHOLDER_STRING,
        shown: received,
        changed: false,
        icon: IconState::InProgress,
    };
    let Some(t) = table_of(q) else {
        row.icon = IconState::NotAvailable;
        return row;
    };
    let p = ctx.p;
    let pb = |b: u8| p.bit(q, b);
    let mut l = received;
    // X(s) / pending rows: shown, title, text and speech of row s.
    let fill = |row: &mut Row, last: &mut u8, s: u8| {
        let (text, speech) = t.row(s);
        row.shown = s;
        row.title = t.title;
        row.text = RowText::Id(text);
        row.speech = speech;
        *last = s;
    };

    // Rule 1: Siege with P.1.
    let mut step;
    if q == SIEGE && pb(1) {
        if l == 0 {
            l = 4;
        }
        return mode_m(row, t, l, ctx, last);
    } else if q == SEVEN_TOMBS && !pb(0) {
        // Rule 2.
        if pb(13) {
            step = Step::Rule4;
        } else if (p.bit(12, 0) || p.bit(12, 13)) && l == 1 {
            fill(&mut row, last, 7);
            row.icon = IconState::InProgress;
            return row;
        } else if p.bit(14, 3) {
            fill(&mut row, last, 5);
            return row;
        } else if p.bit(14, 4) {
            fill(&mut row, last, 6);
            return row;
        } else {
            step = Step::Rule5;
        }
    } else if pb(0) && pb(13) {
        // Rule 3: C(13, 3726).
        return completed_row(row, t, 13, COMPLETE, p, q, last);
    } else if pb(0) {
        return completed_row(row, t, 11, PREVIOUS, p, q, last);
    } else if pb(13) {
        step = Step::Rule4;
    } else {
        step = Step::Rule5;
    }

    if let Step::Rule4 = step {
        // Rule 4: reward pending.
        if t.pending == 0xFFFF {
            step = Step::Rule5;
        } else if !pb(1) {
            step = Step::Rule6;
        } else if q == FALLEN_ANGEL {
            step = Step::Rule5;
        } else {
            let s = (t.pending & 0xFF) as u8 + 1;
            let changed = s != *last;
            fill(&mut row, last, s);
            row.changed = changed;
            row.icon = IconState::InProgress;
            return row;
        }
    }
    if let Step::Rule5 = step {
        // Rule 5: reward still pending from earlier.
        if !pb(1) || !pb(15) {
            step = Step::Rule6;
        } else {
            let s = if q == PRISON_OF_ICE {
                if pb(8) && !pb(9) {
                    6
                } else if !pb(8) {
                    5
                } else {
                    10
                }
            } else if q == BETRAYAL {
                if pb(4) {
                    5
                } else {
                    4
                }
            } else {
                10
            };
            row.shown = s;
            let (text, speech) = t.row(s);
            if text == NULL_STRING {
                row.icon = IconState::NotAvailable;
                return row;
            }
            row.title = t.title;
            row.text = RowText::Id(text);
            row.speech = speech;
            *last = s;
            row.icon = if q == HELLS_FORGE {
                IconState::Completed
            } else {
                IconState::InProgress
            };
            return row;
        }
    }
    debug_assert!(matches!(step, Step::Rule6));
    // Rule 6.
    if l != 0 {
        mode_m(row, t, l, ctx, last)
    } else {
        mode_z(row, q, ctx)
    }
}

fn completed_row(
    mut row: Row,
    t: &tables::QuestTable,
    s: u8,
    id: u16,
    p: &QuestFlags,
    q: u8,
    last: &mut u8,
) -> Row {
    row.shown = s;
    row.title = t.title;
    row.text = RowText::Id(id);
    row.speech = t.completed;
    *last = s;
    row.icon = if p.bit(q, 12) {
        IconState::Completed
    } else {
        IconState::JustCompleted
    };
    row
}

fn mode_m(mut row: Row, t: &tables::QuestTable, l: u8, ctx: &RowCtx<'_>, last: &mut u8) -> Row {
    let q = row.quest;
    let p = ctx.p;
    row.title = t.title;
    let (text, speech) = t.row(l);
    if q == DEN_OF_EVIL {
        // Rule 7.1.
        row.text = if l == 3 || l == 4 {
            if ctx.den > 1 {
                RowText::Append(text, ctx.den)
            } else {
                RowText::Id(DEN_ONE_LEFT)
            }
        } else {
            RowText::Id(text)
        };
        row.shown = l;
    } else {
        // Rule 7.2.
        let by_game = !p.bit(q, 0)
            && !p.bit(q, 13)
            && !p.bit(q, 1)
            && ctx.g.is_some_and(|g| g.bit(q, 13))
            && q != BLADE;
        let mut id = if by_game || p.bit(q, 14) {
            THANK_YOU + u16::from(ctx.multiplayer)
        } else if text == OTHER && !ctx.multiplayer {
            THANK_YOU
        } else {
            text
        };
        row.text = RowText::Id(id);
        if row.title == RESCUE_TITLE && id == RESCUE_FORMAT {
            if ctx.barbarians == 0 {
                id = THANK_YOU;
                row.text = RowText::Id(id);
            } else {
                row.text = RowText::Format(id, ctx.barbarians);
            }
        }
    }
    // Rule 7.3.
    row.changed = l != *last;
    *last = l;
    row.speech = speech;
    row.icon = IconState::InProgress;
    if l == 13 {
        row.icon = if p.bit(q, 12) {
            IconState::Completed
        } else {
            IconState::JustCompleted
        };
    }
    row
}

fn mode_z(mut row: Row, q: u8, ctx: &RowCtx<'_>) -> Row {
    let p = ctx.p;
    let again = THANK_YOU + u16::from(ctx.multiplayer);
    let g13 = ctx.g.is_some_and(|g| g.bit(q, 13));
    let text = if g13 {
        if q == BLACKENED_TEMPLE && !p.bit(q, 0) && p.bit(q, 4) {
            BLACKENED_TEXT
        } else if p.bit(q, 13) || p.bit(q, 1) {
            row.icon = IconState::NotAvailable;
            return row;
        } else {
            again
        }
    } else {
        // The text rule:
        // G.13 clear shows the text only when G.15 is set and P.13 and P.1
        // are both clear; any other case ends in icon state 2 (the test
        // vector q 9 with empty P and G ends in state 2).
        let g15 = ctx.g.is_some_and(|g| g.bit(q, 15));
        if !g15 || p.bit(q, 13) || p.bit(q, 1) {
            row.icon = IconState::NotAvailable;
            return row;
        }
        again
    };
    row.text = RowText::Id(text);
    row.speech = NULL_STRING;
    row.icon = IconState::InProgress;
    row
}
