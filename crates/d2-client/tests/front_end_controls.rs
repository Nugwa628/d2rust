//! CONFIGURE CONTROLS on synthetic input, no game files.

use d2_client::controls::original::{BindingTable, UNBOUND};
use d2_client::ui::front_end::screens::controls::*;
use d2_client::ui::front_end::*;
use d2_client::ui::front_end::{Action, Control};
use d2_client::ui::geom::Point;

const ENTER: u16 = 13;
const DOWN: u16 = 0x28;

fn open() -> ConfigureControls {
    ConfigureControls::open(true, BindingTable::defaults())
}

/// Inventory is expansion row 1: select it, column One (initial).
fn on_inventory() -> ConfigureControls {
    let mut c = open();
    c.key_down(DOWN, false, 0);
    assert_eq!(c.rows()[c.selected()].cmd, 1);
    c
}

#[test]
fn inventory_gets_c_and_character_loses_it() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    assert!(c.editing());
    c.take_sounds();
    c.key_down(0x43, false, 0);
    assert!(!c.editing());
    assert_eq!(c.table().key_of(1, 1), 0x43);
    assert_eq!(c.table().key_of(0, 0), UNBOUND, "Character slot 0 -> None");
    assert_eq!(c.table().key_of(0, 1), 0x41, "Character slot 1 untouched");
    assert_eq!(c.take_sounds(), vec![Sound::CursorSelect]);
}

#[test]
fn cancel_restores_and_accept_keeps() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(0x43, false, 0);
    assert_eq!(c.cancel(), Done::Cancel);
    assert_eq!(c.table(), &BindingTable::defaults());
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(0x43, false, 0);
    assert_eq!(c.accept(), Done::Accept);
    assert_eq!(c.table().key_of(1, 1), 0x43);
}

#[test]
fn esc_while_editing_changes_nothing_and_latches_the_next_enter() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(27, false, 0);
    assert!(!c.editing());
    assert_eq!(c.table(), &BindingTable::defaults());
    assert_eq!(c.latch(), 0);
    c.key_down(ENTER, false, 0);
    assert!(!c.editing(), "swallowed by the latch");
    c.key_down(ENTER, false, 0);
    assert!(c.editing(), "the next Enter edits");
}

#[test]
fn enter_release_is_swallowed_and_enter_assignment_latches() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_up(ENTER, 0);
    assert!(c.editing(), "release of the starting Enter does nothing");
    c.key_down(ENTER, false, 0);
    assert!(!c.editing());
    assert_eq!(c.table().key_of(1, 1), ENTER);
    assert_eq!(c.latch(), 0);
}

#[test]
fn bad_key_keeps_editing_with_a_message_for_two_seconds() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(0x1B + 1, false, 1000); // 0x1C: not an allowed key
    assert!(c.editing());
    assert_eq!(c.message(1000).map(|e| e.string_id()), Some(3979));
    assert!(c.message(2999).is_some());
    assert!(c.message(3000).is_none());
}

#[test]
fn delete_unbinds_and_navigation_skips_separators() {
    let mut c = open();
    c.key_down(0x2E, false, 0);
    assert_eq!(c.table().key_of(0, 1), UNBOUND);
    for _ in 0..40 {
        c.key_down(DOWN, false, 0);
        assert_ne!(c.rows()[c.selected()].cmd, 57);
    }
    assert!(c.selected() < c.rows().len());
    assert!(c.selected() < c.top() + 15, "selection stays visible");
    c.key_down(0x27, false, 0);
    assert_eq!(c.column(), 0);
    c.key_down(0x25, false, 0);
    assert_eq!(c.column(), 1);
}

#[test]
fn wheel_scrolls_two_rows_per_notch_clamped() {
    let mut c = open();
    c.wheel(-120);
    assert_eq!(c.top(), 2);
    c.wheel(120);
    c.wheel(120);
    assert_eq!(c.top(), 0);
    c.wheel(-120 * 100);
    assert_eq!(c.top(), 47);
}

#[test]
fn default_resets_without_saving() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(0x43, false, 0);
    c.default_all();
    assert_eq!(c.table(), &BindingTable::defaults());
}

#[test]
fn layout_numbers() {
    let mut c = open();
    let d = c.draw_list(0, None);
    let heading_x: Vec<i32> = d
        .iter()
        .filter_map(|i| match i {
            CfgDraw::Text {
                string_id: 3921..=3923,
                x,
                y: 100,
                ..
            } => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(heading_x, vec![108, 298, 488]);
    let centres: Vec<i32> = d
        .iter()
        .filter_map(|i| match i {
            CfgDraw::Text {
                string_id: 3972..=3974,
                x,
                y: 421,
                ..
            } => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(centres, vec![193, 399, 605]);
    assert!(d.iter().any(|i| matches!(
        i,
        CfgDraw::Text {
            string_id: 3924,
            y: 122,
            ..
        }
    )));
}

#[test]
fn edit_cell_blinks() {
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    // 16 draws 40 ms apart: the blink steps on the draw clock.
    let shown: Vec<bool> = (0..16)
        .map(|i| {
            c.draw_list(i * 40, None)
                .iter()
                .any(|i| matches!(i, CfgDraw::Text { x: 310, y, .. } if *y == 70 + 52 + 18))
        })
        .collect();
    assert_eq!(shown.iter().filter(|s| !**s).count(), 5);
}

#[test]
fn bindings_round_trip_through_controls_toml() {
    let dir = std::env::temp_dir().join(format!("d2rust-cfg-{}", std::process::id()));
    let path = dir.join("d2rust").join("controls.toml");
    let mut c = on_inventory();
    c.key_down(ENTER, false, 0);
    c.key_down(0x43, false, 0);
    save_table(c.table(), &path).unwrap();
    let t = load_table(Some(&path));
    assert_eq!(t.key_of(1, 1), 0x43);
    assert_eq!(t.key_of(0, 0), UNBOUND);
    // Play input sees Inventory on C and Character on A only.
    let (_, b) = d2_client::controls::load(&path).unwrap();
    use d2_client::controls::{Action, Key};
    assert_eq!(b.inputs(Action::ToggleInventory)[0], Key::C);
    assert_eq!(b.inputs(Action::ToggleCharacter), &[Key::A]);
    std::fs::remove_dir_all(&dir).ok();
}

// ---- through the front-end shell --------------------------------------

use d2_client::ui::front_end::flow::{next, FlowCtx, Next};

fn click_screen(
    s: &mut ControlsScreen,
    ctrls: &[Control],
    ctx: &mut FrontCtx,
    x: i32,
    y: i32,
) -> Option<Trigger> {
    let c = ctrls
        .iter()
        .rposition(|c| {
            c.action != Action::None && c.visible_or_hit() && c.contains(Point::new(x, y))
        })
        .expect("a control under the pointer");
    match ctrls[c].action {
        Action::Custom(id) => s.action(ctx, id),
        _ => None,
    }
}

trait HitOnly {
    fn visible_or_hit(&self) -> bool;
}
impl HitOnly for Control {
    // Key-only controls have a zero box and never contain a point.
    fn visible_or_hit(&self) -> bool {
        self.enabled
    }
}

fn key(s: &mut ControlsScreen, ctrls: &[Control], ctx: &mut FrontCtx, vk: u16) -> Option<Trigger> {
    let c = ctrls.iter().find(|c| c.takes_key(vk)).expect("key control");
    match c.action {
        Action::Custom(id) => s.action(ctx, id),
        _ => None,
    }
}

#[test]
fn flow_returns_to_options() {
    for t in [Trigger::Exit, Trigger::Ok] {
        assert_eq!(next(CONTROLS, t, FlowCtx::default()), Next::Screen(OPTIONS));
    }
}

#[test]
fn flow_options_opens_configure_controls() {
    assert_eq!(
        next(OPTIONS, Trigger::ConfigureControls, FlowCtx::default()),
        Next::Screen(CONTROLS)
    );
    assert_eq!(
        next(CONTROLS, Trigger::ConfigureControls, FlowCtx::default()),
        Next::Stay
    );
}

#[test]
fn screen_click_key_accept_persists_and_cancel_does_not() {
    let dir = std::env::temp_dir().join(format!("d2rust-cfg-s-{}", std::process::id()));
    let path = dir.join("controls.toml");
    let mut flow = FlowCtx::default();
    let mut settings = SettingsSlot::default();
    let mut ctx = FrontCtx {
        expansion: true,
        flow: &mut flow,
        now_ms: 0,
        settings: &mut settings,
    };
    // Cancel after an edit: nothing written.
    let mut s = ControlsScreen::new(Some(path.clone()));
    let ctrls = s.build(&mut ctx);
    // Row 1 (Inventory), column One.
    assert_eq!(
        click_screen(&mut s, &ctrls, &mut ctx, 150, 70 + 59 + 5),
        None
    );
    assert!(s.model().borrow().editing());
    key(&mut s, &ctrls, &mut ctx, 0x43);
    assert_eq!(s.model().borrow().table().key_of(1, 1), 0x43);
    assert_eq!(
        click_screen(&mut s, &ctrls, &mut ctx, 193, 70 + 351),
        Some(Trigger::Exit)
    );
    assert!(!path.exists());
    assert_eq!(s.model().borrow().table(), &BindingTable::defaults());
    // Accept after an edit: written, reloaded on the next entry.
    click_screen(&mut s, &ctrls, &mut ctx, 150, 70 + 59 + 5);
    key(&mut s, &ctrls, &mut ctx, 0x43);
    assert_eq!(
        click_screen(&mut s, &ctrls, &mut ctx, 605, 70 + 351),
        Some(Trigger::Ok)
    );
    assert!(path.is_file());
    let mut s2 = ControlsScreen::new(Some(path));
    s2.build(&mut ctx);
    assert_eq!(s2.model().borrow().table().key_of(1, 1), 0x43);
    // Esc (not editing) cancels.
    let ctrls = s2.build(&mut ctx);
    assert_eq!(key(&mut s2, &ctrls, &mut ctx, 27), Some(Trigger::Exit));
    std::fs::remove_dir_all(&dir).ok();
}

/// The game's known layout of the options screen:
/// 21 track frames, and the button labels
/// light up exactly over Cancel x 161..=225, Default 364..=434, Accept
/// 573..=637, y 399..=437.
#[test]
fn recorded_track_and_button_hover() {
    let mut c = open();
    let tracks = c
        .draw_list(0, None)
        .iter()
        .filter(|i| matches!(i, CfgDraw::Slider { frame: 13, .. }))
        .count();
    assert_eq!(tracks, 21);
    let lit = |c: &mut ConfigureControls, x: i32, y: i32| -> Vec<bool> {
        c.draw_list(0, Some((x, y)))
            .iter()
            .filter_map(|i| match i {
                CfgDraw::Text {
                    string_id: 3972..=3974,
                    y: 421,
                    color,
                    ..
                } => Some(*color != color::GOLD),
                _ => None,
            })
            .collect()
    };
    // Draw order: Cancel, Default, Accept.
    for (x, want) in [
        (160, [false, false, false]),
        (161, [true, false, false]),
        (225, [true, false, false]),
        (226, [false, false, false]),
        (363, [false, false, false]),
        (364, [false, true, false]),
        (434, [false, true, false]),
        (435, [false, false, false]),
        (572, [false, false, false]),
        (573, [false, false, true]),
        (637, [false, false, true]),
        (638, [false, false, false]),
    ] {
        assert_eq!(lit(&mut c, x, 400), want, "x {x}");
    }
    assert_eq!(lit(&mut c, 193, 398), [false; 3]);
    assert_eq!(lit(&mut c, 193, 399), [true, false, false]);
    assert_eq!(lit(&mut c, 193, 437), [true, false, false]);
    assert_eq!(lit(&mut c, 193, 438), [false; 3]);
}
