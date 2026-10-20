//! Rendering facts (`facts/render/`): the tab-separated format of `draws.tsv`, `frame.tsv` and
//! `sprites.tsv`, the D2RUST exporter ([`export`]) and the first-difference compare
//! ([`compare`]).
//!
//! A "fact set" is a directory with those three files. `draws.tsv` lists every draw call of one
//! frame in order, `frame.tsv` holds the inputs the frame was drawn from and the outputs it
//! presented, and `sprites.tsv` holds the measured size and offset of each sprite frame. The
//! exporter writes this from D2RUST's own frame; the compare reads two sets (D2RUST's and a
//! reference one) and reports the first difference.
//!
//! Plain Rust, no Bevy types: the exporter reads a built [`crate::world_view::WorldFrame`] and
//! the frame store; the play loop only calls it.

pub mod compare;
pub mod export;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// The format version written in (and required of) the header line.
pub const VERSION: &str = "v1";

/// `draws.tsv` columns, in file order.
pub const DRAW_COLUMNS: [&str; 17] = [
    "i", "op", "file", "dir", "frame", "tile", "x", "y", "w", "h", "xoff", "yoff", "mode", "light",
    "pal", "unit", "at",
];

/// `frame.tsv` columns, in file order.
pub const FRAME_COLUMNS: [&str; 2] = ["key", "value"];

/// `sprites.tsv` columns, in file order.
pub const SPRITE_COLUMNS: [&str; 7] = ["file", "dir", "frame", "w", "h", "xoff", "yoff"];

/// What a `frame.tsv` key is to the compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Not compared.
    Info,
    /// State the frame is drawn from: compared first.
    Input,
    /// What the frame presented: compared last.
    Output,
}

/// `frame.tsv` keys in file order, each with its [`Role`].
pub const FRAME_KEYS: [(&str, Role); 20] = [
    ("seq", Role::Info),
    ("tick", Role::Input),
    ("w", Role::Input),
    ("h", Role::Input),
    ("act", Role::Input),
    ("level", Role::Input),
    ("player_x", Role::Input),
    ("player_y", Role::Input),
    ("tile_origin_x", Role::Input),
    ("tile_origin_y", Role::Input),
    ("unit_origin_x", Role::Input),
    ("unit_origin_y", Role::Input),
    ("open_mode", Role::Input),
    ("shift_x", Role::Input),
    ("light_quality", Role::Input),
    ("rain", Role::Input),
    ("snow", Role::Input),
    ("draws", Role::Output),
    ("index_sha256", Role::Output),
    ("palette_sha256", Role::Output),
];

/// Cell value meaning "not applicable to this kind of row".
pub const NA: &str = "-";
/// Cell value meaning "this producer did not measure it"; the compare skips such cells.
pub const UNKNOWN: &str = "?";

/// File names inside a scene / dump directory.
///
/// The draw list file.
pub const DRAWS_FILE: &str = "draws.tsv";
/// The per-frame inputs and outputs file.
pub const FRAME_FILE: &str = "frame.tsv";
/// The sprite size file.
pub const SPRITES_FILE: &str = "sprites.tsv";

/// Why reading, parsing or exporting a fact file failed.
#[derive(Debug, thiserror::Error)]
pub enum FactsError {
    /// The file could not be read.
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file's text breaks the format (header, columns, field count, empty field).
    #[error("{file}: line {line}: {why}")]
    Format {
        file: String,
        line: usize,
        why: String,
    },
    /// The exporter could not build the facts.
    #[error("export: {0}")]
    Export(String),
}

/// The header line's fields: `# facts v1; tool: …; command: …; game: …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The program that wrote the file.
    pub tool: String,
    /// The command line or scene that produced it.
    pub command: String,
    /// The game build the facts describe.
    pub game: String,
}

impl Header {
    /// The line, without its `\n`.
    pub fn line(&self) -> String {
        format!(
            "# facts {VERSION}; tool: {}; command: {}; game: {}",
            self.tool, self.command, self.game
        )
    }

    fn parse(line: &str) -> Result<Header, String> {
        let rest = line
            .strip_prefix(&format!("# facts {VERSION}; "))
            .ok_or_else(|| format!("header must start with `# facts {VERSION}; `"))?;
        let (tool, rest) = rest
            .strip_prefix("tool: ")
            .and_then(|r| r.split_once("; command: "))
            .ok_or("header needs `tool: …; command: …`")?;
        let (command, game) = rest
            .rsplit_once("; game: ")
            .ok_or("header needs `; game: …` last")?;
        if tool.is_empty() || command.is_empty() || game.is_empty() {
            return Err("header field is empty".into());
        }
        Ok(Header {
            tool: tool.to_owned(),
            command: command.to_owned(),
            game: game.to_owned(),
        })
    }
}

/// One parsed fact file: its header and rows (each exactly as many
/// fields as the columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The parsed header line.
    pub header: Header,
    /// Data rows (after the column row).
    pub rows: Vec<Vec<String>>,
}

/// Parses `text` as a fact file with `columns`: header line, column row, then rows with exactly
/// that many non-empty fields, every line ending in `\n`. `file` names it in errors.
pub fn parse(file: &str, text: &str, columns: &[&str]) -> Result<Table, FactsError> {
    let err = |line: usize, why: String| FactsError::Format {
        file: file.to_owned(),
        line,
        why,
    };
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(err(text.lines().count(), "last line has no `\\n`".into()));
    }
    let mut lines = text.lines();
    let header = Header::parse(lines.next().ok_or_else(|| err(1, "empty file".into()))?)
        .map_err(|w| err(1, w))?;
    let names = lines.next().ok_or_else(|| err(2, "no column row".into()))?;
    if names.split('\t').ne(columns.iter().copied()) {
        return Err(err(
            2,
            format!("columns `{}`, expected `{}`", names, columns.join("\t")),
        ));
    }
    let mut rows = Vec::new();
    for (n, line) in lines.enumerate() {
        let fields: Vec<String> = line.split('\t').map(str::to_owned).collect();
        if fields.len() != columns.len() {
            return Err(err(
                n + 3,
                format!("{} fields, expected {}", fields.len(), columns.len()),
            ));
        }
        if let Some(f) = fields.iter().find(|f| f.is_empty()) {
            return Err(err(n + 3, format!("empty field {f:?}")));
        }
        rows.push(fields);
    }
    Ok(Table { header, rows })
}

/// Writes a fact file: header line, column row, then one tab-separated line per row.
pub fn write(header: &Header, columns: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = header.line();
    out.push('\n');
    out.push_str(&columns.join("\t"));
    out.push('\n');
    for r in rows {
        out.push_str(&r.join("\t"));
        out.push('\n');
    }
    out
}

/// Reads and parses one file of a directory.
pub fn read(dir: &Path, name: &str, columns: &[&str]) -> Result<Table, FactsError> {
    let path = dir.join(name);
    let text = std::fs::read_to_string(&path).map_err(|source| FactsError::Io {
        path: path.clone(),
        source,
    })?;
    parse(&path.display().to_string(), &text, columns)
}

/// Checks a parsed `frame.tsv` has every key once, in [`FRAME_KEYS`] order.
pub fn check_frame_keys(file: &str, t: &Table) -> Result<(), FactsError> {
    let keys: Vec<&str> = t.rows.iter().map(|r| r[0].as_str()).collect();
    let want: Vec<&str> = FRAME_KEYS.iter().map(|(k, _)| *k).collect();
    if keys != want {
        let at = keys
            .iter()
            .zip(&want)
            .position(|(a, b)| a != b)
            .unwrap_or(keys.len().min(want.len()));
        return Err(FactsError::Format {
            file: file.to_owned(),
            line: at + 3,
            why: format!(
                "key {:?}, expected {:?}",
                keys.get(at).copied().unwrap_or("<end>"),
                want.get(at).copied().unwrap_or("<end>")
            ),
        });
    }
    Ok(())
}

/// Lowercase hex of a SHA-256 digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
