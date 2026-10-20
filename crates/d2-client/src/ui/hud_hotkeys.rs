//! The skill hotkeys and the skill select panel's input: the hotkey
//! table (16 slots of skill, left flag and owner GUID, plus the last used
//! slot), the panel's hit records (60 of them, rebuilt by each draw), its
//! left click, a hotkey assign with the panel open, a hotkey use, the
//! wheel step and the server's hotkey assignment (S→C 0x7B).
//!
//! The functions are pure: they take the table, the records, the local
//! player's skill list and the mouse, and return the messages to send and
//! whether the panel closes; the HUD applies them
//! ([`super::hud::HudUi`]). D2RUST never changes the left / right skill on
//! its own: the switch to the used skill is left to the server's answer.

use d2_proto::client::{BindHotkey, SelectSkill};

use super::hud::SelectIcon;
use crate::bridge::skills::{SkillList, NATIVE};
use crate::ui::panel::ClientIntent;

/// Hotkey slots.
pub const SLOTS: usize = 16;
/// Hit records of the select panel.
pub const RECORDS: usize = 60;
/// An icon's hit box side (x…x + 48, y − 48…y, both inclusive).
const SIDE: i32 = 0x30;

/// One hotkey slot. `skill` −1 = none; a cleared slot's left flag and
/// owner are −1 too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub skill: i32,
    pub left: bool,
    pub owner: u32,
}

impl Hotkey {
    pub const NONE: Self = Self {
        skill: -1,
        left: true,
        owner: u32::MAX,
    };
}

/// The hotkey table and the last used slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkeys {
    pub slots: [Hotkey; SLOTS],
    /// The last used slot (0 until the first use).
    pub last: usize,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            slots: [Hotkey::NONE; SLOTS],
            last: 0,
        }
    }
}

/// One hit record: the icon's x, its bottom y, the skill
/// and the owner GUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub x: i32,
    pub y: i32,
    pub skill: i32,
    pub owner: u32,
}

/// The 60 records of a draw: the icons in draw order, then
/// unused records of x 0, y 0, skill 0, owner −1 (the draw's clear).
pub fn records(icons: &[SelectIcon]) -> Vec<Record> {
    let mut v: Vec<Record> = icons
        .iter()
        .take(RECORDS)
        .map(|i| Record {
            x: i.at.0,
            y: i.at.1,
            skill: i32::from(i.skill),
            owner: i.owner,
        })
        .collect();
    v.resize(
        RECORDS,
        Record {
            x: 0,
            y: 0,
            skill: 0,
            owner: NATIVE,
        },
    );
    v
}

/// Whether the mouse is over a record (both edges inclusive, so a point
/// on the shared edge of two icons is over both).
fn over(r: &Record, m: (i32, i32)) -> bool {
    (r.x..=r.x + SIDE).contains(&m.0) && (r.y - SIDE..=r.y).contains(&m.1)
}

/// The skill list's entry of (skill, owner), if any.
fn entry(list: &SkillList, skill: i32, owner: u32) -> Option<&crate::bridge::skills::SkillEntry> {
    let s = u16::try_from(skill).ok()?;
    list.find(s, owner).and_then(|i| list.entries.get(i))
}

/// An item skill (an owner other than the player) whose charges are
/// spent (fewer than 1).
fn spent(e: &crate::bridge::skills::SkillEntry) -> bool {
    e.owner != NATIVE && e.charges < 1
}

/// The select-skill message (C→S 0x3C) the skill picker and the hotkeys
/// send: `skill` on the left hand (`left`) or the right, from the skill
/// list entry of `owner` (the item GUID of an item skill, else
/// [`NATIVE`]). Scripts select a skill through it too
/// ([`crate::world_view::script_skills`]).
pub fn select(skill: i32, left: bool, owner: u32) -> ClientIntent {
    ClientIntent::from_message(&SelectSkill {
        skill: skill as u32 & 0x7FFF_FFFF,
        left,
        item: owner,
    })
}

fn bind(slot: usize, skill: u16, left: bool, item: u32) -> ClientIntent {
    ClientIntent::from_message(&BindHotkey {
        skill,
        left,
        slot: slot as u16,
        item,
    })
}

/// What a select panel click does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Click {
    pub intents: Vec<ClientIntent>,
    /// The skill panel closes (it may already be closed).
    pub close: bool,
}

/// The select panel's left button down: every
/// record under the mouse with an entry sends C→S 0x3C with the panel's
/// side flag, closes the panel and clears the flag, so a second record
/// under the mouse (a shared edge) goes out as a right skill; a spent
/// item skill closes the panel and ends the walk. With nothing sent and
/// the mouse at y ≤ H − 49 the panel closes. The event is always
/// consumed.
pub fn click(
    recs: &[Record],
    list: &SkillList,
    left: &mut bool,
    mouse: (i32, i32),
    h: i32,
) -> Click {
    let mut out = Click::default();
    let mut found = false;
    for r in recs {
        if !over(r, mouse) {
            continue;
        }
        let Some(e) = entry(list, r.skill, r.owner) else {
            continue;
        };
        if spent(e) {
            out.close = true;
            return out;
        }
        out.intents.push(select(r.skill, *left, r.owner));
        out.close = true;
        *left = false;
        found = true;
    }
    if mouse.1 <= h - 0x31 && !found {
        out.close = true;
    }
    out
}

/// Hotkey k with the panel open: each record
/// under the mouse becomes slot k (an old binding is first unbound), with
/// the panel's side flag; any other slot holding the same skill and owner
/// is cleared. Returns the C→S 0x51 messages in order.
pub fn assign(
    keys: &mut Hotkeys,
    k: usize,
    recs: &[Record],
    left: bool,
    mouse: (i32, i32),
) -> Vec<ClientIntent> {
    let mut out = Vec::new();
    if k >= SLOTS {
        return out;
    }
    for r in recs {
        if !over(r, mouse) {
            continue;
        }
        if keys.slots[k].skill != -1 {
            out.push(bind(k, 0x7FFF, true, u32::MAX));
        }
        keys.slots[k] = Hotkey {
            skill: r.skill,
            left,
            owner: r.owner,
        };
        out.push(bind(k, r.skill as u16 & 0x7FFF, left, r.owner));
        let mine = keys.slots[k];
        for i in 0..SLOTS {
            let s = keys.slots[i];
            if i != k && s.skill == mine.skill && s.owner == mine.owner {
                keys.slots[i] = Hotkey::NONE;
                out.push(bind(i, 0x7FFF, true, u32::MAX));
            }
        }
    }
    out
}

/// Whether slot k is usable: it holds a skill whose entry exists and is not a
/// spent item skill.
pub fn usable(keys: &Hotkeys, k: usize, list: &SkillList) -> bool {
    let Some(s) = keys.slots.get(k) else {
        return false;
    };
    s.skill != -1 && entry(list, s.skill, s.owner).is_some_and(|e| !spent(e))
}

/// Hotkey k with the panel closed: a usable slot becomes the last used and sends C→S 0x3C.
pub fn use_slot(keys: &mut Hotkeys, k: usize, list: &SkillList) -> Option<ClientIntent> {
    if !usable(keys, k, list) {
        return None;
    }
    keys.last = k;
    let s = keys.slots[k];
    Some(select(s.skill, s.left, s.owner))
}

/// The wheel (commands 39 / 40 = −1 / +1): from the last
/// used slot, step by the sign of d with wrap over 0–15, at most 16
/// steps; a slot counts when it holds a right skill that is usable; after
/// |d| counted slots (or the 16 steps) the last counted one is used.
pub fn wheel(keys: &mut Hotkeys, d: i32, list: &SkillList) -> Option<ClientIntent> {
    let mut left = d.unsigned_abs();
    if left == 0 {
        return None;
    }
    let step = if d > 0 { 1 } else { SLOTS - 1 };
    let mut i = keys.last;
    let mut found = None;
    for _ in 0..SLOTS {
        i = (i + step) % SLOTS;
        let s = keys.slots[i];
        if s.skill == -1 || s.left || !usable(keys, i, list) {
            continue;
        }
        found = Some(i);
        left -= 1;
        if left == 0 {
            break;
        }
    }
    use_slot(keys, found?, list)
}

/// S→C 0x7B, the server's hotkey assignment: a slot past 15 is dropped
/// (there is no such slot).
pub fn server_assign(keys: &mut Hotkeys, slot: u8, skill: i32, left: bool, item: u32) {
    if let Some(s) = keys.slots.get_mut(usize::from(slot)) {
        *s = Hotkey {
            skill,
            left,
            owner: item,
        };
    }
}

/// The key command of slot i (its label's): 14 + i for i < 8,
/// 46 + (i − 8) after.
pub fn slot_cmd(i: usize) -> u16 {
    if i < 8 {
        14 + i as u16
    } else {
        46 + (i as u16 - 8)
    }
}

/// The slots whose labels the overlay draws on a record (left or right
/// panel): same skill and owner, and the
/// left flag equal to the panel's side.
pub fn labels_of(keys: &Hotkeys, r: &Record, left: bool) -> Vec<usize> {
    (0..SLOTS)
        .filter(|&i| {
            let s = keys.slots[i];
            s.skill != -1 && s.skill == r.skill && s.owner == r.owner && s.left == left
        })
        .collect()
}

/// The longest hotkey label or charges count the overlay keeps (px).
const FIT_W: i32 = 0x28;

/// The overlay texts' cut: a text wider than
/// 40 px is copied into a 16-unit buffer and loses its last unit until it
/// fits. `None` when the font has no width for it.
pub fn fit(text: &[u16], width: &dyn Fn(&[u16]) -> Option<i32>) -> Option<Vec<u16>> {
    let mut t = text.to_vec();
    if width(&t)? > FIT_W {
        t.truncate(15);
        while !t.is_empty() && width(&t)? > FIT_W {
            t.pop();
        }
    }
    Some(t)
}

/// Where a hotkey label of width `w` goes on an icon at (x, bottom y)
/// (x + 0x22, y − 0x23; a label wider than 13 px moves left by the
/// excess).
pub fn label_at(icon: (i32, i32), w: i32) -> (i32, i32) {
    let dx = if w > 13 { 13 - w } else { 0 };
    (icon.0 + 0x22 + dx, icon.1 - 0x23)
}

/// The charges count's text and place: n > 0 in
/// decimal at (x + 4, y − 1).
pub fn count_text(n: i32, icon: (i32, i32)) -> Option<(Vec<u16>, (i32, i32))> {
    (n > 0).then(|| {
        (
            n.to_string().encode_utf16().collect(),
            (icon.0 + 4, icon.1 - 1),
        )
    })
}

#[cfg(test)]
#[path = "hud_hotkeys_tests.rs"]
mod tests;
