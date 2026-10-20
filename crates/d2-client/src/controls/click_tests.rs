//! Tests for the world-click dispatcher (`super::click`), run against a fake world: walk and run
//! codes, the walk clamp and its held re-send gap, NPC / monster / object interaction, press /
//! held / release filtering, the panel areas that swallow right clicks, and the input reset.

use super::click::*;
use crate::bridge::world::UnitKey;

const P: UnitKey = UnitKey {
    unit_type: 0,
    guid: 1,
};

/// A world with one local player at (100, 100), mode 1, left skill
/// Attack (mode 1), the ground under the mouse at the mapped point.
struct Fake {
    hover: Option<UnitKey>,
    ground: (i32, i32),
    p_mode: u32,
    left: Option<SkillRef>,
    right: Option<SkillRef>,
    rows: Vec<(u16, SkillRowFacts)>,
    rng: u8,
    can_act: bool,
    cursor_item: Option<u32>,
    open_mode: u8,
    counter: u32,
    units: Vec<(UnitKey, (i32, i32), u32)>,
    npc: (bool, bool),
    dist: i32,
    menu: bool,
    /// Every other unit is hostile.
    hostile: bool,
    dead: Vec<UnitKey>,
}

impl Default for Fake {
    fn default() -> Self {
        Fake {
            hover: None,
            ground: (101, 100),
            p_mode: 1,
            left: Some(SkillRef { id: 0, mode: 1 }),
            right: None,
            rows: vec![(0, SkillRowFacts::default())],
            rng: range::H2H,
            can_act: true,
            cursor_item: None,
            open_mode: 0,
            counter: 100,
            units: Vec::new(),
            npc: (false, false),
            dist: 8,
            menu: false,
            hostile: false,
            dead: Vec::new(),
        }
    }
}

impl ClickWorld for Fake {
    fn in_game(&self) -> bool {
        true
    }
    fn player(&self) -> Option<UnitKey> {
        Some(P)
    }
    fn game_menu_open(&self) -> bool {
        self.menu
    }
    fn open_mode(&self) -> u8 {
        self.open_mode
    }
    fn frame_size(&self) -> (i32, i32) {
        (800, 600)
    }
    fn right_panel_bottom(&self) -> i32 {
        432
    }
    fn skill_y_limit(&self) -> i32 {
        560
    }
    fn mouse(&self) -> (i32, i32) {
        (400, 300)
    }
    fn hover(&self) -> Option<UnitKey> {
        self.hover
    }
    fn to_world(&self, _: i32, _: i32) -> (i32, i32) {
        self.ground
    }
    fn position(&self, u: UnitKey) -> Option<(i32, i32)> {
        if u == P {
            return Some((100, 100));
        }
        self.units.iter().find(|x| x.0 == u).map(|x| x.1)
    }
    fn mode(&self, u: UnitKey) -> u32 {
        if u == P {
            self.p_mode
        } else {
            self.units.iter().find(|x| x.0 == u).map_or(0, |x| x.2)
        }
    }
    fn class(&self, _: UnitKey) -> u32 {
        0
    }
    fn stamina(&self) -> i32 {
        1
    }
    fn can_act(&self, _: Option<SkillRef>) -> bool {
        self.can_act
    }
    fn left_skill(&self) -> Option<SkillRef> {
        self.left
    }
    fn right_skill(&self) -> Option<SkillRef> {
        self.right
    }
    fn attack_skill(&self) -> Option<SkillRef> {
        Some(SkillRef { id: 0, mode: 1 })
    }
    fn skill_row(&self, id: u16) -> Option<SkillRowFacts> {
        self.rows.iter().find(|r| r.0 == id).map(|r| r.1)
    }
    fn range(&self, _: SkillRef) -> u8 {
        self.rng
    }
    fn use_state(&self, _: SkillRef) -> u32 {
        0
    }
    fn refusal_sound(&self, _: u32) -> Option<u16> {
        Some(0)
    }
    fn cursor_state(&self) -> u32 {
        0
    }
    fn cursor_unit(&self) -> Option<u32> {
        None
    }
    fn cursor_item(&self) -> Option<u32> {
        self.cursor_item
    }
    fn has_hireling(&self) -> bool {
        false
    }
    fn cursor_unit_mode_record(&self) -> Option<i32> {
        None
    }
    fn in_town(&self, _: UnitKey) -> bool {
        false
    }
    fn is_dead(&self, u: UnitKey) -> bool {
        self.dead.contains(&u)
    }
    fn selectable(&self, _: UnitKey) -> bool {
        true
    }
    fn hostile(&self, _: UnitKey) -> bool {
        self.hostile
    }
    fn monster_npc_interact(&self, _: UnitKey) -> (bool, bool) {
        self.npc
    }
    fn object_has_row(&self, _: UnitKey) -> bool {
        true
    }
    fn distance(&self, _: UnitKey) -> i32 {
        self.dist
    }
    fn path_distance(&self, _: UnitKey) -> i32 {
        self.dist
    }
    fn clear(&self, _: UnitKey) -> bool {
        true
    }
    fn at_object(&self, _: UnitKey) -> bool {
        false
    }
    fn just_portaled(&self) -> bool {
        false
    }
    fn melee_range(&self, _: UnitKey) -> bool {
        false
    }
    fn moving(&self) -> bool {
        false
    }
    fn current_target(&self) -> Option<UnitKey> {
        None
    }
    fn speed_changed(&self, _: bool) -> bool {
        false
    }
    fn pending(&self) -> Option<Pending> {
        None
    }
    fn approach_range(&self, _: SkillRef) -> i32 {
        0
    }
    fn walk_path(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        Some((x, y))
    }
    fn update_counter(&self) -> u32 {
        self.counter
    }
    fn now_ms(&self) -> u32 {
        0
    }
    fn direction(&self) -> u8 {
        0
    }
    fn search_enemy_near(
        &self,
        _: SkillRowFacts,
        _: SkillRef,
        _: Option<UnitKey>,
        _: &mut i32,
        _: &mut i32,
    ) -> Option<UnitKey> {
        None
    }
    fn search_enemy_xy(&self, _: SkillRowFacts) -> Option<UnitKey> {
        None
    }
    fn enemy_near(&self, x: i32, y: i32, radius: i32) -> Option<UnitKey> {
        self.units
            .iter()
            .filter(|(u, ..)| self.hostile && !self.dead.contains(u))
            .map(|&(u, (ux, uy), _)| ((ux - x).pow(2) + (uy - y).pow(2), u))
            .filter(|&(d, _)| d <= radius * radius)
            .min()
            .map(|(_, u)| u)
    }
    fn search_open_xy(
        &self,
        _: Option<UnitKey>,
        _: &mut i32,
        _: &mut i32,
    ) -> Option<Option<UnitKey>> {
        None
    }
}

fn codes(out: &[ClickOut]) -> Vec<(u8, u32, u32)> {
    out.iter()
        .filter_map(|o| match *o {
            ClickOut::Code { code, a, b } => Some((code, a, b)),
            _ => None,
        })
        .collect()
}

fn run(w: &Fake, st: &mut ClickState, kind: Kind, mods: u32) -> Vec<ClickOut> {
    let mut out = Vec::new();
    click(st, w, kind, Some((400, 300)), mods, &mut out);
    out
}

#[test]
fn left_down_on_ground_is_a_walk_not_0x05() {
    let w = Fake {
        ground: (110, 104),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    let out = run(&w, &mut st, Kind::LeftDown, 0);
    assert_eq!(codes(&out), [(1, 110, 104)]);
    assert_eq!(code_bytes(1, 110, 104), Some(vec![1, 110, 0, 104, 0]));
    assert!(st.left_held);
    // Run (the run modifier with stamina left): code 3.
    let mut st = ClickState::default();
    assert_eq!(
        codes(&run(&w, &mut st, Kind::LeftDown, mods::RUN)),
        [(3, 110, 104)]
    );
}

#[test]
fn code_bytes_point_and_unit_layouts() {
    for c in [1u8, 3, 5, 8, 0x0C, 0x0F] {
        assert_eq!(code_bytes(c, 0x1234, 0x5678).unwrap().len(), 5);
    }
    for c in [2u8, 4, 6, 7, 9, 0x0A, 0x0D, 0x0E, 0x10, 0x11] {
        assert_eq!(
            code_bytes(c, 1, 0xAABBCCDD),
            Some(vec![c, 1, 0, 0, 0, 0xDD, 0xCC, 0xBB, 0xAA])
        );
    }
    assert_eq!(code_bytes(0x13, 1, 2), None);
    assert_eq!(code_bytes(0x0B, 1, 2), None);
}

#[test]
fn walk_clamp_vectors() {
    // One subtile east, no run: the distance 1 is under the minimum step 3, so the walk is
    // scaled to 3 subtiles.
    let w = Fake::default();
    let mut st = ClickState::default();
    assert_eq!(codes(&run(&w, &mut st, Kind::LeftDown, 0)), [(1, 103, 100)]);
    // Press with run: the minimum step is 5.
    let mut st = ClickState::default();
    assert_eq!(
        codes(&run(&w, &mut st, Kind::LeftDown, mods::RUN)),
        [(3, 105, 100)]
    );
    // The clicked point is the player's own: the nudge moves it two subtiles ahead (direction 0
    // gives (0, +2)), then the clamp scales that distance 1 up to the step 3, so dy is 6.
    let w = Fake {
        ground: (100, 100),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert_eq!(codes(&run(&w, &mut st, Kind::LeftDown, 0)), [(1, 100, 106)]);
    // Too far (a distance of 0x100 or more): nothing.
    let w = Fake {
        ground: (100 + 0x100, 100),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert!(codes(&run(&w, &mut st, Kind::LeftDown, 0)).is_empty());
}

#[test]
fn held_walk_resends_at_most_every_7_updates() {
    let mut w = Fake {
        ground: (110, 100),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    run(&w, &mut st, Kind::LeftDown, 0);
    st.end_pass();
    assert_eq!(st.last_walk, 100);
    // The player walks (mode 2); 3 updates later: nothing.
    w.p_mode = 2;
    w.counter = 103;
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(codes(&out).is_empty());
    st.end_pass();
    // 7 updates later: the walk is sent again.
    w.counter = 107;
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert_eq!(codes(&out), [(1, 110, 100)]);
    // The per-pass latch: a second positioned dispatch in the same pass does nothing.
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(out.is_empty());
}

// A second button can stay down from walking, but its repeat must not
// replace the button the player subsequently pressed to cast.
#[test]
fn the_latest_pressed_button_owns_held_repeats() {
    for left_cast in [false, true] {
        for target in [None, Some(UnitKey::new(1, 44))] {
            let mut w = Fake {
                ground: (115, 100),
                right: Some(SkillRef { id: 47, mode: 10 }),
                rng: range::NONE,
                rows: vec![
                    (0, SkillRowFacts::default()),
                    (47, SkillRowFacts::default()),
                ],
                units: vec![(UnitKey::new(1, 44), (115, 100), 1)],
                hostile: true,
                ..Fake::default()
            };
            if left_cast {
                w.left = w.right;
            }
            let mut st = ClickState::default();
            run(
                &w,
                &mut st,
                if left_cast {
                    Kind::RightDown
                } else {
                    Kind::LeftDown
                },
                0,
            );
            st.end_pass();
            w.hover = target;
            run(
                &w,
                &mut st,
                if left_cast {
                    Kind::LeftDown
                } else {
                    Kind::RightDown
                },
                mods::STAND_STILL,
            );
            st.end_pass();
            // The previous cast just ended; the held button must send a
            // skill repeat, never the other button's walk/attack intent.
            w.counter += 10;
            let mut out = Vec::new();
            held_repeat(&mut st, &w, mods::STAND_STILL, &mut out);
            let expected = match (left_cast, target.is_some()) {
                (false, false) => (0x0f, 115, 100),
                (false, true) => (0x11, 1, 44),
                (true, false) => (0x08, 115, 100),
                (true, true) => (0x0a, 1, 44),
            };
            assert_eq!(
                codes(&out),
                [expected],
                "left_cast={left_cast}, target={target:?}"
            );
        }
    }
}

// A right-click cast while the left button is held to walk: once the
// right button is up, the held left button keeps walking.
#[test]
fn a_cast_does_not_end_a_held_walk() {
    let mut w = Fake {
        ground: (110, 100),
        right: Some(SkillRef { id: 0, mode: 1 }),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    run(&w, &mut st, Kind::LeftDown, 0);
    st.end_pass();
    w.p_mode = 2;
    // The cast: right down, then right up while left stays held.
    w.counter = 101;
    run(&w, &mut st, Kind::RightDown, 0);
    st.end_pass();
    run(&w, &mut st, Kind::RightUp, 0);
    st.end_pass();
    assert!(st.left_held && !st.right_held);
    assert_ne!(st.press, 0, "the held left button's press stays");
    // The held repeat walks again once the re-send gap has passed.
    w.counter = 110;
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert_eq!(codes(&out), [(1, 110, 100)]);
    // Releasing the left button too ends the press.
    st.end_pass();
    run(&w, &mut st, Kind::LeftUp, 0);
    assert_eq!(st.press, 0);
}

#[test]
fn left_down_on_an_interact_npc_far_away() {
    let npc = UnitKey {
        unit_type: 1,
        guid: 0x44,
    };
    let w = Fake {
        hover: Some(npc),
        units: vec![(npc, (108, 100), 1)],
        npc: (true, true),
        dist: 8,
        ..Fake::default()
    };
    let mut st = ClickState::default();
    let out = run(&w, &mut st, Kind::LeftDown, 0);
    let mut m59 = vec![0x59, 1, 0, 0, 0, 0x44, 0, 0, 0];
    m59.extend_from_slice(&108u32.to_le_bytes());
    m59.extend_from_slice(&100u32.to_le_bytes());
    let tail: Vec<ClickOut> = out
        .into_iter()
        .filter(|o| !matches!(o, ClickOut::HoverCall(_)))
        .collect();
    assert_eq!(
        tail,
        [
            ClickOut::Send(m59),
            ClickOut::NpcHold(npc),
            ClickOut::Code {
                code: 2,
                a: 1,
                b: 0x44
            },
            ClickOut::Pend(Some(Pending {
                code: CODE_INTERACT,
                target: npc
            })),
        ]
    );
    // Within reach 2: the interact code 0x13.
    let w = Fake { dist: 2, ..w };
    let mut st = ClickState::default();
    let out = run(&w, &mut st, Kind::LeftDown, 0);
    assert_eq!(codes(&out), [(CODE_INTERACT, 1, 0x44)]);
}

#[test]
fn left_down_on_a_non_interact_monster_walks_to_it() {
    let m = UnitKey {
        unit_type: 1,
        guid: 9,
    };
    let w = Fake {
        hover: Some(m),
        units: vec![(m, (104, 100), 1)],
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert_eq!(codes(&run(&w, &mut st, Kind::LeftDown, 0)), [(2, 1, 9)]);
    // Held: a held click on a monster here sends nothing (a monster is only acted on at the
    // press); the filter lets it through only while the player is not walking.
    let mut out = Vec::new();
    st.end_pass();
    click(&mut st, &w, Kind::LeftHeld, None, 0, &mut out);
    assert!(codes(&out).is_empty());
}

#[test]
fn right_held_on_ground_press_then_hold_codes() {
    let w = Fake {
        right: Some(SkillRef { id: 36, mode: 1 }),
        rows: vec![(36, SkillRowFacts::default())],
        rng: range::RNG,
        ground: (120, 100),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    let out = run(&w, &mut st, Kind::RightDown, 0);
    assert_eq!(codes(&out), [(0x0C, 120, 100)]);
    assert!(st.right_held);
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert_eq!(codes(&out), [(0x0F, 120, 100)]);
    // Stand Still + left on ground: the skill is used at the point (code 0x05).
    let w = Fake {
        ground: (120, 100),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert_eq!(
        codes(&run(&w, &mut st, Kind::LeftDown, mods::STAND_STILL)),
        [(0x05, 120, 100)]
    );
}

#[test]
fn right_down_in_an_open_panel_half_is_not_dispatched() {
    let w = Fake {
        open_mode: 2,
        right: Some(SkillRef { id: 0, mode: 1 }),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    let mut out = Vec::new();
    assert!(!click(
        &mut st,
        &w,
        Kind::RightDown,
        Some((100, 200)),
        0,
        &mut out
    ));
    assert!(out.is_empty() && !st.latch);
    // Open mode 3: ignored anywhere.
    let w = Fake { open_mode: 3, ..w };
    assert!(!click(
        &mut st,
        &w,
        Kind::RightDown,
        Some((600, 200)),
        0,
        &mut out
    ));
    assert!(out.is_empty());
}

#[test]
fn left_down_with_a_cursor_item_drops_it() {
    let w = Fake {
        can_act: false,
        cursor_item: Some(0x77),
        ..Fake::default()
    };
    let mut st = ClickState::default();
    let out = run(&w, &mut st, Kind::LeftDown, 0);
    assert!(out.contains(&ClickOut::Send(vec![0x17, 0x77, 0, 0, 0])));
    assert!(!st.left_held);
}

#[test]
fn dispatcher_latch_menu_and_filter() {
    let w = Fake {
        menu: true,
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert!(run(&w, &mut st, Kind::LeftDown, 0).is_empty());
    // A release clears the press latch and acts on nothing; a left release then walks to the
    // release point only when the player walks (mode 2) and is not busy.
    let w = Fake {
        p_mode: 2,
        ground: (105, 100),
        ..Fake::default()
    };
    let mut st = ClickState {
        press: 1,
        ..ClickState::default()
    };
    let out = run(&w, &mut st, Kind::LeftUp, 0);
    assert_eq!(st.press, 0);
    assert_eq!(codes(&out), [(1, 105, 100)]);
    // Standing (mode 1): the release sends nothing.
    let w = Fake {
        p_mode: 1,
        ..Fake::default()
    };
    let mut st = ClickState::default();
    assert!(codes(&run(&w, &mut st, Kind::LeftUp, 0)).is_empty());
    // Held without a press: nothing.
    let mut st = ClickState::default();
    let mut out = Vec::new();
    click(&mut st, &w, Kind::LeftHeld, None, 0, &mut out);
    assert!(codes(&out).is_empty());
}

#[test]
fn nudge_table() {
    assert_eq!(nudge((10, 10), 0), (10, 12));
    assert_eq!(nudge((10, 10), 8), (9, 11));
    assert_eq!(nudge((10, 10), 16), (8, 10));
    assert_eq!(nudge((10, 10), 63), (11, 11));
}

// The input reset clears both held flags, so the next pass runs no held repeat.
#[test]
fn input_reset_clears_both_held_buttons() {
    let mut st = ClickState {
        left_held: true,
        right_held: true,
        latch: true,
        ..ClickState::default()
    };
    st.input_reset();
    assert!(!st.left_held && !st.right_held);
    assert!(st.latch, "the per-pass latch is not the reset's");
}

#[test]
fn non_melee_skills_cast_on_distant_monsters_without_a_walk_or_pending_approach() {
    let monster = UnitKey {
        unit_type: 1,
        guid: 9,
    };
    for rng in [range::NONE, range::RNG, range::LOC] {
        for srvdofunc in [0, SRVDOFUNC_APPROACH] {
            for (kind, expected) in [(Kind::LeftDown, 6), (Kind::RightDown, 0xD)] {
                let skill = SkillRef { id: 47, mode: 10 };
                let w = Fake {
                    hover: Some(monster),
                    units: vec![(monster, (120, 100), 1)],
                    hostile: true,
                    left: Some(skill),
                    right: Some(skill),
                    rng,
                    rows: vec![(
                        47,
                        SkillRowFacts {
                            range: rng,
                            srvdofunc,
                            ..Default::default()
                        },
                    )],
                    ..Fake::default()
                };
                let out = run(&w, &mut ClickState::default(), kind, 0);
                assert_eq!(
                    codes(&out),
                    [(expected, 1, 9)],
                    "range {rng}, do {srvdofunc}, {kind:?}"
                );
                assert!(!out.iter().any(|o| matches!(o, ClickOut::Pend(Some(_)))));
            }
        }
    }
}
/// A hostile monster under the cursor at (101, 100), another at (104, 101).
fn fight() -> (Fake, UnitKey, UnitKey) {
    let m = UnitKey {
        unit_type: 1,
        guid: 9,
    };
    let n = UnitKey {
        unit_type: 1,
        guid: 10,
    };
    let w = Fake {
        hover: Some(m),
        units: vec![(m, (101, 100), 1), (n, (104, 101), 1)],
        hostile: true,
        ..Fake::default()
    };
    (w, m, n)
}

fn on_unit(out: &[ClickOut], u: UnitKey) -> bool {
    codes(out)
        .iter()
        .any(|&(code, a, b)| !is_point_code(code) && a == u32::from(u.unit_type) && b == u.guid)
}

#[test]
fn a_held_attack_goes_on_to_the_next_enemy_after_a_kill() {
    let (mut w, m, n) = fight();
    let mut st = ClickState::default();
    assert!(on_unit(&run(&w, &mut st, Kind::LeftDown, 0), m));
    assert!(st.fighting[0]);
    // The enemy dies under the cursor: the repeat attacks the next one, it does not walk.
    w.dead.push(m);
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(on_unit(&out, n), "{out:?}");
    assert!(!codes(&out).iter().any(|&(code, ..)| is_point_code(code)));
    // It left the cursor as well: the repeat still finds it nearby.
    w.hover = None;
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(on_unit(&out, n), "{out:?}");
    assert!(!codes(&out).iter().any(|&(code, ..)| is_point_code(code)));
}

#[test]
fn a_held_attack_follows_a_living_target_that_left_the_cursor() {
    let (mut w, m, _) = fight();
    let mut st = ClickState::default();
    assert!(on_unit(&run(&w, &mut st, Kind::LeftDown, 0), m));
    // The target moved out from under the cursor but is still alive.
    w.hover = None;
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(on_unit(&out, m), "{out:?}");
    assert!(!codes(&out).iter().any(|&(code, ..)| is_point_code(code)));
}

#[test]
fn a_held_ranged_skill_casts_at_the_point_after_the_last_enemy_dies() {
    let (mut w, m, n) = fight();
    w.left = None;
    w.right = Some(SkillRef { id: 0, mode: 1 });
    w.rng = range::NONE;
    w.rows = vec![(
        0,
        SkillRowFacts {
            range: range::NONE,
            ..SkillRowFacts::default()
        },
    )];
    let mut st = ClickState::default();
    assert!(on_unit(&run(&w, &mut st, Kind::RightDown, 0), m));
    assert!(st.fighting[1]);
    w.dead = vec![m, n];
    for _ in 0..2 {
        st.end_pass();
        let mut out = Vec::new();
        held_repeat(&mut st, &w, 0, &mut out);
        // No enemy is left near the cursor: the ranged skill casts there.
        assert_eq!(codes(&out), [(0x0F, 101, 100)]);
    }
}

#[test]
fn a_held_attack_with_no_enemy_left_stands_until_released() {
    let (mut w, m, n) = fight();
    let mut st = ClickState::default();
    run(&w, &mut st, Kind::LeftDown, 0);
    w.dead = vec![m, n];
    w.counter = 200;
    for _ in 0..3 {
        st.end_pass();
        let mut out = Vec::new();
        held_repeat(&mut st, &w, 0, &mut out);
        assert!(codes(&out).is_empty(), "{out:?}");
        assert!(st.left_held && st.fighting[0]);
    }
    // Released: the next press on the ground walks as before.
    st.end_pass();
    run(&w, &mut st, Kind::LeftUp, 0);
    assert!(!st.fighting[0]);
    w.hover = None;
    w.ground = (110, 100);
    st.end_pass();
    assert_eq!(codes(&run(&w, &mut st, Kind::LeftDown, 0)), [(1, 110, 100)]);
    assert!(!st.fighting[0]);
}

#[test]
fn a_held_walk_never_turns_into_a_fight() {
    let (mut w, _, _) = fight();
    w.hover = None;
    w.ground = (110, 100);
    let mut st = ClickState::default();
    run(&w, &mut st, Kind::LeftDown, 0);
    assert!(!st.fighting[0]);
    w.counter = 110;
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert_eq!(codes(&out), [(1, 110, 100)]);
}

#[test]
fn a_held_right_skill_goes_on_after_a_kill() {
    let (mut w, m, n) = fight();
    w.left = None;
    w.right = Some(SkillRef { id: 0, mode: 1 });
    w.rows = vec![(
        0,
        SkillRowFacts {
            range: range::H2H,
            ..SkillRowFacts::default()
        },
    )];
    let mut st = ClickState::default();
    assert!(on_unit(&run(&w, &mut st, Kind::RightDown, 0), m));
    assert!(st.fighting[1]);
    w.dead.push(m);
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(on_unit(&out, n), "{out:?}");
    // A melee skill with no enemy left stands; it does not walk to the cursor.
    w.dead.push(n);
    st.end_pass();
    let mut out = Vec::new();
    held_repeat(&mut st, &w, 0, &mut out);
    assert!(codes(&out).is_empty(), "{out:?}");
    st.end_pass();
    run(&w, &mut st, Kind::RightUp, 0);
    assert!(!st.fighting[1]);
}

#[test]
fn input_reset_ends_a_fight() {
    let (w, ..) = fight();
    let mut st = ClickState::default();
    run(&w, &mut st, Kind::LeftDown, 0);
    st.input_reset();
    assert_eq!(st.fighting, [false; 2]);
}
