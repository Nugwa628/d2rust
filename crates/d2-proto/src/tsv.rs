//! Strict parser of the machine tables `tables/sim/client-messages.tsv` and
//! `server-messages.tsv`, and the check that the generated tables equal
//! them. The TSVs list every client-to-server and server-to-client message
//! (id, size rule, handler size, field layout). [`crate::codegen`] turns
//! them into [`crate::generated`]; this module is the parser it uses and
//! the check ([`check`]) that the generated tables still equal the TSVs.
//!
//! Errors (never defaults, METHODS M07): a wrong header or column count,
//! an id that is malformed, repeated or out of order (rows must run
//! 0x00, 0x01, ... without gaps), an unknown size-rule, handler-size or
//! layout syntax, an unknown enum word, a repeated message name, a layout
//! field outside a fixed size or over the id byte, two fields sharing a
//! bit, a `==N` handler size that differs from the fixed transport
//! size, and a `bits:` layout (S→C only) with a width outside 1..=32, an
//! `@` offset, a non-fixed size or more bits than the size holds.

use crate::generated::{CLIENT_MESSAGES, SERVER_MESSAGES};
use crate::schema::*;

/// `tables/sim/client-messages.tsv` (our spec file; no Blizzard data).
pub const CLIENT_TSV: &str = include_str!("../../../tables/sim/client-messages.tsv");
/// `tables/sim/server-messages.tsv`.
pub const SERVER_TSV: &str = include_str!("../../../tables/sim/server-messages.tsv");

const CLIENT_HEADER: &str = "id\tname\ttransport_size\thandler_size\tlayout\thandler\tkind\tgate\trequest\tscope\tconfirmed";
const SERVER_HEADER: &str =
    "id\tname\tsize\tlayout\tsender\tclient_handler\tclient_unit_handler\tproduced_by\tconfirmed";

/// A TSV error with its file and 1-based line.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{file}:{line}: {msg}")]
pub struct TsvError {
    /// The TSV file name.
    pub file: &'static str,
    /// 1-based line number.
    pub line: usize,
    /// What is wrong.
    pub msg: String,
}

/// A parsed `client-messages.tsv` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRow<'a> {
    /// Message id (the first byte on the wire).
    pub id: u8,
    /// Message name, unique within the file.
    pub name: &'a str,
    /// Size rule for the bytes on the wire.
    pub transport_size: SizeRule,
    /// Size the handler accepts.
    pub handler_size: HandlerSize,
    /// Named fields of the message.
    pub layout: Vec<Field<'a>>,
    /// Handler column: the handler key, `None` when the row has none.
    pub handler: Option<u32>,
    /// `kind` column.
    pub kind: Kind,
    /// `gate` column.
    pub gate: Gate,
    /// `request` column (empty when none).
    pub request: &'a str,
    /// `scope` column.
    pub scope: Scope,
    /// `confirmed` column.
    pub confirmed: Confirmed,
}

impl ClientRow<'_> {
    /// Whether the generated descriptor `m` says the same as this row.
    pub fn matches(&self, m: &ClientMessage) -> bool {
        self.id == m.id
            && self.name == m.name
            && self.transport_size == m.transport_size
            && self.handler_size == m.handler_size
            && self.layout[..] == m.layout[..]
            && self.handler == m.handler
            && self.kind == m.kind
            && self.gate == m.gate
            && self.request == m.request
            && self.scope == m.scope
            && self.confirmed == m.confirmed
    }
}

/// A parsed `server-messages.tsv` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRow<'a> {
    /// Message id (the first byte on the wire).
    pub id: u8,
    /// Message name, unique within the file.
    pub name: &'a str,
    /// Size rule for the message.
    pub size: SizeRule,
    /// Named fields of the message.
    pub layout: Vec<Field<'a>>,
    /// Sender column: one builder key per code path that builds the message.
    pub senders: Vec<u32>,
    /// `client_handler` column.
    pub client_handler: Option<u32>,
    /// `client_unit_handler` column.
    pub client_unit_handler: Option<u32>,
    /// `produced_by` column.
    pub produced_by: ProducedBy,
    /// `confirmed` column.
    pub confirmed: Confirmed,
}

impl ServerRow<'_> {
    /// Whether the generated descriptor `m` says the same as this row.
    pub fn matches(&self, m: &ServerMessage) -> bool {
        self.id == m.id
            && self.name == m.name
            && self.size == m.size
            && self.layout[..] == m.layout[..]
            && self.senders[..] == m.senders[..]
            && self.client_handler == m.client_handler
            && self.client_unit_handler == m.client_unit_handler
            && self.produced_by == m.produced_by
            && self.confirmed == m.confirmed
    }
}

/// Number with an optional `0x` prefix (hex) or decimal.
fn number(s: &str) -> Result<u32, String> {
    let r = match s.strip_prefix("0x") {
        Some(h) if !h.is_empty() && h.bytes().all(|c| c.is_ascii_hexdigit()) => {
            u32::from_str_radix(h, 16)
        }
        None if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) => s.parse(),
        _ => return Err(format!("bad number `{s}`")),
    };
    r.map_err(|_| format!("number out of range `{s}`"))
}

fn small<T: TryFrom<u32>>(s: &str) -> Result<T, String> {
    T::try_from(number(s)?).map_err(|_| format!("number out of range `{s}`"))
}

/// Id cell: `0x` and two uppercase hex digits.
fn id_cell(s: &str) -> Result<u8, String> {
    match s.strip_prefix("0x") {
        Some(h)
            if h.len() == 2
                && h.bytes()
                    .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c)) =>
        {
            Ok(u8::from_str_radix(h, 16).expect("checked hex"))
        }
        _ => Err(format!("bad id `{s}`")),
    }
}

/// Key cell (handler, sender): `-` or `0x` and eight hex digits.
fn addr(s: &str) -> Result<Option<u32>, String> {
    if s == "-" {
        return Ok(None);
    }
    match s.strip_prefix("0x") {
        Some(h) if h.len() == 8 => Ok(Some(number(s)?)),
        _ => Err(format!("bad address `{s}`")),
    }
}

fn message_name(s: &str) -> Result<&str, String> {
    let ok = s == "-"
        || (s.starts_with(|c: char| c.is_ascii_uppercase())
            && s.bytes().all(|c| c.is_ascii_alphanumeric()));
    if ok {
        Ok(s)
    } else {
        Err(format!("bad name `{s}`"))
    }
}

/// Parses a size-rule cell: a fixed byte count, one of the named rules
/// (`chat`, `chat26`, `af`), or a size read from a `u8` / `u16` field of the
/// message (`u16@1*1+3;cap=..;min=..`: width at offset, times, plus, bounds).
pub fn size_rule(s: &str) -> Result<SizeRule, String> {
    match s {
        "chat" => return Ok(SizeRule::Chat),
        "chat26" => return Ok(SizeRule::Chat26),
        "af" => return Ok(SizeRule::Af),
        _ => {}
    }
    if s.bytes().all(|c| c.is_ascii_digit()) {
        return Ok(SizeRule::Fixed(small(s)?));
    }
    let mut parts = s.split(';');
    let main = parts.next().unwrap_or_default();
    let (mut cap, mut min) = (None, None);
    for p in parts {
        match p.split_once('=') {
            Some(("cap", v)) if cap.is_none() => cap = Some(small::<u16>(v)?),
            Some(("min", v)) if min.is_none() => min = Some(small::<u16>(v)?),
            _ => return Err(format!("bad size option `{p}` in `{s}`")),
        }
    }
    let (main, add) = match main.split_once('+') {
        Some((m, a)) => (m, small::<u16>(a)?),
        None => (main, 0),
    };
    let (main, mul) = match main.split_once('*') {
        Some((m, k)) => (m, small::<u16>(k)?),
        None => (main, 1),
    };
    let (width, offset) = match main.split_once('@') {
        Some(("u8", o)) => (Width::U8, o),
        Some(("u16", o)) => (Width::U16, o),
        _ => return Err(format!("bad size rule `{s}`")),
    };
    Ok(SizeRule::Field {
        width,
        offset: small(offset)?,
        mul,
        add,
        cap,
        min: min.unwrap_or(0),
    })
}

fn handler_size(s: &str) -> Result<HandlerSize, String> {
    Ok(match s {
        "chat" => HandlerSize::Chat,
        "any" => HandlerSize::Any,
        "-" => HandlerSize::None,
        _ => {
            if let Some(n) = s.strip_prefix("==") {
                HandlerSize::Exact(small(n)?)
            } else if let Some((a, b)) = s.split_once("..") {
                let (a, b) = (small(a)?, small(b)?);
                if a > b {
                    return Err(format!("empty handler size range `{s}`"));
                }
                HandlerSize::Range(a, b)
            } else {
                return Err(format!("bad handler size `{s}`"));
            }
        }
    })
}

fn field_name(s: &str) -> Result<&str, String> {
    let ok = s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    if ok {
        Ok(s)
    } else {
        Err(format!("bad field name `{s}`"))
    }
}

fn field_type(s: &str) -> Result<FieldType, String> {
    Ok(match s {
        "u8" => FieldType::U8,
        "u16" => FieldType::U16,
        "u32" => FieldType::U32,
        "cstr" => FieldType::Cstr,
        "cstr16" => FieldType::Cstr16,
        _ => {
            let bits = |n: &str| -> Result<u8, String> {
                if n.is_empty() || n.starts_with('0') && n != "0" {
                    return Err(format!("bad field type `{s}`"));
                }
                small(n).map_err(|_| format!("bad field type `{s}`"))
            };
            if let Some(n) = s.strip_prefix("bytes") {
                let n: u16 = match n.bytes().all(|c| c.is_ascii_digit())
                    && !n.is_empty()
                    && !n.starts_with('0')
                {
                    true => n.parse().map_err(|_| format!("bad field type `{s}`"))?,
                    false => return Err(format!("bad field type `{s}`")),
                };
                FieldType::Bytes(n)
            } else if let Some(n) = s.strip_prefix("bit") {
                match bits(n)? {
                    n @ 0..=31 => FieldType::Bit(n),
                    _ => return Err(format!("bit out of range `{s}`")),
                }
            } else if let Some(n) = s.strip_prefix('u') {
                match bits(n)? {
                    n @ 1..=31 if n != 8 && n != 16 => FieldType::Bits(n),
                    _ => return Err(format!("bad bit-field width `{s}`")),
                }
            } else {
                return Err(format!("unknown field type `{s}`"));
            }
        }
    })
}

/// Bytes a fixed-offset field spans (bit fields span their u32; a packed
/// field has no byte offset: `None`).
pub fn field_bytes(ty: FieldType) -> Option<usize> {
    match ty {
        FieldType::U8 => Some(1),
        FieldType::U16 => Some(2),
        FieldType::U32 | FieldType::Bits(_) | FieldType::Bit(_) => Some(4),
        FieldType::Cstr16 => Some(16),
        FieldType::Bytes(n) => Some(n as usize),
        FieldType::Cstr | FieldType::Tail | FieldType::Packed { .. } => None,
    }
}

/// Bit range `[start, end)` of a fixed field in the message.
fn bit_range(ty: FieldType, off: usize) -> Option<(usize, usize)> {
    let b = off * 8;
    Some(match ty {
        FieldType::Bits(n) => (b, b + n as usize),
        FieldType::Bit(n) => (b + n as usize, b + n as usize + 1),
        _ => (b, b + 8 * field_bytes(ty)?),
    })
}

/// `bits:` layout cell (server-messages.tsv `layout`): fields
/// `name:width` packed LSB-first from bit 0 of byte 0, each starting where
/// the previous one ends. Widths are 1..=32; the message has a fixed size
/// and the fields fit in its bits; `@` offsets are not allowed.
fn bits_layout<'a>(s: &'a str, size: &SizeRule) -> Result<Vec<Field<'a>>, String> {
    let Some(n) = size.fixed() else {
        return Err("`bits:` layout needs a fixed size".into());
    };
    let mut out = Vec::new();
    let mut bit = 0usize;
    for tok in s.split(' ') {
        if tok.contains('@') {
            return Err(format!("`@` offset in a `bits:` layout: `{tok}`"));
        }
        let Some((name, w)) = tok.split_once(':') else {
            return Err(format!("bad layout field `{tok}`"));
        };
        let name = field_name(name)?;
        // Decimal, no leading zero.
        let width: u8 =
            match w.bytes().all(|c| c.is_ascii_digit()) && !(w.starts_with('0') && w != "0") {
                true => w.parse().map_err(|_| format!("bad bit width `{tok}`"))?,
                false => return Err(format!("bad bit width `{tok}`")),
            };
        if !(1..=PACKED_MAX_WIDTH).contains(&width) {
            return Err(format!("bit width out of range `{tok}`"));
        }
        if bit + width as usize > 8 * n {
            return Err(format!(
                "`{tok}` ends past bit {} of the fixed size {n}",
                8 * n
            ));
        }
        out.push(Field {
            name,
            ty: FieldType::Packed {
                bit: bit as u16,
                width,
            },
            offset: None,
        });
        bit += width as usize;
    }
    Ok(out)
}

/// Parses a layout cell (field list or `bits:` form), checked against the
/// size rule.
pub fn layout<'a>(s: &'a str, size: &SizeRule) -> Result<Vec<Field<'a>>, String> {
    let mut out: Vec<Field<'a>> = Vec::new();
    if s.is_empty() {
        return Ok(out);
    }
    if let Some(rest) = s.strip_prefix("bits:") {
        return match rest.strip_prefix(' ') {
            Some(rest) if !rest.is_empty() => bits_layout(rest, size),
            _ => Err(format!("bad `bits:` layout `{s}`")),
        };
    }
    for tok in s.split(' ') {
        if out.last().is_some_and(|f| f.ty == FieldType::Tail) {
            return Err(format!("field after a tail field: `{tok}`"));
        }
        let f = match tok.split_once(':') {
            Some((name, rest)) => {
                let name = field_name(name)?;
                match rest.split_once('@') {
                    Some((ty, off)) => Field {
                        name,
                        ty: field_type(ty)?,
                        offset: Some(small(off)?),
                    },
                    None if rest == "cstr" => {
                        if out.last().is_none_or(|f| f.ty != FieldType::Cstr) {
                            return Err(format!("`{tok}` without offset must follow a cstr"));
                        }
                        Field {
                            name,
                            ty: FieldType::Cstr,
                            offset: None,
                        }
                    }
                    None => return Err(format!("bad layout field `{tok}`")),
                }
            }
            None => match tok.split_once('@') {
                // `type@off` (unnamed) or `name@off` (tail).
                Some((head, off)) => match field_type(head) {
                    Ok(ty) => Field {
                        name: "",
                        ty,
                        offset: Some(small(off)?),
                    },
                    Err(_) => Field {
                        name: field_name(head)?,
                        ty: FieldType::Tail,
                        offset: Some(small(off)?),
                    },
                },
                None => return Err(format!("bad layout field `{tok}`")),
            },
        };
        if f.offset == Some(0) {
            return Err(format!("`{tok}` overlaps the id byte"));
        }
        if let (Some(off), Some(w), Some(n)) = (f.offset, field_bytes(f.ty), size.fixed()) {
            if off as usize + w > n {
                return Err(format!("`{tok}` ends past the fixed size {n}"));
            }
        }
        if f.ty == FieldType::Tail && size.fixed().is_some() {
            return Err(format!("tail field `{tok}` in a fixed-size message"));
        }
        out.push(f);
    }
    // Fixed fields share no bit.
    let mut ranges: Vec<(usize, usize, &str)> = out
        .iter()
        .filter_map(|f| {
            let (a, b) = bit_range(f.ty, f.offset? as usize)?;
            Some((a, b, f.name))
        })
        .collect();
    ranges.sort();
    for w in ranges.windows(2) {
        if w[1].0 < w[0].1 {
            return Err(format!("fields `{}` and `{}` overlap", w[0].2, w[1].2));
        }
    }
    Ok(out)
}

fn rows<'a>(
    text: &'a str,
    file: &'static str,
    header: &str,
    mut row: impl FnMut(u8, &[&'a str]) -> Result<(), String>,
) -> Result<(), TsvError> {
    let err = |line: usize, msg: String| TsvError { file, line, msg };
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, h)) if h == header => {}
        _ => return Err(err(1, "header does not match the spec's columns".into())),
    }
    let columns = header.split('\t').count();
    let mut next_id = 0u32;
    for (i, line) in lines {
        let line_no = i + 1;
        let cells: Vec<&str> = line.split('\t').collect();
        if cells.len() != columns {
            return Err(err(
                line_no,
                format!("{} cells, expected {columns}", cells.len()),
            ));
        }
        let id = id_cell(cells[0]).map_err(|m| err(line_no, m))?;
        if id as u32 != next_id {
            return Err(err(
                line_no,
                format!("id 0x{id:02X} out of order (expected 0x{next_id:02X})"),
            ));
        }
        next_id += 1;
        row(id, &cells).map_err(|m| err(line_no, format!("0x{id:02X}: {m}")))?;
    }
    if next_id == 0 {
        return Err(err(2, "no rows".into()));
    }
    Ok(())
}

fn unique_name<'a>(seen: &mut Vec<&'a str>, name: &'a str) -> Result<(), String> {
    if name != "-" {
        if seen.contains(&name) {
            return Err(format!("name `{name}` repeated"));
        }
        seen.push(name);
    }
    Ok(())
}

/// Parses `client-messages.tsv`, checking every row (see the module docs).
pub fn parse_client(text: &str) -> Result<Vec<ClientRow<'_>>, TsvError> {
    let mut out = Vec::new();
    let mut names = Vec::new();
    rows(text, "client-messages.tsv", CLIENT_HEADER, |id, c| {
        let name = message_name(c[1])?;
        unique_name(&mut names, name)?;
        let transport_size = size_rule(c[2])?;
        if matches!(transport_size, SizeRule::Chat26 | SizeRule::Af) {
            return Err(format!("server-only size rule `{}`", c[2]));
        }
        let handler_size = handler_size(c[3])?;
        if let HandlerSize::Exact(n) = handler_size {
            if transport_size != SizeRule::Fixed(n) || n == 0 {
                return Err(format!("handler size `{}` disagrees with `{}`", c[3], c[2]));
            }
        }
        out.push(ClientRow {
            id,
            name,
            transport_size,
            handler_size,
            layout: match layout(c[4], &transport_size)? {
                l if l.iter().any(|f| matches!(f.ty, FieldType::Packed { .. })) => {
                    return Err(format!("`bits:` layout is S→C only: `{}`", c[4]))
                }
                l => l,
            },
            handler: addr(c[5])?,
            kind: match c[6] {
                "handler" => Kind::Handler,
                "stub0" => Kind::Stub0,
                "stub3" => Kind::Stub3,
                "system" => Kind::System,
                "none" => Kind::None,
                k => return Err(format!("bad kind `{k}`")),
            },
            gate: match c[7] {
                "alive" => Gate::Alive,
                "dead" => Gate::Dead,
                "none" => Gate::None,
                "system" => Gate::System,
                "-" => Gate::Unset,
                g => return Err(format!("bad gate `{g}`")),
            },
            request: c[8],
            scope: match c[9] {
                "sim" => Scope::Sim,
                "session" => Scope::Session,
                "out" => Scope::Out,
                "none" => Scope::None,
                s => return Err(format!("bad scope `{s}`")),
            },
            confirmed: confirmed(c[10])?,
        });
        Ok(())
    })?;
    Ok(out)
}

fn confirmed(s: &str) -> Result<Confirmed, String> {
    match s {
        "yes" => Ok(Confirmed::Yes),
        "partial" => Ok(Confirmed::Partial),
        _ => Err(format!("bad confirmed `{s}`")),
    }
}

/// Parses `server-messages.tsv`.
pub fn parse_server(text: &str) -> Result<Vec<ServerRow<'_>>, TsvError> {
    let mut out = Vec::new();
    let mut names = Vec::new();
    rows(text, "server-messages.tsv", SERVER_HEADER, |id, c| {
        let name = message_name(c[1])?;
        unique_name(&mut names, name)?;
        let size = size_rule(c[2])?;
        if size == SizeRule::Chat {
            return Err("client-only size rule `chat`".into());
        }
        let senders = if c[4] == "-" {
            Vec::new()
        } else {
            c[4].split(' ')
                .map(|a| addr(a)?.ok_or_else(|| format!("bad sender `{}`", c[4])))
                .collect::<Result<_, _>>()?
        };
        out.push(ServerRow {
            id,
            name,
            size,
            layout: layout(c[3], &size)?,
            senders,
            client_handler: addr(c[5])?,
            client_unit_handler: addr(c[6])?,
            produced_by: match c[7] {
                "sim" => ProducedBy::Sim,
                "session" => ProducedBy::Session,
                "transport" => ProducedBy::Transport,
                "out" => ProducedBy::Out,
                "none" => ProducedBy::None,
                p => return Err(format!("bad produced_by `{p}`")),
            },
            confirmed: confirmed(c[8])?,
        });
        Ok(())
    })?;
    Ok(out)
}

/// Compares parsed TSVs with the generated tables: one entry per row that
/// differs (`client 0x05`, `server 0x1A`), plus row-count differences.
pub fn diff(client: &[ClientRow], server: &[ServerRow]) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..client.len().max(CLIENT_MESSAGES.len()) {
        match (client.get(i), CLIENT_MESSAGES.get(i)) {
            (Some(r), Some(m)) if r.matches(m) => {}
            _ => out.push(format!("client 0x{i:02X}")),
        }
    }
    for i in 0..server.len().max(SERVER_MESSAGES.len()) {
        match (server.get(i), SERVER_MESSAGES.get(i)) {
            (Some(r), Some(m)) if r.matches(m) => {}
            _ => out.push(format!("server 0x{i:02X}")),
        }
    }
    out
}

/// [`diff`] of the TSV texts.
pub fn check(client_tsv: &str, server_tsv: &str) -> Result<Vec<String>, TsvError> {
    Ok(diff(&parse_client(client_tsv)?, &parse_server(server_tsv)?))
}

#[cfg(test)]
mod mutant_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // The shipped tables parse, and spot rows come out as expected.
    #[test]
    fn spec_tables_parse() {
        let c = parse_client(CLIENT_TSV).unwrap();
        let s = parse_server(SERVER_TSV).unwrap();
        assert_eq!(c.len(), 0x73);
        assert_eq!(s.len(), 0xB7);
        assert_eq!(
            c[0x66].transport_size,
            SizeRule::Field {
                width: Width::U16,
                offset: 1,
                mul: 1,
                add: 3,
                cap: Some(0x1FD),
                min: 3
            }
        );
        assert_eq!(c[0x14].handler_size, HandlerSize::Range(4, 275));
        assert_eq!(
            c[0x51].layout[1],
            Field {
                name: "left",
                ty: FieldType::Bit(15),
                offset: Some(1)
            }
        );
        assert_eq!(c[0x14].layout[3].offset, None);
        assert_eq!(s[0x06].senders, vec![0x0053B320, 0x0053B240]);
        assert_eq!(s[0x80].client_handler, None);
    }

    /// The generated tables say what the TSVs say.
    #[test]
    fn tables_match_tsv() {
        assert_eq!(check(CLIENT_TSV, SERVER_TSV).unwrap(), Vec::<String>::new());
    }

    fn replace_line(text: &str, prefix: &str, from: &str, to: &str) -> String {
        let mut hit = 0;
        let out: Vec<String> = text
            .lines()
            .map(|l| {
                if l.starts_with(prefix) {
                    hit += 1;
                    l.replacen(from, to, 1)
                } else {
                    l.to_string()
                }
            })
            .collect();
        assert_eq!(hit, 1);
        out.join("\n") + "\n"
    }

    /// M08: a changed row is reported, and only that row.
    #[test]
    fn check_reports_exactly_a_changed_row() {
        let c = replace_line(CLIENT_TSV, "0x05\t", "\t5\t==5\t", "\t6\t==6\t");
        assert_eq!(check(&c, SERVER_TSV).unwrap(), vec!["client 0x05"]);
        let c = replace_line(CLIENT_TSV, "0x3A\t", "repeat:u8@2", "count:u8@2");
        assert_eq!(check(&c, SERVER_TSV).unwrap(), vec!["client 0x3A"]);
        let s = replace_line(SERVER_TSV, "0x1A\t", "\t2\t", "\t3\t");
        assert_eq!(check(CLIENT_TSV, &s).unwrap(), vec!["server 0x1A"]);
        let s = replace_line(SERVER_TSV, "0x8F\t", "transport", "out");
        assert_eq!(check(CLIENT_TSV, &s).unwrap(), vec!["server 0x8F"]);
    }

    fn client_err(text: &str) -> String {
        parse_client(text).unwrap_err().msg
    }

    #[test]
    fn strict_errors() {
        // Size disagrees with the layout or the handler size.
        let t = replace_line(CLIENT_TSV, "0x01\t", "y:u16@3", "y:u16@4");
        assert!(client_err(&t).contains("past the fixed size"));
        let t = replace_line(CLIENT_TSV, "0x01\t", "==5", "==6");
        assert!(client_err(&t).contains("disagrees"));
        // Overlapping fields.
        let t = replace_line(CLIENT_TSV, "0x01\t", "y:u16@3", "y:u16@2");
        assert!(client_err(&t).contains("overlap"));
        // Unknown layout syntax and types.
        let t = replace_line(CLIENT_TSV, "0x01\t", "y:u16@3", "y:i16@3");
        assert!(client_err(&t).contains("unknown field type"));
        let t = replace_line(CLIENT_TSV, "0x01\t", "y:u16@3", "y=u16@3");
        assert!(client_err(&t).contains("bad"));
        let t = replace_line(CLIENT_TSV, "0x01\t", "y:u16@3", "y:u8@3x");
        assert!(client_err(&t).contains("bad number"));
        // Bad size rule.
        let t = replace_line(CLIENT_TSV, "0x66\t", ";min=3", ";max=3");
        assert!(client_err(&t).contains("bad size option"));
        // Bad, duplicate and missing ids.
        let t = replace_line(CLIENT_TSV, "0x01\t", "0x01", "0x1");
        assert!(client_err(&t).contains("bad id"));
        let t = replace_line(CLIENT_TSV, "0x02\t", "0x02", "0x01");
        assert!(client_err(&t).contains("out of order"));
        let t = replace_line(CLIENT_TSV, "0x02\t", "0x02", "0x03");
        assert!(client_err(&t).contains("out of order"));
        // Unknown enum word, repeated name.
        let t = replace_line(CLIENT_TSV, "0x01\t", "alive", "living");
        assert!(client_err(&t).contains("bad gate"));
        let t = replace_line(CLIENT_TSV, "0x03\t", "\tRun\t", "\tWalk\t");
        assert!(client_err(&t).contains("repeated"));
        // Header and line number.
        let t = CLIENT_TSV.replacen("transport_size", "size", 1);
        assert_eq!(parse_client(&t).unwrap_err().line, 1);
        let t = replace_line(CLIENT_TSV, "0x01\t", "alive", "living");
        assert_eq!(parse_client(&t).unwrap_err().line, 3);
    }

    // Bit ranges of the packed fields of message 0x96.
    #[test]
    fn bits_layout_0x96_bit_ranges() {
        let s = parse_server(SERVER_TSV).unwrap();
        let ranges: Vec<(&str, u16, u8)> = s[0x96]
            .layout
            .iter()
            .map(|f| match f.ty {
                FieldType::Packed { bit, width } => {
                    assert_eq!(f.offset, None);
                    (f.name, bit, width)
                }
                t => panic!("not packed: {t:?}"),
            })
            .collect();
        assert_eq!(
            ranges,
            [
                ("id", 0, 8),
                ("stamina", 8, 15),
                ("x", 23, 16),
                ("y", 39, 16),
                ("dx", 55, 8),
                ("dy", 63, 8),
            ]
        );
    }

    fn server_err(text: &str) -> String {
        parse_server(text).unwrap_err().msg
    }

    /// M08: each `bits:` rule rejects exactly its perturbation.
    #[test]
    fn bits_layout_strict_errors() {
        let row = "0x96\t";
        // Width sum past 8 × size: 71 bits of 72, then 73.
        let t = replace_line(SERVER_TSV, row, "dy:8", "dy:10");
        assert!(server_err(&t).contains("past bit 72 of the fixed size 9"));
        let t = replace_line(SERVER_TSV, row, "dy:8", "dy:9");
        assert_eq!(parse_server(&t).unwrap()[0x96].layout.len(), 6);
        // Size shrunk under the fields.
        let t = replace_line(SERVER_TSV, row, "\t9\t", "\t8\t");
        assert!(server_err(&t).contains("past bit 64"));
        // Widths 1..=32.
        let t = replace_line(SERVER_TSV, row, "dx:8", "dx:0");
        assert!(server_err(&t).contains("bit width out of range"));
        let t = replace_line(SERVER_TSV, row, "\t9\t", "\t16\t");
        let t = replace_line(&t, row, "dy:8", "dy:33");
        assert!(server_err(&t).contains("bit width out of range"));
        let t = replace_line(SERVER_TSV, row, "dx:8", "dx:08");
        assert!(server_err(&t).contains("bad bit width"));
        // No `@` offsets or byte types mixed in.
        let t = replace_line(SERVER_TSV, row, "dx:8", "dx:u8@7");
        assert!(server_err(&t).contains("`@` offset"));
        let t = replace_line(SERVER_TSV, row, "dx:8", "dx");
        assert!(server_err(&t).contains("bad layout field"));
        let t = replace_line(SERVER_TSV, row, "bits: ", "bits:");
        assert!(server_err(&t).contains("bad `bits:` layout"));
        // Fixed size only.
        let t = replace_line(SERVER_TSV, row, "\t9\t", "\tu8@1+2\t");
        assert!(server_err(&t).contains("needs a fixed size"));
        // S→C only.
        let t = replace_line(
            CLIENT_TSV,
            "0x01\t",
            "x:u16@1 y:u16@3",
            "bits: id:8 x:16 y:16",
        );
        assert!(client_err(&t).contains("S→C only"));
    }

    /// The pack/unpack helpers and the typed struct agree with a 0x96
    /// message built by hand from the rule (LSB-first from bit 0 of byte
    /// 0, fields in layout order).
    #[test]
    fn bits_0x96_round_trip() {
        use crate::server::WalkVerify;
        use crate::FixedMessage;
        let (stamina, x, y, dx, dy) = (0x5ABCu128, 0x1234u128, 0xBEEFu128, 0x7Fu128, 0x80u128);
        let v = 0x96 | stamina << 8 | x << 23 | y << 39 | dx << 55 | dy << 63;
        let hand = &v.to_le_bytes()[..9];
        assert_eq!(hand, [0x96, 0xBC, 0x5A, 0x1A, 0x89, 0x77, 0xDF, 0x3F, 0x40]);

        let row = &crate::SERVER_MESSAGES[0x96];
        let want = [0x96, 0x5ABC, 0x1234, 0xBEEF, 0x7F, 0x80];
        let mut packed = [0u8; 9];
        for (f, &w) in row.layout.iter().zip(&want) {
            let FieldType::Packed { bit, width } = f.ty else {
                panic!("not packed")
            };
            assert_eq!(
                packed_get(hand, bit as usize, width as u32),
                w,
                "{}",
                f.name
            );
            packed_put(&mut packed, bit as usize, width as u32, w);
        }
        assert_eq!(packed, hand);

        let m = WalkVerify::decode(hand).unwrap();
        assert_eq!(
            m,
            WalkVerify {
                stamina: 0x5ABC,
                x: 0x1234,
                y: 0xBEEF,
                dx: 0x7F,
                dy: 0x80
            }
        );
        assert_eq!(m.encode(), hand);
        // The unused top bit (71) decodes to nothing and encodes as 0.
        let mut top = hand.to_vec();
        top[8] |= 0x80;
        assert_eq!(WalkVerify::decode(&top), Ok(m));
    }

    /// A value wider than its field is cut to the field's low bits
    /// (packed fields are masked on write), the neighbours untouched.
    #[test]
    fn packed_put_cuts_wide_values() {
        let mut b = [0; 9];
        packed_put(&mut b, 8, 15, 0x8000 | 0x1234);
        assert_eq!(packed_get(&b, 8, 15), 0x1234);
        assert_eq!((b[0], b[2] & 0x80, b[3]), (0, 0, 0));
        let mut b = [0; 9];
        packed_put(&mut b, 8, 15, u32::MAX);
        assert_eq!(packed_get(&b, 8, 15), 0x7FFF);
        assert_eq!((b[0], b[2] & 0x80, b[3]), (0, 0, 0));
    }
}
