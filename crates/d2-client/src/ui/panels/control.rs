//! The control panel overlays (the border panel owns the border and the
//! base art, [`super::border`]; this module owns everything drawn on top
//! of it and the control panel input): the draw order, the art
//! files; the rest is in the submodules.
//!
//! | Module | Rules |
//! |---|---|
//! | [`globes`] | globes, experience and stamina bars |
//! | [`belt`] | the belt |
//! | [`buttons`] | run / menu buttons, skill buttons, new stats / skills |
//! | [`minipanel`] | the mini panel |
//! | [`input`] | mouse input |

pub mod belt;
pub mod buttons;
pub mod globes;
pub mod input;
pub mod minipanel;

/// The steps of the control panel draw, in order (the border and
/// the base art are [`super::border`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawStep {
    /// Resolution mode 2 only: the left / right border.
    Border,
    /// The control panel base art.
    BaseArt,
    LifeGlobe,
    ManaGlobe,
    ExperienceBar,
    RunButton,
    StaminaBar,
    MenuButton,
    Belt,
    SkillButtons,
    BeltItemHover,
    /// The level-change timer.
    LevelTimer,
    /// The mini-panel menu button tool tip.
    MenuTip,
}

/// R1–r5: step 7 of the UI pass.
pub const DRAW_ORDER: [DrawStep; 13] = [
    DrawStep::Border,
    DrawStep::BaseArt,
    DrawStep::LifeGlobe,
    DrawStep::ManaGlobe,
    DrawStep::ExperienceBar,
    DrawStep::RunButton,
    DrawStep::StaminaBar,
    DrawStep::MenuButton,
    DrawStep::Belt,
    DrawStep::SkillButtons,
    DrawStep::BeltItemHover,
    DrawStep::LevelTimer,
    DrawStep::MenuTip,
];

/// Step 8 of the UI pass, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step8 {
    /// `[6]`, else.
    NewStatsButton,
    /// `[7]`, else.
    NewSkillsButton,
    LifeManaNumbers,
    /// The mini panel, with state 0x15 open.
    MiniPanel,
}

pub const STEP8: [Step8; 4] = [
    Step8::NewStatsButton,
    Step8::NewSkillsButton,
    Step8::LifeManaNumbers,
    Step8::MiniPanel,
];

/// Whether a step 8 entry draws (`state_open(ui)`): the mini panel only
/// with state 0x15 open; the others always (their own gates are their modules').
pub fn step8_runs(step: Step8, state_open: &dyn Fn(u8) -> bool) -> bool {
    match step {
        Step8::MiniPanel => state_open(0x15),
        _ => true,
    }
}

/// The border sides at resolution mode 2 only: left when the open
/// mode is 2 or 3, right when it is 1 or 3.
pub fn border_sides(res_mode: u8, open_mode: u8) -> (bool, bool) {
    if res_mode != 2 {
        return (false, false);
    }
    (matches!(open_mode, 2 | 3), matches!(open_mode, 1 | 3))
}

/// The level-change timer: when P exists and its level id
/// differs from `level_id`, `level_id`:= it and `timer`:= 60
/// (the level name shows while the timer runs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelTimer {
    /// The last level id seen.
    pub level_id: u32,
    /// The level name timer.
    pub timer: u32,
}

/// The level name timer after a level change.
pub const LEVEL_TIMER_START: u32 = 60;

impl LevelTimer {
    pub fn update(&mut self, player_level: Option<u32>) {
        if let Some(l) = player_level {
            if l != self.level_id {
                self.level_id = l;
                self.timer = LEVEL_TIMER_START;
            }
        }
    }
}

/// Every cel draw of the control panel is `CelDraw` with
/// light 0xFF and draw mode 5 unless a rule says otherwise.
pub const CEL_LIGHT: u8 = 0xFF;
pub const CEL_MODE: u8 = 5;

/// The art loaded from the UI folder (`"%s\UI\"` + name).
pub const CONTROL_ART: [&str; 5] = [
    "Panel\\Hlthmana",
    "Panel\\overlap",
    "Panel\\runbutton",
    "Panel\\menubutton",
    "Panel\\ctrlpnl_popbelt",
];
/// `Panel\Level` and `Panel\Levelsocket`.
pub const LEVEL_ART: [&str; 2] = ["Panel\\Level", "Panel\\Levelsocket"];
/// Loaded there and drawn elsewhere.
pub const OTHER_ART: [&str; 9] = [
    "Panel\\btn",
    "InvChar6",
    "InvChar",
    "Arrow",
    "Invwarn",
    "DuroIcon",
    "GemSocket",
    "menupanel",
    "skillpoints",
];
/// `hostilepic` is the tenth of the list.
pub const HOSTILE_PIC: &str = "hostilepic";

/// The mini panel art: `Panel\minipanel` /
/// `Panel\minipanel_s` and `Panel\minipanelbtn`.
pub fn mini_panel_art(multiplayer: bool) -> [&'static str; 2] {
    [
        if multiplayer {
            "Panel\\minipanel"
        } else {
            "Panel\\minipanel_s"
        },
        "Panel\\minipanelbtn",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step7_order() {
        // After the border and the base art: life globe, mana globe,
        // experience bar, run button, stamina bar, menu button, belt, skill
        // buttons, belt item hover text.
        assert_eq!(
            &DRAW_ORDER[2..11],
            &[
                DrawStep::LifeGlobe,
                DrawStep::ManaGlobe,
                DrawStep::ExperienceBar,
                DrawStep::RunButton,
                DrawStep::StaminaBar,
                DrawStep::MenuButton,
                DrawStep::Belt,
                DrawStep::SkillButtons,
                DrawStep::BeltItemHover,
            ]
        );
    }

    #[test]
    fn border_and_menu_tip_in_order() {
        // The border is the first step, only at resolution mode 2: left
        // when the open mode is 2 or 3, right when it is 1 or 3.
        assert_eq!(DRAW_ORDER[0], DrawStep::Border);
        assert_eq!(border_sides(0, 3), (false, false));
        assert_eq!(border_sides(2, 0), (false, false));
        assert_eq!(border_sides(2, 1), (false, true));
        assert_eq!(border_sides(2, 2), (true, false));
        assert_eq!(border_sides(2, 3), (true, true));
        // The menu button tool tip closes the step.
        assert_eq!(DRAW_ORDER[12], DrawStep::MenuTip);
        assert_eq!(DRAW_ORDER[11], DrawStep::LevelTimer);
    }

    #[test]
    fn base_art_follows_the_border() {
        assert_eq!(DRAW_ORDER[1], DrawStep::BaseArt);
    }

    #[test]
    fn level_change_timer() {
        let mut t = LevelTimer::default();
        // No player: unchanged.
        t.update(None);
        assert_eq!(t, LevelTimer::default());
        // A level that differs: id stored, timer 60.
        t.update(Some(1));
        assert_eq!(
            t,
            LevelTimer {
                level_id: 1,
                timer: 60
            }
        );
        // The same level again: the timer is left (the reader counts it
        // down).
        t.timer = 30;
        t.update(Some(1));
        assert_eq!(t.timer, 30);
        t.update(Some(2));
        assert_eq!(
            t,
            LevelTimer {
                level_id: 2,
                timer: 60
            }
        );
        // Level id 0 differs from the initial 0? No: equal, no timer.
        let mut z = LevelTimer::default();
        z.update(Some(0));
        assert_eq!(z.timer, 0);
    }

    // (the step text is exempt)
    #[test]
    fn step8_order_and_mini_panel_gate() {
        assert_eq!(
            STEP8,
            [
                Step8::NewStatsButton,
                Step8::NewSkillsButton,
                Step8::LifeManaNumbers,
                Step8::MiniPanel
            ]
        );
        let none = |_: u8| false;
        assert!(step8_runs(Step8::NewStatsButton, &none));
        assert!(step8_runs(Step8::LifeManaNumbers, &none));
        assert!(!step8_runs(Step8::MiniPanel, &none));
        assert!(step8_runs(Step8::MiniPanel, &|s| s == 0x15));
    }

    #[test]
    fn art_files() {
        assert_eq!(
            CONTROL_ART,
            [
                "Panel\\Hlthmana",
                "Panel\\overlap",
                "Panel\\runbutton",
                "Panel\\menubutton",
                "Panel\\ctrlpnl_popbelt"
            ]
        );
        assert_eq!(LEVEL_ART, ["Panel\\Level", "Panel\\Levelsocket"]);
        assert_eq!(
            mini_panel_art(true),
            ["Panel\\minipanel", "Panel\\minipanelbtn"]
        );
        assert_eq!(
            mini_panel_art(false),
            ["Panel\\minipanel_s", "Panel\\minipanelbtn"]
        );
        // Also loaded there and drawn elsewhere.
        assert!(OTHER_ART.contains(&"InvChar6") && OTHER_ART.contains(&"skillpoints"));
        assert_eq!(HOSTILE_PIC, "hostilepic");
        // Every cel draw: light 0xFF and draw mode 5.
        assert_eq!((CEL_LIGHT, CEL_MODE), (0xFF, 5));
    }
}
