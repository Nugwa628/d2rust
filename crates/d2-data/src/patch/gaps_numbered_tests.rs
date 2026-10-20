//! Gap tests for rule units no claim named before. Synthetic data only:
//! the `tests` fixture and invented headers over the real field lists.

use super::tests::fixture;
use super::*;

fn v(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn one_layer(data: &mut PatchData, body: &str) -> Vec<Finding> {
    let text = format!("d2patch 1\n{body}\n");
    let (layer, f) = parse_layer("a.d2patch", text.as_bytes(), 1);
    assert!(f.is_empty(), "{f:?}");
    apply_stack(data, &[layer], "s.d2stack")
}

fn finding(code: Code, file: &str, pos: usize, line: usize, col: usize) -> Finding {
    Finding::new(code, file, line, col).at_pos(pos)
}

fn with_table(mut f: Finding, table: &str) -> Finding {
    f.table = Some(table.into());
    f
}

/// The report: the normative fields filled by a real conflict, the
/// severity, the human form with bracketed values, and the order S; P
/// (stack position, line, column); B (table); A and N (stack position,
/// line, column, code; N04 last); C (table, row, column).
#[test]
fn report_fields_form_and_order() {
    // Fields, from a real A06 (old value differs from the base cell).
    let mut d = fixture();
    let layers: Vec<Layer> = [
        ("a.d2patch", "d2patch 1\ntable items\nset axe lvl 2 -> 4\n"),
        ("b.d2patch", "d2patch 1\ntable items\nset axe lvl 1 -> 4\n"),
    ]
    .iter()
    .enumerate()
    .map(|(k, (p, t))| parse_layer(p, t.as_bytes(), k + 1).0)
    .collect();
    let f = apply_stack(&mut d, &layers, "s.d2stack");
    let a06 = f.iter().find(|f| f.code == Code::A06).expect("A06");
    assert_eq!((a06.file.as_str(), a06.line, a06.col), ("a.d2patch", 3, 13));
    assert_eq!(a06.table.as_deref(), Some("items"));
    assert_eq!(a06.row, Some((0, v("axe"))));
    assert_eq!(a06.column, Some(v("lvl")));
    assert_eq!(a06.expected, Some(v("2")));
    assert_eq!(a06.found, Some(v("1")));
    assert_eq!(a06.writer.as_deref(), Some("base line 2"));
    assert!(a06.code.is_error());
    // N04 names the layers not applied (b), at line 0, col 0, in the stack file.
    let n04 = f.iter().find(|f| f.code == Code::N04).expect("N04");
    assert_eq!((n04.file.as_str(), n04.line, n04.col), ("s.d2stack", 0, 0));
    assert!(n04.detail.contains("b.d2patch"), "{}", n04.detail);
    assert!(!n04.code.is_error());

    // Severity: N notes, every other class errors.
    for c in [Code::N01, Code::N02, Code::N03, Code::N04] {
        assert!(!c.is_error(), "{c:?}");
    }
    for c in [
        Code::S01,
        Code::P01,
        Code::B01,
        Code::A01,
        Code::C01,
        Code::D01,
    ] {
        assert!(c.is_error(), "{c:?}");
    }

    // Human form; the canonical column; values and keys bracketed.
    let mut h = Finding::new(Code::A06, "core/10-items.d2patch", 5, 16);
    h.table = Some("weapons".into());
    h.row = Some((3, v("Big Club")));
    h.column = Some(v("mindam@2"));
    h.expected = Some(v("3"));
    h.found = Some(v(""));
    h.writer = Some("core/05.d2patch:7".into());
    h.related = vec!["core/05.d2patch:8".into()];
    h.detail = "cell differs".into();
    assert_eq!(
        h.to_string(),
        "error[A06] core/10-items.d2patch:5:16: weapons [Big Club] (#3) mindam@2: cell differs; \
         expected [3]; found []; writer core/05.d2patch:7; related: core/05.d2patch:8"
    );
    let mut n = Finding::new(Code::N01, "b.d2patch", 4, 16);
    n.table = Some("weapons".into());
    n.row = Some((0, v("hax")));
    n.column = Some(v("mindam"));
    n.detail = "replaces [4] written by a.d2patch:5".into();
    assert_eq!(
        n.to_string(),
        "note[N01] b.d2patch:4:16: weapons hax (#0) mindam: replaces [4] written by a.d2patch:5"
    );
    // No table, row or column: no location part; wording from the code.
    assert_eq!(
        Finding::new(Code::S01, "s.d2stack", 1, 1).to_string(),
        "error[S01] s.d2stack:1:1: line 1 is not `d2stack 1`"
    );

    // Order.
    let mut c_items_1 = with_table(finding(Code::C02, "base:x:items", 0, 9, 1), "items");
    c_items_1.row = Some((1, v("clb")));
    c_items_1.column = Some(v("lvl"));
    let mut c_items_0b = with_table(finding(Code::C02, "base:x:items", 0, 2, 4), "items");
    c_items_0b.row = Some((0, v("axe")));
    c_items_0b.column = Some(v("name"));
    let mut c_items_0a = with_table(finding(Code::C02, "base:x:items", 0, 2, 2), "items");
    c_items_0a.row = Some((0, v("axe")));
    c_items_0a.column = Some(v("dam"));
    let c_armor = with_table(finding(Code::C03, "patched set", 0, 0, 0), "armor");
    let want = vec![
        finding(Code::S05, "s.d2stack", 0, 3, 7),
        finding(Code::S06, "s.d2stack", 0, 4, 7),
        finding(Code::P10, "a.d2patch", 1, 3, 5),
        finding(Code::P01, "a.d2patch", 1, 3, 9),
        finding(Code::P02, "a.d2patch", 1, 5, 1),
        finding(Code::P01, "b.d2patch", 2, 1, 1),
        with_table(finding(Code::B03, "base:x:aaa", 0, 0, 0), "aaa"),
        with_table(finding(Code::B02, "base:x:mmm", 0, 7, 0), "mmm"),
        with_table(finding(Code::B01, "zzz", 0, 0, 0), "zzz"),
        finding(Code::A06, "a.d2patch", 1, 4, 1),
        finding(Code::N01, "a.d2patch", 1, 4, 1),
        finding(Code::A03, "a.d2patch", 1, 4, 9),
        finding(Code::N02, "a.d2patch", 1, 6, 1),
        finding(Code::A12, "b.d2patch", 2, 2, 1),
        finding(Code::N04, "s.d2stack", 0, 0, 0),
        c_armor,
        c_items_0a,
        c_items_0b,
        c_items_1,
    ];
    let mut got = want.clone();
    got.reverse();
    got.swap(0, 7);
    got.swap(3, 12);
    sort_report(&mut got);
    let show = |f: &[Finding]| {
        f.iter()
            .map(|f| format!("{:?} {}:{}:{} {:?}", f.code, f.file, f.line, f.col, f.table))
            .collect::<Vec<_>>()
    };
    assert_eq!(show(&got), show(&want));
}

/// Real field lists over invented headers with a repeated column: the
/// leftmost copy binds, so a `set` on `name@2` is N02 and one on `name@1`
/// is not (`armor` `mindam`/`maxdam`); a repeated column no list names
/// (`automap` `Type2`, `chartemplate` `SkillName`) is N02 on every copy;
/// an empty header name is addressable as `[]`. No claim: the edge-case
/// item also states header facts of the LoD tables (which tables repeat which
/// columns, `weapons` column 18), checked by the ignored game test
/// `edge_duplicate_columns_in_1_14d_headers` in `tests/patch_gaps_game.rs`.
#[test]
fn duplicate_columns_leftmost_binds() {
    for (table, dup, listed) in [
        ("armor", "mindam", true),
        ("armor", "maxdam", true),
        ("automap", "Type2", false),
        ("chartemplate", "SkillName", false),
    ] {
        let r = rules()
            .into_iter()
            .find(|r| r.name == table)
            .unwrap_or_else(|| panic!("{table}"));
        let mut header: Vec<Vec<u8>> = Vec::new();
        for list in &r.lists {
            for c in list {
                if !header.contains(c) {
                    header.push(c.clone());
                }
            }
        }
        assert_eq!(
            header.iter().any(|h| h == dup.as_bytes()),
            listed,
            "{table} {dup}"
        );
        if !listed {
            header.push(v(dup));
        }
        let first = header.iter().position(|h| h == dup.as_bytes()).unwrap();
        header.push(v(dup));
        header.push(Vec::new());
        let second = header.len() - 2;
        let mut cells = vec![v("1"); header.len()];
        cells[first] = v("4");
        cells[second] = v("5");
        let row = Row {
            origin: Origin::Base(2),
            writers: vec![Writer::Base; cells.len()],
            cells,
        };
        let t = PatchTable::new(&r, "fixture", header, vec![row]).unwrap();
        assert_eq!(t.bound[first], listed, "{table} {dup}@1");
        assert!(!t.bound[second], "{table} {dup}@2 does not bind");
        let sel = format!("#0 {}", String::from_utf8_lossy(&canonical_token(t.key(0))));
        let mut d = PatchData::from_tables(vec![t]);
        let f = one_layer(
            &mut d,
            &format!(
                "table {table}\nset {sel} {dup}@2 5 -> 6\nset {sel} {dup}@1 4 -> 7\nset {sel} [] 1 -> 2"
            ),
        );
        let got: Vec<(Code, usize)> = f.iter().map(|f| (f.code, f.line)).collect();
        let want: &[(Code, usize)] = if listed {
            &[(Code::N02, 3), (Code::N02, 5)]
        } else {
            &[(Code::N02, 3), (Code::N02, 4), (Code::N02, 5)]
        };
        assert_eq!(got, want, "{table} {dup}");
        let cells = &d.tables[0].rows[0].cells;
        assert_eq!(
            (&cells[first], &cells[second], cells.last().unwrap()),
            (&v("7"), &v("6"), &v("2"))
        );
    }
}
