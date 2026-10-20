//! Consistency of the code with the machine tables and perturbation
//! tests of each check: `items/item-actions.tsv`,
//! `sim/server-messages.tsv` layouts, `sim/client-messages.tsv` sizes.

use crate::items::moves::deferred::{Cond, Test, To, ITEM_ACTIONS};
use crate::items::moves::handlers::HANDLED;
use crate::items::moves::layouts;

const ACTIONS_TSV: &str = include_str!("../../../../../../tables/items/item-actions.tsv");
const SERVER_TSV: &str = include_str!("../../../../../../tables/sim/server-messages.tsv");
const CLIENT_TSV: &str = include_str!("../../../../../../tables/sim/client-messages.tsv");

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

fn flags(s: &str) -> Option<u32> {
    s.split('|').map(hex).try_fold(0, |a, v| Some(a | v?))
}

/// Mismatches between `item-actions.tsv` and [`ITEM_ACTIONS`], one line
/// each: "row N column: tsv … code …".
fn check_actions(tsv: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = tsv.lines();
    let header = lines.next().unwrap_or_default();
    if header != "order\ttest\tflags\tto\tcondition\tmessage\taction\tsender\td2moo" {
        out.push(format!("header {header}"));
    }
    let rows: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if rows.len() != ITEM_ACTIONS.len() {
        out.push(format!(
            "row count {} vs {}",
            rows.len(),
            ITEM_ACTIONS.len()
        ));
    }
    for (line, r) in rows.iter().zip(ITEM_ACTIONS.iter()) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 9 {
            out.push(format!("row {}: {} columns", r.order, c.len()));
            continue;
        }
        let mut cmp = |col: &str, ok: bool, tsv: &str| {
            if !ok {
                out.push(format!("row {} {col}: tsv {tsv}", r.order));
            }
        };
        cmp("order", c[0].parse() == Ok(r.order), c[0]);
        let test = match r.test {
            Test::Cmd => "cmd",
            Test::Item => "item",
        };
        cmp("test", c[1] == test, c[1]);
        cmp("flags", flags(c[2]) == Some(r.flags), c[2]);
        let to = match r.to {
            To::Owner => "owner",
            To::All => "all",
            To::OwnerOrMode1 => "owner|mode1",
        };
        cmp("to", c[3] == to, c[3]);
        let cond = match r.cond {
            Cond::None => "-",
            Cond::NotBroken => "not 0x100",
            Cond::UpdateStats => {
                "mode in 0..2; not 0x100/0x200; owner with item flag 0x40000 sends nothing"
            }
        };
        cmp("condition", c[4] == cond, c[4]);
        cmp("message", hex(c[5]) == Some(u32::from(r.message)), c[5]);
        cmp("action", hex(c[6]) == Some(r.action), c[6]);
        cmp("sender", hex(c[7]) == Some(r.sender), c[7]);
        cmp("d2moo", c[8] == r.d2moo, c[8]);
    }
    out
}

#[test]
fn item_actions_match_tsv() {
    assert_eq!(check_actions(ACTIONS_TSV), Vec::<String>::new());
}

#[test]
fn item_actions_check_catches_perturbations() {
    let p = ACTIONS_TSV.replace("0x2|0x80\towner", "0x2\towner");
    assert_eq!(check_actions(&p), vec!["row 3 flags: tsv 0x2".to_string()]);
    let p = ACTIONS_TSV.replace("0x10\tall\t-\t0x9D\t0x08", "0x10\towner\t-\t0x9D\t0x08");
    assert_eq!(check_actions(&p), vec!["row 7 to: tsv owner".to_string()]);
    let p = ACTIONS_TSV.replace("\tnot 0x100\t", "\t-\t");
    assert_eq!(
        check_actions(&p),
        vec!["row 19 condition: tsv -".to_string()]
    );
    let p = ACTIONS_TSV.replace("0x9C\t0x0F", "0x9C\t0x0E");
    assert_eq!(
        check_actions(&p),
        vec!["row 12 action: tsv 0x0E".to_string()]
    );
    // Swapping two rows reports both rows.
    let mut lines: Vec<&str> = ACTIONS_TSV.lines().collect();
    lines.swap(5, 6);
    assert!(check_actions(&lines.join("\n")).len() > 2);
}

/// One built message per id, with the value each layout field must hold.
type Sample = (u32, Vec<u8>, Vec<(&'static str, u32)>);

fn samples() -> Vec<Sample> {
    let bits = [0xAA, 0xBB, 0xCC];
    vec![
        (0x19, layouts::gold(120, 100).unwrap(), vec![("delta", 20)]),
        (
            0x1D,
            layouts::gold(70, 400).unwrap(),
            vec![("stat", 0x0E), ("value", 70)],
        ),
        (
            0x1E,
            layouts::gold(400, 100).unwrap(),
            vec![("stat", 0x0E), ("value", 400)],
        ),
        (
            0x1F,
            layouts::gold(70000, 100).unwrap(),
            vec![("stat", 0x0E), ("value", 70000)],
        ),
        (
            0x3F,
            layouts::use_stackable(0xFF, 0x0102_0304, 0xFFFF),
            vec![("code", 0xFF), ("item", 0x0102_0304), ("arg", 0xFFFF)],
        ),
        (
            0x42,
            layouts::clear_cursor(4, 0x0A0B_0C0D),
            vec![("type", 4), ("unit", 0x0A0B_0C0D)],
        ),
        (
            0x47,
            layouts::relator1(0, 0x1122_3344),
            vec![("type", 0), ("u8@2", 0), ("unit", 0x1122_3344)],
        ),
        (
            0x48,
            layouts::relator2(1, 9, 0x1122_3344),
            vec![("type", 1), ("arg", 9), ("unit", 0x1122_3344)],
        ),
        (
            0x7D,
            layouts::item_state(0, 0x1111_2222, 0x3333_4444, 0x200, 0x200),
            vec![
                ("type", 0),
                ("unit", 0x1111_2222),
                ("item", 0x3333_4444),
                ("flag", 0x200),
                ("state", 0x200),
            ],
        ),
        (
            0x9C,
            layouts::item_world(0x0E, 2, 0x5555_6666, &bits).unwrap(),
            vec![
                ("action", 0x0E),
                ("size", 11),
                ("category", 2),
                ("item", 0x5555_6666),
            ],
        ),
        (
            0x9D,
            layouts::item_owned(0x05, 6, 0x5555_6666, 0, 0x0000_0001, &bits).unwrap(),
            vec![
                ("action", 5),
                ("size", 16),
                ("category", 6),
                ("item", 0x5555_6666),
                ("owner_type", 0),
                ("owner", 1),
            ],
        ),
    ]
}

fn read(b: &[u8], ty: &str, off: usize) -> Option<u32> {
    let n = match ty {
        "u8" => 1,
        "u16" => 2,
        "u32" => 4,
        _ => return None,
    };
    let s = b.get(off..off + n)?;
    Some(s.iter().rev().fold(0, |a, &x| a << 8 | u32::from(x)))
}

/// Mismatches between the built messages and the `server-messages.tsv`
/// layouts and sizes.
fn check_layouts(tsv: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (id, msg, want) in samples() {
        let key = format!("0x{id:02X}\t");
        let Some(line) = tsv.lines().find(|l| l.starts_with(&key)) else {
            out.push(format!("0x{id:02X}: no row"));
            continue;
        };
        let c: Vec<&str> = line.split('\t').collect();
        if msg[0] as u32 != id {
            out.push(format!("0x{id:02X}: id byte {}", msg[0]));
        }
        match c[2].parse::<usize>() {
            Ok(n) if n != msg.len() => {
                out.push(format!("0x{id:02X} size: tsv {n}, built {}", msg.len()))
            }
            Ok(_) => {}
            Err(_) => {
                // Variable size: "u8@2;min=3" — the size byte holds the length.
                if c[2] != "u8@2;min=3" || msg[2] as usize != msg.len() {
                    out.push(format!("0x{id:02X} size: tsv {}", c[2]));
                }
            }
        }
        let mut seen = Vec::new();
        for tok in c[3].split(' ') {
            let (name, rest) = match tok.split_once(':') {
                Some((n, r)) => (n.to_string(), r),
                None if tok.starts_with("data@") => ("data".to_string(), tok),
                None => (tok.to_string(), tok),
            };
            let Some((ty, off)) = rest.split_once('@') else {
                out.push(format!("0x{id:02X}: token {tok}"));
                continue;
            };
            let Ok(off) = off.parse::<usize>() else {
                out.push(format!("0x{id:02X}: token {tok}"));
                continue;
            };
            if name == "data" {
                if msg.get(off..) != Some(&[0xAA, 0xBB, 0xCC][..]) {
                    out.push(format!("0x{id:02X} data@{off}"));
                }
                continue;
            }
            let Some(&(_, v)) = want.iter().find(|(n, _)| *n == name) else {
                out.push(format!("0x{id:02X}: field {name} not expected"));
                continue;
            };
            seen.push(name.clone());
            if read(&msg, ty, off) != Some(v) {
                out.push(format!("0x{id:02X} {name}: {tok}"));
            }
        }
        for (n, _) in &want {
            if !seen.iter().any(|s| s == n) {
                out.push(format!("0x{id:02X}: field {n} missing from tsv"));
            }
        }
    }
    out
}

#[test]
fn layouts_match_server_tsv() {
    assert_eq!(check_layouts(SERVER_TSV), Vec::<String>::new());
}

#[test]
fn layout_check_catches_perturbations() {
    let p = SERVER_TSV.replace("item:u32@6 flag", "item:u32@7 flag");
    assert_eq!(check_layouts(&p), vec!["0x7D item: item:u32@7".to_string()]);
    let p = SERVER_TSV.replace("ClearCursor\t6", "ClearCursor\t7");
    assert_eq!(
        check_layouts(&p),
        vec!["0x42 size: tsv 7, built 6".to_string()]
    );
    let p = SERVER_TSV.replace(
        "owner_type:u8@8 owner:u32@9",
        "owner_type:u8@8 owner:u32@10",
    );
    assert_eq!(
        check_layouts(&p),
        vec!["0x9D owner: owner:u32@10".to_string()]
    );
}

/// Mismatches between [`HANDLED`] and the `client-messages.tsv` rows.
fn check_handled(tsv: &str) -> Vec<String> {
    let mut out = Vec::new();
    for &(id, size) in &HANDLED {
        let key = format!("0x{id:02X}\t");
        let Some(line) = tsv.lines().find(|l| l.starts_with(&key)) else {
            out.push(format!("0x{id:02X}: no row"));
            continue;
        };
        let c: Vec<&str> = line.split('\t').collect();
        if c[3] != format!("=={size}") || c[2] != size.to_string() {
            out.push(format!("0x{id:02X} size: tsv {} / {}", c[2], c[3]));
        }
        if c[6] != "handler" || c[7] != "alive" {
            out.push(format!("0x{id:02X} kind/gate: tsv {} / {}", c[6], c[7]));
        }
    }
    out
}

#[test]
fn handled_ids_match_client_tsv() {
    assert_eq!(check_handled(CLIENT_TSV), Vec::<String>::new());
    // 0x4C is the cube's, not handled here.
    assert!(!HANDLED.iter().any(|&(i, _)| i == 0x4C));
}

#[test]
fn handled_check_catches_perturbations() {
    let p = CLIENT_TSV.replace("DropItem\t5\t==5", "DropItem\t5\t==6");
    assert_eq!(
        check_handled(&p),
        vec!["0x17 size: tsv 5 / ==6".to_string()]
    );
}
