//! Tests of the edge floors.

use super::super::{Dt1Facts, Fade, REC_HIDDEN};
use super::*;

fn ext(min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> DrawnExtents {
    let mut d = DrawnExtents::new();
    d.widen((min_x, min_y));
    d.widen((max_x, max_y));
    d
}

fn edge_record() -> TileRecord {
    TileRecord {
        tile: (0, 0),
        flags: 0,
        ty: 0,
        dt1: Dt1Facts::default(),
        fade: Fade::OPAQUE,
        logical: None,
    }
}

fn subtiles(v: &[EdgeFloor]) -> Vec<(i32, i32)> {
    v.iter().map(|f| f.subtile).collect()
}

#[test]
fn active_needs_draw_edges_open_mode_0_resolution_mode_2() {
    assert!(edges_active(true, 0, 2));
    assert!(!edges_active(false, 0, 2));
    assert!(!edges_active(true, 1, 2));
    assert!(!edges_active(true, 0, 1));
}

#[test]
fn extents_widen_by_min_and_max() {
    let mut d = DrawnExtents::new();
    assert_eq!(d.get(), None);
    d.widen((50, 60));
    d.widen((40, 70));
    d.widen((45, 65));
    assert_eq!(
        d.get(),
        Some(Extents {
            min_x: 40,
            max_x: 50,
            min_y: 60,
            max_y: 70
        })
    );
}

#[test]
fn snap_is_c_division() {
    assert_eq!(snap(12), 10);
    assert_eq!(snap(-3), 0);
    assert_eq!(snap(-7), -5);
    assert_eq!(snap(15), 15);
}

#[test]
fn strips_start_step_and_snap_per_table_row() {
    let e = ext(100, 200, 100, 200).get().unwrap();
    // px − min x = 12 < 30: start (95, 152 → 150), step (0, −5).
    assert_eq!(
        strip(Side::MinX, &e, (112, 152), false),
        Some([(95, 150), (95, 145), (95, 140)])
    );
    // max x − px = 12 < 25: start (205, 150), step (0, +5).
    assert_eq!(
        strip(Side::MaxX, &e, (188, 152), false),
        Some([(205, 150), (205, 155), (205, 160)])
    );
    // py − min y = 3 < 30: start (110, 95), step (−5, 0).
    assert_eq!(
        strip(Side::MinY, &e, (113, 103), false),
        Some([(110, 95), (105, 95), (100, 95)])
    );
    // max y − py = 1 < 25: start (110, 205), step (+5, 0).
    assert_eq!(
        strip(Side::MaxY, &e, (113, 199), false),
        Some([(110, 205), (115, 205), (120, 205)])
    );
    // Far from every edge: no strip.
    for side in Side::ALL {
        assert_eq!(strip(side, &e, (150, 150), false), None);
    }
}

#[test]
fn perspective_margins_33_and_23() {
    let e = ext(100, 200, 100, 200).get().unwrap();
    // px − min x = 31: not < 30, < 33.
    assert!(strip(Side::MinX, &e, (131, 150), false).is_none());
    assert!(strip(Side::MinX, &e, (131, 150), true).is_some());
    assert!(strip(Side::MinX, &e, (133, 150), true).is_none());
    // max x − px = 24: < 25, not < 23.
    assert!(strip(Side::MaxX, &e, (176, 150), false).is_some());
    assert!(strip(Side::MaxX, &e, (176, 150), true).is_none());
    assert!(strip(Side::MaxX, &e, (178, 150), true).is_some());
    // The same margins on y.
    assert!(strip(Side::MinY, &e, (150, 131), true).is_some());
    assert!(strip(Side::MaxY, &e, (150, 176), true).is_none());
}

#[test]
fn tests_run_in_table_order_on_widened_extents() {
    let mut d = ext(100, 200, 100, 200);
    let mut rec = edge_record();
    let out = edge_floors(&mut d, (110, 105), false, Some(&mut rec), |_| true).unwrap();
    // MinX strip at x 95 reaches y 95; the MinY strip then starts at
    // min y 95 − 5 = 90 (not 95).
    assert_eq!(
        subtiles(&out),
        vec![
            (95, 105),
            (95, 100),
            (95, 95),
            (110, 90),
            (105, 90),
            (100, 90)
        ]
    );
    assert_eq!(out[3].side, Side::MinY);
    assert_eq!((out[2].index, out[2].tile()), (2, (19, 19)));
    assert_eq!(
        d.get(),
        Some(Extents {
            min_x: 95,
            max_x: 200,
            min_y: 90,
            max_y: 200
        })
    );
    assert_eq!(rec.flags & REC_DRAWN, REC_DRAWN);
}

#[test]
fn undrawn_edge_floors_do_not_widen() {
    // View test fails for every floor: nothing drawn, extents unchanged,
    // so the MinY strip starts at 95.
    let mut d = ext(100, 200, 100, 200);
    let mut rec = edge_record();
    let mut seen = Vec::new();
    let out = edge_floors(&mut d, (110, 105), false, Some(&mut rec), |f| {
        seen.push(f.subtile);
        false
    })
    .unwrap();
    assert!(out.is_empty());
    assert_eq!(seen[3], (110, 95));
    assert_eq!(d, ext(100, 200, 100, 200));
    assert_eq!(rec.flags, 0);

    // Hidden record (flag 0x8) or orientation ≠ 0: no draw either.
    let mut rec = edge_record();
    rec.flags = REC_HIDDEN;
    let out = edge_floors(&mut d, (110, 105), false, Some(&mut rec), |_| true).unwrap();
    assert!(out.is_empty());
    let mut rec = edge_record();
    rec.dt1.orientation = 3;
    let out = edge_floors(&mut d, (110, 105), false, Some(&mut rec), |_| true).unwrap();
    assert!(out.is_empty());
}

#[test]
fn edge_record_is_open_question_2() {
    let mut d = ext(100, 200, 100, 200);
    // No strip test passes: nothing needs the record.
    assert_eq!(
        edge_floors(&mut d, (150, 150), false, None, |_| true),
        Ok(vec![])
    );
    assert_eq!(
        edge_floors(&mut d, (110, 150), false, None, |_| true),
        Err(EdgeError::NoEdgeRecord)
    );
    let mut empty = DrawnExtents::new();
    let mut rec = edge_record();
    assert_eq!(
        edge_floors(&mut empty, (110, 150), false, Some(&mut rec), |_| true),
        Err(EdgeError::NoExtents)
    );
}

#[test]
fn edge_keys_follow_the_last_room() {
    assert_eq!(
        edge_key(3, 4),
        OrderKey {
            pass: 3,
            major: 6,
            minor: 4
        }
    );
    // After room 2's layer-2 floors (major 2·2 + 1 = 5).
    assert!(edge_key(3, 0).major > 2 * 2 + 1);
}
