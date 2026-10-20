//! The per-skill bodies of `bodies` (`functions.tsv` notes and status
//! `spec'd-here`), run through the start core on the `tests` fake.

use super::super::bodies::{self, DO_BODIES, PURE_START, START_BODIES};
use super::*;

/// The `functions.tsv` note that states each body here, by (kind, index).
const BODY_NOTES: &[(&str, u16, &str)] = &[("srvst", 18, "\"return 1\"")];

/// Every row of `functions.tsv` whose notes state a body (the notes'
/// "… body is …" phrase) must have that body in `bodies`, on a filled slot; every other
/// slot must have none. Returns the mismatches.
fn bodies_mismatches(tsv: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in tsv.lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        let (Some(kind), Some(Ok(index))) = (c.first(), c.get(1).map(|i| i.parse::<u16>())) else {
            continue;
        };
        let has = match *kind {
            "srvst" => bodies::start(index).is_some(),
            "srvdo" => false,
            _ => continue,
        };
        let notes = c.get(6).copied().unwrap_or("");
        // A note states a body with the "… body is …" phrase; the code's body must
        // be the one stated.
        let noted = notes.contains("1.14d body is ");
        let same = BODY_NOTES
            .iter()
            .any(|&(k, i, n)| k == *kind && i == index && notes.contains(n));
        if has != noted || (has && !same) {
            out.push(format!("{kind} {index}"));
            continue;
        }
        if has && c.get(2) == Some(&"null") {
            out.push(format!("{kind} {index}"));
        }
    }
    out
}

/// Unreferenced slots whose body is a helper here (called directly, not
/// through the table): srvdo 142 = [`bodies::blade_pulse`].
const HELPER_SLOTS: &[(&str, u16)] = &[("srvdo", 142)];

/// Every `spec'd-here` row must name its section in a `body:` note (one
/// of the five bodies sets)
/// and be in [`START_BODIES`] / [`DO_BODIES`]; every slot there must be
/// such a row. Returns the mismatches.
fn specd_mismatches(tsv: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for l in tsv.lines().skip(1) {
        let c: Vec<&str> = l.split('\t').collect();
        let (Some(kind), Some(Ok(index))) = (c.first(), c.get(1).map(|i| i.parse::<u16>())) else {
            continue;
        };
        let listed = match *kind {
            "srvst" => START_BODIES.contains(&index),
            "srvdo" => DO_BODIES.contains(&index),
            _ => continue,
        };
        let specd = c.get(4) == Some(&"spec'd-here");
        let noted = c.get(6).is_some_and(|n| {
            n.contains("body: bodies.md §")
                || n.contains("body: bodies-2.md §")
                || n.contains("body: bodies-2b.md §")
                || n.contains("body: bodies-3.md §")
                || n.contains("body: bodies-4.md §")
        });
        // An `unreferenced` slot whose note names a body is a helper the
        // bodies call directly (srvdo 142, the Blade Shield pulse): no
        // slot body, but the helper must exist.
        let helper = !specd && noted && c.get(4) == Some(&"unreferenced");
        if helper {
            if !HELPER_SLOTS.contains(&(*kind, index)) {
                out.push(format!("{kind} {index}"));
            }
        } else if listed != specd || specd != noted {
            out.push(format!("{kind} {index}"));
        }
        if specd {
            seen.push((*kind, index));
        }
    }
    for &i in START_BODIES {
        if !seen.contains(&("srvst", i)) {
            out.push(format!("srvst {i} missing"));
        }
    }
    for &i in DO_BODIES {
        if !seen.contains(&("srvdo", i)) {
            out.push(format!("srvdo {i} missing"));
        }
    }
    out
}

#[test]
fn bodies_match_tsv_notes() {
    assert_eq!(bodies_mismatches(FUNCTIONS_TSV), Vec::<String>::new());
    assert_eq!(specd_mismatches(FUNCTIONS_TSV), Vec::<String>::new());
    assert_eq!(PURE_START, [18]);
    //– (16),– (29),
    // (85), (83): every filled slot but the
    // three unreferenced ones.
    assert_eq!(START_BODIES.len() + DO_BODIES.len(), 213);
    assert_eq!((START_BODIES.len(), DO_BODIES.len()), (64, 149));
    for &i in PURE_START.iter().chain(START_BODIES) {
        assert!(table::lookup(Kind::Start, i).is_some());
    }
    for &i in DO_BODIES {
        assert!(table::lookup(Kind::Do, i).is_some());
    }
    for i in 0..table::START_SLOTS {
        assert_eq!(bodies::start(i).is_some(), PURE_START.contains(&i));
    }
}

#[test]
fn bodies_check_reports_perturbations() {
    // The note of srvst 18 no longer states the body.
    let bad = FUNCTIONS_TSV.replacen("1.14d body is \"return 1\"", "1.14d body is unknown", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(bodies_mismatches(&bad), ["srvst 18"]);
    // A slot that states a body the code lacks (no `mapped` row is left:
    // an unreferenced one).
    let bad = FUNCTIONS_TSV.replacen(
        "SrvDo138_Unused\tunreferenced\t(none)\t",
        "SrvDo138_Unused\tunreferenced\t(none)\t1.14d body is \"return 1\"",
        1,
    );
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(bodies_mismatches(&bad), ["srvdo 138"]);
    // A spec'd-here row demoted to mapped: the body list disagrees.
    let bad = FUNCTIONS_TSV.replacen("SrvSt02_Kick\tspec'd-here", "SrvSt02_Kick\tmapped", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvst 2", "srvst 2 missing"]);
    // A spec'd-here row without its section note.
    let bad = FUNCTIONS_TSV.replacen("body: bodies.md §4.4", "body: elsewhere", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 30"]);
    // A batch 3 row (holds those sections; the TSV
    // names them) without its section note.
    let bad = FUNCTIONS_TSV.replacen("body: bodies-2.md §8.14", "body: elsewhere", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 54"]);
    // A helper note on an unreferenced slot the code has no helper for.
    let bad = FUNCTIONS_TSV.replacen(
        "SrvDo138_Unused\tunreferenced\t(none)\t",
        "SrvDo138_Unused\tunreferenced\t(none)\tbody: bodies-2.md §2.26",
        1,
    );
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 138"]);
    // A second-set row demoted to mapped.
    let bad = FUNCTIONS_TSV.replacen("SrvDo150_Smite\tspec'd-here", "SrvDo150_Smite\tmapped", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 150", "srvdo 150 missing"]);
    // An unreferenced row promoted: no body for it.
    let bad = FUNCTIONS_TSV.replacen(
        "SrvDo053_Unused\tunreferenced\t(none)\t",
        "SrvDo053_Unused\tspec'd-here\t(none)\tbody: bodies-4.md §9",
        1,
    );
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 53"]);
    // A third-set row without its section note.
    let bad = FUNCTIONS_TSV.replacen("body: bodies-3.md §5.18", "body: elsewhere", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 92"]);
    // A fourth-set row demoted to mapped.
    let bad = FUNCTIONS_TSV.replacen("SrvDo151_Unused\tspec'd-here", "SrvDo151_Unused\tmapped", 1);
    assert_ne!(bad, FUNCTIONS_TSV);
    assert_eq!(specd_mismatches(&bad), ["srvdo 151", "srvdo 151 missing"]);
}

#[test]
fn attract_start_returns_one_and_charges_at_start() {
    let t = tables(
        3,
        &[
            (1, &|r: &mut Skills| {
                r.srvstfunc = 18;
                mana(r, 4, 1, 8, 0);
            }),
            (2, &|r: &mut Skills| {
                r.srvstfunc = 18;
                r.periodic = true;
            }),
        ],
    );
    let mut f = F::new();
    // The seam would refuse: the body answers instead and the seam is
    // never called.
    f.srvst_ret = 0;
    let p = caster(&mut f, 1, 1, 2000);
    f.take_log();
    assert_eq!(start(&mut f, &t, p), 1);
    assert!(f.take_log().is_empty());
    // r ≠ 0, no usemanaondo: (4 + 0) << 8 charged at start.
    assert_eq!(f.get(p, stat::MANA), 2000 - 1024);
    // r ≠ 0 and periodic: type-8 timers with arg 0 deleted after it.
    let q = caster(&mut f, 2, 1, 0);
    f.take_log();
    assert_eq!(start(&mut f, &t, q), 1);
    assert_eq!(f.take_log(), ["delete 1 8 0"]);
}

#[test]
fn attract_start_with_usemanaondo_charges_nothing() {
    let t = tables(
        2,
        &[(1, &|r: &mut Skills| {
            r.srvstfunc = 18;
            r.usemanaondo = true;
            mana(r, 4, 1, 8, 0);
        })],
    );
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 2000);
    assert_eq!(start(&mut f, &t, p), 1);
    assert_eq!(f.get(p, stat::MANA), 2000);
}
