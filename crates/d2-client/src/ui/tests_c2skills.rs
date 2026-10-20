use super::skill_desc::*;
use super::skill_desc_more::*;

#[test]
fn range_font_choice() {
    assert_eq!(range_font(10, 0, 100, 5), (false, 5));
    assert_eq!(range_font(70, 0, 100, 5), (true, 4));
}

#[test]
fn two_line() {
    assert_eq!(range_text_two_line(7, 7), "7");
    assert_eq!(two_line_top_offset(0), -6);
    assert_eq!(two_line_top_offset(7), -7);
    assert_eq!(two_line_plan((0, 0), (1, 2)), TwoLine::BAlone);
    assert_eq!(two_line_plan((1, 2), (0, 0)), TwoLine::AAlone);
    assert_eq!(two_line_plan((1, 2), (3, 4)), TwoLine::Both);
}

#[test]
fn weapon_phys_base_scaling() {
    assert_eq!(weapon_phys_base(10, 20, 0, 64), (5, 10));
    assert_eq!(weapon_phys_base(0, 0, 128, 0), (1, 2));
    assert_eq!(weapon_phys_base(10, 20, 0, 0), (0, 0));
}

#[test]
fn weapon_phys_final_clamp() {
    assert_eq!(weapon_phys_final(10, 20, 50, 0, 0, 1, 2), (18, 33));
    assert_eq!(weapon_phys_final(100, 100, -200, 0, 0, 0, 0), (10, 10));
}

#[test]
fn element_pair_mastery() {
    assert_eq!(element_pair(10, 20, 50), (15, 30));
    assert_eq!(element_pair(30, 20, 0), (20, 20));
}

#[test]
fn poison_pair_rules() {
    assert_eq!(poison_pair(256, 512, 0, 0, 10, 1), (10, 20));
    assert_eq!(poison_pair(256, 512, 0, 5, 10, 2), (5, 10));
    assert_eq!(poison_pair(256, 512, 0, 0, 10, 2), (5, 10));
    assert_eq!(poison_pair(256, 0, 0, 5, 10, 1), (0, 0));
}

#[test]
fn clamp_elements_rule() {
    assert_eq!(clamp_elements(0, 5), (1, 5));
    assert_eq!(clamp_elements(5, 5), (5, 6));
    assert_eq!(clamp_elements(0, 0), (0, 0));
}

#[test]
fn skill_elements_map() {
    assert_eq!(skill_elements(5, 512, 1024), (2, 4, Some(2)));
    assert_eq!(skill_elements(1, 256, 256), (1, 1, Some(1)));
    assert_eq!(skill_elements(2, 0, 0).2, Some(9));
    assert_eq!(skill_elements(3, 0, 0).2, Some(3));
    assert_eq!(skill_elements(4, 0, 0).2, Some(3));
    assert_eq!(skill_elements(9, 0, 0).2, None);
}

#[test]
fn charges() {
    assert_eq!(charge_bonus_term(1, 10, 5, 3, 2), 40);
    assert_eq!(charge_bonus_term(0, 10, 5, 3, 2), 0);
    assert_eq!(charge_index(0), 1);
    assert_eq!(charge_index(2), 2);
    assert_eq!(charge_index(9), 3);
}

#[test]
fn throw_formulas() {
    assert_eq!(throw_percent(-100, 0, 0), -90);
    assert_eq!(throw_base(10, 100, 0), 20);
    assert_eq!(throw_apply_q(10, 5, 100), (20, 20));
}

#[test]
fn potion_tail() {
    assert_eq!(potion_throw(512, 256), (2, 2));
    assert_eq!(potion_throw(256, 1024), (1, 4));
}

#[test]
fn row12() {
    assert_eq!(entry12(10, 20, 50), (15, 30));
    assert_eq!(entry12(10, 20, -100), (1, 2));
}

#[test]
fn row14() {
    assert_eq!(entry14(10, 20, 50), (15, 30));
}

#[test]
fn row15() {
    assert_eq!(entry15(10, 20, 100, 10, 20, 50), (35, 70));
}

#[test]
fn row11() {
    assert_eq!(entry11_base(0, 0), (1, 2));
    assert_eq!(entry11_base(5, 3), (5, 5));
    assert_eq!(entry11_term(10, 50), 15);
    assert_eq!(entry11_term(0, 50), 0);
}

#[test]
fn descatt5_only_other_hand() {
    assert_eq!(descatt5(120, 3), (120, 3, 0, 0));
    assert_eq!(descatt5(0, 0), (0, 0, 0, 0));
}

#[test]
fn driver_double_scale() {
    assert_eq!(driver_scale(10, 20, 64, 1, 2), (6, 12));
    assert_eq!(driver_scale(0, 20, 64, 1, 2), (0, 20));
}

#[test]
fn smite_adds_percent_stats_unscaled() {
    assert_eq!(entry10(10, 20, 0, 5, 5), (10, 20));
    assert_eq!(entry10(10, 20, 0, 100, 100), (11, 21));
}

#[test]
fn ar_thrown_zero() {
    assert_eq!(ar_base(100, true), 0);
    assert_eq!(ar_base(100, false), 100);
}
