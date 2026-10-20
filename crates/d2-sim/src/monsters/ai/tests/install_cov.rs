//! Installing an AI over a running one: the alternate function, the
//! raise re-install and the mode-end neutral start.

use super::*;

#[test]
fn only_the_listed_ais_have_an_alternate_and_it_runs_for_one_think() {
    // "When an alternate runs": BatDemon 29, FrogDemon 52, FetishShaman
    // 65, Diablo 51, Hireable 61, BaalCrab 135, BaalCrabClone 140,
    // UberBaal 145, UberDiablo 147 (the catalogue also carries
    // SandMaggot's, the alternate of the install vector).
    let with_alt: Vec<usize> = AI_TABLE
        .iter()
        .enumerate()
        .filter(|(_, r)| r.alt != 0)
        .map(|(i, _)| i)
        .collect();
    for i in [29, 52, 65, 51, 61, 135, 140, 145, 147] {
        assert!(with_alt.contains(&i), "AI {i} has an alternate");
    }
    // Step 3 looks at the CURRENT state's record and ignores the new
    // state's value: install state 11 over a running base AI (no
    // switchai) → the alternate runs once, the control state is the new
    // one, nothing is reset.
    let mut w = World::new(monstats(15, [0; 5], 15));
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().params = [1, 2, 3];
    w.with(|g, cx| install(g, cx, mon, 11));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.special_state, c.function), (11, 0x005F_1750));
    assert_eq!(c.params, [1, 2, 3], "no reset, no init");
    // The alternate then re-installs the control's state (now 11) through
    // step 4: the record of state 11 for a class without switchai is the
    // base record, so the base think comes back.
    let p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| run_function(g, cx, 0x005F_1750, mon, &p));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.special_state, c.function), (11, 0x005F_1800));
    // A second install now (function = think again, alternate nonzero)
    // would go through step 3 again: one think per install.
    w.with(|g, cx| install(g, cx, mon, 0));
    assert_eq!(w.store.control(mon).unwrap().function, 0x005F_1750);
}

#[test]
fn raise_reinstall_with_an_alternate_takes_step_3() {
    // The first install (monster creation, state 0) left the record's
    // think as the control function; the raise installs again: state
    // stays 0, the function becomes the alternate, params stay.
    let mut w = World::new(monstats(15, [0; 5], 15));
    let mon = w.mon;
    assert_eq!(w.store.control(mon).unwrap().function, 0x005F_1800);
    w.store.control_mut(mon).unwrap().params = [4, 5, 6];
    w.with(|g, cx| install(g, cx, mon, 0));
    let c = w.store.control(mon).unwrap();
    assert_eq!(
        (c.special_state, c.function, c.params),
        (0, 0x005F_1750, [4, 5, 6])
    );
    // The first think runs the alternate, which re-installs state 0
    // through step 4: think installed, params 0 and 1 cleared.
    let p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| run_function(g, cx, 0x005F_1750, mon, &p));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.special_state, c.function), (0, 0x005F_1800));
    assert_eq!(c.params[..2], [0, 0]);
}

#[test]
fn raise_reinstall_without_an_alternate_runs_step_4_again() {
    // FoulCrowNest (43): init sets param 0 := frame, param 1 := 0; no
    // alternate. The second install clears params and commands and runs
    // the init again (its frame is the new one).
    let mut w = World::new(monstats(43, [0; 5], 15));
    let mon = w.mon;
    w.game.frame = 77;
    w.store.control_mut(mon).unwrap().function = 0;
    w.with(|g, cx| install(g, cx, mon, 0));
    assert_eq!(w.store.control(mon).unwrap().params, [77, 0, 0]);
    {
        let c = w.store.control_mut(mon).unwrap();
        c.params = [77, 9, 9];
        c.commands = vec![AiCommand {
            params: [4, 1, 0, 0, 0],
        }];
    }
    w.game.frame = 99;
    w.with(|g, cx| install(g, cx, mon, 0));
    let c = w.store.control(mon).unwrap();
    assert_eq!(c.params, [99, 0, 0], "cleared, init ran again");
    assert!(c.commands.is_empty(), "commands freed");
    assert_eq!(c.function, AI_TABLE[43].think);
    // A class without init: params cleared, function := think.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().params = [1, 2, 3];
    w.with(|g, cx| install(g, cx, mon, 0));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.params, c.function), ([0, 0, 0], AI_TABLE[3].think));
}

#[test]
fn ai_functions_leave_the_next_think_to_the_mode_end() {
    // A started attack: the AI function schedules no think itself (the
    // mode's end does).
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    w.run(true, 2);
    assert_eq!(w.fake.modes().len(), 1, "an attack was requested");
    assert!(w.thinks().is_empty(), "no think from the AI function");
    // The mode ends and the unit returns to neutral: the neutral start
    // adds the think `aidel` frames later.
    let mon = w.mon;
    w.with(|g, cx| neutral_mode_start(g, cx, mon));
    assert_eq!(w.thinks(), [115]);
}
