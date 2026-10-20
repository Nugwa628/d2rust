//! Expected values of the leap and whirl motion.

use super::*;

/// The barbarian's charstats speeds (the leap's `s` 9 and the whirl's
/// step 0x6000 in the facts).
const BARBARIAN: Speeds = Speeds { walk: 6, run: 9 };

/// `skills.txt` `seqnum` of Leap and Whirlwind.
const SEQ_LEAP: u8 = 13;

/// `plrmode` rows of the drawn modes.
const MODE_S1: u32 = 13;

#[test]
fn gravity_follows_the_two_branches_and_the_floor() {
    assert_eq!(leap_gravity(10), 5762);
    assert_eq!(leap_gravity(26), 8462 - 270 * 26);
    assert_eq!(leap_gravity(27), 500);
    assert_eq!(leap_gravity(100), 500);
}

#[test]
fn timed_arc_starts_at_the_height_and_lands_after_n_updates() {
    let mut r = MotionRecord::default();
    timed_arc(&mut r, 0, 4);
    assert_eq!(r.acc[2], -0x1000);
    assert_eq!(r.ticks_left, 4);
    // vz = -az·n²/2 / n = 0x1000·16/2/4.
    assert_eq!(r.vel[2], 0x1000 * 4 / 2);
    let mut rec = r;
    let mut oz = Vec::new();
    for _ in 0..6 {
        rec.update(false, None).unwrap();
        oz.push(rec.offset[2]);
    }
    assert_eq!(rec.flags & motion::DONE, motion::DONE);
    assert!(oz.iter().any(|&z| z < 0), "the unit rises: {oz:?}");
}

/// The recorded leaps (recording note, `a1-cold-plains-leap-bar.tsv`):
/// s 9, d 8 → g 6302, n 13, vz 40963; d 11 → g 5492, n 18, vz 49428.
#[test]
fn leap_record_matches_the_recorded_n_g_and_vz() {
    assert_eq!(leap_record(10, 0), None);
    for (d, n, g, vz) in [(8, 13, 6302, 40963), (11, 18, 5492, 49428)] {
        let r = leap_record(d, 9).unwrap();
        assert_eq!(r.ticks_left, n, "d {d}");
        assert_eq!(r.acc[2], -g, "d {d}");
        assert_eq!(r.vel[2], vz, "d {d}");
    }
}

/// `a1-cold-plains-leap-bar.tsv`: the record's ticks left and `oz` count
/// down one per update (13 → 0), done the update after 0.
#[test]
fn whirl_updates_count_the_full_steps_before_the_end() {
    assert_eq!(whirl_updates(8 << 16, 6), 21);
    assert_eq!(whirl_updates(14 << 16, 6), 37);
    // d 10: a = 2 + ⌈26.7⌉ = 29 → N = 32.
    assert_eq!(whirl_updates(10 << 16, 6), 29);
    // d 9: a = 2 + 24 = 26 → N = 28.
    assert_eq!(whirl_updates(9 << 16, 6), 25);
    // d 0: a = 2 → N = 8.
    assert_eq!(whirl_updates(0, 6), 5);
    assert_eq!(whirl_updates(8 << 16, 0), 1);
}

/// A diagonal whirl's path is its Euclidean length
/// (measured: (10, 5) arrives on the 21st step of
/// 35132, N 24 from the request; the metric 12 would take 23 steps).
#[test]
fn a_diagonal_whirl_runs_its_euclidean_length() {
    let len = path_length((5143, 4263), (5153, 4268));
    assert_eq!(len, 732_715);
    assert_eq!(len, path_length((0, 0), (-10, -5)));
    // At walk 6 (step 0x6000): a = 2 + ⌈29.8⌉ = 32 → N = 32; the metric
    // (12 << 16) would give a = 34 → N = 36.
    assert_eq!(whirl_updates(len, 6), 29);
    assert_eq!(whirl_updates(12 << 16, 6), 33);
}

/// The resource: a Leap request starts mode 18 at frame 0, the arc's
/// offsets appear from the update after frame 5, and the run ends.
#[test]
fn skill_motion_runs_a_requested_leap() {
    use crate::bridge::world::ClientUnit;
    let key = UnitKey::new(0, 1);
    let mut world = ClientWorld::default();
    let mut u = ClientUnit::new(key);
    u.position = Some((10, 10));
    world.units.insert(key, u);
    world.local_player = Some(key);
    let row = SkillRow {
        srvdofunc: LEAP,
        seqnum: SEQ_LEAP,
        ..SkillRow::default()
    };
    let rows = |id: u16| (id == 132).then_some(row);
    let mut m = SkillMotion::default();
    world.server_ticks = 100;
    m.frame(&world, rows, Some(BARBARIAN), None);
    assert_eq!(m.drawn(), None);
    world.units.get_mut(&key).unwrap().last_path_request = Some((
        ModeRequest {
            code: 0x15,
            record: [132, 0, 18, 10, 0, 0, 0],
        },
        1,
    ));
    m.frame(&world, rows, Some(BARBARIAN), None);
    assert_eq!(m.drawn(), Some((key, MODE_S1, 0)));
    let mut drawn = Vec::new();
    let mut lifted = Vec::new();
    for _ in 0..40 {
        world.server_ticks += 1;
        m.frame(&world, rows, Some(BARBARIAN), None);
        drawn.push(m.drawn().map(|d| d.2));
        lifted.push(m.offsets().get(&key).map(|o| o.1));
        // The shadow takes half the height on
        // both axes where the body takes all of it in y.
        let (body, shadow) = (
            m.offsets().get(&key).copied(),
            m.shadow_offsets().get(&key).copied(),
        );
        if let (Some(b), Some(sh)) = (body, shadow) {
            let r = *m.runs.get(&key).unwrap().record().unwrap();
            let [ox, oy, oz] = r.offset;
            assert_eq!(b, (ox, oy + oz));
            assert_eq!(sh, (ox + oz / 2, oy + oz / 2));
        }
    }
    assert_eq!(&drawn[..6], &[1, 2, 3, 4, 5, 6].map(Some));
    assert_eq!(lifted[4], None, "no record before frame 5");
    assert_eq!(lifted[5], Some(0), "made at frame 5, first drawn unstepped");
    assert_eq!(lifted[6], Some(-20));
    assert_eq!(drawn.last(), Some(&None), "the mode ended");
    assert!(m.offsets().is_empty());
    assert!(m.shadow_offsets().is_empty());
}

/// A 0x13 soft hit in the cast's tick, and one later in the run, set no
/// mode and reset no path in the base game: the Leap starts and runs on
/// as without them.
#[test]
fn a_soft_hit_neither_hides_nor_ends_a_leap() {
    use crate::bridge::modes::mode_request;
    use crate::bridge::world::ClientUnit;
    let key = UnitKey::new(0, 1);
    let mut world = ClientWorld::default();
    let mut u = ClientUnit::new(key);
    u.position = Some((10, 10));
    world.units.insert(key, u);
    world.local_player = Some(key);
    let row = SkillRow {
        srvdofunc: LEAP,
        seqnum: SEQ_LEAP,
        ..SkillRow::default()
    };
    let rows = |id: u16| (id == 132).then_some(row);
    let inputs = crate::bridge::world::ModelInputs::default();
    let out = crate::bridge::output::Outputs::default();
    let mut m = SkillMotion::default();
    world.server_ticks = 100;
    m.frame(&world, rows, Some(BARBARIAN), None);
    // The cast, then the soft hit, in one tick.
    mode_request(
        &mut world,
        &inputs,
        key,
        0x15,
        [132, 0, 18, 10, 0, 0, 0],
        &out,
    )
    .unwrap();
    mode_request(&mut world, &inputs, key, 0x13, [0, 0, 0, 0, 0, 0, 0], &out).unwrap();
    assert_eq!(world.units[&key].last_mode_request.unwrap().code, 0x13);
    m.frame(&world, rows, Some(BARBARIAN), None);
    assert_eq!(m.drawn(), Some((key, MODE_S1, 0)));
    world.server_ticks += 1;
    m.frame(&world, rows, Some(BARBARIAN), None);
    // Another soft hit during the run.
    mode_request(&mut world, &inputs, key, 0x13, [0, 0, 0, 0, 0, 0, 0], &out).unwrap();
    world.server_ticks += 1;
    m.frame(&world, rows, Some(BARBARIAN), None);
    assert_eq!(m.drawn(), Some((key, MODE_S1, 2)));
}
