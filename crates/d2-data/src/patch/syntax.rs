//! Layer and stack files: bytes, lines, tokens, statements. One P error
//! per line; a failed line 1 leaves only P01 on the other lines.

use std::collections::BTreeSet;

use super::{Code, Finding, VERSION};
use crate::schema::ExtType;

/// Largest layer or stack file (16 MiB).
pub const MAX_FILE: usize = 16 << 20;
/// Longest statement line.
pub const MAX_LINE: usize = 4096;
/// Longest token.
pub const MAX_TOKEN: usize = 1024;
/// Most layers in a stack.
pub const MAX_LAYERS: usize = 1024;
/// Longest layer path.
pub const MAX_PATH: usize = 255;

/// A token: its bytes (without brackets), 1-based column, bare or
/// bracketed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tok {
    /// Token bytes.
    pub text: Vec<u8>,
    /// 1-based column of the token.
    pub col: usize,
    /// True when written without brackets.
    pub bare: bool,
}

impl Tok {
    fn is(&self, word: &str) -> bool {
        self.bare && self.text == word.as_bytes()
    }
}

/// A row selector: `#n L` or a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sel {
    /// A row key.
    Key(Tok),
    /// `#n L`: row number `n` with its key label.
    Index {
        /// The number parsed from `#n`.
        n: u32,
        /// The `#n` token.
        index: Tok,
        /// The key label token.
        label: Tok,
    },
}

impl Sel {
    /// The selector's first token (error location).
    pub fn first(&self) -> &Tok {
        match self {
            Sel::Key(t) => t,
            Sel::Index { index, .. } => index,
        }
    }
}

/// `like <sel> [<pin>]` of an `add`.
pub type Like = Option<(Sel, Option<Tok>)>;

/// One layer statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// `table <name>`: selects the table later statements apply to.
    Table {
        /// The keyword token.
        kw: Tok,
        /// The table name token.
        name: Tok,
    },
    /// `set`: changes one cell, expecting an old value.
    Set {
        /// The keyword token.
        kw: Tok,
        /// The row selector.
        sel: Sel,
        /// The column reference.
        column: Tok,
        /// The expected current value.
        old: Tok,
        /// The new value.
        new: Tok,
    },
    /// `check`: asserts a cell value.
    Check {
        /// The keyword token.
        kw: Tok,
        /// The row selector.
        sel: Sel,
        /// The column reference.
        column: Tok,
        /// The expected value.
        value: Tok,
    },
    /// `add`: inserts a row.
    Add {
        /// The keyword token.
        kw: Tok,
        /// The number parsed from the `#n` index token.
        n: u32,
        /// The `#n` index token.
        index: Tok,
        /// The new row's key.
        key: Tok,
        /// The optional `like` template.
        like: Like,
    },
    /// `addcol <name> <type> [<default>]`: appends an extension column to
    /// the current table. Every row gets the default (empty when none);
    /// no compiled field binds it, so compiled records keep their size and
    /// bytes, and mods read it through [`crate::schema::ExtStore`].
    AddCol {
        /// The keyword token.
        kw: Tok,
        /// The new column's header name.
        name: Tok,
        /// The column's value type (`int` or `text`).
        ty: ExtType,
        /// The type token.
        ty_tok: Tok,
        /// The optional default value.
        default: Option<Tok>,
    },
    /// `stradd <key> <text>` / `strset <key> <text>`: adds a string or
    /// changes an existing one, by key. Not part of any table zone.
    Str {
        /// The keyword token.
        kw: Tok,
        /// `stradd`: the key is new; `strset`: the key exists.
        add: bool,
        /// The string key.
        key: Tok,
        /// The text (UTF-8).
        text: Tok,
    },
}

impl Stmt {
    /// The statement's keyword token.
    pub fn kw(&self) -> &Tok {
        match self {
            Stmt::Table { kw, .. }
            | Stmt::Set { kw, .. }
            | Stmt::Check { kw, .. }
            | Stmt::Add { kw, .. }
            | Stmt::AddCol { kw, .. }
            | Stmt::Str { kw, .. } => kw,
        }
    }
}

/// A statement with its line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StmtLine {
    /// 1-based line in the layer file.
    pub line: usize,
    /// The parsed statement.
    pub stmt: Stmt,
}

/// A parsed layer, named by its stack path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// The layer's stack path.
    pub path: String,
    /// Statements in file order.
    pub statements: Vec<StmtLine>,
}

type PErr = (Code, usize);

/// Splits at LF (CR LF = LF; an unterminated last line counts). Lines keep
/// any other CR, which P01 then reports.
fn lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
    if data.ends_with(b"\n") || data.is_empty() {
        out.pop();
    }
    let last = out.len();
    for (i, l) in out.iter_mut().enumerate() {
        // A CR before LF; the unterminated last line has no LF.
        let terminated = i + 1 < last || data.ends_with(b"\n");
        if terminated {
            if let Some(s) = l.strip_suffix(b"\r") {
                *l = s;
            }
        }
    }
    out
}

/// P01: the leftmost byte outside 0x20–0x7E.
fn bad_byte(line: &[u8]) -> Option<usize> {
    line.iter()
        .position(|&b| !(0x20..=0x7E).contains(&b))
        .map(|i| i + 1)
}

fn is_blank_or_comment(line: &[u8]) -> bool {
    match line.iter().find(|&&b| b != b' ') {
        None => true,
        Some(&b) => b == b'#',
    }
}

/// Splits a line into tokens.
fn tokenize(line: &[u8]) -> Result<Vec<Tok>, PErr> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            b' ' => {
                i += 1;
                continue;
            }
            b']' => return Err((Code::P10, i + 1)),
            _ => {}
        }
        let (text, end, bare) = if line[i] == b'[' {
            match line[i + 1..].iter().position(|&b| b == b']' || b == b'[') {
                Some(j) if line[i + 1 + j] == b']' => {
                    (line[i + 1..i + 1 + j].to_vec(), i + 2 + j, false)
                }
                _ => return Err((Code::P06, i + 1)),
            }
        } else {
            let j = line[i..]
                .iter()
                .position(|&b| b == b' ' || b == b'[' || b == b']')
                .map_or(line.len(), |j| i + j);
            (line[i..j].to_vec(), j, true)
        };
        if text.len() > MAX_TOKEN {
            return Err((Code::P12, i + 1));
        }
        if end < line.len() && line[end] != b' ' {
            return Err((Code::P10, end + 1));
        }
        toks.push(Tok {
            text,
            col: i + 1,
            bare,
        });
        i = end;
    }
    Ok(toks)
}

/// `#0 | #[1-9][0-9]*`, at most u32::MAX.
fn parse_index(t: &Tok) -> Result<u32, PErr> {
    let digits = &t.text[1..];
    let ok = t.bare
        && t.text.first() == Some(&b'#')
        && !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && (digits == b"0" || digits[0] != b'0');
    ok.then(|| std::str::from_utf8(digits).ok()?.parse::<u32>().ok())
        .flatten()
        .ok_or((Code::P07, t.col))
}

struct Cursor<'a> {
    toks: &'a [Tok],
    i: usize,
    /// Column after the last non-space byte.
    eol: usize,
}

impl Cursor<'_> {
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn need(&mut self) -> Result<Tok, PErr> {
        self.next().ok_or((Code::P05, self.eol))
    }

    fn end(&mut self) -> Result<(), PErr> {
        match self.toks.get(self.i) {
            Some(t) => Err((Code::P05, t.col)),
            None => Ok(()),
        }
    }

    fn sel(&mut self) -> Result<Sel, PErr> {
        let t = self.need()?;
        if t.bare && t.text.first() == Some(&b'#') {
            let n = parse_index(&t)?;
            let label = self.need()?;
            Ok(Sel::Index { n, index: t, label })
        } else {
            Ok(Sel::Key(t))
        }
    }

    fn word(&mut self, word: &str) -> Result<(), PErr> {
        let t = self.need()?;
        if t.is(word) {
            Ok(())
        } else {
            Err((Code::P05, t.col))
        }
    }
}

fn is_pin(t: &Tok) -> bool {
    t.bare
        && t.text.len() == 20
        && t.text.starts_with(b"sha:")
        && t.text[4..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

fn statement(toks: &[Tok], eol: usize) -> Result<Stmt, PErr> {
    let mut c = Cursor { toks, i: 1, eol };
    let kw = toks[0].clone();
    let stmt = match kw.text.as_slice() {
        b"table" => {
            let name = c.need()?;
            let ok = name.bare
                && (1..=32).contains(&name.text.len())
                && name
                    .text
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
            if !ok {
                return Err((Code::P08, name.col));
            }
            Stmt::Table { kw, name }
        }
        b"set" => {
            let sel = c.sel()?;
            let column = c.need()?;
            let old = c.need()?;
            c.word("->")?;
            let new = c.need()?;
            Stmt::Set {
                kw,
                sel,
                column,
                old,
                new,
            }
        }
        b"check" => {
            let sel = c.sel()?;
            let column = c.need()?;
            let value = c.need()?;
            Stmt::Check {
                kw,
                sel,
                column,
                value,
            }
        }
        b"add" => {
            let index = c.need()?;
            if !(index.bare && index.text.first() == Some(&b'#')) {
                return Err((Code::P05, index.col));
            }
            let n = parse_index(&index)?;
            let key = c.need()?;
            let like = match c.next() {
                None => None,
                Some(t) if t.is("like") => {
                    let sel = c.sel()?;
                    let pin = match c.next() {
                        None => None,
                        Some(p) if is_pin(&p) => Some(p),
                        Some(p) => return Err((Code::P11, p.col)),
                    };
                    Some((sel, pin))
                }
                Some(t) => return Err((Code::P05, t.col)),
            };
            Stmt::Add {
                kw,
                n,
                index,
                key,
                like,
            }
        }
        b"addcol" => {
            let name = c.need()?;
            // A header name: non-empty and without `@` (it would read as
            // a `name@k` column reference).
            if name.text.is_empty() || name.text.contains(&b'@') {
                return Err((Code::P05, name.col));
            }
            let ty_tok = c.need()?;
            let ty = match ty_tok.text.as_slice() {
                b"int" if ty_tok.bare => ExtType::Int,
                b"text" if ty_tok.bare => ExtType::Text,
                _ => return Err((Code::P05, ty_tok.col)),
            };
            let default = c.next();
            Stmt::AddCol {
                kw,
                name,
                ty,
                ty_tok,
                default,
            }
        }
        b"stradd" | b"strset" => {
            let key = c.need()?;
            if key.text.is_empty() {
                return Err((Code::P05, key.col));
            }
            let text = c.need()?;
            Stmt::Str {
                add: kw.is("stradd"),
                kw,
                key,
                text,
            }
        }
        _ => unreachable!("keyword checked by the caller"),
    };
    c.end()?;
    Ok(stmt)
}

fn eol_col(line: &[u8]) -> usize {
    line.iter().rposition(|&b| b != b' ').map_or(1, |i| i + 2)
}

/// The shared line checks: P01, P12 (line), tokens. `None` for blank and
/// comment lines.
fn line_tokens(line: &[u8]) -> Result<Option<Vec<Tok>>, PErr> {
    if let Some(col) = bad_byte(line) {
        return Err((Code::P01, col));
    }
    if is_blank_or_comment(line) {
        return Ok(None);
    }
    if line.len() > MAX_LINE {
        return Err((Code::P12, MAX_LINE + 1));
    }
    tokenize(line).map(Some)
}

/// Checks line 1 = `<magic> <version>`: `Ok` or (bad shape, bad version).
fn header(first: Option<&[u8]>, magic: &str) -> Result<(), (bool, usize)> {
    let toks = match first.map(line_tokens) {
        Some(Ok(Some(t))) => t,
        _ => return Err((false, 1)),
    };
    if toks.len() != 2 || !toks[0].is(magic) {
        return Err((false, 1));
    }
    if toks[1].is(&VERSION.to_string()) {
        Ok(())
    } else {
        Err((true, toks[1].col))
    }
}

/// Parses a layer (`path` is its stack path). Findings are P only; the
/// layer holds the statements that parsed.
pub fn parse_layer(path: &str, data: &[u8], pos: usize) -> (Layer, Vec<Finding>) {
    let mut layer = Layer {
        path: path.to_owned(),
        statements: Vec::new(),
    };
    let mut out = Vec::new();
    let mut push = |code, line, col| out.push(Finding::new(code, path, line, col).at_pos(pos));
    if data.len() > MAX_FILE {
        push(Code::P12, 0, 0);
        return (layer, out);
    }
    let lines = lines(data);
    let head_ok = match header(lines.first().copied(), "d2patch") {
        Ok(()) => true,
        Err((version, col)) => {
            match lines.first().and_then(|l| bad_byte(l)) {
                Some(c) => push(Code::P01, 1, c),
                None if version => push(Code::P03, 1, col),
                None => push(Code::P02, 1, 1),
            }
            false
        }
    };
    let mut seen_table = false;
    for (k, line) in lines.iter().enumerate().skip(1) {
        let n = k + 1;
        if !head_ok {
            if let Some(c) = bad_byte(line) {
                push(Code::P01, n, c);
            }
            continue;
        }
        let toks = match line_tokens(line) {
            Ok(None) => continue,
            Ok(Some(t)) => t,
            Err((code, col)) => {
                push(code, n, col);
                continue;
            }
        };
        let kw = &toks[0];
        if !["table", "set", "check", "add", "addcol", "stradd", "strset"]
            .iter()
            .any(|w| kw.is(w))
        {
            push(Code::P04, n, kw.col);
            continue;
        }
        // String statements name their key and stand outside table zones.
        if kw.is("table") {
            seen_table = true;
        } else if !seen_table && !kw.is("stradd") && !kw.is("strset") {
            push(Code::P09, n, kw.col);
            continue;
        }
        match statement(&toks, eol_col(line)) {
            Ok(stmt) => layer.statements.push(StmtLine { line: n, stmt }),
            Err((code, col)) => push(code, n, col),
        }
    }
    (layer, out)
}

/// A stack entry: layer path and its location in the stack file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEntry {
    /// The layer path.
    pub path: String,
    /// 1-based line in the stack file.
    pub line: usize,
    /// 1-based column in the stack file.
    pub col: usize,
}

fn valid_path(p: &[u8]) -> bool {
    p.len() <= MAX_PATH
        && p.ends_with(b".d2patch")
        && p.split(|&b| b == b'/').all(|seg| {
            !seg.is_empty()
                && seg != b"."
                && seg != b".."
                && seg.iter().all(|&b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
                })
        })
}

/// Parses a stack file: its layer paths in order, and S/P findings.
pub fn parse_stack(file: &str, data: &[u8]) -> (Vec<StackEntry>, Vec<Finding>) {
    let mut entries: Vec<StackEntry> = Vec::new();
    let mut out = Vec::new();
    let mut push = |code, line, col| out.push(Finding::new(code, file, line, col));
    if data.len() > MAX_FILE {
        push(Code::P12, 0, 0);
        return (entries, out);
    }
    let lines = lines(data);
    let head_ok = match header(lines.first().copied(), "d2stack") {
        Ok(()) => true,
        Err((version, col)) => {
            match lines.first().and_then(|l| bad_byte(l)) {
                Some(c) => push(Code::P01, 1, c),
                None if version => push(Code::S02, 1, col),
                None => push(Code::S01, 1, 1),
            }
            false
        }
    };
    let mut seen = BTreeSet::new();
    for (k, line) in lines.iter().enumerate().skip(1) {
        let n = k + 1;
        if !head_ok {
            if let Some(c) = bad_byte(line) {
                push(Code::P01, n, c);
            }
            continue;
        }
        let toks = match line_tokens(line) {
            Ok(None) => continue,
            Ok(Some(t)) => t,
            Err((code, col)) => {
                push(code, n, col);
                continue;
            }
        };
        if toks.len() != 2 || !toks[0].is("layer") {
            push(Code::S03, n, toks[0].col);
            continue;
        }
        let p = &toks[1];
        if !p.bare || !valid_path(&p.text) {
            push(Code::S04, n, p.col);
            continue;
        }
        if !seen.insert(p.text.clone()) {
            push(Code::S05, n, p.col);
            continue;
        }
        if entries.len() == MAX_LAYERS {
            push(Code::S07, n, p.col);
            continue;
        }
        entries.push(StackEntry {
            path: String::from_utf8(p.text.clone()).expect("ASCII path"),
            line: n,
            col: p.col,
        });
    }
    (entries, out)
}

/// Parses a stack and its layers. `read` returns a layer's bytes by stack
/// path (`None`: unreadable, S06).
pub fn load_stack(
    file: &str,
    data: &[u8],
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> (Vec<Layer>, Vec<Finding>) {
    let (entries, mut findings) = parse_stack(file, data);
    let mut layers = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        match read(&e.path) {
            None => findings.push(Finding::new(Code::S06, file, e.line, e.col)),
            Some(bytes) => {
                let (layer, f) = parse_layer(&e.path, &bytes, k + 1);
                findings.extend(f);
                layers.push(layer);
            }
        }
    }
    (layers, findings)
}
