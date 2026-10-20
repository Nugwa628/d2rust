//! The credits screen inside the shell, on synthetic text.

use d2_client::ui::front_end::screens::credits;
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;

fn at_credits() -> FrontEnd {
    let mut reg = Registry::default();
    credits::register_with(&mut reg, Box::new(|_| Some(b"*H\r\nAl\r\nBo\r\n".to_vec())));
    let mut f = FrontEnd::new(true, Box::new(true), reg);
    f.start(true);
    f.trigger(Trigger::Continue);
    f.trigger(Trigger::Credits);
    assert_eq!(f.current(), CREDITS);
    f
}

#[test]
fn esc_and_exit_button_return_to_main_menu() {
    let mut f = at_credits();
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);

    let mut f = at_credits();
    let p = Point::new(40, 560);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn space_enter_click_do_nothing_and_no_auto_return() {
    let mut f = at_credits();
    for k in [32, 13] {
        f.input(FrontInput::Key(k));
    }
    let p = Point::new(400, 300);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    for _ in 0..2000 {
        f.tick();
    }
    assert_eq!(f.current(), CREDITS);
}
