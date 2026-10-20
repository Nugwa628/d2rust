//! An edited `.txt` against a table state → a canonical layer, or the
//! first D error. Applying the output after that state gives the edited
//! rows exactly; the layer holds single values, never whole rows.

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::MAX_TOKEN;
use super::{canonical_column, canonical_token, pin, Code, Finding, PatchTable};
use crate::txt::TxtTable;

/// A diff failure: the first D finding.
pub type DiffError = Box<Finding>;

fn d(code: Code, t: &PatchTable, row: Option<usize>, detail: &str) -> Finding {
    let mut f = Finding::new(code, &format!("edited:{}", t.rules.txt_name), 0, 0);
    f.table = Some(t.name().to_owned());
    if let Some(r) = row {
        f.row = Some((r, Vec::new()));
        f.line = r + 2;
    }
    f.detail = detail.to_owned();
    f
}

/// D03: a value the layer needs must fit a token.
fn token(v: &[u8], t: &PatchTable, row: usize) -> Result<String, DiffError> {
    if v.len() > MAX_TOKEN
        || v.iter()
            .any(|&b| !(0x20..=0x7E).contains(&b) || b == b'[' || b == b']')
    {
        return Err(Box::new(d(
            Code::D03,
            t,
            Some(row),
            "value not expressible in a layer",
        )));
    }
    Ok(String::from_utf8(canonical_token(v)).expect("ASCII"))
}

/// The statement lines of one table's diff.
fn diff_table(b: &PatchTable, edited: &[u8], no_like: bool) -> Result<Vec<String>, DiffError> {
    let file = format!("edited:{}", b.rules.txt_name);
    let x = TxtTable::parse(&file, edited)
        .map_err(|e| Box::new(d(Code::D04, b, None, &e.to_string())))?;
    if x.header != b.header {
        return Err(Box::new(d(Code::D02, b, None, "header differs")));
    }
    let xr: Vec<&Vec<Vec<u8>>> = x.records.iter().map(|r| &r.cells).collect();
    let br: Vec<&Vec<Vec<u8>>> = b.rows.iter().map(|r| &r.cells).collect();
    let (nb, nx) = (br.len(), xr.len());
    if nx < nb {
        return Err(Box::new(d(Code::D01, b, None, "rows removed")));
    }
    if nx > nb && b.rules.fixed {
        return Err(Box::new(d(
            Code::D06,
            b,
            None,
            "rows added to a fixed table",
        )));
    }
    let kc = b.key_col;
    let kind = b.rules.kind;
    let unique = b.rules.unique;
    let norm = |v: &[u8]| kind.normalize(v);
    for i in 0..nb {
        if xr[i] == br[i] {
            continue;
        }
        let moved = unique
            && xr[i][kc] != br[i][kc]
            && (0..nb).any(|j| j != i && norm(&br[j][kc]) == norm(&xr[i][kc]));
        let shifted = (i > 0 && xr[i] == br[i - 1]) || (i + 1 < nb && xr[i] == br[i + 1]);
        if moved || shifted {
            return Err(Box::new(d(
                Code::D05,
                b,
                Some(i),
                "rows inserted, deleted or sorted",
            )));
        }
    }
    if unique {
        let mut in_b: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for r in &br {
            *in_b.entry(norm(&r[kc])).or_default() += 1;
        }
        let mut in_x: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for r in &xr {
            *in_x.entry(norm(&r[kc])).or_default() += 1;
        }
        for (i, r) in xr.iter().enumerate() {
            let k = norm(&r[kc]);
            if in_x[&k] > in_b.get(&k).copied().unwrap_or(0).max(1) {
                return Err(Box::new(d(Code::D06, b, Some(i), "duplicate key")));
            }
            if r[kc].is_empty() && (i >= nb || !br[i][kc].is_empty()) {
                return Err(Box::new(d(Code::D06, b, Some(i), "empty key")));
            }
        }
    }

    // Renamed rows' old and new keys.
    let renamed: BTreeSet<&[u8]> = (0..nb)
        .filter(|&i| xr[i][kc] != br[i][kc])
        .flat_map(|i| [br[i][kc].as_slice(), xr[i][kc].as_slice()])
        .collect();
    let count = |rows: &[&Vec<Vec<u8>>], k: &[u8]| rows.iter().filter(|r| r[kc] == k).count();
    let col = |c: usize, row: usize| token(&canonical_column(&b.header, c), b, row);
    let mut out = Vec::new();
    for i in 0..nb {
        if xr[i] == br[i] {
            continue;
        }
        let key = &br[i][kc];
        let sel = if count(&br, key) == 1 && !renamed.contains(key.as_slice()) {
            token(key, b, i)?
        } else {
            format!("#{i} {}", token(key, b, i)?)
        };
        let cols = (0..b.header.len())
            .filter(|&c| c != kc)
            .chain(std::iter::once(kc));
        for c in cols.filter(|&c| xr[i][c] != br[i][c]) {
            out.push(format!(
                "set {sel} {} {} -> {}",
                col(c, i)?,
                token(&br[i][c], b, i)?,
                token(&xr[i][c], b, i)?
            ));
        }
    }
    let empty = vec![Vec::new(); b.header.len()];
    for n in nb..nx {
        let row = xr[n];
        let key = token(&row[kc], b, n)?;
        let mut line = format!("add #{n} {key}");
        let filled: Vec<usize> = (0..row.len())
            .filter(|&c| c != kc && !row[c].is_empty())
            .collect();
        let template = if no_like {
            None
        } else {
            let mut best: Option<(usize, usize)> = None;
            for (t, xt) in xr.iter().enumerate().take(nb) {
                let shares = filled.iter().filter(|&&c| xt[c] == row[c]).count();
                if shares >= 1 && 2 * shares >= filled.len() && best.is_none_or(|(s, _)| shares > s)
                {
                    best = Some((shares, t));
                }
            }
            best.map(|(_, t)| t)
        };
        let base = match template {
            Some(t) => {
                let tk = &xr[t][kc];
                let tsel = if count(&xr[..n], tk) == 1 && !renamed.contains(tk.as_slice()) {
                    token(tk, b, t)?
                } else {
                    format!("#{t} {}", token(tk, b, t)?)
                };
                line.push_str(&format!(" like {tsel} {}", pin(&b.header, xr[t])));
                xr[t]
            }
            None => &empty,
        };
        out.push(line);
        for c in (0..row.len()).filter(|&c| c != kc && row[c] != base[c]) {
            out.push(format!(
                "set #{n} {key} {} {} -> {}",
                col(c, n)?,
                token(&base[c], b, n)?,
                token(&row[c], b, n)?
            ));
        }
    }
    Ok(out)
}

/// Diffs edited files against their tables' states: a canonical
/// layer with one section per changed table, by name.
pub fn diff_tables(edits: &[(&PatchTable, &[u8])], no_like: bool) -> Result<Vec<u8>, DiffError> {
    let mut sections: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (t, e) in edits {
        let lines = diff_table(t, e, no_like)?;
        if !lines.is_empty() {
            sections.insert(t.name(), lines);
        }
    }
    let mut out = String::from("d2patch 1\n");
    for (name, lines) in sections {
        out.push_str(&format!("\ntable {name}\n"));
        for l in lines {
            out.push_str(&l);
            out.push('\n');
        }
    }
    Ok(out.into_bytes())
}
