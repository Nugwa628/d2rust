//! Tests for the client vitals sync: message layouts, the force rule, which message a
//! sync picks, gold and experience messages, and the potion predictions.
use super::*;

const SERVER_TSV: &str = include_str!("../../../../../../tables/sim/server-messages.tsv");

fn cur() -> Current {
    Current {
        life: 100,
        max_life: 100,
        mana: 50,
        stamina: 80,
        x: 5000,
        y: 4000,
        ..Current::default()
    }
}

/// A cache in step with [`cur`].
fn synced() -> SyncCache {
    let mut c = SyncCache::default();
    let r = sync(&mut c, &cur(), true);
    assert!(r.done);
    c
}

/// The `bits:` fields of a `server-messages.tsv` row: (size, [(name,
/// width)]).
fn bits_row(tsv: &str, id: &str) -> Option<(usize, Vec<(String, usize)>)> {
    let line = tsv.lines().find(|l| l.split('\t').next() == Some(id))?;
    let c: Vec<&str> = line.split('\t').collect();
    let size = c.get(2)?.parse().ok()?;
    let fields = c
        .get(3)?
        .strip_prefix("bits: ")?
        .split(' ')
        .map(|f| {
            let (n, w) = f.split_once(':')?;
            Some((n.to_string(), w.parse().ok()?))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((size, fields))
}

/// Mismatches of a builder against its row: each field, alone at all
/// ones, must set exactly its bits (the id at its own value).
fn check_layout(tsv: &str, id: &str, build: &dyn Fn(&str) -> Vec<u8>) -> Vec<String> {
    let Some((size, fields)) = bits_row(tsv, id) else {
        return vec![format!("{id}: no bits row")];
    };
    let mut out = Vec::new();
    let mut at = 0;
    for (name, w) in &fields {
        let mut want = vec![0u8; size];
        if name == "id" {
            want[0] = u8::from_str_radix(&id[2..], 16).unwrap();
        } else {
            want[0] = u8::from_str_radix(&id[2..], 16).unwrap();
            for b in at..at + w {
                want[b / 8] |= 1 << (b % 8);
            }
        }
        let got = build(name);
        if got != want {
            out.push(format!("{id} {name}: {got:02x?} vs {want:02x?}"));
        }
        at += w;
    }
    out
}

fn build_18(field: &str) -> Vec<u8> {
    let m = |f: &str| if f == field { u32::MAX } else { 0 };
    life_mana_update(
        m("life") as i32,
        m("mana") as i32,
        m("stamina") as i32,
        m("life_pred") as u8,
        m("mana_pred") as u8,
        m("x") as u16,
        m("y") as u16,
        m("dx") as u8,
        m("dy") as u8,
    )
    .to_vec()
}

fn build_95(field: &str) -> Vec<u8> {
    let m = |f: &str| if f == field { u32::MAX } else { 0 };
    life_mana_update2(
        m("life") as i32,
        m("mana") as i32,
        m("stamina") as i32,
        m("x") as u16,
        m("y") as u16,
        m("dx") as u8,
        m("dy") as u8,
    )
    .to_vec()
}

#[test]
fn layouts_match_server_tsv_bits() {
    assert_eq!(
        check_layout(SERVER_TSV, "0x18", &build_18),
        Vec::<String>::new()
    );
    assert_eq!(
        check_layout(SERVER_TSV, "0x95", &build_95),
        Vec::<String>::new()
    );
    // Each value is cut to its width.
    let m = life_mana_update(0x8001, 0, 0, 0x81, 0, 0, 0, 0, 0);
    assert_eq!(m, life_mana_update(1, 0, 0, 1, 0, 0, 0, 0, 0));
}

/// A changed width in the row is reported.
#[test]
fn layout_check_reports_perturbations() {
    let bad = SERVER_TSV.replace(
        "bits: id:8 life:15 mana:15 stamina:15 life_pred:7",
        "bits: id:8 life:14 mana:16 stamina:15 life_pred:7",
    );
    let r = check_layout(&bad, "0x18", &build_18);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r[0].starts_with("0x18 life:") && r[1].starts_with("0x18 mana:"));
    assert!(!check_layout("", "0x95", &build_95).is_empty());
}

#[test]
fn force_rule() {
    assert!(!force(9, true));
    assert!(!force(10, false));
    assert!(force(10, true));
    assert!(!force(19, false));
    assert!(force(20, false));
}

#[test]
fn small_changes_wait_unless_forced() {
    let mut c = synced();
    let before = c;
    // M ≤ 0: nothing, even forced.
    let r = sync(
        &mut c,
        &Current {
            max_life: 0,
            ..cur()
        },
        true,
    );
    assert_eq!((r.done, c), (false, before));
    // |Δlife| · 100 / M < 10: nothing.
    let r = sync(&mut c, &Current { life: 91, ..cur() }, false);
    assert_eq!((r, c), (Synced::default(), before));
    // 10 %: sent as 0x95.
    let r = sync(&mut c, &Current { life: 90, ..cur() }, false);
    assert!(r.done);
    assert_eq!(
        r.messages,
        [life_mana_update2(90, 50, 80, 5000, 4000, 0, 0).to_vec()]
    );
    // A drop to zero is not sent unforced; forced it is.
    let r = sync(&mut c, &Current { life: 0, ..cur() }, false);
    assert!(!r.done);
    let r = sync(&mut c, &Current { life: 0, ..cur() }, true);
    assert!(r.done);
    assert_eq!(c.life, 0);
}

#[test]
fn one_message_by_priority() {
    // Predictions differ → 0x18 (even with life and stamina changed).
    let mut c = synced();
    let now = Current {
        lp: 40,
        life: 70,
        stamina: 1,
        ..cur()
    };
    let r = sync(&mut c, &now, true);
    assert_eq!(
        r.messages,
        [life_mana_update(70, 50, 1, 40, 0, 5000, 4000, 0, 0).to_vec()]
    );
    assert_eq!((c.life, c.stamina, c.lp, c.quiet), (70, 1, 40, 0));
    // Mana alone → 0x95.
    let mut c = synced();
    let r = sync(&mut c, &Current { mana: 49, ..cur() }, true);
    assert_eq!(r.messages[0][0], 0x95);
    // Stamina alone → 0x96.
    let mut c = synced();
    let r = sync(
        &mut c,
        &Current {
            stamina: 79,
            dx: 0xFE,
            ..cur()
        },
        true,
    );
    assert_eq!(r.messages, [walk_verify(79, 5000, 4000, -2, 0).to_vec()]);
    assert_eq!(c.dx, 0xFE);
    // Nothing differs → no message, quiet + 1.
    let mut c = synced();
    let r = sync(&mut c, &cur(), true);
    assert!(r.done && r.messages.is_empty());
    assert_eq!(c.quiet, 1);
}

#[test]
fn position_resync_after_quiet_runs() {
    let mut c = synced();
    let moved = Current { x: 5002, ..cur() };
    for q in 1..=4 {
        let r = sync(&mut c, &moved, true);
        assert!(r.messages.is_empty(), "quiet {q}");
        assert_eq!(c.quiet, q);
    }
    // quiet 4 > 3 and |Δx| = 2 → 0x96, cache position updated.
    let r = sync(&mut c, &moved, true);
    assert_eq!(r.messages, [walk_verify(80, 5002, 4000, 0, 0).to_vec()]);
    assert_eq!((c.quiet, c.x), (0, 5002));
    // |Δ| = 1 never resyncs.
    let near = Current { y: 4001, ..moved };
    for _ in 0..8 {
        assert!(sync(&mut c, &near, true).messages.is_empty());
    }
}

#[test]
fn gold_then_experience_after_the_vitals() {
    let mut c = synced();
    let now = Current {
        mana: 10,
        gold: 30,
        exp: 1000,
        ..cur()
    };
    let r = sync(&mut c, &now, true);
    assert_eq!(r.messages.len(), 3);
    assert_eq!(r.messages[0][0], 0x95);
    assert_eq!(r.messages[1], [0x19, 30]);
    assert_eq!(r.messages[2], [0x1B, 0xE8, 0x03]);
    assert_eq!((c.gold, c.exp), (30, 1000));
    assert!(sync(&mut c, &now, true).messages.is_empty());
}

#[test]
fn experience_message_widths() {
    assert_eq!(exp_message(5, 5), None);
    assert_eq!(exp_message(0xFE, 0), Some(vec![0x1A, 0xFE]));
    assert_eq!(exp_message(0xFF, 0), Some(vec![0x1B, 0xFF, 0]));
    assert_eq!(exp_message(0xFFFE, 0), Some(vec![0x1B, 0xFE, 0xFF]));
    assert_eq!(exp_message(0xFFFF, 0), Some(vec![0x1C, 0xFF, 0xFF, 0, 0]));
    // A loss wraps: δ unsigned above 0xFFFE → the new total.
    assert_eq!(exp_message(10, 20), Some(vec![0x1C, 10, 0, 0, 0]));
}

#[test]
fn predictions() {
    // No list → 0.
    assert_eq!(life_prediction(None, 0, 0, 100), 0);
    assert_eq!(life_prediction(Some((256, 10)), 0, 0, 0), 0);
    // q = (256 · (110 − 100) + 25600) >> 8 = 110; v = 110 · 100 / 200 = 55.
    assert_eq!(life_prediction(Some((256, 110)), 100, 25600, 200), 55);
    // Low byte above 100 → 100; low byte of 300 is 44.
    assert_eq!(life_prediction(Some((0, 0)), 0, 101 << 8, 100), 100);
    assert_eq!(life_prediction(Some((0, 0)), 0, 300 << 8, 100), 44);
    // Mana: m = 25600 (100 · 256), ManaRegen 10 → q 250, i = 102;
    // stat 27 = 50: i = 153; + stat 26 2 = 155;
    // v = (20 · 155 + 12800) · 100 / 25600 = 62.
    let v = ManaInputs {
        max_mana: 25600,
        mana_regen: Some(10),
        regen_pct: 50,
        regen_add: 2,
        mana_total: 12800,
    };
    assert_eq!(mana_prediction(Some(120), 100, &v), 62);
    assert_eq!(mana_prediction(None, 100, &v), 0);
    assert_eq!(
        mana_prediction(
            Some(120),
            100,
            &ManaInputs {
                mana_regen: None,
                ..v
            }
        ),
        0
    );
    // ManaRegen 0 reads 7500: i = max(25600 / 7500, 1) = 3.
    let z = ManaInputs {
        mana_regen: Some(0),
        regen_pct: 0,
        regen_add: 0,
        ..v
    };
    assert_eq!(mana_prediction(Some(100), 100, &z), 50);
}
