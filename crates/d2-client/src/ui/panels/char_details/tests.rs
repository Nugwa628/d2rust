use super::*;

const S: Screen = Screen::R800; // sx 80, sy −60

fn blk(name: &str) -> HandBlock {
    HandBlock {
        name: name.into(),
        damage_label: "Damage".into(),
        attack_label: "Fire Bolt\nAttack Rating".into(),
        ..Default::default()
    }
}

#[test]
fn no_points_and_release_walk() {
    assert!(!points_block_drawn(0));
    assert!(points_block_drawn(5));
    // inside the area, outside the close rectangle: consumed, no press
    assert!(down_without_points(true, false));
    assert!(!down_without_points(true, true));
    assert!(!down_without_points(false, false));
    // The release walk (corrects)
    use super::super::character::CharacterPanel;
    use super::super::PanelTables;
    let t = PanelTables::load().unwrap();
    let s = Screen::R640;
    // a press on Vitality released on Strength spends Strength and leaves
    // Vitality pressed (the walk stopped before reaching it)
    let mut p = CharacterPanel {
        stat_pressed: [false, false, true, false],
        ..Default::default()
    };
    let o = p.release(&t, &s, Point::new(130, 100), false, 5);
    assert_eq!(o.len(), 1);
    assert_eq!(p.stat_pressed, [false, false, true, false]);
    // pressed fields before and at the hit are cleared, those after stay
    let mut p = CharacterPanel {
        stat_pressed: [true, true, true, true],
        ..Default::default()
    };
    p.release(&t, &s, Point::new(130, 245), false, 5); // vitality button
    assert_eq!(p.stat_pressed, [false, false, false, true]);
    // no points: mouse up clears no add-button flag and sends nothing
    let mut p = CharacterPanel {
        stat_pressed: [true, true, true, true],
        close_pressed: true,
    };
    assert!(p.release(&t, &s, Point::new(130, 100), false, 0).is_empty());
    assert_eq!(p.stat_pressed, [true; 4]);
    assert!(!p.close_pressed);
}

#[test]
fn class_and_name_lines() {
    assert_eq!(line_y(&S), 600 - 60 - 455);
    assert_eq!(class_span(&S), (80 + 193, 80 + 310));
    assert_eq!(name_span(&S), (80 + 13, 80 + 160));
    assert_eq!(name_code_points(b"Sorceress1"), 10);
    // UTF-8: é counts once; invalid bytes are not counted
    assert_eq!(name_code_points("Zoë".as_bytes()), 3);
    assert_eq!(name_code_points(b"ab\xFFcd"), 4);
    assert_eq!(name_code_points(b""), 0);
    // fonts: ≤ 10 Font16, 11–12 Font8, ≥ 13 Font6
    assert_eq!(name_font(10), 1);
    assert_eq!(name_font(11), 0);
    assert_eq!(name_font(12), 0);
    assert_eq!(name_font(13), 6);
    assert_eq!(name_font(15), 6);
}

#[test]
fn damage_block() {
    let mut once = false;
    // y of entry e = H + sy − 480 + y = 60 + y
    let b = HandBlock {
        damage: true,
        attack: true,
        v1: 120,
        c1: 3,
        ..blk("fire bolt")
    };
    let v = hand_block(&b, 0, &S, false, &mut once);
    let txt = |i: usize| match &v[i] {
        BlockItem::Text(t) => t.clone(),
        other => panic!("{other:?}"),
    };
    // name upper-cased, Font6, color 0, centered in e0
    assert_eq!(
        txt(0),
        BlockText {
            text: "FIRE BOLT".into(),
            font: 6,
            color: 0,
            x1: 242,
            x2: 338,
            y: 153
        }
    );
    // "Damage" in e1, then the descdam value entry e2
    assert_eq!(
        txt(1),
        BlockText {
            text: "Damage".into(),
            font: 6,
            color: 0,
            x1: 242,
            x2: 338,
            y: 161
        }
    );
    assert_eq!(
        v[2],
        BlockItem::DamageValue {
            x1: 343,
            x2: 387,
            y: 158
        }
    );
    // attack label with a LF: two parts at y − 4 and y + 4 in e3
    assert_eq!(
        txt(3),
        BlockText {
            text: "Fire Bolt".into(),
            font: 6,
            color: 0,
            x1: 242,
            x2: 350,
            y: 220 - 4
        }
    );
    assert_eq!(txt(4).y, 220 + 4);
    assert_eq!(txt(4).text, "Attack Rating");
    // v1 only: Font16 below 1,000, color c1, in e5 at y
    assert_eq!(
        txt(5),
        BlockText {
            text: "120".into(),
            font: 1,
            color: 3,
            x1: 350,
            x2: 390,
            y: 220
        }
    );
    assert_eq!(v.len(), 6);
    // v1 ≥ 1,000: Font8
    let big = HandBlock {
        v1: 1000,
        ..b.clone()
    };
    let v = hand_block(&big, 0, &S, false, &mut once);
    assert!(matches!(&v[5], BlockItem::Text(t) if t.font == 0));
    // v2 ≠ 0: Font6, v1 at y − 6 in c1, v2 at y + 2 in c2
    let two = HandBlock {
        v2: 80,
        c2: 1,
        ..b.clone()
    };
    let v = hand_block(&two, 0, &S, false, &mut once);
    assert!(
        matches!(&v[5], BlockItem::Text(t) if (t.text.as_str(), t.font, t.color, t.y) == ("120", 6, 3, 220 - 6))
    );
    assert!(
        matches!(&v[6], BlockItem::Text(t) if (t.text.as_str(), t.font, t.color, t.y) == ("80", 6, 1, 220 + 2))
    );
    // right hand: p = 6 uses e6.. e11
    let v = hand_block(&b, 6, &S, false, &mut once);
    assert!(matches!(&v[0], BlockItem::Text(t) if t.y == 60 + 117));
    assert!(matches!(&v[1], BlockItem::Text(t) if t.y == 60 + 125));
    assert_eq!(
        v[2],
        BlockItem::DamageValue {
            x1: 343,
            x2: 387,
            y: 60 + 122
        }
    );
    assert!(matches!(&v[5], BlockItem::Text(t) if (t.x1, t.x2, t.y) == (350, 390, 60 + 184)));
    // no damage / attack: the name only; v1 = v2 = 0: no attack lines
    let only = hand_block(&blk("x"), 0, &S, false, &mut once);
    assert_eq!(only.len(), 1);
    let zero = hand_block(
        &HandBlock {
            attack: true,
            ..blk("x")
        },
        0,
        &S,
        false,
        &mut once,
    );
    assert_eq!(zero.len(), 1);
    // an attack label without a LF: the name alone at y
    let one = HandBlock {
        attack: true,
        v1: 5,
        attack_label: "Attack".into(),
        ..blk("x")
    };
    let v = hand_block(&one, 0, &S, false, &mut once);
    assert!(matches!(&v[1], BlockItem::Text(t) if (t.text.as_str(), t.y) == ("Attack", 220)));
    // languages 6–9: e0 and e6 up by 1; v1 at y − 7
    let mut once2 = false;
    let v = hand_block(&HandBlock { v2: 1, ..b.clone() }, 0, &S, true, &mut once2);
    assert!(once2);
    assert!(matches!(&v[0], BlockItem::Text(t) if t.y == 152));
    assert!(matches!(&v[5], BlockItem::Text(t) if t.y == 220 - 7));
    // the case table
    assert_eq!(upper_name("fire bolt é"), "FIRE BOLT É");
    assert_eq!(upper_latin1(0xF7), 0xF7);
    assert_eq!(upper_latin1(0xFF), 0x9F);
    assert_eq!(upper_latin1(0x9A), 0x8A);
    assert_eq!(upper_latin1(0x9C), 0x8C);
    assert_eq!(upper_latin1(0x9E), 0x9E);
    assert_eq!(upper_latin1(0xDF), 0xDF);
}

#[test]
fn chances() {
    let m = MonsterFacts {
        level: 10,
        ac: 100,
        align_not_1: true,
    };
    // A = D → pct 50; Lp = Lm → 2 × 50 × 10 / 20 = 50
    assert_eq!(chance_to_hit(60, &m, false, 0, 40, 10), 50);
    // higher player level raises it; clamped to [5, 95]
    assert_eq!(chance_to_hit(60, &m, false, 0, 40, 30), 2 * 50 * 30 / 40);
    assert_eq!(chance_to_hit(100000, &m, false, 0, 0, 90), 95);
    assert_eq!(
        chance_to_hit(0, &MonsterFacts { ac: 1_000_000, ..m }, false, 0, 0, 1),
        5
    );
    // classic game, difficulty ≠ 0, Align ≠ 1: D × 10 / 12 = 83
    let a = chance_to_hit(60, &m, true, 1, 40, 10);
    assert_eq!(a, 2 * (100 * 100 / (100 + 83)) * 10 / 20);
    // Align = 1: no change
    let n = MonsterFacts {
        align_not_1: false,
        ..m
    };
    assert_eq!(chance_to_hit(60, &n, true, 1, 40, 10), 50);
    // D < 0 moves into A; A < 0 moves into D; both 0 → pct 100
    assert_eq!(
        chance_to_hit(0, &MonsterFacts { ac: -20, ..m }, false, 0, 0, 10),
        95
    );
    assert_eq!(
        chance_to_hit(-30, &MonsterFacts { ac: 20, ..m }, false, 0, 0, 10),
        5
    );
    assert_eq!(
        chance_to_hit(0, &MonsterFacts { ac: 0, ..m }, false, 0, 0, 10),
        95
    );
    // to be hit
    assert_eq!(chance_to_be_hit(None, 10, 0, [5, 0, 0], false, 0, 10), 0);
    // A = first non-zero to-hit; D = defense + stat 33 → pct 50 → 50
    assert_eq!(
        chance_to_be_hit(Some(&m), 90, 10, [0, 100, 7], false, 0, 10),
        50
    );
    // the popup clamps
    assert_eq!(popup_clamp(2 * 100 * 90 / 100), 95);
    assert_eq!(popup_clamp(0), 5);
    // classic hell: A × 10 / 15
    let v = chance_to_be_hit(Some(&m), 100, 0, [150, 0, 0], true, 2, 10);
    assert_eq!(v, 2 * (100 * 100 / 200) * 10 / 20);
    // Lm bigger than Lp raises it
    assert_eq!(
        chance_to_be_hit(
            Some(&MonsterFacts { level: 30, ..m }),
            100,
            0,
            [100, 0, 0],
            false,
            0,
            10
        ),
        2 * 50 * 30 / 40
    );
}

#[test]
fn hovered_monster_classes() {
    let mut h = HoveredClasses::default();
    assert_eq!((h.to_hit, h.to_be_hit), (19, 19));
    let u = |monster, attackable, not_inert| HoverUnit {
        is_monster: monster,
        class: 42,
        attackable,
        not_inert,
    };
    h.update(&u(false, true, true));
    assert_eq!((h.to_hit, h.to_be_hit), (19, 19));
    h.update(&u(true, false, true));
    assert_eq!((h.to_hit, h.to_be_hit), (19, 19));
    // an inert attackable monster sets the first only
    h.update(&u(true, true, false));
    assert_eq!((h.to_hit, h.to_be_hit), (42, 19));
    h.update(&HoverUnit {
        class: 7,
        ..u(true, true, true)
    });
    assert_eq!((h.to_hit, h.to_be_hit), (7, 7));
}

#[test]
fn popups() {
    // hy = H + sy = 540
    assert_eq!(to_hit_hand(&S, Point::new(242, 206)), Some(0));
    assert_eq!(to_hit_hand(&S, Point::new(400, 225)), Some(0));
    assert_eq!(to_hit_hand(&S, Point::new(242, 232)), Some(1));
    assert_eq!(to_hit_hand(&S, Point::new(400, 251)), Some(1));
    assert_eq!(to_hit_hand(&S, Point::new(241, 210)), None);
    assert_eq!(to_hit_hand(&S, Point::new(401, 210)), None);
    assert_eq!(to_hit_hand(&S, Point::new(300, 226)), None);
    assert_eq!(to_hit_hand(&S, Point::new(300, 252)), None);
    // rectangle and lines: (sx, H + sy − 480) = (80, 60)
    let (r, l1, l2) = to_hit_popup(&S, 0);
    assert_eq!(r, (80 + 154, 60 + 115, 80 + 154 + 155, 60 + 115 + 30));
    assert_eq!((l1, l2), (Point::new(239, 188), Point::new(239, 200)));
    let (r, l1, l2) = to_hit_popup(&S, 1);
    assert_eq!(r.1, 190);
    assert_eq!((l1, l2), (Point::new(239, 203), Point::new(239, 215)));
    assert_eq!(POPUP_FILL, (0, 2));
    assert_eq!((STR_AVG_HIT, STR_MONSTER_X), (4159, 10103));
    // to-be-hit
    assert!(to_be_hit_hit(&S, Point::new(253, 254)));
    assert!(to_be_hit_hit(&S, Point::new(391, 273)));
    assert!(!to_be_hit_hit(&S, Point::new(392, 260)));
    assert!(!to_be_hit_hit(&S, Point::new(300, 274)));
    assert_eq!(to_be_hit_text_at(&S), Point::new(219, 225));
    assert_eq!((to_be_hit_text_id(5), to_be_hit_text_id(0)), (10105, 10104));
    // the block value
    assert_eq!(block_value(30, &[], [None, None], false), 30);
    // base ≤ 0: walk the entries; layer 0 sets, a matching layer raises
    let e = [(0, 10), (5, 25), (6, 40)];
    assert_eq!(block_value(0, &e, [Some(5), None], true), 25);
    assert_eq!(block_value(0, &e, [None, Some(6)], true), 40);
    assert_eq!(block_value(0, &e, [Some(9), None], true), 10);
    // not weapon class 13: 0
    assert_eq!(block_value(0, &e, [Some(5), None], false), 0);
    // the walk stops after 32 entries
    let long: Vec<(u32, i32)> = (0..40)
        .map(|i| if i == 35 { (0, 99) } else { (1, 0) })
        .collect();
    assert_eq!(block_value(0, &long, [None, None], true), 0);
}

#[test]
fn draw_order() {
    assert_eq!(DRAW_ORDER.len(), 12);
    assert_eq!(DRAW_ORDER[0], "quads");
    assert_eq!(DRAW_ORDER[5], "damage block");
    assert_eq!(DRAW_ORDER[6], "to-hit popup");
    assert_eq!(DRAW_ORDER[7], "to-be-hit popup");
    assert_eq!(DRAW_ORDER[9], "class line");
    assert_eq!(DRAW_ORDER[11], "18 values");
}

#[test]
fn experience_and_next_level() {
    assert_eq!(group_digits(1234567, 128), "1,234,567");
    assert_eq!(group_digits(999, 128), "999");
    assert_eq!(group_digits(1000, 128), "1,000");
    assert_eq!(group_digits(0, 128), "0");
    assert_eq!(group_digits(4_294_967_295, 128), "4,294,967,295");
    // does not fit the buffer: `*`
    assert_eq!(group_digits(1234567, 5), "*");
    // next level: the entry of the class on the row of level L; level 1 → 500
    let max = |_: u32| 99;
    let entry = |c: u32, l: u32| if l == 1 { 500 } else { 1000 * (c + 1) * l };
    assert_eq!(next_level_value(1, 0, &max, &entry, 77), 500);
    assert_eq!(next_level_value(5, 2, &max, &entry, 77), 15000);
    // at the maximum level: stat 30
    assert_eq!(next_level_value(99, 2, &max, &entry, 77), 77);
    // a class outside 0–6 reads as 0
    assert_eq!(next_level_value(5, 9, &max, &entry, 77), entry(0, 5));
}
