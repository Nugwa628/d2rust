//! The synthetic vectors: our own invented fixture tables, no game data.

use super::*;
use crate::schema::ExtValue;

fn v(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn rules(
    name: &str,
    key: Option<&str>,
    kind: KeyKind,
    unique: bool,
    fixed: bool,
    scope: &str,
    fields: &[&str],
) -> TableRules {
    TableRules {
        name: name.into(),
        txt_name: format!("{name}.txt"),
        key_field: key.map(v),
        kind,
        unique,
        fixed,
        scope: scope.into(),
        lists: vec![fields.iter().map(|f| v(f)).collect()],
    }
}

fn table(r: TableRules, header: &[&str], rows: &[&str]) -> PatchTable {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let cells: Vec<Vec<u8>> = line.split(';').map(v).collect();
            Row {
                origin: Origin::Base(i + 2),
                writers: vec![Writer::Base; cells.len()],
                cells,
            }
        })
        .collect();
    PatchTable::new(&r, "fixture", header.iter().map(|h| v(h)).collect(), rows).unwrap()
}

/// The fixture: `items`, `gear` (same scope), `recipes`.
pub(super) fn fixture() -> PatchData {
    let items = table(
        rules(
            "items",
            Some("code"),
            KeyKind::Code,
            true,
            false,
            "items.code",
            &["name", "code", "lvl", "dam"],
        ),
        &["name", "code", "lvl", "dam", "dam", "*note"],
        &[
            "Axe;axe;1;3;0;",
            "Club;clb;1;2;0;old",
            "Axe;ax2;5;7;0;",
            "Big Club;clb;9;8;1;",
        ],
    );
    let gear = table(
        rules(
            "gear",
            Some("code"),
            KeyKind::Code,
            true,
            true,
            "items.code",
            &["name", "code"],
        ),
        &["name", "code"],
        &["Cap;cap", "Belt;blt"],
    );
    let recipes = table(
        rules(
            "recipes",
            None,
            KeyKind::Exact,
            false,
            false,
            "recipes",
            &["description", "enabled", "output"],
        ),
        &["description", "enabled", "output"],
        &["A;1;\"hp1,qty=3\"", "B;1;x", "C;0;y"],
    );
    PatchData::from_tables(vec![items, gear, recipes])
}

fn layer(path: &str, text: &str, pos: usize) -> Layer {
    let (l, f) = parse_layer(path, text.as_bytes(), pos);
    assert!(f.is_empty(), "{path}: {:?}", f);
    l
}

/// Applies one-statement-per-`/` layers; statements after `table items`
/// unless the text starts with `table`.
fn run(layers: &[(&str, &str)]) -> (PatchData, Vec<Finding>) {
    let mut data = fixture();
    let parsed: Vec<Layer> = layers
        .iter()
        .enumerate()
        .map(|(k, (path, body))| {
            let body = body.replace(" / ", "\n");
            let text = if body.starts_with("table ") {
                format!("d2patch 1\n{body}\n")
            } else {
                format!("d2patch 1\ntable items\n{body}\n")
            };
            layer(path, &text, k + 1)
        })
        .collect();
    let f = apply_stack(&mut data, &parsed, "s.d2stack");
    (data, f)
}

fn one(body: &str) -> (PatchData, Vec<Finding>) {
    run(&[("a.d2patch", body)])
}

/// `code file:line:col` of each finding.
fn codes(f: &[Finding]) -> Vec<String> {
    f.iter()
        .map(|f| format!("{:?} {}:{}:{}", f.code, f.file, f.line, f.col))
        .collect()
}

fn expect(body: &str, want: &[&str]) {
    let (_, f) = one(body);
    assert_eq!(codes(&f), want, "{body}");
}

fn row(d: &PatchData, t: &str, i: usize) -> String {
    let t = d.table(t).unwrap();
    t.rows[i]
        .cells
        .iter()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(";")
}

fn digest16(d: &PatchData, t: &str) -> String {
    d.table(t).unwrap().digest()[..16].to_owned()
}

#[test]
fn fixture_digests_and_pins() {
    let d = fixture();
    assert_eq!(digest16(&d, "items"), "2c6f65f2ec858aad");
    assert_eq!(&d.digest()[..16], "29a76856ffd1a6a9");
    let items = d.table("items").unwrap();
    assert_eq!(items.pin(0), "sha:84ec3f726bd38615");
    assert_eq!(items.pin(3), "sha:bddc5549c005d720");
    assert_eq!(items.canonical(3), b"dam@1");
    assert_eq!(items.canonical(4), b"dam@2");
    assert_eq!(items.bound, [true, true, true, true, false, false]);
}

/// Parses a layer and returns its P findings as `code line:col`.
fn parse_codes(text: &str) -> Vec<String> {
    let (_, f) = parse_layer("a.d2patch", text.as_bytes(), 1);
    f.iter()
        .map(|f| format!("{:?} {}:{}", f.code, f.line, f.col))
        .collect()
}

fn p3(stmt: &str) -> Vec<String> {
    parse_codes(&format!("d2patch 1\ntable items\n{stmt}\n"))
}

#[test]
fn parse_vectors() {
    let (l, f) = parse_layer(
        "a.d2patch",
        b"d2patch 1\ntable items\nset axe lvl [1] -> [4 ]\n",
        1,
    );
    assert!(f.is_empty());
    match &l.statements[1].stmt {
        Stmt::Set { new, old, .. } => {
            assert_eq!(new.text, b"4 ");
            assert_eq!(old.text, b"1");
        }
        s => panic!("{s:?}"),
    }
    assert_eq!(p3("set axe lvl[1] -> 4"), ["P10 3:12"]);
    assert_eq!(p3("set #01 axe lvl 1 -> 4"), ["P07 3:5"]);
    assert_eq!(p3("set axe [lvl 1 -> 4"), ["P06 3:9"]);
    assert_eq!(p3("set axe lvl 1 -> 4 5"), ["P05 3:20"]);
    assert_eq!(p3("set axe lvl 1 ->"), ["P05 3:17"]);
    assert_eq!(p3("add #4 spr like #3"), ["P05 3:19"]);
    assert_eq!(p3("Set axe lvl 1 -> 4"), ["P04 3:1"]);
    assert_eq!(p3("add #4 spr like axe sha:0123"), ["P11 3:21"]);
    assert_eq!(p3("set axe lvl 1 -> 4\t"), ["P01 3:19"]);
    let long = format!("set axe lvl 1 -> {}", "x".repeat(1025));
    assert_eq!(p3(&long), ["P12 3:18"]);
    let ok = format!("set axe lvl 1 -> {}", "x".repeat(1024));
    assert!(p3(&ok).is_empty());
    assert_eq!(parse_codes("d2patch 2\n"), ["P03 1:9"]);
    assert_eq!(parse_codes("D2PATCH 1\n"), ["P02 1:1"]);
    assert_eq!(parse_codes("d2patch 1\nset axe lvl 1 -> 4"), ["P09 2:1"]);
}

#[test]
fn parse_more() {
    // `[#5]` is a key; a bare `#5` key in `add` is a key too.
    let (l, f) = parse_layer("a", b"d2patch 1\ntable items\nset [#5] lvl 1 -> 2\n", 1);
    assert!(f.is_empty());
    assert!(matches!(&l.statements[1].stmt, Stmt::Set { sel: Sel::Key(k), .. } if k.text == b"#5"));
    assert_eq!(p3("remove axe"), ["P04 3:1"]);
    assert_eq!(p3("[set] axe lvl 1 -> 2"), ["P04 3:1"]);
    assert_eq!(p3("set axe lvl 1 [->] 2"), ["P05 3:15"]);
    assert_eq!(p3("set #4294967296 x lvl 1 -> 2"), ["P07 3:5"]);
    assert!(p3("set #4294967295 x lvl 1 -> 2").is_empty());
    assert_eq!(p3("add [#4] x"), ["P05 3:5"]);
    assert_eq!(p3("add #4 x lik axe"), ["P05 3:10"]);
    assert_eq!(p3("set axe ] 1 -> 2"), ["P10 3:9"]);
    assert_eq!(p3("set axe [] 1 -> 2"), Vec::<String>::new());
    assert_eq!(p3("set axe [a]b 1 -> 2"), ["P10 3:12"]);
    assert_eq!(p3("set axe [a[b] 1 -> 2"), ["P06 3:9"]);
    assert_eq!(parse_codes("d2patch 1\ntable Items\n"), ["P08 2:7"]);
    assert_eq!(parse_codes("d2patch 1\ntable items x\n"), ["P05 2:13"]);
    // Blank lines and comments only get P01; CR LF = LF; lone CR is P01.
    assert!(parse_codes("d2patch 1\r\n\r\n  # c\r\ntable items\r\n").is_empty());
    assert_eq!(parse_codes("d2patch 1\n# a\rb\n"), ["P01 2:4"]);
    assert_eq!(parse_codes("d2patch 1\ntable items\r"), ["P01 2:12"]);
    // A failed line 1 leaves only P01 elsewhere.
    assert_eq!(
        parse_codes("d2patch 3\nbogus\nx\x01\n"),
        ["P03 1:9", "P01 3:2"]
    );
    // One error per line: P01 before token errors.
    assert_eq!(p3("set ] \x7f"), ["P01 3:7"]);
    let line = format!("set axe lvl 1 -> 2 {}", " ".repeat(4080));
    assert_eq!(p3(&line), ["P12 3:4097"]);
}

#[test]
fn apply_vectors() {
    // V1
    let (d, f) = one("set axe lvl 1 -> 4");
    assert!(f.is_empty());
    assert_eq!(row(&d, "items", 0), "Axe;axe;4;3;0;");
    let items = d.table("items").unwrap();
    assert_eq!(
        items.rows[0].writers[2],
        Writer::Set(Loc { layer: 0, line: 3 })
    );
    assert_eq!(digest16(&d, "items"), "13c4b643f616f070");
    // V2
    let (_, f) = one("set clb lvl 1 -> 4");
    assert_eq!(codes(&f), ["A04 a.d2patch:3:5"]);
    assert_eq!(f[0].related, ["items 1", "items 3"]);
    expect("set #1 axe lvl 1 -> 4", &["A05 a.d2patch:3:5"]);
    expect("set AXE lvl 1 -> 4", &["A03 a.d2patch:3:5"]);
    // V3
    let (_, f) = one("set axe lvl 2 -> 4");
    assert_eq!(codes(&f), ["A06 a.d2patch:3:13"]);
    assert_eq!(f[0].found.as_deref(), Some(&b"1"[..]));
    assert_eq!(f[0].writer.as_deref(), Some("base line 2"));
    expect("set axe lvl 1 -> 1", &["A07 a.d2patch:3:18"]);
    expect("set axe name Axe -> Expansion", &["A11 a.d2patch:3:21"]);
    // V4
    expect("set axe dam 3 -> 4", &["A02 a.d2patch:3:9"]);
    expect("set axe dam@1 3 -> 4", &[]);
    expect("set axe dam@2 0 -> 1", &["N02 a.d2patch:3:9"]);
    expect("set axe lvl@1 1 -> 4", &["A02 a.d2patch:3:9"]);
    expect("set axe dam@3 3 -> 4", &["A02 a.d2patch:3:9"]);
    // V5
    expect(
        "set axe lvl 1 -> 4 / set axe lvl 4 -> 5",
        &["A08 a.d2patch:4:9"],
    );
    // V6
    let (d, f) = one("add #4 spr / set spr name [] -> Spear / set #4 spr lvl [] -> 2");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "items", 4), "Spear;spr;2;;;");
    assert_eq!(digest16(&d, "items"), "d530958cb6927ea0");
    // V7
    expect("add #5 spr", &["A09 a.d2patch:3:5"]);
    expect(
        "set #4 spr lvl [] -> 2 / add #4 spr",
        &["A03 a.d2patch:3:5"],
    );
    // V8
    expect("add #4 [axe ]", &["A12 a.d2patch:3:8"]);
    expect("add #4 []", &["A12 a.d2patch:3:8"]);
    let (_, f) = one("add #4 cap");
    assert_eq!(codes(&f), ["A12 a.d2patch:3:8"]);
    assert_eq!(f[0].related, ["gear 0", "items 4"]);
    // V9
    let (_, f) = one("set #2 ax2 code ax2 -> axe");
    assert_eq!(codes(&f), ["A12 a.d2patch:3:24"]);
    assert_eq!(f[0].related, ["items 0", "items 2"]);
    let (d, f) = one("set #0 axe code axe -> ax2 / set #2 ax2 code ax2 -> axe");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "items", 0), "Axe;ax2;1;3;0;");
    // V10
    expect("set ax2 code ax2 -> ax3", &["A14 a.d2patch:3:5"]);
    expect(
        "set #2 ax2 code ax2 -> ax3 / set ax3 lvl 5 -> 6",
        &["A14 a.d2patch:4:5"],
    );
    // V11
    let (d, f) = one("add #4 spr like axe sha:84ec3f726bd38615 / set spr name Axe -> Spear");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "items", 4), "Spear;spr;1;3;0;");
    assert_eq!(digest16(&d, "items"), "a60483e64d8a3910");
    expect(
        "add #4 spr like axe sha:0000000000000000",
        &["A13 a.d2patch:3:21"],
    );
    // V12
    expect(
        "add #4 spr like axe / set axe lvl 1 -> 2",
        &["A15 a.d2patch:4:5"],
    );
    let (d, f) = one("set axe lvl 1 -> 2 / add #4 spr like axe");
    assert!(f.is_empty());
    assert_eq!(row(&d, "items", 4), "Axe;spr;2;3;0;");
    // V13
    let (_, f) = one("table gear / add #2 x / set x name [] -> X");
    assert_eq!(codes(&f), ["A10 a.d2patch:3:1", "N03 a.d2patch:4:1"]);
    assert_eq!(f[1].detail, "1 statements skipped");
    // V14
    expect(
        "table recipes / add #3 A / set A enabled 1 -> 0",
        &["A04 a.d2patch:4:5"],
    );
    expect("table recipes / set A enabled 1 -> 0 / add #3 A", &[]);
    // V17
    expect(
        "set axe lvl 9 -> 2 / set ax2 lvl 5 -> 6 / set clb lvl 1 -> 2",
        &["A06 a.d2patch:3:13", "A04 a.d2patch:5:5"],
    );
    // V18
    let (d, f) =
        one("table nosuch / set a b 1 -> 2 / set c d 1 -> 2 / table items / set axe lvl 1 -> 2");
    assert_eq!(codes(&f), ["A01 a.d2patch:2:7", "N03 a.d2patch:3:1"]);
    assert_eq!(f[1].detail, "2 statements skipped");
    assert_eq!(row(&d, "items", 0), "Axe;axe;2;3;0;");
    // V19
    expect(
        "set #4 clb code clb -> x / set axe lvl 1 -> 2",
        &["A03 a.d2patch:3:5", "N03 a.d2patch:4:1"],
    );
    // V20
    expect(
        "add #4 x1 / set #4 x1 code x1 -> x2",
        &["A08 a.d2patch:4:11"],
    );
    // `check`
    expect("check axe lvl 1", &[]);
    expect("check axe lvl 2", &["A06 a.d2patch:3:15"]);
}

#[test]
fn layered_vectors() {
    // V15
    let (_, f) = run(&[
        ("one.d2patch", "set axe lvl 1 -> 4"),
        ("two.d2patch", "set axe lvl 1 -> 6"),
    ]);
    assert_eq!(codes(&f), ["A06 two.d2patch:3:13"]);
    assert_eq!(f[0].writer.as_deref(), Some("one.d2patch:3"));
    let (d, f) = run(&[
        ("one.d2patch", "set axe lvl 1 -> 4"),
        ("two.d2patch", "set axe lvl 4 -> 6"),
    ]);
    assert_eq!(codes(&f), ["N01 two.d2patch:3:13"]);
    assert_eq!(row(&d, "items", 0), "Axe;axe;6;3;0;");
    // V16
    let (_, f) = run(&[
        (
            "a.d2patch",
            "add #5 spr / set spr lvl [] -> 2 / set axe lvl 1 -> 2",
        ),
        ("b.d2patch", "set axe lvl 1 -> 3"),
    ]);
    assert_eq!(
        codes(&f),
        [
            "A09 a.d2patch:3:5",
            "N03 a.d2patch:4:1",
            "N04 s.d2stack:0:0"
        ]
    );
    assert_eq!(f[1].detail, "2 statements skipped");
    assert!(f[2].detail.contains("b.d2patch"));
}

#[test]
fn stack_vector() {
    let stack = "d2stack 1\nlayer one.d2patch\nlayer two.d2patch\n";
    let one = "d2patch 1\ntable items\nset axe lvl 1 -> 4\nset #1 clb lvl 1 -> 7\n";
    let two = "d2patch 1\ntable items\nset axe lvl 4 -> 5\nadd #4 spr like axe sha:73706f637e0f52e5\n\
               set spr name Axe -> Spear\n\ntable recipes\nset A output \"hp1,qty=3\" -> \"hp1,qty=5\"\n";
    let (layers, f) = load_stack("s.d2stack", stack.as_bytes(), &mut |p| match p {
        "one.d2patch" => Some(one.as_bytes().to_vec()),
        "two.d2patch" => Some(two.as_bytes().to_vec()),
        _ => None,
    });
    assert!(f.is_empty(), "{f:?}");
    let mut d = fixture();
    let f = apply_stack(&mut d, &layers, "s.d2stack");
    assert_eq!(codes(&f), ["N01 two.d2patch:3:13"]);
    assert_eq!(
        f[0].to_string(),
        "note[N01] two.d2patch:3:13: items axe (#0) lvl: replaces [4] written by one.d2patch:3; writer one.d2patch:3"
    );
    assert_eq!(row(&d, "items", 0), "Axe;axe;5;3;0;");
    assert_eq!(row(&d, "items", 1), "Club;clb;7;2;0;old");
    assert_eq!(row(&d, "items", 4), "Spear;spr;5;3;0;");
    assert_eq!(
        d.table("recipes").unwrap().rows[0].cells[2],
        b"\"hp1,qty=5\""
    );
    assert_eq!(digest16(&d, "items"), "a564579cb18890b5");
    assert_eq!(&d.digest()[..16], "22b98c96c8a9960d");

    let stack_codes = |s: &str| {
        let (_, f) = parse_stack("s", s.as_bytes());
        f.iter()
            .map(|f| format!("{:?} {}:{}", f.code, f.line, f.col))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        stack_codes("d2stack 1\nlayer a.d2patch\nlayer a.d2patch\n"),
        ["S05 3:7"]
    );
    assert_eq!(stack_codes("d2stack 1\nlayer ../a.d2patch\n"), ["S04 2:7"]);
    assert_eq!(stack_codes("d2stack 2\n"), ["S02 1:9"]);
    assert_eq!(stack_codes("layer a.d2patch\n"), ["S01 1:1"]);
    assert_eq!(stack_codes("d2stack 1\nlayers a.d2patch\n"), ["S03 2:1"]);
    assert_eq!(
        stack_codes("d2stack 1\nlayer A.d2patch\nlayer a.txt\nlayer a//b.d2patch\n"),
        ["S04 2:7", "S04 3:7", "S04 4:7"]
    );
    let (_, f) = load_stack("s", b"d2stack 1\nlayer x.d2patch\n", &mut |_| None);
    assert_eq!(codes(&f), ["S06 s:2:7"]);
    let many: String = (0..1025).map(|i| format!("layer l{i}.d2patch\n")).collect();
    assert_eq!(stack_codes(&format!("d2stack 1\n{many}")), ["S07 1026:7"]);
}

fn edited(rows: &[&str]) -> Vec<u8> {
    let mut out = b"name\tcode\tlvl\tdam\tdam\t*note\r\n".to_vec();
    for r in rows {
        out.extend_from_slice(r.replace(';', "\t").as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn diff_items(rows: &[&str]) -> Result<Vec<u8>, DiffError> {
    let d = fixture();
    let e = edited(rows);
    diff_tables(&[(d.table("items").unwrap(), &e)], false)
}

#[test]
fn diff_vector() {
    let rows = [
        "Axe;axe;4;3;0;",
        "Club;clb;1;2;0;",
        "Axe;ax3;5;7;0;",
        "Big Club;clb;9;8;1;",
        "Spear;spr;4;3;0;",
        "Mace;mce;3;;;",
    ];
    let out = diff_items(&rows).unwrap();
    let want = "d2patch 1\n\ntable items\nset axe lvl 1 -> 4\nset #1 clb *note old -> []\n\
                set #2 ax2 code ax2 -> ax3\nadd #4 spr like axe sha:da7b748ddf3353a9\n\
                set #4 spr name Axe -> Spear\nadd #5 mce\nset #5 mce name [] -> Mace\n\
                set #5 mce lvl [] -> 3\n";
    assert_eq!(String::from_utf8_lossy(&out), want);
    // It reproduces the edit.
    let mut d = fixture();
    let (l, f) = parse_layer("d.d2patch", &out, 1);
    assert!(f.is_empty());
    let f = apply_stack(&mut d, &[l], "s");
    assert_eq!(codes(&f), ["N02 d.d2patch:5:12"]);
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(row(&d, "items", i), *r);
    }
    assert_eq!(digest16(&d, "items"), "bea8bb33aac58fd2");

    let base = [
        "Axe;axe;1;3;0;",
        "Club;clb;1;2;0;old",
        "Axe;ax2;5;7;0;",
        "Big Club;clb;9;8;1;",
    ];
    assert_eq!(diff_items(&base).unwrap(), b"d2patch 1\n");
    let code = |r: Result<Vec<u8>, DiffError>| {
        let e = r.unwrap_err();
        (e.code, e.row.map(|r| r.0))
    };
    assert_eq!(code(diff_items(&base[..3])), (Code::D01, None));
    let d = fixture();
    let renamed = edited(&base)
        .splitn(2, |&b| b == b'*')
        .next()
        .unwrap()
        .to_vec();
    let mut e = renamed;
    e.extend_from_slice(b"*nota\r\n");
    for r in base {
        e.extend_from_slice(r.replace(';', "\t").as_bytes());
        e.extend_from_slice(b"\r\n");
    }
    let r = diff_tables(&[(d.table("items").unwrap(), &e)], false);
    assert_eq!(code(r), (Code::D02, None));
    let mut bad = base;
    bad[0] = "A]x;axe;1;3;0;";
    assert_eq!(code(diff_items(&bad)), (Code::D03, Some(0)));
    let inserted = [base[0], "New;new;1;1;1;", base[1], base[2], base[3]];
    assert_eq!(code(diff_items(&inserted)), (Code::D05, Some(2)));
    let mut dup = base.to_vec();
    dup.push("Spear;axe ;1;;;");
    assert_eq!(code(diff_items(&dup)), (Code::D06, Some(0)));
    assert_eq!(
        code(diff_tables(&[(d.table("items").unwrap(), b"x")], false)),
        (Code::D04, None)
    );
}

/// A small deterministic generator for the property tests.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

#[test]
fn diff_round_trip_property() {
    let pool = [
        "", "Axe", "axe", "ax2", "clb", "1", "3", "x y", "spr", "mce", "0",
    ];
    let mut rng = Lcg(7);
    let base = fixture();
    let items = base.table("items").unwrap();
    let mut checked = 0;
    for case in 0..600 {
        let mut rows: Vec<Vec<String>> = items
            .rows
            .iter()
            .map(|r| {
                r.cells
                    .iter()
                    .map(|c| String::from_utf8_lossy(c).into_owned())
                    .collect()
            })
            .collect();
        for _ in 0..rng.next(4) {
            let r = rng.next(rows.len());
            let c = rng.next(6);
            rows[r][c] = pool[rng.next(pool.len())].to_owned();
        }
        for _ in 0..rng.next(3) {
            rows.push(
                (0..6)
                    .map(|_| pool[rng.next(pool.len())].to_owned())
                    .collect(),
            );
        }
        let joined: Vec<String> = rows.iter().map(|r| r.join(";")).collect();
        let refs: Vec<&str> = joined.iter().map(String::as_str).collect();
        let e = edited(&refs);
        let Ok(out) = diff_tables(&[(items, &e)], case % 2 == 0) else {
            continue;
        };
        let mut d = fixture();
        let (l, f) = parse_layer("d.d2patch", &out, 1);
        assert!(f.is_empty(), "{f:?}\n{}", String::from_utf8_lossy(&out));
        let f = apply_stack(&mut d, &[l], "s");
        assert!(!has_errors(&f), "{f:?}\n{}", String::from_utf8_lossy(&out));
        for (i, r) in refs.iter().enumerate() {
            assert_eq!(row(&d, "items", i), *r, "{}", String::from_utf8_lossy(&out));
        }
        assert_eq!(d.table("items").unwrap().rows.len(), refs.len());
        checked += 1;
    }
    assert!(checked >= 200, "only {checked} diffs round-tripped");
}

#[test]
fn order_independence_property() {
    let stmts = [
        "set axe lvl 1 -> 4",
        "set #1 clb name Club -> Cudgel",
        "add #4 spr like axe",
        "set #2 ax2 dam@1 7 -> 8",
        "set #3 clb code clb -> mce",
    ];
    // Every permutation of a valid layer gives identical cells or fails
    // with one of A03, A08, A09, A14, A15.
    let mut reference: Option<String> = None;
    let mut valid = 0;
    let mut perm: Vec<usize> = (0..stmts.len()).collect();
    let mut all = Vec::new();
    permutations(&mut perm, 0, &mut all);
    for p in all {
        let body: Vec<&str> = p.iter().map(|&i| stmts[i]).collect();
        let (d, f) = one(&body.join(" / "));
        if has_errors(&f) {
            for x in f.iter().filter(|f| f.code.is_error()) {
                assert!(
                    matches!(
                        x.code,
                        Code::A03 | Code::A08 | Code::A09 | Code::A14 | Code::A15
                    ),
                    "{body:?}: {x}"
                );
            }
            continue;
        }
        valid += 1;
        let dg = d.digest();
        match &reference {
            None => reference = Some(dg),
            Some(r) => assert_eq!(&dg, r, "{body:?}"),
        }
    }
    assert!(valid >= 10, "{valid} valid permutations");
}

fn permutations(v: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == v.len() {
        out.push(v.clone());
        return;
    }
    for i in k..v.len() {
        v.swap(k, i);
        permutations(v, k + 1, out);
        v.swap(k, i);
    }
}

#[test]
fn parser_never_panics() {
    let alphabet = b"d2patch 1table set check add like -> # [ ] @ sha: axe lvl\n\r\t\x00x0";
    let mut rng = Lcg(11);
    for _ in 0..3000 {
        let n = rng.next(60);
        let mut bytes = b"d2patch 1\ntable items\n".to_vec();
        bytes.extend((0..n).map(|_| alphabet[rng.next(alphabet.len())]));
        let (l, _) = parse_layer("r", &bytes, 1);
        let mut d = fixture();
        apply_stack(&mut d, &[l], "s");
        let _ = parse_stack("s", &bytes);
    }
}

#[test]
fn render_identity() {
    let d = fixture();
    let items = d.table("items").unwrap();
    let r = items.render();
    let t = PatchTable::from_base(&items.rules, "fixture", &r).unwrap();
    assert_eq!(t.render(), r);
    assert_eq!(t.header, items.header);
    // Expansion lines are dropped by `read`.
    let mut with = b"name\tcode\r\nExpansion\t\r\n".to_vec();
    with.extend_from_slice(b"Cap\tcap\r\n");
    let gear = d.table("gear").unwrap();
    let t = PatchTable::from_base(&gear.rules, "fixture", &with).unwrap();
    assert_eq!(t.render(), b"name\tcode\r\nCap\tcap\r\n");
}

#[test]
fn schema_rules() {
    let r = rules_from_schema(crate::schema::schema());
    assert_eq!(r.len(), 85);
    let get = |n: &str| r.iter().find(|t| t.name == n).unwrap();
    let w = get("weapons");
    assert_eq!(
        (w.kind, w.unique, w.fixed, w.scope.as_str()),
        (KeyKind::Code, true, false, "items.code")
    );
    assert_eq!(get("armor").scope, "items.code");
    assert_eq!(get("misc").scope, "items.code");
    let m = get("monseq");
    assert_eq!((m.kind, m.unique), (KeyKind::Name, false));
    let tc = get("treasureclassex");
    assert_eq!(
        (tc.key_field.clone(), tc.kind, tc.unique),
        (None, KeyKind::Name, true)
    );
    let l = get("levels");
    assert_eq!(
        (l.lists.len(), l.kind, l.unique),
        (2, KeyKind::Exact, false)
    );
    assert!(get("inventory").fixed && get("belts").fixed);
    assert_eq!(get("skills").lists.len(), 2);
    assert_eq!(get("pettype").key_field.as_deref(), Some(&b"pet type"[..]));
}

#[test]
fn versioning_and_determinism() {
    // Version 1 is read; a newer one is P03 / S02; unknown statements are
    // never ignored (P04).
    assert!(parse_codes("d2patch 1\n").is_empty());
    assert_eq!(parse_codes("d2patch 2\n"), ["P03 1:9"]);
    assert_eq!(p3("remove axe"), ["P04 3:1"]);
    let (_, f) = parse_stack("s", b"d2stack 2\n");
    assert_eq!(f[0].code, Code::S02);
    // Applying twice, and with the statements permuted, gives identical
    // cells and digests (order independence of a valid layer).
    let stmts = [
        "set axe lvl 1 -> 4",
        "set #1 clb lvl 1 -> 7",
        "set #2 ax2 lvl 5 -> 6",
    ];
    let digest = |order: &[usize]| {
        let body = order
            .iter()
            .map(|&i| stmts[i])
            .collect::<Vec<_>>()
            .join(" / ");
        let (d, f) = one(&body);
        (d.digest(), d.table("items").unwrap().render(), codes(&f))
    };
    let a = digest(&[0, 1, 2]);
    assert_eq!(a, digest(&[0, 1, 2]));
    assert_eq!(a, digest(&[2, 0, 1]));
    assert_eq!(a, digest(&[1, 2, 0]));
    // Failing layers fail under every order.
    let bad = |first: &str, second: &str| one(&format!("{first} / {second}")).1.len();
    assert_eq!(
        bad("set axe lvl 1 -> 4", "set axe lvl 1 -> 5"),
        bad("set axe lvl 1 -> 5", "set axe lvl 1 -> 4")
    );
    assert!(bad("set axe lvl 1 -> 4", "set axe lvl 1 -> 5") > 0);
}

#[test]
fn addcol_parse_vectors() {
    assert!(p3("addcol bonus int 5").is_empty());
    assert!(p3("addcol bonus text").is_empty());
    assert!(p3("addcol [my col] text [a b]").is_empty());
    assert_eq!(p3("addcol bonus"), ["P05 3:13"]);
    assert_eq!(p3("addcol bonus float"), ["P05 3:14"]);
    assert_eq!(p3("addcol bonus [int]"), ["P05 3:14"]);
    assert_eq!(p3("addcol bonus int 5 6"), ["P05 3:20"]);
    assert_eq!(p3("addcol a@1 int"), ["P05 3:8"]);
    assert_eq!(p3("addcol [] text"), ["P05 3:8"]);
    assert_eq!(p3("[addcol] bonus int"), ["P04 3:1"]);
    assert_eq!(parse_codes("d2patch 1\naddcol bonus int\n"), ["P09 2:1"]);
    // `addcol` is part of version 1.
    assert_eq!(VERSION, 1);
}

#[test]
fn addcol_apply_vectors() {
    // Every row gets the default; the column is unbound and typed.
    let (d, f) = one("addcol bonus int 5 / addcol [my note] text");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "items", 0), "Axe;axe;1;3;0;;5;");
    let items = d.table("items").unwrap();
    assert_eq!(
        items.bound,
        [true, true, true, true, false, false, false, false]
    );
    assert_eq!(items.ext.len(), 2);
    assert_eq!((items.ext[0].column, items.ext[0].ty), (6, ExtType::Int));
    let ext = d.ext_store();
    assert_eq!(ext.ext("items", 0, "bonus"), Some(ExtValue::Int(5)));
    assert_eq!(ext.ext("items", 3, "BONUS"), Some(ExtValue::Int(5)));
    assert_eq!(
        ext.ext("items", 0, "my note"),
        Some(ExtValue::Text(Vec::new()))
    );
    assert_eq!(ext.ext("items", 4, "bonus"), None);
    assert_eq!(ext.ext("gear", 0, "bonus"), None);
    assert!(fixture().ext_store().is_empty());

    // A16: the header (any case) and earlier extension columns.
    expect("addcol lvl int", &["A16 a.d2patch:3:8"]);
    expect("addcol LVL int", &["A16 a.d2patch:3:8"]);
    expect("addcol *NOTE text", &["A16 a.d2patch:3:8"]);
    expect(
        "addcol bonus int / addcol Bonus text",
        &["A16 a.d2patch:4:8"],
    );
    // A16: a field name a list would bind, even when the base has no
    // such column.
    let extra = table(
        rules(
            "extra",
            None,
            KeyKind::Exact,
            false,
            false,
            "extra",
            &["a", "hidden"],
        ),
        &["a"],
        &["1", "2"],
    );
    let mut data = PatchData::from_tables(vec![extra]);
    let l = layer(
        "a.d2patch",
        "d2patch 1\ntable extra\naddcol Hidden int\n",
        1,
    );
    let f = apply_stack(&mut data, &[l], "s.d2stack");
    assert_eq!(codes(&f), ["A16 a.d2patch:3:8"]);
    assert_eq!(data.table("extra").unwrap().header, [v("a")]);

    // A17: `int` defaults and `set` values.
    expect("addcol bonus int x5", &["A17 a.d2patch:3:18"]);
    expect("addcol bonus int 2147483648", &["A17 a.d2patch:3:18"]);
    expect("addcol bonus int -", &["A17 a.d2patch:3:18"]);
    expect("addcol bonus text x5", &[]);
    expect("addcol bonus int -2147483648", &[]);
    expect(
        "addcol bonus int 5 / set axe bonus 5 -> nine",
        &["A17 a.d2patch:4:20"],
    );
    // A failed `addcol` is structural: later statements on the table skip.
    expect(
        "addcol lvl int / set axe lvl 1 -> 4",
        &["A16 a.d2patch:3:8", "N03 a.d2patch:4:1"],
    );

    // `set` and `check` on an extension column: no N02; an empty cell
    // reads as the default.
    let (d, f) = one(
        "addcol bonus int 5 / set axe bonus 5 -> -9 / set #1 clb bonus 5 -> [] / check ax2 bonus 5",
    );
    assert!(f.is_empty(), "{f:?}");
    let ext = d.ext_store();
    assert_eq!(ext.ext("items", 0, "bonus"), Some(ExtValue::Int(-9)));
    assert_eq!(ext.ext("items", 1, "bonus"), Some(ExtValue::Int(5)));

    // `add` without `like` starts at the defaults; with `like` it copies.
    let (d, f) =
        one("addcol bonus int 5 / set axe bonus 5 -> 9 / add #4 spr / add #5 hax like axe");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "items", 4), ";spr;;;;;5");
    assert_eq!(row(&d, "items", 5), "Axe;hax;1;3;0;;9");

    // Fixed tables take extension columns (the row count is unchanged).
    let (d, f) = one("table gear / addcol bonus int 1");
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(row(&d, "gear", 1), "Belt;blt;1");

    // A layer without `addcol` leaves the header and the store as they were.
    let (d, _) = one("set axe lvl 1 -> 4");
    assert_eq!(d.table("items").unwrap().header.len(), 6);
    assert!(d.ext_store().is_empty());
}

/// `stradd` / `strset`: parsed outside table zones, kept in stack order
/// with their place, then applied over the string tables (A18 key exists,
/// A19 no such key).
#[test]
fn string_statement_vectors() {
    assert!(parse_codes("d2patch 1\nstradd k1 [Hello there]\nstrset k2 x\n").is_empty());
    assert!(p3("stradd k1 text").is_empty());
    assert_eq!(p3("stradd k1"), ["P05 3:10"]);
    assert_eq!(p3("strset"), ["P05 3:7"]);
    assert_eq!(p3("stradd [] text"), ["P05 3:8"]);
    assert_eq!(p3("strset k1 a b"), ["P05 3:13"]);
    assert_eq!(p3("[stradd] k1 a"), ["P04 3:1"]);

    let (d, f) = run(&[
        ("a.d2patch", "stradd k1 one / strset k1 [two words]"),
        ("b.d2patch", "strset k1 three"),
    ]);
    assert!(f.is_empty(), "{f:?}");
    let got: Vec<_> = d
        .strings
        .iter()
        .map(|e| (e.path.as_str(), e.pos, e.line, e.col))
        .collect();
    assert_eq!(
        got,
        [
            ("a.d2patch", 1, 3, 8),
            ("a.d2patch", 1, 4, 8),
            ("b.d2patch", 2, 3, 8)
        ]
    );
    assert_eq!(
        d.strings[1].edit,
        StringEdit::Set {
            key: v("k1"),
            text: v("two words")
        }
    );
    // The tables themselves are untouched by string statements.
    assert_eq!(d.tables, fixture().tables);

    let mut s = StringTables::default();
    assert!(apply_strings(&mut s, &d.strings).is_empty());
    assert_eq!(s.id(b"k1"), crate::strings::MOD_BASE);
    assert_eq!(
        s.by_index(crate::strings::MOD_BASE).unwrap(),
        "three".encode_utf16().collect::<Vec<_>>()
    );

    let (d, _) = one("stradd k1 x / strset nokey y / stradd k1 z");
    let f = apply_strings(&mut StringTables::default(), &d.strings);
    assert_eq!(codes(&f), ["A19 a.d2patch:4:8", "A18 a.d2patch:5:8"]);
    assert!(has_errors(&f));
}
