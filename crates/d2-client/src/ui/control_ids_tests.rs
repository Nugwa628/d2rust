//! T9 control IDs resolve to the existing pointer handlers at both frame widths.
use super::*;
use crate::ui::ui_ids::{self, UiElement};

struct Strings;
impl crate::ui::StringLookup for Strings {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&[u16]> {
        match id {
            4035 => Some(&[77]),
            4144 => Some(&[67]),
            _ => None,
        }
    }
}
fn elements(u: &Ui, w: &ClientWorld) -> Vec<UiElement> {
    u.root.ui_elements(&UiCtx {
        tick: 0,
        world: w,
        strings: &Strings,
    })
}
fn click_id(u: &mut Ui, w: &ClientWorld, id: &str) {
    let e = ui_ids::find(&elements(u, w), id).unwrap().clone();
    // The host adds ui_left to listed coordinates and removes it for dispatch.
    let left = u.ui.shared.borrow().config.screen.left;
    let frame_at = Point::new(e.at.x + left, e.at.y);
    let at = Point::new(frame_at.x - left, frame_at.y);
    for ev in [
        UiEvent::CursorMoved(at),
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    ] {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &Strings,
        };
        u.ui.before_event(ev, w);
        let routed = u.root.dispatch(ev, &ctx);
        u.ui.after_event(&mut u.root, ev, routed).unwrap();
    }
}
fn open(u: &mut Ui, id: u8) {
    u.ui.set_ui(u32::from(id), 0, false).unwrap();
    u.ui.sync_root(&mut u.root);
}
#[test]
fn hud_ids_reach_run_globe_and_skill_handlers_at_both_widths() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, Some(areas()), true);
        let w = world(AMAZON, 1, true);
        click_id(&mut u, &w, "hud.run");
        assert_eq!(u.ui.sync_run(false), 1);
        for (id, hp) in [("hud.hp_text", true), ("hud.mp_text", false)] {
            let before = u.ui.shared.borrow().esc.menu.settings;
            click_id(&mut u, &w, id);
            let sh = u.ui.shared.borrow();
            // The globe text is drawn from the input flags, and follows the
            // setting at once (not only after the settings are loaded again).
            if hp {
                assert_ne!(before.show_hp_text, sh.esc.menu.settings.show_hp_text);
                assert_eq!(sh.hud.input.show_hp, sh.esc.menu.settings.show_hp_text != 0);
            } else {
                assert_ne!(before.show_mp_text, sh.esc.menu.settings.show_mp_text);
                assert_eq!(sh.hud.input.show_mp, sh.esc.menu.settings.show_mp_text != 0);
            }
        }
        for (id, left) in [("hud.skill_left", true), ("hud.skill_right", false)] {
            if u.ui.is_open(3) {
                u.ui.set_ui(3, 1, false).unwrap();
                u.ui.sync_root(&mut u.root);
            }
            click_id(&mut u, &w, id);
            assert!(u.ui.is_open(3));
            assert_eq!(u.ui.shared.borrow().hud.select_left, left);
        }
        let dead = world(AMAZON, 17, true);
        assert!(!elements(&u, &dead).iter().any(|e| e.id.starts_with("hud.")));
    }
}
#[test]
fn minipanel_ids_reach_all_existing_functions_at_both_widths() {
    for screen in [Screen::R800, Screen::WIDE] {
        for (name, target) in [
            ("character", 2),
            ("inventory", 1),
            ("skills", 4),
            ("party", 0x16),
            ("automap", 0x0a),
            ("message_log", 0x18),
            ("quests", 0x0f),
            ("game_menu", 9),
        ] {
            let mut u = ui_at(screen, Some(areas()), true);
            let w = world(AMAZON, 1, true);
            u.ui.shared.borrow_mut().hud.mini =
                crate::ui::panels::control::minipanel::MiniPanel::new(1);
            u.ui.set_ui(0x15, 1, false).unwrap();
            u.ui.sync_root(&mut u.root);
            click_id(&mut u, &w, "hud.minipanel");
            assert!(u.ui.is_open(0x15));
            u.images(&w); // Input uses the last drawn layout.
            click_id(&mut u, &w, &format!("minipanel.{name}"));
            assert!(u.ui.is_open(target), "{name}");
            if name == "quests" {
                assert_eq!(u.root.take_intents(), [quest_log_ui::request_quest_data()]);
            }
        }
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("minipanel.")));
        open(&mut u, 0x15);
        u.ui.shared.borrow_mut().hud.mini.hidden = true;
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("minipanel.")));
        let els = elements(&u, &w);
        let err = ui_ids::find(&els, "unknown.control").unwrap_err();
        assert!(err.contains("unknown.control") && err.contains("hud.run"));
    }
}
#[test]
fn belt_ids_reach_existing_item_intents_at_both_widths() {
    use crate::bridge::items::mode;
    use crate::ui::panels::control::belt::{BeltBox, BeltRecord};
    use crate::ui::panels::inv_items::tests::world as item_world;
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let boxes = (0..8)
            .map(|i| BeltBox {
                left: 430 + 31 * (i % 4),
                right: 458 + 31 * (i % 4),
                top: 562 - 31 * (i / 4),
                bottom: 590 - 31 * (i / 4),
            })
            .collect();
        u.ui.set_belt_parts(hud_belt::BeltParts {
            records: vec![BeltRecord { boxes }; 14],
            ..Default::default()
        });
        let mut w = item_world(
            &[
                (7, mode::BELT, (0, 0, 0, 0), b"hp1 "),
                (8, mode::BELT, (0, 4, 0, 0), b"mp1 "),
            ],
            None,
        );
        let me = w.local_player.unwrap();
        w.units.get_mut(&me).unwrap().mode = 1;
        assert!(!elements(&u, &w).iter().any(|e| e.id == "belt.slot(4)"));
        click_id(&mut u, &w, "belt.slot(0)");
        assert_eq!(
            u.root.take_intents(),
            [crate::ui::messages::msg_u32s(0x24, &[7])]
        );
        {
            let mut sh = u.ui.shared.borrow_mut();
            sh.hud.belt.state.belt_type = 1;
            sh.hud.belt.state.extra_boxes = true;
            sh.hud.belt.state.popped = true;
        }
        // The open minipanel owns input above the HUD strip. Close it
        // before exposing the second row of a two-row belt.
        assert!(!elements(&u, &w).iter().any(|e| e.id == "belt.slot(4)"));
        u.ui.set_ui(0x15, 1, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(elements(&u, &w).iter().any(|e| e.id == "belt.slot(4)"));
        click_id(&mut u, &w, "belt.slot(4)");
        assert_eq!(
            u.root.take_intents(),
            [crate::ui::messages::msg_u32s(0x24, &[8])]
        );
    }
}
#[test]
fn help_close_button_closes_the_help_screen_at_both_widths() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, Some(areas()), true);
        let w = world(AMAZON, 1, true);
        assert!(!elements(&u, &w).iter().any(|e| e.id == "help.close"));
        open(&mut u, super::help_screen::STATE);
        assert!(u.ui.is_open(super::help_screen::STATE));
        assert!(elements(&u, &w).iter().any(|e| e.id == "help.close"));
        click_id(&mut u, &w, "help.close");
        assert!(!u.ui.is_open(super::help_screen::STATE));
        assert!(!elements(&u, &w).iter().any(|e| e.id == "help.close"));
    }
}
#[test]
fn message_log_ids_scroll_and_close_at_both_widths() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("message_log.")));
        for _ in 0..50 {
            u.ui.shared
                .borrow_mut()
                .messages
                .add(&[65], 0, 0, 800, false, None);
        }
        open(&mut u, 0x18);
        for id in ["message_log.scroll_down", "message_log.scroll_up"] {
            click_id(&mut u, &w, id);
            // A successful step queues the existing silent sound request.
            assert!(u.ui.outcome.sounds.contains(&SoundRequest::Ui(0)));
        }
        // No two of the log's elements overlap, and a click on the scroll
        // track between the arrows leaves the log open.
        let els = elements(&u, &w);
        for (i, a) in els.iter().enumerate() {
            for b in &els[i + 1..] {
                let apart = a.rect.right() <= i64::from(b.rect.x)
                    || b.rect.right() <= i64::from(a.rect.x)
                    || a.rect.bottom() <= i64::from(b.rect.y)
                    || b.rect.bottom() <= i64::from(a.rect.y);
                assert!(apart, "{} overlaps {}", a.id, b.id);
            }
        }
        let up = ui_ids::find(&els, "message_log.scroll_up").unwrap().rect;
        let down = ui_ids::find(&els, "message_log.scroll_down").unwrap().rect;
        let track = Point::new(up.x + 7, (up.y + down.y) / 2);
        for ev in [
            UiEvent::CursorMoved(track),
            UiEvent::Press {
                button: PointerButton::Left,
                at: track,
            },
            UiEvent::Release {
                button: PointerButton::Left,
                at: track,
            },
        ] {
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &Strings,
            };
            u.ui.before_event(ev, &w);
            let routed = u.root.dispatch(ev, &ctx);
            u.ui.after_event(&mut u.root, ev, routed).unwrap();
        }
        assert!(u.ui.is_open(0x18));
        click_id(&mut u, &w, "message_log.close");
        assert!(!u.ui.is_open(0x18));
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("message_log.")));
    }
}

#[test]
fn controls_overlay_ids_use_the_current_cells_and_finish_handlers() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        u.key(&w, Action::GameMenu);
        assert!(u.ui.is_open(9));
        u.ui.shared.borrow_mut().esc.controls = Some(controls_host::ControlsHost::open(true, None));
        let e = elements(&u, &w)
            .into_iter()
            .find(|e| e.id.starts_with("controls.binding("))
            .unwrap();
        click_id(&mut u, &w, &e.id);
        assert!(u
            .ui
            .shared
            .borrow()
            .esc
            .controls
            .as_ref()
            .unwrap()
            .model()
            .editing());
        click_id(&mut u, &w, "controls.default");
        assert!(!u
            .ui
            .shared
            .borrow()
            .esc
            .controls
            .as_ref()
            .unwrap()
            .model()
            .editing());
        let before = elements(&u, &w);
        click_id(&mut u, &w, "controls.scroll_down");
        assert_ne!(
            before.iter().map(|e| &e.id).collect::<Vec<_>>(),
            elements(&u, &w).iter().map(|e| &e.id).collect::<Vec<_>>()
        );
        click_id(&mut u, &w, "controls.scroll_up");
        assert_eq!(before, elements(&u, &w));
        click_id(&mut u, &w, "controls.cancel");
        assert!(u.ui.shared.borrow().esc.controls.is_none());
        u.ui.shared.borrow_mut().esc.controls = Some(controls_host::ControlsHost::open(true, None));
        click_id(&mut u, &w, "controls.accept");
        assert!(u.ui.take_accepted_bindings().is_some());
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("controls.")));
    }
}
#[test]
fn quest_ids_select_existing_rows_and_close_at_both_widths() {
    use crate::ui::panel::Panel;
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        let mut status = [0; 41];
        status[1] = 3;
        u.ui.apply_output(&Output::QuestLog { status }, &w).unwrap();
        open(&mut u, 0x0f);
        let mut panel = quest_log_ui::QuestLogUi {
            sh: u.ui.shared.clone(),
            log: Default::default(),
            shown: Default::default(),
            close_down: Default::default(),
        };
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &Strings,
        };
        let els = panel.ui_elements(&ctx);
        let id = &els
            .iter()
            .find(|e| e.id.starts_with("quests.slot("))
            .unwrap()
            .id;
        let e = ui_ids::find(&els, id).unwrap();
        panel.log.borrow_mut().selected = None;
        panel.event(
            UiEvent::Press {
                button: PointerButton::Left,
                at: e.at,
            },
            &ctx,
        );
        assert!(panel.log.borrow().selected.is_some());
        click_id(&mut u, &w, "quests.close");
        assert!(!u.ui.is_open(0x0f));
        assert!(!elements(&u, &w).iter().any(|e| e.id.starts_with("quests.")));
    }
}
#[test]
fn unspent_buttons_reach_the_existing_handlers() {
    for screen in [Screen::R800, Screen::WIDE] {
        for (name, stat, target) in [("hud.new_stats", 4, 2), ("hud.new_skills", 5, 4)] {
            let mut u = ui_at(screen, None, true);
            let mut w = world(AMAZON, 1, true);
            assert!(!elements(&u, &w).iter().any(|e| e.id == name));
            let k = w.local_player.unwrap();
            w.units.get_mut(&k).unwrap().stats.insert(stat, 1);
            click_id(&mut u, &w, name);
            assert!(u.ui.is_open(target));
        }
    }
}

#[test]
fn help_id_uses_the_existing_help_button_handler() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        u.ui.set_ui(0x22, 1, false).unwrap();
        u.ui.sync_root(&mut u.root);
        assert!(!elements(&u, &w).iter().any(|e| e.id == "hud.help"));
        open(&mut u, 0x22);
        u.images(&w);
        click_id(&mut u, &w, "hud.help");
        assert!(!u.ui.is_open(0x22));
        assert!(u.ui.is_open(0x21));
    }
}
struct Cover(crate::ui::Rect);
impl crate::ui::panel::Panel for Cover {
    fn id(&self) -> crate::ui::PanelId {
        crate::ui::PanelId(0x700)
    }
    fn rect(&self) -> crate::ui::Rect {
        self.0
    }
    fn draw(&self, _: &UiCtx, _: &mut dyn crate::ui::UiDrawSink) {}
    fn hit(&self, _: Point) -> Option<crate::ui::WidgetId> {
        Some(crate::ui::WidgetId(1))
    }
    fn event(&mut self, _: UiEvent, _: &UiCtx) -> crate::ui::UiResponse {
        crate::ui::UiResponse::Consumed
    }
}
#[test]
fn covered_control_ids_are_not_listed() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        let e = ui_ids::find(&elements(&u, &w), "hud.run").unwrap().clone();
        u.root.add(Box::new(Cover(e.rect))).unwrap();
        u.root.open(crate::ui::PanelId(0x700)).unwrap();
        assert!(ui_ids::find(&elements(&u, &w), "hud.run").is_err());
    }
}
/// The Esc menu is modal: it takes the press on the run button (no
/// toggle, not even once the menu closes), so the HUD lists none of its
/// elements while the menu is open; closed, the run button toggles again.
#[test]
fn hud_lists_nothing_under_the_esc_menu() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, Some(areas()), true);
        let w = world(AMAZON, 1, true);
        let run = ui_ids::find(&elements(&u, &w), "hud.run").unwrap().clone();
        u.key(&w, Action::GameMenu);
        assert!(u.ui.is_open(9));
        assert!(!elements(&u, &w).iter().any(|e| e.id.starts_with("hud.")));
        for ev in [
            UiEvent::CursorMoved(run.at),
            UiEvent::Press {
                button: PointerButton::Left,
                at: run.at,
            },
            UiEvent::Release {
                button: PointerButton::Left,
                at: run.at,
            },
        ] {
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &Strings,
            };
            u.ui.before_event(ev, &w);
            let routed = u.root.dispatch(ev, &ctx);
            u.ui.after_event(&mut u.root, ev, routed).unwrap();
        }
        assert!(u.ui.is_open(9));
        assert_eq!(u.ui.sync_run(false), 0);
        u.key(&w, Action::GameMenu);
        assert!(!u.ui.is_open(9));
        assert_eq!(u.ui.sync_run(false), 0);
        click_id(&mut u, &w, "hud.run");
        assert_eq!(u.ui.sync_run(false), 1);
    }
}
/// With a log longer than the list, the arrows are idle only at their end
/// of it: the up arrow at the newest entry, the down arrow at the oldest,
/// and each click on an arrow that is not idle steps.
#[test]
fn message_log_arrows_are_idle_only_at_the_list_ends() {
    for screen in [Screen::R800, Screen::WIDE] {
        let mut u = ui_at(screen, None, true);
        let w = world(AMAZON, 1, true);
        for _ in 0..50 {
            u.ui.shared
                .borrow_mut()
                .messages
                .add(&[65], 0, 0, 800, false, None);
        }
        open(&mut u, 0x18);
        let idle = |u: &Ui, id: &str| ui_ids::find(&elements(u, &w), id).unwrap().idle;
        assert!(idle(&u, "message_log.scroll_up").is_some());
        assert!(idle(&u, "message_log.scroll_down").is_none());
        let mut steps = 0;
        while idle(&u, "message_log.scroll_down").is_none() {
            u.ui.outcome.sounds.clear();
            click_id(&mut u, &w, "message_log.scroll_down");
            assert!(u.ui.outcome.sounds.contains(&SoundRequest::Ui(0)));
            steps += 1;
            assert!(steps < 50, "the down arrow never reaches the end");
        }
        assert!(steps > 1);
        assert!(idle(&u, "message_log.scroll_up").is_none());
        // At the end the down arrow steps nothing.
        u.ui.outcome.sounds.clear();
        click_id(&mut u, &w, "message_log.scroll_down");
        assert!(!u.ui.outcome.sounds.contains(&SoundRequest::Ui(0)));
        click_id(&mut u, &w, "message_log.scroll_up");
        assert!(idle(&u, "message_log.scroll_down").is_none());
    }
}
