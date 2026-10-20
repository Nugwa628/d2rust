//! The character-create screen on synthetic timing, no game files.

use d2_client::ui::front_end::screens::create::*;
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use std::cell::RefCell;
use std::rc::Rc;

fn no_name_taken(_: &str) -> bool {
    false
}

fn typed(st: &mut CreateState, s: &str) {
    for c in s.chars() {
        st.type_char(c);
    }
}

#[test]
fn lineup_and_entry_state() {
    let st = CreateState::new(true, 0);
    let ids: Vec<u8> = st.heroes.iter().map(|h| h.class.id()).collect();
    assert_eq!(ids, [4, 1, 3, 2, 6, 0, 5]);
    assert_eq!(CreateState::new(false, 0).heroes.len(), 5);
    assert!(st.selected.is_none() && st.flags == 0);
    assert!(!st.name_box_visible() && !st.ok_enabled());
}

#[test]
fn walk_forward_then_selected_loop() {
    let mut st = CreateState::new(true, 0);
    st.hover(Some(Class::Paladin));
    assert_eq!(st.texts(), Some(((4008, 5132), Class::Paladin)));
    st.update(Some(Class::Paladin), 0);
    assert!(st.click(Class::Paladin, 0));
    assert_eq!(st.selected, Some(Class::Paladin));
    // pafw: 80 frames x 40 ms.
    assert_eq!(st.hero_frame(Class::Paladin, 40 * 10), Some((FORWARD, 10)));
    // The pass at frame 78 stores it; the next pass reads it and ends the
    // walk.
    st.update(None, 40 * 78);
    assert_eq!(
        st.hero_frame(Class::Paladin, 40 * 78).map(|x| x.0),
        Some(FORWARD)
    );
    st.update(None, 40 * 79);
    assert_eq!(
        st.hero_frame(Class::Paladin, 40 * 79).map(|x| x.0),
        Some(SELECTED)
    );
    // panu3: 9 frames, 80 ms, loops over 8.
    assert_eq!(
        st.hero_frame(Class::Paladin, 40 * 79 + 80 * 9),
        Some((SELECTED, 1))
    );
    // name box, hardcore unchecked, expansion checked, OK disabled
    assert!(st.name_box_visible() && st.expansion_box_visible());
    assert_eq!(st.flags, FLAG_EXPANSION);
    assert!(!st.ok_enabled());
}

#[test]
fn click_rules() {
    let mut st = CreateState::new(true, 0);
    // The hero is under the pointer at the last pass, so
    // the click starts its walk.
    st.update(Some(Class::Paladin), 0);
    assert!(st.click(Class::Paladin, 0));
    // walking hero: any click ignored
    assert!(!st.click(Class::Amazon, 40));
    st.update(None, 40 * 78);
    st.update(None, 40 * 79);
    // second hero while one is selected: first walks back, second forward
    st.update(Some(Class::Amazon), 4000);
    assert!(st.click(Class::Amazon, 4000));
    assert_eq!(st.selected, Some(Class::Amazon));
    assert_eq!(st.hero_frame(Class::Paladin, 4000).map(|x| x.0), Some(BACK));
    assert_eq!(
        st.hero_frame(Class::Amazon, 4000).map(|x| x.0),
        Some(FORWARD)
    );
    // walks finished; clicking the selected hero deselects it
    st.update(None, 4000 + 40 * 80);
    st.update(None, 4000 + 40 * 81);
    assert!(st.click(Class::Amazon, 8000));
    assert_eq!(st.selected, None);
    assert!(!st.name_box_visible());
}

#[test]
fn druid_grey_box_and_expansion_toggle() {
    let mut st = CreateState::new(true, 0);
    st.click(Class::Druid, 0);
    assert!(st.expansion_grey_visible() && !st.expansion_box_visible());
    st.toggle_expansion();
    assert_eq!(st.flags & FLAG_EXPANSION, FLAG_EXPANSION);
    let mut st = CreateState::new(true, 0);
    st.click(Class::Amazon, 0);
    st.toggle_expansion();
    assert_eq!(st.flags & FLAG_EXPANSION, 0);
    // classic install: Expansion never set
    let mut st = CreateState::new(false, 0);
    st.click(Class::Amazon, 0);
    assert_eq!(st.flags, 0);
}

#[test]
fn ok_needs_class_and_valid_name() {
    let mut st = CreateState::new(false, 0);
    typed(&mut st, "Zz");
    assert_eq!(st.name, "", "name box hidden until a class is selected");
    assert_eq!(st.ok(&no_name_taken), OkResult::Disabled);
    st.click(Class::Sorceress, 0);
    typed(&mut st, "Z");
    assert!(!st.ok_enabled());
    typed(&mut st, "z");
    assert!(st.ok_enabled());
    typed(&mut st, "-");
    assert!(!st.ok_enabled());
    st.backspace();
    typed(&mut st, "7 ");
    assert_eq!(st.name, "Zz");
    assert_eq!(
        st.ok(&no_name_taken),
        OkResult::Created(NewCharacter {
            name: "Zz".into(),
            class: Class::Sorceress,
            hardcore: false,
            expansion: false
        })
    );
}

#[test]
fn duplicate_name_blocks_until_edit() {
    let mut st = CreateState::new(false, 0);
    st.click(Class::Amazon, 0);
    typed(&mut st, "Bob");
    let taken = |n: &str| n.eq_ignore_ascii_case("bob");
    assert_eq!(st.ok(&taken), OkResult::NameTaken);
    assert!(st.name_taken);
    typed(&mut st, "b");
    assert!(!st.name_taken);
    assert!(matches!(st.ok(&taken), OkResult::Created(_)));
}

#[test]
fn name_taken_in_folder_ignores_case() {
    let dir = std::env::temp_dir().join(format!("d2rust-create-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Bob.d2s"), b"x").unwrap();
    assert!(name_taken_in(&dir, "BOB"));
    assert!(!name_taken_in(&dir, "Al"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hardcore_warning_and_cancel() {
    let mut st = CreateState::new(false, 0);
    st.click(Class::Barbarian, 0);
    typed(&mut st, "Hc");
    st.toggle_hardcore();
    assert_eq!(st.ok(&no_name_taken), OkResult::Warning);
    let c = st.warning_ok().unwrap();
    assert!(c.hardcore);
    // cancel rebuilds the whole screen
    let mut st = CreateState::new(false, 0);
    st.click(Class::Barbarian, 0);
    typed(&mut st, "Hc");
    st.toggle_hardcore();
    st.ok(&no_name_taken);
    st.warning_cancel(500);
    assert!(st.selected.is_none() && st.name.is_empty() && st.flags == 0 && !st.warning);
}

fn setup(expansion: bool) -> (FrontEnd, Rc<RefCell<Option<NewCharacter>>>) {
    let sink: NewCharacterSink = Rc::default();
    let mut reg = Registry::default();
    register_with(&mut reg, sink.clone(), Box::new(no_name_taken));
    let mut f = FrontEnd::new(expansion, Box::new(false), reg);
    // No saves: Single Player goes straight to character create.
    f.start(false);
    f.trigger(Trigger::SinglePlayer);
    (f, sink)
}

#[test]
fn screen_creates_character_and_starts_game() {
    let (mut f, sink) = setup(true);
    assert_eq!(f.current(), CHAR_CREATE);
    // click Paladin at its cel (521, 339), inside the 88x184 box
    let p = Point::new(540, 300);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    f.tick();
    for c in "Zz".chars() {
        f.input(FrontInput::Char(c as u16));
    }
    f.input(FrontInput::Key(13));
    f.tick();
    assert_eq!(
        f.outcome(),
        Some(Outcome::GameLoad(GameLoad {
            difficulty: None,
            new_character: true,
            online: false
        }))
    );
    let c = sink.borrow().clone().unwrap();
    assert_eq!(
        (c.name.as_str(), c.class, c.expansion),
        ("Zz", Class::Paladin, true)
    );
}

#[test]
fn enter_without_valid_name_stays_and_esc_goes_back() {
    let (mut f, sink) = setup(false);
    f.input(FrontInput::Key(13));
    f.tick();
    assert_eq!(f.current(), CHAR_CREATE);
    assert!(sink.borrow().is_none());
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.current(), CHAR_SELECT);
}

/// Esc leaves the screen whatever the selection (the Exit button's hotkey).
#[test]
fn esc_after_a_class_click_goes_back() {
    let (mut f, _sink) = setup(true);
    let p = Point::new(540, 300);
    f.input(FrontInput::Move(p));
    f.tick();
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    for _ in 0..30 {
        f.tick();
    }
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.current(), CHAR_SELECT);
}

/// B003: click one class, let it walk forward, click another: the second
/// becomes the selected one (the class animation state machine; test vector 2 of
/// the create screen), through the screen's own click path (pointer move,
/// button down and up), not only `CreateState::click`.
#[test]
fn a_second_class_click_through_the_screen_takes_over() {
    let (mut f, sink) = setup(true);
    let click = |f: &mut FrontEnd, p: Point| {
        f.input(FrontInput::Move(p));
        f.tick();
        f.input(FrontInput::Down(p));
        f.input(FrontInput::Up(p));
        f.tick();
    };
    click(&mut f, Point::new(540, 300)); // Paladin
    for _ in 0..200 {
        f.tick();
    }
    click(&mut f, Point::new(140, 300)); // Amazon
    for _ in 0..200 {
        f.tick();
    }
    for c in "Zz".chars() {
        f.input(FrontInput::Char(c as u16));
    }
    f.input(FrontInput::Key(13));
    f.tick();
    let c = sink.borrow().clone().expect("a character was created");
    assert_eq!(c.class, Class::Amazon);
}

/// B003, every ordered pair: the centre of a class's idle box (the pointer
/// there) picks that class after any other class was selected first.
#[test]
fn every_class_can_be_picked_after_every_other() {
    for expansion in [false, true] {
        let slots: &[Slot] = if expansion { &EXPANSION } else { &CLASSIC };
        let centre = |class: Class, x: i32, y: i32| {
            let (w, h, ox, oy) = HERO_BOXES[usize::from(class.id())][0];
            Point::new(x + ox + w / 2, y + oy - h / 2)
        };
        let mut failures = Vec::new();
        for &(a, ax, ay) in slots {
            for &(b, bx, by) in slots.iter().filter(|s| s.0 != a) {
                let (mut f, sink) = setup(expansion);
                let click = |f: &mut FrontEnd, p: Point| {
                    f.input(FrontInput::Move(p));
                    f.tick();
                    f.input(FrontInput::Down(p));
                    f.input(FrontInput::Up(p));
                    f.tick();
                };
                click(&mut f, centre(a, ax, ay));
                for _ in 0..200 {
                    f.tick();
                }
                click(&mut f, centre(b, bx, by));
                for _ in 0..200 {
                    f.tick();
                }
                for c in "Zz".chars() {
                    f.input(FrontInput::Char(c as u16));
                }
                f.input(FrontInput::Key(13));
                f.tick();
                let got = sink.borrow().as_ref().map(|c| c.class);
                if got != Some(b) {
                    failures.push(format!("{a:?} then {b:?}: {got:?} (expansion {expansion})"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
