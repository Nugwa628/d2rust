use std::collections::HashMap;

use super::character_bind_tests::tbl;
use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::skills::{SkillEntry, SkillList};
use crate::bridge::world::{ClientUnit, PlayerData};
use crate::ui::char_feed::{CharTables, DescRow};
use crate::ui::{NoPanelRules, StringLookup, UiDraw};

struct Strs(HashMap<u16, Vec<u16>>, HashMap<String, Vec<u16>>);

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Strs {
    /// Every label id answers "S<id>" unless overridden.
    fn new(over: &[(u16, &str)]) -> Self {
        let mut ids: HashMap<u16, Vec<u16>> =
            (0..6000u16).map(|i| (i, u16s(&format!("S{i}")))).collect();
        for (i, t) in over {
            ids.insert(*i, u16s(t));
        }
        let keys = [("Barbarian".to_string(), u16s("Barbarian"))].into();
        Strs(ids, keys)
    }
}

impl StringLookup for Strs {
    fn get(&self, key: &str) -> Option<&[u16]> {
        self.1.get(key).map(Vec::as_slice)
    }
    fn get_id(&self, id: u16) -> Option<&[u16]> {
        self.0.get(&id).map(Vec::as_slice)
    }
}

fn fonts() -> FontMeasure {
    let mut src = MemorySource::default();
    for id in CHARACTER_FONTS {
        src.insert(crate::ui::font_info(id).unwrap().tbl_path, tbl(6));
    }
    FontMeasure::load(&src, &CHARACTER_FONTS).unwrap()
}

fn world(stats: &[(u16, i32)]) -> ClientWorld {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.class = 4;
    u.mode = 1;
    u.kind = KindData::Player(PlayerData::default());
    for &(s, v) in stats {
        u.stats.insert(s, v);
    }
    w.units.insert(key, u);
    w.local_player = Some(key);
    w
}

fn tables() -> CharTables {
    let mut t = CharTables {
        class_keys: ["Amazon", "Sorceress", "Necromancer", "Paladin", "Barbarian"]
            .map(String::from)
            .to_vec(),
        // State 5: rfblue (bit 1); state 6: armred (bit 5).
        state_flags: vec![0, 0, 0, 0, 0, 1 << 1, 1 << 5],
        ..Default::default()
    };
    t.skill_desc.insert(
        3,
        DescRow {
            name_id: 5000,
            descdam: 1,
            descatt: 2,
            src_dam: 128,
        },
    );
    t
}

type Text = (String, i32, i32, u16, u16);

fn draw(w: &ClientWorld, strings: &Strs, exp: Vec<[u32; 7]>) -> Vec<Text> {
    draw_tables(w, strings, exp, tables())
}

fn draw_with(w: &ClientWorld, strings: &Strs, t: CharTables) -> Vec<Text> {
    draw_tables(w, strings, vec![], t)
}

fn draw_tables(w: &ClientWorld, strings: &Strs, exp: Vec<[u32; 7]>, t: CharTables) -> Vec<Text> {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let mut ui = OriginalUi::new(config, None).unwrap();
    ui.set_fonts(fonts());
    ui.set_char_tables(t);
    ui.set_hud_tables(hud::HudTables {
        experience: exp,
        ..Default::default()
    });
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    ui.set_ui(u32::from(UI_CHARACTER), 2, false).unwrap();
    root.sync_states(&ui.shared.borrow().states);
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    out.iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some((
                String::from_utf16_lossy(&t.text),
                t.at.x,
                t.at.y,
                t.style.font,
                t.style.color,
            )),
            _ => None,
        })
        .collect()
}

fn find<'a>(v: &'a [Text], s: &str) -> Option<&'a Text> {
    v.iter().find(|t| t.0 == s)
}

#[test]
fn labels_come_from_the_string_table_by_id() {
    let t = draw(&world(&[(12, 7)]), &Strs::new(&[]), vec![]);
    // The 15 label rows resolve their ids (`S<id>`), not nothing.
    let labels = t.iter().filter(|x| x.0.starts_with('S')).count();
    assert!(labels >= 15, "{labels} labels in {t:?}");
}

#[test]
fn class_line_is_the_charstats_name_centered_in_its_span() {
    let t = draw(&world(&[]), &Strs::new(&[]), vec![]);
    let s = Screen::R800;
    // "Barbarian": 9 units of 6 in [sx + 193, sx + 310]: 118 wide.
    assert_eq!(
        find(&t, "Barbarian"),
        Some(&(
            "Barbarian".into(),
            s.sx() + 193 + ((118 - 54) >> 1),
            s.h + s.sy() - 455,
            1,
            0
        ))
    );
}

#[test]
fn next_level_is_the_experience_row_of_the_base_level() {
    let mut rows = vec![[99u32; 7]; 10];
    rows[8][4] = 1_234_567; // level 7, class 4
    let t = draw(&world(&[(12, 7), (30, 5)]), &Strs::new(&[]), rows);
    assert!(find(&t, "1,234,567").is_some(), "{t:?}");
    // Without the table: stat 30.
    let t = draw(&world(&[(12, 7), (30, 5000)]), &Strs::new(&[]), vec![]);
    assert!(find(&t, "5,000").is_some());
}

#[test]
fn resist_and_defense_take_their_colours_from_the_state_flags() {
    let mut w = world(&[(39, 10), (31, 77)]);
    let key = UnitKey::new(PLAYER, 1);
    let t = draw(&w, &Strs::new(&[]), vec![]);
    assert_eq!(find(&t, "10").map(|x| x.4), Some(0));
    assert_eq!(find(&t, "77").map(|x| x.4), Some(0));
    let u = w.units.get_mut(&key).unwrap();
    u.states.insert(5); // rfblue
    u.states.insert(6); // armred
    let t = draw(&w, &Strs::new(&[]), vec![]);
    assert_eq!(find(&t, "10").map(|x| x.4), Some(3));
    assert_eq!(find(&t, "77").map(|x| x.4), Some(1));
}

#[test]
fn the_damage_block_prints_name_damage_and_attack_rating() {
    let mut w = world(&[(21, 10), (22, 20), (18, 50), (19, 100), (119, 10)]);
    let key = UnitKey::new(PLAYER, 1);
    w.units.get_mut(&key).unwrap().skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: 3,
            ..Default::default()
        }],
        left: Some(0),
        ..Default::default()
    });
    let strings = Strs::new(&[(5000, "Bash"), (4063, "%s\nAttack Rating")]);
    let t = draw(&w, &strings, vec![]);
    let s = Screen::R800;
    let (sx, base) = (s.sx(), s.h + s.sy() - 480);
    // e0: the upper-cased skill name, Font6, colour 0.
    let name = find(&t, "BASH").expect("name");
    assert_eq!((name.2, name.3, name.4), (base + 93, 6, 0));
    // e1: "Damage".
    assert_eq!(find(&t, "S4061").map(|x| x.2), Some(base + 101));
    // e2 (bare hand): b = 10 + 1, B = 20 + 2, P = strength 0; min (100 + 50) ×
    // 11 / 100 = 16, max 100 × 22 / 100 = 22: "16-22" is too wide for its span
    // at Font16 (30 × 11 / 7 > 44): Font6, one up.
    let dmg = find(&t, "16-22").expect("damage");
    assert_eq!((dmg.1, dmg.2, dmg.3), (sx + 263 + 7, base + 98 - 1, 6));
    // e3: the label split at its LF; e5: `attack_rating` = 100 + 5 × (0 −
    // 7) + 0 = 65, + 10 % = 71, Font16.
    // (the label is formatted with the upper-cased name, like e0.)
    let first = t.iter().find(|x| x.0 == "BASH" && x.2 == base + 160 - 4);
    assert!(first.is_some());
    assert_eq!(find(&t, "Attack Rating").map(|x| x.2), Some(base + 160 + 4));
    let ar = find(&t, "71").expect("attack rating");
    assert_eq!((ar.2, ar.3, ar.4), (base + 160, 1, 0));
}

/// The character panel of a level-1 Amazon (Attack in both hands)
/// with three stat sets
/// and `char-panel-ama-ssd.tsv`: damage `1-2`, `1-3`, `3-10` (Font16, y 158 /
/// 182), attack rating `95`, `270`, `270`.
#[test]
fn the_damage_block_matches_the_recorded_amazon() {
    use crate::bridge::items::ITEM;
    let measured = [
        (20, 25, None, "1-2", "95"),
        (50, 60, None, "1-3", "270"),
        // Short sword: the unit's 21 / 22 include its 2-7; StrBonus 100.
        (50, 60, Some((2, 7)), "3-10", "270"),
    ];
    for (st, dx, sword, dmg, ar) in measured {
        let mut stats = vec![(0, st), (2, dx)];
        if let Some((a, b)) = sword {
            stats.extend([(21, a), (22, b)]);
        }
        let mut w = world(&stats);
        let key = UnitKey::new(PLAYER, 1);
        let u = w.units.get_mut(&key).unwrap();
        u.class = 0;
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 3,
                ..Default::default()
            }],
            left: Some(0),
            right: Some(0),
            ..Default::default()
        });
        let mut t = tables();
        t.tohit_factor = vec![5];
        if sword.is_some() {
            t.weapons.insert(
                *b"ssd ",
                crate::ui::char_feed::WeaponRow {
                    str_bonus: 100,
                    dex_bonus: 0,
                },
            );
            let ik = UnitKey::new(ITEM, 9);
            let mut i = ClientUnit::new(ik);
            i.kind = KindData::Item(crate::bridge::world::ItemData {
                last: Some(crate::bridge::world::ItemRecord {
                    id: 0x9C,
                    action: 0x04,
                    category: 0,
                    owner: None,
                    seq: 0,
                    stream: body_stream(4, b"ssd "),
                }),
                ..Default::default()
            });
            w.units.insert(ik, i);
        }
        let strings = Strs::new(&[(5000, "Attack"), (4063, "%s\nAttack Rating")]);
        let out = draw_with(&w, &strings, t);
        let d: Vec<_> = out.iter().filter(|x| x.0 == dmg).collect();
        assert_eq!(d.len(), 2, "{dmg} in both hands: {out:?}");
        assert!(d.iter().all(|x| x.3 == 1), "Font16");
        let a: Vec<_> = out.iter().filter(|x| x.0 == ar).collect();
        assert_eq!(a.len(), 2, "AR {ar} in both hands: {out:?}");
    }
}

/// An item stream head of an item in body
/// location `loc` (mode 1).
fn body_stream(loc: u8, code: &[u8; 4]) -> Vec<u8> {
    let bits: [(u32, u32); 8] = [
        (0x10, 32),
        (0x65, 10),
        (1, 3),
        (u32::from(loc), 4),
        (0, 4),
        (0, 4),
        (0, 3),
        (u32::from_le_bytes(*code), 32),
    ];
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

// The Attack skill draws "ATTACK" / "Rating".
#[test]
fn the_attack_skill_label_uses_the_no_record_string_upper_cased() {
    let strings = Strs::new(&[
        (5000, "Attack"),
        (4063, "%s\nAttack Rating"),
        (4065, "%s\nRating"),
    ]);
    let row = DescRow {
        name_id: 5000,
        descatt: 1,
        ..Default::default()
    };
    let h = crate::ui::char_feed::hand(0, &row, &strings, &|_| 0, 0);
    assert_eq!(h.attack_label, "ATTACK\nRating");
    let h = crate::ui::char_feed::hand(3, &row, &strings, &|_| 0, 0);
    assert_eq!(h.attack_label, "ATTACK\nAttack Rating");
}
