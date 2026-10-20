//! The front end's ui ids: what each screen lists, and that a click on an
//! id's point does what a click on that button does. No game files.

use super::*;
use crate::ui::front_end::screens::ids::OPTIONS;
use crate::ui::front_end::{FrontInput, Outcome};
use crate::ui::ui_ids;

fn menu(saves: bool) -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(saves));
    f.start(false);
    assert_eq!(f.current(), MAIN_MENU);
    f
}

fn ids(f: &FrontEnd) -> Vec<String> {
    f.ui_elements().into_iter().map(|e| e.id).collect()
}

/// A click on `id` (move, press, release at its point), then one tick.
fn click(f: &mut FrontEnd, id: &str) {
    let els = f.ui_elements();
    let at = ui_ids::find(&els, id).unwrap_or_else(|e| panic!("{e}")).at;
    f.input(FrontInput::Move(at));
    f.input(FrontInput::Down(at));
    f.input(FrontInput::Up(at));
    f.tick();
}

#[test]
fn the_main_menu_lists_the_buttons_that_do_something() {
    let f = menu(false);
    // With the `online` feature the second button is ONLINE and opens the
    // online login.
    let online: &[&str] = if cfg!(feature = "online") {
        &["menu.online"]
    } else {
        &[]
    };
    let want: Vec<&str> = online
        .iter()
        .copied()
        .chain([
            "menu.single_player",
            "menu.credits",
            "menu.exit",
            "menu.display_options",
        ])
        .collect();
    assert_eq!(ids(&f), want);
    // Every point is on its own button.
    for e in f.ui_elements() {
        assert!(e.rect.contains(e.at), "{e:?}");
        assert_eq!(f.hit(e.at).map(|i| f.controls[i].hit_box()), Some(e.rect));
    }
}

#[test]
fn the_trademark_screen_continues_on_a_click() {
    let mut f = FrontEnd::with_screens(true, Box::new(false));
    f.goto(TRADEMARK);
    assert_eq!(ids(&f), ["trademark.continue"]);
    click(&mut f, "trademark.continue");
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn single_player_without_saves_opens_create_and_a_new_character_starts() {
    let mut f = menu(false);
    click(&mut f, "menu.single_player");
    assert_eq!(f.current(), CHAR_CREATE);
    let classes: Vec<String> = ids(&f)
        .into_iter()
        .filter(|i| i.starts_with("create.class."))
        .collect();
    assert_eq!(classes.len(), 7, "{classes:?}");
    for c in ["amazon", "assassin", "druid", "sorceress"] {
        assert!(
            classes.contains(&format!("create.class.{c}")),
            "{classes:?}"
        );
    }
    assert!(ids(&f).contains(&"create.exit".to_owned()));
    click(&mut f, "create.class.amazon");
    for _ in 0..50 {
        f.tick();
    }
    for u in "Probe".encode_utf16() {
        f.input(FrontInput::Char(u));
    }
    f.tick();
    let now = ids(&f);
    for i in ["create.ok", "create.hardcore", "create.expansion"] {
        assert!(now.contains(&i.to_owned()), "{now:?}");
    }
    click(&mut f, "create.ok");
    assert!(
        matches!(f.outcome(), Some(Outcome::GameLoad(g)) if g.new_character),
        "{:?}",
        f.outcome()
    );
}

#[test]
fn the_character_select_lists_its_buttons() {
    let mut f = menu(true);
    click(&mut f, "menu.single_player");
    assert_eq!(f.current(), CHAR_SELECT);
    let now = ids(&f);
    for i in ["char_select.exit", "char_select.create_new"] {
        assert!(now.contains(&i.to_owned()), "{now:?}");
    }
    // No folder: an empty list, no character, and OK, Convert and Delete
    // are disabled, so not listed.
    assert!(!now.iter().any(|i| i.starts_with("char_select.char(")));
    for i in [
        "char_select.ok",
        "char_select.convert",
        "char_select.delete",
    ] {
        assert!(!now.contains(&i.to_owned()), "{now:?}");
    }
    click(&mut f, "char_select.create_new");
    assert_eq!(f.current(), CHAR_CREATE);
    click(&mut f, "create.exit");
    assert_eq!(f.current(), CHAR_SELECT);
    click(&mut f, "char_select.exit");
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn a_slot_is_named_by_its_character() {
    use crate::ui::front_end::control::TextRow;
    let mut text = Control::new(ControlKind::Text, 37, 178, 200, 92);
    text.more_rows.push(TextRow {
        string_id: 0,
        text: "Ab-c".into(),
        color: 0,
    });
    let slot = Control::new(ControlKind::Button, 37, 178, 200, 92)
        .with_action(Action::Custom(select::SLOT_TEXT));
    let empty = Control::new(ControlKind::Button, 309, 178, 200, 92)
        .with_action(Action::Custom(select::SLOT_TEXT + 1));
    let controls = vec![text, slot, empty];
    assert_eq!(
        control_id(CHAR_SELECT, &controls, 1).as_deref(),
        Some("char_select.char(Ab-c)")
    );
    assert_eq!(control_id(CHAR_SELECT, &controls, 2), None);
}

#[test]
fn the_difficulty_screen_lists_the_open_difficulties() {
    let mut f = menu(true);
    f.flow_mut().difficulties_open = 2;
    f.goto(DIFFICULTY);
    assert_eq!(ids(&f), ["difficulty.normal", "difficulty.nightmare"]);
    click(&mut f, "difficulty.nightmare");
    assert!(
        matches!(f.outcome(), Some(Outcome::GameLoad(g)) if g.difficulty == Some(1)),
        "{:?}",
        f.outcome()
    );
}

#[test]
fn the_options_rows_have_ids_and_a_click_changes_the_setting() {
    let mut f = menu(false);
    click(&mut f, "menu.display_options");
    assert_eq!(f.current(), OPTIONS);
    let now = ids(&f);
    for i in [
        "options.video.vsync",
        "options.video.frame_limit",
        "options.video.previous",
    ] {
        assert!(now.contains(&i.to_owned()), "{now:?}");
    }
    // Every row is listed once and its point is in its own row.
    let mut sorted = now.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), now.len());
    let before = f.settings.current.vsync;
    click(&mut f, "options.video.vsync");
    assert_ne!(f.settings.current.vsync, before);
    click(&mut f, "options.video.previous");
    let now = ids(&f);
    assert!(
        now.contains(&"options.options.video_options".to_owned()),
        "{now:?}"
    );
    click(&mut f, "options.options.previous");
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn credits_have_their_button() {
    let mut f = menu(false);
    click(&mut f, "menu.credits");
    assert_eq!(f.current(), CREDITS);
    assert_eq!(ids(&f), ["credits.exit"]);
    click(&mut f, "credits.exit");
    assert_eq!(f.current(), MAIN_MENU);
}

#[test]
fn configure_controls_ids_click_existing_descriptors() {
    use super::super::controls::ControlsScreen;
    use super::super::ids::CONTROLS;
    fn click_in_frame(f: &mut FrontEnd, id: &str, width: i32) {
        let els = f.ui_elements();
        let e = ui_ids::find(&els, id).unwrap();
        let ui_left = (width - 800) / 2;
        let frame_at = Point::new(e.at.x + ui_left, e.at.y);
        assert!((0..width).contains(&frame_at.x));
        assert!((0..600).contains(&frame_at.y));
        // The front end always uses the centred 800x600 layout. Translate
        // the frame click back into its coordinates, as the host does.
        let at = Point::new(frame_at.x - ui_left, frame_at.y);
        f.input(FrontInput::Move(at));
        f.input(FrontInput::Down(at));
        f.input(FrontInput::Up(at));
        f.tick();
    }
    for width in [800, 1068] {
        let mut f = menu(false);
        let screen = ControlsScreen::new(None);
        let model = screen.model();
        f.registry.register(CONTROLS, Box::new(screen));
        f.goto(CONTROLS);
        let e = f
            .ui_elements()
            .into_iter()
            .find(|e| e.id.starts_with("controls.binding("))
            .unwrap();
        for cell in f
            .ui_elements()
            .into_iter()
            .filter(|e| e.id.starts_with("controls.binding("))
        {
            click_in_frame(&mut f, &cell.id, width);
            let m = model.borrow();
            assert!(m.editing());
            assert_eq!(
                cell.id,
                format!(
                    "controls.binding({},{})",
                    m.rows()[m.selected()].cmd,
                    m.column()
                )
            );
        }
        click_in_frame(&mut f, &e.id, width);
        assert!(model.borrow().editing());
        click_in_frame(&mut f, "controls.default", width);
        assert!(!model.borrow().editing());
        let before = ids(&f);
        click_in_frame(&mut f, "controls.scroll_down", width);
        assert_ne!(before, ids(&f));
        click_in_frame(&mut f, "controls.scroll_up", width);
        assert_eq!(before, ids(&f));
        click_in_frame(&mut f, "controls.cancel", width);
        assert_eq!(f.current(), OPTIONS);
        f.goto(CONTROLS);
        click_in_frame(&mut f, "controls.accept", width);
        assert_eq!(f.current(), OPTIONS);
        assert!(!ids(&f).iter().any(|id| id.starts_with("controls.")));
    }
}
