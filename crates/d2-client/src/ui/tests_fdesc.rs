use super::skill_desc::*;

#[test]
fn table_bounds() {
    assert!(!descdam_live(0) && descdam_live(1) && descdam_live(24) && !descdam_live(25));
    assert!(!descatt_live(0) && descatt_live(1) && descatt_live(5) && !descatt_live(6));
}

#[test]
fn scale_truncates_toward_zero() {
    assert_eq!(scale128(100, 64), 50);
    assert_eq!(scale128(-3, 64), -1);
    assert_eq!(scale128(-1, 1), 0);
    assert_eq!(scale128(7, 128), 7);
}

#[test]
fn range_text_vectors() {
    assert_eq!(range_text(5, 5), "5-6");
    assert_eq!(range_text(8, 3), "8-9");
    assert_eq!(range_text(3, 8), "3-8");
    assert_eq!(range_text(9999, 12345), "9999-12K");
    assert_eq!(range_text(12345, 23456), "12K-23K");
}

#[test]
fn kick_prints_v_to_v_plus_1() {
    let v = kick_value(3, 8, 4);
    assert_eq!(v, 7);
    assert_eq!(range_text(v, v), "7-8");
    assert_eq!(kick_value(3, 10, 0), 12);
}

#[test]
fn entry8_vector() {
    assert_eq!(entry8(0, 0, 2560, 5120), (250, 500));
}

#[test]
fn entry9_vector() {
    assert_eq!(entry9(4, 8, 4, 0, 0, (0, 0)), (18, 37));
}

#[test]
fn entry10_vector() {
    assert_eq!(entry10(10, 20, 50, 100, 100), (16, 31));
    assert_eq!(entry10(10, 20, -200, 0, 0), (-9 + 10, -18 + 20));
}

#[test]
fn attack_rating_vector() {
    assert_eq!(ar_value(100, 20), 120);
    assert_eq!(ar_color(false, false, false), 0);
    assert_eq!(ar_color(true, false, false), 3);
    assert_eq!(ar_color(true, true, true), 1);
}

#[test]
fn etype_colors() {
    assert_eq!(etype_color(2), Some(9));
    assert_eq!(etype_color(6), None);
}
