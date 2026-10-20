//! The txt → record compiler. It turns a parsed `.txt` sheet into fixed-size binary
//! records using a table's field list (`FieldDef`s from the schema).
//!
//! The pieces, in the order they appear here: cell conversions by field type (integers,
//! codes, name keys), the linkers that map codes and names to record indices, the
//! diagnostics the compiler reports for accepted-but-odd input, the [`Callbacks`] hook
//! and its standard implementation [`StdCallbacks`] (strings, calc formulas, params and
//! the table-specific callbacks in `callbacks`), the structural field-list checks, and
//! [`compile_table`], which runs two passes: own keys first, then every other cell.
//!
//! The rest of `d2-data` (the loader and the table fixups) calls [`compile_table`] once
//! per table and keeps the shared [`Linkers`] across tables.

use std::collections::{BTreeMap, HashMap};

use crate::bin::DataRules;
use crate::calc::{self, CalcDiag, CalcError, CalcLinks, CalcRegistry, Family, NO_KEYWORDS};
use crate::schema::{CalcBuffer, FieldDef, FieldType, Link, LinkerKind};
use crate::strings::StringTables;
use crate::txt::{bind, names_equal, ErrorCode, TxtError, TxtTable, MAX_COLUMNS};

// ---------------------------------------------------------------- values

/// The integer rule for cells: every byte is a signed digit (so a non-digit byte still
/// contributes), wrapping at 32 bits; a leading `-` negates the result.
pub fn parse_int(cell: &[u8]) -> u32 {
    let (neg, digits) = match cell.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, cell),
    };
    let mut v: u32 = 0;
    for &b in digits {
        let s = i32::from(b as i8) as u32;
        v = v.wrapping_mul(10).wrapping_add(s).wrapping_sub(48);
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// Code key: the first 4 bytes of the cell, padded with spaces.
pub fn code4(cell: &[u8]) -> [u8; 4] {
    let mut c = [b' '; 4];
    let n = cell.len().min(4);
    c[..n].copy_from_slice(&cell[..n]);
    c
}

/// Name key: the first 31 bytes of the cell with `A`–`Z` lowercased. `None` when one of
/// those bytes is ≥ 0x80 (error code E11).
pub fn name_key(cell: &[u8]) -> Option<Vec<u8>> {
    let k = &cell[..cell.len().min(31)];
    if k.iter().any(|&b| b >= 0x80) {
        return None;
    }
    Some(k.to_ascii_lowercase())
}

/// The exact value of a `-?[0-9]+` cell, saturated far outside every
/// range; `None` for other non-empty cells and the empty cell.
fn exact_int(cell: &[u8]) -> Option<i128> {
    let (neg, digits) = match cell.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, cell),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut v: i128 = 0;
    for &d in digits {
        v = (v * 10 + i128::from(d - b'0')).min(1 << 80);
    }
    Some(if neg { -v } else { v })
}

// --------------------------------------------------------------- linkers

/// A linker keyed by 4-byte codes (see [`code4`]) that hands out indices in add order.
#[derive(Debug, Clone, Default)]
pub struct CodeLinker {
    keys: HashMap<u32, u32>,
    n: u32,
}

impl CodeLinker {
    /// add(code): the index, and whether the key was bumped.
    pub fn add(&mut self, code: u32) -> (u32, bool) {
        let mut k = code;
        while self.keys.contains_key(&k) {
            k = k.wrapping_add(1);
        }
        let index = self.n;
        self.keys.insert(k, index);
        self.n += 1;
        (index, k != code)
    }

    /// The index registered for `code`, if any.
    pub fn find(&self, code: u32) -> Option<u32> {
        self.keys.get(&code).copied()
    }

    /// How many adds have happened (the next index).
    pub fn len(&self) -> u32 {
        self.n
    }

    /// True when nothing has been added.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// A linker keyed by normalized names (see [`name_key`]).
#[derive(Debug, Clone, Default)]
pub struct NameLinker {
    keys: HashMap<Vec<u8>, u32>,
    n: u32,
}

impl NameLinker {
    /// find-or-add (types 17, 18).
    pub fn find_or_add(&mut self, key: &[u8]) -> u32 {
        if let Some(&i) = self.keys.get(key) {
            return i;
        }
        let i = self.n;
        self.keys.insert(key.to_vec(), i);
        self.n += 1;
        i
    }

    /// add-always (type 16): the index, and whether the key was new.
    pub fn add_always(&mut self, key: &[u8]) -> (u32, bool) {
        let i = self.n;
        let new = !self.keys.contains_key(key);
        if new {
            self.keys.insert(key.to_vec(), i);
        }
        self.n += 1;
        (i, new)
    }

    /// The index registered for `key`, if any.
    pub fn find(&self, key: &[u8]) -> Option<u32> {
        self.keys.get(key).copied()
    }

    /// How many adds have happened (the next index).
    pub fn len(&self) -> u32 {
        self.n
    }

    /// True when nothing has been added.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// A linker of either kind.
#[derive(Debug, Clone)]
pub enum Linker {
    /// Keyed by 4-byte codes.
    Code(CodeLinker),
    /// Keyed by normalized names.
    Name(NameLinker),
}

impl Linker {
    /// Which kind of key this linker uses.
    pub fn kind(&self) -> LinkerKind {
        match self {
            Linker::Code(_) => LinkerKind::Code,
            Linker::Name(_) => LinkerKind::Name,
        }
    }
}

/// All linkers, by name (`fields.tsv` link: `<table>.<column>`,
/// `items.code`, `@range`, `@treasureclass`). They live across tables.
#[derive(Debug, Clone, Default)]
pub struct Linkers {
    map: HashMap<String, Linker>,
}

impl Linkers {
    /// The linker called `name`, of either kind.
    pub fn get(&self, name: &str) -> Option<&Linker> {
        self.map.get(name)
    }

    /// The linker called `name` when it is a code linker.
    pub fn code(&self, name: &str) -> Option<&CodeLinker> {
        match self.map.get(name) {
            Some(Linker::Code(l)) => Some(l),
            _ => None,
        }
    }

    /// The linker called `name` when it is a name linker.
    pub fn name(&self, name: &str) -> Option<&NameLinker> {
        match self.map.get(name) {
            Some(Linker::Name(l)) => Some(l),
            _ => None,
        }
    }

    /// Creates the linker if absent; `Err` when it exists with the other
    /// kind.
    pub fn ensure(&mut self, name: &str, kind: LinkerKind) -> Result<&mut Linker, LinkerKind> {
        let l = self
            .map
            .entry(name.to_owned())
            .or_insert_with(|| match kind {
                LinkerKind::Code => Linker::Code(CodeLinker::default()),
                LinkerKind::Name => Linker::Name(NameLinker::default()),
            });
        if l.kind() == kind {
            Ok(l)
        } else {
            Err(l.kind())
        }
    }

    /// Inserts a hand-built linker, replacing any linker of that name.
    pub fn insert(&mut self, name: &str, linker: Linker) {
        self.map.insert(name.to_owned(), linker);
    }
}

/// Hand-built linker names (`fields.tsv`).
/// Name of the hand-built range linker, see [`range_linker`].
pub const RANGE_LINKER: &str = "@range";
/// Name of the hand-built treasure class linker, see [`tc_linker`].
pub const TC_LINKER: &str = "@treasureclass";

/// `@range`: `none`, `h2h`, `rng`, `both`, `loc` → 0–4.
pub fn range_linker() -> Linker {
    let mut l = CodeLinker::default();
    for code in [b"none".as_slice(), b"h2h", b"rng", b"both", b"loc"] {
        l.add(u32::from_le_bytes(code4(code)));
    }
    Linker::Code(l)
}

/// `@treasureclass`, add-always: the empty
/// name; 32 automatic TCs (`<code without pad spaces><level>`, levels 3,
/// 6, …, 96) per itemtypes record with byte 0x1D ≠ 0; then the
/// `treasureclassex` names (string at offset 0) up to the first empty one.
/// `None` when a name has a byte ≥ 0x80.
pub fn tc_linker<'a>(
    itemtypes: impl IntoIterator<Item = &'a [u8]>,
    treasureclassex: impl IntoIterator<Item = &'a [u8]>,
) -> Option<NameLinker> {
    let mut l = NameLinker::default();
    l.add_always(b"");
    for rec in itemtypes {
        if rec[0x1D] == 0 {
            continue;
        }
        let mut code = &rec[..4];
        while let [rest @ .., b' '] = code {
            code = rest;
        }
        for level in (3..=96).step_by(3) {
            let mut name = code.to_vec();
            name.extend_from_slice(level.to_string().as_bytes());
            l.add_always(&name_key(&name)?);
        }
    }
    for rec in treasureclassex {
        let name = &rec[..rec.len().min(33)];
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        if end == 0 {
            break;
        }
        l.add_always(&name_key(&name[..end])?);
    }
    Some(l)
}

// ----------------------------------------------------------- diagnostics

/// Conditions the compiler accepts but reports, so callers can count them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagKind {
    /// An integer cell that is not plain digits.
    IntSyntax,
    /// An integer cell whose value does not fit the field's width.
    IntRange,
    /// A text cell longer than the field, cut to fit.
    TextCut,
    /// A linked cell whose name or code is not in its linker.
    LinkMiss,
    /// A column header that appears more than once.
    DupColumn,
    /// A code that was already in a code linker (the key is bumped).
    DupCode,
    /// A name that was already in a name linker.
    DupName,
    /// Non-empty callback text whose lookup misses.
    CbMiss,
    /// A non-empty part of a callback's text is ignored.
    CbStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One reported condition and where it happened.
pub struct Diagnostic {
    /// What happened.
    pub kind: DiagKind,
    /// The txt line (the header is line 1).
    pub line: usize,
    /// The txt column, when the condition belongs to one.
    pub column: Option<usize>,
    /// The field name, when the condition belongs to one.
    pub field: Option<String>,
}

// ------------------------------------------------------------- callbacks

/// One field-callback call (field types 23–25).
pub struct FieldCall<'a> {
    /// The field being filled.
    pub field: &'a FieldDef,
    /// First 256 cell bytes; `None` for a missing field.
    pub text: Option<&'a [u8]>,
    /// The whole record.
    pub record: &'a mut [u8],
    /// Index of the record being built.
    pub record_index: u32,
    /// The slot the callback writes (the field's offset).
    pub slot: u32,
    /// Linkers as of pass 2 (read-only).
    pub linkers: &'a Linkers,
    /// Diagnostics of this call (CbMiss, CbStop).
    pub diagnostics: &'a mut Vec<DiagKind>,
}

/// A callback refused its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackError {
    /// The error code reported for the table.
    pub code: ErrorCode,
    /// Human-readable detail.
    pub detail: String,
}

/// The callbacks a field list uses: one for key fields, one for field callbacks.
pub trait Callbacks {
    /// Key callback (type 22): `text` = first 256 cell bytes.
    fn key(&mut self, field: &FieldDef, text: &[u8]) -> u16;
    /// Field callback (types 23, 24, 25).
    fn field(&mut self, call: FieldCall<'_>) -> Result<(), CallbackError>;
}

/// Calc outcomes counted by [`StdCallbacks`].
pub type CalcDiagCounts = BTreeMap<(CalcBuffer, CalcDiag), usize>;

/// The standard callbacks: `strkey`, `calc(<buffer>)`, `param` and the table-specific
/// callbacks in the `callbacks` submodule. A `cb(...)` name outside that set writes
/// nothing and is counted in `unspecified`.
pub struct StdCallbacks<'s> {
    /// String tables used by `strkey` lookups.
    pub strings: &'s StringTables,
    /// The four code buffers, filled in compile order.
    pub buffers: BTreeMap<CalcBuffer, Vec<u8>>,
    /// Unique and set items behind `@uniques` / `@sets`.
    pub special: SpecialItems,
    /// Calls of unknown table-specific callbacks, by callback name.
    pub unspecified: BTreeMap<String, usize>,
    /// Counts of calc outcomes, by buffer and diagnostic.
    pub calc_diagnostics: CalcDiagCounts,
    /// A mod game's formula keywords; empty for D2RUST's own tables.
    pub calc_registry: &'s CalcRegistry,
}

impl<'s> StdCallbacks<'s> {
    /// Callbacks with empty buffers and counters, reading names from `strings`.
    pub fn new(strings: &'s StringTables) -> Self {
        Self::with_calc(strings, &NO_KEYWORDS)
    }

    /// [`StdCallbacks::new`] compiling formulas with a mod game's keywords.
    pub fn with_calc(strings: &'s StringTables, calc_registry: &'s CalcRegistry) -> Self {
        StdCallbacks {
            strings,
            buffers: CalcBuffer::ALL.iter().map(|&b| (b, Vec::new())).collect(),
            special: SpecialItems::default(),
            unspecified: BTreeMap::new(),
            calc_diagnostics: BTreeMap::new(),
            calc_registry,
        }
    }
}

fn put_u32(record: &mut [u8], offset: u32, v: u32) -> Result<(), CallbackError> {
    let o = offset as usize;
    record
        .get_mut(o..o + 4)
        .ok_or_else(|| CallbackError {
            code: ErrorCode::E13,
            detail: format!("4 bytes at {o} outside the record"),
        })?
        .copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// C `atol` on text starting with `-` or a digit: optional `-`, decimal
/// digits up to the first non-digit, saturating like the CRT.
fn atol(text: &[u8]) -> i32 {
    let (neg, digits) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, text),
    };
    let mut v: i64 = 0;
    for &d in digits.iter().take_while(|d| d.is_ascii_digit()) {
        v = (v * 10 + i64::from(d - b'0')).min(1 << 40);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Name links for formulas: resolves names in calc text through the compiler's linkers.
struct FormulaLinks<'a> {
    linkers: &'a Linkers,
    skills: &'static str,
}

impl CalcLinks for FormulaLinks<'_> {
    fn skill(&self, key: &[u8]) -> Option<u32> {
        self.linkers.name(self.skills)?.find(key)
    }
    fn missile(&self, key: &[u8]) -> Option<u32> {
        self.linkers.name("missiles.Missile")?.find(key)
    }
    fn stat(&self, key: &[u8]) -> Option<u32> {
        self.linkers.name("itemstatcost.stat")?.find(key)
    }
    fn skillcalc(&self, code: u32) -> Option<u32> {
        self.linkers.code("skillcalc.code")?.find(code)
    }
    fn misscalc(&self, code: u32) -> Option<u32> {
        self.linkers.code("misscalc.code")?.find(code)
    }
}

impl Callbacks for StdCallbacks<'_> {
    fn key(&mut self, _field: &FieldDef, text: &[u8]) -> u16 {
        self.strings.strkey(text)
    }

    fn field(&mut self, call: FieldCall<'_>) -> Result<(), CallbackError> {
        let text = call.text.filter(|t| !t.is_empty());
        match &call.field.link {
            Link::Calc(buffer) => {
                let Some(text) = text else {
                    return put_u32(call.record, call.field.offset, u32::MAX);
                };
                let family = Family::of(*buffer);
                // Missiles resolve `skill(` names through the compile-only
                // skills link.
                let skills = match family {
                    Family::Missiles => "skills_lookup.skill",
                    _ => "skills.skill",
                };
                let links = FormulaLinks {
                    linkers: call.linkers,
                    skills,
                };
                let compiled = calc::compile_with(family, &links, self.calc_registry, text)
                    .map_err(|e| CallbackError {
                        code: ErrorCode::E11,
                        detail: match e {
                            CalcError::NonAscii => "formula byte >= 0x80".into(),
                            CalcError::MissileRand => "missile formula calls rand".into(),
                        },
                    })?;
                for d in compiled.diagnostics {
                    *self.calc_diagnostics.entry((*buffer, d)).or_default() += 1;
                }
                if compiled.code.is_empty() {
                    return put_u32(call.record, call.field.offset, u32::MAX);
                }
                let buf = self.buffers.entry(*buffer).or_default();
                let start = buf.len() as u32;
                buf.extend_from_slice(&compiled.code);
                put_u32(call.record, call.field.offset, start)
            }
            Link::Param => {
                let value = match text {
                    None => 0,
                    Some(t) if t[0] == b'-' || t[0].is_ascii_digit() => atol(t) as u32,
                    Some(t) => {
                        let key = name_key(t).ok_or_else(|| CallbackError {
                            code: ErrorCode::E11,
                            detail: "param name byte >= 0x80".into(),
                        })?;
                        ["skills.skill", "montype.type", "states.state"]
                            .iter()
                            .filter_map(|l| call.linkers.name(l))
                            .find_map(|l| l.find(&key))
                            .unwrap_or(0)
                    }
                };
                put_u32(call.record, call.field.offset, value)
            }
            Link::Table(name) => {
                if !callbacks::run(name, call, &self.special)? {
                    *self.unspecified.entry(name.clone()).or_default() += 1;
                }
                Ok(())
            }
            Link::None | Link::Linker(_) => Ok(()),
        }
    }
}

/// The table-specific field callbacks and the `@uniques` / `@sets` linkers.
pub mod callbacks;
pub use callbacks::{special_linker, SpecialItem, SpecialItems, SETS_LINKER, UNIQUES_LINKER};

// --------------------------------------------------------- field checks

fn e13(file: &str, field: &FieldDef, detail: impl Into<String>) -> TxtError {
    TxtError::new(file, ErrorCode::E13)
        .with_field(&field.column)
        .with_detail(detail)
}

/// The structural checks on a field list (linker existence is checked against the live
/// linkers at compile time).
pub fn check_field_list(
    file: &str,
    fields: &[FieldDef],
    record_size: usize,
) -> Result<(), TxtError> {
    if fields.len() > MAX_COLUMNS {
        return Err(TxtError::new(file, ErrorCode::E13)
            .with_detail(format!("{} fields (max 280)", fields.len())));
    }
    if record_size == 0 {
        return Err(TxtError::new(file, ErrorCode::E13).with_detail("record size 0"));
    }
    for (i, f) in fields.iter().enumerate() {
        if f.column.is_empty() {
            return Err(e13(file, f, "empty field name"));
        }
        if fields[..i]
            .iter()
            .any(|g| names_equal(&g.column, &f.column))
        {
            return Err(e13(file, f, "duplicate field name"));
        }
        let t = f.field_type;
        if t.linker_kind().is_some() && f.link.linker().is_none() {
            return Err(e13(file, f, "link type without a linker"));
        }
        if t.is_field_callback() && !matches!(f.link, Link::Calc(_) | Link::Param | Link::Table(_))
        {
            return Err(e13(file, f, "callback type without a callback"));
        }
        let fits = match t {
            FieldType::Bit => (f.offset as usize + (f.length as usize >> 3)) < record_size,
            _ => match t.width(f.length) {
                Some(w) => f.offset as usize + w as usize <= record_size,
                None => match f.link {
                    Link::Calc(_) | Link::Param => f.offset as usize + 4 <= record_size,
                    _ => true,
                },
            },
        };
        if !fits {
            return Err(e13(
                file,
                f,
                format!("does not fit a {record_size}-byte record"),
            ));
        }
    }
    // Type-5 runs.
    let mut i = 0;
    while i < fields.len() {
        if fields[i].field_type == FieldType::Unknown1 && fields[i].length > 1 {
            let mut k = 1;
            loop {
                i += 1;
                match fields.get(i) {
                    Some(g) if g.field_type == FieldType::Unknown1 => {
                        k += 1;
                        if k >= g.length {
                            break;
                        }
                    }
                    _ => {
                        let at = fields.get(i).unwrap_or(&fields[i - 1]);
                        return Err(e13(file, at, "broken u8? run"));
                    }
                }
            }
        }
        i += 1;
    }
    Ok(())
}

// -------------------------------------------------------------- compile

/// A compiled table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// Number of records.
    pub count: usize,
    /// Bytes per record.
    pub record_size: usize,
    /// `count × record_size` bytes.
    pub records: Vec<u8>,
    /// Conditions reported while compiling.
    pub diagnostics: Vec<Diagnostic>,
}

impl Compiled {
    /// Bytes of record `i`.
    pub fn record(&self, i: usize) -> &[u8] {
        &self.records[i * self.record_size..(i + 1) * self.record_size]
    }
}

struct Ctx<'a> {
    file: &'a str,
    diagnostics: Vec<Diagnostic>,
    rules: DataRules,
}

impl Ctx<'_> {
    fn diag(&mut self, kind: DiagKind, line: usize, column: usize, field: &FieldDef) {
        self.diagnostics.push(Diagnostic {
            kind,
            line,
            column: Some(column),
            field: Some(field.name()),
        });
    }

    fn err(&self, code: ErrorCode, line: usize, column: usize, field: &FieldDef) -> TxtError {
        TxtError::new(self.file, code)
            .at_line(line)
            .at_column(column)
            .with_field(&field.column)
    }
}

fn put(record: &mut [u8], offset: u32, bytes: &[u8]) {
    let o = offset as usize;
    record[o..o + bytes.len()].copy_from_slice(bytes);
}

/// Text length limit by field type (what [`DiagKind::TextCut`] cuts to).
fn text_limit(f: &FieldDef) -> Option<usize> {
    match f.field_type.id() {
        1 | 7 => Some(f.length as usize),
        16 => Some((f.length.max(1) as usize - 1).min(31)),
        9..=15 => Some(4),
        17..=21 => Some(31),
        22..=25 => Some(256),
        _ => None,
    }
}

/// Integer range of a type for IntRange.
fn int_range(t: FieldType) -> (i128, i128) {
    match t.id() {
        4..=6 => (-128, 255),
        3 => (-32_768, 65_535),
        26 => (0, 1),
        _ => (-(1 << 31), (1 << 32) - 1),
    }
}

/// Compiles `txt` with `fields` into `count × record_size` bytes
/// in two passes. Registers own keys into `linkers` and runs
/// callbacks through `callbacks`.
pub fn compile_table(
    file: &str,
    txt: &TxtTable,
    fields: &[FieldDef],
    record_size: usize,
    linkers: &mut Linkers,
    callbacks: &mut dyn Callbacks,
) -> Result<Compiled, TxtError> {
    compile_table_with(
        file,
        txt,
        fields,
        record_size,
        linkers,
        callbacks,
        DataRules::Original,
    )
}

/// [`compile_table`] under the data `rules` of the game
/// (`docs/plans/mods/seams.md` MS10d). Under [`DataRules::Mod`] an
/// integer cell out of its field's range is clamped into it instead of
/// wrapping; everything else compiles the same, with the same
/// diagnostics.
pub fn compile_table_with(
    file: &str,
    txt: &TxtTable,
    fields: &[FieldDef],
    record_size: usize,
    linkers: &mut Linkers,
    callbacks: &mut dyn Callbacks,
    rules: DataRules,
) -> Result<Compiled, TxtError> {
    check_field_list(file, fields, record_size)?;
    // Own-key linkers are created as the table loads; lookups need an
    // existing linker of the right kind.
    let linked = || {
        fields.iter().filter_map(|f| {
            let kind = f.field_type.linker_kind()?;
            Some((f, kind, f.link.linker()?))
        })
    };
    for (f, kind, name) in linked().filter(|(f, ..)| f.field_type.is_own_key()) {
        linkers
            .ensure(name, kind)
            .map_err(|_| e13(file, f, format!("linker `{name}` has the other kind")))?;
    }
    for (f, kind, name) in linked() {
        if !f.field_type.is_own_key() {
            match linkers.get(name) {
                None => return Err(e13(file, f, format!("linker `{name}` does not exist yet"))),
                Some(l) if l.kind() != kind => {
                    return Err(e13(file, f, format!("linker `{name}` has the other kind")))
                }
                Some(_) => {}
            }
        }
    }

    let columns = txt.columns();
    let binding = bind(
        &txt.header,
        &fields.iter().map(|f| &f.column).collect::<Vec<_>>(),
    );
    let missing: Vec<usize> = binding.missing_fields().collect();
    if columns + missing.len() > MAX_COLUMNS {
        return Err(TxtError::new(file, ErrorCode::E14).with_detail(format!(
            "{columns} columns + {} missing fields",
            missing.len()
        )));
    }
    let mut ctx = Ctx {
        file,
        rules,
        diagnostics: binding
            .duplicate_columns
            .iter()
            .map(|&c| Diagnostic {
                kind: DiagKind::DupColumn,
                line: 1,
                column: Some(c),
                field: None,
            })
            .collect(),
    };

    let count = txt.records.len();
    let mut records = vec![0u8; count * record_size];

    // Pass 1: own keys.
    let has_keys = binding
        .column_field
        .iter()
        .flatten()
        .any(|&f| fields[f].field_type.is_own_key());
    if has_keys {
        for (r, rec) in txt.records.iter().enumerate() {
            let record = &mut records[r * record_size..(r + 1) * record_size];
            for (c, cell) in rec.cells.iter().enumerate() {
                let Some(fi) = binding.column_field[c] else {
                    continue;
                };
                let f = &fields[fi];
                if f.field_type.is_own_key() {
                    pass1(&mut ctx, f, cell, record, rec.line, c, linkers)?;
                }
            }
        }
    }

    // Pass 2: everything else, then missing fields.
    let linkers: &Linkers = linkers;
    for (r, rec) in txt.records.iter().enumerate() {
        let record = &mut records[r * record_size..(r + 1) * record_size];
        for (c, cell) in rec.cells.iter().enumerate() {
            let Some(fi) = binding.column_field[c] else {
                continue;
            };
            let f = &fields[fi];
            if !f.field_type.is_own_key() {
                pass2(
                    &mut ctx, f, cell, record, rec.line, c, r, linkers, callbacks,
                )?;
            }
        }
        for (k, &fi) in missing.iter().enumerate() {
            let f = &fields[fi];
            let mut diags = Vec::new();
            missing_value(f, record, r, columns + k, linkers, callbacks, &mut diags).map_err(
                |e| {
                    ctx.err(e.code, rec.line, columns + k, f)
                        .with_detail(e.detail)
                },
            )?;
            for d in diags {
                ctx.diag(d, rec.line, columns + k, f);
            }
        }
    }

    Ok(Compiled {
        count,
        record_size,
        records,
        diagnostics: ctx.diagnostics,
    })
}

fn text_cut(ctx: &mut Ctx, f: &FieldDef, cell: &[u8], line: usize, c: usize) {
    if text_limit(f).is_some_and(|lim| cell.len() > lim) {
        ctx.diag(DiagKind::TextCut, line, c, f);
    }
}

fn pass1(
    ctx: &mut Ctx,
    f: &FieldDef,
    cell: &[u8],
    record: &mut [u8],
    line: usize,
    c: usize,
    linkers: &mut Linkers,
) -> Result<(), TxtError> {
    text_cut(ctx, f, cell, line, c);
    let name = f.link.linker().unwrap_or_default();
    let linker = linkers.map.get_mut(name);
    match (f.field_type.id(), linker) {
        (10 | 12 | 14, Some(Linker::Code(l))) => {
            let code = code4(cell);
            let (index, bumped) = l.add(u32::from_le_bytes(code));
            if bumped {
                ctx.diag(DiagKind::DupCode, line, c, f);
            }
            match f.field_type.id() {
                10 => put(record, f.offset, &code),
                12 if index > 0xFF => return Err(ctx.err(ErrorCode::E12, line, c, f)),
                12 => put(record, f.offset, &code[..1]),
                _ if index > 0xFFFF => return Err(ctx.err(ErrorCode::E12, line, c, f)),
                _ => put(record, f.offset, &code[..2]),
            }
        }
        (16, Some(Linker::Name(l))) => {
            let n = cell.len().min(f.length.max(1) as usize - 1);
            let text = &cell[..n];
            let key = name_key(text).ok_or_else(|| ctx.err(ErrorCode::E11, line, c, f))?;
            put(record, f.offset, text);
            put(record, f.offset + n as u32, &[0]);
            if !l.add_always(&key).1 {
                ctx.diag(DiagKind::DupName, line, c, f);
            }
        }
        (17 | 18, Some(Linker::Name(l))) => {
            let key = name_key(cell).ok_or_else(|| ctx.err(ErrorCode::E11, line, c, f))?;
            let index = l.find_or_add(&key);
            if f.field_type.id() == 17 {
                put(record, f.offset, &(index as u16).to_le_bytes());
            } else {
                put(record, f.offset, &index.to_le_bytes());
            }
        }
        _ => {
            return Err(ctx
                .err(ErrorCode::E13, line, c, f)
                .with_detail("linker kind"))
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pass2(
    ctx: &mut Ctx,
    f: &FieldDef,
    cell: &[u8],
    record: &mut [u8],
    line: usize,
    c: usize,
    r: usize,
    linkers: &Linkers,
    callbacks: &mut dyn Callbacks,
) -> Result<(), TxtError> {
    let t = f.field_type;
    text_cut(ctx, f, cell, line, c);
    if t.is_integer() {
        let mut clamped = None;
        if !cell.is_empty() {
            match exact_int(cell) {
                None => ctx.diag(DiagKind::IntSyntax, line, c, f),
                Some(v) => {
                    let (lo, hi) = int_range(t);
                    if v < lo || v > hi {
                        ctx.diag(DiagKind::IntRange, line, c, f);
                        if ctx.rules == DataRules::Mod {
                            // Two's complement of the clamped value, as
                            // parse_int writes a negative cell.
                            clamped = Some(v.clamp(lo, hi) as i64 as u32);
                        }
                    }
                }
            }
        }
        let v = clamped.unwrap_or_else(|| parse_int(cell));
        match t.id() {
            2 | 8 => put(record, f.offset, &v.to_le_bytes()),
            3 => put(record, f.offset, &(v as u16).to_le_bytes()),
            5 if cell.is_empty() => {}
            26 => {
                let byte = f.offset as usize + (f.length as usize >> 3);
                let mask = 1u8 << (f.length & 7);
                if v != 0 {
                    record[byte] |= mask;
                } else {
                    record[byte] &= !mask;
                }
            }
            _ => put(record, f.offset, &[v as u8]),
        }
        return Ok(());
    }
    match t.id() {
        1 | 7 => {
            let n = cell.len().min(f.length as usize);
            put(record, f.offset, &cell[..n]);
            put(record, f.offset + n as u32, &[0]);
        }
        9 => put(record, f.offset, &code4(cell)),
        11 | 13 | 15 | 19 | 20 | 21 => {
            let name = f.link.linker().unwrap_or_default();
            let index = match linkers.get(name) {
                Some(Linker::Code(l)) => l.find(u32::from_le_bytes(code4(cell))),
                Some(Linker::Name(l)) => {
                    let key = name_key(cell).ok_or_else(|| ctx.err(ErrorCode::E11, line, c, f))?;
                    l.find(&key)
                }
                None => return Err(ctx.err(ErrorCode::E13, line, c, f)),
            };
            if index.is_none() && !cell.is_empty() {
                ctx.diag(DiagKind::LinkMiss, line, c, f);
            }
            let v = index.unwrap_or(u32::MAX);
            match t.id() {
                11 | 19 => put(record, f.offset, &v.to_le_bytes()),
                15 | 20 => put(record, f.offset, &(v as u16).to_le_bytes()),
                _ => put(record, f.offset, &[v as u8]),
            }
        }
        22 => {
            let id = callbacks.key(f, &cell[..cell.len().min(256)]);
            put(record, f.offset, &id.to_le_bytes());
        }
        23..=25 => {
            let mut diags = Vec::new();
            callbacks
                .field(FieldCall {
                    field: f,
                    text: Some(&cell[..cell.len().min(256)]),
                    record,
                    record_index: r as u32,
                    slot: c as u32,
                    linkers,
                    diagnostics: &mut diags,
                })
                .map_err(|e| ctx.err(e.code, line, c, f).with_detail(e.detail))?;
            for d in diags {
                ctx.diag(d, line, c, f);
            }
        }
        _ => unreachable!("own-key types are handled in pass 1"),
    }
    Ok(())
}

/// The value written for field `f` when its column is absent from the txt.
fn missing_value(
    f: &FieldDef,
    record: &mut [u8],
    r: usize,
    slot: usize,
    linkers: &Linkers,
    callbacks: &mut dyn Callbacks,
    diagnostics: &mut Vec<DiagKind>,
) -> Result<(), CallbackError> {
    match f.field_type.id() {
        2 | 8 | 9 | 10 | 18 => put(record, f.offset, &[0; 4]),
        3 | 14 | 17 | 22 => put(record, f.offset, &[0; 2]),
        1 | 4 | 5 | 6 | 7 | 12 | 16 => put(record, f.offset, &[0]),
        11 | 19 => put(record, f.offset, &[0xFF; 4]),
        15 | 20 => put(record, f.offset, &[0xFF; 2]),
        13 | 21 => put(record, f.offset, &[0xFF]),
        23..=25 => callbacks.field(FieldCall {
            field: f,
            text: None,
            record,
            record_index: r as u32,
            slot: slot as u32,
            linkers,
            diagnostics,
        })?,
        _ => {} // 26: nothing
    }
    Ok(())
}

#[cfg(test)]
mod tests;
