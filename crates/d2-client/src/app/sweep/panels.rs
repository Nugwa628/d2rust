//! Reachable panel flag combinations from the client's conflict and gate table.
use super::content::Catalog;
use super::*;
use crate::{
    controls::Action,
    ui::{
        geom::Point,
        layout::Screen,
        states::{GateEnv, PlayerLife, UiStates},
    },
};
use std::collections::{BTreeSet, VecDeque};

fn mask(states: &UiStates) -> u64 {
    states
        .flags()
        .iter()
        .enumerate()
        .fold(0, |mask, (i, &open)| mask | (u64::from(open) << i))
}
/// The single-player toggle actions and the UI state each one opens.
/// The Party screen is multiplayer only (its mini-panel button is inert in
/// single player and the P key has no Party handler), so it is not a
/// reachable headless toggle here; the state is counted as a skip.
fn single_player_actions(has_belt: bool) -> Vec<(Action, u8)> {
    let mut actions = vec![
        (Action::ToggleInventory, 1),
        (Action::ToggleCharacter, 2),
        (Action::ToggleSkillMenuRight, 3),
        (Action::ToggleSkillTree, 4),
        (Action::ToggleAutomap, 10),
        (Action::ToggleQuests, 15),
        (Action::ToggleMessageLog, 24),
        (Action::ToggleHelp, 33),
    ];
    if has_belt {
        actions.push((Action::ToggleBelt, 31));
    }
    actions
}

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let catalog = Catalog::load(live)?;
    let belt = catalog.items.iter().find(|i| catalog.fits(i, 0, 8));
    let actions = single_player_actions(belt.is_some());
    let mut setup = String::new();
    if let Some(belt) = belt {
        let id = belt.class;
        setup=format!("at 0 poke item {} @x @y quality normal ilvl 99\nat 4 msg PickItem type=4 id=@4:{id} cursor=1\nat 8 msg EquipItem item=@4:{id} bodyloc=8\n",belt.code);
    } else {
        g.skip("panels:belt-action-needs-worn-belt");
    }
    let states = UiStates::new()?;
    for (id, _) in states.rows().iter().enumerate() {
        if id > 0 && id != 9 && !actions.iter().any(|&(_, ui)| usize::from(ui) == id) {
            g.skip("panels:context-only-state-without-headless-toggle-action");
        }
    }
    let mut seen = BTreeSet::from([0]);
    let mut pending = VecDeque::from([(states, Vec::<usize>::new())]);
    while let Some((states, path)) = pending.pop_front() {
        let bits = mask(&states);
        if bits != 0 {
            let mut steps = setup.clone();
            let mut tick = 16;
            for &index in &path {
                steps.push_str(&format!(
                    "at {tick} input action {}\n",
                    actions[index].0.name()
                ));
                tick += 4;
            }
            for &(_, ui) in &actions {
                steps.push_str(&format!(
                    "at {tick} check ui.open({ui}) == {}\n",
                    bits & (1u64 << ui) != 0
                ));
            }
            tick += 4;
            for &(action, ui) in actions.iter().rev() {
                if bits & (1u64 << ui) != 0 {
                    steps.push_str(&format!("at {tick} input action {}\n", action.name()));
                    tick += 4;
                }
            }
            for &(_, ui) in &actions {
                steps.push_str(&format!("at {tick} check ui.open({ui}) == false\n"));
            }
            g.add(
                "panels",
                format!("combination-{bits:010x}"),
                "char class 0",
                &steps,
                tick + 4,
            )?;
        }
        for (index, &(_, ui)) in actions.iter().enumerate() {
            let mut next = states.clone();
            let mut env = GateEnv {
                expansion: true,
                player: Some(PlayerLife {
                    alive: true,
                    dead: false,
                }),
                chat_blocked: false,
                input_hold: false,
                modal_text: false,
                npc_active: false,
                screen: Screen::R800,
                mouse: Point::new(0, 0),
            };
            if next.set(u32::from(ui), 2, false, &mut env, &mut Vec::new())?
                && seen.insert(mask(&next))
            {
                let mut path = path.clone();
                path.push(index);
                pending.push_back((next, path));
            }
        }
    }
    g.add("panels","action-game-menu".into(),"char class 0","at 4 input wkey ESC\nat 4 input wkey ESC\nat 4 check ui.open(9) == true\nat 16 check ui.open(9) == false",20)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_party_screen_is_not_a_single_player_toggle() {
        for has_belt in [false, true] {
            let actions = single_player_actions(has_belt);
            assert!(!actions.iter().any(|&(a, _)| a == Action::ToggleParty));
            assert!(actions.iter().any(|&(a, _)| a == Action::ToggleInventory));
            assert_eq!(
                actions.iter().any(|&(a, _)| a == Action::ToggleBelt),
                has_belt
            );
        }
    }
}
