//! Coverage claims for the pipeline stop rule and duplicate columns.

use super::tests::fixture;
use super::*;

#[test]
fn duplicate_column_leftmost_binds() {
    let d = fixture();
    let items = d.table("items").unwrap();
    // `dam` appears twice (columns 3 and 4): the leftmost is `dam@1`
    // and binds; the second is `dam@2` and stays unbound.
    assert_eq!(items.canonical(3), b"dam@1");
    assert_eq!(items.canonical(4), b"dam@2");
    assert!(items.bound[3]);
    assert!(!items.bound[4]);
}

#[test]
fn first_failing_layer_stops_the_stack() {
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
    assert!(f.iter().any(|f| f.code == Code::A06));
    // Layer b is never applied; the stack file notes it (N04).
    let n04 = f.iter().find(|f| f.code == Code::N04).expect("N04");
    assert!(n04.detail.contains("b.d2patch"));
}
