//! The five table-specific field callbacks (`cb(...)` in `fields.tsv`):
//! cube recipe inputs and outputs, monster skill modes, monster component
//! choices and monster preset placement. Each writes record bytes outside
//! the field footprints and nothing else.
//!
//! The parent `compile` module dispatches to `run` for `cb(<name>)` fields. This file
//! also builds the `@uniques` / `@sets` name linkers that the cube callbacks use to
//! resolve unique and set item names.

use super::{code4, name_key, CallbackError, CodeLinker, DiagKind, FieldCall, Linkers, NameLinker};
use crate::txt::ErrorCode;

/// Name of the add-always name linker over `uniqueitems`.
pub const UNIQUES_LINKER: &str = "@uniques";
/// Name of the add-always name linker over `setitems`.
pub const SETS_LINKER: &str = "@sets";

/// Base item code and level of a unique or set item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecialItem {
    /// The base item's 4-byte code, little-endian.
    pub code: u32,
    /// The item level read from the record.
    pub lvl: u16,
}

/// The unique and set items the cube callbacks resolve names to, by
/// record number. Filled after `uniqueitems` and `setitems` compile.
#[derive(Debug, Clone, Default)]
pub struct SpecialItems {
    /// Unique items, in `uniqueitems` record order.
    pub uniques: Vec<SpecialItem>,
    /// Set items, in `setitems` record order.
    pub sets: Vec<SpecialItem>,
}

/// Builds `@uniques` or `@sets` from compiled records: the
/// NUL-terminated `index` text at offset 2 of each record, registered
/// add-always in record order, plus each record's base code and level.
/// `None` when a name has a byte ≥ 0x80 (E11).
pub fn special_linker<'a>(
    records: impl IntoIterator<Item = &'a [u8]>,
    code_offset: usize,
    lvl_offset: usize,
) -> Option<(NameLinker, Vec<SpecialItem>)> {
    let mut linker = NameLinker::default();
    let mut items = Vec::new();
    for rec in records {
        let name = &rec[2..34];
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        linker.add_always(&name_key(&name[..end])?);
        items.push(SpecialItem {
            code: u32::from_le_bytes(rec[code_offset..code_offset + 4].try_into().unwrap()),
            lvl: u16::from_le_bytes([rec[lvl_offset], rec[lvl_offset + 1]]),
        });
    }
    Some((linker, items))
}

/// Runs callback `name` (`cb(<name>)`); `false` when it is not one of the
/// five.
pub(super) fn run(
    name: &str,
    call: FieldCall<'_>,
    special: &SpecialItems,
) -> Result<bool, CallbackError> {
    let slot = call.field.offset as usize;
    let FieldCall {
        text,
        record,
        linkers,
        diagnostics,
        ..
    } = call;
    let mut cx = Cx {
        linkers,
        special,
        diagnostics,
    };
    match name {
        "cubemain.input" => cx.cube_input(text, &mut record[20 + 8 * slot..28 + 8 * slot])?,
        "cubemain.output" => cx.cube_output(text, record, 76 + 84 * slot)?,
        "monstats.skillmode" => cx.skillmode(text, record, slot)?,
        "monstats2.composit" => cx.composit(text, record, slot)?,
        "monpreset.place" => cx.place(text, record)?,
        _ => return Ok(false),
    }
    Ok(true)
}

// --------------------------------------------------------------- helpers

/// Drops a leading `"` and cuts at the next `"`.
fn unquote(t: &[u8]) -> &[u8] {
    let t = t.strip_prefix(b"\"").unwrap_or(t);
    let end = t.iter().position(|&b| b == b'"').unwrap_or(t.len());
    &t[..end]
}

/// The bytes before the first byte in `set`, and the bytes
/// after it (`None` when there is no such byte).
fn split<'t>(t: &'t [u8], set: &[u8]) -> (&'t [u8], Option<&'t [u8]>) {
    match t.iter().position(|b| set.contains(b)) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    }
}

/// Parses a number like C `strtol(t, 10)` (leading whitespace, optional sign, digits),
/// clamped to i32.
fn num(t: &[u8]) -> i32 {
    let mut i = 0;
    while i < t.len() && matches!(t[i], 0x09..=0x0D | 0x20) {
        i += 1;
    }
    let neg = match t.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    for &d in t[i..].iter().take_while(|d| d.is_ascii_digit()) {
        v = (v * 10 + i64::from(d - b'0')).min(1 << 40);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn code_u32(t: &[u8]) -> u32 {
    u32::from_le_bytes(code4(t))
}

fn quality_word(t: &[u8]) -> Option<u8> {
    Some(match t {
        b"low" => 1,
        b"nor" => 2,
        b"hiq" => 3,
        b"mag" => 4,
        b"set" => 5,
        b"rar" => 6,
        b"uni" => 7,
        b"crf" => 8,
        b"tmp" => 9,
        _ => return None,
    })
}

fn input_flag(t: &[u8]) -> Option<u16> {
    Some(match t {
        b"nos" => 0x0004,
        b"sock" => 0x0008,
        b"eth" => 0x0010,
        b"noe" => 0x0020,
        b"upg" => 0x0080,
        b"bas" => 0x0100,
        b"exc" => 0x0200,
        b"eli" => 0x0400,
        b"nru" => 0x0800,
        _ => return None,
    })
}

fn output_flag(t: &[u8]) -> Option<u16> {
    Some(match t {
        b"mod" => 0x0001,
        b"eth" => 0x0004,
        b"uns" => 0x0010,
        b"rem" => 0x0020,
        b"exc" => 0x0080,
        b"eli" => 0x0100,
        b"rep" => 0x0200,
        b"rch" => 0x0400,
        _ => return None,
    })
}

fn get_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn set_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn or_u16(b: &mut [u8], o: usize, v: u16) {
    let x = get_u16(b, o) | v;
    set_u16(b, o, x);
}

fn e15(detail: &str) -> CallbackError {
    CallbackError {
        code: ErrorCode::E15,
        detail: detail.into(),
    }
}

/// The name key of a lookup; error E11 when a name byte is >= 0x80.
fn key(t: &[u8]) -> Result<Vec<u8>, CallbackError> {
    name_key(t).ok_or_else(|| CallbackError {
        code: ErrorCode::E11,
        detail: "callback name byte >= 0x80".into(),
    })
}

/// What a name resolved to.
struct Special {
    number: u16,
    item: u16,
    lvl: u16,
}

struct Cx<'a> {
    linkers: &'a Linkers,
    special: &'a SpecialItems,
    diagnostics: &'a mut Vec<DiagKind>,
}

impl Cx<'_> {
    fn code_linker(&self, name: &str) -> Result<&CodeLinker, CallbackError> {
        self.linkers.code(name).ok_or_else(|| CallbackError {
            code: ErrorCode::E13,
            detail: format!("linker `{name}` does not exist yet"),
        })
    }

    fn name_linker(&self, name: &str) -> Result<&NameLinker, CallbackError> {
        self.linkers.name(name).ok_or_else(|| CallbackError {
            code: ErrorCode::E13,
            detail: format!("linker `{name}` does not exist yet"),
        })
    }

    /// Index of item code `c` in `items.code`, or `None` (written as 0).
    fn item(&self, code: u32) -> Result<Option<u32>, CallbackError> {
        Ok(self.code_linker("items.code")?.find(code))
    }

    fn item_type(&self, code: u32) -> Result<Option<u32>, CallbackError> {
        Ok(self.code_linker("itemtypes.code")?.find(code))
    }

    /// A unique (`sets` false) or set item name; absent linkers skip.
    fn special(&self, first: &[u8], sets: bool) -> Result<Option<Special>, CallbackError> {
        let (linker, items) = if sets {
            (SETS_LINKER, &self.special.sets)
        } else {
            (UNIQUES_LINKER, &self.special.uniques)
        };
        let Some(l) = self.linkers.name(linker) else {
            return Ok(None);
        };
        let Some(n) = l.find(&key(first)?) else {
            return Ok(None);
        };
        let s = items[n as usize];
        Ok(Some(Special {
            number: n as u16 + 1,
            item: self.item(s.code)?.unwrap_or(0) as u16,
            lvl: s.lvl,
        }))
    }

    /// CbStop when the ignored text is non-empty.
    fn stop(&mut self, ignored: &[u8]) {
        if !ignored.is_empty() {
            self.diagnostics.push(DiagKind::CbStop);
        }
    }

    // ------------------------------------------------------- cube inputs

    fn cube_input(&mut self, text: Option<&[u8]>, s: &mut [u8]) -> Result<(), CallbackError> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let t = unquote(text);
        if t.is_empty() {
            return Ok(());
        }
        let (first, mut rest) = split(t, b",");
        let short = first.len() <= 4;
        let code = code_u32(first);
        if short && first.eq_ignore_ascii_case(b"any") {
            or_u16(s, 0, 0x0001);
            set_u16(s, 2, 0xFFFF);
        } else if let Some(i) = short.then(|| self.item_type(code)).transpose()?.flatten() {
            or_u16(s, 0, 0x0002);
            set_u16(s, 2, i as u16);
        } else if let Some(i) = short.then(|| self.item(code)).transpose()?.flatten() {
            or_u16(s, 0, 0x0001);
            set_u16(s, 2, i as u16);
        } else if let Some((sp, quality)) = match self.special(first, false)? {
            Some(sp) => Some((sp, 7)),
            None => self.special(first, true)?.map(|sp| (sp, 5)),
        } {
            or_u16(s, 0, 0x0041);
            s[6] = quality;
            set_u16(s, 4, sp.number);
            set_u16(s, 2, sp.item);
        } else {
            self.diagnostics.push(DiagKind::CbMiss);
            return Ok(());
        }
        while let Some(r) = rest {
            let (tok, after) = split(r, b"=,");
            rest = after;
            if tok == b"qty" {
                let v = rest.ok_or_else(|| e15("cube input `qty` without a value"))?;
                let (v, after) = split(v, b",");
                s[7] = num(v) as u8;
                rest = after;
            } else if let Some(q) = quality_word(tok) {
                s[6] = q;
            } else if let Some(f) = input_flag(tok) {
                or_u16(s, 0, f);
            } else {
                self.stop(r);
                break;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------ cube outputs

    fn cube_output(
        &mut self,
        text: Option<&[u8]>,
        record: &mut [u8],
        o: usize,
    ) -> Result<(), CallbackError> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let (first, mut rest) = split(unquote(text), b",");
        let short = first.len() <= 4;
        let code = code_u32(first);
        let kind: u8;
        if first.eq_ignore_ascii_case(b"Cow Portal") {
            kind = 1;
        } else if first.eq_ignore_ascii_case(b"Pandemonium Portal") {
            kind = 2;
        } else if first.eq_ignore_ascii_case(b"Pandemonium Finale Portal") {
            kind = 3;
        } else if first == b"usetype" {
            kind = 0xFF;
        } else if first == b"useitem" {
            kind = 0xFE;
        } else if let Some(i) = short.then(|| self.item(code)).transpose()?.flatten() {
            kind = 0xFC;
            set_u16(record, o + 2, i as u16);
        } else if let Some(i) = short.then(|| self.item_type(code)).transpose()?.flatten() {
            kind = 0xFD;
            set_u16(record, o + 2, i as u16);
        } else if let Some((sp, quality)) = match self.special(first, false)? {
            Some(sp) => Some((sp, 7)),
            None => self.special(first, true)?.map(|sp| (sp, 5)),
        } {
            or_u16(record, o, 0x0008);
            record[o + 6] = quality;
            set_u16(record, o + 4, sp.number);
            set_u16(record, o + 2, sp.item);
            record[o + 11] = sp.lvl as u8;
            kind = 0xFC;
            if quality == 5 {
                // The set branch exits before the modifiers.
                record[o + 8] = kind;
                self.stop(rest.unwrap_or_default());
                return Ok(());
            }
        } else {
            self.diagnostics.push(DiagKind::CbMiss);
            return Ok(());
        }
        record[o + 8] = kind;
        let (mut p, mut q) = (o + 12, o + 18);
        while let Some(r) = rest {
            let (tok, after) = split(r, b"=,");
            rest = after;
            match tok {
                b"qty" | b"pre" | b"suf" | b"sock" => {
                    let v = rest.ok_or_else(|| e15("cube output value word without a value"))?;
                    let (v, after) = split(v, b",");
                    rest = after;
                    let n = num(v);
                    match tok {
                        b"qty" => record[o + 7] = n as u8,
                        b"sock" => {
                            or_u16(record, o, 0x0002);
                            record[o + 7] = n as u8;
                        }
                        _ => {
                            let at = if tok == b"pre" { &mut p } else { &mut q };
                            if *at + 2 > record.len() {
                                return Err(e15("cube output `pre`/`suf` past the record"));
                            }
                            set_u16(record, *at, n as u16);
                            *at += 2;
                        }
                    }
                }
                b"reg" => {
                    or_u16(record, o, 0x0040);
                    record[o + 8] = 0xFF;
                }
                _ => {
                    if let Some(qu) = quality_word(tok) {
                        record[o + 6] = qu;
                    } else if let Some(f) = output_flag(tok) {
                        or_u16(record, o, f);
                    } else {
                        self.stop(r);
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------ skill modes

    fn skillmode(
        &mut self,
        text: Option<&[u8]>,
        record: &mut [u8],
        i: usize,
    ) -> Result<(), CallbackError> {
        let (mode_at, seq_at) = (384 + i, 392 + 2 * i);
        record[mode_at] = 0;
        set_u16(record, seq_at, 0xFFFF);
        let skill = get_u16(record, 368 + 2 * i) as i16;
        let Some(text) = text.filter(|_| skill >= 0) else {
            return Ok(());
        };
        if text.len() <= 3 {
            let m = self
                .code_linker("monmode_lookup.code")?
                .find(code_u32(text))
                .unwrap_or(u32::MAX) as u8;
            record[mode_at] = m;
            if (m as i8) >= 0 && m != 14 {
                return Ok(());
            }
        }
        record[mode_at] = 14;
        let k = key(text)?;
        let seq = self.name_linker("monseq.sequence")?.find(&k);
        if seq.is_none() && !text.is_empty() {
            self.diagnostics.push(DiagKind::CbMiss);
        }
        set_u16(record, seq_at, seq.unwrap_or(u32::MAX) as u16);
        Ok(())
    }

    // ------------------------------------------------- component choices

    fn composit(
        &mut self,
        text: Option<&[u8]>,
        record: &mut [u8],
        i: usize,
    ) -> Result<(), CallbackError> {
        let choices = 38 + 12 * i;
        record[21 + i] = 0;
        record[choices..choices + 12].fill(0xFF);
        let Some(text) = text else {
            return Ok(());
        };
        let linkers: &Linkers = self.linkers;
        let compcode = linkers.code("compcode.code").ok_or_else(|| CallbackError {
            code: ErrorCode::E13,
            detail: "linker `compcode.code` does not exist yet".into(),
        })?;
        let mut t = unquote(text);
        let mut n = 0;
        loop {
            let (tok, rest) = split(t, b",");
            if tok.is_empty() || tok.len() > 4 {
                self.stop(t);
                break;
            }
            let c = compcode.find(code_u32(tok));
            if c.is_none() {
                self.diagnostics.push(DiagKind::CbMiss);
            }
            record[choices + n] = c.unwrap_or(u32::MAX) as u8;
            n += 1;
            let Some(rest) = rest else { break };
            if n == 12 {
                self.stop(rest);
                break;
            }
            t = rest;
        }
        record[21 + i] = n as u8;
        if i == 15 {
            let total: u32 = record[21..37]
                .iter()
                .filter(|&&c| c >= 2)
                .map(|&c| u32::from(c) - 1)
                .sum();
            record[37] = total.min(254) as u8;
        }
        Ok(())
    }

    // ----------------------------------------------- preset placement

    fn place(&mut self, text: Option<&[u8]>, record: &mut [u8]) -> Result<(), CallbackError> {
        record[1] = 0;
        set_u16(record, 2, 0);
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Ok(());
        };
        let k = key(text)?;
        for (kind, linker) in [
            (2, "superuniques.Superunique"),
            (1, "monstats.Id"),
            (0, "monplace.code"),
        ] {
            if let Some(i) = self.name_linker(linker)?.find(&k) {
                record[1] = kind;
                set_u16(record, 2, i as u16);
                return Ok(());
            }
        }
        self.diagnostics.push(DiagKind::CbMiss);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
