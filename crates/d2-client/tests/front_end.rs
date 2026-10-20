//! The front-end shell on synthetic input, no game files.

use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;

fn fe(expansion: bool, saves: bool) -> FrontEnd {
    FrontEnd::with_screens(expansion, Box::new(saves))
}

fn started(expansion: bool, saves: bool) -> FrontEnd {
    let mut f = fe(expansion, saves);
    f.start(true);
    f
}

fn ticks(f: &mut FrontEnd, n: u32) {
    for _ in 0..n {
        f.tick();
    }
}

fn click(f: &mut FrontEnd, x: i32, y: i32) {
    let p = Point::new(x, y);
    f.input(FrontInput::Down(p));
    f.input(FrontInput::Up(p));
    f.tick();
}

#[test]
fn startup_opens_trademark_after_a_flush() {
    let f = started(true, true);
    assert_eq!(f.current(), TRADEMARK);
    assert_eq!(f.entered, [TRADEMARK]);
    assert_eq!(f.flushes, 1);
    assert_eq!(f.palette(), Some(SKY_PALETTE));
}

#[test]
fn later_entry_goes_to_main_menu() {
    let mut f = fe(true, true);
    f.start(false);
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn trademark_click_goes_to_main_menu() {
    let mut f = started(true, true);
    click(&mut f, 400, 300);
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn trademark_keys() {
    for (key, to_menu) in [
        (b'Q' as u16, false),
        (b'N' as u16, true),
        (32, true),
        (27, true),
        (13, true),
        (112, true),
        (113, false),
        (b'7' as u16, false),
    ] {
        let mut f = started(true, true);
        f.input(FrontInput::Key(key));
        f.tick();
        assert_eq!(f.current() == MAIN_MENU, to_menu, "key {key}");
    }
}

#[test]
fn trademark_timer_fires_after_nine_whole_seconds() {
    let mut f = started(true, true);
    // Built at ms 0: first tick with floor(now/1000) > 9 is ms 10,000 = tick 250.
    ticks(&mut f, 249);
    assert_eq!(f.current(), TRADEMARK);
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn flow_table() {
    let id = |f: &FrontEnd| f.current();
    // Main menu → Single Player, saves found → character select.
    let mut f = started(true, true);
    f.trigger(Trigger::Continue);
    f.trigger(Trigger::SinglePlayer);
    assert_eq!(id(&f), CHAR_SELECT);
    assert_eq!(f.game_kind, 0);
    // Exit → main menu.
    f.trigger(Trigger::Exit);
    assert_eq!(id(&f), MAIN_MENU);
    // No saves → character create; Exit → character select.
    let mut g = started(true, false);
    g.trigger(Trigger::Continue);
    g.trigger(Trigger::SinglePlayer);
    assert_eq!(id(&g), CHAR_CREATE);
    g.trigger(Trigger::Exit);
    assert_eq!(id(&g), CHAR_SELECT);
    // Create New → create; OK → game load of a new character.
    g.trigger(Trigger::CreateNew);
    assert_eq!(id(&g), CHAR_CREATE);
    g.trigger(Trigger::Ok);
    assert_eq!(
        g.outcome(),
        Some(Outcome::GameLoad(GameLoad {
            difficulty: None,
            new_character: true,
            online: false
        }))
    );

    // Credits and back.
    let mut f = started(true, true);
    f.trigger(Trigger::Continue);
    f.trigger(Trigger::Credits);
    assert_eq!(id(&f), CREDITS);
    f.trigger(Trigger::Exit);
    assert_eq!(id(&f), MAIN_MENU);

    // Character select OK: one difficulty open → game load, no popup.
    f.trigger(Trigger::SinglePlayer);
    assert_eq!(id(&f), CHAR_SELECT);
    f.flow_mut().difficulties_open = 1;
    f.trigger(Trigger::Ok);
    assert_eq!(
        f.outcome(),
        Some(Outcome::GameLoad(GameLoad {
            difficulty: None,
            new_character: false,
            online: false
        }))
    );

    // More than one open → popup; each button loads; Esc returns.
    for d in 0..3u8 {
        let mut f = started(true, true);
        f.trigger(Trigger::Continue);
        f.trigger(Trigger::SinglePlayer);
        f.flow_mut().difficulties_open = 2;
        f.trigger(Trigger::Ok);
        assert_eq!(id(&f), DIFFICULTY);
        if d == 0 {
            f.trigger(Trigger::Exit);
            assert_eq!(id(&f), CHAR_SELECT);
            f.trigger(Trigger::Ok);
        }
        f.trigger(Trigger::Difficulty(d));
        assert_eq!(
            f.outcome(),
            Some(Outcome::GameLoad(GameLoad {
                difficulty: Some(d),
                new_character: false,
                online: false
            }))
        );
    }

    // The character select's re-entry trigger (the in-game exit itself opens
    // the main menu, see `smoke_frontend`).
    let mut f = started(true, true);
    f.trigger(Trigger::GameExit);
    assert_eq!(id(&f), CHAR_SELECT);
}

#[test]
fn main_menu_enter_is_unbound_and_esc_exits() {
    // Placeholder screens map Enter to OK; the main menu has no OK row.
    let mut f = started(true, true);
    f.trigger(Trigger::Continue);
    f.input(FrontInput::Key(13));
    f.tick();
    assert_eq!(f.current(), MAIN_MENU);
    assert_eq!(f.outcome(), None);
    f.input(FrontInput::Key(27));
    f.tick();
    assert_eq!(f.outcome(), Some(Outcome::Exit));
}

#[test]
fn hit_box_and_button_facts() {
    use d2_client::ui::front_end::control::*;
    // Single Player: (264, 324) 272×35, y = bottom edge.
    let c = Control::new(ControlKind::Button, 264, 324, 272, 35);
    assert!(c.contains(Point::new(264, 289)));
    assert!(!c.contains(Point::new(264, 324)));
    assert!(!c.contains(Point::new(264, 288)));
    assert!(c.contains(Point::new(535, 323)));
    assert!(!c.contains(Point::new(536, 323)));
    assert_eq!(button_tiles(272, 35), 2);
    assert_eq!(button_frame(1, 2, true, true, false), 3);
    assert_eq!(label_font(25), 10);
    assert_eq!(label_k(25), 2);
    assert_eq!(label_font(35), 9);
    assert_eq!(label_k(35), 4);
    // The label rule: 3 only at h = 32; 33 and 34 take 2; 20 takes 1.
    assert_eq!(label_k(32), 3);
    assert_eq!(label_k(33), 2);
    assert_eq!(label_k(34), 2);
    assert_eq!(label_k(21), 2);
    assert_eq!(label_k(20), 1);
}

#[test]
fn logo_frames() {
    assert_eq!(logo_frame(1000, 0), 25);
    assert_eq!(logo_frame(1160, 0), 0);
}

mod loading {
    use d2_client::ui::front_end::screens::loading::*;

    fn run(evs: &[LoadEvent]) -> LoadingScreen {
        let mut s = LoadingScreen::new();
        for e in evs {
            s.event(*e);
        }
        s
    }

    #[test]
    fn game_start_frames() {
        let mut s = run(&[
            LoadEvent::GameStart,
            LoadEvent::S01,
            LoadEvent::S03 { act: 0 },
        ]);
        assert!(!s.wants_input());
        assert_eq!(s.frame(true), None); // no 0x04 yet
        s.event(LoadEvent::S04);
        assert_eq!(s.frame(false), None); // no placed player
        s.frame(true);
        s.frame(true);
        assert_eq!(
            s.presented,
            [
                Presented::Loading { frame: 0 },
                Presented::Loading { frame: 1 },
                Presented::Black,
                Presented::World { act: 0 },
            ]
        );
        assert!(s.wants_input());
    }

    #[test]
    fn act_change_frames_and_video() {
        let mut s = run(&[
            LoadEvent::GameStart,
            LoadEvent::S03 { act: 0 },
            LoadEvent::S04,
        ]);
        s.frame(true);
        s.frame(true);
        s.presented.clear();
        s.event(LoadEvent::S05);
        assert_eq!(s.frame(true), None); // game draws stop
        s.event(LoadEvent::S03 { act: 1 });
        s.event(LoadEvent::S61 { id: 2 });
        s.event(LoadEvent::S04);
        s.frame(true);
        s.frame(true);
        assert_eq!(s.videos, [2]);
        assert_eq!(
            s.presented,
            [
                Presented::Loading { frame: 0 },
                Presented::Black,
                Presented::World { act: 1 },
            ]
        );
    }

    #[test]
    fn clamp_placement_path() {
        let mut s = LoadingScreen::new();
        for _ in 0..12 {
            s.event(LoadEvent::S03 { act: 0 });
        }
        assert_eq!(s.presented[11], Presented::Loading { frame: 9 });
        assert_eq!(placement(800, 600), (272, 428));
        assert_eq!(placement(640, 480), (192, 368));
        assert_eq!(art_path(3), r"DATA\LOCAL\UI\LoadingScreen");
        assert_eq!(video_name(5), Some("ACT04END"));
    }
}
