//! Leap's arc and Whirlwind's spin in the play preview, headless on the
//! user's install (`real_rig`): the barbarian's own `skills` rows, her
//! `charstats` speeds. Expected values: known game runs
//! (Leap and Whirlwind in the Cold Plains);
//! some parts are still open.

use crate::real_rig;

use crate::app_support;

use d2_client::app::single_player;
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::BridgeResource;
use d2_client::rules::unit_composite::MotionRecord;
use d2_client::world_view::skill_motion::{leap_record, SkillMotion};
use d2_client::world_view::walk::PreviewWalk;
use real_rig::Rig;

/// The install's `skills.txt` `Id`s.
const LEAP: usize = 132;
const WHIRLWIND: usize = 151;

/// The barbarian's walk / run speeds (`charstats`).
fn speeds() -> Speeds {
    let c = single_player::new_character("barbarian", "Test").unwrap();
    single_player::walk_speeds(&app_support::game_data(), &c)
        .unwrap()
        .expect("charstats speeds")
}

fn rig_with_walk(skills: &[usize]) -> Rig {
    let mut r = Rig::new("barbarian", skills);
    r.leave_town();
    r.strengthen();
    r
}

fn offsets(r: &Rig) -> Option<(i32, i32)> {
    let w = r.app.world();
    let key = w.resource::<BridgeResource>().0.world().local()?.key;
    w.resource::<SkillMotion>().offsets().get(&key).copied()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_leap_draw_height_follows_the_timed_arc_frame_by_frame() {
    let mut r = rig_with_walk(&[LEAP]);
    r.select_right(LEAP);
    r.right_click_point(5, 0);
    // The expected curve: the spec's record for the leap's distance,
    // stepped once per server tick.
    let mut want: Vec<(i32, i32)> = Vec::new();
    let mut rec: Option<MotionRecord> = None;
    let mut got: Vec<(i32, i32)> = Vec::new();
    let mut d = 0;
    for _ in 0..60 {
        r.step(1);
        if d == 0 {
            // The distance of the request's point from the
            // client unit's own path cell (the walk prediction).
            let w = r.app.world().resource::<BridgeResource>().0.world();
            let u = w.local().unwrap();
            let cell = r.app.world().resource::<PreviewWalk>().predict.cell();
            if let (Some(q), Some(at)) = (u.last_mode_request, cell) {
                let (dx, dy) = (
                    (q.record[2] - i32::from(at.0)).abs(),
                    (q.record[3] - i32::from(at.1)).abs(),
                );
                d = dx.max(dy) + dx.min(dy) / 2;
            }
        }
        if let Some(o) = offsets(&r) {
            got.push(o);
        }
    }
    assert!(!got.is_empty(), "the leap started an arc ({})", r.errors());
    let mut m = leap_record(d, i32::from(speeds().run)).unwrap();
    while m.flags & 1 == 0 {
        m.update(false, None).unwrap();
        want.push(m.draw_offset());
        rec = Some(m);
    }
    assert!(rec.is_some());
    want.retain(|o| *o != (0, 0));
    got.retain(|o| *o != (0, 0));
    assert_eq!(got, want, "the draw offsets are the spec's curve");
    assert!(got.iter().any(|o| o.1 < 0), "the unit rises");
    assert_eq!(offsets(&r), None, "the arc ends on landing");
}

/// The whirl draws its sequence (`seqnum` 10, A1 0, 1, 2, 3, 3, 4, 5, 6)
/// then loops A1 3, 4, 5, 6, one per update
/// (`facts/client/anim/a1-cold-plains-whirlwind-bar.tsv`), and leaves
/// mode 18 at the end of its path.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn whirlwind_draws_its_sequence_while_it_spins() {
    let mut r = rig_with_walk(&[WHIRLWIND]);
    r.select_right(WHIRLWIND);
    r.right_click_point(8, 0);
    let mut frames = Vec::new();
    for _ in 0..80 {
        r.step(1);
        if let Some((_, mode, f)) = r.app.world().resource::<SkillMotion>().drawn() {
            assert_eq!(mode, 7, "drawn mode A1");
            if frames.last() != Some(&f) || frames.len() < 4 {
                frames.push(f);
            }
        }
    }
    assert!(
        frames.len() >= 8,
        "the whirl ran ({frames:?}; {})",
        r.errors()
    );
    assert_eq!(&frames[..4], &[0, 1, 2, 3], "{frames:?}");
    assert!(
        frames[4..]
            .windows(2)
            .all(|w| w[1] == if w[0] == 6 { 3 } else { w[0] + 1 }),
        "the loop over A1 3..6: {frames:?}"
    );
    assert_eq!(r.app.world().resource::<SkillMotion>().drawn(), None);
}
