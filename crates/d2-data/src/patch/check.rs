//! Compiles the patched cells and the unpatched base with the verified txt
//! compiler, reports new diagnostics (C02; C04 for the cut text, missing
//! links and clamped integers a mod game's data may have) and failed
//! live-set checks (C03), and builds the patched live set.

use std::collections::{BTreeMap, BTreeSet};

use super::{base_file, Code, Finding, Origin, PatchData, Writer};
use crate::bin::{self, BinSet, BinTable, CodeFile, DataRules};
use crate::calc::{self, Family, NO_KEYWORDS};
use crate::compile::DiagKind;
use crate::compile_set::{compile_all_under, CompiledSet};
use crate::schema::{schema, ExtStore, FieldType};
use crate::strings::StringTables;
use crate::txt::bind;

/// A diagnostic kind: the compiler's, or a `strkey` cell in no string
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Compile(DiagKind),
    StrMiss,
}

/// (table index, row, column, kind) → field name.
type Diags = BTreeMap<(usize, usize, Option<usize>, Kind), Option<String>>;

/// The result of a patched compile.
#[derive(Debug, Clone)]
pub struct PatchedCompile {
    /// The patched compile; `None` on C01.
    pub compiled: Option<CompiledSet>,
    /// The live set with the patched records and code buffers (no C
    /// error).
    pub live: Option<BinSet>,
    /// New diagnostics (C02) and failed live-set checks (C03).
    pub findings: Vec<Finding>,
    /// The extension columns the stack added (`addcol`), read by mod
    /// code; empty when no layer added one.
    pub ext: ExtStore,
}

fn compile(
    data: &PatchData,
    strings: &StringTables,
    rules: DataRules,
) -> Result<CompiledSet, String> {
    let mut read = |txt: &str| -> Result<Option<(String, Vec<u8>)>, String> {
        Ok(data
            .tables
            .iter()
            .find(|t| t.rules.txt_name == txt)
            .map(|t| (t.source.clone(), t.render())))
    };
    compile_all_under(&mut read, strings, &NO_KEYWORDS, rules).map_err(|e| e.to_string())
}

fn diagnostics(data: &PatchData, set: &CompiledSet, strings: &StringTables) -> Diags {
    let mut out = Diags::new();
    for ct in &set.tables {
        let def = schema().table(&ct.name).expect("compiled from the schema");
        let Some(t) = data
            .tables
            .iter()
            .position(|t| t.rules.txt_name == def.txt_name)
        else {
            continue;
        };
        for d in &ct.compiled.diagnostics {
            // Row i is line i + 2 of the rendered file.
            let row = d.line.saturating_sub(2);
            out.insert((t, row, d.column, Kind::Compile(d.kind)), d.field.clone());
        }
        let table = &data.tables[t];
        let names: Vec<&[u8]> = def.fields.iter().map(|f| f.column.as_slice()).collect();
        let binding = bind(&table.header, &names);
        for (f, field) in def.fields.iter().enumerate() {
            let (FieldType::KeyToWord, Some(c)) = (field.field_type, binding.field_column[f])
            else {
                continue;
            };
            for (i, r) in table.rows.iter().enumerate() {
                let cell = &r.cells[c][..r.cells[c].len().min(256)];
                if !cell.is_empty() && strings.id(cell) == 0 {
                    out.insert((t, i, Some(c), Kind::StrMiss), Some(field.name()));
                }
            }
        }
    }
    out
}

/// The linker a field of `txt` fills from, by field name.
fn field_linker(txt: &str, field: &str) -> Option<String> {
    schema()
        .called()
        .filter(|d| d.txt_name == txt)
        .flat_map(|d| d.fields.iter())
        .find(|f| f.column == field.as_bytes())
        .and_then(|f| f.link.linker().map(str::to_owned))
}

/// The records of runtime table `name` in the patched live set:
/// a record the stack left as the base compile has it keeps the bytes
/// the game ships in its `.bin` (they differ from the text compile only in
/// monstats #707 `NameStr`), so an empty
/// stack gives the shipped tables exactly; any other record is the patched
/// compile. Patch layers are D2RUST's own design: records are compared by
/// position over the rows all three sets share, so appended rows and
/// tables whose record size changed take the patched compile whole.
fn overlay_records(
    patched: &[u8],
    base: Option<&[u8]>,
    live: Option<&BinTable>,
    size: usize,
) -> Vec<u8> {
    let mut out = patched.to_vec();
    let (Some(base), Some(live)) = (base, live) else {
        return out;
    };
    if size == 0 || live.record_size != size {
        return out;
    }
    let shared = out.len().min(base.len()).min(live.records.len()) / size;
    for i in 0..shared {
        let r = i * size..(i + 1) * size;
        if out[r.clone()] == base[r.clone()] {
            out[r.clone()].copy_from_slice(&live.records[r]);
        }
    }
    out
}

fn new_live(
    live: &BinSet,
    compiled: &CompiledSet,
    base_set: &CompiledSet,
    data: &PatchData,
    strings: &StringTables,
    findings: &mut Vec<Finding>,
) -> BinSet {
    let mut tables: Vec<BinTable> = Vec::new();
    for def in schema().runtime() {
        let ct = compiled
            .table(&def.name)
            .expect("runtime tables are called");
        let t = BinTable {
            name: def.name.clone(),
            source: ct.txt_source.clone(),
            count: ct.compiled.count,
            record_size: ct.compiled.record_size,
            records: overlay_records(
                &ct.compiled.records,
                base_set
                    .table(&def.name)
                    .filter(|b| b.compiled.record_size == ct.compiled.record_size)
                    .map(|b| b.compiled.records.as_slice()),
                live.table(&def.name),
                ct.compiled.record_size,
            ),
        };
        if let Err(e) = bin::post_load_check_with(&t, &tables, strings, live.lod, live.rules) {
            let ti = data
                .tables
                .iter()
                .position(|p| p.rules.txt_name == def.txt_name);
            let mut f = Finding::new(Code::C03, "patched set", 0, 0).detail(e.to_string());
            f.table = Some(def.name.clone());
            f.related = data
                .changes
                .iter()
                .filter(|c| c.set.is_none() && Some(c.table) == ti)
                .map(|c| data.loc_name(c.loc))
                .collect();
            findings.push(f);
        }
        tables.push(t);
    }
    let mut code = BTreeMap::new();
    let mut code_reports = BTreeMap::new();
    for (&buffer, bytes) in &compiled.buffers {
        match calc::validate_buffer(
            Family::of(buffer),
            bytes,
            bin::formula_fields(buffer, &tables),
        ) {
            Ok(r) => {
                code_reports.insert(buffer, r);
            }
            Err(e) => {
                let mut f = Finding::new(Code::C03, "patched set", 0, 0)
                    .detail(format!("code buffer {}: {e:?}", buffer.name()));
                f.table = Some(buffer.name().to_owned());
                findings.push(f);
            }
        }
        code.insert(
            buffer,
            CodeFile {
                buffer,
                source: "patch".into(),
                bytes: bytes.clone(),
            },
        );
    }
    BinSet {
        lod: live.lod,
        rules: live.rules,
        strings: strings.clone(),
        tables,
        code,
        code_reports,
        hitclass: live.hitclass.clone(),
        sounds: live.sounds.clone(),
        soundenviron: live.soundenviron.clone(),
    }
}

/// Compiles `patched` and the unpatched `base`. `live` is the loaded
/// `.bin` set (string tables, client-only tables); its record tables and
/// code buffers are replaced by the patched compile.
pub fn compile_patched(base: &PatchData, patched: &PatchData, live: &BinSet) -> PatchedCompile {
    // The mod string edits apply first, so `strkey` cells bind added keys.
    let mut layered = live.strings.clone();
    let mut findings = super::apply_strings(&mut layered, &patched.strings);
    let strings = &layered;
    let c01 = |detail: String| vec![Finding::new(Code::C01, "patched set", 0, 0).detail(detail)];
    let compiled = match compile(patched, strings, live.rules) {
        Ok(c) => c,
        Err(e) => {
            return PatchedCompile {
                compiled: None,
                live: None,
                findings: c01(e),
                ext: patched.ext_store(),
            }
        }
    };
    let base_set = match compile(base, &live.strings, DataRules::Original) {
        Ok(c) => c,
        Err(e) => {
            return PatchedCompile {
                compiled: None,
                live: None,
                findings: c01(format!("base: {e}")),
                ext: patched.ext_store(),
            }
        }
    };
    let before: BTreeSet<_> = diagnostics(base, &base_set, &live.strings)
        .into_keys()
        .collect();
    for (key, field) in diagnostics(patched, &compiled, strings) {
        let (t, row, col, kind) = key;
        let table = &patched.tables[t];
        let Some(r) = table.rows.get(row) else {
            continue;
        };
        let added = matches!(r.origin, Origin::Added(_));
        if !added && before.contains(&key) {
            continue;
        }
        // A mod game's data compiles over-long text cut, missing links as
        // none and out-of-range integers clamped: reported, not refused.
        let tolerated = live.rules == DataRules::Mod
            && matches!(
                kind,
                Kind::Compile(DiagKind::TextCut | DiagKind::IntRange | DiagKind::LinkMiss)
            );
        let mut f = Finding::new(
            if tolerated { Code::C04 } else { Code::C02 },
            &base_file(&table.source, &table.rules.txt_name),
            match r.origin {
                Origin::Base(line) => line,
                Origin::Added(_) => 0,
            },
            col.map_or(0, |c| c + 1),
        );
        f.table = Some(table.name().to_owned());
        f.row = Some((row, table.key(row).to_vec()));
        f.column = col.map(|c| table.canonical(c));
        f.detail = match kind {
            Kind::Compile(k) => format!("{k:?}"),
            Kind::StrMiss => "StrMiss".into(),
        };
        if let Some(name) = &field {
            f.detail.push_str(&format!(" in field `{name}`"));
        }
        let writer = col.map_or(Writer::Base, |c| r.writers[c]);
        f.related = match writer.loc() {
            Some(l) => vec![patched.loc_name(l)],
            None => match field.and_then(|n| field_linker(&table.rules.txt_name, &n)) {
                None => Vec::new(),
                Some(linker) => patched
                    .changes
                    .iter()
                    .filter(|c| {
                        let tt = &patched.tables[c.table];
                        tt.rules.scope == linker
                            && c.set.as_ref().is_some_and(|s| s.column == tt.key_col)
                    })
                    .map(|c| patched.loc_name(c.loc))
                    .collect(),
            },
        };
        findings.push(f);
    }
    let live = new_live(live, &compiled, &base_set, patched, strings, &mut findings);
    super::sort_report(&mut findings);
    let ok = !super::has_errors(&findings);
    PatchedCompile {
        compiled: Some(compiled),
        live: ok.then_some(live),
        findings,
        ext: patched.ext_store(),
    }
}
