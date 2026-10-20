// Spec: specs/world/quests.md §2.4 (quests.tsv), §7.1 (quest-messages.tsv)
//! The two machine tables of the quest spec, embedded and parsed strictly
//! (METHODS M05, M07). They describe hard-coded `Game.exe` tables (the
//! quest init table and the NPC message tables), so they are the data.

use crate::world::{tsv_num, tsv_rows, TsvError};

/// `tables/world/quests.tsv`.
pub const QUESTS_TSV: &str = include_str!("../../../../../tables/world/quests.tsv");
/// `tables/world/quest-messages.tsv`.
pub const MESSAGES_TSV: &str = include_str!("../../../../../tables/world/quest-messages.tsv");

const QUESTS_HEADER: &[&str] = &[
    "index",
    "chain",
    "flag",
    "act",
    "version",
    "no_set_state",
    "init",
    "filter",
    "flag2",
    "init_no",
    "seq_id",
    "callbacks",
    "status_fn",
    "active_fn",
    "seq_fn",
    "msgs",
    "name",
    "spec",
];

const MESSAGES_HEADER: &[&str] = &[
    "table", "entries", "chain", "state", "slot", "npc", "string", "menu",
];

/// One row of `quests.tsv`. `None` = `-` (not stored, 0) or `?` (row 40,
/// not disassembled; see [`QuestRow::unknown`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestRow {
    pub index: u8,
    pub chain: u8,
    /// Table flag slot (`None` for intros).
    pub flag: Option<u8>,
    pub act: u8,
    pub version: Option<u32>,
    pub no_set_state: Option<bool>,
    pub init: u32,
    pub filter: Option<u8>,
    pub flag2: Option<u8>,
    pub init_no: Option<u8>,
    pub seq_id: Option<u8>,
    /// (event id, function) per non-null callback.
    pub callbacks: Vec<(u8, u32)>,
    pub status_fn: Option<u32>,
    pub active_fn: Option<u32>,
    pub seq_fn: Option<u32>,
    pub msgs: Option<u32>,
    pub name: String,
    /// `specified` in the spec column.
    pub specified: bool,
    /// Row values were `?` (Act V intro, open question 6).
    pub unknown: bool,
}

/// One entry of an NPC message table (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageEntry {
    pub table: u32,
    pub state: u8,
    pub slot: u8,
    pub npc: u16,
    pub string: u16,
    pub menu: u32,
}

/// Both tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestTables {
    /// Init-table rows 0–36, then intros 37–40.
    pub rows: Vec<QuestRow>,
    pub messages: Vec<MessageEntry>,
}

fn opt_num(
    t: &'static str,
    line: usize,
    col: &'static str,
    c: &str,
) -> Result<Option<u32>, TsvError> {
    match c {
        "-" | "?" => Ok(None),
        _ => tsv_num(t, line, col, c).map(Some),
    }
}

fn opt_fn(
    t: &'static str,
    line: usize,
    col: &'static str,
    c: &str,
) -> Result<Option<u32>, TsvError> {
    Ok(opt_num(t, line, col, c)?.filter(|&f| f != 0))
}

fn small(t: &'static str, line: usize, col: &'static str, v: u32) -> Result<u8, TsvError> {
    u8::try_from(v).map_err(|_| TsvError::Value {
        table: t,
        line,
        column: col,
        value: v.to_string(),
    })
}

fn opt_small(
    t: &'static str,
    line: usize,
    col: &'static str,
    c: &str,
) -> Result<Option<u8>, TsvError> {
    opt_num(t, line, col, c)?
        .map(|v| small(t, line, col, v))
        .transpose()
}

/// Parses `quests.tsv`.
pub fn parse_quests(text: &str) -> Result<Vec<QuestRow>, TsvError> {
    let t = "quests.tsv";
    let mut rows = Vec::new();
    for (line, c) in tsv_rows(t, text, QUESTS_HEADER)? {
        let mut callbacks = Vec::new();
        if c[11] != "?" && c[11] != "-" {
            for cb in c[11].split(' ') {
                let bad = || TsvError::Value {
                    table: t,
                    line,
                    column: "callbacks",
                    value: cb.to_string(),
                };
                let (ev, f) = cb.split_once(':').ok_or_else(bad)?;
                let ev = small(t, line, "callbacks", tsv_num(t, line, "callbacks", ev)?)?;
                if ev > 14 {
                    return Err(bad());
                }
                callbacks.push((ev, tsv_num(t, line, "callbacks", f)?));
            }
        }
        let specified = match c[17] {
            "specified" => true,
            "catalogued" => false,
            v => {
                return Err(TsvError::Value {
                    table: t,
                    line,
                    column: "spec",
                    value: v.to_string(),
                })
            }
        };
        rows.push(QuestRow {
            index: small(t, line, "index", tsv_num(t, line, "index", c[0])?)?,
            chain: small(t, line, "chain", tsv_num(t, line, "chain", c[1])?)?,
            flag: opt_small(t, line, "flag", c[2])?,
            act: small(t, line, "act", tsv_num(t, line, "act", c[3])?)?,
            version: opt_num(t, line, "version", c[4])?,
            no_set_state: opt_num(t, line, "no_set_state", c[5])?.map(|v| v != 0),
            init: tsv_num(t, line, "init", c[6])?,
            filter: opt_small(t, line, "filter", c[7])?,
            flag2: opt_small(t, line, "flag2", c[8])?,
            init_no: opt_small(t, line, "init_no", c[9])?,
            seq_id: opt_small(t, line, "seq_id", c[10])?,
            callbacks,
            status_fn: opt_fn(t, line, "status_fn", c[12])?,
            active_fn: opt_fn(t, line, "active_fn", c[13])?,
            seq_fn: opt_fn(t, line, "seq_fn", c[14])?,
            msgs: opt_fn(t, line, "msgs", c[15])?,
            name: c[16].to_string(),
            specified,
            unknown: c[7] == "?",
        });
    }
    Ok(rows)
}

/// Parses `quest-messages.tsv` (the `entries` and `chain` columns are a
/// catalogue cross-reference and are only syntax-checked).
pub fn parse_messages(text: &str) -> Result<Vec<MessageEntry>, TsvError> {
    let t = "quest-messages.tsv";
    let mut out = Vec::new();
    for (line, c) in tsv_rows(t, text, MESSAGES_HEADER)? {
        for col in [1, 2] {
            for v in c[col].split(',') {
                tsv_num(t, line, MESSAGES_HEADER[col], v)?;
            }
        }
        let npc = tsv_num(t, line, "npc", c[5])?;
        let string = tsv_num(t, line, "string", c[6])?;
        let bad = |column: &'static str, v: u32| TsvError::Value {
            table: t,
            line,
            column,
            value: v.to_string(),
        };
        out.push(MessageEntry {
            table: tsv_num(t, line, "table", c[0])?,
            state: small(t, line, "state", tsv_num(t, line, "state", c[3])?)?,
            slot: small(t, line, "slot", tsv_num(t, line, "slot", c[4])?)?,
            npc: u16::try_from(npc).map_err(|_| bad("npc", npc))?,
            string: u16::try_from(string).map_err(|_| bad("string", string))?,
            menu: tsv_num(t, line, "menu", c[7])?,
        });
    }
    Ok(out)
}

impl QuestTables {
    /// The embedded tables.
    pub fn load() -> Result<Self, TsvError> {
        Self::parse(QUESTS_TSV, MESSAGES_TSV)
    }

    /// Both tables from text.
    pub fn parse(quests: &str, messages: &str) -> Result<Self, TsvError> {
        Ok(Self {
            rows: parse_quests(quests)?,
            messages: parse_messages(messages)?,
        })
    }

    /// `0x00543790`'s source: entries of `table` in `state` for `npc`, in
    /// slot order.
    pub fn messages_for(
        &self,
        table: u32,
        state: u8,
        npc: u16,
    ) -> impl Iterator<Item = &MessageEntry> {
        self.messages
            .iter()
            .filter(move |m| m.table == table && m.state == state && m.npc == npc)
    }
}
