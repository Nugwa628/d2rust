use super::*;

// T9: resolve the current frame's ids, then use the normal root event path.
#[test]
fn character_and_skill_ids_route_at_both_frame_sizes() {
    use crate::ui::ui_ids::{find, UiElement};
    fn elements(u: &Ui, w: &ClientWorld) -> Vec<UiElement> {
        u.root.ui_elements(&UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        })
    }
    fn click_id(u: &mut Ui, w: &ClientWorld, id: &str) {
        u.images(w);
        let at = find(&elements(u, w), id).unwrap().at;
        u.send(w, UiEvent::CursorMoved(at));
        u.click(w, at);
    }
    for screen in [Screen::R800, Screen::WIDE] {
        let mut w = world(AMAZON, 1, true);
        let key = w.local_player.unwrap();
        w.units
            .get_mut(&key)
            .unwrap()
            .stats
            .extend([(4, 4), (5, 2), (12, 5)]);
        // Base points gate the character buttons, just as for physical clicks.
        let mut u = ui_at(screen, Some(areas()), true);
        let mut fonts = FontMeasure::default();
        fonts.insert(1, FontTable::parse(&character_bind_tests::tbl(6)).unwrap());
        u.key(&w, Action::ToggleCharacter);
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("character.stat(")));
        u.ui.set_fonts(fonts);
        for stat in [0, 2, 3, 1] {
            click_id(&mut u, &w, &format!("character.stat({stat})"));
            assert_eq!(
                u.root.take_intents(),
                vec![character::add_stat_point(stat, 1)]
            );
        }
        w.units.get_mut(&key).unwrap().stats.insert(4, 0);
        let ids = elements(&u, &w);
        assert_eq!(
            ids.iter()
                .filter(|e| e.id.starts_with("character."))
                .count(),
            1
        );
        let error = find(&ids, "character.stat(99)").unwrap_err();
        assert!(error.contains("on screen:"));
        for e in &ids {
            assert!(error.contains(&e.id));
        }
        click_id(&mut u, &w, "character.close");
        assert!(!u.ui.is_open(UI_CHARACTER));
        let mut tables = tree_tables();
        let mut passive = tables.rows[2].clone();
        passive.skill = 9;
        passive.passive = true;
        passive.column = 1;
        tables.rows.push(passive);
        let mut bottom = tables.rows[2].clone();
        bottom.skill = 10;
        bottom.row = 6;
        tables.rows.push(bottom);
        u.ui.set_skill_tree_tables(tables);
        u.key(&w, Action::ToggleSkillTree);
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("skilltree.")));
        u.images(&w);
        let ids = elements(&u, &w);
        assert!(find(&ids, "skilltree.skill(6)").is_ok());
        assert!(find(&ids, "skilltree.skill(7)").is_err()); // level requirement
        assert!(find(&ids, "skilltree.skill(8)").is_err()); // hidden page

        // Points can disappear between press and release. The existing
        // handler keeps that icon pressed and cannot select it without points.
        let at = find(&ids, "skilltree.skill(6)").unwrap().at;
        u.send(
            &w,
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            },
        );
        w.units.get_mut(&key).unwrap().stats.insert(5, 0);
        u.send(
            &w,
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            },
        );
        assert!(find(&elements(&u, &w), "skilltree.skill(6)").is_err());
        assert!(u.root.take_intents().is_empty());
        w.units.get_mut(&key).unwrap().stats.insert(5, 2);
        click_id(&mut u, &w, "skilltree.skill(6)");
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(
                &d2_proto::client::AddSkillPoint { skill: 6 }
            )]
        );
        click_id(&mut u, &w, "skilltree.tab(2)");
        assert!(find(&elements(&u, &w), "skilltree.skill(6)").is_err());
        click_id(&mut u, &w, "skilltree.skill(8)");
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(
                &d2_proto::client::AddSkillPoint { skill: 8 }
            )]
        );
        // The bottom-row icon remains clickable above the close button.
        click_id(&mut u, &w, "skilltree.skill(10)");
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(
                &d2_proto::client::AddSkillPoint { skill: 10 }
            )]
        );
        click_id(&mut u, &w, "skilltree.tab(3)");
        assert!(!elements(&u, &w)
            .iter()
            .any(|e| e.id.starts_with("skilltree.skill(")));
        click_id(&mut u, &w, "skilltree.close");
        assert!(!u.ui.is_open(UI_SKILLTREE));
        u.key(&w, Action::ToggleSkillTree);
        click_id(&mut u, &w, "skilltree.tab(2)");
        w.units.get_mut(&key).unwrap().stats.insert(5, 0);
        assert!(find(&elements(&u, &w), "skilltree.skill(9)").is_err());
        // The close art overlaps the bottom-row icon. Closing without
        // free points must not also select that skill on mouse down.
        click_id(&mut u, &w, "skilltree.close");
        assert!(!u.ui.is_open(UI_SKILLTREE));
        assert!(u.root.take_intents().is_empty());
        u.key(&w, Action::ToggleSkillTree);
        click_id(&mut u, &w, "skilltree.skill(10)");
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(&d2_proto::client::SelectSkill {
                skill: 10,
                left: false,
                item: u32::MAX
            })]
        );
        assert!(!u.ui.is_open(UI_SKILLTREE));
        u.key(&w, Action::ToggleSkillTree);
        click_id(&mut u, &w, "skilltree.skill(8)");
        assert_eq!(
            u.root.take_intents(),
            vec![ClientIntent::from_message(&d2_proto::client::SelectSkill {
                skill: 8,
                left: false,
                item: u32::MAX
            })]
        );
        assert!(!u.ui.is_open(UI_SKILLTREE));
    }
}
