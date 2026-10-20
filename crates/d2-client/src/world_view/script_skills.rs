//! The skills of an input script's `skill` and `cast` steps
//! ([`super::input_script`]): a skill written by name (`fireball`,
//! `fire_ball`, `Fire Ball`) or by its `skills` id, resolved when the step
//! runs against the names of the player's own `skills.txt`, and the
//! select-skill message the skill picker and the hotkeys send for it
//! ([`crate::ui::original::hud_hotkeys::select`]).
//!
//! Client input only: the step selects a skill the local player's skill
//! list already holds, as a player picks it from the skill menu; the
//! server answers the select as it answers the picker's.

use d2_data::bin::TableFiles;

use crate::bridge::skills::NATIVE;
use crate::bridge::world::ClientWorld;
use crate::ui::panel::ClientIntent;
use crate::ui::PointerButton;

/// The longest skill name a step can carry (bytes).
pub const MAX_SKILL_NAME: usize = 32;

/// A skill as a step writes it: a name or a decimal id, kept as text
/// until the step runs (the names come from the player's tables).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SkillName {
    bytes: [u8; MAX_SKILL_NAME],
    len: u8,
}

impl SkillName {
    /// The skill `text` (printable ASCII, 1 to [`MAX_SKILL_NAME`] bytes).
    pub fn new(text: &str) -> Option<Self> {
        let b = text.as_bytes();
        if b.is_empty()
            || b.len() > MAX_SKILL_NAME
            || !b.iter().all(|c| c.is_ascii_graphic() || *c == b' ')
        {
            return None;
        }
        let mut bytes = [0u8; MAX_SKILL_NAME];
        bytes[..b.len()].copy_from_slice(b);
        Some(SkillName {
            bytes,
            len: b.len() as u8,
        })
    }

    /// The text as written.
    pub fn as_str(&self) -> &str {
        // Built from a `&str` of ASCII bytes only.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }

    /// The id, when the step names the skill by number.
    pub fn id(&self) -> Option<u16> {
        self.as_str().parse().ok()
    }
}

impl std::fmt::Debug for SkillName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for SkillName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A skill name as a step matches it: lower case, without spaces,
/// underscores or hyphens (`Fire Ball`, `fire_ball` and `fireball` match).
pub fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The skill names of the player's `skills.txt`: its `skill` column by
/// its `Id` column (the row index when the table has no `Id`).
#[derive(bevy::prelude::Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillNames {
    /// (name key, id), in table order.
    rows: Vec<(String, u16)>,
}

impl SkillNames {
    /// The names of a `skills.txt` text (tab separated, a header row).
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
        let col = |n: &str| head.iter().position(|h| h.trim().eq_ignore_ascii_case(n));
        let c_name = col("skill").ok_or("skills.txt has no `skill` column")?;
        let c_id = col("Id");
        let mut rows = Vec::new();
        for (i, l) in lines.enumerate() {
            let f: Vec<&str> = l.split('\t').collect();
            let name = f.get(c_name).map_or("", |s| s.trim());
            if name.is_empty() {
                continue;
            }
            let id = match c_id {
                Some(c) => f.get(c).and_then(|s| s.trim().parse().ok()),
                None => u16::try_from(i).ok(),
            };
            if let Some(id) = id {
                rows.push((name_key(name), id));
            }
        }
        Ok(SkillNames { rows })
    }

    /// The names of the player's tables, or `None` when they hold no
    /// `skills.txt` (steps then name skills by id).
    pub fn load(files: &dyn TableFiles) -> Result<Option<Self>, String> {
        match files.read_excel("skills.txt").map_err(|e| e.to_string())? {
            Some((_, bytes)) => Self::from_text(&String::from_utf8_lossy(&bytes)).map(Some),
            None => Ok(None),
        }
    }

    /// The id of the first skill whose name matches `name` ([`name_key`]).
    pub fn id(&self, name: &str) -> Option<u16> {
        let key = name_key(name);
        self.rows.iter().find(|(k, _)| *k == key).map(|&(_, id)| id)
    }
}

/// The id a step's skill names: its number, or its name in `names`.
pub fn skill_id(names: Option<&SkillNames>, name: &SkillName) -> Result<u16, String> {
    if let Some(id) = name.id() {
        return Ok(id);
    }
    let names = names.ok_or_else(|| {
        format!(
            "skill `{name}`: no skill names loaded (the install has no skills.txt); \
             name the skill by its id"
        )
    })?;
    names
        .id(name.as_str())
        .ok_or_else(|| format!("no skill named `{name}` in skills.txt"))
}

/// The select-skill message of a step: `name` on `hand`, from the local
/// player's skill list (its own entry of the skill first, else an item
/// skill with charges left, as the skill menu offers them), or why the
/// player cannot select it.
pub fn select_skill(
    world: &ClientWorld,
    names: Option<&SkillNames>,
    name: &SkillName,
    hand: PointerButton,
) -> Result<(u16, ClientIntent), String> {
    let id = skill_id(names, name)?;
    let player = world.local().ok_or("no local player")?;
    let list = player
        .skills
        .as_ref()
        .ok_or("the player's skill list has not arrived")?;
    let mut own = list.entries.iter().filter(|e| e.skill == id);
    let entry = own
        .clone()
        .find(|e| e.owner == NATIVE)
        .or_else(|| own.find(|e| e.charges >= 1))
        .ok_or_else(|| format!("the player does not have skill {id} (`{name}`)"))?;
    let left = hand == PointerButton::Left;
    Ok((
        id,
        crate::ui::original::hud_hotkeys::select(i32::from(id), left, entry.owner),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::skills::{SkillEntry, SkillList};
    use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};

    const TABLE: &str = "skill\tId\tcharclass\n\
                         Attack\t0\t\n\
                         Fire Bolt\t36\tsor\n\
                         Fire Ball\t47\tsor\n\
                         \t48\t\n";

    #[test]
    fn names_match_in_any_case_without_spaces_underscores_or_hyphens() {
        let n = SkillNames::from_text(TABLE).unwrap();
        for s in ["fireball", "fire_ball", "Fire Ball", "FIRE-BALL"] {
            assert_eq!(n.id(s), Some(47), "{s}");
        }
        assert_eq!(n.id("firebolt"), Some(36));
        assert_eq!(n.id("attack"), Some(0));
        assert_eq!(n.id("nova"), None);
        assert!(SkillNames::from_text("Id\tname\n").is_err());
        // no `Id` column: the row index
        let n = SkillNames::from_text("skill\nAttack\nKick\n").unwrap();
        assert_eq!(n.id("kick"), Some(1));
    }

    #[test]
    fn skill_id_reads_a_number_without_names() {
        let n = SkillNames::from_text(TABLE).unwrap();
        let name = |s| SkillName::new(s).unwrap();
        assert_eq!(skill_id(None, &name("47")), Ok(47));
        assert_eq!(skill_id(Some(&n), &name("fire_ball")), Ok(47));
        assert!(skill_id(None, &name("fireball")).is_err());
        assert!(skill_id(Some(&n), &name("nova")).is_err());
        assert!(SkillName::new("").is_none());
        assert!(SkillName::new(&"x".repeat(MAX_SKILL_NAME + 1)).is_none());
        assert_eq!(name("Fire Ball").as_str(), "Fire Ball");
    }

    #[test]
    fn select_skill_sends_the_pickers_message_for_a_skill_the_player_has() {
        let n = SkillNames::from_text(TABLE).unwrap();
        let mut w = ClientWorld::default();
        let me = UnitKey::new(PLAYER, 1);
        let mut p = ClientUnit::new(me);
        p.skills = Some(SkillList {
            entries: vec![
                SkillEntry {
                    skill: 47,
                    owner: NATIVE,
                    ..SkillEntry::default()
                },
                SkillEntry {
                    skill: 36,
                    owner: 77,
                    charges: 3,
                    ..SkillEntry::default()
                },
            ],
            ..SkillList::default()
        });
        w.units.insert(me, p);
        w.local_player = Some(me);
        let name = |s| SkillName::new(s).unwrap();
        let (id, i) = select_skill(&w, Some(&n), &name("fireball"), PointerButton::Right).unwrap();
        assert_eq!(id, 47);
        assert_eq!(
            i,
            crate::ui::original::hud_hotkeys::select(47, false, NATIVE)
        );
        assert_eq!(i.0[0], 0x3C);
        // an item skill: its owner's entry
        let (_, i) = select_skill(&w, Some(&n), &name("Fire Bolt"), PointerButton::Left).unwrap();
        assert_eq!(i, crate::ui::original::hud_hotkeys::select(36, true, 77));
        let e = select_skill(&w, Some(&n), &name("attack"), PointerButton::Left).unwrap_err();
        assert!(e.contains("does not have skill 0"), "{e}");
    }
}
