//! Generates the typed record structs of [`crate::tables`] from the
//! embedded schema: one struct per runtime table, one
//! field per field-list entry, decoded with explicit little-endian reads
//! at the entry's offset. `data-tool gen-tables` writes the output to
//! `src/tables/generated.rs`; a unit test checks the committed file is
//! current.
//!
//! Not covered (the field lists do not describe them): bytes written by
//! the table-specific callbacks and by the post-load fix-ups
//! ([`crate::fixup`]). Read those from the raw record.

use std::fmt::Write;

use crate::schema::{FieldDef, Link, Schema, TableDef};

/// Path of the generated file, relative to the `d2-data` crate.
pub const GENERATED_PATH: &str = "src/tables/generated.rs";

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "do", "dyn", "else",
    "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let", "loop",
    "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized",
    "use", "virtual", "where", "while", "yield",
];

/// Rust field name for a column: lowercase ASCII, other bytes → `_`
/// (runs collapsed, trimmed); `f_` before a leading digit; `_` after a
/// keyword.
pub fn field_ident(column: &[u8]) -> String {
    let mut s = String::new();
    for &b in column {
        let c = b.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            s.push(c as char);
        } else if !s.is_empty() && !s.ends_with('_') {
            s.push('_');
        }
    }
    while s.ends_with('_') {
        s.pop();
    }
    if s.is_empty() {
        s.push_str("field");
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert_str(0, "f_");
    }
    if KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// Rust type name for a table: its alphanumeric runs, each capitalized.
pub fn struct_ident(table: &str) -> String {
    table
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut cs = p.chars();
            let first = cs.next().unwrap().to_ascii_uppercase();
            std::iter::once(first).chain(cs).collect::<String>()
        })
        .collect()
}

/// (Rust type, decode expression) of a field; `None` for a table-specific
/// callback (type 23, 24), whose offset is only an argument.
fn field_code(f: &FieldDef) -> Option<(String, String)> {
    let o = f.offset;
    let len = f.length;
    Some(match f.field_type.id() {
        2 | 8 | 11 | 18 | 19 | 25 => ("u32".into(), format!("u32_at(r, {o})")),
        3 | 15 | 17 | 20 | 22 => ("u16".into(), format!("u16_at(r, {o})")),
        4..=6 | 12 | 13 | 21 => ("u8".into(), format!("r[{o}]")),
        // Text bytes; the NUL after them may be the next field's first
        // byte (see the `unwritten:` notes in `tables.tsv`), so it is not part of the field.
        1 | 7 => (format!("[u8; {len}]"), format!("bytes(r, {o})")),
        16 => {
            let n = len.max(1) - 1;
            (format!("[u8; {n}]"), format!("bytes(r, {o})"))
        }
        9 | 10 => ("[u8; 4]".into(), format!("bytes(r, {o})")),
        14 => ("[u8; 2]".into(), format!("bytes(r, {o})")),
        26 => (
            "bool".into(),
            format!("bit(r, {}, 0x{:02X})", o + (len >> 3), 1u32 << (len & 7)),
        ),
        _ => return None,
    })
}

fn field_doc(f: &FieldDef) -> String {
    let ty = f.field_type.vocabulary();
    let param = match f.field_type.id() {
        1 | 7 | 16 | 26 => format!("({})", f.length),
        _ => String::new(),
    };
    let link = match &f.link {
        Link::None => String::new(),
        Link::Linker(l) => format!(" → `{l}`"),
        Link::Calc(b) => format!(" → `calc({})`", b.name()),
        Link::Param => " → `param`".into(),
        Link::Table(t) => format!(" → `cb({t})`"),
    };
    format!(
        "`{}`: {ty}{param} at {}{link}",
        f.name().replace('`', "'"),
        f.offset
    )
}

fn table_code(out: &mut String, t: &TableDef) {
    let name = struct_ident(&t.name);
    let mut idents: Vec<String> = Vec::new();
    let mut fields = Vec::new();
    for (seq, f) in t.fields.iter().enumerate() {
        let Some((ty, expr)) = field_code(f) else {
            continue;
        };
        let mut id = field_ident(&f.column);
        if idents.contains(&id) {
            id = format!("{id}_{seq}");
        }
        idents.push(id.clone());
        fields.push((id, ty, expr, field_doc(f)));
    }
    let _ = writeln!(
        out,
        "\n/// `{}` (`{}`, {}-byte records).",
        t.name, t.bin_name, t.record_size
    );
    out.push_str("#[derive(Debug, Clone, PartialEq, Eq)]\n");
    let _ = writeln!(out, "pub struct {name} {{");
    for (id, ty, _, doc) in &fields {
        let _ = writeln!(out, "    /// {doc}\n    pub {id}: {ty},");
    }
    out.push_str("}\n\n");
    let _ = writeln!(out, "impl Record for {name} {{");
    let _ = writeln!(out, "    const TABLE: &'static str = \"{}\";", t.name);
    let _ = writeln!(out, "    const SIZE: usize = {};", t.record_size);
    let r = if fields.is_empty() { "_r" } else { "r" };
    let _ = writeln!(out, "    fn decode({r}: &[u8]) -> Self {{");
    let _ = writeln!(out, "        {name} {{");
    for (id, _, expr, _) in &fields {
        let _ = writeln!(out, "            {id}: {expr},");
    }
    out.push_str("        }\n    }\n}\n");
}

/// The generated module source for every runtime table of `schema`.
pub fn generate(schema: &Schema) -> String {
    let mut out = String::from(
        "// Generated from tables/data/fields.tsv and tables/data/tables.tsv.\n\
         // @generated by `data-tool gen-tables` (d2-data::codegen). Do not edit.\n\
         //! One struct per runtime table, one field per field-list entry.\n\n\
         use super::{bit, bytes, decode_all, u16_at, u32_at, Record, WrongTable};\n\
         use crate::bin::BinTable;\n",
    );
    let runtime: Vec<&TableDef> = schema.runtime().collect();
    for t in &runtime {
        table_code(&mut out, t);
    }
    out.push_str("\n/// The runtime tables, in load order.\npub const TABLES: &[&str] = &[\n");
    for t in &runtime {
        let _ = writeln!(out, "    \"{}\",", t.name);
    }
    out.push_str("];\n");
    out.push_str(
        "\n/// Decodes `t` with the struct of its table name: the record count,\n\
         /// or `None` for a name that is not a runtime table.\n\
         pub fn decode_by_name(t: &BinTable) -> Option<Result<usize, WrongTable>> {\n\
         \x20   Some(match t.name.as_str() {\n",
    );
    for t in &runtime {
        let _ = writeln!(
            out,
            "        \"{}\" => decode_all::<{}>(t).map(|v| v.len()),",
            t.name,
            struct_ident(&t.name)
        );
    }
    out.push_str("        _ => return None,\n    })\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::schema;

    #[test]
    fn idents() {
        assert_eq!(field_ident(b"lvl req"), "lvl_req");
        assert_eq!(field_ident(b"A1MaxD(H)"), "a1maxd_h");
        assert_eq!(field_ident(b"2handmindam"), "f_2handmindam");
        assert_eq!(field_ident(b"type"), "type_");
        assert_eq!(field_ident(b"*eol"), "eol");
        assert_eq!(field_ident(b"1.09-save add"), "f_1_09_save_add");
        assert_eq!(struct_ident("monstats2"), "Monstats2");
        assert_eq!(struct_ident("treasureclassex"), "Treasureclassex");
    }

    /// Every decoded field reads its own `fields.tsv` offset with the
    /// little-endian reader of its width.
    #[test]
    fn fields_decode_at_schema_offsets() {
        let mut fields = 0;
        for t in schema().runtime() {
            for f in &t.fields {
                let Some((ty, expr)) = field_code(f) else {
                    assert!(f.field_type.is_field_callback(), "{}", f.name());
                    continue;
                };
                fields += 1;
                let o = f.offset;
                let want = match f.field_type.id() {
                    26 => format!(
                        "bit(r, {}, 0x{:02X})",
                        o + (f.length >> 3),
                        1u32 << (f.length & 7)
                    ),
                    _ => match f.footprint().len() {
                        4 if ty == "u32" => format!("u32_at(r, {o})"),
                        2 if ty == "u16" => format!("u16_at(r, {o})"),
                        1 if ty == "u8" => format!("r[{o}]"),
                        _ => format!("bytes(r, {o})"),
                    },
                };
                assert_eq!(expr, want, "{}.{}", t.name, f.name());
            }
        }
        assert!(fields > 3_000);
        // The readers are little-endian.
        use crate::tables::{Itemstatcost, Record};
        let mut r = vec![0u8; Itemstatcost::SIZE];
        r[0..2].copy_from_slice(&[0x01, 0x02]);
        r[12..16].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        let d = Itemstatcost::decode(&r);
        assert_eq!((d.stat, d.divide), (0x0201, 0x0403_0201));
    }

    /// The committed file is what the generator writes today.
    #[test]
    fn generated_file_is_current() {
        let committed = include_str!("tables/generated.rs");
        assert!(
            generate(schema()) == committed,
            "src/tables/generated.rs is stale: run `cargo run -p data-tool -- gen-tables`"
        );
    }
}
