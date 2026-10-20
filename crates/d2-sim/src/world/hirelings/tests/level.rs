//! Level stats, experience, the restore level and the stat messages on
//! synthetic rows. Row columns are filled only where the reference vectors
//! fix them; other columns stay 0 and their stats are not asserted.

use super::fake::*;
use crate::units::UnitId;
use crate::world::hirelings::level::*;
use crate::world::hirelings::{
    stat, threshold, ExpRatios, HirelingError, HirelingRow, HirelingState, HirelingTables, PetNode,
};

const PLAYER_GUID: u32 = 0x10;
const MERC_GUID: u32 = 0x4433_2211;
const G: [u8; 4] = MERC_GUID.to_le_bytes();

struct Setup {
    w: Fake,
    t: HirelingTables,
    st: HirelingState,
    player: UnitId,
    merc: UnitId,
}

/// A player (level `plvl`) and its living hireling of `id` on `rows`.
fn setup(rows: Vec<HirelingRow>, id: u32, plvl: i32) -> Setup {
    let mut w = Fake::new(true);
    let player = w.add(1, 0, 0, PLAYER_GUID);
    let merc = w.add(2, 1, 271, MERC_GUID);
    w.set(player, stat::LEVEL, plvl);
    w.unit_mut(merc).owner = Some((PLAYER_GUID, 0));
    let mut st = HirelingState::default();
    st.list_mut(player).nodes.push(PetNode {
        guid: MERC_GUID,
        id,
        ..PetNode::default()
    });
    Setup {
        w,
        t: tables(rows),
        st,
        player,
        merc,
    }
}

/// Act 1 Ice (Id 1) with `Exp/Lvl` 105 on every bracket: nextexp 41160 =
/// threshold(7) = 392·Exp/Lvl at L 6 (Ice N vector, cap vector).
fn ice_105() -> Vec<HirelingRow> {
    act1_ice_rows()
        .into_iter()
        .map(|mut r| {
            r.exp_lvl = 105;
            r
        })
        .collect()
}

/// A level-6 Ice hireling at 26460 experience (threshold(6), Exp/Lvl 105).
fn ice_l6(plvl: i32) -> Setup {
    let mut s = setup(ice_105(), 1, plvl);
    s.w.set(s.merc, stat::LEVEL, 6);
    s.w.set(s.merc, stat::EXPERIENCE, 26460);
    s
}

/// A defender of level 6 worth 100000 experience.
fn defender(w: &mut Fake) -> UnitId {
    let d = w.add(3, 1, 0, 0x99);
    w.set(d, stat::LEVEL, 6);
    w.set(d, stat::EXPERIENCE, 100_000);
    d
}

#[test]
fn cap_exp_lvl_105_alvl_6_is_229() {
    let s = ice_l6(10);
    assert_eq!((threshold(105, 7) - threshold(105, 6)) >> 6, 229);
    let g = gain(&s.w, &s.t, &s.st, s.player, s.merc, 100_000, 6, 6);
    assert_eq!(g, 229);
    // Without the owner link the cap does not apply.
    let mut s = s;
    s.w.unit_mut(s.merc).owner = None;
    let g = gain(&s.w, &s.t, &s.st, s.player, s.merc, 100_000, 6, 6);
    assert_eq!(g, 100_000);
}

#[test]
fn gain_clamps_max_level_and_stat_85() {
    let mut s = ice_l6(10);
    s.w.unit_mut(s.merc).owner = None;
    let g = |s: &Setup, exp, alvl| gain(&s.w, &s.t, &s.st, s.player, s.merc, exp, alvl, alvl);
    assert_eq!(g(&s, 0, 6), 1);
    assert_eq!(g(&s, -5, 6), 1);
    assert_eq!(g(&s, 0x0100_0000, 6), 0x7F_FFFF);
    assert_eq!(g(&s, 1000, 99), 0);
    assert_eq!(g(&s, 1000, 98), 1000);
    // The ExpRatio step runs between the level factor and stat 85.
    s.t.exp_ratios = ratios(&[(6, 512)]);
    let r = gain(&s.w, &s.t, &s.st, s.player, s.merc, 1000, 6, 6);
    assert_eq!(r, 500);
    s.t.exp_ratios = Default::default();
    // Stat 85 (total, items count): + pct(gain, 85, 100).
    s.w.unit_mut(s.merc).bonus.insert(stat::ADDEXPERIENCE, 50);
    assert_eq!(g(&s, 1000, 6), 1500);
    // Level factor: defender 7 levels below → 159/256.
    let r = gain(&s.w, &s.t, &s.st, s.player, s.merc, 256, 13, 6);
    assert_eq!(r, 159 + 159 / 2);
}

#[test]
fn merc_kill_gain_229_gives_26918() {
    let mut s = ice_l6(10);
    let d = defender(&mut s.w);
    kill_share(&mut s.w, &s.t, &s.st, s.merc, d, Some(s.player)).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26918);
    let mut want = vec![0xA2, 0x0D];
    want.extend_from_slice(&G);
    want.extend_from_slice(&[0xCA, 0x01]);
    assert_eq!(s.w.sent_to(s.player), vec![want]);
    // No level-up: threshold(7) = 41160 > 26918.
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 6);
}

#[test]
fn player_kill_gives_76_then_26612() {
    let mut s = ice_l6(10);
    let d = defender(&mut s.w);
    kill_share(&mut s.w, &s.t, &s.st, s.player, d, None).unwrap();
    assert_eq!(229 * 86 / 256, 76);
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26612);
    let mut want = vec![0xA1, 0x0D];
    want.extend_from_slice(&G);
    want.push(0x98);
    assert_eq!(s.w.sent_to(s.player), vec![want]);
}

#[test]
fn kill_share_needs_a_player_and_experience() {
    // A monster attacker credits its player owner (caller-supplied).
    let mut s = ice_l6(10);
    let d = defender(&mut s.w);
    let pet = s.w.add(4, 1, 0, 0x77);
    kill_share(&mut s.w, &s.t, &s.st, pet, d, Some(s.player)).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26612);
    // No player owner → nothing.
    let mut s = ice_l6(10);
    let d = defender(&mut s.w);
    let pet = s.w.add(4, 1, 0, 0x77);
    kill_share(&mut s.w, &s.t, &s.st, pet, d, None).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    // Attacker type 2 (object) → nothing.
    let obj = s.w.add(5, 2, 0, 0x78);
    kill_share(&mut s.w, &s.t, &s.st, obj, d, Some(s.player)).unwrap();
    // Defender without experience → nothing.
    s.w.set(d, stat::EXPERIENCE, 0);
    kill_share(&mut s.w, &s.t, &s.st, s.player, d, None).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    assert!(s.w.sent.is_empty());
}

#[test]
fn large_delta_carries_the_old_value() {
    let mut want = vec![0xA0, 0x0D];
    want.extend_from_slice(&G);
    want.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(exp_delta_message(13, MERC_GUID, 0, 70000), want);
    // Boundaries: 0xFE → 0xA1, 0xFF → 0xA2, 0xFFFE → 0xA2, 0xFFFF → 0xA0.
    assert_eq!(exp_delta_message(13, 0, 10, 10 + 0xFE)[0], 0xA1);
    assert_eq!(exp_delta_message(13, 0, 10, 10 + 0xFF)[0], 0xA2);
    assert_eq!(exp_delta_message(13, 0, 10, 10 + 0xFFFE)[0], 0xA2);
    let m = exp_delta_message(13, 0, 10, 10 + 0xFFFF);
    assert_eq!((m[0], &m[6..]), (0xA0, &10u32.to_le_bytes()[..]));
}

#[test]
fn stat_message_encodings() {
    let msg = |op: u8, stat: u8, v: &[u8]| {
        let mut b = vec![op, stat];
        b.extend_from_slice(&G);
        b.extend_from_slice(v);
        b
    };
    assert_eq!(
        stat_message(13, MERC_GUID, 300).unwrap(),
        msg(0x9F, 13, &[0x2C, 0x01])
    );
    assert_eq!(
        stat_message(13, MERC_GUID, 70000).unwrap(),
        msg(0xA0, 13, &70000u32.to_le_bytes())
    );
    assert_eq!(stat_message(0, MERC_GUID, 5).unwrap(), msg(0x9E, 0, &[5]));
    assert_eq!(stat_message(0, 0, 0xFE).unwrap()[0], 0x9E);
    assert_eq!(stat_message(0, 0, 0xFF).unwrap()[0], 0x9F);
    assert_eq!(stat_message(0, 0, 0xFFFE).unwrap()[0], 0x9F);
    assert_eq!(stat_message(0, 0, 0xFFFF).unwrap()[0], 0xA0);
    assert!(stat_message(0xFE, 0, 1).is_ok());
    assert_eq!(stat_message(0xFF, 0, 1), Err(HirelingError::StatId(0xFF)));
}

#[test]
fn send_stats_order_and_sums() {
    let mut s = ice_l6(10);
    let m = s.merc;
    for (st, v) in [
        (stat::STRENGTH, 38),
        (stat::DEXTERITY, 51),
        (stat::MAXHP, 18432),
        (stat::HITPOINTS, 1000),
        (stat::ARMORCLASS, 39),
        (stat::NEXTEXP, 41160),
        (stat::SECONDARY_MINDAMAGE, 1),
        (stat::MINDAMAGE, 2),
        (stat::SECONDARY_MAXDAMAGE, 3),
        (stat::MAXDAMAGE, 4),
        (stat::FIRERESIST, 6),
        (stat::LIGHTRESIST, 7),
        (stat::COLDRESIST, 8),
        (stat::POISONRESIST, 9),
    ] {
        s.w.set(m, st, v);
    }
    // Totals for 12, 6, 13, 30; base for the others.
    for st in [
        stat::LEVEL,
        stat::HITPOINTS,
        stat::EXPERIENCE,
        stat::NEXTEXP,
    ] {
        s.w.unit_mut(m).bonus.insert(st, 1);
    }
    s.w.unit_mut(m).bonus.insert(stat::STRENGTH, 100);
    send_stats(&mut s.w, &s.st, s.player).unwrap();
    // Queued on the merc, nothing sent (HL8).
    assert!(s.w.sent.is_empty());
    let got = s.w.queued[&m].clone();
    // The flush encodes each record with the merc's GUID.
    for b in flush_stats(MERC_GUID, &got).unwrap() {
        assert_eq!(&b[2..6], &G);
    }
    let got: Vec<(u8, u32)> = got.iter().map(|&(st, v)| (st as u8, v)).collect();
    assert_eq!(
        got,
        vec![
            (12, 7),
            (0, 38),
            (2, 51),
            (7, 18432),
            (6, 1001),
            (31, 39),
            (13, 26461),
            (30, 41161),
            (21, 3),
            (22, 7),
            (39, 6),
            (41, 7),
            (43, 8),
            (45, 9),
        ]
    );
    // No living hireling → nothing.
    let mut s = ice_l6(10);
    s.st.list_mut(s.player).nodes[0].dead = true;
    send_stats(&mut s.w, &s.st, s.player).unwrap();
    assert!(s.w.sent.is_empty());
    assert!(s.w.queued.is_empty());
}

#[test]
fn level_speech_bytes() {
    let b = speech_message(MERC_GUID);
    assert_eq!(b.len(), 40);
    let mut want = [0u8; 40];
    want[0] = 0x27;
    want[1] = 1;
    want[2..6].copy_from_slice(&G);
    want[6] = 1;
    want[8] = 3;
    want[10] = 0x7C;
    want[11] = 0x0D;
    assert_eq!(b, want);
    let mut s = ice_l6(10);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 6);
    assert_eq!(s.w.sent_to(s.player), vec![want.to_vec()]);
}

#[test]
fn ice_normal_l6_nextexp_experience_and_skills() {
    // Ice N, L 6, row 3: nextexp 41160, experience threshold(6) = 26460
    // (offer word 6); Inner Sight (Level 1, LvlPerLvl 10, edge case 4):
    // (10·3 >> 5) + 1 = 1.
    let mut rows = ice_105();
    rows[0].skills[0] = skill(8, 0, 1, 10);
    let mut s = setup(rows, 1, 10);
    s.w.reqlevel.insert(8, 1);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 6);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 6);
    assert_eq!(s.w.base(s.merc, stat::NEXTEXP), 41160);
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    assert_eq!(s.w.unit(s.merc).skills.get(&8), Some(&1));
    // Experience above threshold(level) is kept (step 7).
    s.w.set(s.merc, stat::EXPERIENCE, 30000);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 6);
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 30000);
    // Unsigned compare: a negative total counts as large.
    s.w.set(s.merc, stat::EXPERIENCE, -1);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 6);
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), -1);
}

#[test]
fn fire_normal_l2_rounds_toward_zero_and_skips_level_0() {
    // Fire N (Id 0), L 2, row 3 (d = −1). From the vector: offer str 33 /
    // unit 34 → Str 34, Str/Lvl in 1..=7; dex 43 both → Dex 43, Dex/Lvl 0;
    // dmg offer 0–2 / unit 1–3 → Dmg-Min 1, Dmg-Max 3, Dmg/Lvl in 1..=7;
    // nextexp 3600 = 36·Exp/Lvl → 100; offer life 40 (clamped) → any HP +
    // HP/Lvl·d ≤ 40, so the unit's 10240 (0x2800) clamp holds for HP 0.
    for per_lvl in 1..=7 {
        let mut r = row(100, 0, 1, 1, 3);
        r.str_ = 34;
        r.str_lvl = per_lvl;
        r.dex = 43;
        r.dmg_min = 1;
        r.dmg_max = 3;
        r.dmg_lvl = per_lvl;
        r.exp_lvl = 100;
        r.skills[0] = skill(8, 0, 1, 10);
        assert_eq!(r.str_ + ((-r.str_lvl) >> 3), 33, "offer shift");
        let mut s = setup(vec![r], 0, 10);
        s.w.reqlevel.insert(8, 1);
        apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 2);
        let b = |st| s.w.base(s.merc, st);
        assert_eq!(b(stat::STRENGTH), 34);
        assert_eq!(b(stat::DEXTERITY), 43);
        assert_eq!(b(stat::MAXHP), 10240);
        assert_eq!(b(stat::HITPOINTS), 10240);
        assert_eq!(b(stat::SECONDARY_MINDAMAGE), 1);
        assert_eq!(b(stat::SECONDARY_MAXDAMAGE), 3);
        assert_eq!(b(stat::HPREGEN), 5);
        assert_eq!(b(stat::NEXTEXP), 3600);
        assert_eq!(b(stat::EXPERIENCE), 1200);
        // Inner Sight: (10·−1 >> 5) + 1 = 0 → not added.
        assert!(s.w.unit(s.merc).skills.is_empty());
    }
}

#[test]
fn level_stats_formulas_and_clamps() {
    // d = 2 (bracket 3, level 5); each column hand-checked.
    let mut r = row(100, 0, 1, 1, 3);
    r.str_ = 20;
    r.str_lvl = 9; // 20 + 18/8 = 22
    r.dex = 5;
    r.dex_lvl = 3; // 5 + 6/8 = 5 → 10
    r.hp = 100;
    r.hp_lvl = 7; // (14 + 100)·256 = 29184
    r.defense = 10;
    r.def_lvl = -8; // 10 − 16 → 0
    r.dmg_min = -3;
    r.dmg_max = 0;
    r.dmg_lvl = 4; // −3 + 1 → 0; 0 + 1 = 1
    r.ar = 30;
    r.ar_lvl = 11; // 52
    r.resist = 10;
    r.resist_lvl = 3; // 10 + 6/4 = 11
    let mut s = setup(vec![r], 0, 10);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 5);
    let b = |st| s.w.base(s.merc, st);
    assert_eq!(b(stat::STRENGTH), 22);
    assert_eq!(b(stat::DEXTERITY), 10);
    assert_eq!(b(stat::MAXHP), 29184);
    assert_eq!(b(stat::HITPOINTS), 29184);
    assert_eq!(b(stat::ARMORCLASS), 0);
    assert_eq!(b(stat::SECONDARY_MINDAMAGE), 0);
    assert_eq!(b(stat::SECONDARY_MAXDAMAGE), 1);
    assert_eq!(b(stat::TOHIT), 52);
    for st in [39, 41, 43, 45] {
        assert_eq!(b(st), 11);
    }
    assert_eq!(b(stat::HPREGEN), 14);
    // d = −1 (level 2): ÷4 truncates toward 0 (shift would give 9).
    let mut r = row(100, 0, 1, 1, 3);
    r.resist = 10;
    r.resist_lvl = 3;
    r.dmg_min = 5;
    r.dmg_lvl = 9; // 5 + (−9)/8 = 4 (shift: 3)
    r.exp_lvl = 1; // experience 0 < threshold(2) = 12 → written
    let mut s = setup(vec![r], 0, 10);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 2);
    assert_eq!(s.w.base(s.merc, stat::FIRERESIST), 10);
    assert_eq!(s.w.base(s.merc, stat::SECONDARY_MINDAMAGE), 4);
    // Order of the writes (step 4, 6, 7, 8 table order).
    let order: Vec<u16> =
        s.w.log
            .iter()
            .filter_map(|l| l.strip_prefix("set 2 "))
            .map(|l| l.split(' ').next().unwrap().parse().unwrap())
            .collect();
    assert_eq!(
        order,
        vec![12, 30, 13, 0, 2, 7, 6, 31, 23, 24, 19, 39, 41, 43, 45, 74]
    );
}

#[test]
fn apply_level_defaults_and_stops() {
    // merc None → the living hireling; level 0 → level + 1.
    let mut s = ice_l6(10);
    apply_level(&mut s.w, &s.t, &s.st, s.player, None, 0);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 7);
    assert_eq!(s.w.base(s.merc, stat::NEXTEXP), threshold(105, 8));
    // Level 98 = MaxLvl − 1 → nextexp 0; 97 → threshold(98).
    apply_level(&mut s.w, &s.t, &s.st, s.player, None, 97);
    assert_eq!(s.w.base(s.merc, stat::NEXTEXP), threshold(105, 98));
    apply_level(&mut s.w, &s.t, &s.st, s.player, None, 98);
    assert_eq!(s.w.base(s.merc, stat::NEXTEXP), 0);
    // Dead node only → no living hireling → nothing.
    let mut s = ice_l6(10);
    s.st.list_mut(s.player).nodes[0].dead = true;
    apply_level(&mut s.w, &s.t, &s.st, s.player, None, 9);
    assert!(s.w.sent.is_empty() && s.w.log.is_empty());
    // No row for the node's Id: speech and level, then stop.
    let mut s = ice_l6(10);
    s.st.list_mut(s.player).nodes[0].id = 9;
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 9);
    assert_eq!(s.w.sent.len(), 1);
    assert_eq!(s.w.log, vec!["set 2 12 9".to_string()]);
}

#[test]
fn skill_slots_loop() {
    let mut r = row(100, 0, 1, 1, 3);
    r.skills = [
        skill(10, 0, 1, 64), // d = 2: (128 >> 5) + 1 = 5
        skill(11, 15, 3, 0), // reqlevel above → skipped, loop goes on
        skill(12, 1, 40, 0), // → 32
        skill(13, 2, 1, 0),  // no record → skipped
        skill(14, 16, 1, 0), // Mode > 15 → stop
        skill(15, 0, 1, 0),
    ];
    let mut s = setup(vec![r], 0, 10);
    s.w.reqlevel.extend([(10, 1), (11, 6), (12, 5), (15, 1)]);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 5);
    let got: Vec<(u32, i32)> =
        s.w.unit(s.merc)
            .skills
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
    assert_eq!(got, vec![(10, 5), (12, 32)]);
    // Skill id < 1 or ≥ the skill count stops; negative level → 0 → none.
    for stop in [0, -1, 400] {
        let mut r = row(100, 0, 1, 1, 3);
        r.skills[0] = skill(stop, 0, 1, 0);
        r.skills[1] = skill(10, 0, 1, 0);
        let mut s = setup(vec![r], 0, 10);
        s.w.reqlevel.insert(10, 1);
        apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 5);
        assert!(s.w.unit(s.merc).skills.is_empty(), "stop at {stop}");
    }
    let mut r = row(100, 0, 1, 1, 3);
    r.skills[0] = skill(10, 0, -3, 0);
    let mut s = setup(vec![r], 0, 10);
    s.w.reqlevel.insert(10, 1);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), 5);
    assert!(s.w.unit(s.merc).skills.is_empty());
}

#[test]
fn add_experience_guards() {
    // merc level ≥ player level → nothing.
    let mut s = ice_l6(6);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 6, 100).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    // gain ≤ 0 → nothing.
    let mut s = ice_l6(10);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 6, 0).unwrap();
    // No pet node → nothing.
    s.st.list_mut(s.player).nodes[0].guid = 1;
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 6, 100).unwrap();
    assert!(s.w.sent.is_empty());
    // merc level 98 → nothing, even below the player's level.
    let mut s = ice_l6(99);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 98, 100).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    assert!(s.w.sent.is_empty());
}

#[test]
fn one_gain_raises_several_levels() {
    // 26460 + 2·29295 = 85050 = threshold(9) < threshold(10) = 115500.
    let mut s = ice_l6(8);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 6, 29295).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 85050);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 9, "past the player's 8");
    let sent = s.w.sent_to(s.player);
    // Delta and speech sent; the 14 stats queued on the merc.
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0][0], 0xA2);
    assert_eq!(sent[1][0], 0x27);
    let q = &s.w.queued[&s.merc];
    assert_eq!(q.len(), 14);
    assert_eq!(q[0], (12, 9));
    assert_eq!(s.w.log.last().unwrap(), "level_events 1 2");
    // One short of threshold(7): no level change, no events.
    let mut s = ice_l6(10);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 6, 7349).unwrap();
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 41158);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 6);
    assert_eq!(s.w.sent.len(), 1);
}

#[test]
fn level_97_jump_stops_at_98_and_reload_gives_99() {
    let mut r = row(100, 0, 1, 1, 3);
    r.exp_lvl = 105;
    let rows = vec![r];
    let mut s = setup(rows.clone(), 0, 99);
    s.w.set(s.merc, stat::LEVEL, 97);
    let start = threshold(105, 97);
    s.w.set(s.merc, stat::EXPERIENCE, start);
    add_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 97, 3_100_000).unwrap();
    let exp = s.w.base(s.merc, stat::EXPERIENCE);
    assert_eq!(exp, start + 6_200_000);
    assert!(exp as u32 > threshold(105, 99) as u32);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 98);
    assert_eq!(s.w.base(s.merc, stat::NEXTEXP), 0);
    // The delta ≥ 0xFFFF → 0xA0 with the old value.
    let first = &s.w.sent_to(s.player)[0];
    assert_eq!(
        (first[0], &first[6..]),
        (0xA0, &(start as u32).to_le_bytes()[..])
    );
    // Reload.
    assert_eq!(restore_level(&s.t, 0, exp as u32, true), Some(99));
    let mut s2 = setup(rows, 0, 99);
    assert!(
        restore_experience(&mut s2.w, &s2.t, &s2.st, s2.player, s2.merc, exp as u32, false)
            .unwrap()
    );
    assert_eq!(s2.w.base(s2.merc, stat::LEVEL), 99);
    assert_eq!(s2.w.base(s2.merc, stat::EXPERIENCE), exp);
    assert_eq!(s2.w.base(s2.merc, stat::NEXTEXP), 0);
    // The level speech is sent; stat 13 is queued on the merc.
    let sent = s2.w.sent_to(s2.player);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0][0], 0x27);
    assert_eq!(s2.w.queued[&s2.merc], [(13, exp as u32)]);
}

#[test]
fn restore_level_walk() {
    let t = tables(ice_105());
    assert_eq!(restore_level(&t, 1, 0, true), Some(1));
    assert_eq!(
        restore_level(&t, 1, threshold(105, 2) as u32 - 1, true),
        Some(1)
    );
    assert_eq!(
        restore_level(&t, 1, threshold(105, 2) as u32, true),
        Some(2)
    );
    assert_eq!(restore_level(&t, 1, 26460, true), Some(6));
    assert_eq!(restore_level(&t, 1, u32::MAX, true), Some(99));
    // No row of the Id (classic rows absent): the level step finds no
    // row.
    assert_eq!(restore_level(&t, 1, u32::MAX, false), None);
    // Exp/Lvl of the row at the current level: bracket 36 doubles it.
    let mut rows = ice_105();
    rows[1].exp_lvl = 210;
    let t = tables(rows);
    // threshold(36) with 105 reaches 36; threshold(37) uses row 36 (210).
    assert_eq!(
        restore_level(&t, 1, threshold(105, 37) as u32, true),
        Some(36)
    );
    assert_eq!(
        restore_level(&t, 1, threshold(210, 37) as u32, true),
        Some(37)
    );
    // A saved value below the unit's experience leaves it.
    let mut s = ice_l6(10);
    assert!(restore_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 100, false).unwrap());
    assert_eq!(s.w.base(s.merc, stat::EXPERIENCE), 26460);
    assert_eq!(s.w.base(s.merc, stat::LEVEL), 6);
}

#[test]
fn restore_level_null_row_frees_or_is_fatal() {
    // ≥ 0x5C loader: the unit is freed, no hireling, no error; the node
    // stays.
    let mut s = setup(ice_105(), 9, 10);
    assert!(!restore_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 100, false).unwrap());
    assert!(!s.w.units.contains_key(&s.merc));
    assert!(s.w.log.contains(&"free 2".to_string()));
    assert_eq!(s.st.list(s.player).unwrap().nodes.len(), 1);
    // Old loader: fatal (string 0x1660).
    let mut s = setup(ice_105(), 9, 10);
    assert_eq!(
        restore_experience(&mut s.w, &s.t, &s.st, s.player, s.merc, 100, true),
        Err(HirelingError::Fatal(0x1660))
    );
    assert!(s.w.units.contains_key(&s.merc));
}

#[test]
fn offer_uses_the_lowest_bracket_unit_its_own() {
    // Ice brackets 3 / 36 with different HP columns: at L 40 the offer
    // reads bracket 3 (d = 37), the unit bracket 36 (d = 4).
    let mut rows = ice_105();
    (rows[0].hp, rows[0].hp_lvl) = (45, 9);
    (rows[1].hp, rows[1].hp_lvl) = (390, 6);
    (rows[0].ar, rows[0].ar_lvl) = (10, 5);
    (rows[1].ar, rows[1].ar_lvl) = (450, 13);
    // L = (lo' mod 5) + player level − 5: pick the player level that
    // gives L 40 for this seed.
    let t = tables(rows.clone());
    let l10 = t
        .rows
        .offer(true, 10, 22_752_887, 0, 0)
        .expect("offer")
        .level;
    let o = t
        .rows
        .offer(true, 50 - l10, 22_752_887, 0, 0)
        .expect("offer");
    assert_eq!((o.row, o.level), (0, 40));
    assert_eq!(o.life, 45 + 9 * 37);
    let mut s = setup(rows, 1, 50);
    apply_level(&mut s.w, &s.t, &s.st, s.player, Some(s.merc), o.level);
    assert_eq!(s.w.base(s.merc, stat::MAXHP), (6 * 4 + 390) * 256);
    assert_ne!(s.w.base(s.merc, stat::MAXHP), o.life * 256);
    // Attack rating only on the unit (no offer word).
    assert_eq!(s.w.base(s.merc, stat::TOHIT), 450 + 13 * 4);
}

/// An `experience` `ExpRatio` column: `MaxLvl` 99, the `MaxLvl` row's
/// ratio 10, every level 1024 except the given ones.
fn ratios(set: &[(usize, u32)]) -> ExpRatios {
    let mut levels = vec![1024u32; 100];
    for &(l, r) in set {
        levels[l] = r;
    }
    ExpRatios {
        max_level: 99,
        max_row: 10,
        levels,
    }
}

/// The base-game `experience.txt` `ExpRatio` column: 1024 for
/// levels 0–69, −48 per level to 256 at 85, then the listed tail.
fn ratios_114d() -> ExpRatios {
    let mut levels = vec![1024u32; 70];
    for l in 70..=85u32 {
        levels.push(1024 - 48 * (l - 69));
    }
    levels.extend([192, 144, 108, 81, 61, 46, 35, 26, 20, 15, 11, 8, 6, 5]);
    assert_eq!(levels.len(), 100);
    assert_eq!(levels[85], 256);
    ExpRatios {
        max_level: 99,
        max_row: 10,
        levels,
    }
}

#[test]
fn exp_ratio_vectors() {
    let t = ratios_114d();
    assert_eq!(t.ratio(6), 1024);
    assert_eq!(t.ratio(80), 496);
    assert_eq!(t.ratio(97), 8);
    assert_eq!(t.ratio(0), 10);
    assert_eq!(t.ratio(-3), 10);
    assert_eq!(t.ratio(100), 0);
    // Test vectors: 229 / 2421 / 23432 / 1999872.
    assert_eq!(t.apply(229, 6), 229);
    assert_eq!(t.apply(5000, 80), 2421);
    assert_eq!(t.apply(3_000_000, 97), 23432);
    assert_eq!(t.apply(2_000_000, 6), 1_999_872);
    // e ≤ 1048575 with r = 1024: unchanged; e ≤ 0 unchanged.
    assert_eq!(t.apply(1_048_575, 6), 1_048_575);
    assert_eq!(t.apply(0, 6), 0);
    assert_eq!(t.apply(-7, 6), -7);
    // s − 1 ≥ 31 (s = 0 or s > 31): unchanged; no table: unchanged.
    let mut z = ratios(&[]);
    z.max_row = 0;
    assert_eq!(z.apply(5000, 6), 5000);
    z.max_row = 32;
    assert_eq!(z.apply(5000, 6), 5000);
    assert_eq!(ExpRatios::default().apply(5000, 6), 5000);
}
