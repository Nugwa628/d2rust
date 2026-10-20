//! Machine check for CLAUDE.md hard rule 6 (determinism in `d2-sim`).
//!
//! Scans the non-test sources of `crates/d2-sim/src` for constructs that can
//! make the simulation depend on anything but its inputs and the seeded RNG:
//! floats, hash collections, clocks, ambient RNG, file / env access, global
//! mutable state and `unsafe`. Comments, string / char literals and
//! `#[cfg(test)]` items (including the files of `#[cfg(test)] mod x;`) are not
//! scanned. Accepted exceptions live in `determinism-allow.txt`, one entry per
//! (file, rule) with a mandatory reason; an entry that matches nothing is an
//! error, so the list cannot go stale.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One finding: the rule that fired and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
}

/// Names that are a finding wherever they appear as an identifier.
const BANNED_IDENTS: &[(&str, &str)] = &[
    ("f32", "float"),
    ("f64", "float"),
    ("HashMap", "hash-collection"),
    ("HashSet", "hash-collection"),
    ("Instant", "clock"),
    ("SystemTime", "clock"),
    ("thread_rng", "ambient-rng"),
    ("unsafe", "unsafe"),
];

/// `std::<name>` / `core::<name>` paths that are a finding.
const BANNED_STD: &[(&str, &str)] = &[
    ("time", "clock"),
    ("fs", "io"),
    ("io", "io"),
    ("env", "env"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Num(String),
    Punct(char),
}

/// Replace comments, strings and char literals with spaces (newlines kept).
fn blank_non_code(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out: Vec<char> = b.clone();
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let n = b.get(i + 1).copied().unwrap_or('\0');
        if c == '/' && n == '/' {
            let s = i;
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            blank(&mut out, s, i);
        } else if c == '/' && n == '*' {
            let s = i;
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            blank(&mut out, s, i);
        } else if c == '"' || is_raw_start(&b, i).is_some() {
            let s = i;
            if let Some((hashes, open_end)) = is_raw_start(&b, i) {
                i = open_end;
                loop {
                    if i >= b.len() {
                        break;
                    }
                    if b[i] == '"' && (0..hashes).all(|k| b.get(i + 1 + k) == Some(&'#')) {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                while i < b.len() && b[i] != '"' {
                    i += if b[i] == '\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
            }
            blank(&mut out, s, i);
        } else if c == '\'' {
            // Char literal or lifetime.
            let is_char = n == '\\' || (b.get(i + 2) == Some(&'\'') && n != '\'');
            if is_char {
                let s = i;
                i += 1;
                while i < b.len() && b[i] != '\'' {
                    i += if b[i] == '\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                blank(&mut out, s, i);
            } else {
                i += 1;
            }
        } else if c.is_alphanumeric() || c == '_' {
            // Skip a whole identifier so `br` / `r` inside one is not a prefix.
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out.into_iter().collect()
}

/// `r"`, `r#"`, `br"`, `br#"` at `i` (not inside an identifier): returns the
/// number of `#` and the index just after the opening quote.
fn is_raw_start(b: &[char], i: usize) -> Option<(usize, usize)> {
    if i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_') {
        return None;
    }
    let mut j = i;
    if b.get(j) == Some(&'b') {
        j += 1;
    }
    if b.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while b.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    (b.get(j) == Some(&'"')).then_some((hashes, j + 1))
}

/// Tokens of blanked source with their 1-based line.
fn tokenize(code: &str) -> Vec<(Tok, usize)> {
    let b: Vec<char> = code.chars().collect();
    let mut toks = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let s = i;
            let hex = c == '0' && matches!(b.get(i + 1), Some('x' | 'b' | 'o'));
            i += 1;
            while i < b.len() {
                let d = b[i];
                if d.is_alphanumeric() || d == '_' {
                    // Exponent sign: `1e-3`, `2.5E+7` (decimal only).
                    if !hex
                        && (d == 'e' || d == 'E')
                        && matches!(b.get(i + 1), Some('+' | '-'))
                        && b.get(i + 2).is_some_and(|x| x.is_ascii_digit())
                    {
                        i += 2;
                    }
                    i += 1;
                } else if d == '.'
                    && !hex
                    && b.get(i + 1) != Some(&'.')
                    && !b.get(i + 1).is_some_and(|x| x.is_alphabetic() || *x == '_')
                    && !b[s..i].contains(&'.')
                {
                    // `1.5` and `1.` (but not `1..2`, `1.max(2)`).
                    i += 1;
                } else {
                    break;
                }
            }
            toks.push((Tok::Num(b[s..i].iter().collect()), line));
        } else if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            toks.push((Tok::Ident(b[s..i].iter().collect()), line));
        } else {
            toks.push((Tok::Punct(c), line));
            i += 1;
        }
    }
    toks
}

fn is_float_literal(num: &str) -> bool {
    let n = num.replace('_', "");
    if n.starts_with("0x") || n.starts_with("0b") || n.starts_with("0o") {
        return false;
    }
    if n.contains('.') || n.ends_with("f32") || n.ends_with("f64") {
        return true;
    }
    // Exponent form `2e3`, `1E+5`: digits, `e`, optional sign, digits only.
    // (Integer suffixes such as `usize` also contain an `e`.)
    match n.find(['e', 'E']) {
        Some(p) if p > 0 && n[..p].bytes().all(|c| c.is_ascii_digit()) => {
            let rest = n[p + 1..].trim_start_matches(['+', '-']);
            !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Index of the token after the item that starts at `i` (a `#[cfg(test)]`
/// attribute's following item): ends at `;` or the closing `}` at depth 0.
fn skip_item(toks: &[(Tok, usize)], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < toks.len() {
        match toks[i].0 {
            Tok::Punct('(' | '[') => depth += 1,
            Tok::Punct(')' | ']') => depth -= 1,
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => {
                depth -= 1;
                if depth <= 0 {
                    return i + 1;
                }
            }
            Tok::Punct(';') if depth <= 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// True if the attribute tokens (between `#[` and `]`) are a `cfg` that holds
/// only under test: `cfg(test)`, `cfg(all(test, ..))`; `not(test)` is not.
fn attr_is_cfg_test(attr: &[(Tok, usize)]) -> bool {
    let idents: Vec<&str> = attr
        .iter()
        .filter_map(|(t, _)| match t {
            Tok::Ident(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    idents.first() == Some(&"cfg")
        && idents.contains(&"test")
        && !idents.contains(&"not")
        && !idents.contains(&"any")
}

/// What a source scan found: hits, and external modules gated by cfg(test).
pub struct Scan {
    pub hits: Vec<Hit>,
    pub test_mods: Vec<String>,
}

pub fn scan_source(file: &str, src: &str) -> Scan {
    let toks = tokenize(&blank_non_code(src));
    let mut hits = Vec::new();
    let mut test_mods = Vec::new();
    let mut hit = |line: usize, rule: &'static str| {
        hits.push(Hit {
            file: file.to_owned(),
            line,
            rule,
        })
    };
    let mut i = 0;
    while i < toks.len() {
        // `#[attr]` / `#![attr]`
        if toks[i].0 == Tok::Punct('#') {
            let mut j = i + 1;
            if toks.get(j).map(|t| &t.0) == Some(&Tok::Punct('!')) {
                j += 1;
            }
            if toks.get(j).map(|t| &t.0) == Some(&Tok::Punct('[')) {
                let start = j + 1;
                let mut depth = 1;
                j += 1;
                while j < toks.len() && depth > 0 {
                    match toks[j].0 {
                        Tok::Punct('[') => depth += 1,
                        Tok::Punct(']') => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if attr_is_cfg_test(&toks[start..j.saturating_sub(1)]) {
                    // Record `mod name;` so the caller can skip its file.
                    let mut k = j;
                    while toks.get(k).map(|t| &t.0) == Some(&Tok::Punct('#')) {
                        // Further attributes on the same item.
                        while k < toks.len() && toks[k].0 != Tok::Punct(']') {
                            k += 1;
                        }
                        k += 1;
                    }
                    let mut m = k;
                    if matches!(&toks.get(m).map(|t| &t.0), Some(Tok::Ident(s)) if s == "pub") {
                        m += 1;
                        if toks.get(m).map(|t| &t.0) == Some(&Tok::Punct('(')) {
                            while m < toks.len() && toks[m].0 != Tok::Punct(')') {
                                m += 1;
                            }
                            m += 1;
                        }
                    }
                    if let (Some((Tok::Ident(kw), _)), Some((Tok::Ident(name), _)), Some((p, _))) =
                        (toks.get(m), toks.get(m + 1), toks.get(m + 2))
                    {
                        if kw == "mod" && *p == Tok::Punct(';') {
                            test_mods.push(name.clone());
                        }
                    }
                    i = skip_item(&toks, k);
                    continue;
                }
                i = j;
                continue;
            }
        }
        let line = toks[i].1;
        match &toks[i].0 {
            Tok::Ident(s) => {
                if let Some((_, rule)) = BANNED_IDENTS.iter().find(|(n, _)| n == s) {
                    hit(line, rule);
                }
                let next = toks.get(i + 1).map(|t| &t.0);
                let colons = toks.get(i + 1).map(|t| &t.0) == Some(&Tok::Punct(':'))
                    && toks.get(i + 2).map(|t| &t.0) == Some(&Tok::Punct(':'));
                if s == "thread_local" && next == Some(&Tok::Punct('!')) {
                    hit(line, "global-state");
                }
                if s == "static" && matches!(next, Some(Tok::Ident(m)) if m == "mut") {
                    hit(line, "global-state");
                }
                if s == "rand" && colons {
                    hit(line, "ambient-rng");
                }
                if (s == "std" || s == "core") && colons {
                    match toks.get(i + 3).map(|t| &t.0) {
                        Some(Tok::Ident(name)) => {
                            if let Some((_, rule)) = BANNED_STD.iter().find(|(n, _)| n == name) {
                                hit(line, rule);
                            }
                        }
                        // `use std::{fs, time::Duration};`
                        Some(Tok::Punct('{')) => {
                            let mut depth = 0;
                            let mut k = i + 3;
                            while k < toks.len() {
                                match &toks[k].0 {
                                    Tok::Punct('{') => depth += 1,
                                    Tok::Punct('}') => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    Tok::Ident(name) => {
                                        let prev_colons = k >= 2
                                            && toks[k - 1].0 == Tok::Punct(':')
                                            && toks[k - 2].0 == Tok::Punct(':');
                                        if !prev_colons || depth == 1 {
                                            if let Some((_, rule)) =
                                                BANNED_STD.iter().find(|(n, _)| n == name)
                                            {
                                                if depth == 1 {
                                                    hit(toks[k].1, rule);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                                k += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Tok::Num(n) => {
                let tuple_index = i > 0 && toks[i - 1].0 == Tok::Punct('.');
                if !tuple_index && is_float_literal(n) {
                    hit(line, "float");
                }
            }
            Tok::Punct(_) => {}
        }
        i += 1;
    }
    Scan { hits, test_mods }
}

/// Every `.rs` file below `dir`, sorted.
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Scan a crate's `src` tree; `rel_to` makes the reported paths relative.
pub fn scan_tree(src_dir: &Path, rel_to: &Path) -> Result<Vec<Hit>> {
    let mut files = Vec::new();
    rs_files(src_dir, &mut files)?;
    let mut skipped: BTreeSet<PathBuf> = BTreeSet::new();
    let mut scans = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let rel = f
            .strip_prefix(rel_to)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        let scan = scan_source(&rel, &src);
        // `#[cfg(test)] mod x;` in `dir/mod.rs` -> `dir/x.rs` | `dir/x/mod.rs`;
        // in `dir/stem.rs` -> `dir/stem/x.rs` | `dir/stem/x/mod.rs`.
        let parent = f.parent().unwrap_or(src_dir);
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let base = if matches!(stem, "mod" | "lib" | "main") {
            parent.to_path_buf()
        } else {
            parent.join(stem)
        };
        for m in &scan.test_mods {
            skipped.insert(base.join(format!("{m}.rs")));
            skipped.insert(base.join(m).join("mod.rs"));
        }
        scans.push((f.clone(), scan));
    }
    // A skipped file's own `mod` children are test code too.
    let mut hits = Vec::new();
    for (f, scan) in scans {
        let in_skipped_dir = skipped.iter().any(|s| {
            s.file_name().is_some_and(|n| n == "mod.rs")
                && s.parent().is_some_and(|d| f.starts_with(d))
                || s.file_stem()
                    .is_some_and(|stem| s.parent().is_some_and(|d| f.starts_with(d.join(stem))))
        });
        if skipped.contains(&f) || in_skipped_dir {
            continue;
        }
        hits.extend(scan.hits);
    }
    Ok(hits)
}

/// One allowlist entry: every `rule` hit in `file` is accepted, for `reason`.
#[derive(Debug)]
pub struct Allow {
    pub file: String,
    pub rule: String,
    #[allow(dead_code)] // read by humans in the file; parsed to enforce it
    pub reason: String,
    line: usize,
}

/// Format: `file<TAB>rule<TAB>reason`, `#` comments, blank lines ignored.
pub fn parse_allowlist(text: &str) -> Result<Vec<Allow>> {
    let mut out = Vec::new();
    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = t.splitn(3, '\t').map(str::trim).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            bail!(
                "determinism-allow.txt:{}: want `file<TAB>rule<TAB>reason` with a non-empty reason",
                i + 1
            );
        }
        out.push(Allow {
            file: parts[0].to_owned(),
            rule: parts[1].to_owned(),
            reason: parts[2].to_owned(),
            line: i + 1,
        });
    }
    Ok(out)
}

/// Violations (as printable lines): hits not allowed, and unused allow entries.
pub fn judge(hits: &[Hit], allow: &[Allow]) -> Vec<String> {
    let mut used = vec![false; allow.len()];
    let mut out = Vec::new();
    for h in hits {
        let mut ok = false;
        for (k, a) in allow.iter().enumerate() {
            if a.file == h.file && a.rule == h.rule {
                used[k] = true;
                ok = true;
            }
        }
        if !ok {
            out.push(format!(
                "{}:{}: determinism rule `{}` (CLAUDE.md rule 6)",
                h.file, h.line, h.rule
            ));
        }
    }
    for (k, a) in allow.iter().enumerate() {
        if !used[k] {
            out.push(format!(
                "determinism-allow.txt:{}: stale entry ({} / {}) matches no hit",
                a.line, a.file, a.rule
            ));
        }
    }
    out
}

/// Run the check on the workspace; returns violation lines.
pub fn check(root: &Path, allow_file: &Path) -> Result<Vec<String>> {
    let hits = scan_tree(&root.join("crates/d2-sim/src"), root)?;
    let allow = parse_allowlist(
        &std::fs::read_to_string(allow_file)
            .with_context(|| format!("reading {}", allow_file.display()))?,
    )?;
    Ok(judge(&hits, &allow))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(src: &str) -> Vec<(usize, &'static str)> {
        scan_source("x.rs", src)
            .hits
            .into_iter()
            .map(|h| (h.line, h.rule))
            .collect()
    }

    #[test]
    fn each_banned_construct_is_caught_with_its_line() {
        let cases: &[(&str, &str)] = &[
            ("let a: f32 = 0;", "float"),
            ("let a = x as f64;", "float"),
            ("let a = 1.5;", "float"),
            ("let a = 2e3;", "float"),
            ("let a = 1e-3;", "float"),
            ("let a = 7f32;", "float"),
            ("let a = 1E+5;", "float"),
            ("let a = 1.;", "float"),
            ("use std::collections::HashMap;", "hash-collection"),
            ("let s: HashSet<u8>;", "hash-collection"),
            ("use std::time::Duration;", "clock"),
            ("let t = Instant::now();", "clock"),
            ("let t = SystemTime::now();", "clock"),
            ("let r = rand::random::<u8>();", "ambient-rng"),
            ("let r = thread_rng();", "ambient-rng"),
            ("let s = std::fs::read(p);", "io"),
            ("use std::io::Write;", "io"),
            ("let v = std::env::var(k);", "env"),
            ("thread_local! { static X: u8 = 0; }", "global-state"),
            ("static mut X: u8 = 0;", "global-state"),
            ("unsafe { f() }", "unsafe"),
            ("use std::{fmt, fs};", "io"),
            ("use std::{fmt, time::Duration};", "clock"),
        ];
        for (src, rule) in cases {
            let got = rules(&format!("fn a() {{}}\n{src}\n"));
            assert!(
                got.contains(&(2, *rule)),
                "`{src}` should hit `{rule}` on line 2, got {got:?}"
            );
        }
    }

    #[test]
    fn comments_strings_and_chars_are_not_flagged() {
        let src = r##"
// f32 HashMap std::time unsafe
/* f64 /* nested thread_rng */ rand::x */
/// doc: Instant 1.5
fn a() {
    let s = "f32 HashMap std::fs 1.5 \" unsafe";
    let r = r#"HashSet "quoted" SystemTime"#;
    let b = b"f64";
    let c = 'f';
    let q = '\'';
    let l: &'static str = "ok";
    let t = pair.0.1;
    let n = 3.max(4);
    let r2 = 0..10;
    let h = 0xE5;
    let big = 1_000u32;
    let z = 0usize + 4isize as usize;
}
"##;
        assert_eq!(rules(src), vec![], "nothing in non-code should fire");
    }

    #[test]
    fn cfg_test_items_are_skipped_and_their_external_mods_recorded() {
        let src = "\
fn real() {}
#[cfg(test)]
mod tests {
    fn t() { let a = 1.5; let m: HashMap<u8, u8>; }
}
#[cfg(test)]
mod more;
#[cfg(test)]
#[allow(dead_code)]
fn helper() -> [u8; 2] { let f: f32 = 0.0; [0; 2] }
#[cfg(not(test))]
fn live() { let a = 2.5; }
fn tail() { unsafe {} }
";
        let scan = scan_source("x.rs", src);
        let got: Vec<_> = scan.hits.iter().map(|h| (h.line, h.rule)).collect();
        assert_eq!(got, vec![(12, "float"), (13, "unsafe")]);
        assert_eq!(scan.test_mods, vec!["more".to_string()]);
    }

    #[test]
    fn allowlist_needs_a_reason_and_goes_stale() {
        assert!(parse_allowlist("a.rs\tfloat\t").is_err());
        assert!(parse_allowlist("a.rs\tfloat").is_err());
        let allow = parse_allowlist("# c\na.rs\tfloat\twhy\n").unwrap();
        let hit = |file: &str| Hit {
            file: file.into(),
            line: 3,
            rule: "float",
        };
        assert!(judge(&[hit("a.rs")], &allow).is_empty());
        assert_eq!(judge(&[hit("b.rs")], &allow).len(), 2, "unlisted + stale");
        assert_eq!(judge(&[], &allow).len(), 1, "stale entry reported");
    }

    #[test]
    fn external_test_module_files_are_skipped_in_a_tree() {
        let dir = std::env::temp_dir().join(format!("depcheck-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m/tests")).unwrap();
        std::fs::write(
            dir.join("m/mod.rs"),
            "#[cfg(test)]\nmod tests;\nfn a() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("m/tests/mod.rs"),
            "mod deep;\nfn t(){ let a=1.5; }\n",
        )
        .unwrap();
        std::fs::write(dir.join("m/tests/deep.rs"), "fn t(){ let a=2.5; }\n").unwrap();
        std::fs::write(dir.join("m/bad.rs"), "fn t(){ let a=2.5; }\n").unwrap();
        let hits = scan_tree(&dir, &dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let files: Vec<_> = hits.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files, vec!["m/bad.rs"]);
    }
}
