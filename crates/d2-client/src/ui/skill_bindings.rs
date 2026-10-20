//! Assign keys over the skill picker through the existing saved controls
//! and skill slots. Conflicts are visible and leave other actions intact.

use super::OriginalUi;
use crate::bridge::world::ClientWorld;
use crate::controls::{Action, Bindings, Context, ControlsFile, Key, Preset};
use crate::ui::geom::Point;
use crate::ui::original::hud_hotkeys::{self as hk, Hotkeys, Record};
use std::path::Path;

#[derive(Debug)]
pub enum SkillKeyResult {
    Ignored,
    Consumed,
    Assigned { action: Action, bindings: Bindings },
}

fn slot_action(slot: usize) -> Action {
    crate::controls::keymap::action_of_cmd(i32::from(hk::slot_cmd(slot))).expect("skill action")
}

fn prepare(
    bindings: &Bindings,
    hotkeys: &Hotkeys,
    key: Key,
    record: Record,
    expansion: bool,
) -> Result<(usize, Bindings), String> {
    let count = if expansion { hk::SLOTS } else { 8 };
    let existing = bindings
        .action_for(Context::World, key)
        .or_else(|| bindings.action_for(Context::World, crate::ui::edge::left_side(key)?));
    let slot = if let Some(action) = existing {
        (0..count)
            .find(|&i| slot_action(i) == action)
            .ok_or_else(|| {
                format!(
                    "{} is already bound to {}. Change it in Configure Controls.",
                    key.name(),
                    action.name()
                )
            })?
    } else {
        (0..count)
            .find(|&i| {
                let h = hotkeys.slots[i];
                h.skill == record.skill && h.owner == record.owner
            })
            .or_else(|| (0..count).find(|&i| hotkeys.slots[i].skill < 0))
            .ok_or_else(|| {
                "All skill hotkey slots are assigned. Reuse a skill hotkey to replace one."
                    .to_string()
            })?
    };
    let mut next = bindings.clone();
    if !next.inputs(slot_action(slot)).contains(&key) {
        next.set(slot_action(slot), &[key]);
    }
    Ok((slot, next))
}

impl OriginalUi {
    /// Capture an allowed input over a skill. The resulting action goes
    /// through the HUD's normal assignment and server save path.
    pub fn bind_skill_key(
        &mut self,
        world: &ClientWorld,
        bindings: &Bindings,
        key: Key,
        mouse: Point,
        path: Option<&Path>,
    ) -> SkillKeyResult {
        if self.key_mode() != 1 {
            return SkillKeyResult::Ignored;
        }
        let mut sh = self.shared.borrow_mut();
        if !sh.states.is_open(crate::ui::original::hud::UI_SKILL_SELECT)
            || matches!(
                key,
                Key::Escape
                    | Key::MouseLeft
                    | Key::MouseRight
                    | Key::MouseWheelUp
                    | Key::MouseWheelDown
            )
            || world.local().is_none_or(|u| u.is_dead())
        {
            return SkillKeyResult::Ignored;
        }
        let icons = crate::ui::original::hud::select_icons(
            world,
            &sh.hud.tables,
            sh.hud.select_left,
            sh.config.screen.w,
            sh.config.screen.h,
        );
        let Some(record) = hk::records(&icons)
            .into_iter()
            .take(icons.len())
            .find(|r| (r.x..=r.x + 48).contains(&mouse.x) && (r.y - 48..=r.y).contains(&mouse.y))
        else {
            return SkillKeyResult::Ignored;
        };
        let result = prepare(
            bindings,
            &sh.hud.hotkeys,
            key,
            record,
            sh.facts.expansion_game,
        )
        .and_then(|(slot, next)| {
            if next != *bindings {
                if let Some(path) = path {
                    let file =
                        ControlsFile::from_effective(Preset::Original, &next).expect("preset");
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Could not save skill key: {e}"))?;
                    }
                    std::fs::write(path, crate::controls::write(&file))
                        .map_err(|e| format!("Could not save skill key: {e}"))?;
                }
            }
            Ok((slot, next))
        });
        match result {
            Ok((slot, next)) => SkillKeyResult::Assigned {
                action: slot_action(slot),
                bindings: next,
            },
            Err(message) => {
                let text: Vec<u16> = message.encode_utf16().collect();
                let width = sh.config.screen.w;
                let log_open = sh.states.is_open(0x18);
                let fonts = sh.fonts.clone();
                let now = sh.ui_ms32(world.frames);
                sh.messages
                    .add(&text, 1, now, width, log_open, fonts.as_ref());
                SkillKeyResult::Consumed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RECORD: Record = Record {
        x: 10,
        y: 100,
        skill: 47,
        owner: u32::MAX,
    };

    #[test]
    fn arbitrary_keys_bind_and_saved_controls_round_trip() {
        for key in [
            Key::J,
            Key::Digit7,
            Key::F12,
            Key::Numpad3,
            Key::Mouse4,
            Key::LeftCtrl,
            Key::RightCtrl,
            Key::NumpadEnter,
            Key::Up,
        ] {
            let bindings = Bindings::empty();
            let (slot, next) = prepare(&bindings, &Hotkeys::default(), key, RECORD, true).unwrap();
            assert_eq!(
                next.action_for(Context::World, key),
                Some(slot_action(slot))
            );
            let file = ControlsFile::from_effective(Preset::Original, &next).unwrap();
            let (_, loaded) = crate::controls::parse(&crate::controls::write(&file)).unwrap();
            assert_eq!(loaded, next);
        }
    }

    #[test]
    fn conflicts_are_reported_and_existing_bindings_stay_intact() {
        let bindings = Preset::Original.bindings().unwrap();
        let before = bindings.clone();
        let error = prepare(&bindings, &Hotkeys::default(), Key::I, RECORD, true).unwrap_err();
        assert!(error.contains(Action::ToggleInventory.name()));
        assert_eq!(bindings, before);
        let error =
            prepare(&bindings, &Hotkeys::default(), Key::RightCtrl, RECORD, true).unwrap_err();
        assert!(
            error.contains(Action::Run.name()),
            "modifier aliases conflict too"
        );
    }

    #[test]
    fn old_function_keys_and_existing_skill_slots_keep_their_assignments() {
        let bindings = Preset::Original.bindings().unwrap();
        let (slot, next) = prepare(&bindings, &Hotkeys::default(), Key::F1, RECORD, true).unwrap();
        assert_eq!(slot, 0);
        assert_eq!(next, bindings);
        let mut hotkeys = Hotkeys::default();
        hotkeys.slots[5] = hk::Hotkey {
            skill: RECORD.skill,
            owner: RECORD.owner,
            left: false,
        };
        let (slot, _) = prepare(&bindings, &hotkeys, Key::J, RECORD, true).unwrap();
        assert_eq!(slot, 5);
    }
}
