//! Runs layers over the patchable tables. Each statement's first failing
//! check is its error and it then has no effect; a failed structural
//! statement skips the layer's later statements on that table (N03); any
//! A error stops the stack after its layer (N04).

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::{Layer, Like, Sel, Stmt, Tok};
use super::{
    AddedColumn, Change, Code, Finding, Loc, Origin, PatchData, Row, SetChange, StrEdit, Writer,
};
use crate::schema::ExtType;
use crate::strings::StringEdit;
use crate::txt::names_equal;

/// `Expansion`, the removed-row marker.
const EXPANSION: &[u8] = b"Expansion";
/// Related rows listed for A04 and A05.
const RELATED_A04: usize = 10;
const RELATED_A05: usize = 5;

/// Resolves a column token against a header.
pub(crate) fn resolve_column(header: &[Vec<u8>], token: &[u8]) -> Option<usize> {
    let (name, k) = match token.iter().rposition(|&b| b == b'@') {
        Some(p) => {
            let ks = &token[p + 1..];
            let ok = !ks.is_empty() && ks[0] != b'0' && ks.iter().all(u8::is_ascii_digit);
            let k: usize = std::str::from_utf8(ks).ok().filter(|_| ok)?.parse().ok()?;
            (&token[..p], Some(k))
        }
        None => (token, None),
    };
    let m: Vec<usize> = (0..header.len()).filter(|&c| header[c] == name).collect();
    match (m.len(), k) {
        (1, None) => Some(m[0]),
        (n, Some(k)) if n > 1 && k <= n => Some(m[k - 1]),
        _ => None,
    }
}

/// Per-layer state.
#[derive(Default)]
struct LayerState {
    /// Cells written by `set`s and added key cells: (table, row, column).
    w: BTreeSet<(usize, usize, usize)>,
    /// Template rows.
    tp: BTreeSet<(usize, usize)>,
    /// Key writes: table, row, value, location.
    kw: Vec<(usize, Vec<u8>, Tok, usize)>,
    /// `R_T` by table name token.
    r: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>,
    /// Tables whose structural statement failed: first skipped (line,
    /// col) and count.
    failed: BTreeMap<Vec<u8>, Option<(usize, usize, usize)>>,
    findings: Vec<Finding>,
}

struct Ctx<'a> {
    data: &'a mut PatchData,
    pos: usize,
    path: String,
    line: usize,
}

impl Ctx<'_> {
    fn loc(&self) -> Loc {
        Loc {
            layer: self.pos,
            line: self.line,
        }
    }

    fn finding(&self, code: Code, tok: &Tok, table: usize) -> Finding {
        let mut f = Finding::new(code, &self.path, self.line, tok.col).at_pos(self.pos + 1);
        f.table = Some(self.data.tables[table].name().to_owned());
        f
    }

    fn with_row(&self, mut f: Finding, t: usize, row: usize) -> Finding {
        f.row = Some((row, self.data.tables[t].key(row).to_vec()));
        f
    }

    fn related_rows(&self, t: usize, rows: &[usize], max: usize) -> Vec<String> {
        rows.iter()
            .take(max)
            .map(|&r| format!("{} {r}", self.data.tables[t].name()))
            .collect()
    }

    /// Resolves a selector.
    fn resolve(&self, t: usize, sel: &Sel, r_t: &BTreeSet<Vec<u8>>) -> Result<usize, Box<Finding>> {
        let table = &self.data.tables[t];
        match sel {
            Sel::Key(k) => {
                if r_t.contains(&k.text) {
                    return Err(Box::new(self.finding(Code::A14, k, t)));
                }
                let rows = table.rows_keyed(&k.text);
                match rows.len() {
                    0 => Err(Box::new(self.finding(Code::A03, k, t))),
                    1 => Ok(rows[0]),
                    _ => {
                        let mut f = self.finding(Code::A04, k, t);
                        f.related = self.related_rows(t, &rows, RELATED_A04);
                        Err(f.into())
                    }
                }
            }
            Sel::Index { n, index, label } => {
                let n = *n as usize;
                if n >= table.rows.len() {
                    return Err(Box::new(self.finding(Code::A03, index, t)));
                }
                if table.key(n) != label.text.as_slice() {
                    let mut f = self.with_row(self.finding(Code::A05, index, t), t, n);
                    f.expected = Some(label.text.clone());
                    f.found = Some(table.key(n).to_vec());
                    f.related = self.related_rows(t, &table.rows_keyed(&label.text), RELATED_A05);
                    return Err(f.into());
                }
                Ok(n)
            }
        }
    }

    fn column(&self, t: usize, tok: &Tok) -> Result<usize, Box<Finding>> {
        let header = &self.data.tables[t].header;
        resolve_column(header, &tok.text).ok_or_else(|| {
            let mut f = self.finding(Code::A02, tok, t);
            // Related: the columns named exactly so.
            f.related = (0..header.len())
                .filter(|&c| header[c] == tok.text)
                .map(|c| format!("column {c}"))
                .collect();
            Box::new(f)
        })
    }

    fn set(
        &mut self,
        st: &mut LayerState,
        t: usize,
        (sel, column, old, new): (&Sel, &Tok, &Tok, &Tok),
        r_t: &BTreeSet<Vec<u8>>,
    ) -> Result<(), Box<Finding>> {
        let row = self.resolve(t, sel, r_t)?;
        let c = self.column(t, column)?;
        let at = |f: Finding, this: &Self| {
            let mut f = this.with_row(f, t, row);
            f.column = Some(this.data.tables[t].canonical(c));
            f
        };
        if st.w.contains(&(t, row, c)) {
            return Err(Box::new(at(self.finding(Code::A08, column, t), self)));
        }
        if st.tp.contains(&(t, row)) {
            return Err(Box::new(at(self.finding(Code::A15, sel.first(), t), self)));
        }
        let table = &self.data.tables[t];
        let cell = &table.rows[row].cells[c];
        let writer = table.rows[row].writers[c];
        if cell != &old.text {
            let mut f = at(self.finding(Code::A06, old, t), self);
            f.expected = Some(old.text.clone());
            f.found = Some(cell.clone());
            f.writer = Some(self.data.writer_name(t, row, writer));
            return Err(f.into());
        }
        if old.text == new.text {
            return Err(Box::new(at(self.finding(Code::A07, new, t), self)));
        }
        if c == 0 && new.text == EXPANSION {
            return Err(Box::new(at(self.finding(Code::A11, new, t), self)));
        }
        let is_key = c == table.key_col;
        if is_key && table.rules.unique && new.text.is_empty() {
            return Err(Box::new(at(self.finding(Code::A12, new, t), self)));
        }
        let ext = self.ext_col(t, c).map(|a| a.ty);
        if let Some(ty) = ext.filter(|_| !new.text.is_empty()) {
            if ty.value(&new.text).is_none() {
                let mut f = at(self.finding(Code::A17, new, t), self);
                f.detail = format!("not an `{}` value", ty.word());
                return Err(f.into());
            }
        }
        // Effect.
        let loc = self.loc();
        if let Some(prev) = writer.loc().filter(|l| l.layer != loc.layer) {
            let mut f = at(self.finding(Code::N01, old, t), self);
            f.writer = Some(self.data.loc_name(prev));
            f.detail = format!(
                "replaces {} written by {}",
                super::bracket(&old.text),
                self.data.loc_name(prev)
            );
            st.findings.push(f);
        }
        // Extension columns are read by mod code, not by a list.
        if !table.bound[c] && ext.is_none() {
            let f = at(self.finding(Code::N02, column, t), self);
            st.findings.push(f.detail("no list binds this column"));
        }
        let key = table.key(row).to_vec();
        let r = &mut self.data.tables[t].rows[row];
        r.cells[c] = new.text.clone();
        r.writers[c] = Writer::Set(loc);
        st.w.insert((t, row, c));
        if is_key {
            st.kw.push((t, new.text.clone(), new.clone(), self.line));
        }
        self.data.changes.push(Change {
            loc,
            table: t,
            row,
            key,
            set: Some(SetChange {
                column: c,
                old: old.text.clone(),
                new: new.text.clone(),
            }),
        });
        Ok(())
    }

    fn check(
        &self,
        t: usize,
        (sel, column, value): (&Sel, &Tok, &Tok),
        r_t: &BTreeSet<Vec<u8>>,
    ) -> Result<(), Box<Finding>> {
        let row = self.resolve(t, sel, r_t)?;
        let c = self.column(t, column)?;
        let table = &self.data.tables[t];
        let cell = &table.rows[row].cells[c];
        if cell != &value.text {
            let mut f = self.with_row(self.finding(Code::A06, value, t), t, row);
            f.column = Some(table.canonical(c));
            f.expected = Some(value.text.clone());
            f.found = Some(cell.clone());
            f.writer = Some(self.data.writer_name(t, row, table.rows[row].writers[c]));
            return Err(f.into());
        }
        Ok(())
    }

    fn add(
        &mut self,
        st: &mut LayerState,
        t: usize,
        (kw, n, index, key, like): (&Tok, u32, &Tok, &Tok, &Like),
        r_t: &BTreeSet<Vec<u8>>,
    ) -> Result<(), Box<Finding>> {
        let table = &self.data.tables[t];
        if table.rules.fixed {
            return Err(Box::new(self.finding(Code::A10, kw, t)));
        }
        if n as usize != table.rows.len() {
            let mut f = self.finding(Code::A09, index, t);
            f.detail = format!("row count is {}", table.rows.len());
            return Err(f.into());
        }
        let template = match like {
            None => None,
            Some((sel, pin)) => {
                let row = self.resolve(t, sel, r_t)?;
                if let Some(p) = pin {
                    let actual = table.pin(row);
                    if p.text != actual.as_bytes() {
                        let mut f = self.with_row(self.finding(Code::A13, p, t), t, row);
                        f.expected = Some(p.text.clone());
                        f.found = Some(actual.into_bytes());
                        return Err(f.into());
                    }
                }
                Some(row)
            }
        };
        if table.key_col == 0 && key.text == EXPANSION {
            return Err(Box::new(self.finding(Code::A11, key, t)));
        }
        if table.rules.unique && key.text.is_empty() {
            return Err(Box::new(self.finding(Code::A12, key, t)));
        }
        // Effect.
        let loc = self.loc();
        let width = table.header.len();
        let (mut cells, mut writers) = match template {
            Some(r) => (table.rows[r].cells.clone(), vec![Writer::Copy(loc); width]),
            None => (vec![Vec::new(); width], vec![Writer::Add(loc); width]),
        };
        if template.is_none() {
            // A new row starts with each extension column's default.
            for a in &table.ext {
                cells[a.column] = a.default.clone();
            }
        }
        let kc = table.key_col;
        cells[kc] = key.text.clone();
        writers[kc] = Writer::Add(loc);
        let row = table.rows.len();
        self.data.tables[t].rows.push(Row {
            origin: Origin::Added(loc),
            cells,
            writers,
        });
        st.w.insert((t, row, kc));
        if let Some(r) = template {
            st.tp.insert((t, r));
        }
        st.kw.push((t, key.text.clone(), key.clone(), self.line));
        self.data.changes.push(Change {
            loc,
            table: t,
            row,
            key: key.text.clone(),
            set: None,
        });
        Ok(())
    }

    /// The extension column at `c` of table `t`, if `c` is one.
    fn ext_col(&self, t: usize, c: usize) -> Option<&AddedColumn> {
        self.data.tables[t].ext.iter().find(|a| a.column == c)
    }

    /// `addcol`: appends an extension column, every row set to the
    /// default.
    fn addcol(
        &mut self,
        t: usize,
        (name, ty, default): (&Tok, ExtType, Option<&Tok>),
    ) -> Result<(), Box<Finding>> {
        let table = &self.data.tables[t];
        // A16: the name must be new to the header and to every field the
        // compiler could bind, else the column would read as a base field.
        let taken = table.header.iter().any(|h| names_equal(h, &name.text))
            || table
                .rules
                .lists
                .iter()
                .flatten()
                .any(|f| names_equal(f, &name.text));
        if taken {
            return Err(Box::new(self.finding(Code::A16, name, t)));
        }
        if let Some(d) = default.filter(|d| !d.text.is_empty()) {
            if ty.value(&d.text).is_none() {
                let mut f = self.finding(Code::A17, d, t);
                f.detail = format!("not an `{}` value", ty.word());
                return Err(f.into());
            }
        }
        // Effect.
        let default = default.map_or(Vec::new(), |d| d.text.clone());
        let loc = self.loc();
        let table = &mut self.data.tables[t];
        let column = table.header.len();
        table.header.push(name.text.clone());
        table.bound.push(false);
        for r in &mut table.rows {
            r.cells.push(default.clone());
            r.writers.push(Writer::Add(loc));
        }
        table.ext.push(AddedColumn {
            name: name.text.clone(),
            ty,
            default,
            column,
            loc,
        });
        Ok(())
    }
}

/// `R_T`: per table name, the old and new texts of the layer's `set`s
/// whose column token text is that table's key column name.
fn key_texts(data: &PatchData, layer: &Layer) -> BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>> {
    let mut r: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>> = BTreeMap::new();
    let mut cur: Option<usize> = None;
    let mut cur_name = Vec::new();
    for s in &layer.statements {
        match &s.stmt {
            Stmt::Table { name, .. } => {
                cur = data.table_index(&name.text);
                cur_name = name.text.clone();
            }
            Stmt::Set {
                column, old, new, ..
            } => {
                if let Some(t) = cur {
                    let table = &data.tables[t];
                    if column.text == table.header[table.key_col] {
                        let e = r.entry(cur_name.clone()).or_default();
                        e.insert(old.text.clone());
                        e.insert(new.text.clone());
                    }
                }
            }
            _ => {}
        }
    }
    r
}

/// Rows per (scope, normalized key) of the `unique` tables.
fn census(data: &PatchData) -> BTreeMap<(String, Vec<u8>), usize> {
    let mut out = BTreeMap::new();
    for t in data.tables.iter().filter(|t| t.rules.unique) {
        for i in 0..t.rows.len() {
            let k = t.rules.kind.normalize(t.key(i));
            *out.entry((t.rules.scope.clone(), k)).or_default() += 1;
        }
    }
    out
}

/// Rows of `scope` keyed `k` (normalized), sorted by (table, index).
fn key_counts(data: &PatchData, scope: &str, k: &[u8]) -> (usize, Vec<(String, usize)>) {
    let mut rows = Vec::new();
    for t in data.tables.iter().filter(|t| t.rules.scope == scope) {
        for i in 0..t.rows.len() {
            if t.rules.kind.normalize(t.key(i)) == k {
                rows.push((t.name().to_owned(), i));
            }
        }
    }
    rows.sort();
    (rows.len(), rows)
}

/// Applies `layers` in order to `data`. Returns the A and N findings,
/// sorted; any error means the stack stopped after the failing layer.
pub fn apply_stack(data: &mut PatchData, layers: &[Layer], stack_file: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    data.layers = layers.iter().map(|l| l.path.clone()).collect();
    for (pos, layer) in layers.iter().enumerate() {
        let mut st = LayerState {
            r: key_texts(data, layer),
            ..LayerState::default()
        };
        // `b`: counts at layer start, by (scope, normalized key).
        let start = census(data);
        let mut ctx = Ctx {
            data,
            pos,
            path: layer.path.clone(),
            line: 0,
        };
        let mut cur: Option<(Vec<u8>, Option<usize>)> = None;
        for s in &layer.statements {
            ctx.line = s.line;
            // String edits belong to no table: kept for the string tables.
            if let Stmt::Str { add, key, text, .. } = &s.stmt {
                let col = key.col;
                let (key, text) = (key.text.clone(), text.text.clone());
                ctx.data.strings.push(StrEdit {
                    edit: if *add {
                        StringEdit::Add { key, text }
                    } else {
                        StringEdit::Set { key, text }
                    },
                    path: ctx.path.clone(),
                    pos: pos + 1,
                    line: s.line,
                    col,
                });
                continue;
            }
            let name = match (&s.stmt, &cur) {
                (Stmt::Table { name, .. }, _) => name.text.clone(),
                (_, Some((name, _))) => name.clone(),
                // A layer with P errors can hold a statement with no
                // `table` before it (a `table` line that failed P08 still
                // opens the zone): P09 here too, never a panic.
                (stmt, None) => {
                    st.findings.push(
                        Finding::new(Code::P09, &ctx.path, s.line, stmt.kw().col).at_pos(pos + 1),
                    );
                    continue;
                }
            };
            if let Some(skip) = st.failed.get_mut(&name) {
                let kw = s.stmt.kw();
                match skip {
                    Some((_, _, n)) => *n += 1,
                    None => *skip = Some((s.line, kw.col, 1)),
                }
                continue;
            }
            let empty = BTreeSet::new();
            let r_t = st.r.get(&name).unwrap_or(&empty).clone();
            let (result, structural) = match &s.stmt {
                Stmt::Table { name: tok, .. } => {
                    let t = ctx.data.table_index(&tok.text);
                    cur = Some((tok.text.clone(), t));
                    match t {
                        Some(_) => (Ok(()), true),
                        None => {
                            let mut f =
                                Finding::new(Code::A01, &ctx.path, s.line, tok.col).at_pos(pos + 1);
                            f.detail = format!(
                                "no patchable table `{}`",
                                String::from_utf8_lossy(&tok.text)
                            );
                            (Err(Box::new(f)), true)
                        }
                    }
                }
                Stmt::Set {
                    sel,
                    column,
                    old,
                    new,
                    ..
                } => {
                    let t = cur.as_ref().and_then(|c| c.1).expect("table resolved");
                    let table = &ctx.data.tables[t];
                    let structural = column.text == table.header[table.key_col];
                    (
                        ctx.set(&mut st, t, (sel, column, old, new), &r_t),
                        structural,
                    )
                }
                Stmt::Check {
                    sel, column, value, ..
                } => {
                    let t = cur.as_ref().and_then(|c| c.1).expect("table resolved");
                    (ctx.check(t, (sel, column, value), &r_t), false)
                }
                Stmt::Add {
                    kw,
                    n,
                    index,
                    key,
                    like,
                } => {
                    let t = cur.as_ref().and_then(|c| c.1).expect("table resolved");
                    (ctx.add(&mut st, t, (kw, *n, index, key, like), &r_t), true)
                }
                Stmt::AddCol {
                    name: col,
                    ty,
                    default,
                    ..
                } => {
                    let t = cur.as_ref().and_then(|c| c.1).expect("table resolved");
                    (ctx.addcol(t, (col, *ty, default.as_ref())), true)
                }
                Stmt::Str { .. } => unreachable!("string edits are kept above"),
            };
            if let Err(f) = result {
                st.findings.push(*f);
                if structural {
                    st.failed.insert(name, None);
                }
            }
        }
        // End-of-layer uniqueness.
        let mut reported = BTreeSet::new();
        for (t, value, tok, line) in std::mem::take(&mut st.kw) {
            let rules = &ctx.data.tables[t].rules;
            if !rules.unique {
                continue;
            }
            let k = rules.kind.normalize(&value);
            let scope = rules.scope.clone();
            let (now, rows) = key_counts(ctx.data, &scope, &k);
            let b = start.get(&(scope.clone(), k.clone())).copied().unwrap_or(0);
            if now > b.max(1) && reported.insert((scope, k)) {
                let mut f = Finding::new(Code::A12, &ctx.path, line, tok.col).at_pos(pos + 1);
                f.table = Some(ctx.data.tables[t].name().to_owned());
                f.detail = "duplicate key in its scope".into();
                f.related = rows.iter().map(|(n, i)| format!("{n} {i}")).collect();
                st.findings.push(f);
            }
        }
        for (name, skip) in &st.failed {
            if let Some((line, col, n)) = skip {
                let mut f = Finding::new(Code::N03, &ctx.path, *line, *col).at_pos(pos + 1);
                f.table = Some(String::from_utf8_lossy(name).into_owned());
                f.detail = format!("{n} statements skipped");
                st.findings.push(f);
            }
        }
        let failed = st.findings.iter().any(|f| f.code.is_error());
        findings.extend(st.findings);
        if failed {
            if pos + 1 < layers.len() {
                let rest: Vec<&str> = layers[pos + 1..].iter().map(|l| l.path.as_str()).collect();
                let f = Finding::new(Code::N04, stack_file, 0, 0)
                    .detail(format!("not applied: {}", rest.join(", ")));
                findings.push(f);
            }
            break;
        }
    }
    super::sort_report(&mut findings);
    findings
}
