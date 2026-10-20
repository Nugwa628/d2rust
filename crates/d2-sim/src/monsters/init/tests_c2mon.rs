//! Coverage claims added by the c2-monsters session.

use super::*;

fn boss_with_teleport() -> (Fake, UnitId) {
    let mut t = Tables::new(vec![velocity_mon(0, 1)]);
    t.ids.monteleport = Some(184);
    let mut f = fake_with(t);
    let u = f.monster(0, 1);
    (f, u)
}

#[test]
fn teleport_assigns_skill_184_level_1_and_mode_4() {
    let (mut f, u) = boss_with_teleport();
    let cx = f.cx;
    f.log.clear();
    run_umod_init(&cx, &mut f, u, 26, true);
    assert_eq!(f.log, vec!["skill 184 1 Some(4)".to_string()]);
}
