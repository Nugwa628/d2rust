//! Mouse press / release on the front end (the dispatcher
//! and the button handlers), no game files.

use super::screens::create::{register_with, NewCharacterSink};
use super::screens::ids::{CHAR_CREATE, CHAR_SELECT, MAIN_MENU};
use super::screens::main_menu::WIDE;
use super::*;

fn menu() -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(true));
    f.start(false);
    assert_eq!(f.current(), MAIN_MENU);
    f
}

/// Frames of the Single Player button (272×35, two tiles).
fn single_player_frames(f: &FrontEnd) -> Vec<u32> {
    f.draw()
        .iter()
        .filter_map(|d| match d {
            DrawItem::Art { file, frame, at } if *file == WIDE && at.y == 324 => Some(*frame),
            _ => None,
        })
        .collect()
}

const ON: Point = Point::new(300, 310);
const OFF: Point = Point::new(500, 100);

#[test]
fn release_after_dragging_off_fires_nothing() {
    let mut f = menu();
    f.input(FrontInput::Move(ON));
    f.input(FrontInput::Down(ON));
    f.tick();
    assert_eq!(single_player_frames(&f), [2, 3]);
    f.input(FrontInput::Move(OFF));
    f.tick();
    // Dragged off: the up frames.
    assert_eq!(single_player_frames(&f), [0, 1]);
    f.input(FrontInput::Up(OFF));
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn dragging_back_on_fires_on_release() {
    let mut f = menu();
    f.input(FrontInput::Move(ON));
    f.input(FrontInput::Down(ON));
    f.tick();
    f.input(FrontInput::Move(OFF));
    f.tick();
    f.input(FrontInput::Move(ON));
    f.tick();
    assert_eq!(single_player_frames(&f), [2, 3]);
    f.input(FrontInput::Up(ON));
    f.tick();
    assert_eq!(f.current(), CHAR_SELECT);
}

/// The release is judged by the last draw's test, not by its own point.
#[test]
fn release_outside_without_a_draw_since_still_fires() {
    let mut f = menu();
    f.input(FrontInput::Move(ON));
    f.input(FrontInput::Down(ON));
    f.tick();
    f.input(FrontInput::Up(OFF));
    f.tick();
    assert_eq!(f.current(), CHAR_SELECT);
}

/// A check box (descriptor [11] = 2) toggles on the press.
#[test]
fn check_box_toggles_on_the_press() {
    let sink: NewCharacterSink = Default::default();
    let mut reg = Registry::default();
    register_with(&mut reg, sink, Box::new(|_: &str| false));
    let mut f = FrontEnd::new(false, Box::new(false), reg);
    f.start(false);
    f.trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_CREATE);
    let boxed = |f: &FrontEnd| {
        f.controls()
            .iter()
            .find(|c| c.on_press && c.art == Some("FrontEnd\\clickbox") && c.visible)
            .map(|c| (Point::new(c.x + 2, c.y - 2), c.checked))
    };
    // A class first: the hardcore pair shows with the name box.
    let p = Point::new(540, 300);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    for _ in 0..30 {
        f.tick();
    }
    let (at, checked) = boxed(&f).expect("hardcore box");
    assert!(!checked);
    f.input(FrontInput::Down(at));
    f.tick();
    assert_eq!(boxed(&f).map(|b| b.1), Some(true));
    f.input(FrontInput::Up(at));
    f.tick();
    assert_eq!(boxed(&f).map(|b| b.1), Some(true));
}
