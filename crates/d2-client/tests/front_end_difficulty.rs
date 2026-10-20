//! Difficulty box and game-start rules on synthetic input, no game files.

use d2_client::ui::front_end::screens::difficulty::*;
use d2_client::ui::front_end::*;

fn at_box(open: u8) -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(true));
    f.trigger(Trigger::Continue);
    f.trigger(Trigger::SinglePlayer);
    f.flow_mut().difficulties_open = open;
    f.trigger(Trigger::Ok);
    f
}

#[test]
fn spec_vectors() {
    // 0x0420: no box, Normal, flags 0x00100004.
    assert_eq!(ok_decision(0x0420, true), OkAction::Start);
    assert_eq!(start_flags(0x0420), 0x0010_0004);
    // 0x0520: box, Hell off.
    assert_eq!(ok_decision(0x0520, true), OkAction::Box);
    assert!(!hell_enabled(0x0520));
    // 0x0920 off, 0x0A20 on.
    assert!(!hell_enabled(0x0920));
    assert!(hell_enabled(0x0A20));
    // 0x0400 classic on an expansion install: box, Hell off, flags 4.
    assert_eq!(ok_decision(0x0400, true), OkAction::Box);
    assert!(!hell_enabled(0x0400));
    assert_eq!(start_flags(0x0400), 4);
    // 0x0804 classic hardcore: box, Hell on, flags 0x804.
    assert_eq!(ok_decision(0x0804, true), OkAction::Box);
    assert!(hell_enabled(0x0804));
    assert_eq!(start_flags(0x0804), 0x804);
    // 0x0020 on a classic install: nothing.
    assert_eq!(ok_decision(0x0020, false), OkAction::Ignored);
    // Dead hardcore: message, no box.
    assert_eq!(ok_decision(0x050C, true), OkAction::DeadHardcore);
    // Start act: bytes 00 83 00.
    assert_eq!(start_act([0, 0x83, 0], 1), 3);
    assert_eq!(start_act([0, 0x83, 0], 0), 0);
    assert_eq!(start_act([7, 0, 0], 0), 0);
}

#[test]
fn box_buttons_and_keys() {
    let status_open = |s| difficulties_open(s, true);
    assert_eq!(status_open(0x0420), 1);
    assert_eq!(status_open(0x0520), 2);
    assert_eq!(status_open(0x0A20), 3);

    // Hell disabled: button greyed, H does nothing.
    let mut f = at_box(2);
    assert_eq!(f.current(), DIFFICULTY);
    let hell = f.controls().iter().find(|c| c.string_id == 10016).unwrap();
    assert!(!hell.enabled);
    f.input(FrontInput::Key(u16::from(b'H')));
    f.tick();
    assert_eq!(f.outcome(), None);
    // Esc back to character select.
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.current(), CHAR_SELECT);

    // Hell enabled: H starts at Hell; N at Nightmare; R at Normal.
    for (k, d) in [(b'H', 2u8), (b'N', 1), (b'R', 0)] {
        let mut f = at_box(3);
        f.input(FrontInput::Key(u16::from(k)));
        f.tick();
        assert_eq!(
            f.outcome(),
            Some(Outcome::GameLoad(GameLoad {
                difficulty: Some(d),
                new_character: false,
                online: false
            }))
        );
    }
}
