//! World systems: quests ([`quests`]), waypoints ([`waypoints`]), the
//! Horadric Cube ([`cube`]), vendors, NPCs, objects, hirelings, the stash
//! buttons ([`stash`]) and the act environment ([`environment`]).
//!
//! Unit fields, items, objects, levels and messages are owned by other
//! parts of the simulation. Each module reaches them through one narrow
//! trait (its seam: [`waypoints::WaypointWorld`], [`cube::CubeWorld`],
//! [`quests::QuestWorld`]); message bytes leave through the seam's `send`,
//! because `d2-sim` may not depend on `d2-proto`. The wiring layer
//! implements these seams over the real units, items and stats. This file
//! also holds the strict tab-separated table reader the tests share.

pub mod cube;
pub mod environment;
pub mod hirelings;
pub mod npc;
pub mod objects;
pub mod quests;
pub mod stash;
pub mod vendors;
pub mod waypoints;

/// Errors from the strict tab-separated table reader: a header row, then
/// rows of exactly the header's column count.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TsvError {
    #[error("{table}: empty file")]
    Empty { table: &'static str },
    #[error("{table}: header is not {want:?}")]
    Header {
        table: &'static str,
        want: &'static [&'static str],
    },
    #[error("{table} line {line}: {got} columns, want {want}")]
    Columns {
        table: &'static str,
        line: usize,
        got: usize,
        want: usize,
    },
    #[error("{table} line {line}: bad value {value:?} in column {column}")]
    Value {
        table: &'static str,
        line: usize,
        column: &'static str,
        value: String,
    },
}

/// The rows of a tab-separated table whose header must equal `header`.
/// Returns (1-based line number, cells) per data row.
pub(crate) fn tsv_rows<'t>(
    table: &'static str,
    text: &'t str,
    header: &'static [&'static str],
) -> Result<Vec<(usize, Vec<&'t str>)>, TsvError> {
    let mut lines = text.lines().enumerate();
    let (_, head) = lines.next().ok_or(TsvError::Empty { table })?;
    if head.split('\t').ne(header.iter().copied()) {
        return Err(TsvError::Header {
            table,
            want: header,
        });
    }
    let mut rows = Vec::new();
    for (i, line) in lines {
        if line.is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        if cells.len() != header.len() {
            return Err(TsvError::Columns {
                table,
                line: i + 1,
                got: cells.len(),
                want: header.len(),
            });
        }
        rows.push((i + 1, cells));
    }
    Ok(rows)
}

/// Parses a decimal or `0x` hexadecimal cell.
pub(crate) fn tsv_num(
    table: &'static str,
    line: usize,
    column: &'static str,
    cell: &str,
) -> Result<u32, TsvError> {
    let parsed = match cell.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => cell.parse(),
    };
    parsed.map_err(|_| TsvError::Value {
        table,
        line,
        column,
        value: cell.to_string(),
    })
}

#[cfg(test)]
mod mutant_tests;
