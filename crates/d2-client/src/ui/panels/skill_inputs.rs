//! Skill tree inputs: which skills are learnable with
//! free points, the level number and its color, and the icon remap `k`.

/// The `skills.txt` fields the learnability test reads.
#[derive(Clone, Copy, Debug)]
pub struct SkillReq {
    /// `reqlevel`.
    pub req_level: i32,
    /// `reqskill1`–`reqskill3` (None = not set).
    pub req_skills: [Option<i32>; 3],
    /// The `InGame` flag (+5 & 4, bit 10).
    pub in_game: bool,
    pub req_str: i32,
    pub req_dex: i32,
    pub req_int: i32,
    pub req_vit: i32,
    /// `maxlvl` (+0x12C); 20 when < 1.
    pub max_lvl: i32,
}

/// The player's side of the test.
#[derive(Clone, Copy)]
pub struct PlayerFacts<'a> {
    /// Stat 12.
    pub level: i32,
    /// Stats 0, 2, 1, 3 (full values).
    pub strength: i32,
    pub dexterity: i32,
    pub energy: i32,
    pub vitality: i32,
    /// Base stat 5.
    pub free_points: i32,
    /// `skill_level(P, e, 0)` of this skill (0 without a native entry).
    pub base: i32,
    /// The native base level of another skill, `None`
    /// without a native entry.
    pub native_base: &'a dyn Fn(i32) -> Option<i32>,
}

/// The maximum level: `maxlvl`, 20 when < 1.
pub fn max_level(max_lvl: i32) -> i32 {
    if max_lvl < 1 {
        20
    } else {
        max_lvl
    }
}

/// Level ≥ `reqlevel` and each set `reqskill` names a skill
/// the player has natively with base ≥ 1.
fn prereqs(r: &SkillReq, p: &PlayerFacts<'_>) -> bool {
    p.level >= r.req_level
        && r.req_skills
            .iter()
            .flatten()
            .all(|&s| (p.native_base)(s).is_some_and(|b| b >= 1))
}

/// `InGame`, level and the four stats.
fn requirements(r: &SkillReq, p: &PlayerFacts<'_>) -> bool {
    r.in_game
        && p.level >= r.req_level
        && p.strength >= r.req_str
        && p.dexterity >= r.req_dex
        && p.energy >= r.req_int
        && p.vitality >= r.req_vit
}

/// Learnable: the prerequisites, base below the
/// maximum, the stat requirements, and free points ≥ the cost (`cost` =
/// `skillcalc(P, skpoints, s, b)` when `skpoints` ≠ −1, else 1).
pub fn learnable(r: &SkillReq, p: &PlayerFacts<'_>, cost: i32) -> bool {
    prereqs(r, p) && p.base < max_level(r.max_lvl) && requirements(r, p) && p.free_points >= cost
}

/// The icon state and remap of icon i after
/// the test: not learnable → −1 and 5; learnable and state ≠ 1
/// → 0 and 0 (a pressed icon stays pressed).
pub fn icon_state(learnable: bool, state: i32, remap: i32) -> (i32, i32) {
    if !learnable {
        (-1, 5)
    } else if state != 1 {
        (0, 0)
    } else {
        (state, remap)
    }
}

/// The level number `L`: base + `bonus_level`, at least 0,
/// capped by the skill level cap; 0 without an entry.
pub fn level_number(entry: bool, base: i32, bonus: i32, cap: i32) -> i32 {
    if !entry {
        return 0;
    }
    (base + bonus).max(0).min(cap)
}

/// Drawn when `b` ≠ 0 or `L` ≠ 0.
pub fn level_drawn(b: i32, l: i32) -> bool {
    b != 0 || l != 0
}

/// Color by `bonus_level`: > 0 → 3, < 0 → 1, else 0.
pub fn level_color(bonus: i32) -> u8 {
    match bonus {
        1.. => 3,
        0 => 0,
        _ => 1,
    }
}

/// The remap `k`, in this order: the remap table
/// value; 3 when the mouse is strictly inside the icon; 1 when the
/// skill's `InGame` flag is clear; 3 when `b` = 0, `bonus_level` ≠ 0 and
/// the state ≠ −1.
pub fn remap_k(
    table: i32,
    mouse_inside: bool,
    in_game: bool,
    b: i32,
    bonus: i32,
    state: i32,
) -> i32 {
    let mut k = table;
    if mouse_inside {
        k = 3;
    }
    if !in_game {
        k = 1;
    }
    if b == 0 && bonus != 0 && state != -1 {
        k = 3;
    }
    k
}

/// The icon frame: `IconCel` + 1 while pressed.
pub fn icon_frame(icon_cel: u32, pressed: bool) -> u32 {
    icon_cel + u32::from(pressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> SkillReq {
        SkillReq {
            req_level: 6,
            req_skills: [Some(36), None, None],
            in_game: true,
            req_str: 0,
            req_dex: 20,
            req_int: 0,
            req_vit: 0,
            max_lvl: 0,
        }
    }

    fn native(s: i32) -> Option<i32> {
        match s {
            36 => Some(1),
            37 => Some(0),
            _ => None,
        }
    }

    fn player() -> PlayerFacts<'static> {
        PlayerFacts {
            level: 10,
            strength: 20,
            dexterity: 25,
            energy: 10,
            vitality: 20,
            free_points: 3,
            base: 0,
            native_base: &native,
        }
    }

    #[test]
    fn learnability() {
        let r = req();
        let p = player();
        assert!(learnable(&r, &p, 1));
        // each condition alone fails it
        assert!(!learnable(&SkillReq { req_level: 11, ..r }, &p, 1));
        assert!(!learnable(
            &SkillReq {
                req_skills: [Some(37), None, None],
                ..r
            },
            &p,
            1
        ));
        assert!(!learnable(
            &SkillReq {
                req_skills: [Some(99), None, None],
                ..r
            },
            &p,
            1
        ));
        assert!(!learnable(
            &SkillReq {
                in_game: false,
                ..r
            },
            &p,
            1
        ));
        assert!(!learnable(&SkillReq { req_dex: 26, ..r }, &p, 1));
        assert!(!learnable(&SkillReq { req_str: 21, ..r }, &p, 1));
        assert!(!learnable(&SkillReq { req_int: 11, ..r }, &p, 1));
        assert!(!learnable(&SkillReq { req_vit: 21, ..r }, &p, 1));
        assert!(!learnable(
            &r,
            &PlayerFacts {
                free_points: 0,
                ..p
            },
            1
        ));
        // the cost can exceed the points
        assert!(!learnable(&r, &p, 4));
        assert!(learnable(&r, &p, 3));
        // maxlvl: 20 when < 1; base must be below it
        assert_eq!(max_level(0), 20);
        assert_eq!(max_level(-3), 20);
        assert_eq!(max_level(5), 5);
        assert!(!learnable(&r, &PlayerFacts { base: 20, ..p }, 1));
        assert!(learnable(&r, &PlayerFacts { base: 19, ..p }, 1));
        assert!(!learnable(
            &SkillReq { max_lvl: 5, ..r },
            &PlayerFacts { base: 5, ..p },
            1
        ));
        // state and remap
        assert_eq!(icon_state(false, 0, 0), (-1, 5));
        assert_eq!(icon_state(true, -1, 5), (0, 0));
        assert_eq!(
            icon_state(true, 1, 5),
            (1, 5),
            "a pressed icon stays pressed"
        );
        // the icon frame is IconCel + 1 while pressed
        assert_eq!(icon_frame(4, false), 4);
        assert_eq!(icon_frame(4, true), 5);
    }

    #[test]
    fn level_number_and_color() {
        assert_eq!(level_number(false, 5, 5, 60), 0);
        assert_eq!(level_number(true, 5, 3, 60), 8);
        assert_eq!(level_number(true, 2, -5, 60), 0);
        assert_eq!(level_number(true, 50, 20, 60), 60);
        assert!(level_drawn(1, 0));
        assert!(level_drawn(0, 2));
        assert!(!level_drawn(0, 0));
        assert_eq!((level_color(2), level_color(0), level_color(-1)), (3, 0, 1));
    }

    #[test]
    fn remap_order() {
        // the table value by default
        assert_eq!(remap_k(5, false, true, 1, 0, 0), 5);
        // the mouse inside: 3
        assert_eq!(remap_k(5, true, true, 1, 0, 0), 3);
        // a clear InGame flag: 1 (beats the mouse)
        assert_eq!(remap_k(5, true, false, 1, 0, 0), 1);
        // b = 0 with a bonus and the state ≠ −1: 3 (beats both)
        assert_eq!(remap_k(5, true, false, 0, 2, 0), 3);
        // state −1: the last clause does not apply
        assert_eq!(remap_k(5, true, false, 0, 2, -1), 1);
        assert_eq!(remap_k(0, false, true, 0, 0, 0), 0);
    }
}
