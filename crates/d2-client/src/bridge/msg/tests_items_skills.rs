//! Test vectors of the item and skill messages, S→C 0x89 and the
//! no-op and out-of-scope ids; all synthetic.

use super::super::output::{Output, SkillTarget};
use super::super::skills::{SkillEntry, SkillList, NATIVE};
use super::super::world::{
    ClientWorld, ItemData, KindData, SkillDescRow, SkillRow, UnitKey, ITEM, MONSTER, PLAYER,
};
use super::support::{hex, Model};

const P1: UnitKey = UnitKey::new(PLAYER, 1);
const M10: UnitKey = UnitKey::new(MONSTER, 10);

#[test]
fn hireling_stats() {
    let mut m = Model::default();
    m.put(M10).stats.insert(13, 1000);
    m.hex("9e 07 0a 00 00 00 05");
    m.hex("a2 0d 0a 00 00 00 64 00");
    m.hex("a0 0c 0a 00 00 00 0b 00 00 00");
    let u = m.unit(M10);
    assert_eq!((u.stat(7), u.stat(13), u.stat(12)), (5, 1100, 11));
    // Only a monster: a player (0, 10) is not looked up.
    let mut m = Model::default();
    m.put(UnitKey::new(PLAYER, 10));
    m.hex("9e 07 0a 00 00 00 05");
    assert_eq!(m.unit(UnitKey::new(PLAYER, 10)).stat(7), 0);
}

fn item(m: &mut Model, guid: u32) -> UnitKey {
    let k = UnitKey::new(ITEM, guid);
    m.put(k).kind = KindData::Item(ItemData::default());
    k
}

fn flags(m: &Model, k: UnitKey) -> (bool, u32) {
    match &m.unit(k).kind {
        KindData::Item(d) => (d.flags4, d.flags),
        k => panic!("{k:?}"),
    }
}

#[test]
fn item_flags_and_stats() {
    let mut m = Model::default();
    let k = item(&mut m, 7);
    m.hex("40 07 00 00 00 00 01 00 00 01 00 00 00");
    assert_eq!(flags(&m, k), (false, 0x100));
    m.hex("40 07 00 00 00 00 01 00 00 00 00 00 00");
    assert_eq!(flags(&m, k), (false, 0));
    // 0x3E: a 0, GUID 7 (8 bits), set 1, stat 70 (9 bits), value a 0 + 3
    // (8 bits), param c 0 + 0 (8 bits) = 36 bits, LSB first.
    m.put(P1);
    m.w.local_player = Some(P1);
    if let KindData::Item(d) = &mut m.w.units.get_mut(&k).unwrap().kind {
        d.set_flags(4 | 0x4000, true);
    }
    m.hex("3e 07 0e 1a 31 00 00");
    assert_eq!(m.unit(k).stat(70), 3);
    assert_eq!(flags(&m, k), (false, 0));
    // The set writes the base array, layer 0: replaced in
    // place, or inserted in (stat, layer) order.
    use super::super::item_lists::{ItemFields, ItemProp};
    let p = |stat, value| ItemProp {
        stat,
        layer: 0,
        value,
    };
    if let KindData::Item(d) = &mut m.w.units.get_mut(&k).unwrap().kind {
        d.fields = Some(ItemFields {
            base: vec![p(68, -5), p(70, 100)],
            ..ItemFields::default()
        });
    }
    m.hex("3e 07 0e 1a 31 00 00");
    let base = |m: &Model| match &m.unit(k).kind {
        KindData::Item(d) => d.fields.as_ref().unwrap().base.clone(),
        k => panic!("{k:?}"),
    };
    assert_eq!(base(&m), [p(68, -5), p(70, 3)]);
    if let KindData::Item(d) = &mut m.w.units.get_mut(&k).unwrap().kind {
        d.fields.as_mut().unwrap().base = vec![p(31, 20), p(72, 9)];
    }
    m.hex("3e 07 0e 1a 31 00 00");
    assert_eq!(base(&m), [p(31, 20), p(70, 3), p(72, 9)]);
}

#[test]
fn charged_skill_stat_204_updates_the_list_entry() {
    use super::super::item_lists::ItemProp;
    let mut m = Model::default();
    let k = item(&mut m, 7);
    let prop = |layer, value| ItemProp {
        stat: 204,
        layer,
        value,
    };
    let set = |m: &mut Model, props: Vec<ItemProp>| {
        if let KindData::Item(d) = &mut m.w.units.get_mut(&k).unwrap().kind {
            d.props = props;
        }
    };
    let props = |m: &Model| match &m.unit(k).kind {
        KindData::Item(d) => d.props.clone(),
        k => panic!("{k:?}"),
    };
    // a 0, GUID 7, set 1, stat 204, value 12 (a 0, 8 bits), param 0x41.
    let msg = "3e 07 0e 32 c3 20 08";
    // No local player: nothing is written.
    set(&mut m, vec![prop(0x41, 0x0A05)]);
    m.hex(msg);
    assert_eq!(props(&m), [prop(0x41, 0x0A05)]);
    m.put(P1);
    m.w.local_player = Some(P1);
    // Max 10, charges 12: the entry takes the max (0x0A0A).
    m.hex(msg);
    assert_eq!(props(&m), [prop(0x41, 0x0A0A)]);
    // Another layer's entry stays; max 0 writes nothing.
    set(&mut m, vec![prop(0x42, 0x0A05), prop(0x41, 0x0005)]);
    m.hex(msg);
    assert_eq!(props(&m), [prop(0x42, 0x0A05), prop(0x41, 0x0005)]);
}

#[test]
fn use_scroll_set_item_state_remove_items_display() {
    let mut m = Model::default();
    let k = item(&mut m, 7);
    m.put(P1);
    {
        let u = m.w.units.get_mut(&P1).unwrap();
        u.states.insert(54);
        u.state_lists.insert(54, [((1, 0), 1)].into());
    }
    m.hex("7c 00 01 00 00 00");
    let u = m.unit(P1);
    assert!(!u.states.contains(&54) && u.state_lists.is_empty());
    m.hex("7d 00 01 00 00 00 07 00 00 00 00 01 00 00 01 00 00 00");
    assert_eq!(flags(&m, k), (false, 0x100));
    m.hex("7d 00 01 00 00 00 07 00 00 00 00 03 00 00 01 00 00 00");
    assert_eq!(flags(&m, k), (false, 0x100), "code 0x300: nothing");
    m.hex("7d 00 01 00 00 00 07 00 00 00 00 02 00 00 00 00 00 00");
    assert_eq!(flags(&m, k), (false, 0));
    let before = m.w.clone();
    m.hex("92 00 01 00 00 00");
    assert_eq!(m.w, before);
}

#[test]
fn weapon_switch_and_item_table() {
    let mut m = Model::default();
    m.w.expansion = 1;
    m.hex("97");
    assert_eq!(m.w.weapon_set, 0, "no d2exp.mpq");
    m.inputs.expansion_installed = true;
    m.hex("97");
    assert_eq!(m.w.weapon_set, 1);
    m.hex("97");
    assert_eq!(m.w.weapon_set, 0);
    // 0xA6 code 0, index 3, count 2.
    m.w.item_table_ext = vec![vec![1; 0x120]; 2];
    let mut msg = hex("a6 00 26 01 03 00");
    msg.extend((0..0x120).map(|i| i as u8));
    m.recv(&msg);
    let t = &m.w.item_table_ext;
    assert_eq!(t.len(), 4);
    assert_eq!(t[2], vec![0; 0x120]);
    assert_eq!(t[3], msg[6..].to_vec());
    // A short message is refused.
    m.hex("a6 00 06 00 03 00");
    assert_eq!(m.rejected().len(), 1);
}

/// A player with `entries` native skills (base 1).
fn with_skills(rows: Vec<SkillRow>, entries: &[u16]) -> Model {
    let mut m = Model::default();
    m.inputs.tables.skills = rows;
    m.inputs.tables.skilldesc = vec![SkillDescRow { page: 1 }; 4];
    let u = m.put(P1);
    u.skills = Some(SkillList {
        entries: entries
            .iter()
            .map(|&skill| SkillEntry {
                skill,
                base: 1,
                owner: NATIVE,
                ..SkillEntry::default()
            })
            .collect(),
        ..SkillList::default()
    });
    m
}

fn bonuses(m: &Model) -> Vec<i32> {
    m.unit(P1)
        .skills
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.level_bonus)
        .collect()
}

#[test]
fn skill_bonus_by_element_and_page() {
    let mut rows = vec![SkillRow::default(); 3];
    rows[1].enhanceable = true;
    let mut m = with_skills(rows, &[1, 2]);
    m.hex("93 01 00 00 00 01 00 04");
    assert_eq!(bonuses(&m), [1, 0]);
    assert!(m.log.rejected.is_empty());
    // Bonus 0x80 is +128; 0xFF is −1.
    m.hex("93 01 00 00 00 80 00 04");
    assert_eq!(bonuses(&m), [129, 0]);
    m.hex("93 01 00 00 00 ff 00 04");
    assert_eq!(bonuses(&m), [128, 0]);
    // Page: 4 = any, else the skilldesc page must be page + 1.
    m.hex("93 01 00 00 00 01 00 01");
    assert_eq!(bonuses(&m)[0], 128, "skilldesc page 1 ≠ 1 + 1");
    m.hex("93 01 00 00 00 01 00 00");
    assert_eq!(bonuses(&m)[0], 129);
    // Element: 0 = any, else `EType` must equal it.
    m.hex("93 01 00 00 00 01 03 04");
    assert_eq!(bonuses(&m)[0], 129);
    // Bonus 0 and no skill list are fatal.
    m.hex("93 01 00 00 00 00 00 04");
    m.w.units.get_mut(&P1).unwrap().skills = None;
    m.hex("93 01 00 00 00 01 00 04");
    assert_eq!(
        m.rejected(),
        [
            (0x93, "fatal assert 0x96B".to_string()),
            (0x93, "fatal assert 0x96D".to_string())
        ]
    );
    // clamps a negative bonus at 0: base 1 and
    // bonus 0x81 = −127 leave bonus 0, level 1, so the entry stays.
    let mut rows = vec![SkillRow::default(); 3];
    rows[1].enhanceable = true;
    let mut m = with_skills(rows.clone(), &[1, 2]);
    m.hex("93 01 00 00 00 81 00 04");
    assert_eq!(bonuses(&m), [0, 0]);
    // A base-0 entry with bonus 2 qualifies (level 2); −5 clamps the
    // bonus to 0, the level with bonuses is 0, so the entry is removed
    // (with d = 1: base −1 < 1).
    let mut m = with_skills(rows, &[1, 2]);
    {
        let l = m.w.units.get_mut(&P1).unwrap().skills.as_mut().unwrap();
        l.entries[0].base = 0;
        l.entries[0].level_bonus = 2;
    }
    m.hex("93 01 00 00 00 fb 00 04");
    let left: Vec<u16> = m
        .unit(P1)
        .skills
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.skill)
        .collect();
    assert_eq!(left, [2]);
}

#[test]
fn skill_events() {
    let mut m = Model::default();
    m.inputs.tables.skills = vec![SkillRow::default(); 100];
    let k = UnitKey::new(MONSTER, 5);
    m.put(k);
    m.put(P1);
    m.hex("99 01 05 00 00 00 40 00 03 00 01 00 00 00 00 00");
    m.hex("99 01 05 00 00 00 40 00 03 00 02 00 00 00 00 00");
    m.hex("9a 01 05 00 00 00 40 00 00 00 03 00 00 20 00 00 00");
    m.hex("9a 01 05 00 00 00 40 00 00 00 03 10 00 20 00 07 00");
    m.hex("a3 00 40 00 03 00 01 05 00 00 00 06 ff ff ff ff 00 00 00 00 00 00 00 00");
    assert_eq!(
        m.out,
        [
            Output::SkillEvent {
                unit: k,
                skill: 64,
                level: 3,
                target: SkillTarget::Unit(P1),
                w: 0
            },
            Output::SkillEvent {
                unit: k,
                skill: 64,
                level: 3,
                target: SkillTarget::Point(0x10, 0x20),
                w: 7
            },
            Output::SkillDo {
                unit: k,
                target: None,
                skill: 64,
                level: 3,
                x: 0,
                y: 0,
                v: 0
            },
        ]
    );
}

#[test]
fn skill_end() {
    let mut m = Model::default();
    let mut rows = vec![SkillRow::default(); 10];
    rows[5].srvdofunc = 5;
    rows[6].srvdofunc = 67;
    m.inputs.tables.skills = rows;
    let k = UnitKey::new(MONSTER, 5);
    m.put(k).states.insert(18);
    m.hex("a5 01 05 00 00 00 05 00");
    assert!(m.out.is_empty() && !m.unit(k).states.contains(&18));
    m.hex("a5 01 05 00 00 00 06 00");
    assert_eq!(
        m.out,
        [Output::SkillEndFx {
            unit: k,
            skill: 6,
            srvdofunc: 67
        }]
    );
}

#[test]
fn unique_event_sets_the_override_globals() {
    let mut m = Model::default();
    m.hex("89 00").hex("89 0c").hex("89 05");
    let o = &m.w.overrides;
    assert_eq!(o.den_counter, 0);
    assert!(o.glow_flag);
    assert_eq!(o.unique_bits, 1 | 1 << 12 | 1 << 5);
    m.hex("89 14").hex("89 20");
    assert_eq!(
        m.rejected(),
        [
            (0x89, "fatal assert 0x1DA".to_string()),
            (0x89, "fatal assert 0x1D9".to_string())
        ]
    );
}

#[test]
fn no_op_and_out_of_scope_ids() {
    let mut m = Model::default();
    let mut msg = vec![0x12];
    msg.resize(26, 0);
    m.recv(&msg);
    assert!(m.log.unowned.is_empty() && m.log.rejected.is_empty());
    assert_eq!(m.w, ClientWorld::default());
    let mut msg = vec![0x79];
    msg.resize(6, 0);
    m.recv(&msg);
    assert!(m.log.unowned.is_empty() && m.log.rejected.is_empty());
    assert_eq!(m.w, ClientWorld::default());
}
